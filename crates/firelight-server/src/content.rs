//! What the page, API and MCP routes read: the catalog and per-source facts.
//! The binary implements [`Content`] for its hub; tests use a fixed catalog.

use std::sync::Arc;

use firelight_format::catalog::Catalog;

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

pub trait Content: Send + Sync {
    fn catalog(&self) -> Arc<Catalog>;
    /// Bumped whenever the catalog changes (preview's live reload).
    fn generation(&self) -> u64;
    fn source(&self, repo: &str) -> Option<SourceInfo>;
    /// `entry_path` is relative to the repo root (`.firelight/guides/x.md`).
    fn staleness(&self, repo: &str, entry_path: &str) -> Option<Staleness>;
}

/// A fixed catalog, for tests and tools.
pub struct Fixed(pub Arc<Catalog>);

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
}
