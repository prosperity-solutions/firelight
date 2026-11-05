//! The auth matrix (§10, §13.3): the three allowed mixes, wrong and rotated
//! credentials, the Host and Origin checks, credentials that must not cross
//! over, and path variants of `/mcp`.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use base64::Engine as _;
use firelight_format::catalog::{self, Strictness};
use firelight_format::{Limits, parse_repo};
use firelight_server::auth::Access;
use firelight_server::content::Fixed;
use firelight_server::pages::Ui;
use firelight_server::{AppState, router};
use tower::ServiceExt;

const HOST: &str = "firelight.acme.example";
const T1: &str = "old-token-0123456789abcdef";
const T2: &str = "new-token-0123456789abcdef";

fn content() -> Arc<Fixed> {
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
    Arc::new(Fixed::new(Arc::new(catalog::build(
        repos,
        Strictness::Lenient,
    ))))
}

fn access(basic: bool, token: bool) -> Access {
    let a = Access {
        basic: basic.then(|| ("ops".to_string(), "correct horse battery".to_string())),
        tokens: None,
        check_host: true,
        hosts: vec![HOST.into(), "firelight.firelight.svc".into()],
        loopback: false,
        origins: vec![format!("https://{HOST}")],
    };
    if token {
        a.with_tokens(vec![T1.into(), T2.into(), String::new()])
    } else {
        a
    }
}

fn app(a: Access) -> axum::Router {
    let ui = Ui {
        public_url: format!("https://{HOST}"),
        version: "0.0.0-test".into(),
        ..Ui::default()
    };
    router(
        AppState::new()
            .with_content(content(), ui, "#d9541e")
            .with_access(a),
    )
}

struct R<'a> {
    method: Method,
    path: &'a str,
    host: Option<&'a str>,
    auth: Option<String>,
    origin: Option<&'a str>,
}

impl<'a> R<'a> {
    fn get(path: &'a str) -> R<'a> {
        R {
            method: Method::GET,
            path,
            host: Some(HOST),
            auth: None,
            origin: None,
        }
    }
    fn mcp() -> R<'a> {
        R {
            method: Method::POST,
            ..R::get("/mcp")
        }
    }
    fn host(mut self, h: Option<&'a str>) -> Self {
        self.host = h;
        self
    }
    fn bearer(mut self, t: &str) -> Self {
        self.auth = Some(format!("Bearer {t}"));
        self
    }
    fn basic(mut self, u: &str, p: &str) -> Self {
        let b = base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"));
        self.auth = Some(format!("Basic {b}"));
        self
    }
    fn origin(mut self, o: &'a str) -> Self {
        self.origin = Some(o);
        self
    }
    async fn status(self, app: &axum::Router) -> StatusCode {
        let mut b = Request::builder()
            .method(self.method.clone())
            .uri(self.path);
        if let Some(h) = self.host {
            b = b.header("host", h);
        }
        if let Some(a) = &self.auth {
            b = b.header("authorization", a);
        }
        if let Some(o) = self.origin {
            b = b.header("origin", o);
        }
        let body = if self.method == Method::POST {
            b = b
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream");
            Body::from(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            )
        } else {
            Body::empty()
        };
        app.clone()
            .oneshot(b.body(body).unwrap_or_default())
            .await
            .map(|r| r.status())
            .unwrap_or(StatusCode::IM_A_TEAPOT)
    }
}

const PAGES: &[&str] = &[
    "/s/platform",
    "/api/v1/spaces",
    "/api/v1/status",
    "/status",
    "/assets/app.css",
    "/s/all/search?q=argo&format=json",
];

#[tokio::test]
async fn none_none() {
    let app = app(access(false, false));
    for p in PAGES {
        assert_eq!(R::get(p).status(&app).await, StatusCode::OK, "{p}");
    }
    assert_eq!(R::mcp().status(&app).await, StatusCode::OK);
}

