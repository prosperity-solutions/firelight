//! The page model across repos (§5.3, §5.4, §5.7).
//!
//! [`build`] takes every parsed repo, in configured source order, and merges
//! them: spaces (first declarer wins, conflicts are warnings), islands with
//! their cards (owned and contributed) and links, the built-in top and footer
//! rows, an **Unsorted** card for every link whose island or card doesn't
//! exist (never dropped), and entry membership. Cross-repo refs that don't
//! resolve are warnings ([`Strictness::Strict`] makes them errors), because
//! only the hub sees every repo.

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::Diagnostic;
use crate::ids::Ref;
use crate::model::*;
use crate::{SELF_REPO, SPACE_DEFAULT, SPACE_FIRELIGHT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strictness {
    /// Unresolved cross-repo refs are warnings (the hub's view).
    Lenient,
    /// They are errors (`check --strict`).
    Strict,
}

/// A link where it is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedLink {
    pub link: Ref,
    pub label: String,
    pub url: String,
    pub note: Option<String>,
    pub icon: Option<String>,
    /// The concept behind "What is this?", only when it resolves.
    pub entry: Option<Ref>,
    pub order: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedSection {
    /// `None`: links without a section, shown first.
    pub name: Option<String>,
    pub links: Vec<PlacedLink>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedCard {
    pub id: String,
    pub title: String,
    pub tone: Tone,
    pub order: i64,
    /// The repo that owns the card (the island's, or a contributor's).
    pub repo: String,
    pub sections: Vec<PlacedSection>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlacedKind {
    Cards { cards: Vec<PlacedCard> },
    Links { links: Vec<PlacedLink> },
    Template(Template),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedIsland {
    /// `repo/id`; the Unsorted island is `<space>/unsorted` under repo `""`.
    pub r: Ref,
    pub title: String,
    pub place: Place,
    pub order: i64,
    pub kind: PlacedKind,
    pub unsorted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub icon: Option<String>,
    pub order: i64,
    /// Who declared it, in source order; the first one's fields win.
    pub declared_by: Vec<String>,
    /// Repos with content in it.
    pub repos: BTreeSet<String>,
    pub top: Vec<PlacedLink>,
    pub footer: Vec<PlacedLink>,
    /// Ordered: top islands, main islands (Unsorted last), footer islands.
    pub islands: Vec<PlacedIsland>,
    pub entries: Vec<Ref>,
}

#[derive(Debug, Default)]
pub struct Catalog {
    /// In source order.
    pub repos: Vec<Repo>,
    /// Ordered by `order`, then title; `firelight` and `default` included when used.
    pub spaces: Vec<Space>,
    /// `(repo id, diagnostic)`; the diagnostic's path is within that repo.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

impl Catalog {
    pub fn repo(&self, id: &str) -> Option<&Repo> {
        self.repos.iter().find(|r| r.id == id)
    }
    pub fn entry(&self, r: &Ref) -> Option<&Entry> {
        self.repo(&r.repo)?.entry(&r.id)
    }
    pub fn link(&self, r: &Ref) -> Option<&Link> {
        self.repo(&r.repo)?.link(&r.id)
    }
    pub fn space(&self, id: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }
    /// Diagnostics of one repo.
    pub fn diagnostics_of(&self, repo: &str) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|(r, _)| r == repo)
            .map(|(_, d)| d)
            .collect()
    }
}

fn title_from_id(id: &str) -> String {
    let mut out = String::new();
    for (i, w) in id.split('-').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut c = w.chars();
        if let Some(f) = c.next() {
            out.extend(f.to_uppercase());
            out.push_str(c.as_str());
        }
    }
    out
}

struct B<'a> {
    repos: &'a [Repo],
    diags: Vec<(String, Diagnostic)>,
    strict: Strictness,
}

impl B<'_> {
    fn warn(&mut self, repo: &str, l: &Loc, code: &'static str, msg: String) {
        self.diags.push((
            repo.to_string(),
            Diagnostic::warning(&l.path, l.line, l.col, code, msg),
        ));
    }
    fn err(&mut self, repo: &str, l: &Loc, code: &'static str, msg: String) {
        self.diags.push((
            repo.to_string(),
            Diagnostic::error(&l.path, l.line, l.col, code, msg),
        ));
    }
    /// A cross-repo ref that doesn't resolve.
    fn unresolved(&mut self, repo: &str, l: &Loc, what: &str, r: &Ref) {
        let msg = if self.repos.iter().any(|x| x.id == r.repo) {
            format!(
                "{what} {r} does not exist (repo {} has no {:?}); shown as plain text",
                r.repo, r.id
            )
        } else {
            format!(
                "{what} {r} does not resolve: repo {:?} is not connected to this Firelight; shown as plain text",
                r.repo
            )
        };
        match self.strict {
            Strictness::Lenient => self.warn(repo, l, "unresolved-ref", msg),
            Strictness::Strict => self.err(repo, l, "unresolved-ref", msg),
        }
    }
    fn find_repo(&self, id: &str) -> Option<&Repo> {
        self.repos.iter().find(|r| r.id == id)
    }
}

