//! Firelight: the docs your repos write about themselves.
//!
//! The binary: CLI, config loading, startup (docs/HANDOVER.md §3, §12.1).

mod check;
mod config;
mod covers;
mod hub;
mod preview;
mod search;
mod skills;
mod sources;
mod telemetry;

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::config::{Config, Inputs, Purpose};

/// The release version, injected by the release build (`FIRELIGHT_VERSION`);
/// a source build says 0.0.0.
pub const VERSION: &str = match option_env!("FIRELIGHT_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
pub const COMMIT: &str = match option_env!("FIRELIGHT_COMMIT") {
    Some(c) => c,
    None => "unknown",
};
/// `.firelight/` format versions this binary reads (§5).
pub const FORMAT_VERSIONS: &[u32] = firelight_format::FORMAT_VERSIONS;

#[derive(Parser)]
#[command(
    name = "firelight",
    about = "The docs your repos write about themselves.",
    disable_version_flag = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server.
    Serve,
    /// Serve local .firelight/ folders on 127.0.0.1 with live reload, auth off.
    Preview {
        /// Folders holding a .firelight/ (default: the current one).
        dirs: Vec<std::path::PathBuf>,
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: std::net::SocketAddr,
        /// The embedder for meaning search (`off` for speed).
        #[arg(long)]
        embedder: Option<String>,
        /// Build everything once, run the golden questions, report and exit
        /// (non-zero on any error or failing question). No server.
        #[arg(long)]
        check: bool,
    },
    /// Validate .firelight/ folders: no network, no model. Errors as path:line:col.
    Check {
        /// Folders holding a .firelight/ (default: the current one). Several are checked together, so refs between them resolve.
        dirs: Vec<std::path::PathBuf>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
        /// Unresolved cross-repo refs are errors, not warnings.
        #[arg(long)]
        strict: bool,
    },
    /// The skill topics (how to write and check .firelight/), or install the skill.
    Skills {
        /// A topic: format, writing, onboarding, ci, mcp, config.
        topic: Option<String>,
        /// List the topics as JSON.
        #[arg(long)]
        json: bool,
        /// With `install`: write into this repo (.agents/skills, .claude/skills), not your home.
        #[arg(long)]
        project: bool,
    },
    /// In a PR: entries whose `covers` paths changed while the entry didn't.
    Covers {
        /// The PR's base ref (e.g. origin/main).
        #[arg(long)]
        base: String,
        /// The repo (default: the current folder).
        dir: Option<std::path::PathBuf>,
        /// Fail (exit 1) instead of warning.
        #[arg(long)]
        strict: bool,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Write the JSON Schemas of every .firelight/ YAML file (default: print the file names).
    #[command(hide = true)]
    Schema {
        /// Folder to write schema/v1/*.json into.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Check or print the configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Version, commit, supported format versions and the default model.
    Version {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Run the startup validation and exit: 0 when fine, 2 with every problem.
    Check,
    /// Print the effective configuration, secrets as ••••, and where each value came from.
    Show,
    /// Print docs/config.md (generated from the key table).
    #[command(hide = true)]
    Reference,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Version { json } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "version": VERSION,
                        "commit": COMMIT,
                        "format_versions": FORMAT_VERSIONS,
                        "default_model": config::DEFAULT_MODEL,
                    })
                );
            } else {
                println!("firelight {VERSION} (commit {COMMIT})");
                println!(
                    "format versions: {}",
                    FORMAT_VERSIONS
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!("default model: {}", config::DEFAULT_MODEL);
            }
            ExitCode::SUCCESS
        }
        Command::Config { command } => match command {
            ConfigCommand::Reference => {
                print!("{}", config::reference_markdown());
                ExitCode::SUCCESS
            }
            ConfigCommand::Check => match load(Purpose::Check) {
                Ok(_) => {
                    println!("firelight: the configuration is fine.");
                    ExitCode::SUCCESS
                }
                Err(code) => code,
            },
            ConfigCommand::Show => match load(Purpose::Check) {
                Ok(c) => {
                    for (k, v, origin) in &c.shown {
                        println!("{k} = {v}    # {origin}");
                    }
                    ExitCode::SUCCESS
                }
                Err(code) => code,
            },
        },
        Command::Preview {
            dirs,
            listen,
            embedder,
            check,
        } => {
            let format = std::env::var("FIRELIGHT_LOG_FORMAT").unwrap_or_else(|_| "pretty".into());
            let _telemetry = init_logging_with(if check { "warn" } else { "info" }, &format);
            if check {
                return match preview::check(dirs, embedder.as_deref()) {
                    Ok(code) => code,
                    Err(e) => {
                        eprintln!("firelight preview --check: {e:#}");
                        ExitCode::from(2)
                    }
                };
            }
            match preview::run(preview::Args {
                dirs,
                listen,
                embedder,
            }) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("firelight preview: {e:#}");
                    ExitCode::from(1)
                }
            }
        }
        Command::Check { dirs, json, strict } => check::run(check::Args { dirs, json, strict }),
        Command::Skills {
            topic,
            json,
            project,
        } => match topic.as_deref() {
            Some("install") => skills::run_install(project),
            t => skills::show(t, json),
        },
        Command::Covers {
            base,
            dir,
            strict,
            json,
        } => covers::run(covers::Args {
            base,
            dir,
            strict,
            json,
        }),
        Command::Schema { out } => {
            let all = firelight_format::schema::all();
            match out {
                None => {
                    for (name, _) in &all {
                        println!("{name}");
                    }
                    ExitCode::SUCCESS
                }
                Some(dir) => {
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        eprintln!("firelight: cannot create {}: {e}", dir.display());
                        return ExitCode::from(1);
                    }
                    for (name, text) in all {
                        if let Err(e) = std::fs::write(dir.join(&name), text) {
                            eprintln!("firelight: cannot write {name}: {e}");
                            return ExitCode::from(1);
                        }
                    }
                    ExitCode::SUCCESS
                }
            }
        }
        Command::Serve => {
            let cfg = match load(Purpose::Serve) {
                Ok(c) => c,
                Err(code) => return code,
            };
            let _telemetry = init_logging(&cfg);
            match serve(cfg) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    tracing::error!(error = %format!("{e:#}"), "firelight stopped");
                    eprintln!("firelight: {e:#}");
                    ExitCode::from(1)
                }
            }
        }
    }
}

