//! The page (§18): plain HTML by the server, following web/CONTRACT.md.
//! Everything important works without JavaScript.

use std::collections::{BTreeMap, BTreeSet};

use firelight_format::catalog::{Catalog, PlacedCard, PlacedIsland, PlacedKind, PlacedLink, Space};
use firelight_format::model::{Entry, EntryKind, Tone};
use firelight_format::{Ref, SPACE_ALL, SPACE_FIRELIGHT};

use crate::content::{Content, Staleness};
use crate::html::{H, attr, esc, safe_url};
use crate::markdown::{self, Resolve};

/// Page settings from the config (`ui.*` and friends).
#[derive(Debug, Clone)]
pub struct Ui {
    pub title: String,
    pub default_space: Option<String>,
    pub public_url: String,
    pub version: String,
    /// MCP needs a token (the ⓘ overlay shows a placeholder for it).
    pub mcp_token: bool,
    /// `firelight preview`: live reload.
    pub live: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            title: "Firelight".into(),
            default_space: None,
            public_url: "http://127.0.0.1:8080".into(),
            version: "0.0.0".into(),
            mcp_token: false,
            live: false,
        }
    }
}

/// The spaces a person picks from: every space but `firelight`.
pub fn visible_spaces(cat: &Catalog) -> Vec<&Space> {
    cat.spaces
        .iter()
        .filter(|s| s.id != SPACE_FIRELIGHT)
        .collect()
}

/// Where a link lives in a space, for "Show me" (§18.2): the current space if
/// the link is placed there, else its first space.
pub fn show_url(cat: &Catalog, link: &Ref, current: Option<&str>) -> Option<String> {
    let placed_in =
        |s: &Space| -> bool { firelight_search::links_in_space(cat, &s.id).contains(link) };
    if let Some(c) = current
        && c != SPACE_ALL
        && cat.space(c).is_some_and(placed_in)
    {
        return Some(format!("/s/{c}#{link}"));
    }
    if current == Some(SPACE_ALL) {
        return Some(format!("/s/all#{link}"));
    }
    cat.spaces
        .iter()
        .find(|s| placed_in(s))
        .map(|s| format!("/s/{}#{link}", s.id))
}

struct PageResolve<'a> {
    cat: &'a Catalog,
    space: Option<&'a str>,
}

impl Resolve for PageResolve<'_> {
    fn link(&self, r: &Ref) -> Option<(String, Option<String>)> {
        let l = self.cat.link(r)?;
        Some((l.url.clone(), show_url(self.cat, r, self.space)))
    }
    fn entry(&self, r: &Ref) -> Option<String> {
        self.cat.entry(r)?;
        Some(match self.space {
            Some(s) => format!("/e/{}/{}?s={}", r.repo, r.id, s),
            None => format!("/e/{}/{}", r.repo, r.id),
        })
    }
}

