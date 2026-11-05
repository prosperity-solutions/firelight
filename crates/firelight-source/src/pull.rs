//! One pull of one source (§6.2): ask the remote for the ref's commit; if it
//! changed, fetch it, read `.firelight/`, check it, and either make it the
//! served snapshot (no errors) or keep the last good one and remember why the
//! newest isn't served. "Maybe stale" inputs come from git history (§9.5).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use firelight_format::{Diagnostic, InputFile, Limits, Repo, parse_repo};
use firelight_store::{EntryHistory, Snapshot};

use crate::Clock;
use crate::git::{Auth, Git, GitError, mirror_dir};
use crate::github::{AppTokens, owner_repo};

/// How a git source authenticates.
#[derive(Debug, Clone)]
pub enum Credentials {
    Anonymous,
    /// A token read from a file at each pull (rotation without a restart).
    TokenFile {
        path: PathBuf,
        basic: bool,
    },
    /// A GitHub App installation token.
    App(Arc<AppTokens>),
}

/// What a source reads from.
#[derive(Debug, Clone)]
pub enum Origin {
    Dir(PathBuf),
    Git {
        url: String,
        git_ref: Option<String>,
        creds: Credentials,
    },
}

#[derive(Debug, Clone)]
pub struct SourceSpec {
    pub id: String,
    pub origin: Origin,
}

