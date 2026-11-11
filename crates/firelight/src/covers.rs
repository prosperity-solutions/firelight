//! `firelight covers --base <ref> [DIR]` (§9.5, §12.1): in a PR, the entries
//! whose `covers` paths changed while the entry itself didn't. Git history
//! only (no file contents). Warns by default; `--strict` fails unless every
//! flagged entry was updated or consciously acknowledged, with a
//! `Firelight-Covers: checked [<ref>...]` commit trailer or the
//! `firelight-covers-checked` PR label (`--label-checked`).
//!
//! `firelight covers --report [DIR]`: which top-level paths no entry covers.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use firelight_format::{Limits, Repo, parse_repo};

/// The commit trailer that acknowledges flagged entries.
pub const TRAILER: &str = "Firelight-Covers";
/// The PR label that acknowledges every flagged entry (read by the Action).
pub const LABEL: &str = "firelight-covers-checked";

pub struct Args {
    pub base: Option<String>,
    pub dir: Option<PathBuf>,
    pub strict: bool,
    pub json: bool,
    pub report: bool,
    pub label_checked: bool,
}

/// An entry whose covered paths changed in this PR while the entry didn't.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Finding {
    /// `repo/entry`.
    pub entry: String,
    /// The entry's file, relative to the repo root.
    pub path: String,
    /// Covered paths that changed in this PR.
    pub changed: Vec<String>,
    /// Who acknowledged it: a commit SHA or `label`. `None` = still stale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_by: Option<String>,
    /// What to do, for a human or an agent. Only on stale entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// A changed file no entry covers, inside a folder other entries cover.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Gap {
    pub path: String,
    /// The covered folder it sits in.
    pub folder: String,
}

#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct Outcome {
    /// Not updated and not acknowledged: strict mode fails on these.
    pub stale: Vec<Finding>,
    /// Not updated, but acknowledged as still correct.
    pub checked: Vec<Finding>,
    pub gaps: Vec<Gap>,
    /// Trailers that couldn't be used (unknown refs, a bad value).
    pub warnings: Vec<String>,
}

/// One top-level path in `--report`.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct ReportRow {
    /// `dir/` for a folder, the name for a file at the root.
    pub path: String,
    pub files: usize,
    pub covered: usize,
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

fn nul_list(s: &str) -> Vec<String> {
    s.split('\0')
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn pathspecs(specs: &[String]) -> Vec<String> {
    specs.iter().map(|p| format!(":(glob,top){p}")).collect()
}

/// Paths changed between the merge base of `base` and `HEAD`, limited to
/// `specs` (git pathspecs; none = every path).
fn changed(dir: &Path, base: &str, specs: &[String]) -> Result<Vec<String>, String> {
    let range = format!("{base}...HEAD");
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "diff",
        "--name-only",
        "--no-renames",
        "--no-relative",
        "-z",
        &range,
        "--",
    ];
    let specs = pathspecs(specs);
    args.extend(specs.iter().map(String::as_str));
    Ok(nul_list(&git(dir, &args)?))
}

/// Tracked files (the index; `HEAD` in a clean checkout), limited to
/// `specs` (none = every file). `ls-tree` has no glob pathspecs.
fn tracked(dir: &Path, specs: &[String]) -> Result<Vec<String>, String> {
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "ls-files",
        "-z",
        "--cached",
        "--",
    ];
    let specs = pathspecs(specs);
    args.extend(specs.iter().map(String::as_str));
    Ok(nul_list(&git(dir, &args)?))
}

/// The repo's `.firelight/`, parsed. `dir` must be the repo's top folder.
fn load(dir: &Path) -> Result<Repo, String> {
    // Paths in .firelight/ are relative to the repo root: DIR must be it.
    if !git(dir, &["rev-parse", "--show-prefix"])?.trim().is_empty() {
        return Err(format!(
            "{} is inside a git repo, not its top folder; run covers on the repo root",
            dir.display()
        ));
    }
    let limits = Limits::default();
    let files = firelight_source::read_firelight_dir(dir, &limits).map_err(|e| e.to_string())?;
    let (repo, diags) = parse_repo(&files, None, &limits);
    repo.ok_or_else(|| {
        format!(
            "{}/.firelight doesn't parse; run firelight check first ({} problems)",
            dir.display(),
            diags.len()
        )
    })
}