#[tokio::test]
async fn none_token() {
    let app = app(access(false, true));
    for p in PAGES {
        assert_eq!(R::get(p).status(&app).await, StatusCode::OK, "{p}");
    }
    assert_eq!(R::mcp().status(&app).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        R::mcp().bearer("wrong-token-0123456789").status(&app).await,
        StatusCode::UNAUTHORIZED
    );
    // Rotation: the old and the new token both work.
    assert_eq!(R::mcp().bearer(T1).status(&app).await, StatusCode::OK);
    assert_eq!(R::mcp().bearer(T2).status(&app).await, StatusCode::OK);
    // A blank entry in the list never makes an empty bearer valid.
    let mut r = R::mcp();
    r.auth = Some("Bearer ".into());
    assert_eq!(r.status(&app).await, StatusCode::UNAUTHORIZED);
    let mut r = R::mcp();
    r.auth = Some("Bearer".into());
    assert_eq!(r.status(&app).await, StatusCode::UNAUTHORIZED);
    // The scheme is case-insensitive.
    let mut r = R::mcp();
    r.auth = Some(format!("bearer {T1}"));
    assert_eq!(r.status(&app).await, StatusCode::OK);
    // HEAD and OPTIONS on /mcp need the token too.
    for m in [Method::HEAD, Method::OPTIONS, Method::GET, Method::DELETE] {
        let r = R {
            method: m.clone(),
            ..R::get("/mcp")
        };
        assert_eq!(r.status(&app).await, StatusCode::UNAUTHORIZED, "{m}");
    }
}

