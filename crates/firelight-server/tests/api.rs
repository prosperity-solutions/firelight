//! `/api/v1` (§13.3): every route against expected JSON (snapshots on the
//! acme fixtures), and its errors.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use firelight_format::catalog::{self, Strictness};
use firelight_format::{Limits, parse_repo};
use firelight_server::content::Fixed;
use firelight_server::pages::Ui;
use firelight_server::{AppState, router};
use tower::ServiceExt;

fn app() -> axum::Router {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repos = ["fixtures/acme-infra", "fixtures/acme-app"]
        .iter()
        .map(|d| {
            let files = firelight_source::read_firelight_dir(&root.join(d), &Limits::default())
                .unwrap_or_else(|e| panic!("{e}"));
            parse_repo(&files, None, &Limits::default())
                .0
                .unwrap_or_else(|| panic!("{d}"))
        })
        .collect();
    let c = Arc::new(Fixed::new(Arc::new(catalog::build(
        repos,
        Strictness::Lenient,
    ))));
    let ui = Ui {
        public_url: "https://firelight.acme.example".into(),
        version: "0.0.0-test".into(),
        ..Ui::default()
    };
    router(
        AppState::new()
            .with_content(c, ui, "#d9541e")
            .with_access(firelight_server::auth::Access::open()),
    )
}

async fn get(app: &axum::Router, p: &str) -> (StatusCode, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(Request::get(p).body(Body::empty()).unwrap_or_default())
        .await
        .unwrap_or_else(|e| match e {});
    let s = r.status();
    let b = axum::body::to_bytes(r.into_body(), 1 << 22)
        .await
        .unwrap_or_default();
    (
        s,
        serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn every_route() {
    let app = app();
    for (name, p) in [
        ("search", "/api/v1/search?q=admin+rights&limit=3"),
        (
            "search-links",
            "/api/v1/search?q=argo&kind=link&space=platform",
        ),
        ("entries", "/api/v1/entries?kind=guide"),
        ("entries-tag", "/api/v1/entries?tag=access"),
        ("entry", "/api/v1/entries/acme-infra/request-access"),
        ("links", "/api/v1/links?q=argo"),
        ("links-card", "/api/v1/links?space=platform&card=staging"),
        ("spaces", "/api/v1/spaces"),
        ("repos", "/api/v1/repos"),
        ("repo", "/api/v1/repos/acme-infra"),
        ("status", "/api/v1/status"),
    ] {
        let (s, v) = get(&app, p).await;
        assert_eq!(s, StatusCode::OK, "{p}: {v}");
        insta::assert_snapshot!(name, serde_json::to_string_pretty(&v).unwrap_or_default());
    }
}

#[tokio::test]
async fn errors() {
    let app = app();
    for (p, want) in [
        ("/api/v1/search", StatusCode::BAD_REQUEST),
        ("/api/v1/search?q=x&space=nope", StatusCode::BAD_REQUEST),
        ("/api/v1/search?q=x&kind=nope", StatusCode::BAD_REQUEST),
        ("/api/v1/search?q=x&limit=lots", StatusCode::BAD_REQUEST),
        ("/api/v1/entries?kind=link", StatusCode::BAD_REQUEST),
        ("/api/v1/entries/acme-infra/nope", StatusCode::NOT_FOUND),
        ("/api/v1/entries/Bad_Repo/x", StatusCode::BAD_REQUEST),
        ("/api/v1/repos/nope", StatusCode::NOT_FOUND),
        ("/api/v1/search?q=x&repo=nope", StatusCode::BAD_REQUEST),
        ("/api/v1/entries/acme-infra/%FF", StatusCode::BAD_REQUEST),
    ] {
        let (s, v) = get(&app, p).await;
        assert_eq!(s, want, "{p}: {v}");
        assert!(v["error"].is_string(), "{p}: {v}");
    }
    // The limit is clamped to 25.
    let (_, v) = get(&app, "/api/v1/search?q=a&limit=1000").await;
    assert!(v["hits"].as_array().is_some_and(|h| h.len() <= 25));
}