fn check_base(dir: &Path, base: &str) -> Result<(), String> {
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
    // A shallow clone can have the base but no shared history with HEAD.
    git(dir, &["merge-base", base, "HEAD"]).map_err(|_| {
        format!(
            "no shared history between {base:?} and HEAD (a shallow clone?); check out with fetch-depth: 0"
        )
    })?;
    Ok(())
}

/// What the PR's commits acknowledge.
#[derive(Debug, Default, PartialEq)]
struct Acks {
    /// `Firelight-Covers: checked`: every flagged entry, by this commit.
    all: Option<String>,
    /// Entry id → the commit that named it.
    named: BTreeMap<String, String>,
    warnings: Vec<String>,
}

/// Reads `Firelight-Covers` trailers from every commit in `base..HEAD`,
/// oldest first (so the first commit to acknowledge an entry is named).
fn acks(dir: &Path, base: &str, repo: &Repo) -> Result<Acks, String> {
    let range = format!("{base}..HEAD");
    let fmt = format!("--format=%H%x1f%(trailers:key={TRAILER},valueonly,separator=%x1d)%x1e");
    let log = git(dir, &["log", "--reverse", &fmt, &range, "--"])?;
    let ids: BTreeSet<&str> = repo.entries.iter().map(|e| e.id.as_str()).collect();
    let mut a = Acks::default();
    for rec in log.split('\x1e') {
        let rec = rec.trim_matches(|c: char| c == '\n' || c == '\r');
        let Some((sha, values)) = rec.split_once('\x1f') else {
            continue;
        };
        let short = &sha[..sha.len().min(12)];
        for value in values
            .split('\x1d')
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let mut words = value
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|w| !w.is_empty());
            if !words
                .next()
                .is_some_and(|w| w.eq_ignore_ascii_case("checked"))
            {
                a.warnings.push(format!(
                    "commit {short}: `{TRAILER}: {value}` ignored; the value must start with `checked`"
                ));
                continue;
            }
            let refs: Vec<&str> = words.collect();
            if refs.is_empty() {
                a.all.get_or_insert_with(|| short.to_string());
                continue;
            }
            for r in refs {
                let id = match r.split_once('/') {
                    Some((rp, id)) if rp == repo.id => id,
                    Some(_) => "",
                    None => r,
                };
                if ids.contains(id) {
                    a.named
                        .entry(id.to_string())
                        .or_insert_with(|| short.to_string());
                } else {
                    a.warnings.push(format!(
                        "commit {short}: `{TRAILER}` names {r}, which isn't an entry of {}; ignored",
                        repo.id
                    ));
                }
            }
        }
    }
    Ok(a)
}

fn next_step(repo: &str, id: &str, path: &str) -> String {
    format!(
        "update {path} so it matches the change; if it is still correct, add the line \
         `{TRAILER}: checked {repo}/{id}` to the trailer block (last paragraph) of a commit message in this PR"
    )
}

/// Every parent folder of `p` except the root, deepest first.
fn parents(p: &str) -> impl Iterator<Item = &str> {
    p.match_indices('/').rev().map(move |(i, _)| &p[..=i])
}

/// The PR check: flagged entries split into stale and checked, plus gaps.
pub fn find(dir: &Path, base: &str, label_checked: bool) -> Result<Outcome, String> {
    if base.starts_with('-') {
        return Err(format!("--base {base:?} is not a ref"));
    }
    let repo = load(dir)?;
    check_base(dir, base)?;
    let acks = acks(dir, base, &repo)?;
    let mut out = Outcome {
        warnings: acks.warnings.clone(),
        ..Outcome::default()
    };
    let mut covered_changed = BTreeSet::new();
    let mut all_specs = Vec::new();
    for e in repo.entries.iter().filter(|e| !e.covers.is_empty()) {
        all_specs.extend(e.covers.iter().cloned());
        let covered = changed(dir, base, &e.covers)?;
        if covered.is_empty() {
            continue;
        }
        covered_changed.extend(covered.iter().cloned());
        if !changed(dir, base, std::slice::from_ref(&e.path))?.is_empty() {
            continue; // updated in this PR
        }
        let checked_by = acks
            .named
            .get(&e.id)
            .or(acks.all.as_ref())
            .cloned()
            .or_else(|| label_checked.then(|| "label".to_string()));
        let f = Finding {
            entry: format!("{}/{}", repo.id, e.id),
            path: e.path.clone(),
            changed: covered,
            next: checked_by
                .is_none()
                .then(|| next_step(&repo.id, &e.id, &e.path)),
            checked_by,
        };
        if f.checked_by.is_some() {
            out.checked.push(f);
        } else {
            out.stale.push(f);
        }
    }
    // Gaps: changed files no entry covers, in folders that entries do cover.
    if !all_specs.is_empty() {
        let mut folders = BTreeSet::new();
        for f in tracked(dir, &all_specs)? {
            folders.extend(parents(&f).map(str::to_string));
        }
        for f in changed(dir, base, &[])? {
            if f.starts_with(".firelight/") || covered_changed.contains(&f) {
                continue;
            }
            let folder = parents(&f)
                .find(|d| folders.contains(*d))
                .map(str::to_string);
            if let Some(folder) = folder {
                out.gaps.push(Gap { folder, path: f });
            }
        }
    }
    Ok(out)
}

