//! The hub: every configured source, what each serves, and the catalog built
//! from them (§3, §6, §7).
//!
//! Startup serves the cached snapshots at once (a warm restart is ready before
//! the first pull finishes); then every source is polled. A pull that passes
//! replaces the served snapshot; one that fails keeps the last good one and
//! records why. Webhooks only make a poll happen sooner.
//!
//! Concurrency: one pull per source at a time (a per-source lock), and one
//! catalog rebuild at a time (read, build and swap under one lock), so a slow
//! rebuild can never overwrite a newer catalog.

use std::collections::{BTreeMap, BTreeSet};
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
    store: Arc<Mutex<Store>>,
    limits: Limits,
    self_docs: bool,
    served: RwLock<BTreeMap<String, Served>>,
    catalog: RwLock<Arc<Catalog>>,
    /// Held across a whole rebuild: read served, build, swap.
    rebuild_lock: Mutex<()>,
    /// One pull per source at a time.
    pull_locks: BTreeMap<String, Arc<tokio::sync::Mutex<()>>>,
    /// Sources this process has judged at least once. Until then a pull reads
    /// fully, so a commit rejected by an older binary or older limits is judged
    /// again after an upgrade.
    judged: Mutex<BTreeSet<String>>,
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
        let pull_locks = specs
            .iter()
            .map(|s| (s.id.clone(), Arc::new(tokio::sync::Mutex::new(()))))
            .collect();
        Arc::new(Hub {
            specs,
            puller,
            store: Arc::new(Mutex::new(store)),
            limits,
            self_docs,
            served: RwLock::new(BTreeMap::new()),
            catalog: RwLock::new(Arc::new(Catalog::default())),
            rebuild_lock: Mutex::new(()),
            pull_locks,
            judged: Mutex::new(BTreeSet::new()),
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

    /// Run a store operation off the async workers (rusqlite is blocking).
    async fn with_store<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> T + Send + 'static,
    ) -> Option<T> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.lock().ok().map(|mut s| f(&mut s)))
            .await
            .ok()
            .flatten()
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
                            // Cached under other rules (an upgrade): pull it again.
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
        let _one_at_a_time = self.rebuild_lock.lock();
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
        let Some(lock) = self.pull_locks.get(id) else {
            return false;
        };
        let _one_pull = lock.lock().await;

        let previous = self
            .source_states()
            .into_iter()
            .find(|s| s.source_id == id)
            .unwrap_or_default();
        // Skip unchanged remotes only for commits this process judged itself.
        let judged = self.judged.lock().map(|j| j.contains(id)).unwrap_or(false);
        let last_seen = if judged {
            previous.newest_commit.clone()
        } else {
            None
        };
        let started = Instant::now();
        let result = self.puller.pull(spec, last_seen.as_deref()).await;
        let ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let now = firelight_source::format_rfc3339(self.puller.clock.now_unix());
        // By default nothing about the newest commit changes.
        let mut state = SourceState {
            source_id: id.to_string(),
            last_pull_at: Some(now),
            last_pull_ms: Some(ms),
            newest_commit: previous.newest_commit.clone(),
            newest_diagnostics: previous.newest_diagnostics.clone(),
            ..Default::default()
        };
        let mut changed = false;
        match result {
            Ok(Pulled::Unchanged) => {}
            Ok(Pulled::Good {
                snapshot,
                repo,
                diagnostics,
            }) => {
                let id_owned = id.to_string();
                let snap = snapshot.clone();
                match self
                    .with_store(move |s| s.save_snapshot(&id_owned, &snap))
                    .await
                {
                    Some(Ok(())) => {
                        // Only a cached commit counts as seen: a restart must
                        // not skip a commit the cache doesn't hold.
                        state.newest_commit = Some(snapshot.commit.clone());
                        state.newest_diagnostics = None;
                    }
                    Some(Err(e)) => {
                        tracing::warn!(source = %id, error = %e, "cannot cache the snapshot; serving it anyway")
                    }
                    None => {
                        tracing::warn!(source = %id, "cannot cache the snapshot; serving it anyway")
                    }
                }
                let same = self
                    .served
                    .read()
                    .map(|s| s.get(id).is_some_and(|x| x.commit == snapshot.commit))
                    .unwrap_or(false);
                if !same {
                    tracing::info!(source = %id, commit = %snapshot.commit, ms, "pulled a new commit");
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
                // Re-reading the served commit (after a restart) changes nothing.
                changed = !same;
                self.mark_judged(id);
            }
            Ok(Pulled::Bad {
                commit,
                diagnostics,
            }) => {
                let errors = diagnostics.iter().filter(|d| d.is_error()).count();
                tracing::warn!(source = %id, %commit, errors, "the newest commit fails validation; keeping the last good one");
                state.newest_commit = Some(commit);
                state.newest_diagnostics = Some(diagnostics_json(&diagnostics));
                self.mark_judged(id);
            }
            Err(e) => {
                tracing::warn!(source = %id, error = %e, ms, "pull failed");
                state.last_pull_error = Some(e.to_string());
            }
        }
        let st = state.clone();
        if let Some(Err(e)) = self.with_store(move |s| s.record_pull(&st)).await {
            tracing::warn!(source = %id, error = %e, "cannot record the pull");
        }
        if changed {
            self.rebuild();
        }
        changed
    }

    fn mark_judged(&self, id: &str) {
        if let Ok(mut j) = self.judged.lock() {
            j.insert(id.to_string());
        }
    }

    /// Poll every source every `interval`, and at once when `wake` names it.
    /// `on_first_round` runs after every source was tried once. Each pull is
    /// its own task, so one slow source never holds up the others.
    pub async fn run(
        self: Arc<Self>,
        interval: Duration,
        mut wake: mpsc::Receiver<String>,
        on_first_round: impl FnOnce() + Send + 'static,
    ) {
        let first_round: Vec<_> = self
            .specs
            .iter()
            .map(|s| {
                let me = self.clone();
                let id = s.id.clone();
                tokio::spawn(async move { me.pull(&id).await })
            })
            .collect();
        tokio::spawn(async move {
            for p in first_round {
                let _ = p.await;
            }
            on_first_round();
        });
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // the first tick is the round above
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    for s in &self.specs {
                        let me = self.clone();
                        let id = s.id.clone();
                        // A pull still running for this source holds its lock;
                        // this one waits, then sees an unchanged remote.
                        tokio::spawn(async move { me.pull(&id).await });
                    }
                }
                Some(id) = wake.recv() => {
                    tracing::info!(source = %id, "webhook: pulling now");
                    let me = self.clone();
                    tokio::spawn(async move { me.pull(&id).await });
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
