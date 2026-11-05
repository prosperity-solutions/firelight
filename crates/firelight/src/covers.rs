//! `firelight covers --base <ref> [DIR]` (§9.5, §12.1): in a PR, the entries
//! whose `covers` paths changed while the entry itself didn't. Git history
//! only (no file contents). Warns by default; `--strict` fails.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use firelight_format::{Limits, parse_repo};

pub struct Args {
    pub base: String,
    pub dir: Option<PathBuf>,
    pub strict: bool,
    pub json: bool,
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Finding {
    pub entry: String,
    pub path: String,
    /// Covered paths that changed in this PR.
    pub changed: Vec<String>,
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        // Inside a git hook these point at the hook's repo, not `dir`.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Paths changed between the merge base of `base` and `HEAD`, limited to
/// `specs` (git pathspecs).
fn changed(dir: &Path, base: &str, specs: &[String]) -> Result<Vec<String>, String> {
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "diff",
        "--name-only",
        "--no-renames",
        "--no-relative",
        "-z",
    ];
    let range = format!("{base}...HEAD");
    args.push(&range);
    args.push("--");
    let specs: Vec<String> = specs.iter().map(|p| format!(":(glob,top){p}")).collect();
    args.extend(specs.iter().map(String::as_str));
    Ok(git(dir, &args)?
        .split('\0')
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn find(dir: &Path, base: &str) -> Result<Vec<Finding>, String> {
    if base.starts_with('-') {
        return Err(format!("--base {base:?} is not a ref"));
    }
    // The base must exist (a shallow clone without it can't answer).
    git(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{base}^{{commit}}"),
        ],
    )
    .map_err(|_| {
        format!(
            "the base {base:?} isn't in this clone; fetch it (actions/checkout with fetch-depth: 0)"
        )
    })?;
    // Paths in .firelight/ are relative to the repo root: DIR must be it.
    if !git(dir, &["rev-parse", "--show-prefix"])?.trim().is_empty() {
        return Err(format!(
            "{} is inside a git repo, not its top folder; run covers on the repo root",
            dir.display()
        ));
    }
    // A shallow clone can have the base but no shared history with HEAD.
    git(dir, &["merge-base", base, "HEAD"]).map_err(|_| {
        format!(
            "no shared history between {base:?} and HEAD (a shallow clone?); check out with fetch-depth: 0"
        )
    })?;
    let limits = Limits::default();
    let files = firelight_source::read_firelight_dir(dir, &limits).map_err(|e| e.to_string())?;
    let (repo, diags) = parse_repo(&files, None, &limits);
    let repo = repo.ok_or_else(|| {
        format!(
            "{}/.firelight doesn't parse; run firelight check first ({} problems)",
            dir.display(),
            diags.len()
        )
    })?;
    let mut out = Vec::new();
    for e in repo.entries.iter().filter(|e| !e.covers.is_empty()) {
        let covered = changed(dir, base, &e.covers)?;
        if covered.is_empty() {
            continue;
        }
        let entry_changed = !changed(dir, base, std::slice::from_ref(&e.path))?.is_empty();
        if !entry_changed {
            out.push(Finding {
                entry: format!("{}/{}", repo.id, e.id),
                path: e.path.clone(),
                changed: covered,
            });
        }
    }
    Ok(out)
}

pub fn run(a: Args) -> ExitCode {
    let dir = a.dir.unwrap_or_else(|| PathBuf::from("."));
    match find(&dir, &a.base) {
        Err(e) => {
            eprintln!("firelight covers: {e}");
            ExitCode::from(2)
        }
        Ok(f) => {
            if a.json {
                println!("{}", serde_json::to_string_pretty(&f).unwrap_or_default());
            } else if f.is_empty() {
                println!(
                    "firelight covers: every entry whose covered paths changed was updated too."
                );
            } else {
                for x in &f {
                    println!(
                        "{}: {} may be stale: {} changed, the entry didn't",
                        if a.strict { "error" } else { "warning" },
                        x.path,
                        x.changed.join(", ")
                    );
                }
            }
            if a.strict && !f.is_empty() {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@acme.example")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@acme.example")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env("HOME", dir)
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(ok, "git {args:?}");
    }

    fn write(dir: &Path, p: &str, s: &str) {
        let p = dir.join(p);
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(dir));
        let _ = std::fs::write(p, s);
    }

    #[test]
    fn stale_only_when_the_entry_didnt_change() {
        let d = std::env::temp_dir().join(format!("fl-covers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::create_dir_all(&d);
        sh(&d, &["init", "-q", "-b", "main"]);
        write(
            &d,
            ".firelight/repo.yaml",
            "format: 1\nid: infra-repo\nname: Infra\nsummary: Infra.\n",
        );
        write(
            &d,
            ".firelight/concepts/access.md",
            "---\nformat: 1\ntitle: Access\nsummary: Who may do what.\ncovers: [terraform/access/**]\n---\nBody.\n",
        );
        write(&d, "terraform/access/main.tf", "a\n");
        write(&d, "other/x.txt", "a\n");
        sh(&d, &["add", "-A"]);
        sh(&d, &["commit", "-q", "-m", "base"]);
        sh(&d, &["switch", "-q", "-c", "pr"]);
        write(&d, "other/x.txt", "b\n");
        sh(&d, &["commit", "-qam", "unrelated"]);
        assert_eq!(
            find(&d, "main").unwrap_or_default(),
            vec![],
            "unrelated change"
        );
        write(&d, "terraform/access/main.tf", "b\n");
        sh(&d, &["commit", "-qam", "covered"]);
        let f = find(&d, "main").unwrap_or_default();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].changed, ["terraform/access/main.tf"]);
        write(
            &d,
            ".firelight/concepts/access.md",
            "---\nformat: 1\ntitle: Access\nsummary: Who may do what, now.\ncovers: [terraform/access/**]\n---\nBody.\n",
        );
        sh(&d, &["commit", "-qam", "docs too"]);
        assert_eq!(
            find(&d, "main").unwrap_or_default(),
            vec![],
            "the entry changed too"
        );
        assert!(find(&d, "nope").is_err(), "a missing base is an error");
        assert!(find(&d, "--output=x").is_err(), "no option injection");
        let _ = std::fs::create_dir_all(d.join("sub"));
        assert!(
            find(&d.join("sub"), "main").is_err(),
            "a subfolder is refused"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