/// The shell around every page.
/// The shell; `ids` are the link refs that already have an `id` in `main`.
pub fn layout_with_ids(
    ui: &Ui,
    cat: &Catalog,
    content: &dyn Content,
    space: Option<&str>,
    title: &str,
    main: &str,
    mut ids: BTreeSet<Ref>,
) -> String {
    let mut h = H::new();
    let space_attr = space
        .map(|s| format!(" data-space=\"{}\"", attr(s)))
        .unwrap_or_default();
    h.fmt(format_args!("<!doctype html>\n<html lang=\"en\"{space_attr} data-theme=\"auto\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n"));
    h.fmt(format_args!(
        "<title>{}</title>\n",
        esc(&if title.is_empty() {
            ui.title.clone()
        } else {
            format!("{title} · {}", ui.title)
        })
    ));
    h.raw("<link rel=\"stylesheet\" href=\"/assets/app.css\">\n<link rel=\"stylesheet\" href=\"/assets/theme.css\">\n<link rel=\"icon\" href=\"/assets/favicon.svg\" type=\"image/svg+xml\">\n<script type=\"module\" src=\"/assets/app.js\"></script>\n");
    if ui.live {
        h.fmt(format_args!(
            "<meta name=\"firelight-live\" content=\"{}\">\n",
            content.generation()
        ));
    }
    h.raw("</head>\n<body>\n<header class=\"bar\">\n<div class=\"bar-top\">\n<nav class=\"brand\" aria-label=\"Spaces\">\n");
    h.fmt(format_args!(
        "<a class=\"title\" href=\"/\">{}</a>\n",
        esc(&ui.title)
    ));
    let spaces = visible_spaces(cat);
    let current_title = match space {
        Some(SPACE_ALL) => Some("All".to_string()),
        Some(s) => cat.space(s).map(|x| x.title.clone()),
        None => None,
    };
    h.raw("<span class=\"sep\">›</span>\n<details class=\"space-switch\">\n");
    h.fmt(format_args!(
        "<summary>{}</summary>\n<ul class=\"space-list\">\n",
        esc(current_title.as_deref().unwrap_or("Spaces"))
    ));
    for s in &spaces {
        let cur = if space == Some(s.id.as_str()) {
            " aria-current=\"page\""
        } else {
            ""
        };
        h.fmt(format_args!(
            "<li><a href=\"/s/{}\"{cur}>{}",
            attr(&s.id),
            esc(&s.title)
        ));
        if let Some(sum) = &s.summary {
            h.fmt(format_args!("<small>{}</small>", esc(sum)));
        }
        h.raw("</a></li>\n");
    }
    let cur = if space == Some(SPACE_ALL) {
        " aria-current=\"page\""
    } else {
        ""
    };
    h.fmt(format_args!("<li class=\"all\"><a href=\"/s/all\"{cur}>All<small>Everything on one page</small></a></li>\n</ul>\n</details>\n</nav>\n"));
    let search_space = space.unwrap_or(SPACE_ALL);
    h.fmt(format_args!(
        "<form class=\"search\" role=\"search\" action=\"/s/{}/search\" method=\"get\">\n<input type=\"search\" name=\"q\" placeholder=\"Search or ask…\" aria-label=\"Search\" autocomplete=\"off\">\n<kbd class=\"hint\">⌘K</kbd>\n</form>\n",
        attr(search_space)
    ));
    h.raw("<div class=\"tools\">\n<button type=\"button\" class=\"hidden-count\" data-action=\"hidden\" hidden>Hidden (<span>0</span>)</button>\n<button type=\"button\" data-action=\"edit\" aria-pressed=\"false\" title=\"Hide things\">✎</button>\n<button type=\"button\" data-action=\"info\" title=\"About this Firelight\">ⓘ</button>\n<button type=\"button\" data-action=\"theme\" title=\"Light or dark\">◐</button>\n</div>\n</div>\n");
    let sp = space.and_then(|s| cat.space(s));
    if let Some(sp) = sp
        && !sp.top.is_empty()
    {
        h.raw("<ul class=\"row top\">\n");
        for l in &sp.top {
            h.raw("<li>")
                .raw(&link_html(cat, l, space, &mut ids))
                .raw("</li>\n");
        }
        h.raw("</ul>\n");
    }
    h.raw("</header>\n")
        .raw(main)
        .raw("\n<footer class=\"bar-bottom\">\n");
    if let Some(sp) = sp
        && !sp.footer.is_empty()
    {
        h.raw("<ul class=\"row footer\">\n");
        for l in &sp.footer {
            h.raw("<li>")
                .raw(&link_html(cat, l, space, &mut ids))
                .raw("</li>\n");
        }
        h.raw("</ul>\n");
    }
    h.fmt(format_args!("<p class=\"colophon\">Firelight v{} · <a href=\"/spec\">Format</a> · <a href=\"/status\">Status</a></p>\n</footer>\n", esc(&ui.version)));
    h.raw(&dialogs(ui, cat, content));
    h.raw("</body>\n</html>\n");
    h.finish()
}

/// The shell around every page.
pub fn layout(
    ui: &Ui,
    cat: &Catalog,
    content: &dyn Content,
    space: Option<&str>,
    title: &str,
    main: &str,
) -> String {
    layout_with_ids(ui, cat, content, space, title, main, BTreeSet::new())
}

fn icon(name: &str) -> String {
    format!(
        "<svg class=\"icon\" aria-hidden=\"true\"><use href=\"/assets/icons.svg#{}\"></use></svg>",
        attr(name)
    )
}

