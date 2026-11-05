//! Search (§9): the hit types every surface uses, and the space rules.
//!
//! M4 holds a simple in-memory word search ([`words`]) so the page, ⌘K and
//! the no-JS search page work; M5 adds the chunker, SQLite FTS5 ranking, the
//! embedders and fusion behind the same [`Hit`] type.

use firelight_format::catalog::{Catalog, PlacedKind};
use firelight_format::{EntryKind, Ref, SPACE_ALL};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Concept,
    Guide,
    Link,
    Repo,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Concept => "concept",
            Kind::Guide => "guide",
            Kind::Link => "link",
            Kind::Repo => "repo",
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "concept" => Some(Kind::Concept),
            "guide" => Some(Kind::Guide),
            "link" => Some(Kind::Link),
            "repo" => Some(Kind::Repo),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub kind: Kind,
    /// `repo/id`; for a repo hit, `repo/repo`.
    pub r: Ref,
    pub title: String,
    pub summary: String,
    /// The best-matching section's heading and its slug, for entries.
    pub heading: Option<(String, String)>,
    pub score: f64,
    /// Links: their URL.
    pub url: Option<String>,
}

/// Lowercased words of `s` (letters and digits, any language).
pub fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Is `r` visible in `space`? `all` sees every space but `firelight` (§5.7).
pub fn entry_in_space(cat: &Catalog, space: &str, r: &Ref) -> bool {
    if space == SPACE_ALL {
        return cat
            .spaces
            .iter()
            .filter(|s| s.id != firelight_format::SPACE_FIRELIGHT)
            .any(|s| s.entries.contains(r));
    }
    cat.space(space).is_some_and(|s| s.entries.contains(r))
}

/// Links placed in `space` (or in any space but `firelight`, for `all`).
pub fn links_in_space(cat: &Catalog, space: &str) -> Vec<Ref> {
    let mut out: Vec<Ref> = Vec::new();
    for s in cat.spaces.iter().filter(|s| {
        if space == SPACE_ALL {
            s.id != firelight_format::SPACE_FIRELIGHT
        } else {
            s.id == space
        }
    }) {
        out.extend(s.top.iter().chain(s.footer.iter()).map(|l| l.link.clone()));
        for i in &s.islands {
            match &i.kind {
                PlacedKind::Cards { cards } => {
                    for c in cards {
                        for sec in &c.sections {
                            out.extend(sec.links.iter().map(|l| l.link.clone()));
                        }
                    }
                }
                PlacedKind::Links { links } => out.extend(links.iter().map(|l| l.link.clone())),
                PlacedKind::Template(_) => {}
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The simple word search (M4): BM25-like weights by field (title 5,
/// aliases 4, summary 2, body 1), prefix match on the last word, and an exact
/// title or alias match first.
pub fn search(cat: &Catalog, space: &str, query: &str, limit: usize) -> Vec<Hit> {
    let q = words(query);
    if q.is_empty() {
        return Vec::new();
    }
    let last = q.len() - 1;
    let matches = |field: &[String], i: usize, w: &str| -> bool {
        field
            .iter()
            .any(|f| if i == last { f.starts_with(w) } else { f == w })
    };
    let score_fields = |fields: &[(&[String], f64)]| -> f64 {
        let mut s = 0.0;
        for (i, w) in q.iter().enumerate() {
            let mut best: f64 = 0.0;
            for (f, weight) in fields {
                if matches(f, i, w) {
                    best = best.max(*weight);
                }
            }
            if best == 0.0 {
                return 0.0; // every word must match somewhere
            }
            s += best;
        }
        s
    };
    let qnorm = q.join(" ");
    let mut hits = Vec::new();
    for repo in &cat.repos {
        for e in &repo.entries {
            let r = Ref {
                repo: repo.id.clone(),
                id: e.id.clone(),
            };
            if !entry_in_space(cat, space, &r) {
                continue;
            }
            let title = words(&e.title);
            let aliases: Vec<String> = e.aliases.iter().flat_map(|a| words(a)).collect();
            let summary = words(&e.summary);
            let body = words(&e.body.source);
            let mut score = score_fields(&[
                (&title, 5.0),
                (&aliases, 4.0),
                (&summary, 2.0),
                (&body, 1.0),
            ]);
            if score == 0.0 {
                continue;
            }
            if words(&e.title).join(" ") == qnorm
                || e.aliases.iter().any(|a| words(a).join(" ") == qnorm)
            {
                score += 1000.0;
            }
            let heading = best_heading(&e.body.source, &q, &e.body.headings);
            let kind = match e.kind {
                EntryKind::Concept => Kind::Concept,
                EntryKind::Guide => Kind::Guide,
            };
            hits.push(Hit {
                kind,
                r,
                title: e.title.clone(),
                summary: e.summary.clone(),
                heading,
                score,
                url: None,
            });
        }
    }
    let in_space = links_in_space(cat, space);
    for r in in_space {
        let Some(l) = cat.link(&r) else { continue };
        let label = words(&l.label);
        let note = words(l.note.as_deref().unwrap_or(""));
        let concept = l
            .entry
            .as_ref()
            .and_then(|(e, _)| cat.entry(e))
            .map(|e| words(&e.summary))
            .unwrap_or_default();
        let mut score = score_fields(&[(&label, 5.0), (&note, 2.0), (&concept, 1.0)]);
        if score == 0.0 {
            continue;
        }
        if label.join(" ") == qnorm {
            score += 1000.0;
        }
        hits.push(Hit {
            kind: Kind::Link,
            r: r.clone(),
            title: l.label.clone(),
            summary: l.note.clone().unwrap_or_default(),
            heading: None,
            score,
            url: Some(l.url.clone()),
        });
    }
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.title.cmp(&b.title))
    });
    hits.truncate(limit);
    hits
}

/// The heading of the section where the query's words appear most.
fn best_heading(
    md: &str,
    q: &[String],
    headings: &[firelight_format::Heading],
) -> Option<(String, String)> {
    let mut best: Option<(usize, usize)> = None; // (count, heading index)
    let mut idx: Option<usize> = None;
    let mut count = 0usize;
    let mut seen = 0usize;
    for line in md.lines().chain(std::iter::once("# end")) {
        if line.starts_with('#') {
            if let Some(i) = idx
                && count > 0
                && best.is_none_or(|(c, _)| count > c)
            {
                best = Some((count, i));
            }
            idx = (seen < headings.len()).then_some(seen);
            seen += 1;
            count = 0;
            continue;
        }
        let w = words(line);
        count += q.iter().filter(|x| w.contains(x)).count();
    }
    best.and_then(|(_, i)| headings.get(i))
        .map(|h| (h.text.clone(), h.slug.clone()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn word_splitting() {
        assert_eq!(
            super::words("Wie bekomme ich Admin-Rechte?"),
            ["wie", "bekomme", "ich", "admin", "rechte"]
        );
    }
}
