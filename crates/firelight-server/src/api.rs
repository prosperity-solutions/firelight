//! `/api/v1` (§16): the MCP tools as plain JSON over GET.
//!
//! | route | MCP tool |
//! |---|---|
//! | `GET /api/v1/search?q=&space=&kind=&repo=&limit=` | `search` |
//! | `GET /api/v1/entries?space=&kind=&repo=&tag=` | `list_entries` |
//! | `GET /api/v1/entries/{repo}/{id}` | `get_entry` |
//! | `GET /api/v1/links?q=&space=&island=&card=` | `find_link` |
//! | `GET /api/v1/spaces` | `list_spaces` |
//! | `GET /api/v1/repos` | `list_repos` |
//! | `GET /api/v1/repos/{id}` | `get_repo` |
//! | `GET /api/v1/status` | `status` |

use std::collections::BTreeMap;

use axum::Router;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;

use crate::AppState;
use crate::data::{self, Ctx, Error};

/// Query parameters; malformed ones (bad percent-encoding) are a JSON 400.
struct Q(BTreeMap<String, String>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Q {
    type Rejection = Response;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Query::<BTreeMap<String, String>>::try_from_uri(&parts.uri)
            .map(|q| Q(q.0))
            .map_err(|_| bad("the query string is malformed"))
    }
}

fn bad(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": msg }))).into_response()
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/search", get(search))
        .route("/api/v1/entries", get(entries))
        .route("/api/v1/entries/{repo}/{id}", get(entry))
        .route("/api/v1/links", get(links))
        .route("/api/v1/spaces", get(spaces))
        .route("/api/v1/repos", get(repos))
        .route("/api/v1/repos/{id}", get(repo))
        .route("/api/v1/status", get(status))
}

/// Run a data question off the async workers (search may embed the query).
async fn answer(
    state: AppState,
    f: impl FnOnce(&Ctx<'_>) -> data::Result + Send + 'static,
) -> Response {
    let Some(content) = state.content.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({ "error": "starting" })),
        )
            .into_response();
    };
    let base = state.ui.public_url.trim_end_matches('/').to_string();
    let version = state.ui.version.clone();
    let res = tokio::task::spawn_blocking(move || {
        let ctx = Ctx {
            content: content.as_ref(),
            base: &base,
            version: &version,
        };
        f(&ctx)
    })
    .await;
    match res {
        Ok(Ok(v)) => axum::Json(v).into_response(),
        Ok(Err(Error::BadInput(e))) => {
            (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": e }))).into_response()
        }
        Ok(Err(Error::NotFound(e))) => {
            (StatusCode::NOT_FOUND, axum::Json(json!({ "error": e }))).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn opt(q: &BTreeMap<String, String>, k: &str) -> Option<String> {
    q.get(k).filter(|v| !v.is_empty()).cloned()
}

async fn search(State(s): State<AppState>, Q(q): Q) -> Response {
    let limit = match opt(&q, "limit").map(|l| l.parse::<usize>()) {
        Some(Err(_)) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({ "error": "limit must be a number" })),
            )
                .into_response();
        }
        Some(Ok(n)) => Some(n),
        None => None,
    };
    answer(s, move |c| {
        data::search(
            c,
            q.get("q").map(String::as_str).unwrap_or(""),
            opt(&q, "space").as_deref(),
            opt(&q, "kind").as_deref(),
            opt(&q, "repo").as_deref(),
            limit,
        )
    })
    .await
}

async fn entries(State(s): State<AppState>, Q(q): Q) -> Response {
    answer(s, move |c| {
        data::list_entries(
            c,
            opt(&q, "space").as_deref(),
            opt(&q, "kind").as_deref(),
            opt(&q, "repo").as_deref(),
            opt(&q, "tag").as_deref(),
        )
    })
    .await
}

async fn entry(
    State(s): State<AppState>,
    p: Result<Path<(String, String)>, PathRejection>,
) -> Response {
    let Ok(Path((repo, id))) = p else {
        return bad("the path is malformed");
    };
    answer(s, move |c| data::get_entry(c, &format!("{repo}/{id}"))).await
}

async fn links(State(s): State<AppState>, Q(q): Q) -> Response {
    answer(s, move |c| {
        data::find_link(
            c,
            opt(&q, "q").as_deref(),
            opt(&q, "space").as_deref(),
            opt(&q, "island").as_deref(),
            opt(&q, "card").as_deref(),
        )
    })
    .await
}

async fn spaces(State(s): State<AppState>) -> Response {
    answer(s, data::list_spaces).await
}

async fn repos(State(s): State<AppState>) -> Response {
    answer(s, data::list_repos).await
}

async fn repo(State(s): State<AppState>, p: Result<Path<String>, PathRejection>) -> Response {
    let Ok(Path(id)) = p else {
        return bad("the path is malformed");
    };
    answer(s, move |c| data::get_repo(c, &id)).await
}

async fn status(State(s): State<AppState>) -> Response {
    answer(s, data::status).await
}
