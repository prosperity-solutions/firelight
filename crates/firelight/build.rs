//! Embeds Firelight's own `.firelight/` (§6.4): every file under the repo's
//! `.firelight/`, as `SELF_DOCS: &[(&str, &[u8])]`, so the binary documents
//! itself with no network, in air-gapped installs, preview and tests.

use std::fmt::Write as _;
use std::path::Path;

fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let Ok(m) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if m.is_dir() {
            walk(base, &p, out);
        } else if m.is_file() {
            let rel = p
                .strip_prefix(base)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            out.push((rel, p.display().to_string()));
        }
    }
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let base = Path::new(&manifest).join("../../.firelight");
    println!("cargo:rerun-if-changed={}", base.display());
    let mut files = Vec::new();
    walk(&base, &base, &mut files);
    let mut src = String::from(
        "/// Firelight's own `.firelight/`, embedded at build time.\npub static SELF_DOCS: &[(&str, &[u8])] = &[\n",
    );
    for (rel, abs) in &files {
        println!("cargo:rerun-if-changed={abs}");
        let _ = writeln!(src, "    ({rel:?}, include_bytes!({abs:?})),");
    }
    src.push_str("];\n");
    let out = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".into());
    if let Err(e) = std::fs::write(Path::new(&out).join("self_docs.rs"), src) {
        println!("cargo:warning=cannot write self_docs.rs: {e}");
    }
}
