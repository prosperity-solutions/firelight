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
    /// Staleness inputs per entry (§9.5).
    pub history: Vec<firelight_store::EntryHistory>,
}

pub struct Hub {
    specs: Vec<SourceSpec>,
    puller: Puller,
    store: Arc<Mutex<Store>>,
    limits: Limits,
    self_docs: bool,
    served: RwLock<BTreeMap<String, Served>>,
    /// The index, which holds the catalog: the two swap together.
    pub search: Arc<crate::search::Search>,
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
    /// Version, start time and the effective config (secrets as ••••), set
    /// by the binary at startup, for the status (§16.1).
    pub status_extra: RwLock<serde_json::Value>,
}

impl Hub {
    pub fn new(
        specs: Vec<SourceSpec>,
        puller: Puller,
        store: Store,
        limits: Limits,
        self_docs: bool,
        embedder: &str,
    ) -> Arc<Hub> {
        let pull_locks = specs
            .iter()
            .map(|s| (s.id.clone(), Arc::new(tokio::sync::Mutex::new(()))))
            .collect();
        let store = Arc::new(Mutex::new(store));
        Arc::new(Hub {
            specs,
            puller,
            search: Arc::new(crate::search::Search::new(store.clone(), embedder)),
            store,
            limits,
            self_docs,
            served: RwLock::new(BTreeMap::new()),
            rebuild_lock: Mutex::new(()),
            pull_locks,
            judged: Mutex::new(BTreeSet::new()),
            generation: std::sync::atomic::AtomicU64::new(0),
            status_extra: RwLock::new(serde_json::json!({})),
        })
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        self.search.catalog()
    }