/// The result of a pull.
#[derive(Debug)]
pub enum Pulled {
    /// The remote still points at the commit already looked at.
    Unchanged,
    /// A commit (or folder state) that passed: serve it.
    Good {
        snapshot: Snapshot,
        repo: Box<Repo>,
        diagnostics: Vec<Diagnostic>,
    },
    /// A commit that failed validation: keep serving the last good one.
    Bad {
        commit: String,
        diagnostics: Vec<Diagnostic>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PullError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    App(#[from] crate::github::AppError),
    #[error("cannot read the token file {path}: {source}")]
    TokenFile {
        path: String,
        source: std::io::Error,
    },
    #[error(transparent)]
    Dir(#[from] crate::dir::DirError),
}

pub struct Puller {
    pub git: Git,
    pub data_dir: PathBuf,
    pub limits: Limits,
    pub clock: Arc<dyn Clock>,
}

async fn auth_for(url: &str, creds: &Credentials) -> Result<Auth, PullError> {
    Ok(match creds {
        Credentials::Anonymous => Auth::None,
        Credentials::TokenFile { path, basic } => {
            let token = std::fs::read_to_string(path).map_err(|source| PullError::TokenFile {
                path: path.display().to_string(),
                source,
            })?;
            let token = token.trim().to_string();
            if *basic {
                Auth::Basic {
                    user: "x-access-token".into(),
                    token,
                }
            } else {
                Auth::Bearer(token)
            }
        }
        Credentials::App(app) => {
            let (owner, repo) = owner_repo(url).ok_or_else(|| crate::github::AppError::Api {
                what: "url".into(),
                message: format!("{url} is not a GitHub repo URL"),
            })?;
            // GitHub's git endpoint takes installation tokens as basic auth.
            Auth::Basic {
                user: "x-access-token".into(),
                token: app.token_for(&owner, &repo).await?,
            }
        }
    })
}

impl Puller {
    /// Pull `spec`. `last_seen` is the newest commit already looked at (served
    /// or rejected); `None` forces a full read (a cold start).
    pub async fn pull(
        &self,
        spec: &SourceSpec,
        last_seen: Option<&str>,
    ) -> Result<Pulled, PullError> {
        match &spec.origin {
            Origin::Dir(path) => {
                let files = crate::dir::read_firelight_dir(path, &self.limits)?;
                // A folder has no commits: its "commit" is a content hash, so
                // an unchanged folder is Unchanged.
                let commit = content_id(&files);
                if last_seen == Some(commit.as_str()) {
                    return Ok(Pulled::Unchanged);
                }
                let now = crate::format_rfc3339(self.clock.now_unix());
                Ok(self
                    .judge(&spec.id, files, commit, Some(now), |_| Vec::new())
                    .await)
            }
            Origin::Git {
                url,
                git_ref,
                creds,
            } => {
                let _ = std::fs::create_dir_all(&self.git.home);
                let result = self
                    .pull_git(&spec.id, url, git_ref.as_deref(), creds, last_seen)
                    .await;
                if let (Err(PullError::Git(_)), Credentials::App(app)) = (&result, creds)
                    && let Some((owner, _)) = owner_repo(url)
                {
                    // A refused token may be revoked: get a new one next time.
                    app.invalidate(&owner).await;
                }
                result
            }
        }
    }

    async fn pull_git(
        &self,
        id: &str,
        url: &str,
        git_ref: Option<&str>,
        creds: &Credentials,
        last_seen: Option<&str>,
    ) -> Result<Pulled, PullError> {
        let auth = auth_for(url, creds).await?;
        let commit = self.git.ls_remote(url, git_ref, &auth).await?;
        if last_seen == Some(commit.as_str()) {
            return Ok(Pulled::Unchanged);
        }
        let dir = mirror_dir(&self.data_dir, id);
        self.git.ensure_mirror(&dir, url).await?;
        self.git.fetch(&dir, &commit, &auth).await?;
        let files = self
            .git
            .read_firelight(&dir, &commit, &auth, &self.limits)
            .await?;
        let committed_at = self.git.commit_time(&dir, &commit).await.ok();
        let (repo, diags) = parse_repo(&files, Some(id), &self.limits);
        let Some(repo) = repo.filter(|_| !diags.iter().any(Diagnostic::is_error)) else {
            return Ok(Pulled::Bad {
                commit,
                diagnostics: diags,
            });
        };
        let history = self.history(&dir, &commit, &repo).await;
        Ok(Pulled::Good {
            snapshot: Snapshot {
                commit,
                committed_at,
                files,
                history,
            },
            repo: Box::new(repo),
            diagnostics: diags,
        })
    }

    async fn judge<F>(
        &self,
        id: &str,
        files: Vec<InputFile>,
        commit: String,
        committed_at: Option<String>,
        history: F,
    ) -> Pulled
    where
        F: FnOnce(&Repo) -> Vec<EntryHistory>,
    {
        let (repo, diags) = parse_repo(&files, Some(id), &self.limits);
        match repo {
            Some(r) if !diags.iter().any(Diagnostic::is_error) => {
                let h = history(&r);
                Pulled::Good {
                    snapshot: Snapshot {
                        commit,
                        committed_at,
                        files,
                        history: h,
                    },
                    repo: Box::new(r),
                    diagnostics: diags,
                }
            }
            _ => Pulled::Bad {
                commit,
                diagnostics: diags,
            },
        }
    }

    /// For every entry with `covers`: when the entry file and when its covered
    /// paths last changed, from history only (§9.5).
    async fn history(&self, dir: &Path, commit: &str, repo: &Repo) -> Vec<EntryHistory> {
        let mut out = Vec::new();
        for e in repo.entries.iter().filter(|e| !e.covers.is_empty()) {
            let entry_changed = self
                .git
                .last_change(dir, commit, std::slice::from_ref(&e.path))
                .await
                .ok()
                .flatten();
            let covers_changed = self
                .git
                .last_change(dir, commit, &e.covers)
                .await
                .ok()
                .flatten();
            out.push(EntryHistory {
                entry_path: e.path.clone(),
                entry_changed,
                covers_changed,
            });
        }
        out
    }
}

/// A stable id for a folder's contents (FNV-1a over paths, kinds and bytes),
/// so polling a `dir` source can say "unchanged".
pub fn content_id(files: &[InputFile]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: &[u8]| {
        for x in b {
            h ^= u64::from(*x);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for f in files {
        eat(f.path.as_bytes());
        eat(&[0, f.kind as u8]);
        eat(&f.size.to_le_bytes());
        if let Some(c) = &f.contents {
            eat(c);
        }
    }
    format!("dir-{h:016x}")
}

/// Is an entry maybe stale (§9.5)? Covered paths changed after the entry did.
pub fn is_stale(h: &EntryHistory) -> bool {
    match (&h.entry_changed, &h.covers_changed) {
        (Some(e), Some(c)) => match (crate::parse_rfc3339(e), crate::parse_rfc3339(c)) {
            (Some(e), Some(c)) => c > e,
            _ => false,
        },
        _ => false,
    }
}
