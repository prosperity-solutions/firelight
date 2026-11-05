//! The YAML input types, exactly as written in `.firelight/` (§5). Unknown keys
//! are errors (`deny_unknown_fields`). These types also generate the JSON
//! Schemas under `schema/v1/` (§5.6), so a field's doc comment is its schema
//! description.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::pos::P;

/// Just the version, read first, so an unknown version is reported as such
/// instead of as unknown keys.
#[derive(Deserialize)]
pub struct VersionOnly {
    pub format: Option<P<serde_json::Value>>,
    #[serde(rename = "type")]
    pub kind: Option<P<String>>,
}

/// `.firelight/repo.yaml`: who this repo is and how it relates to others.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Firelight repo.yaml (format 1)")]
pub struct RepoYaml {
    /// Always `1`.
    pub format: P<u32>,
    /// The repo id: lowercase letters, digits and dashes. Must equal the source id Firelight is configured with.
    pub id: P<String>,
    /// Display name.
    pub name: P<String>,
    /// One sentence: what this repo is.
    pub summary: P<String>,
    /// Who owns it, as free text.
    #[serde(default)]
    pub owner: Option<String>,
    /// Default space(s) for this repo's islands and entries.
    #[serde(default)]
    pub spaces: Vec<P<String>>,
    /// Cross-repo rules: "when you do X here, also do Y there".
    #[serde(default)]
    pub relations: Vec<RelationYaml>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RelationYaml {
    /// The other repo's id.
    pub repo: P<String>,
    /// The trigger, e.g. "A new app is added there".
    pub when: P<String>,
    /// What to do, e.g. "Add its Helm release in `releases/`".
    pub then: P<String>,
}

/// `.firelight/links.yaml`: links this repo owns.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Firelight links.yaml (format 1)")]
pub struct LinksYaml {
    /// Always `1`.
    pub format: P<u32>,
    #[serde(default)]
    pub links: Vec<LinkYaml>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkYaml {
    pub id: P<String>,
    pub label: P<String>,
    /// `https:`, `http:` or `mailto:`.
    pub url: P<String>,
    /// Small grey text after the label.
    #[serde(default)]
    pub note: Option<String>,
    /// A built-in icon name.
    #[serde(default)]
    pub icon: Option<P<String>>,
    /// Where the link sits: one target or a list of them.
    pub at: P<OneOrMany<AtYaml>>,
    /// Lower first; ties sort by label.
    #[serde(default)]
    pub order: Option<i64>,
    /// "What is this?": a ref to a concept.
    #[serde(default)]
    pub entry: Option<P<String>>,
}

/// A link's place: `{place: top|footer, space?}`, `{island}` (a links island)
/// or `{island, card, section?}` (a cards island).
// Its fields carry no positions of their own: one `at` may be a list, and
// diagnostics point at the `at:` key.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AtYaml {
    /// `top` or `footer`.
    #[serde(default)]
    pub place: Option<String>,
    /// With `place`: only this space (default: every space of the link's repo).
    #[serde(default)]
    pub space: Option<String>,
    /// A ref to a links or cards island.
    #[serde(default)]
    pub island: Option<String>,
    /// A card id of that cards island.
    #[serde(default)]
    pub card: Option<String>,
    /// One of the cards island's sections.
    #[serde(default)]
    pub section: Option<String>,
}

/// Fields every island has.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Firelight island, type: cards (format 1)")]
pub struct CardsIslandYaml {
    pub format: P<u32>,
    #[serde(rename = "type")]
    pub kind: P<String>,
    /// Defaults to the file name.
    #[serde(default)]
    pub id: Option<P<String>>,
    pub title: P<String>,
    /// Defaults to the first of repo.yaml's spaces, else `default`.
    #[serde(default)]
    pub space: Option<P<String>>,
    /// `main` (default), `top` or `footer`.
    #[serde(default)]
    pub place: Option<P<String>>,
    #[serde(default)]
    pub order: Option<i64>,
    /// Optional sub-headings inside every card, in this order.
    #[serde(default)]
    pub sections: Vec<P<String>>,
    #[serde(default)]
    pub cards: Vec<CardYaml>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardYaml {
    pub id: P<String>,
    pub title: P<String>,
    #[serde(default)]
    pub order: Option<i64>,
    /// `danger`, `warn` or `none`.
    #[serde(default)]
    pub tone: Option<P<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Firelight island, type: links (format 1)")]
pub struct LinksIslandYaml {
    pub format: P<u32>,
    #[serde(rename = "type")]
    pub kind: P<String>,
    #[serde(default)]
    pub id: Option<P<String>>,
    pub title: P<String>,
    #[serde(default)]
    pub space: Option<P<String>>,
    #[serde(default)]
    pub place: Option<P<String>>,
    #[serde(default)]
    pub order: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Firelight island, type: template (format 1)")]
pub struct TemplateIslandYaml {
    pub format: P<u32>,
    #[serde(rename = "type")]
    pub kind: P<String>,
    #[serde(default)]
    pub id: Option<P<String>>,
    pub title: P<String>,
    #[serde(default)]
    pub space: Option<P<String>>,
    #[serde(default)]
    pub place: Option<P<String>>,
    #[serde(default)]
    pub order: Option<i64>,
    pub inputs: Vec<InputYaml>,
    pub links: Vec<TemplateLinkYaml>,
    #[serde(default)]
    pub help: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputYaml {
    /// The placeholder name used as `{name}` in URLs.
    pub name: P<String>,
    pub label: P<String>,
    /// An anchored regular expression (`^…$`) every value must match.
    pub pattern: P<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateLinkYaml {
    pub label: P<String>,
    /// May use `{name}` only as the host's first label, one whole path segment, or one query value.
    pub url: P<String>,
    #[serde(default)]
    pub icon: Option<P<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Firelight island, type: cards-contribution (format 1)")]
pub struct CardsContributionYaml {
    pub format: P<u32>,
    #[serde(rename = "type")]
    pub kind: P<String>,
    /// The cards island to add to, e.g. `infra-repo/environments`.
    pub island: P<String>,
    pub cards: Vec<CardYaml>,
}

/// An entry's front matter (`concepts/*.md`, `guides/*.md`).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Firelight entry front matter (format 1)")]
pub struct EntryYaml {
    pub format: P<u32>,
    /// Defaults to the file name without `.md`.
    #[serde(default)]
    pub id: Option<P<String>>,
    pub title: P<String>,
    /// At most 200 characters; search results show it.
    pub summary: P<String>,
    #[serde(default)]
    pub aliases: Vec<P<String>>,
    #[serde(default)]
    pub tags: Vec<P<String>>,
    /// Links this entry is about (the "Show me" targets).
    #[serde(default)]
    pub links: Vec<P<String>>,
    /// Related entries.
    #[serde(default)]
    pub related: Vec<P<String>>,
    /// Paths (globs) this entry describes; changes to them without the entry make it "maybe stale".
    #[serde(default)]
    pub covers: Vec<P<String>>,
    /// Overrides repo.yaml's default spaces.
    #[serde(default)]
    pub spaces: Option<Vec<P<String>>>,
}

/// `.firelight/golden.yaml`: questions that must find their entries.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Firelight golden.yaml (format 1)")]
pub struct GoldenYaml {
    pub format: P<u32>,
    pub questions: Vec<QuestionYaml>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestionYaml {
    pub query: P<String>,
    /// Refs that must each be in the top `within` results.
    pub expect: Vec<P<String>>,
    /// Default 5.
    #[serde(default)]
    pub within: Option<P<u32>>,
    /// `meaning`: needs meaning search; skipped when the embedder is off.
    #[serde(default)]
    pub needs: Option<P<String>>,
}

/// `.firelight/spaces.yaml`: spaces this repo declares.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Firelight spaces.yaml (format 1)")]
pub struct SpacesYaml {
    pub format: P<u32>,
    pub spaces: Vec<SpaceYaml>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpaceYaml {
    pub id: P<String>,
    pub title: P<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub icon: Option<P<String>>,
    #[serde(default)]
    pub order: Option<i64>,
}

/// One value or a list of them.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T: Clone> OneOrMany<T> {
    pub fn to_vec(&self) -> Vec<T> {
        match self {
            OneOrMany::One(t) => vec![t.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}
