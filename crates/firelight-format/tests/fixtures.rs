//! The fixtures under `fixtures/` as snapshots (§13.1, "Format"): every
//! invalid case's diagnostics, and the merged page model of the acme repos.
//! Review changes with `cargo insta review` (or INSTA_UPDATE=always).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use firelight_format::catalog::{self, Catalog, PlacedKind, Strictness};
use firelight_format::repo::FileKind;
use firelight_format::{InputFile, Limits, Repo, parse_repo};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// A test-only reader (the crate itself does no I/O): symlinks come back
/// without contents, as `firelight-source` returns them.
fn read(dir: &Path) -> Vec<InputFile> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<InputFile>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let rel = p
                .strip_prefix(base)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            let m = std::fs::symlink_metadata(&p).unwrap_or_else(|e| panic!("{e}"));
            if m.file_type().is_symlink() {
                out.push(InputFile {
                    path: rel,
                    kind: FileKind::Symlink,
                    contents: None,
                    size: 0,
                });
            } else if m.is_dir() {
                walk(base, &p, out);
            } else {
                let b = std::fs::read(&p).unwrap_or_default();
                out.push(InputFile::regular(rel, b));
            }
        }
    }
    let base = dir.join(".firelight");
    let mut out = Vec::new();
    walk(&base, &base, &mut out);
    out
}

fn check(dirs: &[&str]) -> (Vec<Repo>, String) {
    let mut out = String::new();
    let mut repos = Vec::new();
    for d in dirs {
        let (repo, diags) = parse_repo(&read(&root().join(d)), None, &Limits::default());
        for x in diags {
            let _ = writeln!(out, "{d}/{x}");
        }
        repos.extend(repo);
    }
    (repos, out)
}

