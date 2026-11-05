//! Rules not covered by the fixture snapshots (tests/fixtures.rs).

use super::*;

const REPO: &str = "format: 1\nid: r\nname: R\nsummary: S.\n";

fn check(files: &[(&str, &str)]) -> (Option<Repo>, Vec<String>) {
    let input: Vec<InputFile> = files
        .iter()
        .map(|(p, c)| InputFile::regular(*p, c.as_bytes().to_vec()))
        .collect();
    let (r, d) = parse_repo(&input, None, &Limits::default());
    (
        r,
        d.iter()
            .map(|d| format!("{}:{} {} {}", d.path, d.line, d.code, d.message))
            .collect(),
    )
}

fn has(d: &[String], needle: &str) -> bool {
    d.iter().any(|x| x.contains(needle))
}

#[test]
fn a_broken_repo_yaml_still_checks_the_rest() {
    let (r, d) = check(&[
        ("repo.yaml", "format: 1\nid: r\nname: R\nsumary: typo\n"),
        (
            "links.yaml",
            "format: 1\nlinks:\n  - id: a\n    label: A\n    url: ftp://x\n    at: { place: top }\n",
        ),
    ]);
    assert!(r.is_some());
    assert!(has(&d, "unknown key `sumary`"), "{d:?}");
    assert!(has(&d, "\"ftp://x\" is not an allowed URL"), "{d:?}");
    // Without a usable id nothing else can be checked.
    let (r, _) = check(&[("repo.yaml", "format: 1\nid: Bad Id\n")]);
    assert!(r.is_none());
}

#[test]
fn missing_and_unknown_versions() {
    let (_, d) = check(&[("repo.yaml", REPO), ("golden.yaml", "questions: []\n")]);
    assert!(has(&d, ".firelight/golden.yaml:1 format-missing"), "{d:?}");
    assert_eq!(d.len(), 1, "reported once: {d:?}");
    let (_, d) = check(&[
        ("repo.yaml", REPO),
        ("guides/g.md", "---\ntitle: G\nsummary: S.\n---\nx\n"),
    ]);
    assert!(has(&d, ".firelight/guides/g.md:2 format-missing"), "{d:?}");
    let (r, d) = check(&[(
        "repo.yaml",
        "format: 4294967297\nid: r\nname: R\nsummary: S.\n",
    )]);
    assert!(
        r.is_none() && has(&d, "format-unsupported"),
        "no truncation to 1: {d:?}"
    );
}

#[test]
fn files_outside_the_format_are_only_warnings() {
    let (r, d) = check(&[
        ("repo.yaml", REPO),
        ("notes.yaml", "anything: 1\n"),
        ("drafts/x.md", "no front matter"),
    ]);
    assert!(r.is_some());
    assert!(d.iter().all(|x| x.contains("unknown-file")), "{d:?}");
    assert_eq!(d.len(), 2);
    let (_, d) = check(&[
        ("repo.yaml", REPO),
        ("islands/x.yml", "format: 1\ntype: links\ntitle: X\n"),
    ]);
    assert!(has(&d, "file-name island files end in .yaml"), "{d:?}");
}

#[test]
fn reserved_spaces_and_ids() {
    let (_, d) = check(&[(
        "repo.yaml",
        "format: 1\nid: r\nname: R\nsummary: S.\nspaces: [firelight]\n",
    )]);
    assert!(has(&d, "reserved-space"), "{d:?}");
    let (_, d) = check(&[
        ("repo.yaml", REPO),
        (
            "concepts/c.md",
            "---\nformat: 1\ntitle: C\nsummary: S.\nspaces: [all]\n---\nx\n",
        ),
    ]);
    assert!(has(&d, "reserved-space"), "{d:?}");
    // The self-docs may use the firelight space.
    let (_, d) = check(&[(
        "repo.yaml",
        "format: 1\nid: firelight\nname: Firelight\nsummary: S.\nspaces: [firelight]\n",
    )]);
    assert!(d.is_empty(), "{d:?}");
    // ...but only as the self-docs source.
    let files = [InputFile::regular(
        "repo.yaml",
        b"format: 1\nid: firelight\nname: F\nsummary: S.\n".to_vec(),
    )];
    let (_, d) = parse_repo(&files, Some("other"), &Limits::default());
    assert!(d.iter().any(|x| x.code == "reserved-id"));
}

#[test]
fn template_inputs() {
    let (_, d) = check(&[
        ("repo.yaml", REPO),
        (
            "islands/t.yaml",
            "format: 1\ntype: template\ntitle: T\ninputs:\n  - { name: a, label: A, pattern: \"^x$\" }\n  - { name: b, label: B, pattern: \"^y$\" }\nlinks:\n  - { label: L, url: \"https://ci.acme.example/{a}\" }\n",
        ),
    ]);
    assert!(
        has(&d, "template-unused input \"b\" is used by no link"),
        "{d:?}"
    );
    let (_, d) = check(&[
        ("repo.yaml", REPO),
        (
            "islands/t.yaml",
            "format: 1\ntype: template\ntitle: T\ninputs: []\nlinks: []\n",
        ),
    ]);
    assert!(has(&d, "needs at least one input"), "{d:?}");
}

#[test]
fn a_links_entry_is_a_concept() {
    let (_, d) = check(&[
        ("repo.yaml", REPO),
        (
            "guides/g.md",
            "---\nformat: 1\ntitle: G\nsummary: S.\n---\nx\n",
        ),
        (
            "links.yaml",
            "format: 1\nlinks:\n  - { id: a, label: A, url: \"https://a.acme.example\", at: { place: top }, entry: g }\n",
        ),
    ]);
    assert!(has(&d, "entry-not-concept"), "{d:?}");
}

#[test]
fn special_files_and_symlinks() {
    let files = [
        InputFile::regular("repo.yaml", REPO.as_bytes().to_vec()),
        InputFile {
            path: "guides/a.md".into(),
            kind: FileKind::Symlink,
            contents: None,
            size: 0,
        },
        InputFile {
            path: "guides/b.md".into(),
            kind: FileKind::Special,
            contents: None,
            size: 0,
        },
    ];
    let (_, d) = parse_repo(&files, None, &Limits::default());
    assert!(d.iter().any(|x| x.code == "symlink"));
    assert!(d.iter().any(|x| x.code == "special-file"));
}
