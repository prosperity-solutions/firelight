//! Git through the `git` CLI (D1: `gix` has no partial clone, sparse checkout
//! or bearer-header auth yet).
//!
//! Each source has a **bare, blobless mirror** in the data folder: full commit
//! history and trees (for `covers`, §9.5), blobs fetched on demand, and only
//! `.firelight/` blobs are ever read. Nothing is checked out: files are read
//! straight from the object store (`ls-tree` + `cat-file`), so a symlink in
//! the tree is seen as mode 120000 and never followed, and no path outside
//! `.firelight/` is touched.
//!
//! Every git process runs with a fixed, safe environment: no system or global
//! config, no credential helpers or prompts, no hooks, only the https, http
//! and file protocols, and the auth header passed through `GIT_CONFIG_*`
//! environment variables, never in argv (where `ps` would show it).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use firelight_format::repo::FileKind;
use firelight_format::{InputFile, Limits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

/// Git 2.40 or newer (§6.2).
pub const MIN_GIT: (u32, u32) = (2, 40);

/// The ref Firelight fetches into, inside its bare mirror.
const HEAD_REF: &str = "refs/firelight/head";

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error(
        "git is not installed or not on PATH ({0}); Firelight needs git 2.40 or newer for git and github sources"
    )]
    Missing(String),
    #[error("git {found} is too old; Firelight needs 2.40 or newer")]
    TooOld { found: String },
    #[error("git {what} failed: {message}")]
    Failed { what: String, message: String },
    #[error("git {what} took longer than {secs} s")]
    Timeout { what: String, secs: u64 },
    #[error("the remote has no ref {0}")]
    NoRef(String),
    #[error("git: {0}")]
    Io(#[from] std::io::Error),
}

/// How to authenticate to the remote.
#[derive(Clone, Default)]
pub enum Auth {
    #[default]
    None,
    /// `Authorization: Bearer <token>`.
    Bearer(String),
    /// `Authorization: Basic base64(<user>:<token>)`; GitHub uses `x-access-token`.
    Basic { user: String, token: String },
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Auth::None => "None",
            Auth::Bearer(_) => "Bearer(••••)",
            Auth::Basic { .. } => "Basic(••••)",
        })
    }
}

impl Auth {
    fn header(&self) -> Option<String> {
        use base64::Engine as _;
        match self {
            Auth::None => None,
            Auth::Bearer(t) => Some(format!("Authorization: Bearer {}", t.trim())),
            Auth::Basic { user, token } => Some(format!(
                "Authorization: Basic {}",
                base64::engine::general_purpose::STANDARD
                    .encode(format!("{user}:{}", token.trim()))
            )),
        }
    }
}

/// The git binary and the time limits for its calls.
#[derive(Debug, Clone)]
pub struct Git {
    pub bin: PathBuf,
    pub timeout: Duration,
    /// git's HOME: an empty folder Firelight owns, so no `.netrc` or user
    /// config from a shared folder reaches git or libcurl.
    pub home: PathBuf,
}

impl Default for Git {
    fn default() -> Self {
        Git {
            bin: PathBuf::from("git"),
            timeout: Duration::from_secs(300),
            home: std::env::temp_dir().join("firelight-git-home"),
        }
    }
}

