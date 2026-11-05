//! Search (§13.1): chunker snapshot, words with the embedder off, meaning with
//! the deterministic fake embedder, fusion, the exact-match boost, space and
//! filter rules, golden questions.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use firelight_embed::{Embedder, Fake};
use firelight_format::catalog::{self, Catalog, Strictness};
use firelight_format::{Limits, Ref, parse_repo};
use firelight_search::golden::{self, Outcome};
use firelight_search::index::RRF_K;
use firelight_search::{Filter, Index, Kind};

fn repo(dir: &str) -> firelight_format::Repo {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir);
    let files = firelight_source::read_firelight_dir(&root, &Limits::default())
        .unwrap_or_else(|e| panic!("{e}"));
    parse_repo(&files, None, &Limits::default())
        .0
        .unwrap_or_else(|| panic!("{dir}"))
}

fn acme() -> Arc<Catalog> {
    Arc::new(catalog::build(
        vec![repo("fixtures/acme-infra"), repo("fixtures/acme-app")],
        Strictness::Lenient,
    ))
}

fn fake() -> Fake {
    Fake::with_synonyms(&[("adminrechte", "admin rights"), ("beantragen", "request")])
}

/// An index with every chunk embedded by `e`.
fn embedded(cat: Arc<Catalog>, e: &dyn Embedder) -> Index {
    let idx = Index::build(cat, Some(e.id()), e.max_tokens(), &|t| e.count_tokens(t))
        .unwrap_or_else(|x| panic!("{x}"));
    let missing = idx.missing();
    let texts: Vec<&str> = missing.iter().map(|(_, t)| t.as_str()).collect();
    let vecs = e.embed_documents(&texts).unwrap_or_default();
    let by: HashMap<[u8; 32], Vec<f32>> = missing.iter().map(|(h, _)| *h).zip(vecs).collect();
    idx.set_vectors(&by);
    idx
}

fn r(repo: &str, id: &str) -> Ref {
    Ref {
        repo: repo.into(),
        id: id.into(),
    }
}

#[test]
fn chunker_snapshot() {
    let idx = Index::words_only(acme()).unwrap_or_else(|e| panic!("{e}"));
    let lines: Vec<String> = idx
        .chunks
        .iter()
        .map(|c| {
            format!(
                "{:?} {} {:?} | {}",
                c.kind,
                c.owner,
                c.heading.as_ref().map(|h| &h.1),
                c.text.lines().next().unwrap_or("")
            )
        })
        .collect();
    insta::assert_snapshot!(lines.join("\n"));
}

#[test]
fn words_alone_answer_the_spec_question() {
    let idx = Index::words_only(acme()).unwrap_or_else(|e| panic!("{e}"));
    let hits = idx.search("how do I get admin rights", &Filter::space("all"), 5, None);
    let pos = hits
        .iter()
        .position(|h| h.r == r("acme-infra", "request-access"));
    assert!(pos.is_some_and(|p| p < 3), "{hits:#?}");
}

#[test]
fn meaning_finds_what_words_cant() {
    let cat = acme();
    let q = "adminrechte beantragen";
    let words = Index::words_only(cat.clone()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        !words
            .search(q, &Filter::space("all"), 5, None)
            .iter()
            .any(|h| h.r == r("acme-infra", "request-access")),
        "words alone can't (or the test proves nothing)"
    );
    let f = fake();
    let idx = embedded(cat, &f);
    let qv = f.embed_query(q).unwrap_or_default();
    let hits = idx.search(q, &Filter::space("all"), 5, Some(&qv));
    let pos = hits
        .iter()
        .position(|h| h.r == r("acme-infra", "request-access"));
    assert!(pos.is_some_and(|p| p < 3), "{hits:#?}");
}

#[test]
fn fusion_is_reciprocal_rank() {
    // Rank 1 in one list only vs rank 2 in both: 1/61 < 2/62.
    let one = 1.0 / (RRF_K + 1.0);
    let both = 2.0 / (RRF_K + 2.0);
    assert!(both > one);
    let f = fake();
    let idx = embedded(acme(), &f);
    let q = "deploy";
    let qv = f.embed_query(q).unwrap_or_default();
    let hits = idx.search(q, &Filter::space("all"), 10, Some(&qv));
    for w in hits.windows(2) {
        assert!(
            w[0].score >= w[1].score || w[0].title.to_lowercase() == q,
            "{hits:#?}"
        );
    }
    assert!(hits.iter().all(|h| h.score <= 2.0 / (RRF_K + 1.0) + 1e-9));
}

