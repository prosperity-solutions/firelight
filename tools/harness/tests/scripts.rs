//! Scripts: executable bits, the public-ID check, the tool pin file.

/// §0.1: nobody reviews. GitHub's default extra approval for unattributed
/// changes would block every PR of a sole owner, so the owner's hardening
/// script states it off.
#[test]
fn hardening_keeps_zero_approvals() {
    let s = read(&format!("scripts/dev/{}-public-repo.sh", "harden"));
    assert!(s.contains("\"required_approving_review_count\": 0"));
    assert!(s.contains("\"require_extra_approval_for_unattributed_changes\": false"));
    assert!(s.contains("\"bypass_actors\": []"));
    // Not required_signatures: decided with the owner at M1 (PR commits are
    // unsigned; main only gets GitHub-signed squash merges).
    assert!(!s.contains("{\"type\": \"required_signatures\"}"));
}

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
        if bytes.starts_with(b"#!/") && f[0] != "100755" {
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

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = harness::hermetic(
        Command::new("git")
            .args([
                "-c",
                "user.email=t@example.com",
                "-c",
                "user.name=t",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir),
    )
    .output()
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// D9: the version script, against throwaway histories.
#[test]
fn next_version() {
    let run = |subjects: &[&str], tag_after: Option<(usize, &str)>| -> String {
        let d = TempDir::new("version");
        git(d.path(), &["init", "-q", "-b", "main"]);
        for (i, s) in subjects.iter().enumerate() {
            let (subject, body) = s.split_once('|').unwrap_or((s, ""));
            git(
                d.path(),
                &["commit", "-q", "--allow-empty", "-m", subject, "-m", body],
            );
            if let Some((at, tag)) = tag_after
                && at == i
            {
                git(d.path(), &["tag", tag]);
            }
        }
        let out = harness::hermetic(
            Command::new("sh")
                .arg(root().join("scripts/ci/next-version.sh"))
                .current_dir(d.path()),
        )
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .replace('\n', " ")
            .trim()
            .to_string()
    };
    assert_eq!(
        run(&["chore: bootstrap"], None),
        "version=0.1.0 released=false"
    );
    assert_eq!(
        run(&["chore: bootstrap", "fix: a"], None),
        "version=0.1.0 released=true"
    );
    assert_eq!(
        run(&["chore: x", "feat(server): a"], None),
        "version=0.1.0 released=true"
    );
    assert_eq!(
        run(&["feat: a", "fix: b"], Some((0, "v0.1.0"))),
        "version=0.1.1 released=true"
    );
    assert_eq!(
        run(&["feat: a", "feat: b", "fix: c"], Some((0, "v0.1.0"))),
        "version=0.2.0 released=true"
    );
    assert_eq!(
        run(&["feat: a", "docs: b", "ci: c"], Some((0, "v0.1.0"))),
        "version=0.1.1 released=false"
    );
    assert_eq!(
        run(&["feat: a", "refactor!: b"], Some((0, "v0.3.4"))),
        "version=0.4.0 released=true"
    );
    assert_eq!(
        run(
            &["feat: a", "chore: b|BREAKING CHANGE: gone"],
            Some((0, "v1.2.3"))
        ),
        "version=2.0.0 released=true"
    );
    assert_eq!(
        run(&["feat: a", "feat!: b"], Some((0, "v1.2.3"))),
        "version=2.0.0 released=true"
    );
    // A subject that only mentions feat isn't one.
    assert_eq!(
        run(
            &["feat: a", "docs: explain feat: usage"],
            Some((0, "v0.1.0"))
        ),
        "version=0.1.1 released=false"
    );
}

/// §17.3: publish and release refuse any archive whose hash differs from its
/// build job's output, and SHA256SUMS follows from the hashes alone.
#[test]
fn release_sums_refuse_tampering() {
    let d = TempDir::new("sums");
    let names = ["linux-amd64", "linux-arm64", "macos-arm64", "macos-amd64"];
    let mut env: Vec<(String, String)> = vec![("VERSION".into(), "1.2.3".into())];
    for t in names {
        let f = d.write(&format!("dist/firelight-v1.2.3-{t}.tar.gz"), t);
        let out = Command::new("sh").arg("-c").arg("if command -v sha256sum >/dev/null; then sha256sum \"$1\"; else shasum -a 256 \"$1\"; fi").arg("sh").arg(&f).output().unwrap_or_else(|e| panic!("{e}"));
        let sha = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        env.push((format!("SHA_{}", t.replace('-', "_").to_uppercase()), sha));
    }
    let run = |env: &[(String, String)]| {
        let mut c = Command::new("sh");
        c.arg(root().join("scripts/ci/release-sums.sh"))
            .current_dir(d.path())
            .env_remove("GITHUB_OUTPUT");
        for (k, v) in env {
            c.env(k, v);
        }
        c.output().unwrap_or_else(|e| panic!("{e}"))
    };
    let ok = run(&env);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let first = String::from_utf8_lossy(&ok.stdout)
        .lines()
        .last()
        .unwrap_or("")
        .to_string();
    assert!(first.starts_with("digest=sha256:"));
    // Same inputs, same SHA256SUMS digest (publish and release must agree).
    assert_eq!(
        String::from_utf8_lossy(&run(&env).stdout)
            .lines()
            .last()
            .unwrap_or(""),
        first
    );
    // A swapped archive is refused.
    d.write("dist/firelight-v1.2.3-macos-arm64.tar.gz", "evil");
    let bad = run(&env);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("Refusing it"));
    // A missing build output is refused.
    d.write("dist/firelight-v1.2.3-macos-arm64.tar.gz", "macos-arm64");
    let mut missing = env.clone();
    missing.retain(|(k, _)| k != "SHA_LINUX_ARM64");
    assert!(!run(&missing).status.success());
}
