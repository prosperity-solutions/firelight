//! The hub: every configured source, what each serves, and the catalog built
//! from them (§3, §6, §7).
//!
//! Startup serves the cached snapshots at once (a warm restart is ready before
//! the first pull finishes); then every source is polled. A pull that passes
//! replaces the served snapshot; one that fails keeps the last good one and
//! records why. Webhooks only make a poll happen sooner.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use firelight_format::catalog::{self, Catalog, Strictness};
use firelight_format::{Diagnostic, InputFile, Limits, Repo, parse_repo};
use firelight_source::pull::{Pulled, Puller, SourceSpec};
use firelight_store::{SourceState, Store};
use tokio::sync::mpsc;

include!(concat!(env!("OUT_DIR"), "/self_docs.rs"));

/// The embedded self-docs as input files.
pub fn self_docs_files() -> Vec<InputFile> {
    SELF_DOCS
        .iter()
        .map(|(p, b)| InputFile::regular(*p, b.to_vec()))
        .collect()
}

/// What one source serves now.
#[derive(Debug, Clone)]
#[allow(dead_code, reason = "read by the status, API and page routes (M4, M6)")]
pub struct Served {
    pub repo: Repo,
    pub commit: String,
    pub committed_at: Option<String>,
    /// Warnings of the served commit.
    pub diagnostics: Vec<Diagnostic>,
}

pub struct Hub {
    specs: Vec<SourceSpec>,
    puller: Puller,
    store: Mutex<Store>,
    limits: Limits,
    self_docs: bool,
    served: RwLock<BTreeMap<String, Served>>,
    catalog: RwLock<Arc<Catalog>>,
    /// Bumped on every catalog rebuild.
    generation: std::sync::atomic::AtomicU64,
}

impl Hub {
    pub fn new(
        specs: Vec<SourceSpec>,
        puller: Puller,
        store: Store,
        limits: Limits,
        self_docs: bool,
    ) -> Arc<Hub> {
        Arc::new(Hub {
            specs,
            puller,
            store: Mutex::new(store),
            limits,
            self_docs,
            served: RwLock::new(BTreeMap::new()),
            catalog: RwLock::new(Arc::new(Catalog::default())),
            generation: std::sync::atomic::AtomicU64::new(0),
        })
    }

    #[allow(dead_code, reason = "read by the page, API and MCP routes (M4, M6)")]
    pub fn catalog(&self) -> Arc<Catalog> {
        self.catalog.read().map(|c| c.clone()).unwrap_or_default()
    }

    #[allow(dead_code, reason = "read by preview's live reload (M4)")]
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn source_ids(&self) -> Vec<String> {
        self.specs.iter().map(|s| s.id.clone()).collect()
    }

    pub fn source_states(&self) -> Vec<SourceState> {
        self.store
            .lock()
            .ok()
            .and_then(|s| s.source_states().ok())
            .unwrap_or_default()
    }

    /// Load cached snapshots. Returns true when every source had one, so the
    /// hub can be ready before the first pull (§3).
    pub fn load_cache(&self) -> bool {
        let ids = self.source_ids();
        let mut all = true;
        let mut served = BTreeMap::new();
        if let Ok(mut store) = self.store.lock() {
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            if let Err(e) = store.retain_sources(&refs) {
                tracing::warn!(error = %e, "cannot drop sources no longer configured from the cache");
            }
            for id in &ids {
                match store.load_snapshot(id) {
                    Ok(Some(snap)) => {
                        let (repo, diags) = parse_repo(&snap.files, Some(id), &self.limits);
                        match repo {
                            Some(r) if !diags.iter().any(Diagnostic::is_error) => {
                                served.insert(
                                    id.clone(),
                                    Served {
                                        repo: r,
                                        commit: snap.commit,
                                        committed_at: snap.committed_at,
                                        diagnostics: diags,
                                    },
                                );
                            }
                            // Cached by an older binary with other rules: pull it again.
                            _ => all = false,
                        }
                    }
                    Ok(None) => all = false,
                    Err(e) => {
                        tracing::warn!(source = %id, error = %e, "cannot read the cached snapshot; pulling it again");
                        all = false;
                    }
                }
            }
        }
        if let Ok(mut s) = self.served.write() {
            *s = served;
        }
        self.rebuild();
        all
    }

