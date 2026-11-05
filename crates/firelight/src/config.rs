//! Configuration (docs/HANDOVER.md §10).
//!
//! Built-in defaults, then a TOML file (`FIRELIGHT_CONFIG`, else
//! `/etc/firelight/firelight.toml` if present), then environment variables.
//! Every scalar key has an env var `FIRELIGHT_<SECTION>_<KEY>`; lists are TOML,
//! or JSON in the env var; secrets also accept `<NAME>_FILE`.
//!
//! Loading collects **every** problem before it gives up, and each problem
//! names the key, where the value came from, what is wrong and the fix. The
//! binary prints them all and exits 2.
//!
//! [`KEYS`] is the single table of keys: loading, validation, `config show`
//! and the generated `docs/config.md` all read it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/firelight/firelight.toml";
pub const DEFAULT_MODEL: &str = "Qwen/Qwen3-Embedding-0.6B";

/// What a key holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Str,
    Bool,
    /// A non-negative integer within bounds.
    Int {
        min: u64,
        max: u64,
    },
    /// `120s`, `5m`, `1h`, or plain seconds; at least `min` seconds.
    Duration {
        min: u64,
    },
    /// An `http(s)` URL without userinfo.
    Url,
    Addr,
    Path,
    Enum(&'static [&'static str]),
    /// A list of strings: a TOML array, or a JSON array in the env var.
    List,
    /// `#rrggbb`.
    Color,
    /// A 40-hex git commit.
    Commit,
    /// The `sources` list (§6).
    Sources,
}

/// One configuration key.
#[derive(Debug)]
pub struct Key {
    /// Dotted path, e.g. `server.public_url`.
    pub path: &'static str,
    pub kind: Kind,
    /// Default as it would be written in TOML; `None` = unset.
    pub default: Option<&'static str>,
    /// Never printed; also read from `<ENV>_FILE`.
    pub secret: bool,
    pub doc: &'static str,
}

impl Key {
    /// `server.public_url` → `FIRELIGHT_SERVER_PUBLIC_URL`.
    pub fn env(&self) -> String {
        format!(
            "FIRELIGHT_{}",
            self.path.replace('.', "_").to_ascii_uppercase()
        )
    }
}

const AUTH_HINT: &str = "Pick an auth mode. Behind a VPN or an auth proxy? Set FIRELIGHT_AUTH_UI=none and FIRELIGHT_AUTH_MCP=none.";