/// Removes anything that looks like a credential from git's messages before
/// they reach logs or the status page (§17.1).
fn redact(s: &str) -> String {
    s.lines()
        .map(|l| {
            let lower = l.to_ascii_lowercase();
            if lower.contains("authorization") {
                return "[a line about authorization, redacted]".to_string();
            }
            // https://user:secret@host → https://••••@host
            let mut out = String::new();
            let mut rest = l;
            while let Some(i) = rest.find("://") {
                out.push_str(&rest[..i + 3]);
                rest = &rest[i + 3..];
                let end = rest.find(['/', ' ', '\'', '"']).unwrap_or(rest.len());
                if let Some(at) = rest[..end].rfind('@') {
                    out.push_str("••••@");
                    rest = &rest[at + 1..];
                }
            }
            out.push_str(rest);
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Git {
    fn cmd(&self, dir: Option<&Path>, auth: &Auth) -> Command {
        let mut c = Command::new(&self.bin);
        if let Some(d) = dir {
            c.arg("--git-dir").arg(d);
        }
        c.args([
            "-c",
            "credential.helper=",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.askPass=",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "protocol.http.allow=always",
            "-c",
            "protocol.file.allow=always",
            "-c",
            "http.lowSpeedLimit=1000",
            "-c",
            "http.lowSpeedTime=60",
            "-c",
            "fetch.recurseSubmodules=false",
            "-c",
            "advice.detachedHead=false",
        ]);
        c.env_clear();
        for k in [
            "PATH",
            "TMPDIR",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "NO_PROXY",
            "https_proxy",
            "http_proxy",
            "no_proxy",
        ] {
            if let Some(v) = std::env::var_os(k) {
                c.env(k, v);
            }
        }
        c.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .env("GCM_INTERACTIVE", "never")
            .env("HOME", &self.home)
            .env("LC_ALL", "C");
        if let Some(h) = auth.header() {
            c.env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "http.extraHeader")
                .env("GIT_CONFIG_VALUE_0", h);
        }
        c.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        c
    }

    async fn run(&self, what: &str, mut c: Command) -> Result<Vec<u8>, GitError> {
        let out = match tokio::time::timeout(self.timeout, c.output()).await {
            Err(_) => {
                return Err(GitError::Timeout {
                    what: what.into(),
                    secs: self.timeout.as_secs(),
                });
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(GitError::Missing(e.to_string()));
            }
            Ok(r) => r?,
        };
        if !out.status.success() {
            let msg = redact(String::from_utf8_lossy(&out.stderr).trim());
            return Err(GitError::Failed {
                what: what.into(),
                message: if msg.is_empty() {
                    out.status.to_string()
                } else {
                    msg
                },
            });
        }
        Ok(out.stdout)
    }

    /// `git --version`, checked against [`MIN_GIT`].
    pub async fn check_version(&self) -> Result<String, GitError> {
        let mut c = Command::new(&self.bin);
        c.arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let out = self.run("--version", c).await?;
        let s = String::from_utf8_lossy(&out).trim().to_string();
        let v = s.strip_prefix("git version ").unwrap_or(&s);
        let mut parts = v
            .split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .map(|p| p.parse::<u32>().unwrap_or(0));
        let found = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        if found < MIN_GIT {
            return Err(GitError::TooOld {
                found: v.to_string(),
            });
        }
        Ok(v.to_string())
    }

    /// The commit a remote ref points at, without fetching anything.
    /// `git_ref` None means the remote's default branch (HEAD).
    pub async fn ls_remote(
        &self,
        url: &str,
        git_ref: Option<&str>,
        auth: &Auth,
    ) -> Result<String, GitError> {
        let r = git_ref.unwrap_or("HEAD");
        let mut c = self.cmd(None, auth);
        c.args(["ls-remote", "--", url, r]);
        let out = self.run("ls-remote", c).await?;
        let text = String::from_utf8_lossy(&out);
        // Exact matches first: a branch, a tag (peeled), then HEAD.
        let candidates: Vec<(&str, &str)> =
            text.lines().filter_map(|l| l.split_once('\t')).collect();
        let want = |name: &str| {
            candidates
                .iter()
                .find(|(_, n)| *n == name)
                .map(|(h, _)| h.to_string())
        };
        let found = if r == "HEAD" {
            want("HEAD")
        } else {
            want(&format!("refs/heads/{r}"))
                .or_else(|| want(&format!("refs/tags/{r}^{{}}")))
                .or_else(|| want(&format!("refs/tags/{r}")))
                .or_else(|| want(r))
        };
        found
            .filter(|h| h.len() >= 40 && h.chars().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| GitError::NoRef(r.to_string()))
    }

    /// Create the bare blobless mirror if it doesn't exist; keep its URL current.
    pub async fn ensure_mirror(&self, dir: &Path, url: &str) -> Result<(), GitError> {
        if !dir.join("HEAD").exists() {
            std::fs::create_dir_all(dir)?;
            let mut c = self.cmd(None, &Auth::None);
            c.arg("init").arg("--bare").arg("--quiet").arg(dir);
            self.run("init", c).await?;
        }
        for (k, v) in [
            ("remote.origin.url", url),
            ("remote.origin.promisor", "true"),
            ("remote.origin.partialclonefilter", "blob:none"),
            ("extensions.partialClone", "origin"),
            ("core.repositoryformatversion", "1"),
            ("gc.auto", "0"),
        ] {
            let mut c = self.cmd(Some(dir), &Auth::None);
            c.args(["config", k, v]);
            self.run("config", c).await?;
        }
        Ok(())
    }

    /// Fetch `commit` (commits and trees, full history; blobs later, on demand).
    pub async fn fetch(&self, dir: &Path, commit: &str, auth: &Auth) -> Result<(), GitError> {
        let mut c = self.cmd(Some(dir), auth);
        c.args([
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--filter=blob:none",
            "origin",
        ])
        .arg(format!("+{commit}:{HEAD_REF}"));
        self.run("fetch", c).await?;
        // Changed-path Bloom filters make `covers` lookups cheap; best effort.
        let mut g = self.cmd(Some(dir), &Auth::None);
        g.args(["commit-graph", "write", "--reachable", "--changed-paths"]);
        if let Err(e) = self.run("commit-graph", g).await {
            tracing::debug!(error = %e, "commit-graph write failed; covers lookups are just slower");
        }
        Ok(())
    }

    /// The commit time of `commit` (RFC 3339).
    pub async fn commit_time(&self, dir: &Path, commit: &str) -> Result<String, GitError> {
        let mut c = self.cmd(Some(dir), &Auth::None);
        c.args(["show", "-s", "--format=%cI", commit, "--"]);
        Ok(String::from_utf8_lossy(&self.run("show", c).await?)
            .trim()
            .to_string())
    }

    /// Every file under `.firelight/` at `commit`, read from the object store
    /// within `limits`. The listing needs only trees; the `.firelight/` blobs
    /// are then fetched in one batch (not one lazy fetch per file), and only
    /// those within the size limits are read. Like the folder reader, it skips
    /// `.DS_Store` and folders deeper than [`crate::dir::MAX_DEPTH`].
    pub async fn read_firelight(
        &self,
        dir: &Path,
        commit: &str,
        auth: &Auth,
        limits: &Limits,
    ) -> Result<Vec<InputFile>, GitError> {
        let mut c = self.cmd(Some(dir), auth);
        c.args(["ls-tree", "-r", "-z", commit, "--", ".firelight/"]);
        let out = self.run("ls-tree", c).await?;
        let mut files = Vec::new();
        let mut blobs: Vec<(usize, String)> = Vec::new();
        for rec in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let rec = String::from_utf8_lossy(rec);
            let Some((meta, path)) = rec.split_once('\t') else {
                continue;
            };
            let Some(rel) = path.strip_prefix(".firelight/") else {
                continue;
            };
            if rel.rsplit('/').next() == Some(".DS_Store")
                || rel.matches('/').count() > crate::dir::MAX_DEPTH
            {
                continue;
            }
            // One past the limit is enough for the format to report it.
            if files.len() as u64 > limits.max_files {
                break;
            }
            let f: Vec<&str> = meta.split_whitespace().collect();
            let (mode, kind, oid) = (
                f.first().copied().unwrap_or(""),
                f.get(1).copied().unwrap_or(""),
                f.get(2).copied().unwrap_or(""),
            );
            let file_kind = match (mode, kind) {
                ("120000", _) => FileKind::Symlink,
                ("100644" | "100755", "blob") => FileKind::Regular,
                _ => FileKind::Special, // a submodule (160000) or anything else
            };
            if file_kind == FileKind::Regular
                && oid.len() >= 40
                && oid.chars().all(|c| c.is_ascii_hexdigit())
            {
                blobs.push((files.len(), oid.to_string()));
            }
            files.push(InputFile {
                path: rel.to_string(),
                kind: file_kind,
                contents: None,
                size: 0,
            });
        }
        if blobs.is_empty() {
            return Ok(files);
        }
        let oids: Vec<String> = blobs.iter().map(|(_, o)| o.clone()).collect();
        self.prefetch(dir, auth, &oids).await?;
        let sizes = self.sizes(dir, &oids).await?;
        let mut total: u64 = 0;
        let mut read: Vec<(usize, String)> = Vec::new();
        for ((i, oid), size) in blobs.into_iter().zip(sizes) {
            files[i].size = size;
            if size <= limits.max_file_bytes && total + size <= limits.max_source_bytes {
                total += size;
                read.push((i, oid));
            }
        }
        if !read.is_empty() {
            let contents = self
                .cat_blobs(
                    dir,
                    auth,
                    &read.iter().map(|(_, o)| o.clone()).collect::<Vec<_>>(),
                )
                .await?;
            for ((i, _), b) in read.into_iter().zip(contents) {
                files[i].size = b.len() as u64;
                files[i].contents = Some(b);
            }
        }
        Ok(files)
    }

    /// Fetch the given blobs from the promisor remote in one request (those
    /// already present are skipped by git).
    async fn prefetch(&self, dir: &Path, auth: &Auth, oids: &[String]) -> Result<(), GitError> {
        let mut c = self.cmd(Some(dir), auth);
        c.args([
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--filter=blob:none",
            "--stdin",
            "origin",
        ])
        .stdin(Stdio::piped());
        let mut child = c.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::Missing(e.to_string())
            } else {
                GitError::Io(e)
            }
        })?;
        if let Some(mut stdin) = child.stdin.take() {
            let input: String = oids.iter().map(|o| format!("{o}\n")).collect();
            stdin.write_all(input.as_bytes()).await?;
        }
        let out = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Err(_) => {
                return Err(GitError::Timeout {
                    what: "fetch".into(),
                    secs: self.timeout.as_secs(),
                });
            }
            Ok(r) => r?,
        };
        if !out.status.success() {
            return Err(GitError::Failed {
                what: "fetch (blobs)".into(),
                message: redact(String::from_utf8_lossy(&out.stderr).trim()),
            });
        }
        Ok(())
    }

    /// Sizes of objects already in the mirror, in order.
    async fn sizes(&self, dir: &Path, oids: &[String]) -> Result<Vec<u64>, GitError> {
        let mut c = self.cmd(Some(dir), &Auth::None);
        // No lazy fetching here: everything was prefetched.
        c.env("GIT_NO_LAZY_FETCH", "1")
            .args(["cat-file", "--batch-check=%(objectsize)"])
            .stdin(Stdio::piped());
        let mut child = c.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            let input: String = oids.iter().map(|o| format!("{o}\n")).collect();
            stdin.write_all(input.as_bytes()).await?;
        }
        let out = child.wait_with_output().await?;
        let text = String::from_utf8_lossy(&out.stdout);
        let sizes: Vec<u64> = text
            .lines()
            .map(|l| l.trim().parse().unwrap_or(u64::MAX))
            .collect();
        if sizes.len() != oids.len() {
            return Err(GitError::Failed {
                what: "cat-file --batch-check".into(),
                message: "missing objects after the fetch".into(),
            });
        }
        Ok(sizes)
    }

    /// Read blobs with one `cat-file --batch` (lazy-fetched from the promisor remote).
    async fn cat_blobs(
        &self,
        dir: &Path,
        auth: &Auth,
        oids: &[String],
    ) -> Result<Vec<Vec<u8>>, GitError> {
        let mut c = self.cmd(Some(dir), auth);
        c.args(["cat-file", "--batch"]).stdin(Stdio::piped());
        let mut child = c.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::Missing(e.to_string())
            } else {
                GitError::Io(e)
            }
        })?;
        let mut stdin = child.stdin.take().ok_or_else(|| GitError::Failed {
            what: "cat-file".into(),
            message: "no stdin".into(),
        })?;
        let input: String = oids.iter().map(|o| format!("{o}\n")).collect();
        let writer = tokio::spawn(async move {
            let _ = stdin.write_all(input.as_bytes()).await;
            drop(stdin);
        });
        let mut stdout = child.stdout.take().ok_or_else(|| GitError::Failed {
            what: "cat-file".into(),
            message: "no stdout".into(),
        })?;
        let read = async {
            let mut all = Vec::new();
            stdout.read_to_end(&mut all).await?;
            Ok::<_, std::io::Error>(all)
        };
        let all = match tokio::time::timeout(self.timeout, read).await {
            Err(_) => {
                return Err(GitError::Timeout {
                    what: "cat-file".into(),
                    secs: self.timeout.as_secs(),
                });
            }
            Ok(r) => r?,
        };
        let _ = writer.await;
        let status = child.wait().await?;
        if !status.success() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err).await;
            }
            return Err(GitError::Failed {
                what: "cat-file".into(),
                message: redact(err.trim()),
            });
        }
        // "<oid> blob <size>\n<bytes>\n" per object.
        let mut out = Vec::new();
        let mut pos = 0;
        while pos < all.len() && out.len() < oids.len() {
            let nl = all[pos..]
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| pos + i)
                .ok_or_else(|| GitError::Failed {
                    what: "cat-file".into(),
                    message: "truncated output".into(),
                })?;
            let header = String::from_utf8_lossy(&all[pos..nl]).to_string();
            let parts: Vec<&str> = header.split(' ').collect();
            if parts.get(1) != Some(&"blob") {
                return Err(GitError::Failed {
                    what: "cat-file".into(),
                    message: format!("unexpected object: {header}"),
                });
            }
            let size: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let start = nl + 1;
            let end = start + size;
            if end > all.len() {
                return Err(GitError::Failed {
                    what: "cat-file".into(),
                    message: "truncated object".into(),
                });
            }
            out.push(all[start..end].to_vec());
            pos = end + 1;
        }
        Ok(out)
    }

    /// When any of `paths` last changed at or before `commit` (git history
    /// only, no file contents): the committer date, RFC 3339. Paths are
    /// repo-relative globs (`terraform/access/**`).
    pub async fn last_change(
        &self,
        dir: &Path,
        commit: &str,
        paths: &[String],
    ) -> Result<Option<String>, GitError> {
        if paths.is_empty() {
            return Ok(None);
        }
        let mut c = self.cmd(Some(dir), &Auth::None);
        c.args(["log", "-1", "--format=%cI", commit, "--"]);
        for p in paths {
            c.arg(format!(":(glob,top){p}"));
        }
        let s = String::from_utf8_lossy(&self.run("log", c).await?)
            .trim()
            .to_string();
        Ok((!s.is_empty()).then_some(s))
    }
}

/// The mirror folder for a source id under the data folder.
pub fn mirror_dir(data: &Path, source_id: &str) -> PathBuf {
    data.join("git").join(format!("{source_id}.git"))
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn redaction() {
        assert_eq!(
            redact(
                "fatal: unable to access 'https://x-access-token:ghs_secret@github.com/a/b/': 403"
            ),
            "fatal: unable to access 'https://••••@github.com/a/b/': 403"
        );
        assert_eq!(
            redact("Authorization: Bearer abc"),
            "[a line about authorization, redacted]"
        );
        assert_eq!(
            redact("plain https://github.com/a/b"),
            "plain https://github.com/a/b"
        );
    }
}