/// `--report`: per top-level path, how many tracked files an entry covers.
pub fn report(dir: &Path) -> Result<Vec<ReportRow>, String> {
    let repo = load(dir)?;
    let specs: Vec<String> = repo
        .entries
        .iter()
        .flat_map(|e| e.covers.iter().cloned())
        .collect();
    let covered: BTreeSet<String> = if specs.is_empty() {
        BTreeSet::new()
    } else {
        tracked(dir, &specs)?.into_iter().collect()
    };
    let mut rows: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for f in tracked(dir, &[])? {
        let top = match f.split_once('/') {
            Some((d, _)) => format!("{d}/"),
            None => f.clone(),
        };
        if top == ".firelight/" {
            continue;
        }
        let r = rows.entry(top).or_default();
        r.0 += 1;
        if covered.contains(&f) {
            r.1 += 1;
        }
    }
    Ok(rows
        .into_iter()
        .map(|(path, (files, covered))| ReportRow {
            path,
            files,
            covered,
        })
        .collect())
}

fn print_report(rows: &[ReportRow], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(rows).unwrap_or_default());
        return;
    }
    println!("firelight covers --report: how much of each top-level path an entry covers");
    let width = rows.iter().map(|r| r.path.len()).max().unwrap_or(0);
    for want in ["not covered", "partly", "covered"] {
        for r in rows {
            let state = match r.covered {
                0 => "not covered",
                c if c < r.files => "partly",
                _ => "covered",
            };
            if state == want {
                println!(
                    "  {state:<11}  {:<width$}  {} of {} files",
                    r.path, r.covered, r.files
                );
            }
        }
    }
    println!(
        "Add `covers: [<path>/**]` to the entry that describes a path, so a change there flags it."
    );
}

fn print_outcome(o: &Outcome, strict: bool) {
    let level = if strict { "error" } else { "warning" };
    for f in &o.stale {
        println!("{level}: {} may be stale", f.entry);
        println!("  entry:   {}", f.path);
        println!("  changed: {}", f.changed.join(", "));
        if let Some(n) = &f.next {
            println!("  next:    {n}");
        }
    }
    for f in &o.checked {
        let by = f.checked_by.as_deref().unwrap_or("");
        let by = if by == "label" {
            format!("the {LABEL} label")
        } else {
            format!("{TRAILER} in commit {by}")
        };
        println!(
            "checked: {} ({}) still correct, by {by}; changed: {}",
            f.entry,
            f.path,
            f.changed.join(", ")
        );
    }
    for w in &o.warnings {
        println!("warning: {w}");
    }
    for g in &o.gaps {
        println!(
            "warning: no entry covers {} (other files in {} are covered); add it to the covers of the entry that describes it",
            g.path, g.folder
        );
    }
    if o.stale.is_empty() {
        println!(
            "firelight covers: every entry whose covered paths changed was updated{}.",
            if o.checked.is_empty() {
                ""
            } else {
                " or checked"
            }
        );
    } else {
        println!(
            "firelight covers: {} entr{} may be stale: update {}, or acknowledge {} with `{TRAILER}: checked <ref>` in a commit message.",
            o.stale.len(),
            if o.stale.len() == 1 { "y" } else { "ies" },
            if o.stale.len() == 1 { "it" } else { "them" },
            if o.stale.len() == 1 { "it" } else { "each" },
        );
    }
}