/// Load the config; on any problem print them all and return exit code 2.
fn load(purpose: Purpose) -> Result<Config, ExitCode> {
    config::load(&Inputs::from_process(), purpose).map_err(|problems| {
        eprintln!("{problems}");
        ExitCode::from(2)
    })
}

fn init_logging(cfg: &Config) -> telemetry::Guard {
    telemetry::init(&cfg.log_level, &cfg.log_format)
}

fn init_logging_with(level: &str, format: &str) -> telemetry::Guard {
    telemetry::init(level, format)
}

/// Who may ask what (§10), from the config.
fn access(cfg: &Config) -> firelight_server::auth::Access {
    use firelight_server::auth::{Access, host_name, is_loopback_name, origin};
    let public = cfg
        .public_url
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();
    let public_host = cfg
        .public_url
        .as_ref()
        .and_then(|u| u.host_str())
        .and_then(host_name);
    let mut hosts: Vec<String> = cfg
        .allowed_hosts
        .iter()
        .filter_map(|h| host_name(h))
        .collect();
    hosts.extend(public_host.clone());
    let mut origins: Vec<String> = cfg
        .allowed_origins
        .iter()
        .filter_map(|o| origin(o))
        .collect();
    if origins.is_empty() {
        origins.extend(origin(&public));
    }
    let a = Access {
        basic: (cfg.ui_auth == config::UiAuth::Basic)
            .then(|| cfg.basic.as_ref().map(|(u, p)| (u.clone(), p.0.clone())))
            .flatten(),
        tokens: None,
        check_host: true,
        hosts,
        loopback: public_host.as_deref().is_some_and(is_loopback_name),
        origins,
    };
    if cfg.mcp_auth == config::McpAuth::Token {
        a.with_tokens(cfg.mcp_tokens.iter().map(|t| t.0.clone()).collect())
    } else {
        a
    }
}

/// Version, start time and the effective config for the status (§16.1).
/// Secrets are already `••••` in `shown`; URLs lose their queries.
fn status_extra(cfg: &Config) -> serde_json::Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let config: Vec<serde_json::Value> = cfg
        .shown
        .iter()
        .map(|(k, v, o)| serde_json::json!({ "key": k, "value": hub::scrub_urls(v), "from": o.to_string() }))
        .collect();
    serde_json::json!({
        "version": VERSION, "commit": COMMIT,
        "started_at": firelight_source::format_rfc3339(now),
        "config": config,
    })
}