/// Every key Firelight reads. Order is the order of `docs/config.md`.
pub static KEYS: &[Key] = &[
    Key {
        path: "server.listen",
        kind: Kind::Addr,
        default: Some("127.0.0.1:8080"),
        secret: false,
        doc: "Address and port to listen on. The container image and chart use `0.0.0.0:8080`.",
    },
    Key {
        path: "server.public_url",
        kind: Kind::Url,
        default: None,
        secret: false,
        doc: "The URL people open, e.g. `https://firelight.acme.example`. **Required for `serve`.** Its host is the one the Host check accepts, and its origin the one the Origin check on `/mcp` accepts.",
    },
    Key {
        path: "server.allowed_hosts",
        kind: Kind::List,
        default: Some("[]"),
        secret: false,
        doc: "More `Host` header values to accept, e.g. the in-cluster Service name for agents running in the cluster.",
    },
    Key {
        path: "server.allowed_origins",
        kind: Kind::List,
        default: Some("[]"),
        secret: false,
        doc: "Browser origins allowed to call `/mcp`. Empty means the origin of `server.public_url` only.",
    },
    Key {
        path: "data.dir",
        kind: Kind::Path,
        default: Some("/var/lib/firelight"),
        secret: false,
        doc: "Where the database cache, the git clones and the models live. Deleting it loses nothing: git is the truth.",
    },
    Key {
        path: "sources",
        kind: Kind::Sources,
        default: Some("[]"),
        secret: false,
        doc: "The repos and folders to read, as `[[sources]]` tables (`id`, `kind` = `github` | `git` | `dir`, `url` or `path`, optional `ref`, `token_file`, `token_auth` = `bearer` | `basic`). `git` URLs are `https://`, `http://` or a local `file://`; `github` URLs are `https://github.com/<owner>/<repo>` (with a GitHub App, a token, or anonymous for public repos). In the env var: a JSON array of the same objects.",
    },
    Key {
        path: "poll.interval",
        kind: Kind::Duration { min: 10 },
        default: Some("120s"),
        secret: false,
        doc: "How often to ask each remote for its newest commit. Nothing is fetched when nothing changed.",
    },
    Key {
        path: "webhook.secret",
        kind: Kind::Str,
        default: None,
        secret: true,
        doc: "The GitHub webhook secret. Unset: `/webhook/github` answers 404.",
    },
    Key {
        path: "github_app.id",
        kind: Kind::Int {
            min: 1,
            max: u64::MAX,
        },
        default: None,
        secret: false,
        doc: "GitHub App id, for `github` sources. Needs only *Contents: read* and *Metadata: read*.",
    },
    Key {
        path: "github_app.private_key",
        kind: Kind::Str,
        default: None,
        secret: true,
        doc: "The GitHub App's private key (PEM).",
    },
    Key {
        path: "limits.max_files",
        kind: Kind::Int {
            min: 1,
            max: 1_000_000,
        },
        default: Some("1000"),
        secret: false,
        doc: "Most files read from one source's `.firelight/`.",
    },
    Key {
        path: "limits.max_source_bytes",
        kind: Kind::Int {
            min: 1024,
            max: 1 << 34,
        },
        default: Some("8388608"),
        secret: false,
        doc: "Most bytes read from one source's `.firelight/` (8 MiB).",
    },
    Key {
        path: "limits.max_file_bytes",
        kind: Kind::Int {
            min: 1024,
            max: 1 << 30,
        },
        default: Some("262144"),
        secret: false,
        doc: "Largest single file (256 KiB).",
    },
    Key {
        path: "embedder",
        kind: Kind::Enum(&["candle", "openai-compatible", "off"]),
        default: Some("candle"),
        secret: false,
        doc: "Meaning search backend: `candle` (in-process, CPU), `openai-compatible` (`POST {base}/embeddings`), or `off` (word search only, fully supported).",
    },
    Key {
        path: "embed.model",
        kind: Kind::Str,
        default: Some(DEFAULT_MODEL),
        secret: false,
        doc: "Hugging Face repo id (`candle`), or the model name sent to an `openai-compatible` endpoint.",
    },
    Key {
        path: "embed.revision",
        kind: Kind::Commit,
        default: None,
        secret: false,
        doc: "The model's 40-hex commit on Hugging Face. Branch names are refused. Unset with the default model: the revision pinned in the binary.",
    },
    Key {
        path: "embed.sha256",
        kind: Kind::Str,
        default: None,
        secret: false,
        doc: "Per-file checksums for a non-default model, as `file=hex` pairs separated by commas.",
    },
    Key {
        path: "embed.hf_endpoint",
        kind: Kind::Url,
        default: Some("https://huggingface.co"),
        secret: false,
        doc: "A Hugging Face mirror.",
    },
    Key {
        path: "embed.path",
        kind: Kind::Path,
        default: None,
        secret: false,
        doc: "A local model folder: no download at all, for air-gapped installs.",
    },
    Key {
        path: "embed.base_url",
        kind: Kind::Url,
        default: None,
        secret: false,
        doc: "Base URL of an `openai-compatible` endpoint, e.g. `http://ollama:11434/v1`.",
    },
    Key {
        path: "embed.api_key",
        kind: Kind::Str,
        default: None,
        secret: true,
        doc: "API key for the `openai-compatible` endpoint, if it needs one.",
    },
    Key {
        path: "self_docs",
        kind: Kind::Bool,
        default: Some("true"),
        secret: false,
        doc: "Serve Firelight's own docs (embedded in the binary) as the source `firelight`.",
    },
    Key {
        path: "golden_file",
        kind: Kind::Path,
        default: None,
        secret: false,
        doc: "A `golden.yaml` for the built-in readiness check when `self_docs` is off.",
    },
    Key {
        path: "auth.ui",
        kind: Kind::Enum(&["basic", "none"]),
        default: None,
        secret: false,
        doc: "Auth for the page, the API and the status page: `basic` or `none`. **Must be stated**; there is no default.",
    },
    Key {
        path: "auth.mcp",
        kind: Kind::Enum(&["token", "none"]),
        default: None,
        secret: false,
        doc: "Auth for `/mcp`: `token` (`Authorization: Bearer …`) or `none`. **Must be stated.** `ui = basic` with `mcp = none` is refused.",
    },
    Key {
        path: "auth.user",
        kind: Kind::Str,
        default: None,
        secret: false,
        doc: "Basic-auth user name, with `auth.ui = basic`.",
    },
    Key {
        path: "auth.password",
        kind: Kind::Str,
        default: None,
        secret: true,
        doc: "Basic-auth password, with `auth.ui = basic`.",
    },
    Key {
        path: "auth.mcp_tokens",
        kind: Kind::List,
        default: Some("[]"),
        secret: true,
        doc: "Accepted MCP bearer tokens, with `auth.mcp = token`. Several, for rotation. In a `_FILE`: one token per line.",
    },
    Key {
        path: "otel.record_queries",
        kind: Kind::Bool,
        default: Some("false"),
        secret: false,
        doc: "Add the search query text to search spans. Off: only its length and the result count are recorded. Tracing itself turns on with the standard `OTEL_EXPORTER_OTLP_ENDPOINT`.",
    },
    Key {
        path: "log.level",
        kind: Kind::Enum(&["error", "warn", "info", "debug", "trace"]),
        default: Some("info"),
        secret: false,
        doc: "Log level.",
    },
    Key {
        path: "log.format",
        kind: Kind::Enum(&["json", "pretty"]),
        default: Some("json"),
        secret: false,
        doc: "`json` lines on stdout, or `pretty` for humans.",
    },
    Key {
        path: "ui.title",
        kind: Kind::Str,
        default: Some("Firelight"),
        secret: false,
        doc: "The page title, e.g. `Firelight·acme`.",
    },
    Key {
        path: "ui.accent",
        kind: Kind::Color,
        default: Some("#d9541e"),
        secret: false,
        doc: "The page's accent colour, `#rrggbb`.",
    },
    Key {
        path: "ui.default_space",
        kind: Kind::Str,
        default: None,
        secret: false,
        doc: "Where `/` redirects. Unset: the space list (a single space: straight into it). A space that doesn't exist is a runtime warning, never a startup error.",
    },
];

/// Where a value came from.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    Default,
    File(PathBuf),
    Env(String),
    Unset,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::Default => write!(f, "default"),
            Origin::File(p) => write!(f, "file {}", p.display()),
            Origin::Env(n) => write!(f, "env {n}"),
            Origin::Unset => write!(f, "not set"),
        }
    }
}

/// One startup problem.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub key: String,
    pub origin: Origin,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.key, self.origin, self.message)
    }
}

/// Every problem found, in key order.
#[derive(Debug)]
pub struct Problems(pub Vec<Problem>);

impl fmt::Display for Problems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.0.len();
        writeln!(
            f,
            "firelight: refusing to start: {n} problem{} in the configuration.",
            if n == 1 { "" } else { "s" }
        )?;
        for p in &self.0 {
            writeln!(f, "  ✗ {p}")?;
        }
        write!(
            f,
            "Every key and its env var: https://github.com/prosperity-solutions/firelight/blob/main/docs/config.md"
        )
    }
}

