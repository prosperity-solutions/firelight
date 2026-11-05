//! Search in the running server (§8, §9): the embedder, the index swapped
//! with each catalog, the background embedding, query vectors, and the golden
//! checks.
//!
//! - Every catalog rebuild builds a word index at once, attaches the vectors
//!   the cache already has, and swaps it in together with the catalog (the
//!   index holds the catalog, so the two can never disagree).
//! - One background task embeds what's missing, a few chunks at a time,
//!   filling the current index in place; a newer index replaces the work.
//! - A query is embedded on the caller's blocking thread, at most two at a
//!   time; past that, or on an embedder error, search uses words alone.
//! - The built-in golden check runs on every start against the embedded
//!   self-docs only (never user content) and gates readiness; repo golden
//!   questions run after each full index and only warn.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use firelight_embed::Shared;
use firelight_embed::model::{self, Spec};
use firelight_format::catalog::{self, Catalog, Strictness};
use firelight_format::{GoldenQuestion, InputFile, Limits, parse_repo};
use firelight_search::golden::{self, Outcome};
use firelight_search::{Filter, Hit, Index};
use firelight_server::content::SearchInfo;
use firelight_store::Store;

/// What builds the embedder (from the config, or preview's flags).
#[derive(Debug, Clone)]
pub struct EmbedConfig {
    /// `candle`, `openai-compatible` or `off`.
    pub backend: String,
    pub model: String,
    pub revision: Option<String>,
    pub sha256: Option<String>,
    pub hf_endpoint: String,
    pub path: Option<PathBuf>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    /// Models live under `<models_dir>/models/…`.
    pub models_dir: PathBuf,
}

impl EmbedConfig {
    #[cfg(test)]
    pub fn off() -> EmbedConfig {
        EmbedConfig {
            backend: "off".into(),
            model: String::new(),
            revision: None,
            sha256: None,
            hf_endpoint: "https://huggingface.co".into(),
            path: None,
            base_url: None,
            api_key: None,
            models_dir: PathBuf::new(),
        }
    }

    /// The Hugging Face spec of a `candle` model.
    pub fn spec(&self) -> Result<Spec, String> {
        let default = self.model == model::DEFAULT_REPO;
        let revision = match (&self.revision, default) {
            (Some(r), _) => r.clone(),
            (None, true) => model::DEFAULT_REVISION.to_string(),
            (None, false) => {
                return Err(format!(
                    "embed.revision is required for {}: its 40-hex commit",
                    self.model
                ));
            }
        };
        let sha256 = parse_sha256(self.sha256.as_deref().unwrap_or(""))?;
        let spec = Spec {
            repo: self.model.clone(),
            revision,
            sha256,
            endpoint: self.hf_endpoint.trim_end_matches('/').to_string(),
        };
        spec.validate().map_err(|e| e.to_string())?;
        // From a mirror, the listing and the files come from one server, so
        // one can't vouch for the other: the default model's checksums are
        // compiled in; any other model needs embed.sha256 for every file.
        let official = spec.endpoint == "https://huggingface.co";
        if !official && model::pinned_expectations(&spec).is_none() {
            return Err(format!(
                "a model other than the default from a mirror ({}) needs embed.sha256 for {:?}",
                spec.endpoint,
                model::FILES
            ));
        }
        Ok(spec)
    }
}

/// `file=hex,file=hex` → map.
pub fn parse_sha256(s: &str) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut out = std::collections::BTreeMap::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (f, h) = part
            .split_once('=')
            .ok_or_else(|| format!("embed.sha256: {part:?} should be file=hex"))?;
        out.insert(f.trim().to_string(), h.trim().to_ascii_lowercase());
    }
    Ok(out)
}

