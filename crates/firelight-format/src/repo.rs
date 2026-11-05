//! Parse and check one `.firelight/` folder (§5). No I/O: the caller hands in
//! the files.

use std::collections::{BTreeMap, BTreeSet};

use serde::de::DeserializeOwned;

use crate::diag::{self, Diagnostic};
use crate::icons::is_icon;
use crate::ids::{Ref, id_rule, valid_id};
use crate::markdown::{scan, split_front_matter};
use crate::model::*;
use crate::pos::P;
use crate::raw::*;
use crate::template;
use crate::yaml;
use crate::{FORMAT_VERSIONS, RESERVED_SPACES, SELF_REPO, SPACE_ALL, SPACE_FIRELIGHT};

/// Per-source limits (§5, configurable through `limits.*`).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_files: u64,
    pub max_source_bytes: u64,
    pub max_file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_files: 1000,
            max_source_bytes: 8 << 20,
            max_file_bytes: 256 << 10,
        }
    }
}

/// One file of a `.firelight/` folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputFile {
    /// Relative to `.firelight/`, with `/` separators, e.g. `guides/x.md`.
    pub path: String,
    pub kind: FileKind,
    /// `None` when not read: a symlink, a special file, or over a size limit.
    pub contents: Option<Vec<u8>>,
    /// The file's size (also when its contents were not read).
    pub size: u64,
}