impl std::error::Error for Problems {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceKind {
    Github,
    Git,
    Dir,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TokenAuth {
    Bearer,
    Basic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceConfig {
    pub id: String,
    pub kind: SourceKind,
    pub url: Option<url::Url>,
    pub path: Option<PathBuf>,
    pub git_ref: Option<String>,
    pub token_file: Option<PathBuf>,
    pub token_auth: TokenAuth,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiAuth {
    Basic,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum McpAuth {
    Token,
    None,
}

/// A secret: never printed.
#[derive(Clone, PartialEq)]
pub struct Secret(pub String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "••••")
    }
}

/// The effective, validated configuration.
#[derive(Debug, Clone)]
#[allow(
    dead_code,
    reason = "every key is validated now; the milestones that use them read them (docs/HANDOVER.md §22)"
)]
pub struct Config {
    pub listen: SocketAddr,
    pub public_url: Option<url::Url>,
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub data_dir: PathBuf,
    pub sources: Vec<SourceConfig>,
    pub poll_interval: Duration,
    pub webhook_secret: Option<Secret>,
    pub github_app: Option<(u64, Secret)>,
    pub max_files: u64,
    pub max_source_bytes: u64,
    pub max_file_bytes: u64,
    pub embedder: String,
    pub embed_model: String,
    pub embed_revision: Option<String>,
    pub embed_sha256: Option<String>,
    pub embed_hf_endpoint: url::Url,
    pub embed_path: Option<PathBuf>,
    pub embed_base_url: Option<url::Url>,
    pub embed_api_key: Option<Secret>,
    pub self_docs: bool,
    pub golden_file: Option<PathBuf>,
    pub ui_auth: UiAuth,
    pub mcp_auth: McpAuth,
    pub basic: Option<(String, Secret)>,
    pub mcp_tokens: Vec<Secret>,
    pub otel_record_queries: bool,
    pub log_level: String,
    pub log_format: String,
    pub ui_title: String,
    pub ui_accent: String,
    pub ui_default_space: Option<String>,
    /// Every key's effective value as shown by `config show`.
    pub shown: Vec<(String, String, Origin)>,
}

/// What the loader reads; injected so tests never touch the real
/// environment or file system layout.
/// Reads a file to a string.
pub type ReadFn = Box<dyn Fn(&Path) -> std::io::Result<String>>;

pub struct Inputs {
    pub env: BTreeMap<String, String>,
    /// Reads a file to a string (config file, `_FILE` secrets).
    pub read: ReadFn,
    pub exists: Box<dyn Fn(&Path) -> bool>,
}

impl Inputs {
    pub fn from_process() -> Self {
        Inputs {
            // vars_os: a non-UTF-8 variable must not panic before any message.
            env: std::env::vars_os()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.to_string_lossy().into_owned(),
                    )
                })
                .collect(),
            read: Box::new(|p| std::fs::read_to_string(p)),
            exists: Box::new(|p| p.exists()),
        }
    }
}

/// What the caller is about to do; `serve` needs more than `check`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Purpose {
    Serve,
    /// `config check` / `config show`: same rules as serve.
    Check,
    /// `preview`: local folders only, auth off, loopback.
    Preview,
}

/// Env vars that look like ours but aren't keys.
fn ignored_env(name: &str) -> bool {
    name == "FIRELIGHT_CONFIG"
        // Kubernetes service links for a Service named `firelight` (§10).
        || name == "FIRELIGHT_PORT"
        || name.starts_with("FIRELIGHT_PORT_")
        || name.starts_with("FIRELIGHT_SERVICE_")
        // The installer's own variables (§11.1).
        || name.starts_with("FIRELIGHT_INSTALL_")
        // Test and build helpers.
        || name.starts_with("FIRELIGHT_TEST_")
        || name == "FIRELIGHT_GUARD_GH"
        // Baked into release builds by scripts/ci/build-release.sh.
        || name == "FIRELIGHT_VERSION"
        || name == "FIRELIGHT_COMMIT"
}

struct Raw {
    value: Value,
    origin: Origin,
}

