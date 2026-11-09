//! Golden questions (§9.4): queries that must find their entries.
//!
//! An expected entry in a repo that isn't loaded (a cross-repo question run
//! in one repo's own CI) can't be found, so it is skipped, not failed. A
//! question whose every expect is in such repos is skipped as a whole; one
//! with some loaded expects is judged on those. Skips are always reported,
//! so a mistyped repo id stays visible.

use std::collections::BTreeSet;

use firelight_format::{GoldenQuestion, Ref};

use crate::index::{Filter, Index};

/// The deepest a golden question may look (`within` is 1–25).
pub const MAX_WITHIN: usize = 25;

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Pass,
    /// Where each expected ref ranked (1-based), `None` when not in the top 25.
    Fail {
        ranks: Vec<(Ref, Option<usize>)>,
    },
    /// Not run; see [`Skip`].
    Skipped(Skip),
}

/// Why a question didn't run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// `needs: meaning` with the embedder off.
    NeedsMeaning,
    /// Every expected entry is in a repo that isn't loaded.
    NotLoaded,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Result {
    /// The repo whose golden.yaml (or golden file) holds the question.
    pub repo: String,
    pub query: String,
    /// The expects that were judged (those in loaded repos).
    pub expect: Vec<Ref>,
    /// Expects in repos that aren't loaded: skipped, never failed.
    pub not_loaded: Vec<Ref>,
    pub within: usize,
    pub outcome: Outcome,
}

impl Result {
    pub fn failed(&self) -> bool {
        matches!(self.outcome, Outcome::Fail { .. })
    }
    pub fn skipped(&self) -> bool {
        matches!(self.outcome, Outcome::Skipped(_))
    }
}

/// The repos named by skipped expects, sorted and deduplicated: the ids a
/// report prints so a typo shows.
pub fn not_loaded_repos(results: &[Result]) -> Vec<String> {
    results
        .iter()
        .flat_map(|r| r.not_loaded.iter().map(|x| x.repo.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Run `questions` (each with its repo) against `index` under `filter`.
/// `qvec` embeds a query; `None` from it means words only, and `needs:
/// meaning` questions are then skipped. "Loaded" means in `index.cat`.
pub fn run(
    index: &Index,
    questions: &[(String, GoldenQuestion)],
    filter: &Filter,
    qvec: &dyn Fn(&str) -> Option<Vec<f32>>,
) -> Vec<Result> {
    let loaded: BTreeSet<&str> = index.cat.repos.iter().map(|r| r.id.as_str()).collect();
    questions
        .iter()
        .map(|(repo, q)| {
            let (expect, not_loaded): (Vec<Ref>, Vec<Ref>) = q
                .expect
                .iter()
                .map(|(r, _)| r.clone())
                .partition(|r| loaded.contains(r.repo.as_str()));
            let within = usize::try_from(q.within).unwrap_or(5).clamp(1, MAX_WITHIN);
            if expect.is_empty() && !not_loaded.is_empty() {
                // Don't embed a question that can't run.
                return Result {
                    repo: repo.clone(),
                    query: q.query.clone(),
                    expect,
                    not_loaded,
                    within,
                    outcome: Outcome::Skipped(Skip::NotLoaded),
                };
            }
            let v = qvec(&q.query);
            let outcome = if q.needs_meaning && v.is_none() {
                Outcome::Skipped(Skip::NeedsMeaning)
            } else {
                let hits = index.search(&q.query, filter, MAX_WITHIN, v.as_deref());
                let ranks: Vec<(Ref, Option<usize>)> = expect
                    .iter()
                    .map(|r| {
                        (
                            r.clone(),
                            hits.iter().position(|h| h.r == *r).map(|i| i + 1),
                        )
                    })
                    .collect();
                if ranks.iter().all(|(_, k)| k.is_some_and(|k| k <= within)) {
                    Outcome::Pass
                } else {
                    Outcome::Fail { ranks }
                }
            };
            Result {
                repo: repo.clone(),
                query: q.query.clone(),
                expect,
                not_loaded,
                within,
                outcome,
            }
        })
        .collect()
}

/// One line per result, for logs, `preview --check` and the status page.
pub fn describe(r: &Result) -> String {
    let base = match &r.outcome {
        Outcome::Pass => format!("{:?}: passes", r.query),
        Outcome::Skipped(Skip::NeedsMeaning) => format!(
            "{:?}: skipped (needs meaning; the embedder is off)",
            r.query
        ),
        Outcome::Skipped(Skip::NotLoaded) => {
            return format!(
                "{:?}: skipped ({})",
                r.query,
                not_loaded_note(&r.not_loaded)
            );
        }
        Outcome::Fail { ranks } => {
            let parts: Vec<String> = ranks
                .iter()
                .map(|(x, k)| match k {
                    Some(k) => format!("{x} ranked {k}"),
                    None => format!("{x} not in the top {MAX_WITHIN}"),
                })
                .collect();
            format!(
                "{:?} should find {} within {}: {}",
                r.query,
                r.expect
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                r.within,
                parts.join("; ")
            )
        }
    };
    if r.not_loaded.is_empty() {
        base
    } else {
        format!("{base} (skipped {})", not_loaded_note(&r.not_loaded))
    }
}

/// `app-repo/x: repo app-repo not loaded; infra-repo/y: …`
fn not_loaded_note(refs: &[Ref]) -> String {
    refs.iter()
        .map(|x| format!("{x}: repo {} not loaded", x.repo))
        .collect::<Vec<_>>()
        .join("; ")
}