    /// Rebuild the catalog from what every source serves (and the self-docs).
    pub fn rebuild(&self) {
        let served = self.served.read().map(|s| s.clone()).unwrap_or_default();
        let mut repos = Vec::new();
        // The self-docs first, unless a dir source declares `firelight` (§5.4).
        if self.self_docs && !served.contains_key(firelight_format::SELF_REPO) {
            let (repo, diags) = parse_repo(
                &self_docs_files(),
                Some(firelight_format::SELF_REPO),
                &self.limits,
            );
            match repo {
                Some(r) if !diags.iter().any(Diagnostic::is_error) => repos.push(r),
                _ => tracing::error!(
                    ?diags,
                    "the embedded self-docs don't pass their own checks; a build bug"
                ),
            }
        }
        // Source order decides space conflicts (§5.7).
        for spec in &self.specs {
            if let Some(s) = served.get(&spec.id) {
                repos.push(s.repo.clone());
            }
        }
        let cat = catalog::build(repos, Strictness::Lenient);
        if let Ok(mut c) = self.catalog.write() {
            *c = Arc::new(cat);
        }
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Pull one source now; returns whether the catalog changed.
    pub async fn pull(&self, id: &str) -> bool {
        let Some(spec) = self.specs.iter().find(|s| s.id == id) else {
            return false;
        };
        let last_seen = self
            .source_states()
            .into_iter()
            .find(|s| s.source_id == id)
            .and_then(|s| s.newest_commit);
        // Nothing served yet (a cold start or a lost cache): read fully.
        let have = self
            .served
            .read()
            .map(|s| s.contains_key(id))
            .unwrap_or(false);
        let last_seen = if have { last_seen } else { None };
        let started = Instant::now();
        let result = self.puller.pull(spec, last_seen.as_deref()).await;
        let ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let now = firelight_source::format_rfc3339(self.puller.clock.now_unix());
        let mut state = SourceState {
            source_id: id.to_string(),
            last_pull_at: Some(now),
            last_pull_ms: Some(ms),
            newest_commit: last_seen.clone(),
            ..Default::default()
        };
        let mut changed = false;
        match result {
            Ok(Pulled::Unchanged) => {
                // Keep the newest commit's diagnostics as they were.
                state.newest_diagnostics = self
                    .source_states()
                    .into_iter()
                    .find(|s| s.source_id == id)
                    .and_then(|s| s.newest_diagnostics);
            }
            Ok(Pulled::Good {
                snapshot,
                repo,
                diagnostics,
            }) => {
                tracing::info!(source = %id, commit = %snapshot.commit, ms, "pulled a new commit");
                state.newest_commit = Some(snapshot.commit.clone());
                if let Ok(mut store) = self.store.lock()
                    && let Err(e) = store.save_snapshot(id, &snapshot)
                {
                    tracing::warn!(source = %id, error = %e, "cannot cache the snapshot; serving it anyway");
                }
                if let Ok(mut s) = self.served.write() {
                    s.insert(
                        id.to_string(),
                        Served {
                            repo: *repo,
                            commit: snapshot.commit,
                            committed_at: snapshot.committed_at,
                            diagnostics,
                        },
                    );
                }
                changed = true;
            }
            Ok(Pulled::Bad {
                commit,
                diagnostics,
            }) => {
                let errors = diagnostics.iter().filter(|d| d.is_error()).count();
                tracing::warn!(source = %id, %commit, errors, "the newest commit fails validation; keeping the last good one");
                state.newest_commit = Some(commit);
                state.newest_diagnostics = Some(diagnostics_json(&diagnostics));
            }
            Err(e) => {
                tracing::warn!(source = %id, error = %e, ms, "pull failed");
                state.last_pull_error = Some(e.to_string());
            }
        }
        if let Ok(mut store) = self.store.lock()
            && let Err(e) = store.record_pull(&state)
        {
            tracing::warn!(source = %id, error = %e, "cannot record the pull");
        }
        if changed {
            self.rebuild();
        }
        changed
    }

    /// Poll every source every `interval`, and at once when `wake` names it.
    /// `on_first_round` runs after every source was tried once.
    pub async fn run(
        self: Arc<Self>,
        interval: Duration,
        mut wake: mpsc::Receiver<String>,
        on_first_round: impl FnOnce() + Send + 'static,
    ) {
        let mut first = Some(on_first_round);
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let pulls = self.specs.iter().map(|s| {
                        let me = self.clone();
                        let id = s.id.clone();
                        tokio::spawn(async move { me.pull(&id).await })
                    }).collect::<Vec<_>>();
                    for p in pulls {
                        let _ = p.await;
                    }
                    if let Some(f) = first.take() {
                        f();
                    }
                }
                Some(id) = wake.recv() => {
                    tracing::info!(source = %id, "webhook: pulling now");
                    self.pull(&id).await;
                }
            }
        }
    }
}

pub fn diagnostics_json(d: &[Diagnostic]) -> String {
    serde_json::to_string(
        &d.iter()
            .map(|x| serde_json::json!({ "path": x.path, "line": x.line, "col": x.col, "severity": x.severity.as_str(), "code": x.code, "message": x.message }))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests;