    /// The status (§16.1) as JSON. Source URLs never carry credentials.
    pub fn status_json(&self) -> serde_json::Value {
        use serde_json::json;
        let cat = self.catalog();
        let states = self.source_states();
        let served = self.served.read().map(|s| s.clone()).unwrap_or_default();
        let info = self.search.info();
        let sources: Vec<serde_json::Value> = self
            .specs
            .iter()
            .map(|spec| {
                let st = states.iter().find(|x| x.source_id == spec.id);
                let (kind, url, git_ref) = match &spec.origin {
                    firelight_source::pull::Origin::Dir(p) => ("dir", p.display().to_string(), None),
                    firelight_source::pull::Origin::Git { url, git_ref, .. } => {
                        ("git", strip_credentials(url), git_ref.clone())
                    }
                };
                let sv = served.get(&spec.id);
                let counts = cat.repo(&spec.id).map(|r| {
                    let concepts = r.entries.iter().filter(|e| e.kind == firelight_format::EntryKind::Concept).count();
                    json!({ "concepts": concepts, "guides": r.entries.len() - concepts, "links": r.links.len(), "islands": r.islands.len() })
                });
                let errors: Option<serde_json::Value> = st
                    .and_then(|x| x.newest_diagnostics.as_deref())
                    .and_then(|d| serde_json::from_str(d).ok());
                let warnings: Vec<serde_json::Value> = sv
                    .map(|x| serde_json::from_str(&diagnostics_json(&x.diagnostics)).unwrap_or_default())
                    .into_iter()
                    .collect();
                let health = match (sv, st.and_then(|x| x.last_pull_error.as_ref()), errors.as_ref().is_some_and(|e| e.as_array().is_some_and(|a| !a.is_empty()))) {
                    (None, _, _) => "not served yet",
                    (Some(_), Some(_), _) => "pull failing; serving the last good commit",
                    (Some(_), None, true) => "newest commit fails its checks; serving the last good one",
                    _ => "ok",
                };
                json!({
                    "id": spec.id, "kind": kind, "url": url, "ref": git_ref,
                    "commit": sv.map(|x| x.commit.clone()), "committed_at": sv.and_then(|x| x.committed_at.clone()),
                    "newest_commit": st.and_then(|x| x.newest_commit.clone()),
                    "last_pull_at": st.and_then(|x| x.last_pull_at.clone()),
                    "last_pull_ms": st.and_then(|x| x.last_pull_ms),
                    "last_pull_error": st.and_then(|x| x.last_pull_error.as_deref().map(scrub_urls)),
                    "errors": errors, "warnings": warnings.into_iter().next().unwrap_or(json!([])),
                    "counts": counts, "health": health,
                })
            })
            .collect();
        let (db_size, db_version) = self
            .store
            .lock()
            .map(|s| (s.size_bytes(), s.schema_version().unwrap_or(0)))
            .unwrap_or((0, 0));
        let stale = cat
            .repos
            .iter()
            .flat_map(|r| r.entries.iter().map(move |e| (r, e)))
            .filter(|(r, e)| {
                !e.covers.is_empty()
                    && firelight_server::content::Content::staleness(self, &r.id, &e.path).is_some()
            })
            .count();
        let mut v = json!({
            "format_versions": firelight_format::FORMAT_VERSIONS,
            "sources": sources,
            "maybe_stale": stale,
            "embedder": {
                "backend": info.backend, "model": info.model, "dims": info.dims,
                "chunks_embedded": info.embedded, "chunks": info.chunks,
                "last_index_ms": info.last_index_ms, "problem": info.problem.as_deref().map(scrub_urls),
            },
            "golden": {
                "builtin": info.builtin,
                "failures": info.golden_failures.iter().map(|(r, l)| json!({ "repo": r, "question": l })).collect::<Vec<_>>(),
            },
            "database": { "size_bytes": db_size, "schema_version": db_version },
        });
        if let (Some(o), Ok(extra)) = (v.as_object_mut(), self.status_extra.read())
            && let Some(e) = extra.as_object()
        {
            for (k, x) in e {
                o.insert(k.clone(), x.clone());
            }
        }
        v
    }

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
                                        history: snap.history,
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
        self.search.set_catalog(Arc::new(cat));
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
                            history: snapshot.history,
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

/// The page, API and MCP routes read the hub through this.
impl firelight_server::content::Content for Hub {
    fn catalog(&self) -> Arc<Catalog> {
        Hub::catalog(self)
    }
    fn generation(&self) -> u64 {
        Hub::generation(self)
    }
    fn source(&self, repo: &str) -> Option<firelight_server::content::SourceInfo> {
        let s = self.served.read().ok()?;
        let x = s.get(repo)?;
        Some(firelight_server::content::SourceInfo {
            commit: x.commit.clone(),
            committed_at: x.committed_at.clone(),
            web_url: None,
        })
    }
    fn search(
        &self,
        query: &str,
        filter: &firelight_search::Filter,
        limit: usize,
    ) -> Vec<firelight_search::Hit> {
        self.search.search(query, filter, limit)
    }
    fn status(&self) -> serde_json::Value {
        self.status_json()
    }
    fn search_info(&self) -> firelight_server::content::SearchInfo {
        self.search.info()
    }
    fn staleness(
        &self,
        repo: &str,
        entry_path: &str,
    ) -> Option<firelight_server::content::Staleness> {
        let s = self.served.read().ok()?;
        let h = s
            .get(repo)?
            .history
            .iter()
            .find(|h| h.entry_path == entry_path)?;
        if !firelight_source::pull::is_stale(h) {
            return None;
        }
        Some(firelight_server::content::Staleness {
            entry_changed: h.entry_changed.clone()?,
            covers_changed: h.covers_changed.clone()?,
        })
    }
}

#[cfg(test)]
mod tests;

/// A git URL without user or password (config refuses them; this is the
/// second line of defence for the status output).
fn strip_credentials(u: &str) -> String {
    match url::Url::parse(u) {
        Ok(mut x) => {
            let _ = x.set_username("");
            let _ = x.set_password(None);
            x.set_query(None);
            x.set_fragment(None);
            x.to_string()
        }
        Err(_) => scrub_urls(u.split('@').next_back().unwrap_or(u)),
    }
}

/// Every URL in `s` loses its query and fragment (a `?private_token=…` must
/// never reach the status, §17.1).
pub fn scrub_urls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ']' | '}' | ')' | '>')
            })
            .unwrap_or(tail.len());
        let url = &tail[..end];
        let cut = url.find(['?', '#']).unwrap_or(url.len());
        let url = &url[..cut];
        // `user:pass@host`: keep only what follows the last `@` of the authority.
        let auth_end = url.find('/').unwrap_or(url.len());
        let start = url[..auth_end].rfind('@').map_or(0, |i| i + 1);
        out.push_str(&url[start..]);
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod scrub_tests {
    #[test]
    fn urls_lose_queries() {
        assert_eq!(
            super::scrub_urls(
                r#"[{"url":"https://git.acme.example/a.git?private_token=s3cret#x"}] and http://h/p?k=v."#
            ),
            r#"[{"url":"https://git.acme.example/a.git"}] and http://h/p"#
        );
        assert_eq!(
            super::strip_credentials("https://u:p@git.acme.example/a?key=1"),
            "https://git.acme.example/a"
        );
    }
}
