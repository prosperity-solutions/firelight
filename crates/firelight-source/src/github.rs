//! GitHub App authentication (§6.2): app id + private key → a JWT → an
//! installation token per owner, cached until 5 minutes before it expires.
//! The installation is found by owner, never configured. The app needs only
//! *Contents: read* and *Metadata: read*.
//!
//! The JWT is RS256, signed with `ring` (already the TLS provider), so no
//! extra RSA crate is needed.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use tokio::sync::Mutex;

use crate::Clock;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(
        "the GitHub App private key is not a PEM RSA key (BEGIN RSA PRIVATE KEY or BEGIN PRIVATE KEY): {0}"
    )]
    Key(String),
    #[error("GitHub API {what}: {message}")]
    Api { what: String, message: String },
    #[error(
        "the GitHub App is not installed on {0}: install it on that owner (organisation or user), with Contents: read and Metadata: read"
    )]
    NotInstalled(String),
}

/// Parse a PEM private key (PKCS#1 as GitHub issues it, or PKCS#8).
pub fn parse_key(pem: &str) -> Result<RsaKeyPair, AppError> {
    let (label, b64): (String, String) = {
        let mut label = String::new();
        let mut body = String::new();
        let mut inside = false;
        for line in pem.lines().map(str::trim) {
            if let Some(l) = line
                .strip_prefix("-----BEGIN ")
                .and_then(|l| l.strip_suffix("-----"))
            {
                label = l.to_string();
                inside = true;
            } else if line.starts_with("-----END ") {
                break;
            } else if inside {
                body.push_str(line);
            }
        }
        (label, body)
    };
    let der = base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .map_err(|e| AppError::Key(e.to_string()))?;
    match label.as_str() {
        "RSA PRIVATE KEY" => RsaKeyPair::from_der(&der).map_err(|e| AppError::Key(e.to_string())),
        "PRIVATE KEY" => RsaKeyPair::from_pkcs8(&der).map_err(|e| AppError::Key(e.to_string())),
        other => Err(AppError::Key(format!("unexpected PEM label {other:?}"))),
    }
}

fn b64url(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// The app JWT: issued 60 s in the past (clock drift), valid 9 minutes.
pub fn jwt(app_id: u64, key: &RsaKeyPair, now_unix: i64) -> Result<String, AppError> {
    let header = b64url(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = b64url(
        format!(
            r#"{{"iat":{},"exp":{},"iss":"{app_id}"}}"#,
            now_unix - 60,
            now_unix + 540
        )
        .as_bytes(),
    );
    let input = format!("{header}.{claims}");
    let mut sig = vec![0; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        input.as_bytes(),
        &mut sig,
    )
    .map_err(|e| AppError::Key(e.to_string()))?;
    Ok(format!("{input}.{}", b64url(&sig)))
}

#[derive(Clone)]
struct Cached {
    token: String,
    expires_unix: i64,
}

/// Installation tokens per owner, for one GitHub App.
pub struct AppTokens {
    app_id: u64,
    key: RsaKeyPair,
    api: String,
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
    cache: Mutex<HashMap<String, Cached>>,
}

impl std::fmt::Debug for AppTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppTokens(app {})", self.app_id)
    }
}

/// `owner/repo` of a GitHub URL (`https://github.com/acme/app-repo(.git)`).
pub fn owner_repo(url: &str) -> Option<(String, String)> {
    let rest = url.split_once("://")?.1;
    let mut parts = rest.split('/');
    let _host = parts.next()?;
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.trim_end_matches(".git").to_string();
    (!owner.is_empty() && !repo.is_empty()).then_some((owner, repo))
}

impl AppTokens {
    /// `api` is `https://api.github.com` (tests point it at a mock).
    pub fn new(
        app_id: u64,
        private_key_pem: &str,
        api: &str,
        http: reqwest::Client,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, AppError> {
        Ok(AppTokens {
            app_id,
            key: parse_key(private_key_pem)?,
            api: api.trim_end_matches('/').to_string(),
            http,
            clock,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Forget the cached token of `owner` (after git refused it: revoked, or
    /// the installation changed).
    pub async fn invalidate(&self, owner: &str) {
        self.cache.lock().await.remove(owner);
    }

    /// An installation token for the owner of `owner/repo`, from the cache
    /// while it has more than 5 minutes left.
    pub async fn token_for(&self, owner: &str, repo: &str) -> Result<String, AppError> {
        let now = self.clock.now_unix();
        let mut cache = self.cache.lock().await;
        if let Some(c) = cache.get(owner)
            && c.expires_unix - now > 300
        {
            return Ok(c.token.clone());
        }
        let jwt = jwt(self.app_id, &self.key, now)?;
        let api = |what: &str, e: String| AppError::Api {
            what: what.into(),
            message: e,
        };
        let inst = self
            .http
            .get(format!("{}/repos/{owner}/{repo}/installation", self.api))
            .bearer_auth(&jwt)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "firelight")
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| api("installation", e.to_string()))?;
        if inst.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AppError::NotInstalled(owner.to_string()));
        }
        if !inst.status().is_success() {
            return Err(api("installation", format!("HTTP {}", inst.status())));
        }
        let id = inst
            .json::<serde_json::Value>()
            .await
            .map_err(|e| api("installation", e.to_string()))?["id"]
            .as_u64()
            .ok_or_else(|| api("installation", "no id in the response".into()))?;
        let tok = self
            .http
            .post(format!("{}/app/installations/{id}/access_tokens", self.api))
            .bearer_auth(&jwt)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "firelight")
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| api("access_tokens", e.to_string()))?;
        if !tok.status().is_success() {
            return Err(api("access_tokens", format!("HTTP {}", tok.status())));
        }
        let v: serde_json::Value = tok
            .json()
            .await
            .map_err(|e| api("access_tokens", e.to_string()))?;
        let token = v["token"]
            .as_str()
            .ok_or_else(|| api("access_tokens", "no token in the response".into()))?
            .to_string();
        let expires_unix = v["expires_at"]
            .as_str()
            .and_then(crate::parse_rfc3339)
            .unwrap_or(now + 3000);
        cache.insert(
            owner.to_string(),
            Cached {
                token: token.clone(),
                expires_unix,
            },
        );
        Ok(token)
    }
}

/// Verify GitHub's `X-Hub-Signature-256: sha256=<hex>` over the raw body,
/// in constant time.
pub fn verify_webhook(secret: &[u8], body: &[u8], header: Option<&str>) -> bool {
    use hmac::{Hmac, KeyInit, Mac};
    let Some(hex) = header.and_then(|h| h.strip_prefix("sha256=")) else {
        return false;
    };
    let Some(sig) = decode_hex(hex) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<sha2::Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests;
