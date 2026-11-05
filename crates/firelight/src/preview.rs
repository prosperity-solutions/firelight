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
        let puller = Puller {
            git: Git::default(),
            data_dir: data,
            limits,
            clock: Arc::new(SystemClock),
        };
        let store = Store::memory().context("in-memory database")?;
        let hub = Hub::new(specs.clone(), puller, store, limits, true);
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
