//! Template islands: inputs that turn into links (§5.4).
//!
//! A template URL may use a `{name}` placeholder only in three positions, so a
//! value can never change the scheme or the registered domain:
//! 1. inside the host's **first label** (`https://app-{pr}.preview.acme.example`);
//!    the host must then have at least three labels, use only letters, digits,
//!    dots and dashes around the placeholder, and not end in a number (an IP);
//! 2. as **one whole path segment** (`/runs/{id}/logs`);
//! 3. as **one whole query value** (`?pr={pr}`).
//!
//! Filling a template checks each value against its anchored `pattern`, then
//! per position: a host value must also match `^[a-z0-9-]{1,63}$`; a path value
//! is percent-encoded as one segment and `.`/`..` are refused; a query value is
//! percent-encoded as a query component.

use std::collections::BTreeMap;

/// Where a placeholder sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Host,
    Path,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Lit(String),
    Var(String, Position),
}

/// A checked template URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pieces: Vec<Piece>,
}

impl Url {
    /// The placeholder names it uses, in order.
    pub fn names(&self) -> Vec<&str> {
        self.pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Var(n, _) => Some(n.as_str()),
                Piece::Lit(_) => None,
            })
            .collect()
    }
}

/// Input names: `^[a-z][a-z0-9_]{0,31}$`.
pub fn valid_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// Split `s` into literals and `{name}` placeholders, all at `pos`.
fn split_vars(s: &str, pos: Position, out: &mut Vec<Piece>) -> Result<(), String> {
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        if i > 0 {
            out.push(Piece::Lit(rest[..i].to_string()));
        }
        let after = &rest[i + 1..];
        let j = after
            .find('}')
            .ok_or_else(|| format!("unclosed {{ in {s:?}"))?;
        let name = &after[..j];
        if !valid_name(name) {
            return Err(format!(
                "{{{name}}} is not a placeholder name: lowercase letters, digits and _"
            ));
        }
        out.push(Piece::Var(name.to_string(), pos));
        rest = &after[j + 1..];
    }
    if rest.contains('}') {
        return Err(format!("stray }} in {s:?}"));
    }
    if !rest.is_empty() {
        out.push(Piece::Lit(rest.to_string()));
    }
    Ok(())
}

/// Check a template URL against the placement rules and the input names.
pub fn check_url(url: &str, inputs: &[&str]) -> Result<Url, String> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        ("https://", r)
    } else if let Some(r) = url.strip_prefix("http://") {
        ("http://", r)
    } else {
        return Err(format!("{url:?} must start with https:// or http://"));
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.contains('@') {
        return Err("a template URL may not contain a user name or password".into());
    }
    let mut pieces = vec![Piece::Lit(scheme.to_string())];
    // Host: placeholders only in the first label.
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.contains(']') || h.ends_with(']') => (h, Some(p)),
        _ => (authority, None),
    };
    if host.is_empty() {
        return Err(format!("{url:?} has no host"));
    }
    // Only plain host characters around the placeholders: a `\` (which
    // browsers read as `/`), `%`, `@` or anything else could let a value
    // escape into the authority.
    let mut around = Vec::new();
    split_vars(host, Position::Host, &mut around)?;
    let plain: String = around
        .iter()
        .filter_map(|p| match p {
            Piece::Lit(l) => Some(l.as_str()),
            Piece::Var(..) => None,
        })
        .collect();
    if !plain
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(format!(
            "the host {host:?} may only use letters, digits, dots and dashes around a placeholder"
        ));
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.iter().any(|l| l.is_empty()) {
        return Err(format!("the host {host:?} has an empty label"));
    }
    for (i, label) in labels.iter().enumerate() {
        if i > 0 && label.contains(['{', '}']) {
            return Err(format!(
                "a placeholder may only sit in the host's first label, so a value can't change the domain; {label:?} isn't the first label"
            ));
        }
    }
    if labels[0].contains('{') && labels.len() < 3 {
        return Err(format!(
            "with a placeholder in the host, the host needs at least three labels (like app-{{pr}}.preview.acme.example), so the value can't become the registered domain; {host:?} has {}",
            labels.len()
        ));
    }
    if labels[0].contains('{')
        && labels
            .last()
            .is_some_and(|l| l.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(format!(
            "the host {host:?} ends in a number, so a value could turn it into an IP address"
        ));
    }
    split_vars(labels[0], Position::Host, &mut pieces)?;
    let rest_host = &host[labels[0].len()..];
    let mut lit = rest_host.to_string();
    if let Some(p) = port {
        if p.contains(['{', '}']) || !p.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!(
                "the port {p:?} must be digits, without placeholders"
            ));
        }
        lit.push(':');
        lit.push_str(p);
    }
    pieces.push(Piece::Lit(lit));
    // Path, query, fragment.
    let (before_frag, frag) = match tail.split_once('#') {
        Some((a, f)) => (a, Some(f)),
        None => (tail, None),
    };
    let (path, query) = match before_frag.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (before_frag, None),
    };
    for (i, seg) in path.split('/').enumerate() {
        if i > 0 {
            pieces.push(Piece::Lit("/".into()));
        }
        if seg.contains(['{', '}']) {
            let name = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}'));
            match name {
                Some(n) if valid_name(n) => pieces.push(Piece::Var(n.to_string(), Position::Path)),
                _ => {
                    return Err(format!(
                        "in the path a placeholder must be one whole segment, like /runs/{{id}}/; {seg:?} isn't"
                    ));
                }
            }
        } else {
            pieces.push(Piece::Lit(seg.to_string()));
        }
    }
    if let Some(q) = query {
        pieces.push(Piece::Lit("?".into()));
        for (i, pair) in q.split('&').enumerate() {
            if i > 0 {
                pieces.push(Piece::Lit("&".into()));
            }
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if k.contains(['{', '}']) {
                return Err(format!("a placeholder may not be a query key ({k:?})"));
            }
            pieces.push(Piece::Lit(if pair.contains('=') {
                format!("{k}=")
            } else {
                k.to_string()
            }));
            if v.contains(['{', '}']) {
                let name = v.strip_prefix('{').and_then(|s| s.strip_suffix('}'));
                match name {
                    Some(n) if valid_name(n) => {
                        pieces.push(Piece::Var(n.to_string(), Position::Query))
                    }
                    _ => {
                        return Err(format!(
                            "in the query a placeholder must be one whole value, like ?pr={{pr}}; {v:?} isn't"
                        ));
                    }
                }
            } else {
                pieces.push(Piece::Lit(v.to_string()));
            }
        }
    }
    if let Some(f) = frag {
        if f.contains(['{', '}']) {
            return Err("a placeholder may not sit in the #fragment".into());
        }
        pieces.push(Piece::Lit(format!("#{f}")));
    }
    let u = Url { pieces };
    for n in u.names() {
        if !inputs.contains(&n) {
            return Err(format!(
                "{{{n}}} is not one of this island's inputs ({})",
                inputs.join(", ")
            ));
        }
    }
    Ok(u)
}