/// One link, per the contract; `ids` holds the refs that already have an `id`.
pub fn link_html(
    cat: &Catalog,
    l: &PlacedLink,
    space: Option<&str>,
    ids: &mut BTreeSet<Ref>,
) -> String {
    let mut h = H::new();
    let Some(url) = safe_url(&l.url) else {
        return esc(&l.label);
    };
    let id = if ids.insert(l.link.clone()) {
        format!(" id=\"{}\"", attr(&l.link.to_string()))
    } else {
        String::new()
    };
    h.fmt(format_args!(
        "<a class=\"link\" href=\"{}\" data-ref=\"{}\"{id}>",
        attr(url),
        attr(&l.link.to_string())
    ));
    if let Some(i) = &l.icon {
        h.raw(&icon(i));
    }
    h.fmt(format_args!(
        "<span class=\"label\">{}</span>",
        esc(&l.label)
    ));
    if let Some(n) = &l.note {
        h.fmt(format_args!("<span class=\"note\">{}</span>", esc(n)));
    }
    h.raw("</a>");
    if let Some(e) = &l.entry
        && cat.entry(e).is_some()
    {
        let s = space.map(|s| format!("?s={}", attr(s))).unwrap_or_default();
        h.fmt(format_args!(
            "<a class=\"what\" href=\"/e/{}/{}{s}\" title=\"What is this?\">?</a>",
            attr(&e.repo),
            attr(&e.id)
        ));
    }
    h.finish()
}

fn eye(r: &str) -> String {
    format!(
        "<button type=\"button\" class=\"eye\" data-hide=\"{}\" aria-pressed=\"false\" title=\"Hide\">👁</button>",
        attr(r)
    )
}

fn card_html(
    cat: &Catalog,
    island: &Ref,
    c: &PlacedCard,
    space: Option<&str>,
    ids: &mut BTreeSet<Ref>,
) -> String {
    let mut h = H::new();
    let tone = match c.tone {
        Tone::Danger => " tone-danger",
        Tone::Warn => " tone-warn",
        Tone::None => "",
    };
    let r = format!("{island}/{}", c.id);
    h.fmt(format_args!(
        "<article class=\"card{tone}\" data-ref=\"{}\">\n<h3>{}</h3>{}\n",
        attr(&r),
        esc(&c.title),
        eye(&r)
    ));
    let mut any = false;
    for sec in &c.sections {
        if sec.links.is_empty() {
            continue;
        }
        any = true;
        if let Some(n) = &sec.name {
            h.fmt(format_args!("<h4>{}</h4>", esc(n)));
        }
        h.raw("<ul class=\"links\">\n");
        for l in &sec.links {
            h.raw("<li>")
                .raw(&link_html(cat, l, space, ids))
                .raw("</li>\n");
        }
        h.raw("</ul>\n");
    }
    if !any {
        h.raw("<p class=\"empty\">—</p>\n");
    }
    h.raw("</article>\n");
    h.finish()
}

