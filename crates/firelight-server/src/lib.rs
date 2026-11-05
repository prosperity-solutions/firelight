//! HTTP: pages, `/api/v1`, `/mcp`, `/webhook`, health and readiness
//! (docs/HANDOVER.md §3, §16, §18).
//!
//! M1 holds the skeleton: the router, `/healthz` and `/readyz`, and graceful
//! shutdown. Later milestones add the routes behind the same [`AppState`].

mod api;
pub mod auth;
pub mod content;
pub mod data;
pub mod html;
pub mod markdown;
pub mod mcp;
pub mod onboarding;
pub mod pages;
mod routes;

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};

/// What every route can see.
#[derive(Clone, Default)]
pub struct AppState {
    ready: Arc<AtomicBool>,
    webhook: Option<Arc<Webhook>>,
    content: Option<Arc<dyn content::Content>>,
    access: Arc<auth::Access>,
    ui: Arc<pages::Ui>,
    accent: Arc<str>,
}

/// GitHub webhooks (§6.2): verified by HMAC, a `push` to a configured
/// source's ref triggers a pull of that source.
pub struct Webhook {
    pub secret: Vec<u8>,
    pub targets: Vec<WebhookTarget>,
    pub wake: tokio::sync::mpsc::Sender<String>,
}

/// One source a push can wake.
#[derive(Debug, Clone)]
pub struct WebhookTarget {
    pub source_id: String,
    /// `owner/repo`, lowercase.
    pub repo: String,
    /// The branch; `None` = the repo's default branch.
    pub branch: Option<String>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    /// With content to serve (the page routes need it).
    pub fn with_content(
        mut self,
        c: Arc<dyn content::Content>,
        ui: pages::Ui,
        accent: &str,
    ) -> Self {
        self.content = Some(c);
        self.ui = Arc::new(ui);
        self.accent = Arc::from(accent);
        self
    }

    /// Who may ask what (§10).
    pub fn with_access(mut self, a: auth::Access) -> Self {
        self.access = Arc::new(a);
        self
    }

    /// With webhooks on (a secret is configured).
    pub fn with_webhook(mut self, w: Webhook) -> Self {
        self.webhook = Some(Arc::new(w));
        self
    }

    /// Readiness (§3): a catalog is loaded and the built-in golden check
    /// passed. The startup sequence flips this once both hold.
    pub fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::SeqCst);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
}

/// The full router.
pub fn router(state: AppState) -> Router {
    use axum::middleware::from_fn_with_state;
    // Always open: probes and the webhook (its HMAC is its auth).
    let open = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        // GitHub sends push payloads up to 25 MB.
        .route(
            "/webhook/github",
            post(webhook).layer(axum::extract::DefaultBodyLimit::max(25 << 20)),
        );
    // Exactly `/mcp` (no nesting: `/mcp/` and `/mcp/x` don't exist), with
    // its own Origin and bearer checks.
    let mcp = Router::new()
        .route("/mcp", mcp_service(&state))
        .route_layer(from_fn_with_state(state.clone(), auth::mcp));
    // The page and the API, behind basic auth when it is on.
    let ui = Router::new()
        .merge(routes::pages())
        .merge(api::routes())
        .route_layer(from_fn_with_state(state.clone(), auth::basic));
    // The Host check covers everything but the open routes, unknown paths too.
    let checked = mcp
        .merge(ui)
        .layer(from_fn_with_state(state.clone(), auth::host));
    open.merge(checked)
        .with_state(state)
        .layer(axum::middleware::from_fn(routes::security_headers))
}

/// `/mcp`, exactly (no nesting, so `/mcp/x` or `//mcp` never reach it).
fn mcp_service(state: &AppState) -> axum::routing::MethodRouter<AppState> {
    match &state.content {
        Some(c) => axum::routing::any_service(
            mcp::Mcp::new(c.clone(), &state.ui.public_url, &state.ui.version).service(),
        ),
        None => axum::routing::any(|| async { (StatusCode::SERVICE_UNAVAILABLE, "starting\n") }),
    }
}

/// Liveness: the process answers. Always open, no auth, no Host check (§10).
async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok\n")
}

/// Readiness: 200 once ready, 503 before. Always open (§10).
async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    if state.is_ready() {
        (StatusCode::OK, "ready\n")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "starting\n")
    }
}

