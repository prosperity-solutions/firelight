//! `firelight check [DIR]…` (§12.1): validate `.firelight/` folders with the
//! format crate only. No network, no model, no database.

use std::path::PathBuf;
use std::process::ExitCode;

use firelight_format::catalog::{self, Strictness};
use firelight_format::{Diagnostic, Limits, Severity, parse_repo};

pub struct Args {
    pub dirs: Vec<PathBuf>,
    pub json: bool,
    pub strict: bool,
}

/// Every diagnostic, with the folder it belongs to. Paths are shown relative
/// to the current folder when the checked folder isn't `.`.
pub fn run(args: Args) -> ExitCode {
    let limits = Limits::default();
    let dirs = if args.dirs.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        args.dirs
    };
    let mut all: Vec<Diagnostic> = Vec::new();
    let mut repos = Vec::new();
    let mut prefixes = std::collections::BTreeMap::new();
    for dir in &dirs {
        let prefix = if dir == &PathBuf::from(".") {
            String::new()
        } else {
            format!("{}/", dir.display().to_string().trim_end_matches('/'))
        };
        let files = match firelight_source::read_firelight_dir(dir, &limits) {
            Ok(f) => f,
            Err(e) => {
                all.push(Diagnostic::error(
                    format!("{prefix}.firelight/"),
                    0,
                    0,
                    "folder",
                    e.to_string(),
                ));
                continue;
            }
        };
        let (repo, diags) = parse_repo(&files, None, &limits);
        all.extend(diags.into_iter().map(|mut d| {
            d.path = format!("{prefix}{}", d.path);
            d
        }));
        if let Some(r) = repo {
            // The first folder with an id keeps it; the catalog reports a
            // second one against that id.
            prefixes.entry(r.id.clone()).or_insert(prefix);
            repos.push(r);
        }
    }
    let strict = if args.strict {
        Strictness::Strict
    } else {
        Strictness::Lenient
    };
    let cat = catalog::build(repos, strict);
    for (repo, mut d) in cat.diagnostics {
        d.path = format!(
            "{}{}",
            prefixes.get(&repo).cloned().unwrap_or_default(),
            d.path
        );
        all.push(d);
    }
    firelight_format::diag::sort(&mut all);
    all.dedup();
    let errors = all.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = all.len() - errors;
    if args.json {
        let v: Vec<_> = all
            .iter()
            .map(|d| {
                serde_json::json!({
                    "path": d.path, "line": d.line, "col": d.col,
                    "severity": d.severity.as_str(), "code": d.code, "message": d.message,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({ "errors": errors, "warnings": warnings, "diagnostics": v })
        );
    } else {
        for d in &all {
            println!("{d}");
        }
        let n = cat.repos.len();
        let what = cat
            .repos
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        match (errors, warnings) {
            (0, 0) => println!(
                "✓ {n} repo{} checked ({what}): all good.",
                if n == 1 { "" } else { "s" }
            ),
            (0, w) => println!(
                "✓ {n} repo{} checked ({what}): no errors, {w} warning{}.",
                if n == 1 { "" } else { "s" },
                if w == 1 { "" } else { "s" }
            ),
            (e, w) => println!(
                "✗ {e} error{}, {w} warning{}.",
                if e == 1 { "" } else { "s" },
                if w == 1 { "" } else { "s" }
            ),
        }
    }
    if errors > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