/// One island. `filled` holds a template's rendered links (on `/i/…`).
pub fn island_html(
    cat: &Catalog,
    i: &PlacedIsland,
    space: Option<&str>,
    ids: &mut BTreeSet<Ref>,
    filled: Option<&TemplateFill>,
) -> String {
    let mut h = H::new();
    let r = i.r.to_string();
    let id = format!("island-{r}");
    match &i.kind {
        PlacedKind::Cards { cards } => {
            let extra = if i.unsorted { " unsorted" } else { "" };
            h.fmt(format_args!("<section class=\"island cards{extra}\" data-ref=\"{}\" id=\"{}\">\n<h2>{}</h2>{}\n<div class=\"card-grid\">\n", attr(&r), attr(&id), esc(&i.title), eye(&r)));
            for c in cards {
                h.raw(&card_html(cat, &i.r, c, space, ids));
            }
            h.raw("</div>\n</section>\n");
        }
        PlacedKind::Links { links } => {
            h.fmt(format_args!("<section class=\"island links-island\" data-ref=\"{}\" id=\"{}\">\n<h2>{}</h2>{}\n<ul class=\"row\">\n", attr(&r), attr(&id), esc(&i.title), eye(&r)));
            for l in links {
                h.raw("<li>")
                    .raw(&link_html(cat, l, space, ids))
                    .raw("</li>\n");
            }
            h.raw("</ul>\n</section>\n");
        }
        PlacedKind::Template(t) => {
            h.fmt(format_args!(
                "<section class=\"island template\" data-ref=\"{}\" id=\"{}\">\n<h2>{}</h2>{}\n",
                attr(&r),
                attr(&id),
                esc(&i.title),
                eye(&r)
            ));
            h.fmt(format_args!(
                "<form class=\"template-form\" method=\"get\" action=\"/i/{}/{}\">\n",
                attr(&i.r.repo),
                attr(&i.r.id)
            ));
            if let Some(s) = space {
                h.fmt(format_args!(
                    "<input type=\"hidden\" name=\"s\" value=\"{}\">\n",
                    attr(s)
                ));
            }
            for inp in &t.inputs {
                let val = filled
                    .and_then(|f| f.values.get(&inp.name))
                    .map(|v| format!(" value=\"{}\"", attr(v)))
                    .unwrap_or_default();
                let ph = inp
                    .placeholder
                    .as_deref()
                    .map(|p| format!(" placeholder=\"{}\"", attr(p)))
                    .unwrap_or_default();
                h.fmt(format_args!(
                    "<label>{} <input name=\"{}\"{ph}{val} required autocomplete=\"off\"></label>\n",
                    esc(&inp.label),
                    attr(&inp.name),
                ));
            }
            h.raw("<button type=\"submit\">Open</button>\n</form>\n");
            if let Some(help) = &t.help {
                h.fmt(format_args!("<p class=\"help\">{}</p>\n", esc(help)));
            }
            match filled {
                Some(TemplateFill {
                    links: Ok(links), ..
                }) => {
                    h.raw("<ul class=\"links template-links\" data-filled=\"true\">\n");
                    for (label, url, ic) in links {
                        if let Some(u) = safe_url(url) {
                            h.fmt(format_args!("<li><a class=\"link\" href=\"{}\">{}<span class=\"label\">{}</span></a></li>\n", attr(u), ic.as_deref().map(icon).unwrap_or_default(), esc(label)));
                        }
                    }
                    h.raw("</ul>\n<p class=\"template-error\" hidden></p>\n");
                }
                Some(TemplateFill { links: Err(e), .. }) => {
                    h.raw("<ul class=\"links template-links\" data-filled=\"false\"></ul>\n");
                    h.fmt(format_args!("<p class=\"template-error\">{}</p>\n", esc(e)));
                }
                None => {
                    h.raw("<ul class=\"links template-links\" data-filled=\"false\"></ul>\n<p class=\"template-error\" hidden></p>\n");
                }
            }
            h.raw("</section>\n");
        }
    }
    h.finish()
}

/// A template island filled from its form values.
pub struct TemplateFill {
    pub values: BTreeMap<String, String>,
    /// `(label, url, icon)` per link, or why nothing renders.
    pub links: Result<Vec<(String, String, Option<String>)>, String>,
}

/// Fill a template island (§5.4): every value must match its pattern and its
/// positions' rules, or nothing renders.
pub fn fill_template(
    t: &firelight_format::model::Template,
    values: &BTreeMap<String, String>,
) -> TemplateFill {
    use firelight_format::template::{check_pattern, check_url, fill};
    let names: Vec<&str> = t.inputs.iter().map(|i| i.name.as_str()).collect();
    let mut patterns = BTreeMap::new();
    for i in &t.inputs {
        match check_pattern(&i.pattern) {
            Ok(re) => {
                patterns.insert(i.name.clone(), re);
            }
            Err(e) => {
                return TemplateFill {
                    values: values.clone(),
                    links: Err(e),
                };
            }
        }
    }
    for i in &t.inputs {
        let v = values.get(&i.name).map(String::as_str).unwrap_or("");
        if v.is_empty() {
            return TemplateFill {
                values: values.clone(),
                links: Err(format!("{} is empty", i.label)),
            };
        }
        if !patterns.get(&i.name).is_some_and(|re| re.is_match(v)) {
            return TemplateFill {
                values: values.clone(),
                links: Err(format!("{} doesn't look right", i.label)),
            };
        }
    }
    let mut out = Vec::new();
    for l in &t.links {
        let filled = check_url(&l.url, &names).and_then(|u| fill(&u, values, &patterns));
        match filled {
            Ok(u) => out.push((l.label.clone(), u, l.icon.clone())),
            Err(e) => {
                return TemplateFill {
                    values: values.clone(),
                    links: Err(e),
                };
            }
        }
    }
    TemplateFill {
        values: values.clone(),
        links: Ok(out),
    }
}

