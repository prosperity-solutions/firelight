//! Search (§9): chunks ([`chunk`]), the index and its ranking ([`index`]),
//! golden questions ([`golden`]), the hit type every surface uses, and the
//! space rules. Embedding happens outside: this crate only sees vectors.

pub mod chunk;
pub mod golden;
pub mod index;

pub use index::{Filter, Index};

use firelight_format::catalog::{Catalog, PlacedKind};
use firelight_format::{Ref, SPACE_ALL};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// Is the repo visible in `space`?
pub fn repo_in_space(cat: &Catalog, space: &str, repo: &str) -> bool {
    cat.spaces
        .iter()
        .filter(|s| {
            if space == SPACE_ALL {
                s.id != firelight_format::SPACE_FIRELIGHT
            } else {
                s.id == space
            }
        })
        .any(|s| s.repos.contains(repo))
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
