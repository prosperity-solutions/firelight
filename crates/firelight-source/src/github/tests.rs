//! GitHub App tokens against a local mock (wiremock), and webhook HMAC (§13.2).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use wiremock::matchers::{header_regex, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

/// A throwaway RSA key, made at test time so none sits in the repo (secret
/// scanning would rightly refuse a committed private key).
fn test_key() -> String {
    let out = std::process::Command::new("openssl")
        .args(["genrsa", "2048"])
        .output()
        .unwrap_or_else(|e| panic!("openssl is needed for this test: {e}"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct TestClock(AtomicI64);
impl crate::Clock for TestClock {
    fn now_unix(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

const NOW: i64 = 1_793_000_000; // 2026-10-26

#[test]
fn jwt_shape() {
    let key = parse_key(&test_key()).unwrap_or_else(|e| panic!("{e}"));
    let t = jwt(12345, &key, NOW).unwrap_or_else(|e| panic!("{e}"));
    let parts: Vec<&str> = t.split('.').collect();
    assert_eq!(parts.len(), 3);
    let dec = |s: &str| {
        String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(s)
                .unwrap_or_default(),
        )
        .unwrap_or_default()
    };
    assert_eq!(dec(parts[0]), r#"{"alg":"RS256","typ":"JWT"}"#);
    let claims: serde_json::Value = serde_json::from_str(&dec(parts[1])).unwrap_or_default();
    assert_eq!(claims["iss"], "12345");
    assert_eq!(claims["iat"], NOW - 60);
    assert_eq!(claims["exp"], NOW + 540);
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[2])
            .unwrap_or_default()
            .len(),
        256
    );
    assert!(parse_key("not a key").is_err());
}

#[test]
fn owner_and_repo() {
    assert_eq!(
        owner_repo("https://github.com/acme/app-repo.git"),
        Some(("acme".into(), "app-repo".into()))
    );
    assert_eq!(
        owner_repo("https://github.com/acme/app-repo"),
        Some(("acme".into(), "app-repo".into()))
    );
    assert_eq!(owner_repo("https://github.com/acme"), None);
}

#[tokio::test]
async fn tokens_are_found_by_owner_and_cached_until_five_minutes_before_expiry() {
    let server = MockServer::start().await;
    let expires = crate::format_rfc3339(NOW + 3600);
    Mock::given(method("GET"))
        .and(path("/repos/acme/app-repo/installation"))
        .and(header_regex(
            "authorization",
            r"^Bearer [A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 77 })))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/app/installations/77/access_tokens"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(
                serde_json::json!({ "token": "ghs_test_token", "expires_at": expires }),
            ),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/other/x/installation"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let clock = Arc::new(TestClock(AtomicI64::new(NOW)));
    let app = AppTokens::new(
        12345,
        &test_key(),
        &server.uri(),
        crate::http_client(),
        clock.clone(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        app.token_for("acme", "app-repo").await.ok().as_deref(),
        Some("ghs_test_token")
    );
    // Cached: no new calls while more than 5 minutes are left.
    clock.0.store(NOW + 3000, Ordering::SeqCst);
    assert_eq!(
        app.token_for("acme", "app-repo").await.ok().as_deref(),
        Some("ghs_test_token")
    );
    // 4 minutes left: a new token.
    clock.0.store(NOW + 3360, Ordering::SeqCst);
    assert_eq!(
        app.token_for("acme", "app-repo").await.ok().as_deref(),
        Some("ghs_test_token")
    );
    // Not installed on an owner: a clear error.
    let e = app
        .token_for("other", "x")
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(e.contains("not installed on other"), "{e}");
    server.verify().await;
}

#[test]
fn webhook_signatures() {
    use hmac::{Hmac, KeyInit, Mac};
    let secret = b"webhook-secret";
    let body = br#"{"ref":"refs/heads/main"}"#;
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret).unwrap_or_else(|e| panic!("{e}"));
    mac.update(body);
    let good: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(verify_webhook(
        secret,
        body,
        Some(&format!("sha256={good}"))
    ));
    assert!(!verify_webhook(secret, body, None), "missing");
    assert!(
        !verify_webhook(secret, body, Some(&good)),
        "no sha256= prefix"
    );
    assert!(
        !verify_webhook(secret, b"{}", Some(&format!("sha256={good}"))),
        "another body"
    );
    assert!(
        !verify_webhook(b"other", body, Some(&format!("sha256={good}"))),
        "another secret"
    );
    assert!(!verify_webhook(secret, body, Some("sha256=zz")), "not hex");
    assert!(!verify_webhook(secret, body, Some("sha256=")), "empty");
}