/// `/s/<space>` (or `/s/all`).
pub fn space_page(
    ui: &Ui,
    content: &dyn Content,
    space: &str,
    fills: &BTreeMap<Ref, TemplateFill>,
) -> Option<String> {
    let cat = content.catalog();
    let mut ids = BTreeSet::new();
    let mut main = H::new();
    if space == SPACE_ALL {
        main.raw("<main class=\"space all\">\n");
        for s in visible_spaces(&cat) {
            if s.islands.is_empty() {
                continue;
            }
            main.fmt(format_args!("<section class=\"space-group\" data-space=\"{}\"><h2 class=\"space-title\">{}</h2>\n", attr(&s.id), esc(&s.title)));
            for i in &s.islands {
                main.raw(&island_html(
                    &cat,
                    i,
                    Some(SPACE_ALL),
                    &mut ids,
                    fills.get(&i.r),
                ));
            }
            main.raw("</section>\n");
        }
        main.raw("</main>");
        return Some(layout_with_ids(
            ui,
            &cat,
            content,
            Some(SPACE_ALL),
            "All",
            &main.finish(),
            ids,
        ));
    }
    let s = cat.space(space)?;
    main.fmt(format_args!("<main class=\"space\">\n"));
    if s.islands.is_empty() {
        main.raw("<p class=\"empty-space\">Nothing on this page yet. Add a <code>.firelight/</code> folder to a repo; the ⓘ menu has the prompt.</p>\n");
    }
    for i in &s.islands {
        main.raw(&island_html(
            &cat,
            i,
            Some(space),
            &mut ids,
            fills.get(&i.r),
        ));
    }
    if !s.entries.is_empty() {
        main.raw("<section class=\"island entries\"><h2>Concepts and guides</h2>\n<ul class=\"entry-list\">\n");
        for r in &s.entries {
            if let Some(e) = cat.entry(r) {
                main.fmt(format_args!(
                    "<li><span class=\"kind\">{}</span> <a href=\"/e/{}/{}?s={}\">{}</a> <span class=\"summary\">{}</span></li>\n",
                    kind_label(e.kind),
                    attr(&r.repo),
                    attr(&r.id),
                    attr(space),
                    esc(&e.title),
                    esc(&e.summary)
                ));
            }
        }
        main.raw("</ul>\n</section>\n");
    }
    main.raw("</main>");
    let title = s.title.clone();
    Some(layout_with_ids(
        ui,
        &cat,
        content,
        Some(space),
        &title,
        &main.finish(),
        ids,
    ))
}

fn kind_label(k: EntryKind) -> &'static str {
    match k {
        EntryKind::Concept => "Concept",
        EntryKind::Guide => "Guide",
    }
}

/// `/` without a redirect: the space list.
pub fn space_index(ui: &Ui, content: &dyn Content) -> String {
    let cat = content.catalog();
    let mut m = H::new();
    m.raw("<main class=\"space-index\">\n<ul class=\"space-cards\">\n");
    for s in visible_spaces(&cat) {
        m.fmt(format_args!(
            "<li><a href=\"/s/{}\">{}<strong>{}</strong>",
            attr(&s.id),
            s.icon.as_deref().map(icon).unwrap_or_default(),
            esc(&s.title)
        ));
        if let Some(sum) = &s.summary {
            m.fmt(format_args!("<span>{}</span>", esc(sum)));
        }
        m.raw("</a></li>\n");
    }
    m.raw("<li class=\"all\"><a href=\"/s/all\"><strong>All</strong><span>Everything on one page</span></a></li>\n</ul>\n</main>");
    layout(ui, &cat, content, None, "", &m.finish())
}