/// Load and validate. All problems come back at once.
pub fn load(inputs: &Inputs, purpose: Purpose) -> Result<Config, Problems> {
    let mut problems: Vec<Problem> = Vec::new();
    let mut raw: BTreeMap<&'static str, Raw> = BTreeMap::new();
    let prob = |problems: &mut Vec<Problem>, key: &str, origin: &Origin, msg: String| {
        problems.push(Problem {
            key: key.to_string(),
            origin: origin.clone(),
            message: msg,
        });
    };

    // 1. The TOML file.
    let (file, explicit) = match inputs.env.get("FIRELIGHT_CONFIG") {
        Some(p) if !p.is_empty() => (Some(PathBuf::from(p)), true),
        _ => {
            let p = PathBuf::from(DEFAULT_CONFIG_PATH);
            ((inputs.exists)(&p).then_some(p), false)
        }
    };
    if let Some(path) = &file {
        let origin = Origin::File(path.clone());
        match (inputs.read)(path) {
            Err(e) => prob(
                &mut problems,
                "FIRELIGHT_CONFIG",
                &if explicit {
                    Origin::Env("FIRELIGHT_CONFIG".into())
                } else {
                    Origin::Default
                },
                format!(
                    "cannot read the config file {}: {e}. Fix the path, or unset FIRELIGHT_CONFIG.",
                    path.display()
                ),
            ),
            Ok(text) => match toml::from_str::<toml::Table>(&text) {
                Err(e) => prob(
                    &mut problems,
                    "FIRELIGHT_CONFIG",
                    &origin,
                    toml_error(&text, &e),
                ),
                Ok(table) => {
                    let mut flat = Vec::new();
                    flatten("", &toml::Value::Table(table), &mut flat);
                    for (path, v) in flat {
                        match KEYS.iter().find(|k| k.path == path) {
                            // An empty string means "not set" (the chart
                            // renders unset values that way).
                            Some(_) if matches!(&v, toml::Value::String(x) if x.is_empty()) => {}
                            Some(k) => {
                                raw.insert(
                                    k.path,
                                    Raw {
                                        value: toml_to_json(&v),
                                        origin: origin.clone(),
                                    },
                                );
                            }
                            None => prob(
                                &mut problems,
                                &path,
                                &origin,
                                format!(
                                    "unknown key.{} Every key: docs/config.md.",
                                    suggest(&path)
                                ),
                            ),
                        }
                    }
                }
            },
        }
    }

    // 2. Environment variables override the file.
    for (name, value) in &inputs.env {
        if !name.starts_with("FIRELIGHT_") || ignored_env(name) {
            continue;
        }
        let origin = Origin::Env(name.clone());
        if let Some(k) = KEYS.iter().find(|k| k.env() == *name) {
            if value.is_empty() {
                continue; // an empty variable means "not set"
            }
            let v = parse_env(k, value);
            match v {
                Ok(v) => {
                    raw.insert(k.path, Raw { value: v, origin });
                }
                Err(e) => prob(&mut problems, k.path, &origin, e),
            }
            continue;
        }
        if let Some(k) = KEYS
            .iter()
            .find(|k| k.secret && format!("{}_FILE", k.env()) == *name)
        {
            if value.is_empty() {
                continue; // an empty variable means "not set"
            }
            match (inputs.read)(Path::new(value)) {
                Ok(s) => {
                    let v = if k.kind == Kind::List {
                        Value::Array(
                            s.lines()
                                .map(str::trim)
                                .filter(|l| !l.is_empty())
                                .map(|l| Value::String(l.into()))
                                .collect(),
                        )
                    } else {
                        Value::String(s.trim_end_matches(['\n', '\r']).to_string())
                    };
                    if raw
                        .get(k.path)
                        .is_some_and(|r| matches!(r.origin, Origin::Env(_)))
                    {
                        prob(
                            &mut problems,
                            k.path,
                            &origin,
                            format!("both {} and {name} are set. Keep one.", k.env()),
                        );
                    }
                    raw.insert(k.path, Raw { value: v, origin });
                }
                Err(e) => prob(
                    &mut problems,
                    k.path,
                    &origin,
                    format!(
                        "cannot read the file {value}: {e}. Fix the path or mount the secret there."
                    ),
                ),
            }
            continue;
        }
        prob(
            &mut problems,
            name,
            &origin,
            format!(
                "unknown environment variable.{} Every key and its env var: docs/config.md.",
                suggest_env(name)
            ),
        );
    }

    // 3. Typed values, with defaults.
    let mut b = Builder {
        raw: &raw,
        problems: Vec::new(),
        shown: Vec::new(),
    };
    let listen = b.addr("server.listen");
    let public_url = b.url("server.public_url");
    let allowed_hosts = b.list("server.allowed_hosts");
    let allowed_origins = b.list("server.allowed_origins");
    let data_dir = b
        .path("data.dir")
        .unwrap_or_else(|| PathBuf::from("/var/lib/firelight"));
    let sources = b.sources(inputs);
    let poll_interval = b.duration("poll.interval");
    let webhook_secret = b.string("webhook.secret").map(Secret);
    if webhook_secret
        .as_ref()
        .is_some_and(|w| w.0.trim().len() < 16)
    {
        prob(&mut problems, "webhook.secret", &origin_of_raw(&raw, "webhook.secret"), "needs at least 16 characters, or anyone could sign deliveries. Generate one with: openssl rand -hex 32".into());
    }
    let app_id = b.int("github_app.id");
    let app_key = b.string("github_app.private_key").map(Secret);
    let max_files = b.int("limits.max_files").unwrap_or(1000);
    let max_source_bytes = b.int("limits.max_source_bytes").unwrap_or(8 << 20);
    let max_file_bytes = b.int("limits.max_file_bytes").unwrap_or(256 << 10);
    let embedder = b.string("embedder").unwrap_or_else(|| "candle".into());
    let embed_model = b
        .string("embed.model")
        .unwrap_or_else(|| DEFAULT_MODEL.into());
    let embed_revision = b.commit("embed.revision");
    let embed_sha256 = b.string("embed.sha256");
    let embed_hf_endpoint = b.url("embed.hf_endpoint");
    let embed_path = b.path("embed.path");
    let embed_base_url = b.url("embed.base_url");
    let embed_api_key = b.string("embed.api_key").map(Secret);
    let self_docs = b.bool("self_docs").unwrap_or(true);
    let golden_file = b.path("golden_file");
    let ui = b.string("auth.ui");
    let mcp = b.string("auth.mcp");
    let user = b.string("auth.user");
    let password = b.string("auth.password").map(Secret);
    let mcp_tokens: Vec<Secret> = b.list("auth.mcp_tokens").into_iter().map(Secret).collect();
    let otel_record_queries = b.bool("otel.record_queries").unwrap_or(false);
    let log_level = b.string("log.level").unwrap_or_else(|| "info".into());
    let log_format = b.string("log.format").unwrap_or_else(|| "json".into());
    let ui_title = b.string("ui.title").unwrap_or_else(|| "Firelight".into());
    let ui_accent = b.color("ui.accent").unwrap_or_else(|| "#d9541e".into());
    let ui_default_space = b.string("ui.default_space");
    problems.append(&mut b.problems);
    let origin_of = |k: &str| raw.get(k).map_or(Origin::Unset, |r| r.origin.clone());

    // 4. Rules across keys.
    if purpose != Purpose::Preview && public_url.is_none() && !raw.contains_key("server.public_url")
    {
        {
            prob(
                &mut problems,
                "server.public_url",
                &origin_of("server.public_url"),
                "required for serve: the URL people open. Set FIRELIGHT_SERVER_PUBLIC_URL, e.g. https://firelight.acme.example".into(),
            );
        }
    }
    let ui_auth = match ui.as_deref() {
        Some("basic") => UiAuth::Basic,
        Some(_) => UiAuth::None,
        None => {
            if purpose != Purpose::Preview && !raw.contains_key("auth.ui") {
                prob(&mut problems, "auth.ui", &Origin::Unset, AUTH_HINT.into());
            }
            UiAuth::None
        }
    };
    let mcp_auth = match mcp.as_deref() {
        Some("token") => McpAuth::Token,
        Some(_) => McpAuth::None,
        None => {
            if purpose != Purpose::Preview && !raw.contains_key("auth.mcp") {
                prob(&mut problems, "auth.mcp", &Origin::Unset, AUTH_HINT.into());
            }
            McpAuth::None
        }
    };
    let basic = if ui_auth == UiAuth::Basic {
        if user.as_deref().is_none_or(str::is_empty) {
            prob(
                &mut problems,
                "auth.user",
                &origin_of("auth.user"),
                "required with auth.ui = basic. Set FIRELIGHT_AUTH_USER.".into(),
            );
        }
        if password.as_ref().is_none_or(|p| p.0.is_empty()) {
            prob(&mut problems, "auth.password", &origin_of("auth.password"), "required with auth.ui = basic. Set FIRELIGHT_AUTH_PASSWORD_FILE to a mounted secret (or FIRELIGHT_AUTH_PASSWORD).".into());
        }
        user.clone().zip(password.clone())
    } else {
        None
    };
    if ui_auth == UiAuth::Basic && mcp.as_deref() == Some("none") {
        prob(
            &mut problems,
            "auth.mcp",
            &origin_of("auth.mcp"),
            "none is refused with auth.ui = basic: MCP would expose everything the login protects. Set FIRELIGHT_AUTH_MCP=token and give it tokens.".into(),
        );
    }
    if mcp_auth == McpAuth::Token && mcp_tokens.iter().all(|t| t.0.trim().is_empty()) {
        prob(&mut problems, "auth.mcp_tokens", &origin_of("auth.mcp_tokens"), "required with auth.mcp = token: at least one token. Set FIRELIGHT_AUTH_MCP_TOKENS_FILE to a file with one token per line.".into());
    }
    if mcp_auth == McpAuth::Token
        && mcp_tokens
            .iter()
            .any(|t| !t.0.is_empty() && t.0.trim().len() < 16)
    {
        prob(
            &mut problems,
            "auth.mcp_tokens",
            &origin_of("auth.mcp_tokens"),
            "every token needs at least 16 characters. Generate one with: openssl rand -hex 32"
                .into(),
        );
    }
    let github_app = match (app_id, app_key) {
        (Some(id), Some(key)) => Some((id, key)),
        (None, None) => None,
        (Some(_), None) => {
            prob(&mut problems, "github_app.private_key", &Origin::Unset, "required with github_app.id. Set FIRELIGHT_GITHUB_APP_PRIVATE_KEY_FILE to the mounted PEM.".into());
            None
        }
        (None, Some(_)) => {
            prob(
                &mut problems,
                "github_app.id",
                &Origin::Unset,
                "required with github_app.private_key. Set FIRELIGHT_GITHUB_APP_ID.".into(),
            );
            None
        }
    };
    if embedder == "openai-compatible" && embed_base_url.is_none() {
        prob(&mut problems, "embed.base_url", &origin_of("embed.base_url"), "required with embedder = openai-compatible. Set FIRELIGHT_EMBED_BASE_URL, e.g. http://ollama:11434/v1".into());
    }
    if embedder == "candle"
        && embed_model != DEFAULT_MODEL
        && embed_revision.is_none()
        && embed_path.is_none()
    {
        prob(
            &mut problems,
            "embed.revision",
            &Origin::Unset,
            format!(
                "required for a model other than the default ({DEFAULT_MODEL}): the model's 40-hex commit on Hugging Face. Set FIRELIGHT_EMBED_REVISION."
            ),
        );
    }
    if let Some(p) = &embed_path
        && !(inputs.exists)(p)
    {
        prob(
            &mut problems,
            "embed.path",
            &origin_of("embed.path"),
            format!(
                "{} does not exist. Point it at a local model folder.",
                p.display()
            ),
        );
    }
    if let Some(p) = &golden_file
        && !(inputs.exists)(p)
    {
        prob(
            &mut problems,
            "golden_file",
            &origin_of("golden_file"),
            format!("{} does not exist.", p.display()),
        );
    }

    if !problems.is_empty() {
        return Err(Problems(problems));
    }
    let Some(listen) = listen else {
        // addr() reports a problem whenever it returns None; unreachable.
        return Err(Problems(vec![Problem {
            key: "server.listen".into(),
            origin: Origin::Default,
            message: "missing".into(),
        }]));
    };
    let Some(embed_hf_endpoint) = embed_hf_endpoint else {
        return Err(Problems(vec![Problem {
            key: "embed.hf_endpoint".into(),
            origin: Origin::Default,
            message: "missing".into(),
        }]));
    };
    Ok(Config {
        listen,
        public_url,
        allowed_hosts,
        allowed_origins,
        data_dir,
        sources,
        poll_interval: poll_interval.unwrap_or(Duration::from_secs(120)),
        webhook_secret,
        github_app,
        max_files,
        max_source_bytes,
        max_file_bytes,
        embedder,
        embed_model,
        embed_revision,
        embed_sha256,
        embed_hf_endpoint,
        embed_path,
        embed_base_url,
        embed_api_key,
        self_docs,
        golden_file,
        ui_auth,
        mcp_auth,
        basic,
        mcp_tokens,
        otel_record_queries,
        log_level,
        log_format,
        ui_title,
        ui_accent,
        ui_default_space,
        shown: b.shown,
    })
}

