//! The index (§9.2): chunks, their FTS5 word index and (when an embedder is
//! on) their vectors, built for one catalog and swapped together with it.
//!
//! Ranking: a word list (bm25 over title 5, aliases 4, summary 2, body 1; the
//! words ORed, the last one also as a prefix) and a meaning list (cosine of the
//! query vector against every chunk vector), each grouped to its entry, link
//! or repo by the best chunk; reciprocal-rank fusion (k = 60); then an exact
//! title or alias match goes first. Filters apply before the limit.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use firelight_format::Ref;
use firelight_format::catalog::Catalog;
use firelight_store::fts::{FtsIndex, FtsRow, match_expr};

use crate::chunk::{Chunk, chunks};
use crate::{Hit, Kind, entry_in_space, links_in_space, repo_in_space, words};

/// Reciprocal-rank fusion constant (§9.2).
pub const RRF_K: f64 = 60.0;
/// How many chunks each list looks at before grouping.
const DEPTH: usize = 500;

/// What a search may return.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// A space id, or `all`; `None` = no space rule (golden questions, tools).
    pub space: Option<String>,
    /// Only these repos.
    pub repos: Option<BTreeSet<String>>,
    /// Only these kinds.
    pub kinds: Option<BTreeSet<Kind>>,
}

impl Filter {
    pub fn space(space: &str) -> Filter {
        Filter {
            space: Some(space.to_string()),
            ..Filter::default()
        }
    }
}

pub struct Index {
    pub cat: Arc<Catalog>,
    pub chunks: Vec<Chunk>,
    fts: FtsIndex,
    /// Per chunk, its vector under [`Index::embedder`]; filled in place while
    /// the embedder works through the chunks, so search improves as it goes.
    vecs: RwLock<Vec<Option<Arc<[f32]>>>>,
    /// The embedder whose vectors this index takes; `None`: words only.
    pub embedder: Option<String>,
}

impl Index {
    /// A word index over `cat`; chunks are cut under `max_tokens` as counted
    /// by `count`.
    pub fn build(
        cat: Arc<Catalog>,
        embedder: Option<String>,
        max_tokens: usize,
        count: &dyn Fn(&str) -> usize,
    ) -> Result<Index, String> {
        let chunks = chunks(&cat, max_tokens, count);
        let rows: Vec<FtsRow<'_>> = chunks
            .iter()
            .map(|c| FtsRow {
                title: &c.title,
                aliases: &c.aliases,
                summary: &c.summary,
                body: &c.body,
            })
            .collect();
        let fts = FtsIndex::build(&rows).map_err(|e| e.to_string())?;
        let n = chunks.len();
        Ok(Index {
            cat,
            chunks,
            fts,
            vecs: RwLock::new(vec![None; n]),
            embedder,
        })
    }

    /// A word-only index with the default chunk size (tests, tools).
    pub fn words_only(cat: Arc<Catalog>) -> Result<Index, String> {
        Index::build(cat, None, 512, &approx_tokens)
    }

    /// Attach vectors by chunk hash (from the cache or fresh). Only for an
    /// index built for an embedder.
    pub fn set_vectors(&self, by_hash: &HashMap<[u8; 32], Vec<f32>>) {
        if self.embedder.is_none() {
            return;
        }
        let Ok(mut vecs) = self.vecs.write() else {
            return;
        };
        for (c, v) in self.chunks.iter().zip(vecs.iter_mut()) {
            if v.is_none()
                && let Some(x) = by_hash.get(&c.hash)
            {
                *v = Some(Arc::from(x.as_slice()));
            }
        }
    }

    /// Chunks with no vector yet (hash, text), deduplicated.
    pub fn missing(&self) -> Vec<([u8; 32], String)> {
        if self.embedder.is_none() {
            return Vec::new();
        }
        let vecs = self.vecs.read().map(|v| v.clone()).unwrap_or_default();
        let mut seen = BTreeSet::new();
        self.chunks
            .iter()
            .zip(&vecs)
            .filter(|(c, v)| v.is_none() && seen.insert(c.hash))
            .map(|(c, _)| (c.hash, c.text.clone()))
            .collect()
    }

    pub fn hashes(&self) -> Vec<[u8; 32]> {
        self.chunks.iter().map(|c| c.hash).collect()
    }

    /// (chunks with a vector, all chunks).
    pub fn coverage(&self) -> (usize, usize) {
        let with = self
            .vecs
            .read()
            .map(|v| v.iter().filter(|x| x.is_some()).count())
            .unwrap_or(0);
        (with, self.chunks.len())
    }