#[test]
fn an_exact_title_or_alias_goes_first() {
    let f = fake();
    let idx = embedded(acme(), &f);
    for q in ["elevate", "Request access rights", "just-in-time access"] {
        let qv = f.embed_query(q).unwrap_or_default();
        let hits = idx.search(q, &Filter::space("all"), 5, Some(&qv));
        assert_eq!(
            hits.first().map(|h| h.r.clone()),
            Some(r("acme-infra", "request-access")),
            "{q}: {hits:#?}"
        );
    }
}

#[test]
fn spaces_and_filters() {
    let cat = acme();
    let idx = Index::words_only(cat.clone()).unwrap_or_else(|e| panic!("{e}"));
    for s in cat
        .spaces
        .iter()
        .map(|s| s.id.clone())
        .chain(["all".to_string()])
    {
        for q in [
            "access", "deploy", "acme", "argo", "release", "app", "status",
        ] {
            for h in idx.search(q, &Filter::space(&s), 100, None) {
                let ok = match h.kind {
                    Kind::Repo => cat
                        .spaces
                        .iter()
                        .filter(|x| {
                            if s == "all" {
                                x.id != "firelight"
                            } else {
                                x.id == s
                            }
                        })
                        .any(|x| x.repos.contains(&h.r.repo)),
                    Kind::Link => firelight_search::links_in_space(&cat, &s).contains(&h.r),
                    _ => cat
                        .spaces
                        .iter()
                        .filter(|x| {
                            if s == "all" {
                                x.id != "firelight"
                            } else {
                                x.id == s
                            }
                        })
                        .any(|x| x.entries.contains(&h.r)),
                };
                assert!(ok, "{:?} {} leaked into {s} for {q:?}", h.kind, h.r);
            }
        }
    }
    // Repo hits come back under `all`.
    let hits = idx.search("acme", &Filter::space("all"), 50, None);
    assert!(hits.iter().any(|h| h.kind == Kind::Repo), "{hits:#?}");
    // Filters apply before the limit: one result, and it is the right kind.
    let f = Filter {
        kinds: Some(BTreeSet::from([Kind::Link])),
        ..Filter::space("all")
    };
    let hits = idx.search("argo", &f, 1, None);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, Kind::Link);
    let f = Filter {
        repos: Some(BTreeSet::from(["acme-app".to_string()])),
        ..Filter::default()
    };
    assert!(
        idx.search("deploy", &f, 50, None)
            .iter()
            .all(|h| h.r.repo == "acme-app")
    );
}

#[test]
fn query_syntax_stays_literal() {
    let idx = Index::words_only(acme()).unwrap_or_else(|e| panic!("{e}"));
    for q in [
        "NEAR(argo cd)",
        "title:argo",
        "^argo",
        "\"argo",
        "argo*",
        "a AND OR NOT b",
        "-",
    ] {
        let _ = idx.search(q, &Filter::space("all"), 5, None);
    }
    let hits = idx.search("title:argo", &Filter::space("all"), 5, None);
    assert!(
        !hits.is_empty(),
        "`title:` is a word here, not a column filter"
    );
}

#[test]
fn golden_questions() {
    let cat = acme();
    let qs: Vec<(String, firelight_format::GoldenQuestion)> = cat
        .repos
        .iter()
        .flat_map(|x| x.golden.iter().map(move |q| (x.id.clone(), q.clone())))
        .collect();
    assert!(qs.iter().any(|(_, q)| q.needs_meaning));
    let words = Index::words_only(cat.clone()).unwrap_or_else(|e| panic!("{e}"));
    let res = golden::run(&words, &qs, &Filter::default(), &|_| None);
    for x in &res {
        let q = qs
            .iter()
            .find(|(_, q)| q.query == x.query)
            .map(|(_, q)| q.needs_meaning);
        if q == Some(true) {
            assert_eq!(x.outcome, Outcome::Skipped, "{}", golden::describe(x));
        } else {
            assert_eq!(x.outcome, Outcome::Pass, "{}", golden::describe(x));
        }
    }
    let f = fake();
    let idx = embedded(cat, &f);
    let res = golden::run(&idx, &qs, &Filter::default(), &|q| f.embed_query(q).ok());
    for x in &res {
        assert_eq!(x.outcome, Outcome::Pass, "{}", golden::describe(x));
    }
    // A question that can't pass reports where its entry ranked.
    let mut bad = qs[0].clone();
    bad.1.query = "zebra".into();
    let res = golden::run(&idx, &[bad], &Filter::default(), &|_| None);
    assert!(res[0].failed());
    assert!(
        golden::describe(&res[0]).contains("not in the top"),
        "{}",
        golden::describe(&res[0])
    );
}

