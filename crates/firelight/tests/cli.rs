//! The agent-tooling commands (§12.1) through the real binary: the exit codes
//! of `preview --check`, and `skills install --project`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_firelight"));
    c.env_remove("FIRELIGHT_EMBEDDER")
        .env("FIRELIGHT_LOG_FORMAT", "pretty");
    no_hook_git(&mut c);
    c
}

/// Inside a git hook, GIT_DIR and friends point at the hook's repo.
fn no_hook_git(c: &mut Command) {
    for v in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
    ] {
        c.env_remove(v);
    }
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fl-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(&d);
    d
}

fn write(dir: &Path, p: &str, s: &str) {
    let p = dir.join(p);
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(dir));
    let _ = std::fs::write(p, s);
}

fn repo(dir: &Path, golden: &str) {
    write(
        dir,
        ".firelight/repo.yaml",
        "format: 1\nid: infra-repo\nname: Infra\nsummary: Infra.\n",
    );
    write(
        dir,
        ".firelight/guides/deploy.md",
        "---\nformat: 1\ntitle: Deploy the app\nsummary: Ship it.\n---\nSteps.\n",
    );
    write(dir, ".firelight/golden.yaml", golden);
}

fn code(c: &mut Command) -> i32 {
    c.output()
        .map(|o| o.status.code().unwrap_or(-1))
        .unwrap_or(-1)
}

