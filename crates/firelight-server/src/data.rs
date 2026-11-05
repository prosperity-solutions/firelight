//! The answers behind `/api/v1` and the MCP tools (§16): one function per
//! question, returning JSON. Every item names the repo and commit it came
//! from, so the MCP provenance wrapper can say where each piece is from.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use firelight_format::catalog::{Catalog, PlacedKind};
use firelight_format::{EntryKind, Ref, SPACE_ALL};
use firelight_search::{Filter, Kind};

use crate::content::Content;

/// The most results any search returns.
pub const MAX_LIMIT: usize = 25;

/// A bad question: becomes 400 / an MCP invalid-params error.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    BadInput(String),
    NotFound(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadInput(s) | Error::NotFound(s) => f.write_str(s),
        }
    }
}

pub type Result = std::result::Result<Value, Error>;

/// Where answers point: `https://firelight.acme.example`, no trailing slash.
pub struct Ctx<'a> {
    pub content: &'a dyn Content,
    pub base: &'a str,
    /// Firelight's version: the "commit" of the built-in self-docs.
    pub version: &'a str,
}

/// The most items a listing returns.
pub const MAX_ITEMS: usize = 1000;

impl Ctx<'_> {
    fn permalink(&self, r: &Ref) -> String {
        format!("{}/e/{}/{}", self.base, r.repo, r.id)
    }

    fn commit(&self, repo: &str) -> Value {
        match self.content.source(repo) {
            Some(s) => json!(s.commit),
            None if repo == firelight_format::SELF_REPO => {
                json!(format!("built into Firelight v{}", self.version))
            }
            None => Value::Null,
        }
    }

    fn stale(&self, repo: &str, path: &str) -> Value {
        match self.content.staleness(repo, path) {
            Some(s) => {
                json!({ "stale": true, "entry_changed": s.entry_changed, "covers_changed": s.covers_changed })
            }
            None => json!({ "stale": false }),
        }
    }
}

fn kind_of(k: EntryKind) -> &'static str {
    match k {
        EntryKind::Concept => "concept",
        EntryKind::Guide => "guide",
    }
}

fn check_space(cat: &Catalog, space: Option<&str>) -> std::result::Result<String, Error> {
    let s = space.unwrap_or(SPACE_ALL);
    if s == SPACE_ALL || cat.space(s).is_some() {
        Ok(s.to_string())
    } else {
        Err(Error::BadInput(format!(
            "there is no space {s:?}; list_spaces names them"
        )))
    }
}

fn parse_kind(k: Option<&str>) -> std::result::Result<Option<Kind>, Error> {
    match k {
        None | Some("") => Ok(None),
        Some(k) => Kind::parse(k).map(Some).ok_or_else(|| {
            Error::BadInput(format!("kind {k:?}: use concept, guide, link or repo"))
        }),
    }
}

/// `search`: ranked hits (§9).
pub fn search(
    c: &Ctx<'_>,
    query: &str,
    space: Option<&str>,
    kind: Option<&str>,
    repo: Option<&str>,
    limit: Option<usize>,
) -> Result {
    let cat = c.content.catalog();
    let space = check_space(&cat, space)?;
    let kind = parse_kind(kind)?;
    let query: String = query.trim().chars().take(200).collect();
    if query.is_empty() {
        return Err(Error::BadInput("query is empty".into()));
    }
    let limit = limit.unwrap_or(10).clamp(1, MAX_LIMIT);
    let filter = Filter {
        space: Some(space.clone()),
        repos: repo.map(|r| BTreeSet::from([r.to_string()])),
        kinds: kind.map(|k| BTreeSet::from([k])),
    };
    let hits = c.content.search(&query, &filter, limit);
    let rows: Vec<Value> = hits
        .iter()
        .map(|h| {
            let mut v = json!({
                "ref": h.r.to_string(),
                "kind": h.kind.as_str(),
                "title": h.title,
                "summary": h.summary,
                "score": h.score,
                "repo": h.r.repo,
                "commit": c.commit(&h.r.repo),
            });
            match h.kind {
                Kind::Concept | Kind::Guide => {
                    let anchor = h
                        .heading
                        .as_ref()
                        .map(|(_, s)| format!("#{s}"))
                        .unwrap_or_default();
                    v["permalink"] = json!(format!("{}{anchor}", c.permalink(&h.r)));
                    if let Some((t, _)) = &h.heading {
                        v["heading"] = json!(t);
                    }
                    let path = cat.entry(&h.r).map(|e| e.path.clone()).unwrap_or_default();
                    v["stale"] = c.stale(&h.r.repo, &path)["stale"].clone();
                }
                Kind::Link => {
                    v["url"] = json!(h.url);
                    v["permalink"] = json!(
                        crate::pages::show_url(&cat, &h.r, Some(&space))
                            .map(|u| format!("{}{u}", c.base))
                    );
                }
                Kind::Repo => v["permalink"] = json!(format!("{}/r/{}", c.base, h.r.repo)),
            }
            v
        })
        .collect();
    Ok(json!({ "query": query, "space": space, "hits": rows }))
}

