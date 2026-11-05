//! Test helpers for the repo harness. The tests live in `tests/`; they read the
//! repository's own files (docs/HANDOVER.md §17.3: every rule there is asserted
//! by a test or a CI check).

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|e| panic!("repo root: {e}"))
}

pub fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

pub fn yaml(rel: &str) -> serde_json::Value {
    serde_saphyr::from_str(&read(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// A real-looking ID that is written down nowhere in the repo
/// (scripts/dev/public-ids.sh would refuse the repo otherwise).
pub fn fake_id() -> String {
    format!("{}-2222-3333-4444-{}", "1".repeat(8), "5".repeat(12))
}

/// This process's environment without git's GIT_* variables. A git hook
/// (lefthook's pre-push runs these tests) exports GIT_DIR and friends;
/// inherited, they would point the test's git commands at the real repo.
pub fn hermetic(cmd: &mut Command) -> &mut Command {
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
}

/// A unique temp dir, removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "fl-harness-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap_or_else(|e| panic!("{e}"));
        TempDir(p)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn write(&self, rel: &str, s: &str) -> PathBuf {
        let p = self.0.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap_or_else(|e| panic!("{e}"));
        }
        std::fs::write(&p, s).unwrap_or_else(|e| panic!("{e}"));
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