/// Merge repos (in configured source order) into the page model.
pub fn build(repos: Vec<Repo>, strict: Strictness) -> Catalog {
    let mut b = B {
        repos: &repos,
        diags: Vec::new(),
        strict,
    };

    // 1. Spaces: declarations, first declarer wins.
    let mut spaces: BTreeMap<String, Space> = BTreeMap::new();
    let new_space = |id: &str| Space {
        id: id.to_string(),
        title: title_from_id(id),
        summary: None,
        icon: None,
        order: 0,
        declared_by: vec![],
        repos: BTreeSet::new(),
        top: vec![],
        footer: vec![],
        islands: vec![],
        entries: vec![],
    };
    for repo in &repos {
        for d in &repo.space_decls {
            match spaces.get_mut(&d.id) {
                None => {
                    let mut s = new_space(&d.id);
                    s.title = d.title.clone();
                    s.summary = d.summary.clone();
                    s.icon = d.icon.clone();
                    s.order = d.order;
                    s.declared_by.push(repo.id.clone());
                    spaces.insert(d.id.clone(), s);
                }
                Some(s) => {
                    let differs = s.title != d.title
                        || s.summary != d.summary
                        || s.icon != d.icon
                        || s.order != d.order;
                    if differs {
                        let first = s.declared_by.first().cloned().unwrap_or_default();
                        b.warn(&repo.id, &d.loc, "space-conflict", format!("space {:?} is also declared by {first}, which comes first in the source order, so its title, summary, icon and order win; make them equal or remove this declaration", d.id));
                    }
                    s.declared_by.push(repo.id.clone());
                }
            }
        }
    }
    // Referenced but declared nowhere: still exists, titled from its id.
    let mut referenced: Vec<(String, String, Loc)> = Vec::new();
    for repo in &repos {
        let repo_loc = Loc {
            path: ".firelight/repo.yaml".into(),
            line: 0,
            col: 0,
        };
        for s in &repo.spaces {
            referenced.push((s.clone(), repo.id.clone(), repo_loc.clone()));
        }
        for i in &repo.islands {
            referenced.push((i.space.clone(), repo.id.clone(), i.loc.clone()));
        }
        for e in &repo.entries {
            for s in e.spaces.iter().flatten() {
                referenced.push((s.clone(), repo.id.clone(), e.loc.clone()));
            }
        }
        for l in &repo.links {
            for (a, al) in &l.at {
                if let At::Place { space: Some(s), .. } = a {
                    referenced.push((s.clone(), repo.id.clone(), al.clone()));
                }
            }
        }
    }
    let mut warned = BTreeSet::new();
    for (s, repo, l) in referenced {
        if !spaces.contains_key(&s) {
            let builtin = s == SPACE_DEFAULT || (s == SPACE_FIRELIGHT && repo == SELF_REPO);
            if !builtin && warned.insert(s.clone()) {
                b.warn(&repo, &l, "space-undeclared", format!("space {s:?} is declared nowhere, so it is titled {:?}; declare it in a spaces.yaml (any repo may)", title_from_id(&s)));
            }
            let mut sp = new_space(&s);
            if s == SPACE_FIRELIGHT {
                sp.title = "Firelight".into();
            }
            spaces.insert(s.clone(), sp);
        }
    }

    // 2. Islands, by ref.
    let mut islands: BTreeMap<Ref, (Island, Vec<PlacedCard>)> = BTreeMap::new();
    for repo in &repos {
        for i in &repo.islands {
            let r = Ref {
                repo: repo.id.clone(),
                id: i.id.clone(),
            };
            let cards = match &i.kind {
                IslandKind::Cards { sections, cards } => cards
                    .iter()
                    .map(|c| placed_card(c, &repo.id, sections))
                    .collect(),
                _ => vec![],
            };
            islands.insert(r, (i.clone(), cards));
        }
    }
    // Contributions add cards to cards islands.
    for repo in &repos {
        for c in &repo.contributions {
            let Some((island, cards)) = islands.get_mut(&c.island) else {
                if c.island.repo != repo.id {
                    b.unresolved(&repo.id, &c.loc, "cards island", &c.island);
                }
                continue;
            };
            let IslandKind::Cards { sections, .. } = &island.kind else {
                if c.island.repo == repo.id {
                    continue; // reported by the repo's own check
                }
                b.warn(
                    &repo.id,
                    &c.loc,
                    "contribution-target",
                    format!(
                        "{} is a {} island; cards can only be added to a cards island",
                        c.island,
                        island.kind.type_name()
                    ),
                );
                continue;
            };
            let sections = sections.clone();
            for card in &c.cards {
                if let Some(existing) = cards.iter().find(|x| x.id == card.id) {
                    b.err(
                        &repo.id,
                        &card.loc,
                        "card-clash",
                        format!(
                            "card {:?} already exists in {} (owned by {}); pick another id",
                            card.id, c.island, existing.repo
                        ),
                    );
                    continue;
                }
                cards.push(placed_card(card, &repo.id, &sections));
            }
        }
    }

    // 3. Links into islands, cards, the top/footer rows, or Unsorted.
    let mut unsorted: BTreeMap<String, BTreeMap<String, Vec<PlacedLink>>> = BTreeMap::new(); // space → repo → links
    let mut island_links: BTreeMap<Ref, Vec<PlacedLink>> = BTreeMap::new();
    for repo in &repos {
        let first_space = repo
            .default_spaces()
            .into_iter()
            .next()
            .unwrap_or_else(|| SPACE_DEFAULT.into());
        for l in &repo.links {
            let entry = match &l.entry {
                Some((r, lc)) => {
                    if b.find_repo(&r.repo).and_then(|x| x.entry(&r.id)).is_some() {
                        Some(r.clone())
                    } else {
                        if r.repo != repo.id {
                            b.unresolved(&repo.id, lc, "entry", r);
                        }
                        None
                    }
                }
                None => None,
            };
            let pl = PlacedLink {
                link: Ref {
                    repo: repo.id.clone(),
                    id: l.id.clone(),
                },
                label: l.label.clone(),
                url: l.url.clone(),
                note: l.note.clone(),
                icon: l.icon.clone(),
                entry,
                order: l.order,
            };
            for (at, al) in &l.at {
                match at {
                    At::Place { place, space } => {
                        let targets = match space {
                            Some(s) => vec![s.clone()],
                            None => repo.default_spaces(),
                        };
                        for s in targets {
                            if let Some(sp) = spaces.get_mut(&s) {
                                sp.repos.insert(repo.id.clone());
                                match place {
                                    Place::Footer => sp.footer.push(pl.clone()),
                                    _ => sp.top.push(pl.clone()),
                                }
                            } else {
                                let mut sp = new_space(&s);
                                sp.repos.insert(repo.id.clone());
                                if *place == Place::Footer {
                                    sp.footer.push(pl.clone())
                                } else {
                                    sp.top.push(pl.clone())
                                }
                                spaces.insert(s, sp);
                            }
                        }
                    }
                    At::Island { island } => match islands.get(island) {
                        Some((i, _)) if matches!(i.kind, IslandKind::Links) => island_links
                            .entry(island.clone())
                            .or_default()
                            .push(pl.clone()),
                        found => {
                            let why = match found {
                                Some((i, _)) => format!(
                                    "{island} is a {} island, not a links island",
                                    i.kind.type_name()
                                ),
                                None => format!("island {island} does not exist"),
                            };
                            b.warn(
                                &repo.id,
                                al,
                                "link-unsorted",
                                format!(
                                    "link {:?}: {why}, so it goes into the Unsorted card",
                                    l.id
                                ),
                            );
                            unsorted
                                .entry(first_space.clone())
                                .or_default()
                                .entry(repo.id.clone())
                                .or_default()
                                .push(pl.clone());
                        }
                    },
                    At::Card {
                        island,
                        card,
                        section,
                    } => {
                        let placed = match islands.get_mut(island) {
                            Some((i, cards)) if matches!(i.kind, IslandKind::Cards { .. }) => {
                                match cards.iter_mut().find(|c| &c.id == card) {
                                    Some(c) => {
                                        let mut sec = section.clone();
                                        if let Some(s) = &sec
                                            && !c
                                                .sections
                                                .iter()
                                                .any(|x| x.name.as_deref() == Some(s.as_str()))
                                        {
                                            b.warn(&repo.id, al, "unknown-section", format!("link {:?}: {island} has no section {s:?}, so the link sits outside the sections", l.id));
                                            sec = None;
                                        }
                                        if let Some(slot) =
                                            c.sections.iter_mut().find(|x| x.name == sec)
                                        {
                                            slot.links.push(pl.clone());
                                        }
                                        Ok(())
                                    }
                                    None => Err(format!("island {island} has no card {card:?}")),
                                }
                            }
                            Some((i, _)) => Err(format!(
                                "{island} is a {} island, not a cards island",
                                i.kind.type_name()
                            )),
                            None => Err(format!("island {island} does not exist")),
                        };
                        if let Err(why) = placed {
                            b.warn(
                                &repo.id,
                                al,
                                "link-unsorted",
                                format!(
                                    "link {:?}: {why}, so it goes into the Unsorted card",
                                    l.id
                                ),
                            );
                            unsorted
                                .entry(first_space.clone())
                                .or_default()
                                .entry(repo.id.clone())
                                .or_default()
                                .push(pl.clone());
                        }
                    }
                }
            }
        }
    }

    // 4. Islands into their spaces.
    for (r, (i, mut cards)) in islands {
        let kind = match &i.kind {
            IslandKind::Cards { .. } => {
                cards.sort_by(|a, b| (a.order, &a.title, &a.id).cmp(&(b.order, &b.title, &b.id)));
                for c in &mut cards {
                    for s in &mut c.sections {
                        sort_links(&mut s.links);
                    }
                }
                PlacedKind::Cards { cards }
            }
            IslandKind::Links => {
                let mut links = island_links.remove(&r).unwrap_or_default();
                sort_links(&mut links);
                PlacedKind::Links { links }
            }
            IslandKind::Template(t) => PlacedKind::Template(t.clone()),
        };
        let sp = spaces
            .entry(i.space.clone())
            .or_insert_with(|| new_space(&i.space));
        sp.repos.insert(r.repo.clone());
        if let PlacedKind::Cards { cards } = &kind {
            sp.repos.extend(cards.iter().map(|c| c.repo.clone()));
        }
        sp.islands.push(PlacedIsland {
            r,
            title: i.title.clone(),
            place: i.place,
            order: i.order,
            kind,
            unsorted: false,
        });
    }
    for (space, by_repo) in unsorted {
        let cards = by_repo
            .into_iter()
            .map(|(repo, mut links)| {
                sort_links(&mut links);
                let title = repos
                    .iter()
                    .find(|r| r.id == repo)
                    .map_or(repo.clone(), |r| r.name.clone());
                PlacedCard {
                    id: repo.clone(),
                    title,
                    tone: Tone::None,
                    order: 0,
                    repo,
                    sections: vec![PlacedSection { name: None, links }],
                }
            })
            .collect();
        let sp = spaces
            .entry(space.clone())
            .or_insert_with(|| new_space(&space));
        sp.islands.push(PlacedIsland {
            r: Ref {
                repo: String::new(),
                id: "unsorted".into(),
            },
            title: "Unsorted".into(),
            place: Place::Main,
            order: i64::MAX,
            kind: PlacedKind::Cards { cards },
            unsorted: true,
        });
    }

    // 5. Entries into spaces, and their cross-repo refs.
    for repo in &repos {
        for e in &repo.entries {
            let r = Ref {
                repo: repo.id.clone(),
                id: e.id.clone(),
            };
            for s in e.spaces_in(repo) {
                let sp = spaces.entry(s.clone()).or_insert_with(|| new_space(&s));
                sp.repos.insert(repo.id.clone());
                sp.entries.push(r.clone());
            }
            let check = |b: &mut B<'_>, t: &Ref, l: &Loc, what: &str, is_link: bool| {
                if t.repo == repo.id {
                    return; // checked per repo
                }
                let ok = b.find_repo(&t.repo).is_some_and(|x| {
                    if is_link {
                        x.link(&t.id).is_some()
                    } else {
                        x.entry(&t.id).is_some()
                    }
                });
                if !ok {
                    b.unresolved(&repo.id, l, what, t);
                }
            };
            for (t, l) in &e.links {
                check(&mut b, t, l, "link", true);
            }
            for (t, l) in &e.related {
                check(&mut b, t, l, "entry", false);
            }
            for br in &e.body.refs {
                check(
                    &mut b,
                    &br.target,
                    &br.loc,
                    if matches!(br.kind, BodyRefKind::Link) {
                        "link"
                    } else {
                        "entry"
                    },
                    matches!(br.kind, BodyRefKind::Link),
                );
            }
        }
        if let Some(a) = &repo.about {
            for br in &a.refs {
                if br.target.repo != repo.id {
                    let is_link = matches!(br.kind, BodyRefKind::Link);
                    let ok = b.find_repo(&br.target.repo).is_some_and(|x| {
                        if is_link {
                            x.link(&br.target.id).is_some()
                        } else {
                            x.entry(&br.target.id).is_some()
                        }
                    });
                    if !ok {
                        b.unresolved(
                            &repo.id,
                            &br.loc,
                            if is_link { "link" } else { "entry" },
                            &br.target,
                        );
                    }
                }
            }
        }
        for q in &repo.golden {
            for (t, l) in &q.expect {
                if t.repo != repo.id {
                    let ok = b
                        .find_repo(&t.repo)
                        .is_some_and(|x| x.entry(&t.id).is_some() || x.link(&t.id).is_some());
                    if !ok {
                        b.unresolved(&repo.id, l, "entry or link", t);
                    }
                }
            }
        }
    }

    // 6. Order everything.
    let mut spaces: Vec<Space> = spaces.into_values().collect();
    for s in &mut spaces {
        sort_links(&mut s.top);
        sort_links(&mut s.footer);
        dedup_links(&mut s.top);
        dedup_links(&mut s.footer);
        s.islands.sort_by(|a, b| {
            (a.place, a.unsorted, a.order, &a.title, &a.r)
                .cmp(&(b.place, b.unsorted, b.order, &b.title, &b.r))
        });
        s.entries.sort();
        s.entries.dedup();
    }
    spaces.sort_by(|a, b| (a.order, &a.title, &a.id).cmp(&(b.order, &b.title, &b.id)));

    let mut diags = b.diags;
    diags.sort_by(|a, b| {
        (
            &a.0,
            &a.1.path,
            a.1.line,
            a.1.col,
            a.1.severity,
            a.1.code,
            &a.1.message,
        )
            .cmp(&(
                &b.0,
                &b.1.path,
                b.1.line,
                b.1.col,
                b.1.severity,
                b.1.code,
                &b.1.message,
            ))
    });
    diags.dedup();
    Catalog {
        repos,
        spaces,
        diagnostics: diags,
    }
}

fn placed_card(c: &Card, repo: &str, sections: &[String]) -> PlacedCard {
    let mut s = vec![PlacedSection {
        name: None,
        links: vec![],
    }];
    s.extend(sections.iter().map(|n| PlacedSection {
        name: Some(n.clone()),
        links: vec![],
    }));
    PlacedCard {
        id: c.id.clone(),
        title: c.title.clone(),
        tone: c.tone,
        order: c.order,
        repo: repo.to_string(),
        sections: s,
    }
}

/// Lower order first; ties by label (§5.3).
fn sort_links(v: &mut [PlacedLink]) {
    v.sort_by(|a, b| {
        (a.order, a.label.to_lowercase(), &a.link).cmp(&(b.order, b.label.to_lowercase(), &b.link))
    });
}

fn dedup_links(v: &mut Vec<PlacedLink>) {
    let mut seen = BTreeSet::new();
    v.retain(|l| seen.insert(l.link.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(title_from_id("platform"), "Platform");
        assert_eq!(title_from_id("data-team"), "Data Team");
    }
}
