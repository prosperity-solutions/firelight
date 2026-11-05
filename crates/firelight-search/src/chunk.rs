//! Chunking (§9.1): what gets indexed and embedded.
//!
//! - Entries: one chunk per `##`/`###` section, its text prefixed with the
//!   path `Title › Section` and the entry's aliases; the text before the first
//!   heading is its own chunk, together with the summary.
//! - Links: one small chunk: label, note, the cards and islands it sits in,
//!   and the linked concept's summary.
//! - Repos: one chunk: summary, relations and the first section of about.md.
//!
//! A chunk over the token cap is split on paragraphs, an oversized paragraph
//! on sentences, and a sentence that is still too long is cut.

use sha2::Digest as _;

use firelight_format::catalog::{Catalog, PlacedKind};
use firelight_format::{EntryKind, Ref};

use crate::Kind;

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub kind: Kind,
    /// The entry, link or repo (`repo/repo`) the chunk belongs to.
    pub owner: Ref,
    /// The section's heading and slug (entries).
    pub heading: Option<(String, String)>,
    /// Word-search fields (bm25 weights 5, 4, 2, 1).
    pub title: String,
    pub aliases: String,
    pub summary: String,
    pub body: String,
    /// What the embedder reads.
    pub text: String,
    /// sha256 of `text`: the embedding cache key.
    pub hash: [u8; 32],
}

fn hash(s: &str) -> [u8; 32] {
    sha2::Sha256::digest(s.as_bytes()).into()
}

/// Every chunk of the catalog, in a stable order.
pub fn chunks(cat: &Catalog, max_tokens: usize, count: &dyn Fn(&str) -> usize) -> Vec<Chunk> {
    let mut out = Vec::new();
    for repo in &cat.repos {
        for e in &repo.entries {
            entry_chunks(&repo.id, e, max_tokens, count, &mut out);
        }
    }
    link_chunks(cat, max_tokens, count, &mut out);
    for repo in &cat.repos {
        let mut body = String::new();
        for r in &repo.relations {
            body.push_str(&format!("{}: {} → {}\n", r.repo, r.when, r.then));
        }
        if let Some(a) = &repo.about {
            body.push_str(first_section(&a.source));
        }
        let text = format!("{}\n{}\n{}", repo.name, repo.summary, body);
        push_split(
            &mut out,
            Chunk {
                kind: Kind::Repo,
                owner: Ref {
                    repo: repo.id.clone(),
                    id: repo.id.clone(),
                },
                heading: None,
                title: repo.name.clone(),
                aliases: repo.id.clone(),
                summary: repo.summary.clone(),
                body,
                text,
                hash: [0; 32],
            },
            max_tokens,
            count,
        );
    }
    out
}

/// about.md up to its second heading (or all of it).
fn first_section(md: &str) -> &str {
    let mut seen = 0;
    let mut offset = 0;
    for line in md.split_inclusive('\n') {
        if line.starts_with('#') {
            seen += 1;
            if seen == 2 {
                return &md[..offset];
            }
        }
        offset += line.len();
    }
    md
}

/// ATX heading level of a line (`## x` → 2), outside code fences.
fn heading_level(line: &str) -> Option<usize> {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return None;
    }
    let n = t.chars().take_while(|c| *c == '#').count();
    ((1..=6).contains(&n) && t[n..].chars().next().is_none_or(|c| c == ' ' || c == '\t'))
        .then_some(n)
}

fn entry_chunks(
    repo: &str,
    e: &firelight_format::Entry,
    max_tokens: usize,
    count: &dyn Fn(&str) -> usize,
    out: &mut Vec<Chunk>,
) {
    let owner = Ref {
        repo: repo.to_string(),
        id: e.id.clone(),
    };
    let kind = match e.kind {
        EntryKind::Concept => Kind::Concept,
        EntryKind::Guide => Kind::Guide,
    };
    let aliases = e.aliases.join(", ");
    // Sections: (heading index into body.headings, text).
    let mut sections: Vec<(Option<usize>, String)> = vec![(None, String::new())];
    let mut seen = 0usize;
    let mut fence: Option<String> = None;
    for line in e.body.source.lines() {
        let t = line.trim_start();
        if let Some(f) = &fence {
            if t.starts_with(f.as_str()) {
                fence = None;
            }
        } else if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(t[..3].to_string());
        } else if let Some(level) = heading_level(line) {
            let idx = seen;
            seen += 1;
            if level == 2 || level == 3 {
                sections.push((Some(idx), String::new()));
                continue;
            }
        }
        if let Some(last) = sections.last_mut() {
            last.1.push_str(line);
            last.1.push('\n');
        }
    }
    for (i, (h, text)) in sections.into_iter().enumerate() {
        let heading = h
            .and_then(|i| e.body.headings.get(i))
            .map(|h| (h.text.clone(), h.slug.clone()));
        let intro = i == 0;
        if !intro && text.trim().is_empty() && heading.is_none() {
            continue;
        }
        let path = match &heading {
            Some((t, _)) => format!("{} › {t}", e.title),
            None => e.title.clone(),
        };
        let summary = if intro {
            e.summary.clone()
        } else {
            String::new()
        };
        let body = match &heading {
            Some((t, _)) => format!("{t}\n{text}"),
            None => text.clone(),
        };
        let mut emb = path;
        if !aliases.is_empty() {
            emb.push_str(&format!(" ({aliases})"));
        }
        emb.push('\n');
        if intro {
            emb.push_str(&e.summary);
            emb.push('\n');
        }
        emb.push_str(&text);
        push_split(
            out,
            Chunk {
                kind,
                owner: owner.clone(),
                heading,
                title: e.title.clone(),
                aliases: aliases.clone(),
                summary,
                body,
                text: emb,
                hash: [0; 32],
            },
            max_tokens,
            count,
        );
    }
}