/// `POST /webhook/github` (§6.2): 404 without a secret; 401 for a missing or
/// bad signature; 202 for a push to a configured source's ref (which pulls it
/// now); 204 for any other valid event. Authenticated by its HMAC only.
async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(w) = state.webhook.as_ref() else {
        return StatusCode::NOT_FOUND;
    };
    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok());
    if !firelight_source::github::verify_webhook(&w.secret, &body, sig) {
        tracing::warn!("webhook: missing or bad signature");
        return StatusCode::UNAUTHORIZED;
    }
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if event != "push" {
        return StatusCode::NO_CONTENT;
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return StatusCode::NO_CONTENT;
    };
    let repo = v["repository"]["full_name"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    let pushed = v["ref"].as_str().unwrap_or("");
    let default = v["repository"]["default_branch"].as_str().unwrap_or("");
    let mut woke = false;
    for t in w.targets.iter().filter(|t| t.repo == repo) {
        let branch = t.branch.as_deref().unwrap_or(default);
        if !branch.is_empty() && pushed == format!("refs/heads/{branch}") {
            woke |= w.wake.try_send(t.source_id.clone()).is_ok();
        }
    }
    if woke {
        StatusCode::ACCEPTED
    } else {
        StatusCode::NO_CONTENT
    }
}

/// Errors from serving.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("cannot listen on {addr}: {source}. Pick a free address with FIRELIGHT_SERVER_LISTEN.")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    #[error("the HTTP server stopped: {0}")]
    Serve(std::io::Error),
}

/// Bind `addr`, then serve until `shutdown` resolves, finishing in-flight
/// requests. `on_bound` runs once the socket listens, with the real address.
pub async fn serve<F, B>(
    addr: SocketAddr,
    state: AppState,
    on_bound: B,
    shutdown: F,
) -> Result<(), ServeError>
where
    F: std::future::Future<Output = ()> + Send + 'static,
    B: FnOnce(SocketAddr),
{
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| ServeError::Bind { addr, source })?;
    let local = listener.local_addr().map_err(ServeError::Serve)?;
    on_bound(local);
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(ServeError::Serve)
}

