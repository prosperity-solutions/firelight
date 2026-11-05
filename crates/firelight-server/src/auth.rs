//! Who may ask what (§10): the Host check, the Origin check on `/mcp`, the
//! MCP bearer tokens and the page's basic auth, in one middleware.
//!
//! - `/healthz`, `/readyz` and `/webhook/github` are always open (the webhook
//!   has its HMAC). Everything else passes the Host check first: 421 unless
//!   the host is `public_url`'s, in `allowed_hosts`, or a loopback name where
//!   loopback is allowed. This blocks DNS rebinding even with auth `none`.
//! - `/mcp` (one exact route): an `Origin` header not in the allowed origins gets 403
//!   (no header passes: CLI clients), then the bearer token when
//!   `mcp = "token"`.
//! - Every other path needs basic auth when `ui = "basic"`.
//!
//! Comparisons are constant-time; a failure is logged with the path and the
//! reason, never with what was presented. `X-Forwarded-*` headers are ignored:
//! a reverse proxy must pass the `Host` the client used.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::AppState;

#[derive(Clone, Default)]
pub struct Access {
    /// `ui = "basic"`: user and password.
    pub basic: Option<(String, String)>,
    /// `mcp = "token"`: every accepted token (rotation).
    pub tokens: Option<Vec<String>>,
    /// Off: no Host check (tests built without a public URL).
    pub check_host: bool,
    /// Accepted host names, normalised by [`host_name`].
    pub hosts: Vec<String>,
    /// Loopback names pass too (preview, or a loopback `public_url`).
    pub loopback: bool,
    /// Browser origins allowed on `/mcp`, normalised `scheme://host[:port]`.
    pub origins: Vec<String>,
}

/// `Host: Example.com.:8443` → `example.com`; `[::1]:80` → `::1`. `None`
/// for anything that isn't a plain authority (userinfo, junk).
pub fn host_name(h: &str) -> Option<String> {
    let a: axum::http::uri::Authority = h.trim().parse().ok()?;
    if a.as_str().contains('@') {
        return None;
    }
    let host = a.host();
    let host = host
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .unwrap_or(host);
    let host = host.strip_suffix('.').unwrap_or(host);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// `localhost`, `<name>.localhost`, `127.0.0.1` or `::1` (§10).
pub fn is_loopback_name(h: &str) -> bool {
    h == "localhost"
        || h.strip_suffix(".localhost")
            .is_some_and(|l| !l.is_empty() && !l.ends_with('.'))
        || h == "127.0.0.1"
        || h == "::1"
}

/// The origin of a URL as `scheme://host[:port]` (default ports dropped).
pub fn origin(s: &str) -> Option<String> {
    let u = url::Url::parse(s.trim()).ok()?;
    match u.origin() {
        url::Origin::Tuple(..) => Some(u.origin().ascii_serialization()),
        url::Origin::Opaque(_) => None,
    }
}

/// Constant-time equality (length may leak; content doesn't).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn digest(b: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(b).into()
}

impl Access {
    /// Tokens as stored: blanks dropped (an unset variable in a rotation
    /// list must never make an empty bearer valid).
    pub fn with_tokens(mut self, tokens: Vec<String>) -> Access {
        self.tokens = Some(
            tokens
                .into_iter()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
        );
        self
    }

    pub fn host_ok(&self, headers: &HeaderMap) -> bool {
        if !self.check_host {
            return true;
        }
        let mut all = headers.get_all(header::HOST).iter();
        let (Some(v), None) = (all.next(), all.next()) else {
            return false; // missing or repeated
        };
        let Some(h) = v.to_str().ok().and_then(host_name) else {
            return false;
        };
        self.hosts.contains(&h) || (self.loopback && is_loopback_name(&h))
    }

    pub fn origin_ok(&self, headers: &HeaderMap) -> bool {
        let mut all = headers.get_all(header::ORIGIN).iter();
        let Some(o) = all.next() else {
            return true; // CLI clients send none
        };
        if all.next().is_some() {
            return false;
        }
        match o.to_str().ok().and_then(origin) {
            Some(o) => self.origins.contains(&o),
            None => false, // `null` and junk are present, not absent
        }
    }

