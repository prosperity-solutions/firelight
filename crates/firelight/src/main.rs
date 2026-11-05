//! Firelight: the docs your repos write about themselves.
//!
//! The binary: CLI, config loading, startup (docs/HANDOVER.md §3, §12.1).

mod check;
mod config;

use std::process::ExitCode;

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
    let filter = tracing_subscriber::EnvFilter::new(format!("{},hyper=warn", cfg.log_level));
    let b = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stdout);
    if cfg.log_format == "pretty" {
        b.init();
    } else {
        b.json().flatten_event(true).with_current_span(false).init();
    }
}

fn serve(cfg: Config) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        std::fs::create_dir_all(&cfg.data_dir).map_err(|e| {
            anyhow::anyhow!(
                "cannot create the data folder {}: {e}. Point FIRELIGHT_DATA_DIR at a writable folder.",
                cfg.data_dir.display()
            )
        })?;
        tracing::info!(version = VERSION, commit = COMMIT, "firelight starting");
        let state = firelight_server::AppState::new();
        let ready = state.clone();
        firelight_server::serve(
            cfg.listen,
            state,
            move |addr| {
                tracing::info!(%addr, "listening");
                // M1: nothing to load yet, so ready once listening. Later
                // milestones flip this after the catalog and golden check (§3).
                ready.set_ready(true);
            },
            firelight_server::shutdown_signal(),
        )
        .await?;
        Ok(())
    })
}
