//! `firelight preview [DIR…]` (§12.1, §12.2): the same server as `serve`, with
//! local folders as `dir` sources, auth off, on loopback, reloading the page
//! when a file changes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use firelight_format::{Limits, parse_repo};
use firelight_source::SystemClock;
use firelight_source::git::Git;
use firelight_source::pull::{Origin, Puller, SourceSpec};
use firelight_store::Store;

use crate::hub::Hub;

pub struct Args {
    pub dirs: Vec<PathBuf>,
    pub listen: std::net::SocketAddr,
    pub embedder: Option<String>,
}

/// The repo id a folder declares, or its folder name.
fn id_of(dir: &std::path::Path, limits: &Limits) -> String {
    let files = firelight_source::read_firelight_dir(dir, limits).unwrap_or_default();
    let (repo, _) = parse_repo(&files, None, limits);
    repo.map(|r| r.id).unwrap_or_else(|| {
        dir.canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
            .unwrap_or_else(|| "preview".into())
    })
}

pub fn run(args: Args) -> anyhow::Result<()> {
    check_listen(args.listen)?;
    let embed = embed_config(args.embedder.as_deref())?;
    let limits = Limits::default();
    let dirs = if args.dirs.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        args.dirs
    };
    let mut specs = Vec::new();
    for d in &dirs {
        let id = id_of(d, &limits);
        if specs.iter().any(|s: &SourceSpec| s.id == id) {
            anyhow::bail!("two folders declare the repo id {id:?}; preview each repo once");
        }
        specs.push(SourceSpec {
            id,
            origin: Origin::Dir(d.clone()),
        });
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let data = std::env::temp_dir().join(format!("firelight-preview-{}", std::process::id()));
        let puller = Puller {
            git: Git::default(),
            data_dir: data,
            limits,
            clock: Arc::new(SystemClock),
        };
        let store = Store::memory().context("in-memory database")?;
        let hub = Hub::new(specs.clone(), puller, store, limits, true, &embed.backend);
        hub.load_cache();
        for s in &specs {
            hub.pull(&s.id).await;
        }
        // Report what doesn't pass at start, then every change (and recovery).
        let mut last = problems(&hub);
        report(&[], &last);
        // Live reload: re-read every folder each second.
        let watcher = hub.clone();
        let ids: Vec<String> = specs.iter().map(|s| s.id.clone()).collect();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_secs(1));
            t.tick().await; // the first tick is immediate; the folders were just read
            loop {
                t.tick().await;
                let mut changed = false;
                for id in &ids {
                    changed |= watcher.pull(id).await;
                }
                if changed {
                    tracing::info!("reloaded");
                }
                let now = problems(&watcher);
                if now != last {
                    report(&last, &now);
                    last = now;
                }
            }
        });
        let ui = firelight_server::pages::Ui {
            title: "Firelight preview".into(),
            default_space: None,
            public_url: format!("http://{}", args.listen),
            version: crate::VERSION.into(),
            mcp_token: false,
            live: true,
        };
        let state = firelight_server::AppState::new().with_content(hub.clone(), ui, "#d9541e");
        state.set_ready(true);
        // The page serves at once with word search; meaning search joins when
        // the model is ready (the first run downloads it).
        let (fail_tx, fail_rx) = tokio::sync::oneshot::channel::<String>();
        let search = hub.search.clone();
        tokio::spawn(async move {
            if embed.backend != "off" {
                eprintln!(
                    "firelight preview: loading the {} embedder (`--embedder off` skips it)",
                    embed.backend
                );
            }
            match crate::search::load_embedder(&embed).await {
                Ok(Some(e)) => {
                    eprintln!("firelight preview: meaning search is on ({})", e.id());
                    search.start_embedder(e).await;
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = fail_tx.send(e);
                }
            }
        });
        let server = firelight_server::serve(
            args.listen,
            state,
            |addr| eprintln!("firelight preview: http://{addr}/ (Ctrl-C to stop)"),
            firelight_server::shutdown_signal(),
        );
        tokio::select! {
            r = server => r?,
            Ok(e) = fail_rx => anyhow::bail!("the embedder can't start: {e}. Try --embedder off"),
        }
        Ok(())
    })
}