/// `/e/<repo>/<id>`.
pub fn entry_page(
    ui: &Ui,
    content: &dyn Content,
    r: &Ref,
    from_space: Option<&str>,
) -> Option<String> {
    let cat = content.catalog();
    let repo = cat.repo(&r.repo)?;
    let e: &Entry = repo.entry(&r.id)?;
    let space_list = e.spaces_in(repo);
    let back_space = from_space
        .filter(|s| *s == SPACE_ALL || cat.space(s).is_some())
        .map(str::to_string)
        .or_else(|| space_list.first().cloned());
    let resolve = PageResolve {
        cat: &cat,
        space: back_space.as_deref(),
    };
    let mut m = H::new();
    m.raw("<main class=\"entry\">\n");
    if let Some(bs) = &back_space {
        let t = if bs == SPACE_ALL {
            "All".to_string()
        } else {
            cat.space(bs).map_or(bs.clone(), |s| s.title.clone())
        };
        m.fmt(format_args!(
            "<p class=\"back\"><a href=\"/s/{}\">← {}</a></p>\n",
            attr(bs),
            esc(&t)
        ));
    }
    m.fmt(format_args!(
        "<p class=\"meta\"><span class=\"kind\">{}</span> · <a href=\"/r/{}\">{}</a>",
        kind_label(e.kind),
        attr(&repo.id),
        esc(&repo.id)
    ));
    let stale: Option<Staleness> = content.staleness(&repo.id, &e.path);
    if let Some(st) = &stale {
        m.fmt(format_args!(
            " <span class=\"stale\" title=\"{}\">⚠ maybe stale</span>",
            attr(&format!(
                "The paths this entry covers changed on {} after the entry did ({}).",
                day(&st.covers_changed),
                day(&st.entry_changed)
            ))
        ));
    }
    m.raw("</p>\n");
    m.fmt(format_args!(
        "<h1>{}</h1>\n<p class=\"summary\">{}</p>\n",
        esc(&e.title),
        esc(&e.summary)
    ));
    m.raw("<div class=\"prose\">\n")
        .raw(&markdown::render(&e.body.source, &repo.id, &resolve))
        .raw("</div>\n");
    let related: Vec<&Ref> = e
        .related
        .iter()
        .map(|(r, _)| r)
        .filter(|r| cat.entry(r).is_some())
        .collect();
    if !related.is_empty() {
        m.raw("<aside class=\"related\"><h2>Related</h2><ul>\n");
        for rr in related {
            if let (Some(t), Some(url)) = (cat.entry(rr), resolve.entry(rr)) {
                let prefix = if rr.repo == repo.id {
                    String::new()
                } else {
                    format!("{} › ", esc(&rr.repo))
                };
                m.fmt(format_args!(
                    "<li><a href=\"{}\">{prefix}{}</a></li>\n",
                    attr(&url),
                    esc(&t.title)
                ));
            }
        }
        m.raw("</ul></aside>\n");
    }
    let src = content.source(&repo.id);
    m.fmt(format_args!(
        "<p class=\"source\">Source: <code>{}/{}</code>",
        esc(&repo.id),
        esc(&e.path)
    ));
    if let Some(s) = &src {
        let short: String = s.commit.chars().take(7).collect();
        m.fmt(format_args!(" · commit <code>{}</code>", esc(&short)));
        if let Some(at) = &s.committed_at {
            m.fmt(format_args!(" · {}", esc(&day(at))));
        }
    }
    m.raw("</p>\n</main>");
    let page_space = from_space.filter(|s| *s == SPACE_ALL || cat.space(s).is_some());
    Some(layout(ui, &cat, content, page_space, &e.title, &m.finish()))
}

fn day(rfc3339: &str) -> String {
    rfc3339.chars().take(10).collect()
}