pub fn run(a: Args) -> ExitCode {
    let dir = a.dir.unwrap_or_else(|| PathBuf::from("."));
    if a.report {
        return match report(&dir) {
            Err(e) => {
                eprintln!("firelight covers: {e}");
                ExitCode::from(2)
            }
            Ok(rows) => {
                print_report(&rows, a.json);
                ExitCode::SUCCESS
            }
        };
    }
    let Some(base) = a.base else {
        eprintln!(
            "firelight covers: pass --base <ref> (the PR's base, e.g. origin/main) or --report"
        );
        return ExitCode::from(2);
    };
    match find(&dir, &base, a.label_checked) {
        Err(e) => {
            eprintln!("firelight covers: {e}");
            ExitCode::from(2)
        }
        Ok(o) => {
            if a.json {
                let v = serde_json::json!({
                    "ok": !a.strict || o.stale.is_empty(),
                    "strict": a.strict,
                    "stale": o.stale,
                    "checked": o.checked,
                    "gaps": o.gaps,
                    "warnings": o.warnings,
                });
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            } else {
                print_outcome(&o, a.strict);
            }
            if a.strict && !o.stale.is_empty() {
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

    const ACCESS: &str = "---\nformat: 1\ntitle: Access\nsummary: Who may do what.\ncovers: [terraform/access/**]\n---\nBody.\n";
    const REQUEST: &str = "---\nformat: 1\ntitle: Request access\nsummary: How to get admin.\ncovers: [terraform/access/**]\n---\n1. Ask.\n";

    /// A repo on `main` with two entries covering terraform/access/, and a
    /// `pr` branch checked out.
    fn repo(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fl-covers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::create_dir_all(&d);
        sh(&d, &["init", "-q", "-b", "main"]);
        write(
            &d,
            ".firelight/repo.yaml",
            "format: 1\nid: infra-repo\nname: Infra\nsummary: Infra.\n",
        );
        write(&d, ".firelight/concepts/access.md", ACCESS);
        write(&d, ".firelight/guides/request-access.md", REQUEST);
        write(&d, "terraform/access/main.tf", "a\n");
        write(&d, "terraform/billing/main.tf", "a\n");
        write(&d, "other/x.txt", "a\n");
        write(&d, "Makefile", "a\n");
        sh(&d, &["add", "-A"]);
        sh(&d, &["commit", "-q", "-m", "base"]);
        sh(&d, &["switch", "-q", "-c", "pr"]);
        d
    }

    fn entries(fs: &[Finding]) -> Vec<&str> {
        fs.iter().map(|f| f.entry.as_str()).collect()
    }

    #[test]
    fn stale_only_when_the_entry_didnt_change() {
        let d = repo("stale");
        write(&d, "other/x.txt", "b\n");
        sh(&d, &["commit", "-qam", "unrelated"]);
        assert_eq!(
            find(&d, "main", false).unwrap_or_default(),
            Outcome::default(),
            "unrelated change"
        );
        write(&d, "terraform/access/main.tf", "b\n");
        sh(&d, &["commit", "-qam", "covered"]);
        let o = find(&d, "main", false).unwrap_or_default();
        assert_eq!(
            entries(&o.stale),
            ["infra-repo/access", "infra-repo/request-access"]
        );
        assert_eq!(o.stale[0].changed, ["terraform/access/main.tf"]);
        assert_eq!(o.stale[0].path, ".firelight/concepts/access.md");
        let next = o.stale[0].next.as_deref().unwrap_or("");
        assert!(
            next.contains("update .firelight/concepts/access.md"),
            "{next}"
        );
        assert!(
            next.contains("`Firelight-Covers: checked infra-repo/access`"),
            "{next}"
        );
        write(
            &d,
            ".firelight/concepts/access.md",
            &ACCESS.replace("Who may do what.", "Who may do what, now."),
        );
        sh(&d, &["commit", "-qam", "docs too"]);
        let o = find(&d, "main", false).unwrap_or_default();
        assert_eq!(entries(&o.stale), ["infra-repo/request-access"], "updated");
        assert!(
            find(&d, "nope", false).is_err(),
            "a missing base is an error"
        );
        assert!(
            find(&d, "--output=x", false).is_err(),
            "no option injection"
        );
        let _ = std::fs::create_dir_all(d.join("sub"));
        assert!(
            find(&d.join("sub"), "main", false).is_err(),
            "a subfolder is refused"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_trailer_acknowledges_named_or_all_entries() {
        let d = repo("ack");
        write(&d, "terraform/access/main.tf", "b\n");
        sh(
            &d,
            &[
                "commit",
                "-qam",
                "bump\n\nFirelight-Covers: checked infra-repo/access, nope other-repo/x",
            ],
        );
        let sha = git(&d, &["rev-parse", "--short=12", "HEAD"]).unwrap_or_default();
        let o = find(&d, "main", false).unwrap_or_default();
        assert_eq!(entries(&o.checked), ["infra-repo/access"], "named by ref");
        assert_eq!(o.checked[0].checked_by.as_deref(), Some(sha.trim()));
        assert_eq!(o.checked[0].next, None);
        assert_eq!(entries(&o.stale), ["infra-repo/request-access"]);
        assert_eq!(o.warnings.len(), 2, "unknown refs warn: {:?}", o.warnings);
        assert!(o.warnings[0].contains("names nope"), "{:?}", o.warnings);
        assert!(
            o.warnings[1].contains("names other-repo/x"),
            "{:?}",
            o.warnings
        );

        // A bare id works too; a value without `checked` is ignored.
        write(&d, "other/x.txt", "b\n");
        sh(
            &d,
            &[
                "commit",
                "-qam",
                "more\n\nFirelight-Covers: looks fine\nfirelight-covers: Checked request-access",
            ],
        );
        let o = find(&d, "main", false).unwrap_or_default();
        assert!(o.stale.is_empty(), "{:?}", o.stale);
        assert_eq!(o.checked.len(), 2);
        assert!(o.warnings.iter().any(|w| w.contains("looks fine")));

        // `checked` alone covers every flagged entry; the earliest commit is named.
        let d2 = repo("ack-all");
        write(&d2, "terraform/access/main.tf", "b\n");
        sh(
            &d2,
            &["commit", "-qam", "bump\n\nFirelight-Covers: checked"],
        );
        let sha = git(&d2, &["rev-parse", "--short=12", "HEAD"]).unwrap_or_default();
        write(&d2, "other/x.txt", "b\n");
        sh(
            &d2,
            &["commit", "-qam", "again\n\nFirelight-Covers: checked"],
        );
        let o = find(&d2, "main", false).unwrap_or_default();
        assert!(o.stale.is_empty());
        assert!(
            o.checked
                .iter()
                .all(|f| f.checked_by.as_deref() == Some(sha.trim()))
        );

        // Not in the trailer block (not the last paragraph): no acknowledgement.
        let d3 = repo("ack-body");
        write(&d3, "terraform/access/main.tf", "b\n");
        sh(
            &d3,
            &[
                "commit",
                "-qam",
                "bump\n\nFirelight-Covers: checked\n\nmore text after",
            ],
        );
        let o = find(&d3, "main", false).unwrap_or_default();
        assert_eq!(o.stale.len(), 2, "a trailer must be in the last paragraph");
        for x in [d, d2, d3] {
            let _ = std::fs::remove_dir_all(&x);
        }
    }

    #[test]
    fn the_label_acknowledges_everything() {
        let d = repo("label");
        write(&d, "terraform/access/main.tf", "b\n");
        sh(&d, &["commit", "-qam", "bump"]);
        let o = find(&d, "main", true).unwrap_or_default();
        assert!(o.stale.is_empty());
        assert_eq!(o.checked.len(), 2);
        assert!(
            o.checked
                .iter()
                .all(|f| f.checked_by.as_deref() == Some("label"))
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn gaps_are_uncovered_changes_in_covered_folders() {
        let d = repo("gaps");
        write(&d, "terraform/access/main.tf", "b\n");
        write(&d, "terraform/billing/main.tf", "b\n");
        write(&d, "terraform/versions.tf", "b\n");
        write(&d, "other/x.txt", "b\n");
        write(&d, "Makefile", "b\n");
        sh(&d, &["add", "-A"]);
        sh(&d, &["commit", "-qm", "many"]);
        let o = find(&d, "main", false).unwrap_or_default();
        assert_eq!(
            o.gaps,
            [
                Gap {
                    path: "terraform/billing/main.tf".into(),
                    folder: "terraform/".into()
                },
                Gap {
                    path: "terraform/versions.tf".into(),
                    folder: "terraform/".into()
                },
            ]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn report_lists_top_level_paths() {
        let d = repo("report");
        let rows = report(&d).unwrap_or_default();
        assert_eq!(
            rows,
            [
                ReportRow {
                    path: "Makefile".into(),
                    files: 1,
                    covered: 0
                },
                ReportRow {
                    path: "other/".into(),
                    files: 1,
                    covered: 0
                },
                ReportRow {
                    path: "terraform/".into(),
                    files: 2,
                    covered: 1
                },
            ]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn parents_are_deepest_first() {
        assert_eq!(parents("a/b/c.txt").collect::<Vec<_>>(), ["a/b/", "a/"]);
        assert_eq!(parents("c.txt").count(), 0);
    }
}
