//! Firelight: the docs your repos write about themselves.
//!
//! The binary: CLI, config loading, startup (docs/HANDOVER.md §3, §12.1).

mod check;
mod config;
mod hub;
mod preview;
mod search;
mod sources;

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
        } => {
            let format = std::env::var("FIRELIGHT_LOG_FORMAT").unwrap_or_else(|_| "pretty".into());
            init_logging_with("info", &format);
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
            init_logging(&cfg);
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

fn init_logging(cfg: &Config) {
    init_logging_with(&cfg.log_level, &cfg.log_format);
}

fn init_logging_with(level: &str, format: &str) {
    let filter = tracing_subscriber::EnvFilter::new(format!("{level},hyper=warn"));
    let b = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stdout);
    if format == "pretty" {
        b.init();
    } else {
        b.json().flatten_event(true).with_current_span(false).init();
    }
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
    rt.block_on(async move {
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
        let mut state = firelight_server::AppState::new().with_content(started.hub.clone(), ui, &cfg.ui_accent);
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
                search.start_embedder(e);
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
    })
}