/// `/r/<repo>`: what a repo is and how it relates to others.
pub fn repo_page(ui: &Ui, content: &dyn Content, id: &str) -> Option<String> {
    let cat = content.catalog();
    let repo = cat.repo(id)?;
    let resolve = PageResolve {
        cat: &cat,
        space: None,
    };
    let mut m = H::new();
    m.fmt(format_args!("<main class=\"entry repo\">\n<p class=\"meta\"><span class=\"kind\">Repo</span></p>\n<h1>{}</h1>\n<p class=\"summary\">{}</p>\n", esc(&repo.name), esc(&repo.summary)));
    if let Some(o) = &repo.owner {
        m.fmt(format_args!("<p class=\"owner\">Owner: {}</p>\n", esc(o)));
    }
    if let Some(a) = &repo.about {
        m.raw("<div class=\"prose\">\n")
            .raw(&markdown::render(&a.source, &repo.id, &resolve))
            .raw("</div>\n");
    }
    if !repo.relations.is_empty() {
        m.raw("<h2>Relations</h2>\n<ul class=\"relations\">\n");
        for rel in &repo.relations {
            let connected = cat.repo(&rel.repo).is_some();
            let target = if connected {
                format!("<a href=\"/r/{}\">{}</a>", attr(&rel.repo), esc(&rel.repo))
            } else {
                format!(
                    "{} <span class=\"not-connected\">not connected</span>",
                    esc(&rel.repo)
                )
            };
            m.fmt(format_args!(
                "<li><strong>When</strong> {} (in {target}): {}</li>\n",
                esc(&rel.when),
                esc(&rel.then)
            ));
        }
        m.raw("</ul>\n");
    }
    m.raw("</main>");
    Some(layout(ui, &cat, content, None, &repo.name, &m.finish()))
}

/// The no-JS search results page.
pub fn search_page(ui: &Ui, content: &dyn Content, space: &str, q: &str) -> String {
    let cat = content.catalog();
    let hits = firelight_search::search(&cat, space, q, 25);
    let mut m = H::new();
    m.fmt(format_args!(
        "<main class=\"search-page\">\n<h1>Results for “{}”</h1>\n",
        esc(q)
    ));
    if hits.is_empty() {
        m.raw("<p class=\"empty\">Nothing found. Try other words, or browse the page.</p>\n");
    } else {
        m.raw("<ol class=\"hits\">\n");
        for hit in &hits {
            let url = hit_url(&cat, hit, space);
            m.fmt(format_args!(
                "<li><span class=\"kind\">{}</span> <a href=\"{}\">{}</a> <span class=\"repo\">{}</span><p>{}</p></li>\n",
                esc(&hit.kind.as_str().to_uppercase()),
                attr(&url),
                esc(&hit.title),
                esc(&hit.r.repo),
                esc(&hit.summary)
            ));
        }
        m.raw("</ol>\n");
    }
    m.raw("</main>");
    layout(
        ui,
        &cat,
        content,
        Some(space),
        &format!("Search: {q}"),
        &m.finish(),
    )
}

/// Where a hit opens: an entry's permalink (at its best section), a link's URL.
pub fn hit_url(cat: &Catalog, hit: &firelight_search::Hit, space: &str) -> String {
    match hit.kind {
        firelight_search::Kind::Link => hit
            .url
            .as_deref()
            .and_then(safe_url)
            .map(str::to_string)
            .or_else(|| show_url(cat, &hit.r, Some(space)))
            .unwrap_or_default(),
        firelight_search::Kind::Repo => format!("/r/{}", hit.r.repo),
        _ => {
            let anchor = hit
                .heading
                .as_ref()
                .map(|(_, s)| format!("#{s}"))
                .unwrap_or_default();
            format!("/e/{}/{}?s={}{anchor}", hit.r.repo, hit.r.id, space)
        }
    }
}

/// The JSON for ⌘K (web/CONTRACT.md).
pub fn search_json(content: &dyn Content, space: &str, q: &str) -> serde_json::Value {
    let cat = content.catalog();
    let hits = firelight_search::search(&cat, space, q, 25);
    let rows: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            let mut v = serde_json::json!({
                "kind": h.kind.as_str(), "ref": h.r.to_string(), "title": h.title, "repo": h.r.repo,
                "summary": h.summary, "url": hit_url(&cat, h, space),
            });
            if let Some((t, _)) = &h.heading {
                v["heading"] = serde_json::json!(t);
            }
            if h.kind == firelight_search::Kind::Link {
                v["show"] = serde_json::json!(show_url(&cat, &h.r, Some(space)));
            }
            v
        })
        .collect();
    serde_json::json!({ "hits": rows })
}

