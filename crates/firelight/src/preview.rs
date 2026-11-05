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
    if let Some(e) = &args.embedder
        && e != "off"
    {
        tracing::info!(embedder = %e, "meaning search arrives with M5; previewing with word search");
    }
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
        let puller = Puller { git: Git::default(), data_dir: data, limits, clock: Arc::new(SystemClock) };
        let store = Store::memory().context("in-memory database")?;
        let hub = Hub::new(specs.clone(), puller, store, limits, true);
        hub.load_cache();
        for s in &specs {
            hub.pull(&s.id).await;
        }
        // Report what doesn't pass, once per change, like `check` would.
        let report = |hub: &Hub| {
            for st in hub.source_states() {
                if let Some(d) = st.newest_diagnostics.filter(|d| d != "[]") {
                    eprintln!("firelight preview: {} doesn't pass its checks, so the last good version shows: {d}", st.source_id);
                }
                if let Some(e) = st.last_pull_error {
                    eprintln!("firelight preview: {}: {e}", st.source_id);
                }
            }
        };
        report(&hub);
        // Live reload: re-read every folder each second.
        let watcher = hub.clone();
        let ids: Vec<String> = specs.iter().map(|s| s.id.clone()).collect();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_secs(1));
            loop {
                t.tick().await;
                let mut changed = false;
                for id in &ids {
                    changed |= watcher.pull(id).await;
                }
                if changed {
                    tracing::info!("reloaded");
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
        firelight_server::serve(
            args.listen,
            state,
            |addr| eprintln!("firelight preview: http://{addr}/ (Ctrl-C to stop)"),
            firelight_server::shutdown_signal(),
        )
        .await?;
        Ok(())
    })
}