#[test]
fn preview_check_exit_codes() {
    let ok = tmp("ok");
    repo(
        &ok,
        "format: 1\nquestions:\n  - query: deploy the app\n    expect: [deploy]\n",
    );
    assert_eq!(
        code(
            bin()
                .args(["preview", "--check", "--embedder", "off"])
                .arg(&ok)
        ),
        0
    );

    let failing = tmp("golden");
    repo(
        &failing,
        "format: 1\nquestions:\n  - query: zebra crossing\n    expect: [deploy]\n",
    );
    assert_eq!(
        code(
            bin()
                .args(["preview", "--check", "--embedder", "off"])
                .arg(&failing)
        ),
        1,
        "a failing golden question"
    );

    let broken = tmp("broken");
    repo(&broken, "format: 1\nquestions: []\n");
    write(
        &broken,
        ".firelight/links.yaml",
        "format: 1\nlinks:\n  - { id: x, label: X, url: 'javascript:alert(1)' }\n",
    );
    assert_eq!(
        code(
            bin()
                .args(["preview", "--check", "--embedder", "off"])
                .arg(&broken)
        ),
        1,
        "an error"
    );

    assert_eq!(
        code(
            bin()
                .args(["preview", "--check", "--embedder", "nope"])
                .arg(&ok)
        ),
        2,
        "an unknown embedder"
    );
    for d in [ok, failing, broken] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// A golden question that expects another repo's entry: skipped (and said
/// so) when that repo isn't loaded, run when it is.
#[test]
fn preview_check_skips_expects_in_repos_not_loaded() {
    let app = tmp("xrepo-app");
    write(
        &app,
        ".firelight/repo.yaml",
        "format: 1\nid: app-repo\nname: App\nsummary: App.\n",
    );
    write(
        &app,
        ".firelight/golden.yaml",
        "format: 1\nquestions:\n  - query: deploy the app\n    expect: [infra-repo/deploy]\n",
    );
    let infra = tmp("xrepo-infra");
    repo(&infra, "format: 1\nquestions: []\n");

    let out = bin()
        .args(["preview", "--check", "--embedder", "off"])
        .arg(&app)
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(
        text.contains("- app-repo: \"deploy the app\": skipped (infra-repo/deploy: repo infra-repo not loaded)"),
        "{text}"
    );
    assert!(
        text.contains("1 expect(s) skipped: repo(s) not loaded here: infra-repo."),
        "{text}"
    );
    assert!(text.contains("0 passed, 0 failed, 1 skipped"), "{text}");

    let out = bin()
        .args(["preview", "--check", "--embedder", "off"])
        .arg(&app)
        .arg(&infra)
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("1 passed, 0 failed, 0 skipped"), "{text}");
    assert!(!text.contains("not loaded"), "{text}");
    for d in [app, infra] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn skills_install_project_links_claude() {
    let d = tmp("skills");
    let mut init = Command::new("git");
    init.args(["init", "-q"]).current_dir(&d);
    no_hook_git(&mut init);
    let ok = init.status().is_ok_and(|s| s.success());
    assert!(ok);
    let _ = std::fs::create_dir_all(d.join("sub"));
    // From a subfolder: it still lands at the repo's top.
    assert_eq!(
        code(
            bin()
                .args(["skills", "install", "--project"])
                .current_dir(d.join("sub"))
        ),
        0
    );
    assert!(d.join(".agents/skills/firelight/SKILL.md").is_file());
    let link = std::fs::read_link(d.join(".claude/skills/firelight")).unwrap_or_default();
    assert_eq!(link, PathBuf::from("../../.agents/skills/firelight"));
    assert!(
        d.join(".claude/skills/firelight/SKILL.md").is_file(),
        "the link resolves"
    );
    // Again: nothing changes.
    assert_eq!(
        code(
            bin()
                .args(["skills", "install", "--project"])
                .current_dir(&d)
        ),
        0
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Every message in the self-hosting guide's troubleshooting table is one the
/// code really prints (§11.2.10).
#[test]
fn troubleshooting_messages_exist() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let guide = std::fs::read_to_string(root.join("docs/self-hosting.md")).unwrap_or_default();
    let mut code = String::new();
    for f in [
        "crates/firelight/src/config.rs",
        "crates/firelight/src/main.rs",
        "crates/firelight/src/preview.rs",
        "crates/firelight/src/search.rs",
    ] {
        code.push_str(&std::fs::read_to_string(root.join(f)).unwrap_or_default());
    }
    let table = guide
        .split("| Message (as printed) | Fix |")
        .nth(1)
        .unwrap_or("");
    let mut n = 0;
    for line in table.lines().skip(2).take_while(|l| l.starts_with('|')) {
        let Some(msg) = line.split('`').nth(1) else {
            continue;
        };
        if line.starts_with("| 421") {
            continue;
        }
        n += 1;
        assert!(
            code.contains(msg),
            "the guide quotes {msg:?}, which the code never prints"
        );
    }
    assert!(n >= 10, "the table should list the refusals ({n})");
}

/// The self-hosting guide is mirrored as the self-docs entry
/// `firelight/self-hosting` (§11.2), so search and MCP find it. Regenerate
/// with UPDATE_SNAPSHOTS=1.
#[test]
fn self_hosting_is_mirrored_in_the_self_docs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let guide = std::fs::read_to_string(root.join("docs/self-hosting.md")).unwrap_or_default();
    let body = guide.split_once('\n').map_or("", |x| x.1).trim_start();
    let want = format!(
        "---\nformat: 1\ntitle: Run Firelight yourself\nsummary: \"The full self-hosting guide: Docker, Compose with TLS, Helm, plain Kubernetes, connecting repos, webhooks, auth proxies, models, operations and troubleshooting.\"\naliases: [self-hosting, self host, deploy, docker compose, helm, kubernetes, troubleshooting]\ntags: [operations]\nrelated: [run-firelight]\n---\n<!-- Generated from docs/self-hosting.md; edit that file. -->\n\n{body}"
    );
    let p = root.join(".firelight/guides/self-hosting.md");
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        let _ = std::fs::write(&p, &want);
    }
    let got = std::fs::read_to_string(&p).unwrap_or_default();
    assert!(
        got == want,
        ".firelight/guides/self-hosting.md is stale. Run: UPDATE_SNAPSHOTS=1 cargo test -p firelight --test cli"
    );
}
