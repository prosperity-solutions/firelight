//! Rendering (§13.1): HTML snapshots of the home page, an entry, a template
//! island with and without input, the Unsorted card; escaping through a real
//! repo; budgets (§18.6); headers.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use firelight_format::catalog::{self, Strictness};
use firelight_format::{InputFile, Limits, Ref, parse_repo};
use firelight_server::content::Fixed;
use firelight_server::pages::{self, Ui};
use firelight_server::{AppState, router};
use tower::ServiceExt;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn repo_from(dir: &str) -> firelight_format::Repo {
    let files = firelight_source::read_firelight_dir(&root().join(dir), &Limits::default())
        .unwrap_or_else(|e| panic!("{e}"));
    parse_repo(&files, None, &Limits::default())
        .0
        .unwrap_or_else(|| panic!("{dir} doesn't parse"))
}

fn acme() -> Arc<Fixed> {
    let cat = catalog::build(
        vec![
            repo_from("fixtures/acme-infra"),
            repo_from("fixtures/acme-app"),
        ],
        Strictness::Lenient,
    );
    Arc::new(Fixed(Arc::new(cat)))
}

fn ui() -> Ui {
    Ui {
        title: "Firelight·acme".into(),
        version: "0.0.0-test".into(),
        public_url: "https://firelight.acme.example".into(),
        ..Ui::default()
    }
}

/// Only the <main> part, so a change to the shell doesn't touch every snapshot.
fn main_of(html: &str) -> String {
    let s = html.find("<main").unwrap_or(0);
    let e = html.find("</main>").map_or(html.len(), |i| i + 7);
    html[s..e].to_string()
}

#[test]
fn snapshots() {
    let c = acme();
    let ui = ui();
    insta::assert_snapshot!(
        "home-platform",
        pages::space_page(&ui, c.as_ref(), "platform", &BTreeMap::new()).unwrap_or_default()
    );
    insta::assert_snapshot!(
        "entry",
        main_of(
            &pages::entry_page(
                &ui,
                c.as_ref(),
                &Ref {
                    repo: "acme-infra".into(),
                    id: "request-access".into()
                },
                Some("platform")
            )
            .unwrap_or_default()
        )
    );
    insta::assert_snapshot!("space-index", main_of(&pages::space_index(&ui, c.as_ref())));
    // The template island, empty and filled, and the Unsorted card are in these.
    let island =
        c.0.space("platform")
            .and_then(|s| s.islands.iter().find(|i| i.r.id == "previews"))
            .cloned()
            .unwrap_or_else(|| panic!("no previews island"));
    let firelight_format::catalog::PlacedKind::Template(t) = &island.kind else {
        panic!("not a template")
    };
    let mut ids = Default::default();
    insta::assert_snapshot!(
        "template-empty",
        pages::island_html(&c.0, &island, Some("platform"), &mut ids, None)
    );
    let good = pages::fill_template(t, &BTreeMap::from([("pr".to_string(), "42".to_string())]));
    insta::assert_snapshot!(
        "template-filled",
        pages::island_html(
            &c.0,
            &island,
            Some("platform"),
            &mut Default::default(),
            Some(&good)
        )
    );
    let bad = pages::fill_template(t, &BTreeMap::from([("pr".to_string(), "4/2".to_string())]));
    insta::assert_snapshot!(
        "template-refused",
        pages::island_html(
            &c.0,
            &island,
            Some("platform"),
            &mut Default::default(),
            Some(&bad)
        )
    );
    let unsorted =
        c.0.space("platform")
            .and_then(|s| s.islands.iter().find(|i| i.unsorted))
            .cloned()
            .unwrap_or_else(|| panic!("no Unsorted"));
    insta::assert_snapshot!(
        "unsorted",
        pages::island_html(
            &c.0,
            &unsorted,
            Some("platform"),
            &mut Default::default(),
            None
        )
    );
}