/// Resolves on SIGTERM or Ctrl-C (Kubernetes sends SIGTERM).
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "cannot listen for SIGTERM; only Ctrl-C stops the server");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
    tracing::info!("shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn status(state: &AppState, path: &str) -> StatusCode {
        router(state.clone())
            .oneshot(Request::get(path).body(Body::empty()).unwrap_or_default())
            .await
            .map(|r| r.status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    fn signed(secret: &[u8], body: &str) -> String {
        use hmac_test::sign;
        format!("sha256={}", sign(secret, body.as_bytes()))
    }

    mod hmac_test {
        /// HMAC-SHA256 by hand (RFC 2104), so the test doesn't share code
        /// with what it checks.
        pub fn sign(key: &[u8], msg: &[u8]) -> String {
            use sha2_impl::sha256;
            let mut k = [0u8; 64];
            if key.len() > 64 {
                k[..32].copy_from_slice(&sha256(key));
            } else {
                k[..key.len()].copy_from_slice(key);
            }
            let ipad: Vec<u8> = k
                .iter()
                .map(|b| b ^ 0x36)
                .chain(msg.iter().copied())
                .collect();
            let inner = sha256(&ipad);
            let opad: Vec<u8> = k
                .iter()
                .map(|b| b ^ 0x5c)
                .chain(inner.iter().copied())
                .collect();
            sha256(&opad).iter().map(|b| format!("{b:02x}")).collect()
        }
        mod sha2_impl {
            pub fn sha256(data: &[u8]) -> [u8; 32] {
                const K: [u32; 64] = [
                    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
                    0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
                    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
                    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
                    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
                    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
                    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
                    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
                    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
                    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
                    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
                ];
                let mut h: [u32; 8] = [
                    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                    0x1f83d9ab, 0x5be0cd19,
                ];
                let mut m = data.to_vec();
                let bits = (data.len() as u64) * 8;
                m.push(0x80);
                while m.len() % 64 != 56 {
                    m.push(0);
                }
                m.extend_from_slice(&bits.to_be_bytes());
                for chunk in m.chunks(64) {
                    let mut w = [0u32; 64];
                    for i in 0..16 {
                        w[i] = u32::from_be_bytes([
                            chunk[4 * i],
                            chunk[4 * i + 1],
                            chunk[4 * i + 2],
                            chunk[4 * i + 3],
                        ]);
                    }
                    for i in 16..64 {
                        let s0 = w[i - 15].rotate_right(7)
                            ^ w[i - 15].rotate_right(18)
                            ^ (w[i - 15] >> 3);
                        let s1 = w[i - 2].rotate_right(17)
                            ^ w[i - 2].rotate_right(19)
                            ^ (w[i - 2] >> 10);
                        w[i] = w[i - 16]
                            .wrapping_add(s0)
                            .wrapping_add(w[i - 7])
                            .wrapping_add(s1);
                    }
                    let mut a = h;
                    for i in 0..64 {
                        let s1 =
                            a[4].rotate_right(6) ^ a[4].rotate_right(11) ^ a[4].rotate_right(25);
                        let ch = (a[4] & a[5]) ^ (!a[4] & a[6]);
                        let t1 = a[7]
                            .wrapping_add(s1)
                            .wrapping_add(ch)
                            .wrapping_add(K[i])
                            .wrapping_add(w[i]);
                        let s0 =
                            a[0].rotate_right(2) ^ a[0].rotate_right(13) ^ a[0].rotate_right(22);
                        let maj = (a[0] & a[1]) ^ (a[0] & a[2]) ^ (a[1] & a[2]);
                        let t2 = s0.wrapping_add(maj);
                        a = [
                            t1.wrapping_add(t2),
                            a[0],
                            a[1],
                            a[2],
                            a[3].wrapping_add(t1),
                            a[4],
                            a[5],
                            a[6],
                        ];
                    }
                    for i in 0..8 {
                        h[i] = h[i].wrapping_add(a[i]);
                    }
                }
                let mut out = [0u8; 32];
                for i in 0..8 {
                    out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
                }
                out
            }
        }
    }

    async fn post(state: &AppState, event: &str, sig: Option<String>, body: &str) -> StatusCode {
        let mut r = Request::post("/webhook/github").header("x-github-event", event);
        if let Some(s) = sig {
            r = r.header("x-hub-signature-256", s);
        }
        router(state.clone())
            .oneshot(r.body(Body::from(body.to_string())).unwrap_or_default())
            .await
            .map(|r| r.status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    #[tokio::test]
    async fn webhooks() {
        // No secret: the route doesn't exist.
        assert_eq!(
            post(&AppState::new(), "push", None, "{}").await,
            StatusCode::NOT_FOUND
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let state = AppState::new().with_webhook(Webhook {
            secret: b"s3cret".to_vec(),
            targets: vec![WebhookTarget {
                source_id: "app".into(),
                repo: "acme/app-repo".into(),
                branch: None,
            }],
            wake: tx,
        });
        let push = r#"{"ref":"refs/heads/main","repository":{"full_name":"Acme/App-Repo","default_branch":"main"}}"#;
        assert_eq!(
            post(&state, "push", None, push).await,
            StatusCode::UNAUTHORIZED,
            "missing"
        );
        assert_eq!(
            post(&state, "push", Some("sha256=00".into()), push).await,
            StatusCode::UNAUTHORIZED,
            "bad"
        );
        assert_eq!(
            post(&state, "push", Some(signed(b"other", push)), push).await,
            StatusCode::UNAUTHORIZED,
            "other secret"
        );
        assert_eq!(
            post(&state, "push", Some(signed(b"s3cret", push)), push).await,
            StatusCode::ACCEPTED
        );
        assert_eq!(rx.try_recv().ok().as_deref(), Some("app"));
        let other_branch = push.replace("refs/heads/main", "refs/heads/feature");
        assert_eq!(
            post(
                &state,
                "push",
                Some(signed(b"s3cret", &other_branch)),
                &other_branch
            )
            .await,
            StatusCode::NO_CONTENT
        );
        let ping = r#"{"zen":"hi"}"#;
        assert_eq!(
            post(&state, "ping", Some(signed(b"s3cret", ping)), ping).await,
            StatusCode::NO_CONTENT
        );
        assert!(rx.try_recv().is_err(), "nothing else woke a source");
    }

    #[tokio::test]
    async fn health_and_readiness() {
        let s = AppState::new();
        assert_eq!(status(&s, "/healthz").await, StatusCode::OK);
        assert_eq!(status(&s, "/readyz").await, StatusCode::SERVICE_UNAVAILABLE);
        s.set_ready(true);
        assert_eq!(status(&s, "/readyz").await, StatusCode::OK);
        assert_eq!(status(&s, "/nope").await, StatusCode::NOT_FOUND);
    }
}
