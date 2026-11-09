//! Page routes (§18) and the security headers on every response (§17.1).

use std::collections::BTreeMap;

use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;

use firelight_format::catalog::PlacedKind;
use firelight_format::{Ref, SPACE_ALL};

use crate::AppState;
use crate::content::Content;
use crate::html::{H, attr, esc};
use crate::pages;

pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// CSP and friends on every response.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert("content-security-policy", HeaderValue::from_static(CSP));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    res
}

pub fn pages() -> Router<AppState> {
    Router::new()
        .route("/", get(home))
        .route("/s/{space}", get(space))
        .route("/s/{space}/search", get(search))
        .route("/e/{repo}/{id}", get(entry))
        .route("/r/{repo}", get(repo))
        .route("/i/{repo}/{id}", get(island))
        .route("/assets/theme.css", get(theme))
        .route("/assets/{file}", get(asset))
        .route("/__live", get(live))
        .route("/spec", get(spec))
        .route("/status", get(status))
}

#[allow(
    clippy::result_large_err,
    reason = "an axum Response is the natural early return of a handler"
)]
fn content(state: &AppState) -> Result<&dyn Content, Response> {
    state
        .content
        .as_deref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "starting\n").into_response())
}

fn not_found(state: &AppState, c: &dyn Content, what: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(pages::message_page(&state.ui, c, "Not here", what)),
    )
        .into_response()
}