/// Hostile text in every field a repo controls never becomes markup.
#[test]
fn hostile_repo_text_is_escaped_everywhere() {
    let x = "<script>alert(1)</script>";
    let q = "\" onmouseover=\"alert(1)";
    let files = vec![
        InputFile::regular("repo.yaml", format!("format: 1\nid: evil\nname: '{x}'\nsummary: '{q}'\nowner: '{x}'\nrelations:\n  - {{ repo: other, when: '{x}', then: '{q}' }}\n").into_bytes()),
        InputFile::regular("links.yaml", format!("format: 1\nlinks:\n  - {{ id: l, label: '{x}', note: '{q}', url: 'https://a.acme.example/?\"><script>', at: {{ place: top }} }}\n").into_bytes()),
        InputFile::regular("islands/c.yaml", format!("format: 1\ntype: cards\ntitle: '{x}'\nsections: ['{x}']\ncards:\n  - {{ id: c, title: '{q}' }}\n").into_bytes()),
        InputFile::regular("guides/g.md", format!("---\nformat: 1\ntitle: '{x}'\nsummary: '{q}'\n---\n# {x}\n\n<img src=x onerror=alert(1)>\n\n[a](javascript:alert(1))\n").into_bytes()),
    ];
    let (repo, _) = parse_repo(&files, None, &Limits::default());
    let repo = repo.unwrap_or_else(|| panic!("the hostile repo should still parse"));
    let cat = Arc::new(catalog::build(vec![repo], Strictness::Lenient));
    let c = Fixed(cat);
    let ui = ui();
    let pages_html = [
        pages::space_page(&ui, &c, "default", &BTreeMap::new()).unwrap_or_default(),
        pages::entry_page(
            &ui,
            &c,
            &Ref {
                repo: "evil".into(),
                id: "g".into(),
            },
            None,
        )
        .unwrap_or_default(),
        pages::repo_page(&ui, &c, "evil").unwrap_or_default(),
        pages::search_page(&ui, &c, "default", x),
    ];
    for h in pages_html {
        assert!(!h.contains("<script>alert"), "{h}");
        assert!(!h.contains("<img src=x"), "{h}");
        assert!(!h.contains("\" onmouseover=\""), "{h}");
        assert!(!h.contains("href=\"javascript"), "{h}");
        assert!(!h.contains("\"><script>"), "{h}");
    }
}

#[test]
fn budgets() {
    let web = root().join("web");
    let size = |ext: &str| -> u64 {
        std::fs::read_dir(&web)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(ext))
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum()
    };
    let js = size(".js");
    let css = size(".css");
    assert!(
        js <= 40 * 1024,
        "web/*.js is {js} bytes; the budget is 40 KB (§18.6)"
    );
    assert!(
        css <= 25 * 1024,
        "web/*.css is {css} bytes; the budget is 25 KB (§18.6)"
    );
}

#[tokio::test]
async fn routes_and_headers() {
    let state = AppState::new().with_content(acme(), ui(), "#123abc");
    let get = |p: &str| {
        let s = state.clone();
        let p = p.to_string();
        async move {
            router(s)
                .oneshot(
                    Request::get(p.as_str())
                        .body(Body::empty())
                        .unwrap_or_default(),
                )
                .await
                .unwrap_or_else(|e| match e {})
        }
    };
    let r = get("/s/platform").await;
    assert_eq!(r.status(), StatusCode::OK);
    let csp = r
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"),
        "{csp}"
    );
    assert_eq!(
        r.headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        get("/").await.status(),
        StatusCode::OK,
        "two spaces: the space list"
    );
    assert_eq!(get("/s/nope").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        get("/e/acme-infra/nope").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(get("/assets/app.js").await.status(), StatusCode::OK);
    assert_eq!(get("/assets/nope.js").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        get("/assets/../Cargo.toml").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get("/__live").await.status(),
        StatusCode::NOT_FOUND,
        "live reload is preview-only"
    );
    let body = axum::body::to_bytes(get("/assets/theme.css").await.into_body(), 1 << 16)
        .await
        .unwrap_or_default();
    assert_eq!(
        String::from_utf8_lossy(&body),
        ":root { --accent: #123abc; }\n"
    );
    let body = axum::body::to_bytes(
        get("/i/acme-app/previews?pr=..&format=json")
            .await
            .into_body(),
        1 << 16,
    )
    .await
    .unwrap_or_default();
    assert!(String::from_utf8_lossy(&body).contains("\"ok\":false"));
}

#[tokio::test]
async fn default_space_redirects_and_falls_back() {
    let mut u = ui();
    u.default_space = Some("support".into());
    let r = router(AppState::new().with_content(acme(), u, "#d9541e"))
        .oneshot(Request::get("/").body(Body::empty()).unwrap_or_default())
        .await
        .unwrap_or_else(|e| match e {});
    assert_eq!(
        r.headers().get("location").and_then(|v| v.to_str().ok()),
        Some("/s/support")
    );
    let mut u = ui();
    u.default_space = Some("gone".into());
    let r = router(AppState::new().with_content(acme(), u, "#d9541e"))
        .oneshot(Request::get("/").body(Body::empty()).unwrap_or_default())
        .await
        .unwrap_or_else(|e| match e {});
    assert_eq!(
        r.status(),
        StatusCode::OK,
        "a missing default space shows the list, never an error"
    );
}