fn render(c: &Catalog) -> String {
    let mut s = String::new();
    for sp in &c.spaces {
        let _ = writeln!(
            s,
            "space {} {:?} (order {}, declared by [{}], repos [{}])",
            sp.id,
            sp.title,
            sp.order,
            sp.declared_by.join(", "),
            sp.repos.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        let row = |l: &[catalog::PlacedLink]| {
            l.iter()
                .map(|x| x.label.clone())
                .collect::<Vec<_>>()
                .join(" · ")
        };
        let _ = writeln!(s, "  top: {}", row(&sp.top));
        for i in &sp.islands {
            let _ = writeln!(
                s,
                "  island {} {:?} [{}] order {}",
                i.r,
                i.title,
                i.place.as_str(),
                i.order
            );
            match &i.kind {
                PlacedKind::Cards { cards } => {
                    for c in cards {
                        let _ = writeln!(
                            s,
                            "    card {} {:?} {:?} (from {})",
                            c.id, c.title, c.tone, c.repo
                        );
                        for sec in &c.sections {
                            if !sec.links.is_empty() {
                                let links = sec
                                    .links
                                    .iter()
                                    .map(|l| {
                                        format!(
                                            "{} <{}>{}",
                                            l.label,
                                            l.url,
                                            l.entry
                                                .as_ref()
                                                .map(|e| format!(" ?{e}"))
                                                .unwrap_or_default()
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                let _ = writeln!(
                                    s,
                                    "      {}: {links}",
                                    sec.name.as_deref().unwrap_or("-")
                                );
                            }
                        }
                    }
                }
                PlacedKind::Links { links } => {
                    let _ = writeln!(s, "    links: {}", row(links));
                }
                PlacedKind::Template(t) => {
                    let _ = writeln!(
                        s,
                        "    template inputs [{}] links [{}]",
                        t.inputs
                            .iter()
                            .map(|i| i.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        t.links
                            .iter()
                            .map(|l| l.url.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }
        let _ = writeln!(s, "  footer: {}", row(&sp.footer));
        let _ = writeln!(
            s,
            "  entries: {}",
            sp.entries
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    for (repo, d) in &c.diagnostics {
        let _ = writeln!(s, "{repo}: {d}");
    }
    s
}

#[test]
fn acme_catalog() {
    let (repos, diags) = check(&["acme-infra", "acme-app"]);
    let cat = catalog::build(repos, Strictness::Lenient);
    insta::assert_snapshot!("acme", format!("{diags}---\n{}", render(&cat)));
}

#[test]
fn acme_entries_and_headings() {
    let (repos, _) = check(&["acme-infra", "acme-app"]);
    let mut s = String::new();
    for r in &repos {
        for e in &r.entries {
            let _ = writeln!(
                s,
                "{}/{} {} {:?} spaces={:?} aliases={:?} covers={:?}",
                r.id,
                e.id,
                e.kind.as_str(),
                e.title,
                e.spaces_in(r),
                e.aliases,
                e.covers
            );
            for h in &e.body.headings {
                let _ = writeln!(s, "  h{} {:?} #{}", h.level, h.text, h.slug);
            }
            for b in &e.body.refs {
                let _ = writeln!(
                    s,
                    "  ref {:?} {} at {}:{}",
                    b.kind, b.target, b.loc.line, b.loc.col
                );
            }
        }
        for q in &r.golden {
            let _ = writeln!(
                s,
                "{} golden {:?} within {} meaning={} expect {:?}",
                r.id,
                q.query,
                q.within,
                q.needs_meaning,
                q.expect.iter().map(|e| e.0.to_string()).collect::<Vec<_>>()
            );
        }
    }
    insta::assert_snapshot!("acme-entries", s);
}

#[test]
fn strict_makes_cross_repo_refs_errors() {
    // acme-app alone: its refs into acme-infra can't resolve.
    let (repos, _) = check(&["acme-app"]);
    let lenient = catalog::build(repos.clone(), Strictness::Lenient);
    assert!(
        lenient
            .diagnostics
            .iter()
            .any(|(_, d)| d.code == "unresolved-ref" && !d.is_error())
    );
    let strict = catalog::build(repos, Strictness::Strict);
    assert!(
        strict
            .diagnostics
            .iter()
            .any(|(_, d)| d.code == "unresolved-ref" && d.is_error())
    );
}

#[test]
fn invalid_cases() {
    let dir = root().join("invalid");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{e}"))
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(names.len() >= 9);
    for n in names {
        let (repos, mut out) = check(&[&format!("invalid/{n}")]);
        let cat = catalog::build(repos, Strictness::Lenient);
        for (_, d) in &cat.diagnostics {
            let _ = writeln!(out, "invalid/{n}/{d}");
        }
        assert!(
            out.contains(": error: "),
            "invalid/{n} produced no error:\n{out}"
        );
        insta::assert_snapshot!(format!("invalid-{n}"), out);
    }
}

#[test]
fn limits_are_checked_before_parsing() {
    let big = InputFile {
        path: "links.yaml".into(),
        kind: FileKind::Regular,
        contents: None,
        size: 10 << 20,
    };
    let repo = InputFile::regular(
        "repo.yaml",
        b"format: 1\nid: x\nname: X\nsummary: S\n".to_vec(),
    );
    let (r, d) = parse_repo(&[repo.clone(), big], None, &Limits::default());
    assert!(r.is_none());
    assert!(d.iter().any(|d| d.code == "too-large"), "{d:?}");
    let many: Vec<InputFile> = (0..1001)
        .map(|i| InputFile::regular(format!("guides/g{i}.md"), vec![]))
        .collect();
    let (r, d) = parse_repo(&many, None, &Limits::default());
    assert!(r.is_none() && d.iter().any(|d| d.code == "too-many-files"));
    let file = InputFile::regular("about.md", vec![b'a'; 300 << 10]);
    let (_, d) = parse_repo(&[repo, file], None, &Limits::default());
    assert!(d.iter().any(|d| d.code == "file-too-large"));
}

#[test]
fn the_source_id_must_match() {
    let (_, d) = parse_repo(
        &read(&root().join("acme-app")),
        Some("other"),
        &Limits::default(),
    );
    assert!(d.iter().any(|d| d.code == "repo-id-mismatch"));
    let (_, d) = parse_repo(
        &read(&root().join("acme-app")),
        Some("acme-app"),
        &Limits::default(),
    );
    assert!(!d.iter().any(|d| d.is_error()));
}
