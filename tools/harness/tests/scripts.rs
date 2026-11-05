//! Scripts: executable bits, the public-ID check, the tool pin file.

use harness::{TempDir, fake_id, read, root};
use std::io::Write;
use std::process::{Command, Stdio};

/// Every tracked file with a #! line is executable in git: the workflow and
/// the hooks run them directly.
#[test]
fn scripts_are_executable() {
    let Ok(out) = Command::new("git")
        .args(["ls-files", "-s"])
        .current_dir(root())
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return; // not a git checkout, e.g. a source tarball
    }
    let mut bad = vec![];
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 4 {
            continue;
        }
        let Ok(bytes) = std::fs::read(root().join(f[3])) else {
            continue;
        };
        if bytes.starts_with(b"#!") && f[0] != "100755" {
            bad.push(format!("{} (git update-index --chmod=+x {})", f[3], f[3]));
        }
    }
    assert!(bad.is_empty(), "not executable in git: {bad:?}");
}

fn run_public_ids(input: &str, args: &[&str]) -> (i32, String) {
    let mut c = Command::new("sh")
        .arg(root().join("scripts/dev/public-ids.sh"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"));
    if let Some(mut s) = c.stdin.take() {
        let _ = s.write_all(input.as_bytes());
    }
    let out = c.wait_with_output().unwrap_or_else(|e| panic!("{e}"));
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn public_ids() {
    let id = fake_id();
    let link = format!("![s](https://github.com/user-attachments/assets/{id})\n");
    for (name, input, want) in [
        ("empty", String::new(), 0),
        (
            "clean",
            "no ids here\nShip-Stamp: v1 8dd0657d6ac71ec1\n".to_string(),
            0,
        ),
        (
            "all-zero example",
            "id: 00000000-0000-0000-0000-000000000000\n".into(),
            0,
        ),
        ("real id", format!("service {id}\n"), 1),
        ("upper case", format!("x {}", id.to_uppercase()), 1),
        ("attachment link", link.clone(), 0),
        (
            "link and a real id",
            format!("{} {id}\n", link.trim_end()),
            1,
        ),
    ] {
        let (code, out) = run_public_ids(&input, &[]);
        assert_eq!(code, want, "{name}");
        assert!(
            !out.to_lowercase().contains(&id),
            "{name}: the output repeats the ID"
        );
    }
}

#[test]
fn tool_pins_are_complete() {
    let text = read("tools/versions.toml");
    let mut section = String::new();
    let mut seen: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for l in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if let Some(s) = l.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_string();
            continue;
        }
        let k = l.split('=').next().unwrap_or("").trim().to_string();
        seen.entry(section.clone()).or_default().push(k);
    }
    for (tool, keys) in &seen {
        let has = |k: &str| keys.iter().any(|x| x == k);
        assert!(
            has("kind") && has("version"),
            "[{tool}] needs kind and version"
        );
        if keys.iter().any(|k| k == "url") {
            for p in ["linux_amd64", "linux_arm64", "darwin_arm64", "darwin_amd64"] {
                assert!(
                    has(&format!("asset_{p}")) && has(&format!("sha256_{p}")),
                    "[{tool}] needs asset/sha256 for {p}"
                );
            }
            assert!(
                has("released") && has("repo"),
                "[{tool}] needs released and repo (tools/depage)"
            );
        }
    }
    // Every tool the CI file runs through scripts/dev/t is pinned.
    let ci = read(".github/workflows/ci.yml");
    for l in ci.lines() {
        let Some(chunk) = l.trim_start().strip_prefix("run: scripts/dev/t ") else {
            continue;
        };
        let name = chunk.split_whitespace().next().unwrap_or("");
        assert!(
            seen.contains_key(name),
            "ci.yml runs scripts/dev/t {name}, which tools/versions.toml doesn't pin"
        );
    }
}

#[test]
fn fresh_deps_lines_are_well_formed() {
    for (i, l) in read(".github/fresh-deps.txt").lines().enumerate() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = l.split_whitespace().collect();
        assert!(
            f.len() >= 4,
            "fresh-deps.txt:{}: <name> <version> <expires> <reason>",
            i + 1
        );
    }
}

#[test]
fn session_start_hook_never_fails() {
    let d = TempDir::new("session");
    let out = Command::new("sh")
        .arg(root().join("scripts/dev/session-start.sh"))
        .env("CLAUDE_PROJECT_DIR", d.path())
        .env("PATH", "/usr/bin:/bin") // no gh: the offline path
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(out.status.success());
    d.write(
        "notes/progress.md",
        "## State\nNext action: x\n## Milestones\n- M1\n## Blocked\nnothing\n",
    );
    let out = Command::new("sh")
        .arg(root().join("scripts/dev/session-start.sh"))
        .env("CLAUDE_PROJECT_DIR", d.path())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("Next action: x") && s.contains("nothing") && !s.contains("- M1"),
        "{s}"
    );
}
