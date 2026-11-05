//! Entries and `about.md`: front matter, headings and link targets (§5.2).
//!
//! Markdown is CommonMark plus tables and task lists. This module only reads
//! it: headings get stable slugs, `link:` and `entry:` targets are collected,
//! other URLs must be `https:`, `http:`, `mailto:` or a `#fragment`, and images
//! are refused. Raw HTML is not an error here; the renderer escapes it (§17.1).

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::diag::Diagnostic;
use crate::ids::{Ref, Slugger};
use crate::model::{BodyRef, BodyRefKind, Heading, Loc, Markdown};

pub fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS
}

/// Split `---\n<yaml>---\n<body>`. Returns the YAML, the body, and the 1-based
/// line where the body starts.
pub fn split_front_matter(text: &str) -> Option<(&str, &str, u32)> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let mut offset = 0;
    for (line, l) in (2u32..).zip(rest.split_inclusive('\n')) {
        if l.trim_end_matches(['\n', '\r']) == "---" {
            let yaml = &rest[..offset];
            let body = &rest[offset + l.len()..];
            return Some((yaml, body, line + 1));
        }
        offset += l.len();
    }
    None
}

/// What a link destination is allowed to be.
pub enum Dest {
    /// `link:<ref>`
    Link(Ref),
    /// `entry:<ref>[#heading]`
    Entry(Ref, Option<String>),
    /// `https:`, `http:`, `mailto:` or `#fragment`.
    External,
}

/// Classify a destination; `Err` is the message for a refused one.
pub fn classify(dest: &str, home: &str) -> Result<Dest, String> {
    if let Some(r) = dest.strip_prefix("link:") {
        return Ref::parse(r, home).map(Dest::Link);
    }
    if let Some(r) = dest.strip_prefix("entry:") {
        let (r, h) = match r.split_once('#') {
            Some((r, h)) => (r, Some(h.to_string())),
            None => (r, None),
        };
        return Ref::parse(r, home).map(|r| Dest::Entry(r, h));
    }
    if dest.starts_with('#') {
        return Ok(Dest::External);
    }
    let lower = dest.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
    {
        if dest.chars().any(|c| c.is_control() || c == ' ') {
            return Err(format!("{dest:?} has spaces or control characters"));
        }
        return Ok(Dest::External);
    }
    Err(format!(
        "{dest:?} is not an allowed link: use https:, http: or mailto:, link:<ref> for a link, entry:<ref> for an entry, or #heading"
    ))
}

struct Lines(Vec<usize>);

impl Lines {
    fn new(s: &str) -> Self {
        let mut v = vec![0];
        v.extend(s.match_indices('\n').map(|(i, _)| i + 1));
        Lines(v)
    }
    /// 1-based (line, col) of a byte offset.
    fn at(&self, offset: usize) -> (u32, u32) {
        let i = self.0.partition_point(|&s| s <= offset).saturating_sub(1);
        let col = offset - self.0[i];
        (
            u32::try_from(i + 1).unwrap_or(u32::MAX),
            u32::try_from(col + 1).unwrap_or(u32::MAX),
        )
    }
}

/// Scan a markdown body. `first_line` is the file line `body` starts on.
pub fn scan(body: &str, home: &str, path: &str, first_line: u32) -> (Markdown, Vec<Diagnostic>) {
    let lines = Lines::new(body);
    let mut md = Markdown {
        source: body.to_string(),
        first_line,
        ..Markdown::default()
    };
    let mut diags = Vec::new();
    let mut slugger = Slugger::default();
    let mut heading: Option<(u8, String)> = None;
    let loc = |off: usize| {
        let (l, c) = lines.at(off);
        Loc {
            path: path.to_string(),
            line: l + first_line - 1,
            col: c,
        }
    };
    for (ev, range) in Parser::new_ext(body, options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((level as u8, String::new()))
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, text)) = heading.take() {
                    let slug = slugger.next(&text);
                    md.headings.push(Heading {
                        level,
                        text: text.trim().to_string(),
                        slug,
                    });
                }
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, h)) = heading.as_mut() {
                    h.push_str(&t);
                }
            }
            Event::Start(Tag::Image { .. }) => {
                let l = loc(range.start);
                diags.push(Diagnostic::error(path, l.line, l.col, "image", "images are not supported in v1 (no remote fetches on the page). Link to the image instead"));
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let l = loc(range.start);
                match classify(&dest_url, home) {
                    Ok(Dest::Link(target)) => md.refs.push(BodyRef {
                        kind: BodyRefKind::Link,
                        target,
                        loc: l,
                    }),
                    Ok(Dest::Entry(target, h)) => md.refs.push(BodyRef {
                        kind: BodyRefKind::Entry { heading: h },
                        target,
                        loc: l,
                    }),
                    Ok(Dest::External) => {}
                    Err(msg) => {
                        diags.push(Diagnostic::error(path, l.line, l.col, "link-scheme", msg))
                    }
                }
            }
            _ => {}
        }
    }
    (md, diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter() {
        let t = "---\nformat: 1\ntitle: X\n---\nHello\n";
        let (y, b, line) = split_front_matter(t).unwrap_or_default();
        assert_eq!((y, b, line), ("format: 1\ntitle: X\n", "Hello\n", 5));
        assert!(split_front_matter("no front matter").is_none());
        assert!(split_front_matter("---\nunterminated\n").is_none());
    }

    #[test]
    fn scans() {
        let body = "Intro [portal](link:access-portal) and [model](entry:access-model#what-it-is).\n\n## What it does not do\n\nSee [x](infra-repo/y)\n\n## What it does not do\n\n![pic](https://e.example/p.png) <script>x</script> [ok](https://acme.example) [m](mailto:a@acme.example) [top](#intro) [bad](javascript:alert(1)) [d](data:text/html,x)\n";
        let (md, diags) = scan(body, "infra", "g.md", 5);
        assert_eq!(
            md.headings
                .iter()
                .map(|h| h.slug.as_str())
                .collect::<Vec<_>>(),
            ["what-it-does-not-do", "what-it-does-not-do-2"]
        );
        assert_eq!(md.refs.len(), 2);
        assert_eq!(md.refs[0].target.to_string(), "infra/access-portal");
        assert_eq!(md.refs[0].loc.line, 5);
        assert!(
            matches!(&md.refs[1].kind, BodyRefKind::Entry { heading: Some(h) } if h == "what-it-is")
        );
        let msgs: Vec<String> = diags
            .iter()
            .map(|d| format!("{}:{} {}", d.line, d.code, d.message))
            .collect();
        assert_eq!(diags.len(), 4, "{msgs:#?}"); // relative link, image, javascript:, data:
        assert!(msgs.iter().any(|m| m.starts_with("13:image")));
        assert!(msgs.iter().any(|m| m.contains("javascript:")));
        assert!(msgs.iter().any(|m| m.contains("data:text")));
        assert!(msgs.iter().any(|m| m.starts_with("9:link-scheme")));
    }
}