/// Build the embedder. `None` for `off`. Downloads and verifies a model when
/// it isn't cached. Call from an async context; the heavy parts block on
/// their own threads.
pub async fn load_embedder(cfg: &EmbedConfig) -> Result<Option<Shared>, String> {
    match cfg.backend.as_str() {
        "off" => Ok(None),
        "openai-compatible" => {
            let base = cfg
                .base_url
                .clone()
                .ok_or("embed.base_url is required with openai-compatible")?;
            let model = cfg.model.clone();
            let key = cfg.api_key.clone();
            let rt = tokio::runtime::Handle::current();
            let e = tokio::task::spawn_blocking(move || {
                firelight_embed::openai::OpenAi::new(&base, &model, key, rt)
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
            Ok(Some(Arc::new(e)))
        }
        "candle" => {
            let (dir, id) = if let Some(p) = cfg.path.clone() {
                let spec = cfg.spec().ok();
                // Hashing a local model reads every byte: off the workers.
                let dir = tokio::task::spawn_blocking(move || model::local(spec.as_ref(), &p))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                (dir, format!("{}@local", cfg.model))
            } else {
                let spec = cfg.spec()?;
                let client = firelight_source::http_client();
                let dir = model::ensure(&spec, &cfg.models_dir, &client)
                    .await
                    .map_err(|e| e.to_string())?;
                (dir, format!("{}@{}", spec.repo, spec.revision))
            };
            let e = tokio::task::spawn_blocking(move || {
                firelight_embed::candle::Candle::load(&dir, &id)
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
            Ok(Some(Arc::new(e)))
        }
        other => Err(format!("unknown embedder {other:?}")),
    }
}

/// Why a golden run didn't finish: worth retrying (the embedder) or not.
pub enum Retry {
    Yes(String),
    No(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Pending,
    Pass,
    Fail,
    Skipped,
}

impl Builtin {
    fn as_str(self) -> &'static str {
        match self {
            Builtin::Pending => "pending",
            Builtin::Pass => "pass",
            Builtin::Fail => "fail",
            Builtin::Skipped => "skipped",
        }
    }
}

struct Status {
    backend: String,
    model: Option<String>,
    problem: Option<String>,
    last_index_ms: Option<u64>,
    builtin: Builtin,
    builtin_results: Vec<golden::Result>,
    repo_results: Vec<golden::Result>,
}

pub struct Search {
    embedder: RwLock<Option<Shared>>,
    index: RwLock<Arc<Index>>,
    /// The newest catalog. `reindex` always builds from this, under
    /// `reindex_lock`, with the embedder as it is then: an older build can
    /// never be swapped in over a newer catalog or a newer embedder.
    latest: RwLock<Arc<Catalog>>,
    reindex_lock: Mutex<()>,
    store: Arc<Mutex<Store>>,
    wake: tokio::sync::watch::Sender<u64>,
    qcache: Mutex<VecDeque<(String, Vec<f32>)>>,
    inflight: Arc<AtomicUsize>,
    /// Chunks the embedder refused one by one (bad input, content filter):
    /// left to word search instead of retried forever.
    refused: Mutex<HashSet<[u8; 32]>>,
    status: RwLock<Status>,
    /// The built-in golden check passed (or was skipped).
    pub golden_ok: AtomicBool,
    generation: AtomicUsize,
}

const QUERY_CACHE: usize = 64;
const QUERY_SLOTS: usize = 2;
/// A query waits this long for its vector, then searches with words.
const QUERY_WAIT: Duration = Duration::from_secs(2);
const BATCH: usize = 8;

/// Run blocking work from either side: on a multi-thread runtime's worker it
/// tells tokio to move other tasks away; elsewhere it just runs.
pub fn blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

/// Gives a query slot back when dropped, even after a panic.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Search {
    pub fn new(store: Arc<Mutex<Store>>, backend: &str) -> Search {
        let (wake, _) = tokio::sync::watch::channel(0);
        let empty = Arc::new(Catalog::default());
        Search {
            embedder: RwLock::new(None),
            index: RwLock::new(Arc::new(Index::words_only(empty.clone()).unwrap_or_else(
                |e| {
                    tracing::error!(error = %e, "cannot build an empty word index");
                    std::process::exit(1)
                },
            ))),
            latest: RwLock::new(empty),
            reindex_lock: Mutex::new(()),
            store,
            wake,
            qcache: Mutex::new(VecDeque::new()),
            inflight: Arc::new(AtomicUsize::new(0)),
            refused: Mutex::new(HashSet::new()),
            status: RwLock::new(Status {
                backend: backend.to_string(),
                model: None,
                problem: None,
                last_index_ms: None,
                builtin: Builtin::Pending,
                builtin_results: Vec::new(),
                repo_results: Vec::new(),
            }),
            golden_ok: AtomicBool::new(false),
            generation: AtomicUsize::new(0),
        }
    }

    pub fn index(&self) -> Arc<Index> {
        self.index
            .read()
            .map(|i| i.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        self.index().cat.clone()
    }

    pub fn embedder(&self) -> Option<Shared> {
        self.embedder.read().ok().and_then(|e| e.clone())
    }

    pub fn set_problem(&self, p: Option<String>) {
        if let Ok(mut s) = self.status.write() {
            s.problem = p;
        }
    }

    /// A new catalog: index it and swap both in. Blocking (chunking and the
    /// word index); async callers are moved off the worker by [`blocking`].
    pub fn set_catalog(self: &Arc<Self>, cat: Arc<Catalog>) {
        if let Ok(mut l) = self.latest.write() {
            *l = cat;
        }
        blocking(|| self.reindex());
        // Without an embedder no embed loop runs: repo golden questions run
        // here, after each re-index (§9.4).
        if self.embedder().is_none()
            && let Ok(h) = tokio::runtime::Handle::try_current()
        {
            let me = self.clone();
            h.spawn(async move { me.run_repo_golden().await });
        }
    }

    /// Build the index for the newest catalog and the current embedder (with
    /// cached vectors) and swap it in.
    fn reindex(&self) {
        let _one_at_a_time = self.reindex_lock.lock();
        let cat = self.latest.read().map(|c| c.clone()).unwrap_or_default();
        let emb = self.embedder();
        let built = match &emb {
            Some(e) => {
                let e2 = e.clone();
                Index::build(cat, Some(e.id()), e.max_tokens(), &move |t| {
                    e2.count_tokens(t)
                })
            }
            None => Index::words_only(cat),
        };
        let idx = match built {
            Ok(i) => i,
            Err(err) => {
                tracing::error!(error = %err, "cannot build the word index; keeping the old one");
                return;
            }
        };
        if let Some(e) = &emb
            && let Ok(store) = self.store.lock()
            && let Ok(v) = store.vectors(&e.id(), e.dims(), &idx.hashes())
        {
            idx.set_vectors(&v);
        }
        if let Ok(mut i) = self.index.write() {
            *i = Arc::new(idx);
        }
        let g = self.generation.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        let _ = self.wake.send(g);
    }

    /// Search the current index.
    pub fn search(&self, q: &str, f: &Filter, limit: usize) -> Vec<Hit> {
        let idx = self.index();
        let v = self.query_vec(&idx, q);
        idx.search(q, f, limit, v.as_deref())
    }

    /// The query's vector, if meaning search can help now. Never waits more
    /// than [`QUERY_WAIT`], and never more than [`QUERY_SLOTS`] at a time.
    fn query_vec(&self, idx: &Index, q: &str) -> Option<Vec<f32>> {
        let e = self.embedder()?;
        if idx.embedder.as_deref() != Some(e.id().as_str()) || idx.coverage().0 == 0 {
            return None;
        }
        let key = q.trim().to_lowercase();
        if let Ok(c) = self.qcache.lock()
            && let Some((_, v)) = c.iter().find(|(k, _)| *k == key)
        {
            return Some(v.clone());
        }
        // Busy: answer with words now rather than queue behind other queries.
        if self.inflight.fetch_add(1, Ordering::SeqCst) >= QUERY_SLOTS {
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        let slot = Slot(self.inflight.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let q2 = q.to_string();
        // The slot stays taken until the embedding really ends, so a hung
        // backend can hold at most QUERY_SLOTS threads.
        std::thread::spawn(move || {
            let _slot = slot;
            let _ = tx.send(e.embed_query(&q2));
        });
        match rx.recv_timeout(QUERY_WAIT) {
            Ok(Ok(v)) => {
                if let Ok(mut c) = self.qcache.lock() {
                    c.push_front((key, v.clone()));
                    c.truncate(QUERY_CACHE);
                }
                Some(v)
            }
            Ok(Err(err)) => {
                tracing::warn!(error = %err, "cannot embed the query; searching with words");
                None
            }
            Err(_) => {
                tracing::warn!("embedding the query took over 2 s; searching with words");
                None
            }
        }
    }

    /// Install the embedder, rebuild the index for it, and start embedding.
    pub async fn start_embedder(self: &Arc<Self>, e: Shared) {
        if let Ok(mut s) = self.status.write() {
            s.model = Some(e.id());
            s.problem = None;
        }
        if let Ok(mut x) = self.embedder.write() {
            *x = Some(e.clone());
        }
        let me = self.clone();
        let e2 = e.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(mut st) = me.store.lock()
                && let Ok(n) = st.prune_other_models(&e2.id(), e2.dims())
                && n > 0
            {
                tracing::info!(removed = n, "dropped vectors of an earlier model");
            }
            me.reindex();
        })
        .await;
        let me = self.clone();
        tokio::spawn(async move { me.embed_loop(e).await });
    }

    /// Embed what the current index lacks; start over when a newer index
    /// arrives. A batch error is retried text by text: texts the backend
    /// refuses are left to words, and if every text fails the backend is
    /// down, so wait (30 s, doubling to 10 min) and try again.
    async fn embed_loop(self: Arc<Self>, e: Shared) {
        let mut rx = self.wake.subscribe();
        let mut wait = Duration::from_secs(30);
        loop {
            rx.mark_unchanged();
            let started = Instant::now();
            let mut did = 0usize;
            let idx = self.index();
            let refused = self.refused.lock().map(|r| r.clone()).unwrap_or_default();
            let missing: Vec<([u8; 32], String)> = idx
                .missing()
                .into_iter()
                .filter(|(h, _)| !refused.contains(h))
                .collect();
            let mut outage = false;
            for batch in missing.chunks(BATCH) {
                if !Arc::ptr_eq(&idx, &self.index()) {
                    break; // a newer index: start over with it
                }
                let rows = self.embed_batch(&e, batch).await;
                let Some(rows) = rows else {
                    outage = true;
                    break;
                };
                did += rows.len();
                let by: HashMap<[u8; 32], Vec<f32>> = rows.iter().cloned().collect();
                idx.set_vectors(&by);
                // A swap during this batch: the new index shares most chunks.
                let cur = self.index();
                if !Arc::ptr_eq(&idx, &cur) {
                    cur.set_vectors(&by);
                }
                let store = self.store.clone();
                let id = e.id();
                let _ = tokio::task::spawn_blocking(move || {
                    if let Ok(mut s) = store.lock()
                        && let Err(err) = s.put_vectors(&id, &rows)
                    {
                        tracing::warn!(error = %err, "cannot cache vectors");
                    }
                })
                .await;
            }
            if outage {
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(600));
                continue;
            }
            wait = Duration::from_secs(30);
            if !Arc::ptr_eq(&idx, &self.index()) {
                continue;
            }
            if did > 0 {
                let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                tracing::info!(chunks = did, ms, "indexed");
                if let Ok(mut s) = self.status.write() {
                    s.last_index_ms = Some(ms);
                }
                self.prune_unused(&e, &idx).await;
            }
            self.run_repo_golden().await;
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    /// One batch; on error, text by text. `None`: the backend is down.
    async fn embed_batch(
        &self,
        e: &Shared,
        batch: &[([u8; 32], String)],
    ) -> Option<Vec<([u8; 32], Vec<f32>)>> {
        let run = |texts: Vec<String>| {
            let e = e.clone();
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
                e.embed_documents(&refs)
            })
        };
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        match run(texts).await {
            Ok(Ok(v)) => {
                self.set_problem(None);
                return Some(batch.iter().map(|(h, _)| *h).zip(v).collect());
            }
            Ok(Err(err)) => {
                tracing::warn!(error = %err, "embedding a batch failed; trying text by text")
            }
            Err(_) => return None,
        }
        let mut rows = Vec::new();
        let mut failed = Vec::new();
        let mut last_err = String::new();
        for (h, t) in batch {
            match run(vec![t.clone()]).await {
                Ok(Ok(mut v)) if v.len() == 1 => rows.push((*h, v.remove(0))),
                Ok(Err(err)) => {
                    last_err = err.to_string();
                    failed.push(*h);
                }
                _ => failed.push(*h),
            }
        }
        if rows.is_empty() {
            tracing::warn!(error = %last_err, "the embedder fails on every text; waiting before trying again");
            self.set_problem(Some(format!("embedding failed: {last_err}")));
            return None;
        }
        if !failed.is_empty() {
            tracing::warn!(chunks = failed.len(), error = %last_err, "the embedder refused some chunks; they stay word-search only");
            if let Ok(mut r) = self.refused.lock() {
                r.extend(failed);
            }
        }
        Some(rows)
    }

    /// Keep the cache near the size of what is indexed: once it holds more
    /// than twice the current chunks (edits leave old vectors behind), drop
    /// the active model's vectors no chunk uses.
    async fn prune_unused(&self, e: &Shared, idx: &Arc<Index>) {
        let store = self.store.clone();
        let (id, dims) = (e.id(), e.dims());
        let keep = idx.hashes();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(mut s) = store.lock()
                && s.vector_count(&id, dims).unwrap_or(0) > 2 * keep.len() + 1000
            {
                match s.retain_vectors(&id, dims, &keep) {
                    Ok(n) => tracing::info!(removed = n, "pruned unused vectors"),
                    Err(err) => tracing::warn!(error = %err, "cannot prune vectors"),
                }
            }
        })
        .await;
    }

    /// Every source's golden.yaml against the full index; failures warn.
    async fn run_repo_golden(self: &Arc<Self>) {
        let me = self.clone();
        let res = tokio::task::spawn_blocking(move || {
            let idx = me.index();
            let qs: Vec<(String, GoldenQuestion)> = idx
                .cat
                .repos
                .iter()
                .filter(|r| r.id != firelight_format::SELF_REPO)
                .flat_map(|r| r.golden.iter().map(move |q| (r.id.clone(), q.clone())))
                .collect();
            let res = golden::run(&idx, &qs, &Filter::default(), &|q| me.query_vec(&idx, q));
            for r in res.iter().filter(|r| r.failed()) {
                tracing::warn!(repo = %r.repo, "golden question fails: {}", golden::describe(r));
            }
            res
        })
        .await
        .unwrap_or_default();
        if let Ok(mut s) = self.status.write() {
            s.repo_results = res;
        }
    }

    /// The built-in check (§9.4) against the self-docs only (or the repos the
    /// golden file names). An embedder error is not a result: it is retried
    /// (5 s, doubling to 5 min) while `/readyz` stays 503.
    pub async fn run_builtin(
        self: &Arc<Self>,
        self_docs: Option<Vec<InputFile>>,
        golden_file: Option<PathBuf>,
        limits: Limits,
    ) -> Builtin {
        let mut wait = Duration::from_secs(5);
        let res = loop {
            let me = self.clone();
            let (d, g) = (self_docs.clone(), golden_file.clone());
            let res = tokio::task::spawn_blocking(move || me.builtin_blocking(d, g, &limits))
                .await
                .unwrap_or_else(|e| Err(Retry::No(e.to_string())));
            match res {
                Err(Retry::Yes(e)) => {
                    tracing::warn!(error = %e, "the built-in golden check can't embed yet; retrying");
                    self.set_problem(Some(e));
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(Duration::from_secs(300));
                }
                Err(Retry::No(e)) => break Err(e),
                Ok(r) => break Ok(r),
            }
        };
        let (state, results) = match res {
            Ok(None) => (Builtin::Skipped, Vec::new()),
            Ok(Some(r)) if r.iter().any(golden::Result::failed) => (Builtin::Fail, r),
            Ok(Some(r)) => (Builtin::Pass, r),
            Err(e) => {
                tracing::error!(error = %e, "the built-in golden check couldn't run");
                self.set_problem(Some(e));
                (Builtin::Fail, Vec::new())
            }
        };
        for r in results.iter().filter(|r| r.failed()) {
            tracing::error!("built-in golden question fails: {}", golden::describe(r));
        }
        for r in results
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Skipped))
        {
            tracing::info!("built-in golden question skipped: {}", golden::describe(r));
        }
        if let Ok(mut s) = self.status.write() {
            s.builtin = state;
            s.builtin_results = results;
            if state != Builtin::Fail {
                s.problem = None;
            }
        }
        self.golden_ok.store(
            matches!(state, Builtin::Pass | Builtin::Skipped),
            Ordering::SeqCst,
        );
        state
    }

    fn builtin_blocking(
        &self,
        self_docs: Option<Vec<InputFile>>,
        golden_file: Option<PathBuf>,
        limits: &Limits,
    ) -> Result<Option<Vec<golden::Result>>, Retry> {
        let emb = self.embedder();
        let (cat, qs) = if let Some(files) = self_docs {
            let (repo, diags) = parse_repo(&files, Some(firelight_format::SELF_REPO), limits);
            let repo =
                repo.ok_or_else(|| Retry::No(format!("the self-docs don't parse: {diags:?}")))?;
            let qs: Vec<(String, GoldenQuestion)> = repo
                .golden
                .iter()
                .map(|q| (repo.id.clone(), q.clone()))
                .collect();
            (catalog::build(vec![repo], Strictness::Lenient), qs)
        } else if let Some(path) = golden_file {
            let qs = read_golden_file(&path, limits).map_err(Retry::No)?;
            let names: BTreeSet<String> = qs
                .iter()
                .flat_map(|(_, q)| q.expect.iter().map(|(r, _)| r.repo.clone()))
                .collect();
            // Only the repos the file names, as they are served now.
            let latest = self.latest.read().map(|c| c.clone()).unwrap_or_default();
            let repos: Vec<_> = latest
                .repos
                .iter()
                .filter(|r| names.contains(&r.id))
                .cloned()
                .collect();
            (catalog::build(repos, Strictness::Lenient), qs)
        } else {
            tracing::warn!(
                "self_docs is off and no golden_file is set: the built-in golden check is skipped"
            );
            return Ok(None);
        };
        let idx = self.embedded_index(Arc::new(cat), emb.as_ref())?;
        // Embed every question first: an embedder error must not turn a
        // `needs: meaning` question into "skipped" (a broken model would pass).
        let mut vecs: HashMap<String, Vec<f32>> = HashMap::new();
        if let Some(e) = &emb {
            for (_, q) in &qs {
                let v = e
                    .embed_query(&q.query)
                    .map_err(|x| Retry::Yes(format!("embedding a golden question failed: {x}")))?;
                vecs.insert(q.query.clone(), v);
            }
        }
        Ok(Some(golden::run(&idx, &qs, &Filter::default(), &|q| {
            vecs.get(q).cloned()
        })))
    }

    /// A fully embedded index over `cat` (cache first), for the golden check.
    fn embedded_index(&self, cat: Arc<Catalog>, emb: Option<&Shared>) -> Result<Index, Retry> {
        let Some(e) = emb else {
            return Index::words_only(cat).map_err(Retry::No);
        };
        let e2 = e.clone();
        let idx = Index::build(cat, Some(e.id()), e.max_tokens(), &move |t| {
            e2.count_tokens(t)
        })
        .map_err(Retry::No)?;
        if let Ok(store) = self.store.lock()
            && let Ok(v) = store.vectors(&e.id(), e.dims(), &idx.hashes())
        {
            idx.set_vectors(&v);
        }
        let missing = idx.missing();
        for batch in missing.chunks(BATCH) {
            let texts: Vec<&str> = batch.iter().map(|(_, t)| t.as_str()).collect();
            let vecs = e
                .embed_documents(&texts)
                .map_err(|x| Retry::Yes(format!("embedding the golden set failed: {x}")))?;
            let rows: Vec<([u8; 32], Vec<f32>)> = batch.iter().map(|(h, _)| *h).zip(vecs).collect();
            idx.set_vectors(&rows.iter().cloned().collect());
            if let Ok(mut s) = self.store.lock() {
                let _ = s.put_vectors(&e.id(), &rows);
            }
        }
        Ok(idx)
    }

    pub fn info(&self) -> SearchInfo {
        let idx = self.index();
        let (embedded, chunks) = idx.coverage();
        let emb = self.embedder();
        let s = self.status.read();
        let Ok(s) = s else {
            return SearchInfo::default();
        };
        let mut failures: Vec<(String, String)> = Vec::new();
        for r in s
            .builtin_results
            .iter()
            .chain(&s.repo_results)
            .filter(|r| r.failed())
        {
            failures.push((r.repo.clone(), golden::describe(r)));
        }
        SearchInfo {
            backend: s.backend.clone(),
            model: s.model.clone(),
            dims: emb.map(|e| e.dims()),
            embedded,
            chunks,
            last_index_ms: s.last_index_ms,
            problem: s.problem.clone(),
            builtin: s.builtin.as_str().to_string(),
            golden_failures: failures,
        }
    }
}

