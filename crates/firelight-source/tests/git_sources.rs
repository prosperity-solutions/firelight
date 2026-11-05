//! §13.2, hermetic: local bare repos, served over file:// and over a local
//! smart-HTTP server (git http-backend), through the real git CLI.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use firelight_format::Limits;
use firelight_source::git::Git;
use firelight_source::pull::{Credentials, Origin, Pulled, Puller, SourceSpec, is_stale};
use firelight_source::{Clock, SystemClock};

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "fl-src-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap_or_else(|e| panic!("{e}"));
        Tmp(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A working repo that pushes to a bare one.
struct Remote {
    _tmp: Tmp,
    work: PathBuf,
    bare: PathBuf,
}

fn git(dir: &Path, args: &[&str], date: &str) {
    let mut c = Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            c.env_remove(k);
        }
    }
    let out = c
        .args([
            "-c",
            "user.email=t@acme.example",
            "-c",
            "user.name=T",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

const REPO: &str = "format: 1\nid: infra\nname: Infra\nsummary: Infra for the tests.\n";
const CONCEPT: &str = "---\nformat: 1\ntitle: Access\nsummary: Who may do what.\ncovers: [terraform/access/**]\n---\nBody.\n";

impl Remote {
    fn new() -> Self {
        let tmp = Tmp::new("remote");
        let work = tmp.0.join("work");
        let bare = tmp.0.join("infra.git");
        std::fs::create_dir_all(&work).unwrap_or_else(|e| panic!("{e}"));
        git(
            &tmp.0,
            &["init", "--bare", "-q", "infra.git"],
            "2026-10-01T00:00:00Z",
        );
        git(&work, &["init", "-q"], "2026-10-01T00:00:00Z");
        git(
            &work,
            &["remote", "add", "origin", bare.to_str().unwrap_or("")],
            "2026-10-01T00:00:00Z",
        );
        Remote {
            _tmp: tmp,
            work,
            bare,
        }
    }
    fn write(&self, path: &str, contents: &str) {
        let p = self.work.join(path);
        std::fs::create_dir_all(p.parent().unwrap_or(&self.work)).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(p, contents).unwrap_or_else(|e| panic!("{e}"));
    }
    fn commit(&self, msg: &str, date: &str) {
        git(&self.work, &["add", "-A"], date);
        git(&self.work, &["commit", "-q", "-m", msg], date);
        git(
            &self.work,
            &["push", "-q", "-f", "origin", "HEAD:main"],
            date,
        );
    }
    fn url(&self) -> String {
        format!("file://{}", self.bare.display())
    }
}

fn puller(data: &Path) -> Puller {
    Puller {
        git: Git::default(),
        data_dir: data.to_path_buf(),
        limits: Limits::default(),
        clock: Arc::new(SystemClock),
    }
}

fn spec(url: String, creds: Credentials) -> SourceSpec {
    SourceSpec {
        id: "infra".into(),
        origin: Origin::Git {
            url,
            git_ref: None,
            creds,
        },
    }
}

fn good(p: &Pulled) -> &firelight_store::Snapshot {
    match p {
        Pulled::Good { snapshot, .. } => snapshot,
        other => panic!("expected Good, got {other:?}"),
    }
}

#[tokio::test]
async fn the_whole_life_of_a_source() {
    let r = Remote::new();
    r.write(".firelight/repo.yaml", REPO);
    r.write(".firelight/concepts/access.md", CONCEPT);
    r.write("terraform/access/main.tf", "# v1\n");
    r.write("secret.txt", "not for firelight\n");
    r.commit("first", "2026-10-01T10:00:00Z");
    let data = Tmp::new("data");
    let p = puller(&data.0);
    let s = spec(r.url(), Credentials::Anonymous);

    // First pull.
    let first = p.pull(&s, None).await.unwrap_or_else(|e| panic!("{e}"));
    let snap = good(&first).clone();
    assert_eq!(
        snap.files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["concepts/access.md", "repo.yaml"],
        "only .firelight/ is read"
    );
    assert_eq!(snap.committed_at.as_deref(), Some("2026-10-01T10:00:00Z"));
    assert!(!is_stale(&snap.history[0]));

    // No change: nothing fetched.
    assert!(matches!(
        p.pull(&s, Some(&snap.commit)).await,
        Ok(Pulled::Unchanged)
    ));

    // Covered paths change after the entry: maybe stale.
    r.write("terraform/access/main.tf", "# v2\n");
    r.commit("change access", "2026-10-05T10:00:00Z");
    let second = p
        .pull(&s, Some(&snap.commit))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let snap2 = good(&second).clone();
    assert_ne!(snap2.commit, snap.commit);
    assert!(is_stale(&snap2.history[0]), "{:?}", snap2.history);

    // A broken commit is rejected with its diagnostics (the caller keeps
    // serving the last good one).
    r.write(
        ".firelight/concepts/access.md",
        "---\nformat: 1\ntitle: Access\nsummary: S.\nrelated: [nope]\n---\nx\n",
    );
    r.commit("break", "2026-10-06T10:00:00Z");
    match p.pull(&s, Some(&snap2.commit)).await {
        Ok(Pulled::Bad {
            commit,
            diagnostics,
        }) => {
            assert_ne!(commit, snap2.commit);
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.code == "unresolved-ref" && d.line == 5),
                "{diagnostics:?}"
            );
        }
        other => panic!("expected Bad, got {other:?}"),
    }

    // Fixed again, and the entry is no longer stale (it changed after the covers).
    r.write(
        ".firelight/concepts/access.md",
        CONCEPT.replace("Body.", "Body, updated.").as_str(),
    );
    r.commit("fix", "2026-10-07T10:00:00Z");
    let fixed = p.pull(&s, None).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(!is_stale(&good(&fixed).history[0]));

    // Force-push to an unrelated history still works.
    let r2 = Remote::new();
    r2.write(".firelight/repo.yaml", REPO);
    r2.commit("rewritten", "2026-10-08T10:00:00Z");
    git(
        &r2.work,
        &[
            "push",
            "-q",
            "-f",
            r.bare.to_str().unwrap_or(""),
            "HEAD:main",
        ],
        "2026-10-08T10:00:00Z",
    );
    let forced = p
        .pull(&s, Some(&good(&fixed).commit))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(good(&forced).files.len(), 1);

    // A deleted ref is an error, not a crash.
    let gone = spec(r.url(), Credentials::Anonymous);
    let gone = SourceSpec {
        origin: Origin::Git {
            url: r.url(),
            git_ref: Some("does-not-exist".into()),
            creds: Credentials::Anonymous,
        },
        ..gone
    };
    let e = p
        .pull(&gone, None)
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(e.contains("no ref does-not-exist"), "{e}");
}

