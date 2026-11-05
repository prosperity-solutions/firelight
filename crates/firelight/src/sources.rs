//! Startup for sources (§3): config → git check → database → cached catalog.

use std::sync::Arc;

use anyhow::Context as _;
use firelight_format::Limits;
use firelight_server::{Webhook, WebhookTarget};
use firelight_source::git::Git;
use firelight_source::github::{AppTokens, owner_repo};
use firelight_source::pull::{Credentials, Origin, Puller, SourceSpec};
use firelight_source::{SystemClock, http_client};
use firelight_store::Store;
use tokio::sync::mpsc;

use crate::config::{Config, SourceKind, TokenAuth};
use crate::hub::Hub;

pub struct Started {
    pub hub: Arc<Hub>,
    pub wake: mpsc::Receiver<String>,
    /// Held for the life of the server, so the wake channel stays open
    /// without webhooks.
    pub wake_tx: mpsc::Sender<String>,
    pub webhook: Option<Webhook>,
    /// Every source had a cached snapshot: ready before the first pull.
    pub warm: bool,
}

pub fn limits(cfg: &Config) -> Limits {
    Limits {
        max_files: cfg.max_files,
        max_source_bytes: cfg.max_source_bytes,
        max_file_bytes: cfg.max_file_bytes,
    }
}

/// Build the source specs from the config.
pub fn specs(cfg: &Config, app: Option<Arc<AppTokens>>) -> Vec<SourceSpec> {
    cfg.sources
        .iter()
        .map(|s| {
            let origin = match s.kind {
                SourceKind::Dir => Origin::Dir(s.path.clone().unwrap_or_default()),
                SourceKind::Git | SourceKind::Github => {
                    let url = s.url.as_ref().map(url::Url::to_string).unwrap_or_default();
                    let creds = match (&s.token_file, s.kind, &app) {
                        (Some(path), SourceKind::Github, _) => Credentials::TokenFile {
                            path: path.clone(),
                            basic: true,
                        },
                        (Some(path), _, _) => Credentials::TokenFile {
                            path: path.clone(),
                            basic: s.token_auth == TokenAuth::Basic,
                        },
                        (None, SourceKind::Github, Some(app)) => Credentials::App(app.clone()),
                        _ => Credentials::Anonymous,
                    };
                    Origin::Git {
                        url,
                        git_ref: s.git_ref.clone(),
                        creds,
                    }
                }
            };
            SourceSpec {
                id: s.id.clone(),
                origin,
            }
        })
        .collect()
}

pub async fn start(cfg: &Config) -> anyhow::Result<Started> {
    let git = Git {
        home: cfg.data_dir.join("git-home"),
        ..Git::default()
    };
    if cfg.sources.iter().any(|s| s.kind != SourceKind::Dir) {
        let v = git.check_version().await.context("git sources need git")?;
        tracing::info!(git = %v, "git found");
    }
    let app = match &cfg.github_app {
        Some((id, key)) => Some(Arc::new(
            AppTokens::new(
                *id,
                &key.0,
                "https://api.github.com",
                http_client(),
                Arc::new(SystemClock),
            )
            .context("github_app.private_key")?,
        )),
        None => None,
    };
    let specs = specs(cfg, app);
    let db = cfg.data_dir.join("firelight.db");
    let store = Store::open(&db).with_context(|| format!("opening {}", db.display()))?;
    let puller = Puller {
        git,
        data_dir: cfg.data_dir.clone(),
        limits: limits(cfg),
        clock: Arc::new(SystemClock),
    };
    let hub = Hub::new(specs, puller, store, limits(cfg), cfg.self_docs);
    let warm = hub.load_cache() && !cfg.sources.is_empty();
    let (tx, wake) = mpsc::channel(64);
    let webhook = cfg.webhook_secret.as_ref().map(|secret| Webhook {
        secret: secret.0.as_bytes().to_vec(),
        targets: cfg
            .sources
            .iter()
            .filter(|s| s.kind != SourceKind::Dir)
            .filter_map(|s| {
                let url = s.url.as_ref()?;
                if url.host_str() != Some("github.com") {
                    return None;
                }
                let (owner, repo) = owner_repo(url.as_str())?;
                Some(WebhookTarget {
                    source_id: s.id.clone(),
                    repo: format!("{owner}/{repo}").to_ascii_lowercase(),
                    branch: s.git_ref.clone(),
                })
            })
            .collect(),
        wake: tx.clone(),
    });
    Ok(Started {
        hub,
        wake,
        wake_tx: tx,
        webhook,
        warm,
    })
}