fn origin_of_raw(raw: &BTreeMap<&'static str, Raw>, k: &str) -> Origin {
    raw.get(k).map_or(Origin::Unset, |r| r.origin.clone())
}

/// Typed access to the raw values, collecting problems and `config show` rows.
struct Builder<'a> {
    raw: &'a BTreeMap<&'static str, Raw>,
    problems: Vec<Problem>,
    shown: Vec<(String, String, Origin)>,
}

impl Builder<'_> {
    fn key(path: &str) -> &'static Key {
        // Every call site names a key from KEYS; a typo fails the tests.
        KEYS.iter().find(|k| k.path == path).unwrap_or(&KEYS[0])
    }

    /// The raw value (or the parsed default) and its origin.
    fn get(&mut self, path: &'static str) -> Option<(Value, Origin)> {
        let k = Self::key(path);
        let out = match self.raw.get(path) {
            Some(r) => Some((r.value.clone(), r.origin.clone())),
            None => k.default.map(|d| (parse_default(k, d), Origin::Default)),
        };
        let shown = match &out {
            None => "(unset)".to_string(),
            Some(_) if k.secret => "••••".to_string(),
            Some((v, _)) => match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
        };
        let origin = out.as_ref().map_or(Origin::Unset, |o| o.1.clone());
        self.shown.push((path.to_string(), shown, origin));
        out
    }

    fn bad(&mut self, path: &str, origin: Origin, msg: String) {
        self.problems.push(Problem {
            key: path.into(),
            origin,
            message: msg,
        });
    }

    fn string(&mut self, path: &'static str) -> Option<String> {
        let (v, o) = self.get(path)?;
        let s = match v {
            Value::String(s) => s,
            other => {
                self.bad(
                    path,
                    o,
                    format!("expected a string, got {}.", type_name(&other)),
                );
                return None;
            }
        };
        if let Kind::Enum(allowed) = Self::key(path).kind
            && !allowed.contains(&s.as_str())
        {
            self.bad(
                path,
                o,
                format!("{s:?} is not one of {}.", allowed.join(", ")),
            );
            return None;
        }
        Some(s)
    }

    fn bool(&mut self, path: &'static str) -> Option<bool> {
        let (v, o) = self.get(path)?;
        match v {
            Value::Bool(b) => Some(b),
            other => {
                self.bad(
                    path,
                    o,
                    format!("expected true or false, got {}.", type_name(&other)),
                );
                None
            }
        }
    }

    fn int(&mut self, path: &'static str) -> Option<u64> {
        let (v, o) = self.get(path)?;
        let Kind::Int { min, max } = Self::key(path).kind else {
            return None;
        };
        match whole(&v) {
            Some(n) if (min..=max).contains(&n) => Some(n),
            Some(n) => {
                self.bad(path, o, format!("{n} is out of range ({min}..={max})."));
                None
            }
            None => {
                self.bad(
                    path,
                    o,
                    format!("expected a whole number, got {}.", type_name(&v)),
                );
                None
            }
        }
    }

    fn duration(&mut self, path: &'static str) -> Option<Duration> {
        let (v, o) = self.get(path)?;
        let Kind::Duration { min } = Self::key(path).kind else {
            return None;
        };
        let secs = match &v {
            Value::Number(_) => whole(&v),
            Value::String(s) => parse_duration(s),
            _ => None,
        };
        match secs {
            Some(s) if s >= min => Some(Duration::from_secs(s)),
            Some(s) => {
                self.bad(
                    path,
                    o,
                    format!("{s}s is too short; the minimum is {min}s."),
                );
                None
            }
            None => {
                self.bad(
                    path,
                    o,
                    format!("{v} is not a duration. Write e.g. \"120s\", \"5m\" or \"1h\"."),
                );
                None
            }
        }
    }

    fn url(&mut self, path: &'static str) -> Option<url::Url> {
        let s = self.string(path)?;
        let o = self
            .raw
            .get(path)
            .map_or(Origin::Default, |r| r.origin.clone());
        match check_url(&s) {
            Ok(u) => Some(u),
            Err(e) => {
                self.bad(path, o, e);
                None
            }
        }
    }

    fn addr(&mut self, path: &'static str) -> Option<SocketAddr> {
        let s = self.string(path)?;
        let o = self
            .raw
            .get(path)
            .map_or(Origin::Default, |r| r.origin.clone());
        match s.parse() {
            Ok(a) => Some(a),
            Err(_) => {
                self.bad(
                    path,
                    o,
                    format!("{s:?} is not an address and port. Write e.g. \"0.0.0.0:8080\"."),
                );
                None
            }
        }
    }

    fn path(&mut self, path: &'static str) -> Option<PathBuf> {
        self.string(path).map(PathBuf::from)
    }

    fn color(&mut self, path: &'static str) -> Option<String> {
        let s = self.string(path)?;
        let ok =
            s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !ok {
            let o = self
                .raw
                .get(path)
                .map_or(Origin::Default, |r| r.origin.clone());
            self.bad(
                path,
                o,
                format!("{s:?} is not a colour. Write it as #rrggbb, e.g. #d9541e."),
            );
            return None;
        }
        Some(s)
    }

    fn commit(&mut self, path: &'static str) -> Option<String> {
        let s = self.string(path)?;
        if s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(s.to_ascii_lowercase());
        }
        let o = self
            .raw
            .get(path)
            .map_or(Origin::Default, |r| r.origin.clone());
        self.bad(path, o, format!("{s:?} is not a 40-hex commit. Branch names and tags are refused, because they move: copy the commit from the model's \"Files and versions\" page."));
        None
    }

    fn list(&mut self, path: &'static str) -> Vec<String> {
        let Some((v, o)) = self.get(path) else {
            return vec![];
        };
        match v {
            Value::Array(items) => {
                let mut out = vec![];
                for i in items {
                    match i {
                        Value::String(s) => out.push(s),
                        other => self.bad(
                            path,
                            o.clone(),
                            format!("expected a list of strings, found {}.", type_name(&other)),
                        ),
                    }
                }
                out
            }
            other => {
                self.bad(
                    path,
                    o,
                    format!("expected a list, got {}.", type_name(&other)),
                );
                vec![]
            }
        }
    }

    fn sources(&mut self, inputs: &Inputs) -> Vec<SourceConfig> {
        let Some((v, o)) = self.get("sources") else {
            return vec![];
        };
        let Value::Array(items) = v else {
            self.bad(
                "sources",
                o,
                "expected a list of [[sources]] tables.".into(),
            );
            return vec![];
        };
        let mut out = vec![];
        let mut ids = BTreeSet::new();
        for (i, item) in items.iter().enumerate() {
            let key = format!("sources[{i}]");
            let Value::Object(m) = item else {
                self.bad(&key, o.clone(), "expected a table with id and kind.".into());
                continue;
            };
            for k in m.keys() {
                if ![
                    "id",
                    "kind",
                    "url",
                    "path",
                    "ref",
                    "token_file",
                    "token_auth",
                ]
                .contains(&k.as_str())
                {
                    self.bad(&format!("{key}.{k}"), o.clone(), "unknown key. A source has id, kind, url or path, ref, token_file, token_auth.".into());
                }
            }
            let s = |f: &str| m.get(f).and_then(Value::as_str).map(str::to_string);
            let id = s("id").unwrap_or_default();
            if !valid_id(&id) {
                self.bad(&format!("{key}.id"), o.clone(), format!("{id:?} is not a valid id: lowercase letters, digits and dashes, starting with a letter or digit, at most 63 characters."));
            }
            let kind = match s("kind").as_deref() {
                Some("github") => SourceKind::Github,
                Some("git") => SourceKind::Git,
                Some("dir") => SourceKind::Dir,
                other => {
                    self.bad(
                        &format!("{key}.kind"),
                        o.clone(),
                        format!("{other:?} is not a source kind: github, git or dir."),
                    );
                    continue;
                }
            };
            if id == "firelight" && kind != SourceKind::Dir {
                self.bad(&format!("{key}.id"), o.clone(), "firelight is reserved for Firelight's own docs. Only a dir source may use it (to replace the built-in self-docs).".into());
            }
            if !id.is_empty() && !ids.insert(id.clone()) {
                self.bad(
                    &format!("{key}.id"),
                    o.clone(),
                    format!("{id} is used by two sources. Every source needs its own id."),
                );
            }
            let mut url = None;
            let mut path = None;
            match kind {
                SourceKind::Dir => match s("path") {
                    Some(p) => {
                        if !(inputs.exists)(Path::new(&p)) {
                            self.bad(
                                &format!("{key}.path"),
                                o.clone(),
                                format!("{p} does not exist."),
                            );
                        }
                        path = Some(PathBuf::from(p));
                    }
                    None => self.bad(
                        &format!("{key}.path"),
                        o.clone(),
                        "required for a dir source.".into(),
                    ),
                },
                _ => match s("url") {
                    // A local repo over file:// (tests, preview, the Veld
                    // `serve` variant); git sources only.
                    Some(u) if kind == SourceKind::Git && u.starts_with("file://") => {
                        match url::Url::parse(&u) {
                            Ok(parsed) => url = Some(parsed),
                            Err(e) => self.bad(
                                &format!("{key}.url"),
                                o.clone(),
                                format!("{u:?} is not a file:// URL ({e})."),
                            ),
                        }
                    }
                    Some(u) => match check_url(&u) {
                        Ok(parsed) => {
                            if kind == SourceKind::Github
                                && (parsed.host_str() != Some("github.com")
                                    || parsed.scheme() != "https")
                            {
                                self.bad(&format!("{key}.url"), o.clone(), "a github source needs a https://github.com/<owner>/<repo> URL. Use kind = \"git\" for other hosts.".into());
                            }
                            // A token over plain http would travel in clear text.
                            let loopback = matches!(
                                parsed.host_str(),
                                Some("localhost" | "127.0.0.1" | "[::1]")
                            );
                            if parsed.scheme() == "http" && s("token_file").is_some() && !loopback {
                                self.bad(&format!("{key}.url"), o.clone(), "a token_file needs an https:// URL, or the token would travel in clear text.".into());
                            }
                            url = Some(parsed);
                        }
                        Err(e) => self.bad(&format!("{key}.url"), o.clone(), e),
                    },
                    None => self.bad(
                        &format!("{key}.url"),
                        o.clone(),
                        "required for a git or github source.".into(),
                    ),
                },
            }
            let token_file = s("token_file").map(PathBuf::from);
            if let Some(tf) = &token_file
                && (inputs.read)(tf).is_err()
            {
                self.bad(
                    &format!("{key}.token_file"),
                    o.clone(),
                    format!(
                        "cannot read {}. Mount the token there, or remove token_file.",
                        tf.display()
                    ),
                );
            }
            let token_auth = match s("token_auth").as_deref() {
                None | Some("bearer") => TokenAuth::Bearer,
                Some("basic") => TokenAuth::Basic,
                Some(other) => {
                    self.bad(
                        &format!("{key}.token_auth"),
                        o.clone(),
                        format!("{other:?} is not bearer or basic."),
                    );
                    TokenAuth::Bearer
                }
            };
            out.push(SourceConfig {
                id,
                kind,
                url,
                path,
                git_ref: s("ref"),
                token_file,
                token_auth,
            });
        }
        out
    }
}

