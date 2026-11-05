//! HTTP: pages, `/api/v1`, `/mcp`, `/webhook`, health and readiness
//! (docs/HANDOVER.md §3, §16, §18).
//!
//! M1 holds the skeleton: the router, `/healthz` and `/readyz`, and graceful
//! shutdown. Later milestones add the routes behind the same [`AppState`].

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;

/// What every route can see.
#[derive(Clone, Default)]
pub struct AppState {
    ready: Arc<AtomicBool>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
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
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
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
