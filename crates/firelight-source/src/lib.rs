//! Sources (§6): `dir` folders, `git` repos over HTTPS, and `github` repos
//! (GitHub App, token or anonymous); polling, GitHub App tokens, webhook
//! verification, and the last-good rule.

pub mod dir;
pub mod git;
pub mod github;
pub mod pull;

pub use dir::{DirError, read_firelight_dir};

/// The one HTTP client builder: rustls with the `ring` provider (§19: no
/// aws-lc, no OpenSSL), installed once per process.
pub fn http_client() -> reqwest::Client {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
        .user_agent(concat!("firelight/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// The time, injected so tests control it (§14).
pub trait Clock: Send + Sync {
    fn now_unix(&self) -> i64;
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }
}

/// RFC 3339 → Unix seconds (`2026-11-05T10:00:00Z`, `…+01:00`, fractions).
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (
        n(0..4)?,
        n(5..7)?,
        n(8..10)?,
        n(11..13)?,
        n(14..16)?,
        n(17..19)?,
    );
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let off = match rest {
        "Z" | "z" => 0,
        r if r.len() == 6 => {
            let o =
                r.get(1..3)?.parse::<i64>().ok()? * 3600 + r.get(4..6)?.parse::<i64>().ok()? * 60;
            if r.starts_with('-') { -o } else { o }
        }
        _ => return None,
    };
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - off)
}

/// Unix seconds → `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_rfc3339(t: i64) -> String {
    let z = t.div_euclid(86_400) + 719_468;
    let secs = t.rem_euclid(86_400);
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn times() {
        assert_eq!(super::parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            super::parse_rfc3339("2026-11-05T11:00:00+01:00"),
            super::parse_rfc3339("2026-11-05T10:00:00Z")
        );
        let t = super::parse_rfc3339("2026-02-28T23:59:59Z").unwrap_or(0);
        assert_eq!(super::format_rfc3339(t), "2026-02-28T23:59:59Z");
        assert_eq!(super::format_rfc3339(t + 1), "2026-03-01T00:00:00Z");
        assert!(super::parse_rfc3339("yesterday").is_none());
    }
}
