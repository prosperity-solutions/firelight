//! What the page, API and MCP routes read: the catalog and per-source facts.
//! The binary implements [`Content`] for its hub; tests use a fixed catalog.

use std::sync::{Arc, OnceLock};

use firelight_format::catalog::Catalog;
use firelight_search::{Filter, Hit, Index};

/// The commit a source serves.
#[derive(Debug, Clone, Default)]
pub struct SourceInfo {
    pub commit: String,
    pub committed_at: Option<String>,
    /// A browsable URL of the repo (for the "Source" line), if known.
    pub web_url: Option<String>,
}

/// "Maybe stale" (§9.5): the covered paths changed after the entry did.
#[derive(Debug, Clone)]
pub struct Staleness {
    pub entry_changed: String,
    pub covers_changed: String,
}

/// The search side of the status page (§16.1).
#[derive(Debug, Clone, Default)]
pub struct SearchInfo {
    /// `candle`, `openai-compatible` or `off`.
    pub backend: String,
    pub model: Option<String>,
    pub dims: Option<usize>,
    /// Chunks with a vector, and all chunks.
    pub embedded: usize,
    pub chunks: usize,
    pub last_index_ms: Option<u64>,
    /// Why the embedder isn't running yet, if it isn't.
    pub problem: Option<String>,
    /// The built-in golden check: `pending`, `pass`, `fail` or `skipped`.
    pub builtin: String,
    /// One line per golden question that didn't pass (built-in first), with
    /// its repo; built-in failures keep `/readyz` unready.
    pub golden_failures: Vec<(String, String)>,
    /// One line per golden question with expects in repos that aren't
    /// loaded (skipped, not failed), with its repo.
    pub golden_not_loaded: Vec<(String, String)>,
}

pub trait Content: Send + Sync {
    fn catalog(&self) -> Arc<Catalog>;
    /// Bumped whenever the catalog changes (preview's live reload).
    fn generation(&self) -> u64;
    fn source(&self, repo: &str) -> Option<SourceInfo>;
    /// `entry_path` is relative to the repo root (`.firelight/guides/x.md`).
    fn staleness(&self, repo: &str, entry_path: &str) -> Option<Staleness>;
    /// Search the served catalog (§9). Blocking: it may embed the query, so
    /// async callers use `spawn_blocking`.
    fn search(&self, query: &str, filter: &Filter, limit: usize) -> Vec<Hit>;
    /// The status (§16.1) as JSON: what `/api/v1/status` and MCP `status`
    /// return. Secrets never appear in it.
    fn status(&self) -> serde_json::Value {
        serde_json::json!({ "sources": [] })
    }
    fn search_info(&self) -> SearchInfo {
        SearchInfo {
            backend: "off".into(),
            builtin: "skipped".into(),
            ..SearchInfo::default()
        }
    }
}

/// A fixed catalog with word search, for tests and tools.
pub struct Fixed(pub Arc<Catalog>, OnceLock<Option<Index>>);

impl Fixed {
    pub fn new(cat: Arc<Catalog>) -> Fixed {
        Fixed(cat, OnceLock::new())
    }
}

impl Content for Fixed {
    fn catalog(&self) -> Arc<Catalog> {
        self.0.clone()
    }
    fn generation(&self) -> u64 {
        1
    }
    fn source(&self, _repo: &str) -> Option<SourceInfo> {
        None
    }
    fn staleness(&self, _repo: &str, _entry_path: &str) -> Option<Staleness> {
        None
    }
    fn search(&self, query: &str, filter: &Filter, limit: usize) -> Vec<Hit> {
        self.1
            .get_or_init(|| Index::words_only(self.0.clone()).ok())
            .as_ref()
            .map(|i| i.search(query, filter, limit, None))
            .unwrap_or_default()
    }
}