/// The ⓘ overlay and the other dialogs (§18.5).
fn dialogs(ui: &Ui, cat: &Catalog, _content: &dyn Content) -> String {
    let mut h = H::new();
    h.raw("<dialog id=\"palette\" class=\"palette\" aria-label=\"Search\">\n<form method=\"dialog\"><input type=\"search\" placeholder=\"Search or ask…\" aria-label=\"Search\" autocomplete=\"off\"></form>\n<ul class=\"results\" role=\"listbox\"></ul>\n<p class=\"palette-empty\" hidden>Nothing found.</p>\n</dialog>\n");
    let mcp_url = format!("{}/mcp", ui.public_url.trim_end_matches('/'));
    let header = if ui.mcp_token {
        " --header \"Authorization: Bearer <your token>\""
    } else {
        ""
    };
    h.raw("<dialog id=\"info\" class=\"info\" aria-label=\"About this Firelight\">\n<nav class=\"tabs\" role=\"tablist\">\n<button role=\"tab\" aria-selected=\"true\" data-tab=\"mcp\">Connect MCP</button>\n<button role=\"tab\" aria-selected=\"false\" data-tab=\"repo\">Add a repo</button>\n<button role=\"tab\" aria-selected=\"false\" data-tab=\"status\">Status</button>\n</nav>\n");
    if cat.space(SPACE_FIRELIGHT).is_some() {
        h.raw("<p class=\"docs-link\"><a href=\"/s/firelight\">Firelight docs</a>: how to write <code>.firelight/</code>, connect agents and run Firelight.</p>\n");
    }
    h.raw("<section data-panel=\"mcp\">\n<p>Give your coding agent every repo's docs. In Claude Code:</p>\n");
    h.fmt(format_args!(
        "<pre class=\"copy\"><code>claude mcp add --transport http firelight {}{}</code></pre>\n",
        esc(&mcp_url),
        esc(header)
    ));
    h.raw("<p>Any other MCP client:</p>\n");
    let json = if ui.mcp_token {
        format!(
            "{{\"mcpServers\": {{\"firelight\": {{\"type\": \"http\", \"url\": \"{mcp_url}\", \"headers\": {{\"Authorization\": \"Bearer <your token>\"}}}}}}}}"
        )
    } else {
        format!(
            "{{\"mcpServers\": {{\"firelight\": {{\"type\": \"http\", \"url\": \"{mcp_url}\"}}}}}}"
        )
    };
    h.fmt(format_args!(
        "<pre class=\"copy\"><code>{}</code></pre>\n</section>\n",
        esc(&json)
    ));
    h.raw("<section data-panel=\"repo\" hidden>\n<p>Paste this into a coding agent in the repo you want to add:</p>\n");
    h.fmt(format_args!(
        "<pre class=\"copy\"><code>{}</code></pre>\n</section>\n",
        esc(crate::onboarding::PROMPT)
    ));
    h.raw("<section data-panel=\"status\" hidden>\n");
    h.fmt(format_args!(
        "<p>{} repos, {} spaces. <a href=\"/status\">Full status</a></p>\n",
        cat.repos.len(),
        visible_spaces(cat).len()
    ));
    h.raw("</section>\n<form method=\"dialog\"><button>Close</button></form>\n</dialog>\n");
    h.raw("<dialog id=\"hidden-dialog\" aria-label=\"Hidden items\">\n<h2>Hidden</h2><ul class=\"hidden-list\"></ul>\n<p><button type=\"button\" data-action=\"reset-hidden\">Show everything again</button></p>\n<form method=\"dialog\"><button>Close</button></form>\n</dialog>\n");
    h.finish()
}

/// A plain page for errors and not-found.
pub fn message_page(ui: &Ui, content: &dyn Content, title: &str, text: &str) -> String {
    let cat = content.catalog();
    let main = format!(
        "<main class=\"message\"><h1>{}</h1><p>{}</p><p><a href=\"/\">Back to the start</a></p></main>",
        esc(title),
        esc(text)
    );
    layout(ui, &cat, content, None, title, &main)
}