#[tokio::test]
async fn symlinks_in_the_tree_are_refused_not_followed() {
    let r = Remote::new();
    r.write(".firelight/repo.yaml", REPO);
    r.write("secret.txt", "TOP SECRET\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink("../../secret.txt", r.work.join(".firelight/about.md"))
        .unwrap_or_else(|e| panic!("{e}"));
    r.commit("symlink", "2026-10-01T10:00:00Z");
    let data = Tmp::new("data");
    match puller(&data.0)
        .pull(&spec(r.url(), Credentials::Anonymous), None)
        .await
    {
        Ok(Pulled::Bad { diagnostics, .. }) => {
            assert!(
                diagnostics.iter().any(|d| d.code == "symlink"),
                "{diagnostics:?}"
            );
        }
        other => panic!("expected Bad, got {other:?}"),
    }
}

#[tokio::test]
async fn a_repo_id_that_does_not_match_the_source_is_refused() {
    let r = Remote::new();
    r.write(
        ".firelight/repo.yaml",
        &REPO.replace("id: infra", "id: other"),
    );
    r.commit("x", "2026-10-01T10:00:00Z");
    let data = Tmp::new("data");
    assert!(matches!(
        puller(&data.0)
            .pull(&spec(r.url(), Credentials::Anonymous), None)
            .await,
        Ok(Pulled::Bad { .. })
    ));
}

#[tokio::test]
async fn a_dir_source() {
    let tmp = Tmp::new("dir");
    std::fs::create_dir_all(tmp.0.join(".firelight")).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(tmp.0.join(".firelight/repo.yaml"), REPO).unwrap_or_else(|e| panic!("{e}"));
    let data = Tmp::new("data");
    let p = puller(&data.0);
    let s = SourceSpec {
        id: "infra".into(),
        origin: Origin::Dir(tmp.0.clone()),
    };
    let first = p.pull(&s, None).await.unwrap_or_else(|e| panic!("{e}"));
    let commit = good(&first).commit.clone();
    assert!(matches!(
        p.pull(&s, Some(&commit)).await,
        Ok(Pulled::Unchanged)
    ));
    std::fs::write(tmp.0.join(".firelight/about.md"), "About.\n").unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        p.pull(&s, Some(&commit)).await,
        Ok(Pulled::Good { .. })
    ));
}