/// The embedder settings from the config.
fn embed_config(cfg: &Config) -> search::EmbedConfig {
    search::EmbedConfig {
        backend: cfg.embedder.clone(),
        model: cfg.embed_model.clone(),
        revision: cfg.embed_revision.clone(),
        sha256: cfg.embed_sha256.clone(),
        hf_endpoint: cfg
            .embed_hf_endpoint
            .as_str()
            .trim_end_matches('/')
            .to_string(),
        path: cfg.embed_path.clone(),
        base_url: cfg
            .embed_base_url
            .as_ref()
            .map(|u| u.as_str().trim_end_matches('/').to_string()),
        api_key: cfg.embed_api_key.as_ref().map(|k| k.0.clone()),
        models_dir: cfg.data_dir.clone(),
    }
}

fn serve(cfg: Config) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let res = rt.block_on(async move {
        std::fs::create_dir_all(&cfg.data_dir).map_err(|e| {
            anyhow::anyhow!("cannot create the data folder {}: {e}. Point FIRELIGHT_DATA_DIR at a writable folder.", cfg.data_dir.display())
        })?;
        tracing::info!(version = VERSION, commit = COMMIT, "firelight starting");
        let started = sources::start(&cfg).await?;
        let ui = firelight_server::pages::Ui {
            title: cfg.ui_title.clone(),
            default_space: cfg.ui_default_space.clone(),
            public_url: cfg.public_url.as_ref().map(|u| u.as_str().trim_end_matches('/').to_string()).unwrap_or_default(),
            version: VERSION.into(),
            mcp_token: cfg.mcp_auth == config::McpAuth::Token,
            live: false,
        };
        started.hub.search.record_queries.store(cfg.otel_record_queries, Ordering::Relaxed);
        if let Ok(mut x) = started.hub.status_extra.write() {
            *x = status_extra(&cfg);
        }
        let mut state = firelight_server::AppState::new()
            .with_content(started.hub.clone(), ui, &cfg.ui_accent)
            .with_access(access(&cfg));
        if let Some(w) = started.webhook {
            state = state.with_webhook(w);
        }
        // Ready = the catalog is loaded (warm cache or first round of pulls)
        // and the built-in golden check passed (§3, §9.4).
        let sources_ready = Arc::new(AtomicBool::new(false));
        let search = started.hub.search.clone();
        let update: Arc<dyn Fn() + Send + Sync> = {
            let (state, sources_ready, search) = (state.clone(), sources_ready.clone(), search.clone());
            Arc::new(move || {
                state.set_ready(sources_ready.load(Ordering::SeqCst) && search.golden_ok.load(Ordering::SeqCst));
            })
        };
        if started.warm {
            tracing::info!("serving the cached catalog while the first pulls run");
            sources_ready.store(true, Ordering::SeqCst);
        }
        let hub = started.hub.clone();
        let _wake_tx = started.wake_tx;
        let interval = cfg.poll_interval;
        let (after_pull, sr) = (update.clone(), sources_ready.clone());
        tokio::spawn(hub.run(interval, started.wake, move || {
            // A cold start is ready once every source was tried once (§3).
            sr.store(true, Ordering::SeqCst);
            after_pull();
        }));
        // The model loads after the server listens: /healthz answers during a
        // first download, /readyz says 503 until the golden check passes.
        let (fail_tx, fail_rx) = tokio::sync::oneshot::channel::<String>();
        let embed = embed_config(&cfg);
        let (self_docs, golden_file, limits) = (cfg.self_docs, cfg.golden_file.clone(), sources::limits(&cfg));
        let sources_wait = sources_ready.clone();
        tokio::spawn(async move {
            let mut wait = Duration::from_secs(5);
            let emb = loop {
                match search::load_embedder(&embed).await {
                    Ok(e) => break e,
                    // A remote endpoint may come up later; a model that can't
                    // be verified never will.
                    Err(e) if embed.backend == "openai-compatible" => {
                        tracing::warn!(error = %e, "the embeddings endpoint isn't answering; retrying");
                        search.set_problem(Some(e));
                        tokio::time::sleep(wait).await;
                        wait = (wait * 2).min(Duration::from_secs(300));
                    }
                    Err(e) => {
                        let _ = fail_tx.send(e);
                        return;
                    }
                }
            };
            if let Some(e) = emb {
                tracing::info!(embedder = %e.id(), dims = e.dims(), "embedder ready");
                search.start_embedder(e).await;
            }
            // A golden file names served repos: wait until they are loaded.
            if !self_docs && golden_file.is_some() {
                while !sources_wait.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
            let docs = self_docs.then(hub::self_docs_files);
            let state = search.run_builtin(docs, golden_file, limits).await;
            tracing::info!(result = ?state, "built-in golden check");
            update();
        });
        let server = firelight_server::serve(cfg.listen, state, move |addr| tracing::info!(%addr, "listening"), firelight_server::shutdown_signal());
        tokio::select! {
            r = server => r?,
            Ok(e) = fail_rx => anyhow::bail!("the embedder can't start: {e}"),
        }
        Ok(())
    });
    // Don't wait for an embedding batch to finish: telemetry still flushes,
    // and Kubernetes' grace period isn't spent on work a restart redoes.
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    res
}