impl InputFile {
    /// A regular file with its contents.
    pub fn regular(path: impl Into<String>, contents: Vec<u8>) -> Self {
        InputFile {
            path: path.into(),
            kind: FileKind::Regular,
            size: contents.len() as u64,
            contents: Some(contents),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Regular,
    /// Refused, never followed.
    Symlink,
    /// A FIFO, socket or device: refused, never read.
    Special,
}

const SUMMARY_MAX: usize = 200;
const PREFIX: &str = ".firelight/";

/// Parse a `.firelight/` folder. `expected_id` is the configured source id,
/// if any. Returns `None` when there is nothing usable (no repo.yaml, an
/// unsupported format version, a limit exceeded); otherwise the repo, which is
/// only fit to serve when no diagnostic is an error.
pub fn parse_repo(
    files: &[InputFile],
    expected_id: Option<&str>,
    limits: &Limits,
) -> (Option<Repo>, Vec<Diagnostic>) {
    let mut c = Checker {
        diags: Vec::new(),
        home: String::new(),
    };
    let repo = c.run(files, expected_id, limits);
    diag::sort(&mut c.diags);
    (repo, c.diags)
}

struct Checker {
    diags: Vec<Diagnostic>,
    home: String,
}

fn loc<T>(path: &str, p: &P<T>) -> Loc {
    Loc {
        path: path.to_string(),
        line: p.line,
        col: p.col,
    }
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

impl Checker {
    fn err(&mut self, path: &str, line: u32, col: u32, code: &'static str, msg: impl Into<String>) {
        self.diags
            .push(Diagnostic::error(path, line, col, code, msg));
    }
    fn warn(
        &mut self,
        path: &str,
        line: u32,
        col: u32,
        code: &'static str,
        msg: impl Into<String>,
    ) {
        self.diags
            .push(Diagnostic::warning(path, line, col, code, msg));
    }
    fn err_at<T>(&mut self, path: &str, p: &P<T>, code: &'static str, msg: impl Into<String>) {
        self.err(path, p.line, p.col, code, msg);
    }
    fn warn_at<T>(&mut self, path: &str, p: &P<T>, code: &'static str, msg: impl Into<String>) {
        self.warn(path, p.line, p.col, code, msg);
    }

    fn run(
        &mut self,
        files: &[InputFile],
        expected_id: Option<&str>,
        limits: &Limits,
    ) -> Option<Repo> {
        // Limits first, before anything is parsed.
        if files.len() as u64 > limits.max_files {
            self.err(
                PREFIX,
                0,
                0,
                "too-many-files",
                format!(
                    ".firelight/ has {} files; the limit is {} (limits.max_files)",
                    files.len(),
                    limits.max_files
                ),
            );
            return None;
        }
        let total: u64 = files
            .iter()
            .map(|f| f.contents.as_ref().map_or(f.size, |c| c.len() as u64))
            .sum();
        if total > limits.max_source_bytes {
            self.err(
                PREFIX,
                0,
                0,
                "too-large",
                format!(
                    ".firelight/ holds {total} bytes; the limit is {} (limits.max_source_bytes)",
                    limits.max_source_bytes
                ),
            );
            return None;
        }
        let mut texts: BTreeMap<String, String> = BTreeMap::new();
        for f in files {
            let path = format!("{PREFIX}{}", f.path);
            if f.kind == FileKind::Special {
                self.err(&path, 0, 0, "special-file", "not a regular file (a FIFO, socket or device); .firelight/ holds plain files only");
                continue;
            }
            let Some(bytes) = &f.contents else {
                if f.kind == FileKind::Regular {
                    self.err(
                        &path,
                        0,
                        0,
                        "file-too-large",
                        format!(
                            "{} bytes; the limit is {} (limits.max_file_bytes)",
                            f.size, limits.max_file_bytes
                        ),
                    );
                } else {
                    self.err(
                        &path,
                        0,
                        0,
                        "symlink",
                        "symlinks are refused in .firelight/; copy the file instead",
                    );
                }
                continue;
            };
            if bytes.len() as u64 > limits.max_file_bytes {
                self.err(
                    &path,
                    0,
                    0,
                    "file-too-large",
                    format!(
                        "{} bytes; the limit is {} (limits.max_file_bytes)",
                        bytes.len(),
                        limits.max_file_bytes
                    ),
                );
                continue;
            }
            match String::from_utf8(bytes.clone()) {
                Ok(s) => {
                    texts.insert(f.path.clone(), s);
                }
                Err(_) => self.err(&path, 0, 0, "not-utf8", "not UTF-8 text"),
            }
        }

        // Every YAML file and front matter must state a version we read.
        let mut no_version: BTreeSet<String> = BTreeSet::new();
        for (p, text) in &texts {
            if !is_format_file(p) {
                continue; // reported as an unknown file below
            }
            let yaml_text = if p.ends_with(".md") {
                if p == "about.md" {
                    continue;
                }
                match split_front_matter(text) {
                    Some((y, _, _)) => y,
                    None => continue,
                }
            } else if p.ends_with(".yaml") || p.ends_with(".yml") {
                text.as_str()
            } else {
                continue;
            };
            let off = u32::from(p.ends_with(".md"));
            let path = format!("{PREFIX}{p}");
            match yaml::parse::<VersionOnly>(yaml_text, &path, off) {
                Err(_) => {} // reported by the full parse
                Ok(v) => {
                    match v.format {
                        None => {
                            self.err(&path, 1 + off, 1, "format-missing", "missing `format: 1` (every YAML file and front matter starts with it)");
                            no_version.insert(p.clone());
                        }
                        Some(f) => {
                            let n = f.value.as_u64().and_then(|n| u32::try_from(n).ok());
                            if !n.is_some_and(|n| FORMAT_VERSIONS.contains(&n)) {
                                self.err(
                                &path,
                                f.line + off,
                                f.col,
                                "format-unsupported",
                                format!("format {} is not one this Firelight reads ({}). The whole source is skipped until it is updated or Firelight is upgraded", f.value, FORMAT_VERSIONS.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")),
                            );
                                return None;
                            }
                        }
                    }
                }
            }
        }

        // repo.yaml first: it names the repo every ref is relative to.
        let Some(repo_text) = texts.get("repo.yaml") else {
            if texts.contains_key("repo.yml") {
                self.err(
                    ".firelight/repo.yml",
                    0,
                    0,
                    "file-name",
                    "name it repo.yaml",
                );
            }
            self.err(
                ".firelight/repo.yaml",
                0,
                0,
                "repo-missing",
                "missing: every .firelight/ folder needs a repo.yaml (format, id, name, summary)",
            );
            return None;
        };
        let path = ".firelight/repo.yaml";
        let ry: RepoYaml = match yaml::parse(repo_text, path, 0) {
            Ok(r) => r,
            Err(d) => {
                self.diags.push(d);
                // Keep checking the other files with whatever id it has, so
                // every problem shows at once; the repo is unusable anyway.
                let loose = yaml::parse::<LooseRepo>(repo_text, path, 0)
                    .ok()
                    .and_then(|l| l.id);
                match loose {
                    Some(id) if valid_id(&id) => RepoYaml {
                        format: P::new(1, 1, 1),
                        name: id.map(id.value.clone()),
                        summary: id.map("-".to_string()),
                        id,
                        owner: None,
                        spaces: vec![],
                        relations: vec![],
                    },
                    _ => return None,
                }
            }
        };
        if !valid_id(&ry.id) {
            self.err_at(path, &ry.id, "id", id_rule("repo id", &ry.id));
            return None;
        }
        self.home = ry.id.value.clone();
        if let Some(e) = expected_id
            && e != ry.id.value
        {
            self.err_at(
                path,
                &ry.id,
                "repo-id-mismatch",
                format!(
                    "id {:?} must equal the source id {e:?} this Firelight is configured with",
                    ry.id.value
                ),
            );
        }
        if ry.id.value == SELF_REPO && expected_id.is_some_and(|e| e != SELF_REPO) {
            self.err_at(
                path,
                &ry.id,
                "reserved-id",
                "firelight is reserved for Firelight's own docs",
            );
        }
        self.nonempty(path, &ry.name, "name");
        self.summary(path, &ry.summary);
        let mut repo = Repo {
            id: ry.id.value.clone(),
            name: ry.name.value.clone(),
            summary: ry.summary.value.clone(),
            owner: ry.owner.clone(),
            ..Repo::default()
        };
        for s in &ry.spaces {
            if self.space_ref(path, s) {
                repo.spaces.push(s.value.clone());
            }
        }
        for r in &ry.relations {
            if !valid_id(&r.repo) {
                self.err_at(path, &r.repo, "id", id_rule("repo id", &r.repo));
                continue;
            }
            self.nonempty(path, &r.when, "when");
            self.nonempty(path, &r.then, "then");
            repo.relations.push(Relation {
                repo: r.repo.value.clone(),
                when: r.when.value.clone(),
                then: r.then.value.clone(),
                loc: loc(path, &r.repo),
            });
        }

        let mut ids: BTreeMap<String, String> = BTreeMap::new(); // id → where
        let mut claim = |c: &mut Checker, id: &P<String>, path: &str, what: &str| -> bool {
            if !valid_id(id) {
                c.err_at(path, id, "id", id_rule(&format!("{what} id"), id));
                return false;
            }
            if let Some(prev) = ids.get(&id.value) {
                c.err_at(path, id, "duplicate-id", format!("id {:?} is already used by {prev}; ids are unique per repo across concepts, guides, links and islands", id.value));
                return false;
            }
            ids.insert(id.value.clone(), format!("{path}:{}", id.line));
            true
        };

        for (p, text) in &texts {
            if no_version.contains(p) {
                continue;
            }
            let path = format!("{PREFIX}{p}");
            let parts: Vec<&str> = p.split('/').collect();
            match parts.as_slice() {
                ["repo.yaml"] => {}
                ["about.md"] => {
                    let (md, d) = scan(text, &self.home, &path, 1);
                    self.diags.extend(d);
                    repo.about = Some(md);
                }
                ["links.yaml"] => {
                    if let Some(ly) = self.parse::<LinksYaml>(text, &path, 0) {
                        for l in ly.links {
                            if !claim(self, &l.id, &path, "link") {
                                continue;
                            }
                            if let Some(link) = self.link(&path, l) {
                                repo.links.push(link);
                            }
                        }
                    }
                }
                ["islands", file] if file.ends_with(".yml") => {
                    self.err(&path, 0, 0, "file-name", "island files end in .yaml");
                }
                ["islands", file] if file.ends_with(".yaml") => {
                    let dspace = repo.default_spaces().into_iter().next().unwrap_or_default();
                    let mut out = IslandsOut {
                        islands: &mut repo.islands,
                        contributions: &mut repo.contributions,
                    };
                    self.island(&path, text, stem(file), &dspace, &mut out, &mut claim);
                }
                ["concepts", file] | ["guides", file] if file.ends_with(".md") => {
                    let kind = if parts[0] == "concepts" {
                        EntryKind::Concept
                    } else {
                        EntryKind::Guide
                    };
                    if let Some(e) = self.entry(&path, text, stem(file), kind) {
                        let idp = P::new(e.id.clone(), e.loc.line, e.loc.col);
                        if claim(self, &idp, &path, "entry") {
                            repo.entries.push(e);
                        }
                    }
                }
                ["golden.yaml"] => {
                    if let Some(g) = self.parse::<GoldenYaml>(text, &path, 0) {
                        for q in g.questions {
                            self.nonempty(&path, &q.query, "query");
                            let mut expect = Vec::new();
                            for e in &q.expect {
                                if let Some(r) = self.reference(&path, e) {
                                    expect.push((r, loc(&path, e)));
                                }
                            }
                            if expect.is_empty() {
                                self.err_at(
                                    &path,
                                    &q.query,
                                    "golden-expect",
                                    "expect needs at least one ref",
                                );
                            }
                            let needs_meaning = match &q.needs {
                                None => false,
                                Some(n) if n.value == "meaning" => true,
                                Some(n) => {
                                    self.err_at(
                                        &path,
                                        n,
                                        "golden-needs",
                                        format!(
                                            "needs: {:?} — the only value is `meaning`",
                                            n.value
                                        ),
                                    );
                                    false
                                }
                            };
                            let within = q.within.as_ref().map_or(5, |w| w.value);
                            if let Some(w) = &q.within
                                && !(1..=25).contains(&w.value)
                            {
                                self.err_at(
                                    &path,
                                    w,
                                    "golden-within",
                                    "within must be between 1 and 25",
                                );
                            }
                            repo.golden.push(GoldenQuestion {
                                query: q.query.value.clone(),
                                expect,
                                within,
                                needs_meaning,
                                loc: loc(&path, &q.query),
                            });
                        }
                    }
                }
                ["spaces.yaml"] => {
                    if let Some(s) = self.parse::<SpacesYaml>(text, &path, 0) {
                        let mut seen = BTreeSet::new();
                        for sp in s.spaces {
                            if !valid_id(&sp.id) {
                                self.err_at(&path, &sp.id, "id", id_rule("space id", &sp.id));
                                continue;
                            }
                            if RESERVED_SPACES.contains(&sp.id.as_str()) {
                                self.err_at(
                                    &path,
                                    &sp.id,
                                    "reserved-space",
                                    format!(
                                        "{:?} is reserved ({}); pick another id",
                                        sp.id.value,
                                        RESERVED_SPACES.join(", ")
                                    ),
                                );
                                continue;
                            }
                            if !seen.insert(sp.id.value.clone()) {
                                self.err_at(
                                    &path,
                                    &sp.id,
                                    "duplicate-id",
                                    format!("space {:?} is declared twice", sp.id.value),
                                );
                                continue;
                            }
                            self.nonempty(&path, &sp.title, "title");
                            let icon = sp.icon.as_ref().and_then(|i| self.icon(&path, i));
                            repo.space_decls.push(SpaceDecl {
                                id: sp.id.value.clone(),
                                title: sp.title.value.clone(),
                                summary: sp.summary.clone(),
                                icon,
                                order: sp.order.unwrap_or(0),
                                loc: loc(&path, &sp.id),
                            });
                        }
                    }
                }
                _ => {
                    let hint = if p.ends_with(".yml") {
                        " (YAML files end in .yaml)"
                    } else {
                        ""
                    };
                    self.warn(
                        &path,
                        0,
                        0,
                        "unknown-file",
                        format!("not part of the .firelight/ format, so it is ignored{hint}"),
                    );
                }
            }
        }

        self.local_refs(&repo);
        Some(repo)
    }

    fn parse<T: DeserializeOwned>(&mut self, text: &str, path: &str, off: u32) -> Option<T> {
        match yaml::parse::<T>(text, path, off) {
            Ok(v) => Some(v),
            Err(d) => {
                self.diags.push(d);
                None
            }
        }
    }

    fn nonempty(&mut self, path: &str, p: &P<String>, what: &str) {
        if p.trim().is_empty() {
            self.err_at(path, p, "empty", format!("{what} is empty"));
        }
    }

    fn summary(&mut self, path: &str, p: &P<String>) {
        self.nonempty(path, p, "summary");
        let n = p.chars().count();
        if n > SUMMARY_MAX {
            self.err_at(path, p, "summary-too-long", format!("summary has {n} characters; keep it to {SUMMARY_MAX}, because search results and list_entries show it"));
        }
    }

    fn icon(&mut self, path: &str, p: &P<String>) -> Option<String> {
        if is_icon(p) {
            Some(p.value.clone())
        } else {
            self.err_at(
                path,
                p,
                "icon",
                format!(
                    "{:?} is not a built-in icon: {}",
                    p.value,
                    crate::icons::ICONS.join(", ")
                ),
            );
            None
        }
    }

    /// A space id used (not declared): valid, and not a reserved one.
    fn space_ref(&mut self, path: &str, p: &P<String>) -> bool {
        if !valid_id(p) {
            self.err_at(path, p, "id", id_rule("space id", p));
            return false;
        }
        if p.value == SPACE_ALL || (p.value == SPACE_FIRELIGHT && self.home != SELF_REPO) {
            self.err_at(
                path,
                p,
                "reserved-space",
                format!(
                    "{:?} is reserved: `all` is generated, `firelight` holds Firelight's own docs",
                    p.value
                ),
            );
            return false;
        }
        true
    }

    fn reference(&mut self, path: &str, p: &P<String>) -> Option<Ref> {
        match Ref::parse(p, &self.home) {
            Ok(r) => Some(r),
            Err(m) => {
                self.err_at(path, p, "ref", m);
                None
            }
        }
    }

    fn url(&mut self, path: &str, p: &P<String>) -> bool {
        let lower = p.to_ascii_lowercase();
        let ok = (lower.starts_with("https://") || lower.starts_with("http://")) && lower.len() > 8
            || lower.starts_with("mailto:") && lower.len() > 7;
        if !ok || p.chars().any(|c| c.is_control() || c == ' ') {
            self.err_at(
                path,
                p,
                "link-scheme",
                format!(
                    "{:?} is not an allowed URL: use https:, http: or mailto:",
                    p.value
                ),
            );
            return false;
        }
        true
    }

    fn link(&mut self, path: &str, l: LinkYaml) -> Option<Link> {
        self.nonempty(path, &l.label, "label");
        if !self.url(path, &l.url) {
            return None;
        }
        let icon = l.icon.as_ref().and_then(|i| self.icon(path, i));
        let mut at = Vec::new();
        let targets = l.at.to_vec();
        if targets.is_empty() {
            self.err_at(path, &l.at, "at", "at needs at least one target");
        }
        for a in &targets {
            let ap = l.at.map(a.clone());
            if let Some(x) = self.at(path, &ap) {
                at.push((x, loc(path, &ap)));
            }
        }
        let entry = l
            .entry
            .as_ref()
            .and_then(|e| self.reference(path, e).map(|r| (r, loc(path, e))));
        Some(Link {
            id: l.id.value.clone(),
            label: l.label.value.clone(),
            url: l.url.value.clone(),
            note: l.note.clone(),
            icon,
            at,
            order: l.order.unwrap_or(0),
            entry,
            loc: loc(path, &l.id),
        })
    }

    fn at(&mut self, path: &str, a: &P<AtYaml>) -> Option<At> {
        let v = &a.value;
        let shape = "at is one of {place: top|footer, space?}, {island: <ref>} or {island: <ref>, card: <id>, section?}";
        match (&v.place, &v.island) {
            (Some(place), None) => {
                if v.card.is_some() || v.section.is_some() {
                    self.err_at(path, a, "at", shape);
                    return None;
                }
                let place_v = match place.as_str() {
                    "top" => Place::Top,
                    "footer" => Place::Footer,
                    _ => {
                        self.err_at(path, a, "at", format!("place {place:?}: links go in `top` or `footer` (use an island for main)"));
                        return None;
                    }
                };
                let space = match &v.space {
                    Some(s) => {
                        if !self.space_ref(path, &a.map(s.clone())) {
                            return None;
                        }
                        Some(s.clone())
                    }
                    None => None,
                };
                Some(At::Place {
                    place: place_v,
                    space,
                })
            }
            (None, Some(island)) => {
                if v.space.is_some() {
                    self.err_at(
                        path,
                        a,
                        "at",
                        "space goes with place only; an island already belongs to a space",
                    );
                    return None;
                }
                let r = self.reference(path, &a.map(island.clone()))?;
                match &v.card {
                    None => {
                        if v.section.is_some() {
                            self.err_at(path, a, "at", "section needs a card");
                            return None;
                        }
                        Some(At::Island { island: r })
                    }
                    Some(card) => {
                        if !valid_id(card) {
                            self.err_at(path, a, "id", id_rule("card id", card));
                            return None;
                        }
                        Some(At::Card {
                            island: r,
                            card: card.clone(),
                            section: v.section.clone(),
                        })
                    }
                }
            }
            _ => {
                self.err_at(path, a, "at", shape);
                None
            }
        }
    }

    fn cards(&mut self, path: &str, cards: &[CardYaml], seen: &mut BTreeSet<String>) -> Vec<Card> {
        let mut out = Vec::new();
        for c in cards {
            if !valid_id(&c.id) {
                self.err_at(path, &c.id, "id", id_rule("card id", &c.id));
                continue;
            }
            if !seen.insert(c.id.value.clone()) {
                self.err_at(
                    path,
                    &c.id,
                    "duplicate-id",
                    format!("card {:?} appears twice in this island", c.id.value),
                );
                continue;
            }
            self.nonempty(path, &c.title, "title");
            let tone = match c.tone.as_ref().map(|t| t.as_str()) {
                None | Some("none") => Tone::None,
                Some("danger") => Tone::Danger,
                Some("warn") => Tone::Warn,
                Some(other) => {
                    if let Some(t) = &c.tone {
                        self.err_at(
                            path,
                            t,
                            "tone",
                            format!("tone {other:?}: use danger, warn or none"),
                        );
                    }
                    Tone::None
                }
            };
            out.push(Card {
                id: c.id.value.clone(),
                title: c.title.value.clone(),
                order: c.order.unwrap_or(0),
                tone,
                loc: loc(path, &c.id),
            });
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn island_common(
        &mut self,
        path: &str,
        id: &Option<P<String>>,
        file_id: &str,
        space: &Option<P<String>>,
        place: &Option<P<String>>,
        title: &P<String>,
        default_space: &str,
    ) -> (P<String>, String, Place) {
        let idp = id
            .clone()
            .unwrap_or_else(|| P::new(file_id.to_string(), 1, 1));
        self.nonempty(path, title, "title");
        let space = match space {
            Some(s) if self.space_ref(path, s) => s.value.clone(),
            _ => default_space.to_string(),
        };
        let place = match place {
            None => Place::Main,
            Some(p) => Place::parse(p).unwrap_or_else(|| {
                self.err_at(
                    path,
                    p,
                    "place",
                    format!("place {:?}: use main, top or footer", p.value),
                );
                Place::Main
            }),
        };
        (idp, space, place)
    }

    fn island(
        &mut self,
        path: &str,
        text: &str,
        file_id: &str,
        default_space: &str,
        out: &mut IslandsOut<'_>,
        claim: &mut ClaimFn<'_>,
    ) {
        let Some(v) = self.parse::<VersionOnly>(text, path, 0) else {
            return;
        };
        let Some(kind) = v.kind else {
            self.err(
                path,
                1,
                1,
                "island-type",
                "missing `type`: cards, links, template or cards-contribution",
            );
            return;
        };
        match kind.as_str() {
            "cards" => {
                let Some(y) = self.parse::<CardsIslandYaml>(text, path, 0) else {
                    return;
                };
                let (idp, space, place) = self.island_common(
                    path,
                    &y.id,
                    file_id,
                    &y.space,
                    &y.place,
                    &y.title,
                    default_space,
                );
                if !claim(self, &idp, path, "island") {
                    return;
                }
                let mut seen_sections = BTreeSet::new();
                let mut sections = Vec::new();
                for s in &y.sections {
                    if s.trim().is_empty() || !seen_sections.insert(s.value.clone()) {
                        self.err_at(
                            path,
                            s,
                            "section",
                            format!("section {:?} is empty or repeated", s.value),
                        );
                        continue;
                    }
                    sections.push(s.value.clone());
                }
                let cards = self.cards(path, &y.cards, &mut BTreeSet::new());
                out.islands.push(Island {
                    id: idp.value.clone(),
                    title: y.title.value.clone(),
                    space,
                    place,
                    order: y.order.unwrap_or(0),
                    kind: IslandKind::Cards { sections, cards },
                    loc: loc(path, &idp),
                });
            }
            "links" => {
                let Some(y) = self.parse::<LinksIslandYaml>(text, path, 0) else {
                    return;
                };
                let (idp, space, place) = self.island_common(
                    path,
                    &y.id,
                    file_id,
                    &y.space,
                    &y.place,
                    &y.title,
                    default_space,
                );
                if !claim(self, &idp, path, "island") {
                    return;
                }
                out.islands.push(Island {
                    id: idp.value.clone(),
                    title: y.title.value.clone(),
                    space,
                    place,
                    order: y.order.unwrap_or(0),
                    kind: IslandKind::Links,
                    loc: loc(path, &idp),
                });
            }
            "template" => {
                let Some(y) = self.parse::<TemplateIslandYaml>(text, path, 0) else {
                    return;
                };
                let (idp, space, place) = self.island_common(
                    path,
                    &y.id,
                    file_id,
                    &y.space,
                    &y.place,
                    &y.title,
                    default_space,
                );
                if !claim(self, &idp, path, "island") {
                    return;
                }
                let mut inputs = Vec::new();
                let mut names = BTreeSet::new();
                for i in &y.inputs {
                    if !template::valid_name(&i.name) {
                        self.err_at(path, &i.name, "template-input", format!("{:?} is not an input name: lowercase letters, digits and _, starting with a letter", i.name.value));
                        continue;
                    }
                    if !names.insert(i.name.value.clone()) {
                        self.err_at(
                            path,
                            &i.name,
                            "duplicate-id",
                            format!("input {:?} appears twice", i.name.value),
                        );
                        continue;
                    }
                    if let Err(m) = template::check_pattern(&i.pattern) {
                        self.err_at(path, &i.pattern, "template-pattern", m);
                        continue;
                    }
                    inputs.push(TemplateInput {
                        name: i.name.value.clone(),
                        label: i.label.value.clone(),
                        pattern: i.pattern.value.clone(),
                        placeholder: i.placeholder.clone(),
                    });
                }
                if y.inputs.is_empty() {
                    self.err_at(
                        path,
                        &y.title,
                        "template-input",
                        "a template island needs at least one input",
                    );
                }
                let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
                let mut used = BTreeSet::new();
                let mut links = Vec::new();
                for l in &y.links {
                    self.nonempty(path, &l.label, "label");
                    match template::check_url(&l.url, &name_refs) {
                        Ok(u) => {
                            used.extend(u.names().into_iter().map(String::from));
                            let icon = l.icon.as_ref().and_then(|i| self.icon(path, i));
                            links.push(TemplateLink {
                                label: l.label.value.clone(),
                                url: l.url.value.clone(),
                                icon,
                            });
                        }
                        Err(m) => self.err_at(path, &l.url, "template-url", m),
                    }
                }
                for n in names.difference(&used) {
                    self.warn_at(
                        path,
                        &y.title,
                        "template-unused",
                        format!("input {n:?} is used by no link"),
                    );
                }
                out.islands.push(Island {
                    id: idp.value.clone(),
                    title: y.title.value.clone(),
                    space,
                    place,
                    order: y.order.unwrap_or(0),
                    kind: IslandKind::Template(Template {
                        inputs,
                        links,
                        help: y.help.clone(),
                    }),
                    loc: loc(path, &idp),
                });
            }
            "cards-contribution" => {
                let Some(y) = self.parse::<CardsContributionYaml>(text, path, 0) else {
                    return;
                };
                let Some(r) = self.reference(path, &y.island) else {
                    return;
                };
                let cards = self.cards(path, &y.cards, &mut BTreeSet::new());
                out.contributions.push(CardsContribution {
                    island: r,
                    cards,
                    loc: loc(path, &y.island),
                });
            }
            other => {
                self.warn_at(path, &kind, "island-type-unknown", format!("island type {other:?} is not one this Firelight knows (cards, links, template, cards-contribution); the island is skipped, the rest of the repo works"));
            }
        }
    }

    fn entry(&mut self, path: &str, text: &str, file_id: &str, kind: EntryKind) -> Option<Entry> {
        let Some((yaml_text, body, body_line)) = split_front_matter(text) else {
            self.err(path, 1, 1, "front-matter", "an entry starts with front matter: a `---` line, `format: 1`, title, summary, then `---`");
            return None;
        };
        let y = self.parse::<EntryYaml>(yaml_text, path, 1)?;
        let shift = |p: &P<String>| {
            let mut p = p.clone();
            p.shift(1);
            p
        };
        let title = shift(&y.title);
        self.nonempty(path, &title, "title");
        self.summary(path, &shift(&y.summary));
        let idp =
            y.id.as_ref()
                .map(shift)
                .unwrap_or_else(|| P::new(file_id.to_string(), 1, 1));
        let refs = |c: &mut Checker, v: &[P<String>]| -> Vec<(Ref, Loc)> {
            v.iter()
                .map(shift)
                .filter_map(|p| c.reference(path, &p).map(|r| (r, loc(path, &p))))
                .collect()
        };
        let links = refs(self, &y.links);
        let related = refs(self, &y.related);
        let mut covers = Vec::new();
        for c in y.covers.iter().map(shift) {
            let bad = c.is_empty()
                || c.starts_with('/')
                || c.contains('\\')
                || c.split('/').any(|s| s == ".." || s == ".");
            if bad {
                self.err_at(path, &c, "covers", format!("{:?}: covers are repo-relative paths or globs like terraform/access/**, without .. or a leading /", c.value));
            } else {
                covers.push(c.value.clone());
            }
        }
        let spaces = y.spaces.as_ref().map(|v| {
            v.iter()
                .map(shift)
                .filter(|s| self.space_ref(path, s))
                .map(|s| s.value.clone())
                .collect::<Vec<_>>()
        });
        let (md, d) = scan(body, &self.home, path, body_line);
        self.diags.extend(d);
        Some(Entry {
            id: idp.value.clone(),
            kind,
            title: title.value.clone(),
            summary: y.summary.value.clone(),
            aliases: y.aliases.iter().map(|a| a.value.clone()).collect(),
            tags: y.tags.iter().map(|a| a.value.clone()).collect(),
            links,
            related,
            covers,
            spaces,
            body: md,
            path: path.to_string(),
            loc: loc(path, &idp),
        })
    }

    /// Every same-repo ref must resolve (§5): unresolved is an error here;
    /// cross-repo refs are the catalog's job.
    fn local_refs(&mut self, repo: &Repo) {
        let home = repo.id.clone();
        let mut errs = Vec::new();
        let mut wrong_kind = Vec::new();
        let entry_exists = |id: &str| repo.entry(id).is_some();
        let link_exists = |id: &str| repo.link(id).is_some();
        let mut check = |r: &Ref, l: &Loc, ok: bool, what: &str| {
            if r.repo == home && !ok {
                errs.push(Diagnostic::error(
                    &l.path,
                    l.line,
                    l.col,
                    "unresolved-ref",
                    format!("{what} {:?} does not exist in this repo", r.id),
                ));
            }
        };
        for l in &repo.links {
            if let Some((r, lc)) = &l.entry {
                check(r, lc, entry_exists(&r.id), "entry");
            }
        }
        for e in &repo.entries {
            for (r, lc) in &e.links {
                check(r, lc, link_exists(&r.id), "link");
            }
            for (r, lc) in &e.related {
                check(r, lc, entry_exists(&r.id), "entry");
            }
            for b in &e.body.refs {
                let ok = match &b.kind {
                    BodyRefKind::Link => link_exists(&b.target.id),
                    BodyRefKind::Entry { .. } => entry_exists(&b.target.id),
                };
                check(
                    &b.target,
                    &b.loc,
                    ok,
                    if matches!(b.kind, BodyRefKind::Link) {
                        "link"
                    } else {
                        "entry"
                    },
                );
            }
        }
        if let Some(a) = &repo.about {
            for b in &a.refs {
                let ok = match &b.kind {
                    BodyRefKind::Link => link_exists(&b.target.id),
                    BodyRefKind::Entry { .. } => entry_exists(&b.target.id),
                };
                check(
                    &b.target,
                    &b.loc,
                    ok,
                    if matches!(b.kind, BodyRefKind::Link) {
                        "link"
                    } else {
                        "entry"
                    },
                );
            }
        }
        for q in &repo.golden {
            for (r, lc) in &q.expect {
                check(
                    r,
                    lc,
                    entry_exists(&r.id) || link_exists(&r.id),
                    "entry or link",
                );
            }
        }
        for c in &repo.contributions {
            if c.island.repo == home {
                match repo.island(&c.island.id).map(|i| &i.kind) {
                    Some(IslandKind::Cards { .. }) => {}
                    Some(k) => wrong_kind.push(Diagnostic::error(
                        &c.loc.path,
                        c.loc.line,
                        c.loc.col,
                        "contribution-target",
                        format!(
                            "{:?} is a {} island; cards can only be added to a cards island",
                            c.island.id,
                            k.type_name()
                        ),
                    )),
                    None => check(&c.island, &c.loc, false, "cards island"),
                }
            }
        }
        errs.append(&mut wrong_kind);
        // "What is this?" points at a concept (§5.3).
        for l in &repo.links {
            if let Some((r, lc)) = &l.entry
                && r.repo == home
                && repo
                    .entry(&r.id)
                    .is_some_and(|e| e.kind == EntryKind::Guide)
            {
                errs.push(Diagnostic::warning(&lc.path, lc.line, lc.col, "entry-not-concept", format!("{:?} is a guide; a link's `entry` is the concept that says what the link is", r.id)));
            }
        }
        // Heading anchors on local entry refs.
        for e in repo
            .entries
            .iter()
            .map(|e| &e.body)
            .chain(repo.about.iter())
        {
            for b in &e.refs {
                if let BodyRefKind::Entry { heading: Some(h) } = &b.kind
                    && b.target.repo == home
                    && let Some(t) = repo.entry(&b.target.id)
                    && !t.body.headings.iter().any(|x| &x.slug == h)
                {
                    errs.push(Diagnostic::warning(
                        &b.loc.path,
                        b.loc.line,
                        b.loc.col,
                        "unresolved-heading",
                        format!("entry {:?} has no heading #{h}", b.target.id),
                    ));
                }
            }
        }
        self.diags.extend(errs);
    }
}

/// The files the format defines (others are ignored, with a warning).
fn is_format_file(p: &str) -> bool {
    let parts: Vec<&str> = p.split('/').collect();
    match parts.as_slice() {
        ["repo.yaml" | "links.yaml" | "golden.yaml" | "spaces.yaml" | "about.md"] => true,
        ["islands", f] => f.ends_with(".yaml"),
        ["concepts" | "guides", f] => f.ends_with(".md"),
        _ => false,
    }
}

/// Registers an id as used; false (with a diagnostic) when it is taken or invalid.
type ClaimFn<'a> = dyn FnMut(&mut Checker, &P<String>, &str, &str) -> bool + 'a;

/// Just the id of a repo.yaml that doesn't parse.
#[derive(serde::Deserialize)]
struct LooseRepo {
    id: Option<P<String>>,
}

/// Where parsed islands go (split out so `run` can borrow the repo while
/// islands are added).
struct IslandsOut<'a> {
    islands: &'a mut Vec<Island>,
    contributions: &'a mut Vec<CardsContribution>,
}

#[cfg(test)]
mod tests;