/// An input pattern must be anchored and compile (with a size cap). It is
/// compiled as `^(?:…)$`, so `^a|b$` still has to match the whole value.
pub fn check_pattern(p: &str) -> Result<regex::Regex, String> {
    if !p.starts_with('^') || !p.ends_with('$') {
        return Err(format!(
            "{p:?} must be anchored: start with ^ and end with $"
        ));
    }
    regex::RegexBuilder::new(&format!("^(?:{p})$"))
        .size_limit(64 << 10)
        .build()
        .map_err(|e| format!("{p:?} is not a valid pattern: {e}"))
}

fn is_host_value(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 63
        && v.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// Percent-encode everything but RFC 3986 unreserved characters. Safe as one
/// path segment and as one query component.
pub fn encode(v: &str) -> String {
    let mut s = String::new();
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Fill a checked URL. `patterns` maps each input name to its compiled
/// pattern. Every value must be present and valid; nothing renders otherwise.
pub fn fill(
    url: &Url,
    values: &BTreeMap<String, String>,
    patterns: &BTreeMap<String, regex::Regex>,
) -> Result<String, String> {
    let mut out = String::new();
    for p in &url.pieces {
        match p {
            Piece::Lit(s) => out.push_str(s),
            Piece::Var(name, pos) => {
                let v = values.get(name).map(String::as_str).unwrap_or("");
                if v.is_empty() {
                    return Err(format!("{name} is empty"));
                }
                match patterns.get(name) {
                    Some(re) if re.is_match(v) => {}
                    _ => return Err(format!("{name} doesn't match its pattern")),
                }
                match pos {
                    Position::Host => {
                        if !is_host_value(v) {
                            return Err(format!("{name} can only use a-z, 0-9 and - here"));
                        }
                        out.push_str(v);
                    }
                    Position::Path => {
                        if v == "." || v == ".." {
                            return Err(format!("{name} can't be . or .."));
                        }
                        out.push_str(&encode(v));
                    }
                    Position::Query => out.push_str(&encode(v)),
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pats(p: &str) -> BTreeMap<String, regex::Regex> {
        let mut m = BTreeMap::new();
        m.insert(
            "v".to_string(),
            check_pattern(p).unwrap_or_else(|e| panic!("{e}")),
        );
        m
    }
    fn vals(v: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("v".to_string(), v.to_string())])
    }

    #[test]
    fn placement_rules() {
        for ok in [
            "https://app-{v}.preview.acme.example",
            "https://{v}.preview.acme.example/x",
            "https://ci.acme.example/runs/{v}/logs",
            "https://ci.acme.example/search?q={v}&x=1",
            "http://ci.acme.example:8080/{v}",
            "https://ci.acme.example/#top",
        ] {
            assert!(
                check_url(ok, &["v"]).is_ok(),
                "{ok}: {:?}",
                check_url(ok, &["v"])
            );
        }
        for bad in [
            "{v}://x.acme.example",
            "javascript:{v}",
            "https://{v}.example",
            "https://x.{v}.example",
            "https://acme.example.{v}",
            "https://acme.example:{v}/",
            "https://u:{v}@acme.example/",
            "https://ci.acme.example/runs-{v}/",
            "https://ci.acme.example/{v}{v}",
            "https://ci.acme.example/?{v}=1",
            "https://ci.acme.example/?q=a{v}",
            "https://ci.acme.example/#{v}",
            "https://ci.acme.example/{other}",
            "https://ci.acme.example/{Bad}",
            "https://ci.acme.example/{v",
            "https://{v}\\.preview.acme.example/",
            "https://{v}.preview.acme.example\\x/",
            "https://a-{v}.b.10.20/",
            "https://{v}%2e.preview.acme.example/",
            "https://{v}..acme.example/",
        ] {
            assert!(check_url(bad, &["v"]).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn patterns_are_anchored() {
        assert!(check_pattern("^[0-9]+$").is_ok());
        assert!(check_pattern("[0-9]+").is_err());
        assert!(check_pattern("^[0-9]+").is_err());
        assert!(check_pattern("^(unclosed$").is_err());
        // Alternation can't escape the anchors.
        let re = check_pattern("^a|b$").unwrap_or_else(|e| panic!("{e}"));
        assert!(re.is_match("a") && re.is_match("b"));
        assert!(!re.is_match("ab") && !re.is_match("xb") && !re.is_match("ax"));
    }

    /// A test per position: dot-segments, /, %, @, #, ? and Unicode (§5.4).
    #[test]
    fn encoding_per_position() {
        let any = pats("^.+$");
        let host = check_url("https://app-{v}.preview.acme.example/", &["v"])
            .unwrap_or_else(|e| panic!("{e}"));
        let path = check_url("https://ci.acme.example/runs/{v}/logs", &["v"])
            .unwrap_or_else(|e| panic!("{e}"));
        let query =
            check_url("https://ci.acme.example/s?q={v}", &["v"]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fill(&host, &vals("pr-12"), &any),
            Ok("https://app-pr-12.preview.acme.example/".into())
        );
        for evil in [
            "..",
            ".",
            "a/b",
            "a%2e",
            "a@b",
            "a#b",
            "a?b",
            "ünï",
            "evil.example",
            "A",
        ] {
            assert!(fill(&host, &vals(evil), &any).is_err(), "host {evil}");
        }
        assert_eq!(
            fill(&path, &vals("a/b"), &any),
            Ok("https://ci.acme.example/runs/a%2Fb/logs".into())
        );
        assert_eq!(
            fill(&path, &vals("50%"), &any),
            Ok("https://ci.acme.example/runs/50%25/logs".into())
        );
        assert_eq!(
            fill(&path, &vals("a@b#c?d"), &any),
            Ok("https://ci.acme.example/runs/a%40b%23c%3Fd/logs".into())
        );
        assert_eq!(
            fill(&path, &vals("ü"), &any),
            Ok("https://ci.acme.example/runs/%C3%BC/logs".into())
        );
        assert!(fill(&path, &vals(".."), &any).is_err());
        assert!(fill(&path, &vals("."), &any).is_err());
        assert_eq!(
            fill(&path, &vals("..."), &any),
            Ok("https://ci.acme.example/runs/.../logs".into())
        );
        assert_eq!(
            fill(&query, &vals("a&b=c#d"), &any),
            Ok("https://ci.acme.example/s?q=a%26b%3Dc%23d".into())
        );
        assert_eq!(
            fill(&query, &vals("../x"), &any),
            Ok("https://ci.acme.example/s?q=..%2Fx".into())
        );
        assert_eq!(
            fill(&query, &vals("ü?"), &any),
            Ok("https://ci.acme.example/s?q=%C3%BC%3F".into())
        );
        // The pattern comes first.
        assert!(fill(&path, &vals("abc"), &pats("^[0-9]{1,6}$")).is_err());
        assert!(fill(&path, &vals(""), &any).is_err());
        assert!(fill(&path, &BTreeMap::new(), &any).is_err());
    }
}