/// `list_entries`: every entry, for browsing instead of trusting search.
pub fn list_entries(
    c: &Ctx<'_>,
    space: Option<&str>,
    kind: Option<&str>,
    repo: Option<&str>,
    tag: Option<&str>,
) -> Result {
    let cat = c.content.catalog();
    let space = check_space(&cat, space)?;
    let kind = parse_kind(kind)?;
    if kind == Some(Kind::Link) || kind == Some(Kind::Repo) {
        return Err(Error::BadInput("entries are concepts or guides".into()));
    }
    let mut out = Vec::new();
    for r in cat.repos.iter().filter(|r| repo.is_none_or(|x| x == r.id)) {
        for e in &r.entries {
            let rf = Ref {
                repo: r.id.clone(),
                id: e.id.clone(),
            };
            if !firelight_search::entry_in_space(&cat, &space, &rf) {
                continue;
            }
            if kind.is_some_and(|k| k.as_str() != kind_of(e.kind)) {
                continue;
            }
            if tag.is_some_and(|t| !e.tags.iter().any(|x| x == t)) {
                continue;
            }
            if out.len() >= MAX_ITEMS {
                break;
            }
            out.push(json!({
                "ref": rf.to_string(), "kind": kind_of(e.kind), "title": e.title,
                "summary": e.summary, "tags": e.tags, "repo": r.id, "commit": c.commit(&r.id),
                "permalink": c.permalink(&rf),
            }));
        }
    }
    Ok(json!({ "space": space, "entries": out }))
}

/// `get_entry`: one entry in full.
pub fn get_entry(c: &Ctx<'_>, r: &str) -> Result {
    let cat = c.content.catalog();
    let rf = parse_ref(r)?;
    let e = cat
        .entry(&rf)
        .ok_or_else(|| Error::NotFound(format!("there is no entry {rf}")))?;
    let src = c.content.source(&rf.repo);
    let links: Vec<Value> = e
        .links
        .iter()
        .filter_map(|(l, _)| {
            cat.link(l)
                .map(|x| json!({ "ref": l.to_string(), "label": x.label, "url": x.url }))
        })
        .collect();
    let related: Vec<String> = e.related.iter().map(|(x, _)| x.to_string()).collect();
    let mut v = json!({
        "ref": rf.to_string(), "kind": kind_of(e.kind), "title": e.title, "summary": e.summary,
        "aliases": e.aliases, "tags": e.tags, "markdown": e.body.source, "links": links,
        "related": related, "covers": e.covers, "repo": rf.repo, "path": e.path,
        "commit": c.commit(&rf.repo),
        "commit_date": src.and_then(|s| s.committed_at),
        "permalink": c.permalink(&rf),
    });
    let st = c.stale(&rf.repo, &e.path);
    v["stale"] = st["stale"].clone();
    if st["stale"] == json!(true) {
        v["entry_changed"] = st["entry_changed"].clone();
        v["covers_changed"] = st["covers_changed"].clone();
    }
    Ok(v)
}

fn parse_ref(r: &str) -> std::result::Result<Ref, Error> {
    let (repo, id) = r
        .split_once('/')
        .ok_or_else(|| Error::BadInput(format!("{r:?} is not a ref: use repo/id")))?;
    if !firelight_format::ids::valid_id(repo) || !firelight_format::ids::valid_id(id) {
        return Err(Error::BadInput(format!("{r:?} is not a ref: use repo/id")));
    }
    Ok(Ref {
        repo: repo.into(),
        id: id.into(),
    })
}

/// A link where it sits: (link, space, island, card, section).
type Placement = (Ref, String, String, Option<String>, Option<String>);

