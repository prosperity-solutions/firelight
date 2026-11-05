//! `firelight check` as a user runs it: exit codes, output shape, --json,
//! --strict (§12.1).

use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> (i32, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO_BIN_EXE_firelight"))
        .arg("check")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn the_acme_repos_pass_together() {
    let (code, out) = run(&["fixtures/acme-infra", "fixtures/acme-app"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("✓ 2 repos checked (acme-infra, acme-app): no errors, 2 warnings."),
        "{out}"
    );
}

#[test]
fn alone_cross_repo_refs_warn_and_strict_fails() {
    let (code, out) = run(&["fixtures/acme-app"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("warning: link acme-infra/argo-staging does not resolve"),
        "{out}"
    );
    let (code, out) = run(&["--strict", "fixtures/acme-app"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("error: link acme-infra/argo-staging does not resolve"),
        "{out}"
    );
}

#[test]
fn errors_are_path_line_col_and_exit_1() {
    let (code, out) = run(&["fixtures/invalid/ids-and-refs"]);
    assert_eq!(code, 1);
    assert!(
        out.lines()
            .any(|l| l
                .starts_with("fixtures/invalid/ids-and-refs/.firelight/links.yaml:3:9: error: ")),
        "{out}"
    );
}

#[test]
fn json() {
    let (code, out) = run(&["--json", "fixtures/invalid/golden"]);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    assert_eq!(v["errors"], 5);
    let d = &v["diagnostics"][0];
    for k in ["path", "line", "col", "severity", "code", "message"] {
        assert!(!d[k].is_null(), "{k} missing in {d}");
    }
}

#[test]
fn a_folder_without_firelight() {
    let (code, out) = run(&["crates"]);
    assert_eq!(code, 1);
    assert!(out.contains("crates has no .firelight/ folder"), "{out}");
}
