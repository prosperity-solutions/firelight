//! The agent guard against its fixtures (scripts/dev/testdata/guard/*.json).
//! The commands live in fixture files, not here, because the guard blocks
//! command text that merely mentions what it forbids. "@UUID@" in a fixture
//! stands for a real-looking ID built at runtime.

use harness::{TempDir, fake_id, hermetic, root};
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

fn git_repo(branch: &str) -> TempDir {
    let d = TempDir::new("guard-repo");
    for args in [
        vec!["init", "-q", "-b", branch],
        vec![
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "chore: init",
        ],
    ] {
        let out = hermetic(Command::new("git").args(&args).current_dir(d.path()))
            .output()
            .unwrap_or_else(|e| panic!("git: {e}"));
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    d
}

/// A stand-in for gh that answers the two calls the guard makes.
fn fake_gh(dir: &TempDir, gh: &Value) -> String {
    let mut s = String::from("#!/bin/sh\n");
    if let Some(login) = gh["login"].as_str() {
        s += &format!("if [ \"$1 $2\" = \"api user\" ]; then echo '{login}'; exit 0; fi\n");
        for (pr, author) in gh["authors"].as_object().into_iter().flatten() {
            let a = author.as_str().unwrap_or("");
            s += &format!("if [ \"$1 $2 $3\" = \"pr view {pr}\" ]; then echo '{a}'; exit 0; fi\n");
        }
    }
    s += "echo 'fake gh: unexpected call' >&2; exit 1\n";
    let p = dir.write("bin/gh", &s);
    let _ = Command::new("chmod").arg("755").arg(&p).status();
    p.to_string_lossy().into_owned()
}

#[test]
fn agent_guard_fixtures() {
    let guard = root().join("scripts/dev/agent-guard.sh");
    let mut files: Vec<_> = std::fs::read_dir(root().join("scripts/dev/testdata/guard"))
        .unwrap_or_else(|e| panic!("{e}"))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    assert!(files.len() > 50, "guard fixtures missing");
    let id = fake_id();
    let mut failures = vec![];
    for f in &files {
        let name = f
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let raw = std::fs::read_to_string(f)
            .unwrap_or_else(|e| panic!("{e}"))
            .replace("@UUID@", &id);
        let fx: Value = serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{name}: {e}"));
        let project = TempDir::new("guard-project");
        if fx["bootstrap_done"].as_bool() == Some(true) {
            project.write("notes/bootstrap.done", "");
        }
        let work = git_repo(fx["branch"].as_str().unwrap_or("main"));
        for (rel, body) in fx["files"].as_object().into_iter().flatten() {
            let p = work.path().join(rel);
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, body.as_str().unwrap_or(""));
        }
        let mut payload = fx["payload"].clone();
        payload["cwd"] = Value::String(work.path().to_string_lossy().into_owned());
        let ghdir = TempDir::new("guard-gh");
        let gh = fake_gh(&ghdir, &fx["gh"]);
        let mut child = hermetic(
            Command::new("sh")
                .arg(&guard)
                .current_dir(work.path())
                .env("CLAUDE_PROJECT_DIR", project.path())
                .env("FIRELIGHT_GUARD_GH", gh)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"));
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(payload.to_string().as_bytes());
        }
        let out = child.wait_with_output().unwrap_or_else(|e| panic!("{e}"));
        let stderr = String::from_utf8_lossy(&out.stderr);
        let got = match out.status.code() {
            Some(0) => "allow",
            Some(2) => "block",
            c => {
                failures.push(format!("{name}: guard crashed ({c:?}): {stderr}"));
                continue;
            }
        };
        let want = fx["expect"].as_str().unwrap_or("");
        if got != want {
            failures.push(format!("{name}: want {want}, got {got}: {stderr}"));
        } else if let Some(r) = fx["reason"].as_str().filter(|r| !r.is_empty())
            && !stderr.contains(r)
        {
            failures.push(format!(
                "{name}: blocked for another reason (want {r:?}): {stderr}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} guard fixture(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn settings_wire_the_guard_fail_closed() {
    let s: Value = serde_json::from_str(&harness::read(".claude/settings.json"))
        .unwrap_or_else(|e| panic!("{e}"));
    let pre = &s["hooks"]["PreToolUse"][0];
    assert_eq!(pre["matcher"], "Bash|Monitor");
    assert_eq!(pre["hooks"][0]["onFailure"], "block");
    assert!(
        pre["hooks"][0]["command"]
            .as_str()
            .unwrap_or("")
            .ends_with("scripts/dev/agent-guard.sh")
    );
    let deny: Vec<&str> = s["permissions"]["deny"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    for d in [
        "Bash(git push --force)",
        "Bash(gh pr merge *--admin*)",
        "Bash(gh secret *)",
        "Bash(gh repo edit*)",
        "Bash(git tag *)",
    ] {
        assert!(deny.contains(&d), "settings.json deny is missing {d}");
    }
}
