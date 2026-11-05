//! Markdown to HTML for entries and `about.md` (§5.2, §17.1).
//!
//! - Raw HTML (blocks and inline) is **escaped and shown as text**, never
//!   rendered.
//! - `link:<ref>` becomes the link's URL with a "Show me" target;
//!   `entry:<ref>[#h]` a permalink; a ref that doesn't resolve renders as plain
//!   text, never a dead link.
//! - Other URLs pass [`crate::html::safe_url`] or the link becomes plain text.
//! - Images are not rendered (the checker refuses them; this is the second
//!   line): their alt text shows instead.
//! - Headings get stable ids (the same slugs as the format crate) and a
//!   copy-link anchor.

use pulldown_cmark::{CowStr, Event, HeadingLevel, LinkType, Parser, Tag, TagEnd, html};

use firelight_format::ids::{Ref, Slugger};
use firelight_format::markdown::{Dest, classify, options};

use crate::html::{attr, safe_url};

/// How refs in markdown resolve on this page.
pub trait Resolve {
    /// The URL of link `r`, and where "Show me" goes (`/s/<space>#<ref>`).
    fn link(&self, r: &Ref) -> Option<(String, Option<String>)>;
    /// The permalink of entry `r`, if it exists.
    fn entry(&self, r: &Ref) -> Option<String>;
}

/// Render `md` from repo `home` to HTML.
pub fn render(md: &str, home: &str, resolve: &dyn Resolve) -> String {
    let mut st = State {
        home,
        resolve,
        links: Vec::new(),
        in_image: 0,
    };
    let mut events: Vec<Event<'_>> = Vec::new();
    let mut slugger = Slugger::default();
    // While inside a heading: its level, its events (already processed by the
    // same rules), and its text for the slug.
    let mut heading: Option<(HeadingLevel, Vec<Event<'_>>, String)> = None;
    for ev in Parser::new_ext(md, options()) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) if heading.is_none() => {
                heading = Some((level, Vec::new(), String::new()))
            }
            Event::End(TagEnd::Heading(_)) if heading.is_some() => {
                if let Some((level, inner, text)) = heading.take() {
                    let slug = slugger.next(&text);
                    let n = level as u8;
                    events.push(Event::Html(CowStr::from(format!(
                        "<h{n} id=\"{}\">",
                        attr(&slug)
                    ))));
                    events.extend(inner);
                    events.push(Event::Html(CowStr::from(format!(
                        " <a class=\"anchor\" href=\"#{}\" aria-label=\"Link to this section\">🔗</a></h{n}>",
                        attr(&slug)
                    ))));
                }
            }
            ev => match heading.as_mut() {
                Some((_, inner, text)) => {
                    if let Event::Text(t) | Event::Code(t) = &ev {
                        text.push_str(t);
                    }
                    st.push(ev, inner);
                }
                None => st.push(ev, &mut events),
            },
        }
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}

struct State<'r> {
    home: &'r str,
    resolve: &'r dyn Resolve,
    /// For each open link: did we render it (true) or turn it into text?
    links: Vec<bool>,
    in_image: usize,
}

impl State<'_> {
    /// Apply every rule to one event and push the result.
    fn push<'a>(&mut self, ev: Event<'a>, out: &mut Vec<Event<'a>>) {
        match ev {
            Event::Html(h) | Event::InlineHtml(h) => out.push(Event::Text(h)),
            Event::Start(Tag::Image { .. }) => self.in_image += 1,
            Event::End(TagEnd::Image) => self.in_image = self.in_image.saturating_sub(1),
            // Inside an image only its alt text survives, as text: no links either.
            Event::Text(t) | Event::Code(t) if self.in_image > 0 => out.push(Event::Text(t)),
            _ if self.in_image > 0 => {}
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => match link_start(link_type, &dest_url, &title, self.home, self.resolve) {
                Some(html) => {
                    self.links.push(true);
                    out.push(Event::Html(CowStr::from(html)));
                }
                None => self.links.push(false),
            },
            Event::End(TagEnd::Link) => {
                if self.links.pop() == Some(true) {
                    out.push(Event::Html(CowStr::from("</a>")));
                }
            }
            other => out.push(other),
        }
    }
}