#[tokio::test]
async fn basic_token() {
    let app = app(access(true, true));
    for p in PAGES {
        assert_eq!(
            R::get(p).status(&app).await,
            StatusCode::UNAUTHORIZED,
            "{p}"
        );
        assert_eq!(
            R::get(p).basic("ops", "wrong").status(&app).await,
            StatusCode::UNAUTHORIZED,
            "{p}"
        );
        assert_eq!(
            R::get(p)
                .basic("Ops", "correct horse battery")
                .status(&app)
                .await,
            StatusCode::UNAUTHORIZED,
            "{p}"
        );
        assert_eq!(
            R::get(p)
                .basic("ops", "correct horse battery")
                .status(&app)
                .await,
            StatusCode::OK,
            "{p}"
        );
        // A token never opens the page or the API.
        assert_eq!(
            R::get(p).bearer(T1).status(&app).await,
            StatusCode::UNAUTHORIZED,
            "{p}"
        );
    }
    // The login never opens /mcp.
    assert_eq!(
        R::mcp()
            .basic("ops", "correct horse battery")
            .status(&app)
            .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(R::mcp().bearer(T2).status(&app).await, StatusCode::OK);
}

#[tokio::test]
async fn open_routes_stay_open() {
    let app = app(access(true, true));
    for p in ["/healthz", "/readyz"] {
        let s = R::get(p).host(None).status(&app).await;
        assert!(
            s == StatusCode::OK || s == StatusCode::SERVICE_UNAVAILABLE,
            "{p}: {s}"
        );
    }
}

#[tokio::test]
async fn host_check() {
    let app = app(access(false, false));
    for h in [
        None,
        Some("evil.example"),
        Some("firelight.acme.example.evil.example"),
        Some("localhost"),
        Some("127.0.0.1:8080"),
        Some("0.0.0.0"),
    ] {
        assert_eq!(
            R::get("/s/platform").host(h).status(&app).await,
            StatusCode::MISDIRECTED_REQUEST,
            "{h:?}"
        );
        assert_eq!(
            R::mcp().host(h).status(&app).await,
            StatusCode::MISDIRECTED_REQUEST,
            "{h:?}"
        );
    }
    for h in [
        "FIRELIGHT.acme.example",
        "firelight.acme.example.",
        "firelight.acme.example:8443",
        "firelight.firelight.svc:8080",
    ] {
        assert_eq!(
            R::get("/s/platform").host(Some(h)).status(&app).await,
            StatusCode::OK,
            "{h}"
        );
    }
    // Unknown paths are behind the check too.
    assert_eq!(
        R::get("/nope")
            .host(Some("evil.example"))
            .status(&app)
            .await,
        StatusCode::MISDIRECTED_REQUEST
    );
}

#[tokio::test]
async fn loopback_hosts_where_allowed() {
    let mut a = access(false, false);
    a.loopback = true;
    let app = app(a);
    for h in [
        "localhost:3000",
        "127.0.0.1",
        "[::1]:8080",
        "firelight.run.firelight.localhost",
    ] {
        assert_eq!(
            R::get("/s/platform").host(Some(h)).status(&app).await,
            StatusCode::OK,
            "{h}"
        );
    }
    for h in ["localhost.evil.example", "127.0.0.1.nip.io", "0.0.0.0:8080"] {
        assert_eq!(
            R::get("/s/platform").host(Some(h)).status(&app).await,
            StatusCode::MISDIRECTED_REQUEST,
            "{h}"
        );
    }
}

#[tokio::test]
async fn origin_check() {
    let app = app(access(false, true));
    assert_eq!(
        R::mcp()
            .bearer(T1)
            .origin("https://evil.example")
            .status(&app)
            .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        R::mcp().bearer(T1).origin("null").status(&app).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        R::mcp().origin("https://evil.example").status(&app).await,
        StatusCode::FORBIDDEN,
        "Origin first"
    );
    assert_eq!(
        R::mcp()
            .bearer(T1)
            .origin("https://firelight.acme.example")
            .status(&app)
            .await,
        StatusCode::OK
    );
    assert_eq!(
        R::mcp()
            .bearer(T1)
            .origin("https://firelight.acme.example:443")
            .status(&app)
            .await,
        StatusCode::OK
    );
    assert_eq!(
        R::mcp()
            .bearer(T1)
            .origin("http://firelight.acme.example")
            .status(&app)
            .await,
        StatusCode::FORBIDDEN,
        "scheme counts"
    );
}

#[tokio::test]
async fn mcp_is_one_exact_path() {
    let app = app(access(false, true));
    for p in ["/mcp/", "/mcp/x", "//mcp", "/MCP", "/%6dcp", "/mcp/../mcp"] {
        let mut r = R::mcp();
        r.path = p;
        let s = r.status(&app).await;
        assert!(
            s == StatusCode::NOT_FOUND || s == StatusCode::METHOD_NOT_ALLOWED,
            "{p}: {s} (only /mcp exists)"
        );
    }
}

#[tokio::test]
async fn mcp_is_stateless_json() {
    let app = app(access(false, true));
    // No standalone SSE stream: GET and DELETE are 405 once the token passes.
    for m in [Method::GET, Method::DELETE] {
        let r = R {
            method: m.clone(),
            ..R::get("/mcp")
        }
        .bearer(T1);
        assert_eq!(r.status(&app).await, StatusCode::METHOD_NOT_ALLOWED, "{m}");
    }
}

#[tokio::test]
async fn webhook_and_api_rules() {
    let app = app(access(true, true));
    // The webhook has its HMAC; no Host check, no login (404: none configured).
    let r = R {
        method: Method::POST,
        ..R::get("/webhook/github")
    }
    .host(None);
    assert_eq!(r.status(&app).await, StatusCode::NOT_FOUND);
    // The API is behind the Host check like the page.
    assert_eq!(
        R::get("/api/v1/spaces")
            .host(Some("evil.example"))
            .status(&app)
            .await,
        StatusCode::MISDIRECTED_REQUEST
    );
    assert_eq!(
        R::get("/__live")
            .host(Some("evil.example"))
            .status(&app)
            .await,
        StatusCode::MISDIRECTED_REQUEST
    );
}