#[cfg(test)]
mod redaction_tests {
    //! §17.1: secrets never reach the status (API, MCP, page) in any form.
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::*;

    const SECRETS: &[&str] = &[
        "pw-0123456789-secret",
        "tok-old-0123456789abcdef",
        "tok-new-0123456789abcdef",
        "hook-0123456789-secret",
        "sk-embed-0123456789",
        "qtok-0123456789",
        "userinfo-pass-0123",
    ];

    #[test]
    fn status_never_shows_a_secret() {
        let env: BTreeMap<String, String> = [
            ("FIRELIGHT_SERVER_PUBLIC_URL", "https://firelight.acme.example"),
            ("FIRELIGHT_AUTH_UI", "basic"),
            ("FIRELIGHT_AUTH_USER", "ops"),
            ("FIRELIGHT_AUTH_PASSWORD", "pw-0123456789-secret"),
            ("FIRELIGHT_AUTH_MCP", "token"),
            ("FIRELIGHT_AUTH_MCP_TOKENS", r#"["tok-old-0123456789abcdef","tok-new-0123456789abcdef"]"#),
            ("FIRELIGHT_WEBHOOK_SECRET", "hook-0123456789-secret"),
            ("FIRELIGHT_EMBEDDER", "openai-compatible"),
            ("FIRELIGHT_EMBED_BASE_URL", "https://llm.acme.example/v1?key=qtok-0123456789"),
            ("FIRELIGHT_EMBED_API_KEY", "sk-embed-0123456789"),
            ("FIRELIGHT_SOURCES", r#"[{"id":"infra","kind":"git","url":"https://git.acme.example/infra.git?private_token=qtok-0123456789"}]"#),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
        let inputs = config::Inputs {
            env,
            read: Box::new(|_p: &Path| {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "none"))
            }),
            exists: Box::new(|_p: &Path| false),
        };
        let cfg = match config::load(&inputs, Purpose::Serve) {
            Ok(c) => c,
            Err(p) => panic!("this config should load: {} problems", p.0.len()),
        };
        let extra = status_extra(&cfg).to_string();
        let store = firelight_store::Store::memory().unwrap_or_else(|e| panic!("{e}"));
        let puller = firelight_source::pull::Puller {
            git: firelight_source::git::Git::default(),
            data_dir: PathBuf::from("/nonexistent"),
            limits: firelight_format::Limits::default(),
            clock: Arc::new(firelight_source::SystemClock),
        };
        let specs = firelight_source::pull::SourceSpec {
            id: "infra".into(),
            origin: firelight_source::pull::Origin::Git {
                url: "https://u:userinfo-pass-0123@git.acme.example/infra.git?private_token=qtok-0123456789".into(),
                git_ref: None,
                creds: firelight_source::pull::Credentials::Anonymous,
            },
        };
        let hub = hub::Hub::new(
            vec![specs],
            puller,
            store,
            firelight_format::Limits::default(),
            true,
            "off",
        );
        hub.search.set_problem(Some(
            "POST https://u:userinfo-pass-0123@llm.acme.example/v1?key=qtok-0123456789: refused"
                .into(),
        ));
        let status = hub.status_json().to_string();
        for s in SECRETS {
            assert!(
                !extra.contains(s),
                "config in the status shows {s}: {extra}"
            );
            assert!(!status.contains(s), "the status shows {s}: {status}");
        }
        assert!(extra.contains("••••"), "secrets show as ••••: {extra}");
    }
}