fn link_start(
    link_type: LinkType,
    dest: &str,
    title: &str,
    home: &str,
    resolve: &dyn Resolve,
) -> Option<String> {
    let title_attr = if title.is_empty() {
        String::new()
    } else {
        format!(" title=\"{}\"", attr(title))
    };
    if link_type == LinkType::Email {
        return Some(format!("<a href=\"mailto:{}\">", attr(dest)));
    }
    match classify(dest, home).ok()? {
        Dest::Link(r) => {
            let (url, show) = resolve.link(&r)?;
            let url = safe_url(&url)?;
            let show = show
                .map(|s| format!(" data-show=\"{}\"", attr(&s)))
                .unwrap_or_default();
            Some(format!(
                "<a class=\"link-ref\" href=\"{}\"{show}{title_attr}>",
                attr(url)
            ))
        }
        Dest::Entry(r, h) => {
            let mut url = resolve.entry(&r)?;
            if let Some(h) = h {
                url.push('#');
                url.push_str(&h);
            }
            Some(format!("<a href=\"{}\"{title_attr}>", attr(&url)))
        }
        Dest::External => {
            let url = safe_url(dest)?;
            Some(format!("<a href=\"{}\"{title_attr}>", attr(url)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct R;
    impl Resolve for R {
        fn link(&self, r: &Ref) -> Option<(String, Option<String>)> {
            (r.id == "portal").then(|| {
                (
                    "https://access.acme.example".into(),
                    Some(format!("/s/platform#{r}")),
                )
            })
        }
        fn entry(&self, r: &Ref) -> Option<String> {
            (r.id == "model").then(|| format!("/e/{}/{}", r.repo, r.id))
        }
    }

    fn r(md: &str) -> String {
        render(md, "infra", &R)
    }

    /// A test per vector (§13.1, "Rendering").
    #[test]
    fn raw_html_never_renders() {
        let out = r(
            "<script>alert(1)</script>\n\nText <img src=x onerror=alert(1)> and <b onclick=\"x()\">bold</b>.\n",
        );
        assert!(!out.contains("<script"), "{out}");
        assert!(!out.contains("<img"), "{out}");
        assert!(!out.contains("<b "), "{out}");
        assert!(out.contains("&lt;script&gt;"), "{out}");
        assert!(out.contains("&lt;img src=x onerror=alert(1)&gt;"), "{out}");
    }

    #[test]
    fn dangerous_urls_become_text() {
        for (md, bad) in [
            ("[x](javascript:alert(1))", "javascript:"),
            ("[x](JAVASCRIPT:alert(1))", "JAVASCRIPT:"),
            ("[x](data:text/html,<script>)", "data:"),
            ("[x](vbscript:msgbox)", "vbscript:"),
            ("[x](//evil.example/)", "//evil"),
            ("<javascript:alert(1)>", "javascript:alert"),
            ("[x](&#106;avascript:alert(1))", "avascript:"),
            ("[x]: javascript:alert(1)\n\n[y][x]", "javascript:"),
        ] {
            let out = r(md);
            assert!(
                !out.contains("href=\"j")
                    && !out.contains(&format!("href=\"{bad}"))
                    && !out.contains("href=\"data")
                    && !out.contains("href=\"//"),
                "{md} → {out}"
            );
        }
        assert!(r("[x](https://ok.acme.example)").contains("href=\"https://ok.acme.example\""));
        // The same rules inside headings and other containers.
        for md in [
            "## [x](javascript:alert(1))",
            "> [x](javascript:alert(1))",
            "- [x](data:text/html,y)",
            "| [x](javascript:y) |\n|---|\n",
        ] {
            let out = r(md);
            assert!(
                !out.contains("javascript:alert") || !out.contains("href=\"javascript"),
                "{md} → {out}"
            );
            assert!(
                !out.contains("href=\"javascript") && !out.contains("href=\"data"),
                "{md} → {out}"
            );
        }
        assert!(!r("## <script>x</script>").contains("<script"));
    }

    #[test]
    fn images_show_only_their_alt_text() {
        let out = r("![chart <b>](https://img.acme.example/x.png)");
        assert!(!out.contains("<img"), "{out}");
        assert!(out.contains("chart"), "{out}");
        let out = r("![a `b` c](https://img.acme.example/x.png)");
        assert!(out.contains('b'), "code in alt text stays as text: {out}");
        for md in [
            "![[x](https://a.acme.example)](https://img.acme.example/y.png)",
            "![[x](javascript:alert(1))](https://img.acme.example/y.png)",
        ] {
            let out = r(md);
            assert!(!out.contains("<a"), "no link inside an image: {out}");
            assert!(out.contains('x'), "{out}");
        }
    }

    #[test]
    fn refs_and_headings() {
        let out = r(
            "Open [the portal](link:portal), see [the model](entry:model#what-it-is) and [gone](entry:gone).\n\n## What it does not do\n\n## What it does not do\n",
        );
        assert!(out.contains("<a class=\"link-ref\" href=\"https://access.acme.example\" data-show=\"/s/platform#infra/portal\">the portal</a>"), "{out}");
        assert!(
            out.contains("<a href=\"/e/infra/model#what-it-is\">the model</a>"),
            "{out}"
        );
        assert!(
            out.contains(" and gone."),
            "an unresolved ref is plain text: {out}"
        );
        assert!(out.contains("<h2 id=\"what-it-does-not-do\">What it does not do <a class=\"anchor\" href=\"#what-it-does-not-do\""), "{out}");
        assert!(out.contains("id=\"what-it-does-not-do-2\""), "{out}");
    }

    #[test]
    fn attribute_injection_through_titles_and_urls() {
        let out = r("[x](https://a.acme.example/\"onmouseover=\"alert(1) \"t\\\"onclick=x\")");
        assert!(!out.contains("\"onmouseover"), "{out}");
        assert!(!out.contains("\" onclick"), "{out}");
    }

    #[test]
    fn tables_and_task_lists() {
        let out = r("| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n- [ ] not\n");
        assert!(
            out.contains("<table>") && out.contains("type=\"checkbox\""),
            "{out}"
        );
    }
}
