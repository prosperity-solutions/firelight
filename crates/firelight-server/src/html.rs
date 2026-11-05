//! HTML building with escaping by default (§17.1). Every string from a repo
//! goes through [`esc`] (text) or [`attr`] (attribute values); URLs also go
//! through [`safe_url`], which allow-lists schemes.

use std::fmt::Write as _;

/// Escape text for an HTML text node.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

/// Escape for a double-quoted attribute value (same set, so it is safe in
/// either context).
pub fn attr(s: &str) -> String {
    esc(s)
}

/// A URL a repo gave us, if its scheme is allowed: `https:`, `http:`,
/// `mailto:`, a site-absolute path (`/e/…`, built by us) or a `#fragment`.
/// Anything else (`javascript:`, `data:`, protocol-relative `//host`) is
/// refused. Control characters and whitespace are refused too.
pub fn safe_url(u: &str) -> Option<&str> {
    if u.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = u.to_ascii_lowercase();
    let ok = lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || (u.starts_with('/') && !u.starts_with("//") && !u.starts_with("/\\"))
        || u.starts_with('#');
    ok.then_some(u)
}

/// A tiny HTML writer: push raw markup we wrote, or escaped text.
#[derive(Default)]
pub struct H(pub String);

impl H {
    pub fn new() -> Self {
        H(String::new())
    }
    /// Markup written by Firelight itself (never repo text).
    pub fn raw(&mut self, s: &str) -> &mut Self {
        self.0.push_str(s);
        self
    }
    pub fn text(&mut self, s: &str) -> &mut Self {
        self.0.push_str(&esc(s));
        self
    }
    pub fn fmt(&mut self, args: std::fmt::Arguments<'_>) -> &mut Self {
        let _ = self.0.write_fmt(args);
        self
    }
    pub fn finish(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(
            esc(r#"<script>alert("x")</script> & 'y'"#),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &#39;y&#39;"
        );
        assert_eq!(attr(r#"" onmouseover="x"#), "&quot; onmouseover=&quot;x");
    }

    #[test]
    fn urls() {
        for ok in [
            "https://a.acme.example/x?y=1#z",
            "http://a.acme.example",
            "mailto:a@acme.example",
            "/e/repo/id#h",
            "#top",
        ] {
            assert_eq!(safe_url(ok), Some(ok), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:text/html,x",
            "//evil.example/x",
            "/\\evil.example",
            "vbscript:x",
            "file:///etc/passwd",
            "https://a.acme.example/\nx",
            "java\tscript:x",
            " javascript:x",
            "relative/path",
        ] {
            assert_eq!(safe_url(bad), None, "{bad}");
        }
    }
}