/// Ids match `^[a-z0-9][a-z0-9-]{0,62}$` (§5).
pub fn valid_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// A TOML parse error without the offending source line, which may hold a
/// secret (§17.1): the message and the position only.
fn toml_error(text: &str, e: &toml::de::Error) -> String {
    match e.span() {
        Some(span) => {
            let before = &text[..span.start.min(text.len())];
            let line = before.matches('\n').count() + 1;
            let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
            format!(
                "not valid TOML at line {line}, column {col}: {}",
                e.message().trim()
            )
        }
        None => format!("not valid TOML: {}", e.message().trim()),
    }
}

fn check_url(s: &str) -> Result<url::Url, String> {
    const CREDENTIAL: &str = "a URL with a user name or password in it is refused, because it would end up in logs. Put the token in a file and set token_file (or the matching _FILE variable).";
    // An `@` before the path may be userinfo: never echo such a string, even
    // when it doesn't parse or has another scheme.
    let authority = s.split_once("://").map_or(s, |(_, rest)| rest);
    let authority = authority.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err(CREDENTIAL.into());
    }
    let u = url::Url::parse(s).map_err(|e| format!("{s:?} is not a URL ({e})."))?;
    if !matches!(u.scheme(), "https" | "http") {
        return Err(format!("{s:?} must be an http or https URL."));
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err(CREDENTIAL.into());
    }
    if u.host_str().is_none_or(str::is_empty) {
        return Err(format!("{s:?} has no host."));
    }
    Ok(u)
}

fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = if let Some(n) = s.strip_suffix("ms") {
        return n.trim().parse::<u64>().ok().map(|ms| ms / 1000);
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else {
        (s, 1)
    };
    num.trim()
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(mult))
}

fn parse_default(k: &Key, d: &str) -> Value {
    match k.kind {
        Kind::Bool => Value::Bool(d == "true"),
        Kind::Int { .. } => d.parse::<u64>().map(Value::from).unwrap_or(Value::Null),
        Kind::List | Kind::Sources => serde_json::from_str(d).unwrap_or(Value::Array(vec![])),
        _ => Value::String(d.to_string()),
    }
}

/// An env var's text as a typed value for its key.
fn parse_env(k: &Key, s: &str) -> Result<Value, String> {
    match k.kind {
        Kind::Bool => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(Value::Bool(true)),
            "false" | "0" | "no" => Ok(Value::Bool(false)),
            _ => Err(format!("{s:?} is not true or false.")),
        },
        Kind::Int { .. } => s
            .trim()
            .parse::<u64>()
            .map(Value::from)
            .map_err(|_| format!("{s:?} is not a whole number.")),
        Kind::List | Kind::Sources => match serde_json::from_str::<Value>(s) {
            Ok(v @ Value::Array(_)) => Ok(v),
            _ => Err(format!(
                "expected a JSON array in {}, e.g. {}",
                k.env(),
                if k.kind == Kind::Sources {
                    r#"[{"id":"app-repo","kind":"github","url":"https://github.com/acme/app-repo"}]"#
                } else {
                    r#"["one","two"]"#
                }
            )),
        },
        _ => Ok(Value::String(s.to_string())),
    }
}

