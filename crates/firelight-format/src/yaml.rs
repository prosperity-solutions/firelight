//! YAML with a tight budget (D8: serde-saphyr), mapped to diagnostics.
//!
//! The budget is far below the crate's defaults: a `.firelight/` file is a
//! small, hand-written document, so anything near these limits is an attack
//! or a mistake. A billion-laughs file fails here, before any expansion.

use serde::de::DeserializeOwned;

use crate::diag::Diagnostic;

fn options() -> serde_saphyr::Options {
    let mut o = serde_saphyr::Options::default();
    o.with_snippet = false;
    o.duplicate_keys = serde_saphyr::DuplicateKeyPolicy::Error;
    let mut b = serde_saphyr::Budget::default();
    b.max_events = 100_000;
    b.max_nodes = 20_000;
    b.max_depth = 24;
    b.max_aliases = 64;
    b.max_anchors = 64;
    b.max_documents = 1;
    b.max_merge_keys = 64;
    b.max_total_scalar_bytes = 1 << 20;
    b.max_recorded_anchor_events = 10_000;
    b.max_recorded_anchor_bytes = 256 << 10;
    o.budget = Some(b);
    o
}

/// Parse `text`; on failure one diagnostic at the error's position, shifted by
/// `line_offset` (front matter starts below the `---` line).
pub(crate) fn parse<T: DeserializeOwned>(
    text: &str,
    path: &str,
    line_offset: u32,
) -> Result<T, Diagnostic> {
    serde_saphyr::from_str_with_options::<T>(text, options()).map_err(|e| {
        let (line, col) = e
            .location()
            .map(|l| {
                (
                    u32::try_from(l.line()).unwrap_or(0),
                    u32::try_from(l.column()).unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        let line = if line > 0 { line + line_offset } else { 0 };
        Diagnostic::error(
            path,
            line,
            col,
            code_for(&e.to_string()),
            clean(&e.to_string()),
        )
    })
}

fn code_for(msg: &str) -> &'static str {
    if msg.contains("unknown field") {
        "unknown-key"
    } else if msg.contains("missing field") {
        "missing-key"
    } else if msg.contains("budget") || msg.contains("limit") || msg.contains("exceed") {
        "yaml-too-complex"
    } else {
        "yaml"
    }
}

/// The library's message without its position suffix and advice that only
/// makes sense to a Rust programmer.
fn clean(msg: &str) -> String {
    let mut m = msg.to_string();
    if let Some(i) = m.rfind(" at line ") {
        m.truncate(i);
    }
    m = m.replace(", set DuplicateKeyPolicy in Options if acceptable", "");
    m = m
        .replace("unknown field `", "unknown key `")
        .replace("missing field `", "missing key `");
    m = m.replace("expected one of ", "expected one of: ");
    if m.contains("mapping values are not allowed") {
        m = format!(
            "{m}: a value that contains \": \" needs quotes, e.g. summary: \"Day one: accounts and access\""
        );
    }
    if m.contains("budget") {
        m = format!(
            "{m}. A .firelight/ file must stay small and simple (anchors and aliases are capped)"
        );
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize, Debug)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct T {
        a: u32,
        #[serde(default)]
        b: Vec<String>,
    }

    #[test]
    fn positions_and_messages() {
        let e = parse::<T>("a: 1\nc: 2\n", "x.yaml", 0)
            .err()
            .unwrap_or_else(|| panic!("should fail"));
        assert_eq!((e.line, e.col, e.code), (2, 1, "unknown-key"));
        assert_eq!(e.message, "unknown key `c`, expected one of: a, b");
        let e = parse::<T>("a: 1\na: 2\n", "x.yaml", 1)
            .err()
            .unwrap_or_else(|| panic!("should fail"));
        assert_eq!(e.line, 3);
        assert_eq!(e.message, "duplicate mapping key: a");
    }

    #[test]
    fn billion_laughs_is_refused() {
        let mut s = String::from(
            "a: 1\nb: &l0 [\"lol\", \"lol\", \"lol\", \"lol\", \"lol\", \"lol\", \"lol\", \"lol\", \"lol\"]\n",
        );
        for i in 1..12 {
            let p = i - 1;
            s.push_str(&format!(
                "x{i}: &l{i} [*l{p}, *l{p}, *l{p}, *l{p}, *l{p}, *l{p}, *l{p}, *l{p}, *l{p}]\n"
            ));
        }
        // Into an untyped value, so the expansion itself is what is limited.
        let r = parse::<serde_json::Value>(&s, "bomb.yaml", 0);
        assert!(r.is_err(), "a billion-laughs document must be refused");
    }
}