    fn credential<'h>(headers: &'h HeaderMap, scheme: &str) -> Option<&'h str> {
        let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
        let (s, rest) = v.trim().split_once(' ')?;
        let rest = rest.trim();
        (s.eq_ignore_ascii_case(scheme) && !rest.is_empty()).then_some(rest)
    }

    pub fn token_ok(&self, headers: &HeaderMap) -> bool {
        let Some(tokens) = &self.tokens else {
            return true;
        };
        let Some(t) = Self::credential(headers, "bearer") else {
            return false;
        };
        let t = digest(t.as_bytes());
        // Every token is compared, so timing doesn't say which one matched.
        tokens
            .iter()
            .fold(false, |ok, x| ct_eq(&digest(x.as_bytes()), &t) | ok)
    }

    pub fn basic_ok(&self, headers: &HeaderMap) -> bool {
        use base64::Engine as _;
        let Some((user, pass)) = &self.basic else {
            return true;
        };
        let Some(b) = Self::credential(headers, "basic") else {
            return false;
        };
        let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(b) else {
            return false;
        };
        let Some(i) = raw.iter().position(|c| *c == b':') else {
            return false;
        };
        let (u, p) = (&raw[..i], &raw[i + 1..]);
        ct_eq(&digest(u), &digest(user.as_bytes())) & ct_eq(&digest(p), &digest(pass.as_bytes()))
    }
}

fn deny(status: StatusCode, msg: &'static str) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], msg).into_response()
}

/// The Host check, on every route but the open ones.
pub async fn host(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if !state.access.host_ok(req.headers()) {
        tracing::warn!(path = %req.uri().path(), "refused: Host is not this server's (DNS rebinding?)");
        return deny(
            StatusCode::MISDIRECTED_REQUEST,
            "this server doesn't answer to that host name\n",
        );
    }
    next.run(req).await
}

/// `/mcp`: the Origin check, then the bearer token. Basic auth never opens it.
pub async fn mcp(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let a = &state.access;
    if !a.origin_ok(req.headers()) {
        tracing::warn!("refused /mcp: Origin not allowed");
        return deny(StatusCode::FORBIDDEN, "origin not allowed\n");
    }
    if !a.token_ok(req.headers()) {
        tracing::warn!("refused /mcp: missing or wrong token");
        let mut r = deny(StatusCode::UNAUTHORIZED, "a bearer token is required\n");
        r.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"firelight\""),
        );
        return r;
    }
    next.run(req).await
}

/// The page and `/api/v1`: basic auth when `ui = "basic"`. A token never
/// opens them.
pub async fn basic(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if !state.access.basic_ok(req.headers()) {
        if req.headers().contains_key(header::AUTHORIZATION) {
            tracing::warn!(path = %req.uri().path(), "refused: wrong user or password");
        }
        let mut r = deny(StatusCode::UNAUTHORIZED, "login required\n");
        r.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Firelight\", charset=\"UTF-8\""),
        );
        return r;
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(
            host_name("Firelight.Acme.Example.:8443").as_deref(),
            Some("firelight.acme.example")
        );
        assert_eq!(host_name("[::1]:80").as_deref(), Some("::1"));
        assert_eq!(host_name("127.0.0.1").as_deref(), Some("127.0.0.1"));
        assert_eq!(host_name("user@acme.example"), None);
        assert_eq!(host_name("a b"), None);
        assert!(is_loopback_name("app.firelight.localhost"));
        assert!(!is_loopback_name(".localhost"));
        assert!(!is_loopback_name("localhost.acme.example"));
        assert!(!is_loopback_name("127.0.0.1.nip.io"));
        assert!(!is_loopback_name("0.0.0.0"));
    }

    #[test]
    fn origins() {
        assert_eq!(
            origin("HTTPS://App.Acme.Example:443").as_deref(),
            Some("https://app.acme.example")
        );
        assert_eq!(
            origin("http://localhost:8080").as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(origin("null"), None);
    }

    #[test]
    fn constant_time_eq() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