// ---- Over HTTP, with a bearer token -------------------------------------

/// A smart-HTTP git server: `git http-backend` as CGI, requiring a bearer token.
async fn http_server(root: PathBuf, token: &'static str) -> String {
    use axum::body::Bytes;
    use axum::http::{HeaderMap, Method, StatusCode, Uri};
    use axum::response::IntoResponse;
    let exec = String::from_utf8_lossy(
        &Command::new("git")
            .arg("--exec-path")
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default(),
    )
    .trim()
    .to_string();
    let backend = PathBuf::from(exec).join("git-http-backend");
    let handler = move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
        let root = root.clone();
        let backend = backend.clone();
        async move {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if auth != format!("Bearer {token}") {
                return (
                    StatusCode::UNAUTHORIZED,
                    [("www-authenticate", "Basic realm=\"x\"")],
                    Vec::new(),
                )
                    .into_response();
            }
            let mut c = tokio::process::Command::new(&backend);
            c.env_clear()
                .env("GIT_PROJECT_ROOT", &root)
                .env("GIT_HTTP_EXPORT_ALL", "1")
                .env("PATH_INFO", uri.path())
                .env("QUERY_STRING", uri.query().unwrap_or(""))
                .env("REQUEST_METHOD", method.as_str())
                .env(
                    "CONTENT_TYPE",
                    headers
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or(""),
                )
                .env("CONTENT_LENGTH", body.len().to_string())
                .env(
                    "GIT_PROTOCOL",
                    headers
                        .get("git-protocol")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or(""),
                )
                .env(
                    "HTTP_GIT_PROTOCOL",
                    headers
                        .get("git-protocol")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or(""),
                )
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped());
            let Ok(mut child) = c.spawn() else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(&body).await;
            }
            let Ok(out) = child.wait_with_output().await else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            let raw = out.stdout;
            let split = raw
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|i| (i, 4))
                .or_else(|| raw.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2)));
            let Some((i, n)) = split else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            let mut status = StatusCode::OK;
            let mut resp_headers = HeaderMap::new();
            for line in String::from_utf8_lossy(&raw[..i]).lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let v = v.trim();
                    if k.eq_ignore_ascii_case("status") {
                        status = v
                            .split(' ')
                            .next()
                            .and_then(|c| c.parse().ok())
                            .and_then(|c| StatusCode::from_u16(c).ok())
                            .unwrap_or(StatusCode::OK);
                    } else if let (Ok(k), Ok(v)) = (
                        axum::http::HeaderName::from_bytes(k.as_bytes()),
                        axum::http::HeaderValue::from_str(v),
                    ) {
                        resp_headers.insert(k, v);
                    }
                }
            }
            (status, resp_headers, raw[i + n..].to_vec()).into_response()
        }
    };
    let app = axum::Router::new().fallback(handler);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let addr = listener.local_addr().unwrap_or_else(|e| panic!("{e}"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn over_http_with_a_token() {
    let r = Remote::new();
    r.write(".firelight/repo.yaml", REPO);
    r.commit("first", "2026-10-01T10:00:00Z");
    let root = r.bare.parent().map(Path::to_path_buf).unwrap_or_default();
    let base = http_server(root, "s3cret-token").await;
    let url = format!("{base}/infra.git");
    let data = Tmp::new("data");
    let tok = Tmp::new("tok");
    let good_tok = tok.0.join("good");
    let bad_tok = tok.0.join("bad");
    std::fs::write(&good_tok, "s3cret-token\n").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(&bad_tok, "wrong-token-value").unwrap_or_else(|e| panic!("{e}"));
    let p = puller(&data.0);
    let ok = p
        .pull(
            &spec(
                url.clone(),
                Credentials::TokenFile {
                    path: good_tok,
                    basic: false,
                },
            ),
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(good(&ok).files.len(), 1);
    // A wrong token fails, and the error never shows it.
    let e = p
        .pull(
            &spec(
                url.clone(),
                Credentials::TokenFile {
                    path: bad_tok,
                    basic: false,
                },
            ),
            None,
        )
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(!e.is_empty() && !e.contains("wrong-token-value"), "{e}");
    let e = p
        .pull(&spec(url, Credentials::Anonymous), None)
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        !e.is_empty(),
        "anonymous must fail against a token-protected server"
    );
}

#[test]
fn the_clock_is_injectable() {
    struct Fixed;
    impl Clock for Fixed {
        fn now_unix(&self) -> i64 {
            42
        }
    }
    assert_eq!(Fixed.now_unix(), 42);
}