fn link_chunks(
    cat: &Catalog,
    max_tokens: usize,
    count: &dyn Fn(&str) -> usize,
    out: &mut Vec<Chunk>,
) {
    // Where each link sits: card and island titles.
    let mut places: std::collections::BTreeMap<Ref, Vec<String>> = Default::default();
    for s in &cat.spaces {
        for i in &s.islands {
            match &i.kind {
                PlacedKind::Cards { cards } => {
                    for c in cards {
                        for sec in &c.sections {
                            for l in &sec.links {
                                let p = places.entry(l.link.clone()).or_default();
                                for t in [&i.title, &c.title] {
                                    if !p.contains(t) {
                                        p.push(t.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                PlacedKind::Links { links } => {
                    for l in links {
                        let p = places.entry(l.link.clone()).or_default();
                        if !p.contains(&i.title) {
                            p.push(i.title.clone());
                        }
                    }
                }
                PlacedKind::Template(_) => {}
            }
        }
    }
    for repo in &cat.repos {
        for l in &repo.links {
            let r = Ref {
                repo: repo.id.clone(),
                id: l.id.clone(),
            };
            let concept = l
                .entry
                .as_ref()
                .and_then(|(e, _)| cat.entry(e))
                .map(|e| e.summary.clone())
                .unwrap_or_default();
            let place = places.get(&r).map(|p| p.join(", ")).unwrap_or_default();
            let note = l.note.clone().unwrap_or_default();
            let body = format!("{place}\n{concept}");
            let text = format!("{}\n{note}\n{place}\n{concept}", l.label);
            push_split(
                out,
                Chunk {
                    kind: Kind::Link,
                    owner: r,
                    heading: None,
                    title: l.label.clone(),
                    aliases: String::new(),
                    summary: note,
                    body,
                    text,
                    hash: [0; 32],
                },
                max_tokens,
                count,
            );
        }
    }
}

/// Push `c`, split into pieces under `max` tokens when it is longer.
fn push_split(out: &mut Vec<Chunk>, c: Chunk, max: usize, count: &dyn Fn(&str) -> usize) {
    if count(&c.text) <= max {
        out.push(Chunk {
            hash: hash(&c.text),
            ..c
        });
        return;
    }
    // The head (path line, aliases, summary) rides on every piece.
    let (head, rest) = c.text.split_once('\n').unwrap_or((&c.text, ""));
    let budget = max.saturating_sub(count(head)).max(max / 2);
    for piece in pieces(rest, budget, count) {
        let text = format!("{head}\n{piece}");
        out.push(Chunk {
            body: piece.clone(),
            hash: hash(&text),
            text,
            ..c.clone()
        });
    }
}

/// Split on paragraphs, then sentences, then by length.
fn pieces(text: &str, max: usize, count: &dyn Fn(&str) -> usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.trim().is_empty() {
            out.push(std::mem::take(cur));
        }
        cur.clear();
    };
    for para in text.split("\n\n") {
        let units: Vec<String> = if count(para) <= max {
            vec![para.to_string()]
        } else {
            sentences(para)
                .into_iter()
                .flat_map(|s| cut(&s, max, count))
                .collect()
        };
        for u in units {
            let joined = if cur.is_empty() {
                u.clone()
            } else {
                format!("{cur}\n\n{u}")
            };
            if count(&joined) <= max {
                cur = joined;
            } else {
                flush(&mut cur, &mut out);
                cur = u;
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn sentences(p: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    let b = p.as_bytes();
    for (i, c) in p.char_indices() {
        if matches!(c, '.' | '!' | '?' | '。') {
            let end = i + c.len_utf8();
            if end >= p.len() || b[end].is_ascii_whitespace() {
                out.push(p[start..end].to_string());
                start = end;
            }
        }
    }
    if start < p.len() {
        out.push(p[start..].to_string());
    }
    out
}

/// Cut `s` into pieces under `max` tokens, at char boundaries.
fn cut(s: &str, max: usize, count: &dyn Fn(&str) -> usize) -> Vec<String> {
    if count(s) <= max {
        return vec![s.to_string()];
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        // Grow by halving: find the longest prefix under max.
        let (mut lo, mut hi) = (start + 1, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let piece: String = chars[start..mid].iter().collect();
            if count(&piece) <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        out.push(chars[start..lo].iter().collect());
        start = lo;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> usize {
        s.split_whitespace().count()
    }

    #[test]
    fn splitting_keeps_every_piece_under_the_cap() {
        let long = "word ".repeat(50) + "end. " + &"x".repeat(10);
        let text = format!("one two three\n\n{long}\n\nfour five");
        for p in pieces(&text, 8, &words) {
            assert!(words(&p) <= 8, "{p:?}");
        }
        let joined: String = pieces(&text, 8, &words).join(" ");
        assert_eq!(words(&joined), words(&text), "nothing lost");
    }

    #[test]
    fn headings_outside_fences_only() {
        assert_eq!(heading_level("## A"), Some(2));
        assert_eq!(heading_level("#tag"), None);
        assert_eq!(heading_level("    ## code"), None);
    }

    #[test]
    fn about_first_section() {
        assert_eq!(first_section("# T\nintro\n## More\nx"), "# T\nintro\n");
        assert_eq!(first_section("intro only"), "intro only");
    }
}