/// A golden file (`golden_file`, §9.4) whose refs name their repos.
fn read_golden_file(path: &Path, limits: &Limits) -> Result<Vec<(String, GoldenQuestion)>, String> {
    let text = std::fs::read(path).map_err(|e| format!("golden_file {}: {e}", path.display()))?;
    let files = vec![
        InputFile::regular(
            "repo.yaml",
            b"format: 1\nid: golden-file\nname: Golden file\nsummary: The golden file.\n".to_vec(),
        ),
        InputFile::regular("golden.yaml", text),
    ];
    let (repo, diags) = parse_repo(&files, None, limits);
    let errors: Vec<String> = diags
        .iter()
        .filter(|d| d.is_error())
        .map(ToString::to_string)
        .collect();
    let repo =
        repo.ok_or_else(|| format!("golden_file {}: {}", path.display(), errors.join("; ")))?;
    Ok(repo
        .golden
        .into_iter()
        .map(|q| ("golden-file".to_string(), q))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_with(e: Option<Shared>) -> Arc<Search> {
        let store = Arc::new(Mutex::new(
            Store::memory().unwrap_or_else(|x| panic!("{x}")),
        ));
        let s = Arc::new(Search::new(store, if e.is_some() { "fake" } else { "off" }));
        if let Some(e) = e
            && let Ok(mut x) = s.embedder.write()
        {
            *x = Some(e);
        }
        s
    }

    /// The built-in check passes on the shipped self-docs, with words alone
    /// and with an embedder, and gates readiness.
    #[tokio::test(flavor = "multi_thread")]
    async fn builtin_golden_passes_on_the_self_docs() {
        for e in [
            None,
            Some(Arc::new(firelight_embed::Fake::default()) as Shared),
        ] {
            let s = search_with(e);
            let docs = Some(crate::hub::self_docs_files());
            let state = s.run_builtin(docs, None, Limits::default()).await;
            assert_eq!(state, Builtin::Pass, "{:?}", s.info().golden_failures);
            assert!(s.golden_ok.load(Ordering::SeqCst));
        }
    }

    fn self_docs_catalog() -> Arc<Catalog> {
        let (repo, _) = parse_repo(
            &crate::hub::self_docs_files(),
            Some(firelight_format::SELF_REPO),
            &Limits::default(),
        );
        Arc::new(catalog::build(
            repo.into_iter().collect(),
            Strictness::Lenient,
        ))
    }

    /// Catalog updates and the embedder arriving, in any order and at once,
    /// always end with the newest catalog indexed for the embedder.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_newest_catalog_and_the_embedder_always_win() {
        for round in 0..20 {
            let s = search_with(None);
            let old = Arc::new(Catalog::default());
            let new = self_docs_catalog();
            s.set_catalog(old);
            let e: Shared = Arc::new(firelight_embed::Fake::default());
            let (s1, s2, n) = (s.clone(), s.clone(), new.clone());
            let a = tokio::spawn(async move { s1.start_embedder(e).await });
            let b = tokio::task::spawn_blocking(move || s2.set_catalog(n));
            let _ = tokio::join!(a, b);
            let idx = s.index();
            assert!(
                Arc::ptr_eq(&idx.cat, &new),
                "round {round}: an older catalog came back"
            );
            assert_eq!(
                idx.embedder.as_deref(),
                Some("fake-64"),
                "round {round}: words-only index won"
            );
        }
    }

    /// A failing question keeps readiness off and names where its entry ranked.
    #[tokio::test(flavor = "multi_thread")]
    async fn builtin_golden_failure_blocks_readiness() {
        let s = search_with(None);
        let mut docs = crate::hub::self_docs_files();
        for f in &mut docs {
            if f.path == "golden.yaml" {
                *f = InputFile::regular(
                    "golden.yaml",
                    b"format: 1\nquestions:\n  - query: zebra crossing\n    expect: [check-a-repo]\n".to_vec(),
                );
            }
        }
        assert_eq!(
            s.run_builtin(Some(docs), None, Limits::default()).await,
            Builtin::Fail
        );
        assert!(!s.golden_ok.load(Ordering::SeqCst));
        let info = s.info();
        assert_eq!(info.builtin, "fail");
        assert!(
            info.golden_failures
                .iter()
                .any(|(_, l)| l.contains("zebra")),
            "{info:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_self_docs_and_no_golden_file_skips() {
        let s = search_with(None);
        assert_eq!(
            s.run_builtin(None, None, Limits::default()).await,
            Builtin::Skipped
        );
        assert!(s.golden_ok.load(Ordering::SeqCst));
    }

    /// The model job (§13.4, D4): the default model on the self-docs, the
    /// built-in golden set, index time and query latency into the job
    /// summary; fails over the p95 budget (`FIRELIGHT_P95_BUDGET_MS`).
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "downloads the default model; the model job runs it"]
    async fn model_job() {
        if std::env::var_os("FIRELIGHT_MODEL_TEST").is_none() {
            return;
        }
        let models = std::env::var_os("FIRELIGHT_MODEL_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
        let mut cfg = EmbedConfig::off();
        cfg.backend = "candle".into();
        cfg.model = model::DEFAULT_REPO.into();
        cfg.models_dir = models;
        let t = Instant::now();
        let e = load_embedder(&cfg)
            .await
            .unwrap_or_else(|x| panic!("{x}"))
            .unwrap_or_else(|| panic!("no embedder"));
        let load = t.elapsed();
        let s = search_with(Some(e.clone()));
        let t = Instant::now();
        let state = s
            .run_builtin(Some(crate::hub::self_docs_files()), None, Limits::default())
            .await;
        let index = t.elapsed();
        let chunks = s.info();
        assert_eq!(state, Builtin::Pass, "{:?}", chunks.golden_failures);
        let queries = [
            "how do I validate my firelight folder",
            "what is the .firelight folder",
            "connect my coding agent",
            "wie prüfe ich meinen Ordner",
            "add a repo",
            "template island inputs",
            "golden questions",
            "maybe stale",
            "spaces",
            "where do links go",
            "run firelight locally",
            "MCP token",
            "hide a card",
            "release",
            "what does covers do",
            "basic auth",
            "webhook secret",
            "write a guide",
            "aliases",
            "model download",
        ];
        let mut ms: Vec<f64> = Vec::new();
        for q in queries {
            let t = Instant::now();
            let _ = e.embed_query(q);
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        ms.sort_by(f64::total_cmp);
        let p = |x: f64| ms[((ms.len() as f64 - 1.0) * x).round() as usize];
        let (p50, p95) = (p(0.5), p(0.95));
        let summary = format!(
            "### Model job\n\n| | |\n|---|---|\n| Model | `{}` |\n| Load (incl. download/verify) | {:.1} s |\n| Built-in golden | {:?} |\n| Index + golden (self-docs) | {:.1} s |\n| Query p50 | {p50:.0} ms |\n| Query p95 | {p95:.0} ms |\n| Peak memory | {} |\n",
            e.id(),
            load.as_secs_f64(),
            state,
            index.as_secs_f64(),
            // Linux: the process's high-water mark.
            std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|t| t
                    .lines()
                    .find(|l| l.starts_with("VmHWM:"))
                    .map(|l| l[6..].trim().to_string()))
                .unwrap_or_else(|| "n/a".into())
        );
        eprintln!("{summary}");
        if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
                let _ = f.write_all(summary.as_bytes());
            }
        }
        let budget: f64 = std::env::var("FIRELIGHT_P95_BUDGET_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1500.0);
        assert!(
            p95 <= budget,
            "query p95 {p95:.0} ms is over the {budget:.0} ms budget (D4)"
        );
    }

    #[test]
    fn sha256_pairs() {
        let m = parse_sha256("config.json=AB, model.safetensors=cd").unwrap_or_default();
        assert_eq!(m.get("config.json").map(String::as_str), Some("ab"));
        assert!(parse_sha256("nonsense").is_err());
        assert!(parse_sha256("").unwrap_or_default().is_empty());
    }

    #[test]
    fn mirrors_need_checksums_for_other_models() {
        let mut c = EmbedConfig::off();
        c.backend = "candle".into();
        c.model = model::DEFAULT_REPO.into();
        c.hf_endpoint = "https://hf-mirror.acme.example".into();
        assert!(
            c.spec().is_ok(),
            "the default model's checksums are compiled in"
        );
        c.model = "intfloat/multilingual-e5-small".into();
        c.revision = Some("614241f622f53c4eeff9890bdc4f31cfecc418b3".into());
        assert!(c.spec().is_err());
        c.hf_endpoint = "https://huggingface.co".into();
        assert!(c.spec().is_ok());
        c.model = "../etc".into();
        assert!(c.spec().is_err());
    }
}