async fn home(State(state): State<AppState>) -> Response {
    let c = match content(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let cat = c.catalog();
    if let Some(d) = &state.ui.default_space {
        if d == SPACE_ALL || cat.space(d).is_some() {
            return Redirect::to(&format!("/s/{d}")).into_response();
        }
        // Never a startup error (§5.7): a warning, and the space list.
        tracing::warn!(space = %d, "ui.default_space names a space that doesn't exist; showing the space list");
    }
    let spaces = pages::visible_spaces(&cat);
    if spaces.len() == 1 {
        return Redirect::to(&format!("/s/{}", spaces[0].id)).into_response();
    }
    Html(pages::space_index(&state.ui, c)).into_response()
}

async fn space(State(state): State<AppState>, Path(space): Path<String>) -> Response {
    let c = match content(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    match pages::space_page(&state.ui, c, &space, &BTreeMap::new()) {
        Some(p) => Html(p).into_response(),
        None => not_found(&state, c, &format!("There is no space {space:?}.")),
    }
}

async fn search(
    State(state): State<AppState>,
    Path(space): Path<String>,
    Query(q): Query<BTreeMap<String, String>>,
) -> Response {
    let c = match content(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    if space != SPACE_ALL && c.catalog().space(&space).is_none() {
        return not_found(&state, c, &format!("There is no space {space:?}."));
    }
    let query = q
        .get("q")
        .map(String::as_str)
        .unwrap_or("")
        .trim()
        .chars()
        .take(200)
        .collect::<String>();
    // Search may embed the query: off the async workers.
    let Some(content) = state.content.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "starting\n").into_response();
    };
    let json = q.get("format").map(String::as_str) == Some("json");
    let ui = state.ui.clone();
    let span = tracing::Span::current();
    let res = tokio::task::spawn_blocking(move || {
        let _in = span.enter();
        if json {
            axum::Json(pages::search_json(content.as_ref(), &space, &query)).into_response()
        } else {
            Html(pages::search_page(&ui, content.as_ref(), &space, &query)).into_response()
        }
    })
    .await;
    res.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn entry(
    State(state): State<AppState>,
    Path((repo, id)): Path<(String, String)>,
    Query(q): Query<BTreeMap<String, String>>,
) -> Response {
    let c = match content(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let r = Ref { repo, id };
    match pages::entry_page(&state.ui, c, &r, q.get("s").map(String::as_str)) {
        Some(p) => Html(p).into_response(),
        None => not_found(&state, c, &format!("There is no entry {r}.")),
    }
}

async fn repo(State(state): State<AppState>, Path(repo): Path<String>) -> Response {
    let c = match content(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    match pages::repo_page(&state.ui, c, &repo) {
        Some(p) => Html(p).into_response(),
        None => not_found(&state, c, &format!("There is no repo {repo:?}.")),
    }
}

/// `/i/<repo>/<id>?<name>=<value>…`: a template island, filled on the server
/// (works without JS), or `&format=json` for the live island.
async fn island(
    State(state): State<AppState>,
    Path((repo, id)): Path<(String, String)>,
    Query(q): Query<BTreeMap<String, String>>,
) -> Response {
    let c = match content(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let cat = c.catalog();
    let r = Ref { repo, id };
    let found = cat.spaces.iter().find_map(|s| {
        s.islands
            .iter()
            .find(|i| i.r == r)
            .and_then(|i| match &i.kind {
                PlacedKind::Template(t) => Some((s.id.clone(), t.clone())),
                _ => None,
            })
    });
    let Some((home_space, t)) = found else {
        return not_found(&state, c, &format!("There is no template island {r}."));
    };
    let values: BTreeMap<String, String> = t
        .inputs
        .iter()
        .filter_map(|i| {
            q.get(&i.name)
                .map(|v| (i.name.clone(), v.chars().take(500).collect()))
        })
        .collect();
    let fill = pages::fill_template(&t, &values);
    if q.get("format").map(String::as_str) == Some("json") {
        return axum::Json(match &fill.links {
            Ok(links) => serde_json::json!({ "ok": true, "links": links.iter().map(|(l, u, i)| serde_json::json!({"label": l, "url": u, "icon": i})).collect::<Vec<_>>() }),
            Err(e) => serde_json::json!({ "ok": false, "error": e }),
        })
        .into_response();
    }
    let space = q
        .get("s")
        .filter(|s| *s == SPACE_ALL || cat.space(s).is_some())
        .cloned()
        .unwrap_or(home_space);
    let mut fills = BTreeMap::new();
    fills.insert(r, fill);
    match pages::space_page(&state.ui, c, &space, &fills) {
        Some(p) => Html(p).into_response(),
        None => not_found(&state, c, "That space is gone."),
    }
}

async fn theme(State(state): State<AppState>) -> Response {
    let accent = if state.accent.is_empty() {
        "#d9541e"
    } else {
        &state.accent
    };
    // ui.accent is validated as #rrggbb, so nothing else reaches the CSS.
    let ok = accent.len() == 7
        && accent.starts_with('#')
        && accent[1..].chars().all(|c| c.is_ascii_hexdigit());
    let css = format!(
        ":root {{ --accent: {}; }}\n",
        if ok { accent } else { "#d9541e" }
    );
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        css,
    )
        .into_response()
}

async fn asset(Path(file): Path<String>) -> Response {
    let Some((_, body)) = crate::ASSETS.iter().find(|(n, _)| *n == file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let ct = if file.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if file.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else {
        "image/svg+xml"
    };
    // Assets change with the binary: revalidate, cheap and never stale.
    (
        [
            (header::CONTENT_TYPE, ct),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        *body,
    )
        .into_response()
}

async fn live(State(state): State<AppState>) -> Response {
    if !state.ui.live {
        return StatusCode::NOT_FOUND.into_response();
    }
    let g = state.content.as_ref().map_or(0, |c| c.generation());
    (
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(serde_json::json!({ "generation": g })),
    )
        .into_response()
}

/// `/spec`: the format reference, embedded (§5.6).
pub const FORMAT_MD: &str = include_str!("../../../docs/format.md");

async fn spec(State(state): State<AppState>) -> Response {
    let c = match content(&state) {
        Ok(c) => c,
        Err(r) => return r,
    };
    struct NoRefs;
    impl crate::markdown::Resolve for NoRefs {
        fn link(&self, _: &Ref) -> Option<(String, Option<String>)> {
            None
        }
        fn entry(&self, _: &Ref) -> Option<String> {
            None
        }
    }
    let body = crate::markdown::render(FORMAT_MD, "firelight", &NoRefs);
    let main = format!("<main class=\"entry spec\"><div class=\"prose\">{body}</div></main>");
    Html(pages::layout(
        &state.ui,
        &c.catalog(),
        c,
        None,
        "The .firelight/ format",
        &main,
    ))
    .into_response()
}

/// `/status` (§16.1): the same data as `/api/v1/status` and MCP `status`
/// (already redacted by the content), as a page.
async fn status(State(state): State<AppState>) -> Response {
    let Some(content) = state.content.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "starting\n").into_response();
    };
    let ui = state.ui.clone();
    let page = tokio::task::spawn_blocking(move || {
        let c = content.as_ref();
        let st = c.status();
        let cat = c.catalog();
        let s = |v: &serde_json::Value| -> String {
            match v {
                serde_json::Value::String(x) => x.clone(),
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            }
        };
        let mut m = H::new();
        m.fmt(format_args!(
            "<main class=\"status\"><h1>Status</h1>\n<p>Firelight v{}{}{}</p>\n",
            esc(&ui.version),
            st["started_at"].as_str().map(|t| format!(", started {}", esc(t))).unwrap_or_default(),
            st["format_versions"].as_array().map(|v| format!(", format versions {}", esc(&v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")))).unwrap_or_default(),
        ));
        m.raw("<h2>Sources</h2>\n<table><thead><tr><th>Repo</th><th>From</th><th>Serving</th><th>Last pull</th><th>Health</th></tr></thead><tbody>\n");
        let sources = st["sources"].as_array().cloned().unwrap_or_default();
        for src in &sources {
            let id = s(&src["id"]);
            let commit: String = s(&src["commit"]).chars().take(7).collect();
            let newest: String = s(&src["newest_commit"]).chars().take(7).collect();
            let serving = if !newest.is_empty() && newest != commit {
                format!("{commit} (newest {newest} isn't served: it fails its checks or couldn't be read; the last good commit stays)")
            } else {
                commit
            };
            let pulled = match (src["last_pull_at"].as_str(), src["last_pull_ms"].as_i64()) {
                (Some(t), Some(ms)) => format!("{t} ({ms} ms)"),
                (Some(t), None) => t.to_string(),
                _ => String::new(),
            };
            let from = format!("{} {}{}", s(&src["kind"]), s(&src["url"]), src["ref"].as_str().map(|r| format!(" @ {r}")).unwrap_or_default());
            m.fmt(format_args!(
                "<tr><td><a href=\"/r/{}\">{}</a></td><td><code>{}</code></td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>\n",
                attr(&id), esc(&id), esc(&from), esc(&serving), esc(&pulled), esc(&s(&src["health"])),
            ));
        }
        for r in cat.repos.iter().filter(|r| !sources.iter().any(|x| x["id"] == serde_json::json!(r.id))) {
            m.fmt(format_args!(
                "<tr><td><a href=\"/r/{}\">{}</a></td><td>built in</td><td></td><td></td><td>ok</td></tr>\n",
                attr(&r.id), esc(&r.id)
            ));
        }
        m.raw("</tbody></table>\n");
        // Problems per source: why the newest commit isn't served, warnings.
        for src in &sources {
            let mut lines: Vec<String> = Vec::new();
            if let Some(e) = src["last_pull_error"].as_str() {
                lines.push(format!("pull: {e}"));
            }
            for d in src["errors"].as_array().into_iter().flatten().chain(src["warnings"].as_array().into_iter().flatten()) {
                lines.push(format!("{}:{}:{}: {}: {}", s(&d["path"]), s(&d["line"]), s(&d["col"]), s(&d["severity"]), s(&d["message"])));
            }
            if !lines.is_empty() {
                m.fmt(format_args!("<h3><code>{}</code></h3><ul>\n", esc(&s(&src["id"]))));
                for l in lines {
                    m.fmt(format_args!("<li>{}</li>\n", esc(&l)));
                }
                m.raw("</ul>\n");
            }
        }
        if !cat.diagnostics.is_empty() {
            m.raw("<h2>Catalog warnings</h2><ul>\n");
            for (repo, d) in &cat.diagnostics {
                m.fmt(format_args!("<li><code>{}</code>: {}</li>\n", esc(repo), esc(&d.to_string())));
            }
            m.raw("</ul>\n");
        }
        m.fmt(format_args!("<p>Maybe stale entries: {}</p>\n", st["maybe_stale"].as_u64().unwrap_or(0)));
        let e = &st["embedder"];
        m.raw("<h2>Search</h2>\n<table><tbody>\n");
        let row = |m: &mut H, k: &str, v: &str| {
            if !v.is_empty() {
                m.fmt(format_args!("<tr><th>{}</th><td>{}</td></tr>\n", esc(k), esc(v)));
            }
        };
        row(&mut m, "Embedder", &s(&e["backend"]));
        row(&mut m, "Model", &s(&e["model"]));
        row(&mut m, "Dimensions", &s(&e["dims"]));
        if e["chunks"].is_number() {
            row(&mut m, "Chunks embedded", &format!("{} of {}", s(&e["chunks_embedded"]), s(&e["chunks"])));
        }
        row(&mut m, "Last index", &e["last_index_ms"].as_u64().map(|x| format!("{x} ms")).unwrap_or_default());
        row(&mut m, "Problem", &s(&e["problem"]));
        row(&mut m, "Built-in golden check", &s(&st["golden"]["builtin"]));
        m.raw("</tbody></table>\n");
        let fails = st["golden"]["failures"].as_array().cloned().unwrap_or_default();
        if !fails.is_empty() {
            m.raw("<h3>Golden questions that don't pass</h3><ul>\n");
            for f in &fails {
                m.fmt(format_args!("<li><code>{}</code>: {}</li>\n", esc(&s(&f["repo"])), esc(&s(&f["question"]))));
            }
            m.raw("</ul>\n");
        }
        let skipped = st["golden"]["not_loaded"].as_array().cloned().unwrap_or_default();
        if !skipped.is_empty() {
            m.raw("<h3>Golden expects in repos that aren't loaded (skipped)</h3><ul>\n");
            for f in &skipped {
                m.fmt(format_args!("<li><code>{}</code>: {}</li>\n", esc(&s(&f["repo"])), esc(&s(&f["question"]))));
            }
            m.raw("</ul>\n");
        }
        if st["database"].is_object() {
            m.fmt(format_args!(
                "<h2>Database</h2>\n<p>{} bytes, schema version {}</p>\n",
                s(&st["database"]["size_bytes"]), s(&st["database"]["schema_version"])
            ));
        }
        if let Some(cfg) = st["config"].as_array() {
            m.raw("<h2>Configuration</h2>\n<table><thead><tr><th>Key</th><th>Value</th><th>From</th></tr></thead><tbody>\n");
            for r in cfg {
                m.fmt(format_args!(
                    "<tr><td><code>{}</code></td><td><code>{}</code></td><td>{}</td></tr>\n",
                    esc(&s(&r["key"])), esc(&s(&r["value"])), esc(&s(&r["from"]))
                ));
            }
            m.raw("</tbody></table>\n");
        }
        m.raw("</main>");
        pages::layout(&ui, &cat, c, None, "Status", &m.finish())
    })
    .await;
    match page {
        Ok(p) => Html(p).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
