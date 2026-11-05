//! Golden questions (§9.4): queries that must find their entries.

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
    /// `needs: meaning` with the embedder off.
    Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Result {
    /// The repo whose golden.yaml (or golden file) holds the question.
    pub repo: String,
    pub query: String,
    pub expect: Vec<Ref>,
    pub within: usize,
    pub outcome: Outcome,
}

impl Result {
    pub fn failed(&self) -> bool {
        matches!(self.outcome, Outcome::Fail { .. })
    }
}

/// Run `questions` (each with its repo) against `index` under `filter`.
/// `qvec` embeds a query; `None` from it means words only, and `needs:
/// meaning` questions are then skipped.
pub fn run(
    index: &Index,
    questions: &[(String, GoldenQuestion)],
    filter: &Filter,
    qvec: &dyn Fn(&str) -> Option<Vec<f32>>,
) -> Vec<Result> {
    questions
        .iter()
        .map(|(repo, q)| {
            let expect: Vec<Ref> = q.expect.iter().map(|(r, _)| r.clone()).collect();
            let within = usize::try_from(q.within).unwrap_or(5).clamp(1, MAX_WITHIN);
            let v = qvec(&q.query);
            let outcome = if q.needs_meaning && v.is_none() {
                Outcome::Skipped
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
                within,
                outcome,
            }
        })
        .collect()
}

/// One line per failure, for logs and the status page.
pub fn describe(r: &Result) -> String {
    match &r.outcome {
        Outcome::Pass => format!("{:?}: passes", r.query),
        Outcome::Skipped => format!(
            "{:?}: skipped (needs meaning; the embedder is off)",
            r.query
        ),
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
    }
}