    fn visible(&self, f: &Filter, c: &Chunk, links: &Option<BTreeSet<Ref>>) -> bool {
        if f.kinds.as_ref().is_some_and(|k| !k.contains(&c.kind)) {
            return false;
        }
        if f.repos.as_ref().is_some_and(|r| !r.contains(&c.owner.repo)) {
            return false;
        }
        let Some(space) = &f.space else { return true };
        match c.kind {
            Kind::Concept | Kind::Guide => entry_in_space(&self.cat, space, &c.owner),
            Kind::Link => links.as_ref().is_some_and(|l| l.contains(&c.owner)),
            Kind::Repo => repo_in_space(&self.cat, space, &c.owner.repo),
        }
    }

    /// Search. `qvec` is the query's vector under [`Index::embedder`], or
    /// `None` for words only.
    pub fn search(&self, query: &str, f: &Filter, limit: usize, qvec: Option<&[f32]>) -> Vec<Hit> {
        let q = words(query);
        if q.is_empty() || limit == 0 {
            return Vec::new();
        }
        let links: Option<BTreeSet<Ref>> = f
            .space
            .as_ref()
            .map(|s| links_in_space(&self.cat, s).into_iter().collect());
        // Owner → (kind, best chunk) per list, in rank order.
        let mut word_rank: Vec<(Kind, Ref, usize)> = Vec::new();
        let mut seen = BTreeSet::new();
        for (row, _) in self.fts.query(&match_expr(&q), DEPTH).unwrap_or_default() {
            let Some(c) = self.chunks.get(row) else {
                continue;
            };
            if self.visible(f, c, &links) && seen.insert((c.kind, c.owner.clone())) {
                word_rank.push((c.kind, c.owner.clone(), row));
            }
        }
        let mut mean_rank: Vec<(Kind, Ref, usize)> = Vec::new();
        if let Some(qv) = qvec {
            let vecs = self.vecs.read().map(|v| v.clone()).unwrap_or_default();
            let mut scored: Vec<(f32, usize)> = vecs
                .iter()
                .enumerate()
                .filter_map(|(i, v)| v.as_ref().map(|v| (firelight_dot(qv, v), i)))
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            let mut seen = BTreeSet::new();
            for (_, i) in scored.into_iter().take(DEPTH * 4) {
                let c = &self.chunks[i];
                if self.visible(f, c, &links) && seen.insert((c.kind, c.owner.clone())) {
                    mean_rank.push((c.kind, c.owner.clone(), i));
                }
            }
        }
        // Fusion.
        let mut fused: HashMap<(Kind, Ref), (f64, usize)> = HashMap::new();
        for list in [&word_rank, &mean_rank] {
            for (rank, (k, r, chunk)) in list.iter().enumerate() {
                let e = fused.entry((*k, r.clone())).or_insert((0.0, *chunk));
                e.0 += 1.0 / (RRF_K + (rank + 1) as f64);
            }
        }
        let qnorm = q.join(" ");
        let mut hits: Vec<(bool, Hit)> = fused
            .into_iter()
            .map(|((kind, r), (score, chunk))| {
                let c = &self.chunks[chunk];
                let exact = self.exact(kind, &r, &qnorm);
                let (title, summary, url) = self.describe(kind, &r);
                (
                    exact,
                    Hit {
                        kind,
                        r,
                        title,
                        summary,
                        heading: c.heading.clone(),
                        score,
                        url,
                    },
                )
            })
            .collect();
        hits.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.score.total_cmp(&a.1.score))
                .then_with(|| a.1.title.cmp(&b.1.title))
                .then_with(|| a.1.r.cmp(&b.1.r))
        });
        hits.into_iter().take(limit).map(|(_, h)| h).collect()
    }

    fn exact(&self, kind: Kind, r: &Ref, qnorm: &str) -> bool {
        let same = |s: &str| words(s).join(" ") == qnorm;
        match kind {
            Kind::Concept | Kind::Guide => self
                .cat
                .entry(r)
                .is_some_and(|e| same(&e.title) || e.aliases.iter().any(|a| same(a))),
            Kind::Link => self.cat.link(r).is_some_and(|l| same(&l.label)),
            Kind::Repo => self
                .cat
                .repo(&r.repo)
                .is_some_and(|x| same(&x.name) || same(&x.id)),
        }
    }

    fn describe(&self, kind: Kind, r: &Ref) -> (String, String, Option<String>) {
        match kind {
            Kind::Concept | Kind::Guide => self
                .cat
                .entry(r)
                .map(|e| (e.title.clone(), e.summary.clone(), None))
                .unwrap_or_default(),
            Kind::Link => self
                .cat
                .link(r)
                .map(|l| {
                    (
                        l.label.clone(),
                        l.note.clone().unwrap_or_default(),
                        Some(l.url.clone()),
                    )
                })
                .unwrap_or_default(),
            Kind::Repo => self
                .cat
                .repo(&r.repo)
                .map(|x| (x.name.clone(), x.summary.clone(), None))
                .unwrap_or_default(),
        }
    }
}

fn firelight_dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Words × 1.5: the token estimate without a tokenizer.
pub fn approx_tokens(text: &str) -> usize {
    let w = text.split_whitespace().count();
    w + w / 2
}
