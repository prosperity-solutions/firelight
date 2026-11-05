//! One repo's `.firelight/`, parsed and checked (§5). Refs are resolved to
//! [`Ref`]s (repo + id); cross-repo targets are checked later by the catalog.

use crate::ids::Ref;

/// Where something came from, for diagnostics found after parsing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct Loc {
    /// Relative to the repo root, e.g. `.firelight/links.yaml`.
    pub path: String,
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Repo {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub owner: Option<String>,
    /// Default spaces for this repo's islands and entries; empty = `default`.
    pub spaces: Vec<String>,
    pub relations: Vec<Relation>,
    /// `about.md`, raw markdown.
    pub about: Option<Markdown>,
    pub links: Vec<Link>,
    pub islands: Vec<Island>,
    pub contributions: Vec<CardsContribution>,
    pub entries: Vec<Entry>,
    pub golden: Vec<GoldenQuestion>,
    pub space_decls: Vec<SpaceDecl>,
}

impl Repo {
    /// This repo's default spaces, or `[default]`.
    pub fn default_spaces(&self) -> Vec<String> {
        if self.spaces.is_empty() {
            vec![crate::SPACE_DEFAULT.to_string()]
        } else {
            self.spaces.clone()
        }
    }

    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn link(&self, id: &str) -> Option<&Link> {
        self.links.iter().find(|l| l.id == id)
    }

    pub fn island(&self, id: &str) -> Option<&Island> {
        self.islands.iter().find(|i| i.id == id)
    }
}

#[derive(Debug, Clone)]
pub struct Relation {
    pub repo: String,
    pub when: String,
    pub then: String,
    pub loc: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum At {
    /// The built-in top or footer row; `space: None` = every space of the link's repo.
    Place { place: Place, space: Option<String> },
    /// A `links` island.
    Island { island: Ref },
    /// A card of a `cards` island, optionally under one of its sections.
    Card {
        island: Ref,
        card: String,
        section: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    Top,
    Main,
    Footer,
}

impl Place {
    pub fn parse(s: &str) -> Option<Place> {
        match s {
            "top" => Some(Place::Top),
            "main" => Some(Place::Main),
            "footer" => Some(Place::Footer),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Place::Top => "top",
            Place::Main => "main",
            Place::Footer => "footer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Link {
    pub id: String,
    pub label: String,
    pub url: String,
    pub note: Option<String>,
    pub icon: Option<String>,
    pub at: Vec<(At, Loc)>,
    pub order: i64,
    pub entry: Option<(Ref, Loc)>,
    pub loc: Loc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Danger,
    Warn,
    None,
}

#[derive(Debug, Clone)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub order: i64,
    pub tone: Tone,
    pub loc: Loc,
}

#[derive(Debug, Clone)]
pub struct Island {
    pub id: String,
    pub title: String,
    pub space: String,
    pub place: Place,
    pub order: i64,
    pub kind: IslandKind,
    pub loc: Loc,
}

#[derive(Debug, Clone)]
pub enum IslandKind {
    Cards {
        sections: Vec<String>,
        cards: Vec<Card>,
    },
    Links,
    Template(Template),
}

impl IslandKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            IslandKind::Cards { .. } => "cards",
            IslandKind::Links => "links",
            IslandKind::Template(_) => "template",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub inputs: Vec<TemplateInput>,
    pub links: Vec<TemplateLink>,
    pub help: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateInput {
    pub name: String,
    pub label: String,
    /// Anchored (`^…$`), checked to compile.
    pub pattern: String,
    pub placeholder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateLink {
    pub label: String,
    /// Checked by [`crate::template::check_url`]: placeholders only in safe positions.
    pub url: String,
    pub icon: Option<String>,
}

/// Cards another repo adds to a cards island.
#[derive(Debug, Clone)]
pub struct CardsContribution {
    pub island: Ref,
    pub cards: Vec<Card>,
    pub loc: Loc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKind {
    Concept,
    Guide,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EntryKind::Concept => "concept",
            EntryKind::Guide => "guide",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub kind: EntryKind,
    pub title: String,
    pub summary: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub links: Vec<(Ref, Loc)>,
    pub related: Vec<(Ref, Loc)>,
    pub covers: Vec<String>,
    /// Its own spaces, else the repo's (see [`Entry::spaces_in`]).
    pub spaces: Option<Vec<String>>,
    pub body: Markdown,
    /// Relative to the repo root.
    pub path: String,
    pub loc: Loc,
}

impl Entry {
    pub fn spaces_in(&self, repo: &Repo) -> Vec<String> {
        match &self.spaces {
            Some(s) if !s.is_empty() => s.clone(),
            _ => repo.default_spaces(),
        }
    }
}

/// Markdown as written, plus what the checker found in it.
#[derive(Debug, Clone, Default)]
pub struct Markdown {
    pub source: String,
    /// Line of `source`'s first line in its file (front matter shifts it).
    pub first_line: u32,
    pub headings: Vec<Heading>,
    /// `link:` and `entry:` targets, resolved.
    pub refs: Vec<BodyRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    /// Stable anchor; duplicates get `-2`, `-3`.
    pub slug: String,
    /// Byte range of the heading in [`Markdown::source`], as the parser saw
    /// it (setext and blockquote headings included).
    pub span: (usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyRefKind {
    Link,
    Entry { heading: Option<String> },
}

#[derive(Debug, Clone)]
pub struct BodyRef {
    pub kind: BodyRefKind,
    pub target: Ref,
    pub loc: Loc,
}

#[derive(Debug, Clone)]
pub struct GoldenQuestion {
    pub query: String,
    pub expect: Vec<(Ref, Loc)>,
    pub within: u32,
    pub needs_meaning: bool,
    pub loc: Loc,
}

#[derive(Debug, Clone)]
pub struct SpaceDecl {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub icon: Option<String>,
    pub order: i64,
    pub loc: Loc,
}