#[test]
fn vectors_follow_their_embedder() {
    let f = fake();
    let idx = embedded(acme(), &f);
    assert_eq!(idx.coverage().0, idx.coverage().1);
    assert!(idx.missing().is_empty());
    let words = Index::words_only(acme()).unwrap_or_else(|e| panic!("{e}"));
    words.set_vectors(&HashMap::from([(words.chunks[0].hash, vec![1.0])]));
    assert_eq!(words.coverage().0, 0, "a words-only index takes no vectors");
    assert!(words.missing().is_empty());
}

fn tiny_repo(guide: &str) -> Arc<Catalog> {
    let files = vec![
        firelight_format::InputFile::regular(
            "repo.yaml",
            b"format: 1\nid: tiny\nname: Tiny\nsummary: A test repo.\n".to_vec(),
        ),
        firelight_format::InputFile::regular("guides/g.md", guide.as_bytes().to_vec()),
    ];
    let repo = parse_repo(&files, None, &Limits::default())
        .0
        .unwrap_or_else(|| panic!("tiny repo"));
    Arc::new(catalog::build(vec![repo], Strictness::Lenient))
}

/// Sections follow the parser's headings: setext, blockquote and a fence
/// that holds a `## ` line all agree with the anchors.
#[test]
fn sections_follow_the_parsers_headings() {
    let g = "---\nformat: 1\ntitle: G\nsummary: S.\n---\nIntro.\n\nSetup\n-----\n\nText A\n\n> ## Quoted\n\nText B\n\n````\n```\n## not a heading\n```\n````\n\n### Last\n\nText C\n";
    let idx = Index::words_only(tiny_repo(g)).unwrap_or_else(|e| panic!("{e}"));
    let got: Vec<(Option<String>, String)> = idx
        .chunks
        .iter()
        .filter(|c| c.kind == Kind::Guide)
        .map(|c| {
            (
                c.heading.as_ref().map(|h| h.1.clone()),
                c.body.trim().to_string(),
            )
        })
        .collect();
    let slugs: Vec<Option<&str>> = got.iter().map(|(s, _)| s.as_deref()).collect();
    assert_eq!(
        slugs,
        [None, Some("setup"), Some("quoted"), Some("last")],
        "{got:#?}"
    );
    assert!(
        got[1].1.contains("Text A") && !got[1].1.contains("Text B"),
        "{got:#?}"
    );
    assert!(
        got[2].1.contains("not a heading"),
        "a fenced line is text: {got:#?}"
    );
    assert!(got[3].1.contains("Text C"), "{got:#?}");
}

/// More than 500 matching chunks in other spaces don't hide the one in the
/// space searched (filters apply before the cut).
#[test]
fn filters_apply_before_the_cut() {
    let mut files = vec![
        firelight_format::InputFile::regular(
            "repo.yaml",
            b"format: 1\nid: big\nname: Big\nsummary: Many guides.\nspaces: [noise]\n".to_vec(),
        ),
        firelight_format::InputFile::regular(
            "spaces.yaml",
            b"format: 1\nspaces:\n  - { id: noise, title: Noise }\n  - { id: mine, title: Mine }\n".to_vec(),
        ),
        firelight_format::InputFile::regular(
            "guides/target.md",
            b"---\nformat: 1\ntitle: Target\nsummary: How the widget works.\nspaces: [mine]\n---\nThe widget.\n".to_vec(),
        ),
    ];
    for i in 0..700 {
        files.push(firelight_format::InputFile::regular(
            format!("guides/n{i}.md"),
            format!("---\nformat: 1\ntitle: Widget {i}\nsummary: Widget widget widget.\n---\nWidget widget.\n").into_bytes(),
        ));
    }
    let limits = Limits {
        max_files: 2000,
        ..Limits::default()
    };
    let repo = parse_repo(&files, None, &limits)
        .0
        .unwrap_or_else(|| panic!("big"));
    let cat = Arc::new(catalog::build(vec![repo], Strictness::Lenient));
    let idx = Index::words_only(cat).unwrap_or_else(|e| panic!("{e}"));
    assert!(idx.chunks.len() > 500);
    let hits = idx.search("widget", &Filter::space("mine"), 5, None);
    assert_eq!(
        hits.first().map(|h| h.r.clone()),
        Some(r("big", "target")),
        "{hits:#?}"
    );
}