fn flatten(prefix: &str, v: &toml::Value, out: &mut Vec<(String, toml::Value)>) {
    match v {
        toml::Value::Table(t)
            if t.is_empty()
                && !prefix.is_empty()
                && !KEYS
                    .iter()
                    .any(|k| k.path.starts_with(&format!("{prefix}.")) || k.path == prefix) =>
        {
            out.push((prefix.to_string(), v.clone()));
        }
        toml::Value::Table(t) if prefix.is_empty() || !KEYS.iter().any(|k| k.path == prefix) => {
            for (k, v) in t {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&p, v, out);
            }
        }
        other => out.push((prefix.to_string(), other.clone())),
    }
}

fn toml_to_json(v: &toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => {
            serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number)
        }
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect(),
        ),
    }
}

/// A non-negative whole number, also written as a float without a fraction
/// (Helm's `toToml` turns every number in values.yaml into one, e.g. `1000.0`).
fn whole(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| {
        v.as_f64()
            .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f <= 9.007_199_254_740_992e15)
            .map(|f| f as u64)
    })
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "nothing",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "a table",
    }
}

fn suggest(path: &str) -> String {
    closest(path, KEYS.iter().map(|k| k.path.to_string()))
        .map(|k| format!(" Did you mean {k}?"))
        .unwrap_or_default()
}

fn suggest_env(name: &str) -> String {
    closest(name, KEYS.iter().map(Key::env))
        .map(|k| format!(" Did you mean {k}?"))
        .unwrap_or_default()
}

fn closest(word: &str, candidates: impl Iterator<Item = String>) -> Option<String> {
    candidates
        .map(|c| (levenshtein(word, &c), c))
        .filter(|(d, c)| *d <= (c.len() / 4).max(2))
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// `docs/config.md`, generated from [`KEYS`].
pub fn reference_markdown() -> String {
    let mut s = String::from(
        "# Configuration reference\n\n\
         <!-- Generated by `firelight config reference` from crates/firelight/src/config.rs. Do not edit: run `just schema`. -->\n\n\
         Firelight reads built-in defaults, then a TOML file (`FIRELIGHT_CONFIG`, else `/etc/firelight/firelight.toml` if it exists), then environment variables. \
         Later wins. Every key has an environment variable; lists take a JSON array there. \
         Secrets also accept `<VARIABLE>_FILE`: a path to read the value from, e.g. a mounted Kubernetes secret.\n\n\
         Firelight refuses to start when anything is wrong, lists every problem at once with the fix, and exits with code 2. \
         `firelight config check` runs the same checks; `firelight config show` prints the effective configuration with secrets as `••••`.\n\n\
         | key | environment variable | default | |\n|---|---|---|---|\n",
    );
    for k in KEYS {
        let env = if k.secret {
            format!("`{0}` or `{0}_FILE`", k.env())
        } else {
            format!("`{}`", k.env())
        };
        let default = k.default.map_or("—".to_string(), |d| format!("`{d}`"));
        s.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            k.path,
            env,
            default,
            k.doc.replace('|', "\\|")
        ));
    }
    s.push_str(
        "\n## Auth must be stated\n\n\
         There is no default auth mode. Set `auth.ui` (`basic` or `none`) and `auth.mcp` (`token` or `none`). \
         Behind a VPN or an auth proxy, `none` for both is fine. `ui = basic` with `mcp = none` is refused, \
         because MCP would expose everything the login protects.\n\n\
         ## Ignored variables\n\n\
         `FIRELIGHT_CONFIG` names the file. Variables Kubernetes injects for a Service named `firelight` \
         (`FIRELIGHT_PORT…`, `FIRELIGHT_SERVICE_…`) and the installer's `FIRELIGHT_INSTALL_…` are ignored. \
         Any other unknown `FIRELIGHT_…` variable is an error, so a typo never passes silently.\n",
    );
    s
}

#[cfg(test)]
mod tests;