/// Per source: its failing checks and its last read error, if any.
type Problems = Vec<(String, Option<String>, Option<String>)>;

fn problems(hub: &Hub) -> Problems {
    hub.source_states()
        .into_iter()
        .map(|s| {
            (
                s.source_id,
                s.newest_diagnostics.filter(|d| d != "[]"),
                s.last_pull_error,
            )
        })
        .collect()
}

/// Print what is wrong now that wasn't before, and what passes again.
fn report(before: &[(String, Option<String>, Option<String>)], now: &Problems) {
    for (id, diags, err) in now {
        let old = before.iter().find(|(i, ..)| i == id);
        let (old_d, old_e) = old.map_or((None, None), |(_, d, e)| (d.as_ref(), e.as_ref()));
        if let Some(d) = diags
            && old_d != Some(d)
        {
            eprintln!(
                "firelight preview: {id} doesn't pass its checks, so the last good version shows: {d}"
            );
        }
        if let Some(e) = err
            && old_e != Some(e)
        {
            eprintln!("firelight preview: {id}: {e}");
        }
        if diags.is_none() && err.is_none() && (old_d.is_some() || old_e.is_some()) {
            eprintln!("firelight preview: {id} passes again");
        }
    }
}

/// Auth is off in preview, so it never listens beyond this machine (§12.1).
fn check_listen(listen: std::net::SocketAddr) -> anyhow::Result<()> {
    if !listen.ip().is_loopback() {
        anyhow::bail!(
            "preview listens on loopback only (auth is off): use 127.0.0.1:<port> or [::1]:<port>, not {listen}"
        );
    }
    Ok(())
}

/// The embedder for preview: `--embedder`, else `FIRELIGHT_EMBEDDER`, else
/// `candle`; the other `FIRELIGHT_EMBED_*` variables apply as for `serve`.
/// Models are kept in `$FIRELIGHT_DATA_DIR` or the user cache folder, so a
/// preview downloads a model once.
fn embed_config(flag: Option<&str>) -> anyhow::Result<crate::search::EmbedConfig> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let backend = flag
        .map(str::to_string)
        .or_else(|| env("FIRELIGHT_EMBEDDER"))
        .unwrap_or_else(|| "candle".into());
    if !["candle", "openai-compatible", "off"].contains(&backend.as_str()) {
        anyhow::bail!("--embedder {backend:?}: use candle, openai-compatible or off");
    }
    let models_dir = env("FIRELIGHT_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| env("XDG_CACHE_HOME").map(|d| PathBuf::from(d).join("firelight")))
        .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".cache/firelight")))
        .unwrap_or_else(|| std::env::temp_dir().join("firelight"));
    Ok(crate::search::EmbedConfig {
        backend,
        model: env("FIRELIGHT_EMBED_MODEL")
            .unwrap_or_else(|| firelight_embed::model::DEFAULT_REPO.into()),
        revision: env("FIRELIGHT_EMBED_REVISION"),
        sha256: env("FIRELIGHT_EMBED_SHA256"),
        hf_endpoint: env("FIRELIGHT_EMBED_HF_ENDPOINT")
            .unwrap_or_else(|| "https://huggingface.co".into()),
        path: env("FIRELIGHT_EMBED_PATH").map(PathBuf::from),
        base_url: env("FIRELIGHT_EMBED_BASE_URL"),
        api_key: env("FIRELIGHT_EMBED_API_KEY"),
        models_dir,
    })
}

#[cfg(test)]
mod listen_tests {
    #[test]
    fn loopback_only() {
        for ok in ["127.0.0.1:8080", "[::1]:8080"] {
            assert!(
                super::check_listen(ok.parse().unwrap_or_else(|e| panic!("{e}"))).is_ok(),
                "{ok}"
            );
        }
        for bad in ["0.0.0.0:8080", "[::]:8080", "192.0.2.1:8080"] {
            assert!(
                super::check_listen(bad.parse().unwrap_or_else(|e| panic!("{e}"))).is_err(),
                "{bad}"
            );
        }
    }
}
