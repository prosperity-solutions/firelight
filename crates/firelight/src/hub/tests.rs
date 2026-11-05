//! The hub end to end, in process: a file:// git source, last good, warm
//! restart from the cache, deleting the cache, and the self-docs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use firelight_source::SystemClock;
use firelight_source::git::Git;
use firelight_source::pull::{Credentials, Origin, Puller, SourceSpec};
use firelight_store::Store;

use super::*;

fn git(dir: &Path, args: &[&str]) {
    let mut c = Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            c.env_remove(k);
        }
    }
    let out = c
        .args([
            "-c",
            "user.email=t@acme.example",
            "-c",
            "user.name=T",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Setup {
    root: PathBuf,
    work: PathBuf,
    url: String,
}

impl Setup {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("fl-hub-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap_or_else(|e| panic!("{e}"));
        git(&root, &["init", "--bare", "-q", "infra.git"]);
        git(&work, &["init", "-q"]);
        let url = format!("file://{}", root.join("infra.git").display());
        Setup { root, work, url }
    }
    fn commit(&self, files: &[(&str, &str)]) {
        for (p, c) in files {
            let p = self.work.join(p);
            std::fs::create_dir_all(p.parent().unwrap_or(&self.work))
                .unwrap_or_else(|e| panic!("{e}"));
            std::fs::write(p, c).unwrap_or_else(|e| panic!("{e}"));
        }
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-q", "-m", "x"]);
        git(&self.work, &["push", "-q", "-f", &self.url, "HEAD:main"]);
    }
    fn hub(&self) -> Arc<Hub> {
        let data = self.root.join("data");
        let store = Store::open(&data.join("firelight.db")).unwrap_or_else(|e| panic!("{e}"));
        let puller = Puller {
            git: Git::default(),
            data_dir: data,
            limits: Limits::default(),
            clock: Arc::new(SystemClock),
        };
        let spec = SourceSpec {
            id: "infra".into(),
            origin: Origin::Git {
                url: self.url.clone(),
                git_ref: None,
                creds: Credentials::Anonymous,
            },
        };
        Hub::new(vec![spec], puller, store, Limits::default(), true, "off")
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const REPO: &str = "format: 1\nid: infra\nname: Infra\nsummary: Infra.\n";

fn repo_yaml() -> String {
    REPO.to_string()
}

fn concept(title: &str) -> String {
    format!("---\nformat: 1\ntitle: {title}\nsummary: S.\n---\nBody.\n")
}

#[tokio::test]
async fn last_good_warm_restart_and_a_lost_cache() {
    let s = Setup::new("life");
    s.commit(&[
        (".firelight/repo.yaml", &repo_yaml()),
        (".firelight/concepts/argo.md", &concept("Argo")),
    ]);
    let hub = s.hub();
    assert!(!hub.load_cache(), "cold: nothing cached");
    // The self-docs are there before any pull.
    assert!(hub.catalog().repo("firelight").is_some());
    assert!(hub.pull("infra").await);
    assert_eq!(
        hub.catalog()
            .entry(&firelight_format::Ref {
                repo: "infra".into(),
                id: "argo".into()
            })
            .map(|e| e.title.as_str()),
        Some("Argo")
    );

    // A broken commit: the last good one stays served, the reason is recorded.
    s.commit(&[(
        ".firelight/concepts/argo.md",
        "---\nformat: 1\ntitle: Argo\nsummary: S.\nrelated: [nope]\n---\nx\n",
    )]);
    assert!(!hub.pull("infra").await);
    assert!(hub.catalog().repo("infra").is_some());
    let st = hub
        .source_states()
        .into_iter()
        .find(|x| x.source_id == "infra")
        .unwrap_or_default();
    assert!(
        st.newest_diagnostics
            .unwrap_or_default()
            .contains("unresolved-ref")
    );
    assert_ne!(st.newest_commit, st.served_commit);

    // Fixed again.
    s.commit(&[(".firelight/concepts/argo.md", &concept("Argo CD"))]);
    assert!(hub.pull("infra").await);
    drop(hub);

    // A warm restart serves the cache before any pull.
    let hub = s.hub();
    assert!(hub.load_cache(), "warm");
    assert_eq!(
        hub.catalog()
            .entry(&firelight_format::Ref {
                repo: "infra".into(),
                id: "argo".into()
            })
            .map(|e| e.title.as_str()),
        Some("Argo CD")
    );
    // Unchanged remote: nothing to do.
    assert!(!hub.pull("infra").await);
    drop(hub);

    // Delete the database: nothing is lost, the next pull rebuilds it.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!(
            "{}{suffix}",
            s.root.join("data/firelight.db").display()
        ));
    }
    let hub = s.hub();
    assert!(!hub.load_cache());
    assert!(hub.pull("infra").await);
    assert_eq!(
        hub.catalog()
            .entry(&firelight_format::Ref {
                repo: "infra".into(),
                id: "argo".into()
            })
            .map(|e| e.title.as_str()),
        Some("Argo CD")
    );
}

#[test]
fn the_embedded_self_docs_pass_their_own_checks() {
    let (repo, diags) =
        firelight_format::parse_repo(&self_docs_files(), Some("firelight"), &Limits::default());
    assert!(repo.is_some(), "{diags:?}");
    assert!(
        !diags.iter().any(firelight_format::Diagnostic::is_error),
        "{diags:?}"
    );
}

#[tokio::test]
async fn why_a_commit_is_rejected_survives_failed_pulls_and_restarts() {
    let s = Setup::new("survive");
    s.commit(&[
        (".firelight/repo.yaml", &repo_yaml()),
        (".firelight/concepts/argo.md", &concept("Argo")),
    ]);
    let hub = s.hub();
    hub.load_cache();
    assert!(hub.pull("infra").await);
    s.commit(&[(
        ".firelight/concepts/argo.md",
        "---\nformat: 1\ntitle: Argo\nsummary: S.\nrelated: [nope]\n---\nx\n",
    )]);
    assert!(!hub.pull("infra").await);
    let why = |h: &Hub| {
        h.source_states()
            .into_iter()
            .find(|x| x.source_id == "infra")
            .and_then(|x| x.newest_diagnostics)
            .unwrap_or_default()
    };
    assert!(why(&hub).contains("unresolved-ref"));
    // The remote goes away: the pull fails, the reason stays.
    let moved = s.root.join("moved.git");
    std::fs::rename(s.root.join("infra.git"), &moved).unwrap_or_else(|e| panic!("{e}"));
    assert!(!hub.pull("infra").await);
    assert!(
        why(&hub).contains("unresolved-ref"),
        "a failed pull erased the reason"
    );
    std::fs::rename(&moved, s.root.join("infra.git")).unwrap_or_else(|e| panic!("{e}"));
    // Unchanged: still there.
    assert!(!hub.pull("infra").await);
    assert!(why(&hub).contains("unresolved-ref"));
    drop(hub);
    // A restart judges the rejected commit again (an upgrade may accept it).
    let hub = s.hub();
    assert!(hub.load_cache());
    assert!(!hub.pull("infra").await);
    let st = hub
        .source_states()
        .into_iter()
        .find(|x| x.source_id == "infra")
        .unwrap_or_default();
    assert!(st.last_pull_error.is_none(), "{st:?}");
    assert!(
        why(&hub).contains("unresolved-ref"),
        "judged again, still rejected"
    );
}