/// `find_link`: links by words and by where they sit.
pub fn find_link(
    c: &Ctx<'_>,
    query: Option<&str>,
    space: Option<&str>,
    island: Option<&str>,
    card: Option<&str>,
) -> Result {
    let cat = c.content.catalog();
    let space = check_space(&cat, space)?;
    let lower = |s: &str| s.to_lowercase();
    // (link, place) for every placement in the space(s).
    let mut placed: Vec<Placement> = Vec::new();
    for s in cat.spaces.iter().filter(|s| {
        if space == SPACE_ALL {
            s.id != firelight_format::SPACE_FIRELIGHT
        } else {
            s.id == space
        }
    }) {
        for l in &s.top {
            placed.push((l.link.clone(), s.id.clone(), "top row".into(), None, None));
        }
        for l in &s.footer {
            placed.push((l.link.clone(), s.id.clone(), "footer".into(), None, None));
        }
        for i in &s.islands {
            match &i.kind {
                PlacedKind::Cards { cards } => {
                    for cd in cards {
                        for sec in &cd.sections {
                            for l in &sec.links {
                                placed.push((
                                    l.link.clone(),
                                    s.id.clone(),
                                    i.title.clone(),
                                    Some(cd.title.clone()),
                                    sec.name.clone(),
                                ));
                            }
                        }
                    }
                }
                PlacedKind::Links { links } => {
                    for l in links {
                        placed.push((l.link.clone(), s.id.clone(), i.title.clone(), None, None));
                    }
                }
                PlacedKind::Template(_) => {}
            }
        }
    }
    let words = query
        .map(|q| firelight_search::words(&q.chars().take(200).collect::<String>()))
        .unwrap_or_default();
    let mut out = Vec::new();
    for (r, sp, isl, cd, sec) in placed {
        if island.is_some_and(|x| !lower(&isl).contains(&lower(x))) {
            continue;
        }
        if card.is_some_and(|x| !cd.as_deref().is_some_and(|c| lower(c).contains(&lower(x)))) {
            continue;
        }
        let Some(l) = cat.link(&r) else { continue };
        if !words.is_empty() {
            let hay = firelight_search::words(&format!(
                "{} {} {isl} {} {}",
                l.label,
                l.note.as_deref().unwrap_or(""),
                cd.as_deref().unwrap_or(""),
                sec.as_deref().unwrap_or("")
            ));
            if !words
                .iter()
                .all(|w| hay.iter().any(|h| h.starts_with(w.as_str())))
            {
                continue;
            }
        }
        if out.len() >= MAX_ITEMS {
            break;
        }
        let place: Vec<String> = [Some(isl), cd, sec].into_iter().flatten().collect();
        out.push(json!({
            "ref": r.to_string(), "label": l.label, "url": l.url, "note": l.note,
            "space": sp, "where": place.join(" › "),
            "concept": l.entry.as_ref().map(|(e, _)| e.to_string()),
            "repo": r.repo, "commit": c.commit(&r.repo),
        }));
    }
    Ok(json!({ "space": space, "links": out }))
}

/// `list_spaces`.
pub fn list_spaces(c: &Ctx<'_>) -> Result {
    let cat = c.content.catalog();
    let conflicts: Vec<Value> = cat
        .diagnostics
        .iter()
        .filter(|(_, d)| d.code == "space-conflict" || d.code == "space-undeclared")
        .map(|(repo, d)| json!({ "repo": repo, "message": d.message }))
        .collect();
    let spaces: Vec<Value> = cat
        .spaces
        .iter()
        .map(|s| {
            let links = firelight_search::links_in_space(&cat, &s.id).len();
            json!({
                "id": s.id, "title": s.title, "summary": s.summary, "repos": s.repos,
                "declared_by": s.declared_by,
                "sources": s.repos.iter().map(|r| json!({ "repo": r, "commit": c.commit(r) })).collect::<Vec<_>>(),
                "counts": { "entries": s.entries.len(), "islands": s.islands.len(), "links": links },
                "permalink": format!("{}/s/{}", c.base, s.id),
            })
        })
        .collect();
    Ok(json!({ "spaces": spaces, "conflicts": conflicts }))
}

/// `list_repos`.
pub fn list_repos(c: &Ctx<'_>) -> Result {
    let cat = c.content.catalog();
    let status = c.content.status();
    let health = |id: &str| -> Value {
        status["sources"]
            .as_array()
            .and_then(|a| a.iter().find(|s| s["id"] == json!(id)))
            .map(|s| json!({ "last_pull": s["last_pull_at"], "health": s["health"] }))
            .unwrap_or_else(|| json!({ "last_pull": null, "health": "built in" }))
    };
    let repos: Vec<Value> = cat
        .repos
        .iter()
        .map(|r| {
            let h = health(&r.id);
            json!({
                "id": r.id, "name": r.name, "summary": r.summary, "owner": r.owner,
                "commit": c.commit(&r.id), "last_pull": h["last_pull"], "health": h["health"],
                "permalink": format!("{}/r/{}", c.base, r.id),
            })
        })
        .collect();
    Ok(json!({ "repos": repos }))
}

/// `get_repo`.
pub fn get_repo(c: &Ctx<'_>, id: &str) -> Result {
    let cat = c.content.catalog();
    let r = cat
        .repo(id)
        .ok_or_else(|| Error::NotFound(format!("there is no repo {id:?}")))?;
    let concepts = r
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Concept)
        .count();
    let guides = r.entries.len() - concepts;
    let relations: Vec<Value> = r
        .relations
        .iter()
        .map(|x| json!({ "repo": x.repo, "when": x.when, "then": x.then }))
        .collect();
    Ok(json!({
        "id": r.id, "name": r.name, "summary": r.summary, "owner": r.owner, "spaces": r.spaces,
        "relations": relations, "about": r.about.as_ref().map(|a| a.source.clone()),
        "counts": { "concepts": concepts, "guides": guides, "links": r.links.len(), "islands": r.islands.len(), "golden": r.golden.len() },
        "repo": r.id, "commit": c.commit(&r.id),
        "permalink": format!("{}/r/{}", c.base, r.id),
    }))
}

/// `status` (§16.1).
pub fn status(c: &Ctx<'_>) -> Result {
    Ok(c.content.status())
}
