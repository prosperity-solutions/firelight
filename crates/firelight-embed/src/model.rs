//! Model download and verification (§8.3).
//!
//! A model is a Hugging Face repo at a pinned 40-hex revision. Every file is
//! checked before use: large (LFS) files against their sha256, small files
//! against their git blob SHA-1, both read from the revision's file listing,
//! or against `FIRELIGHT_EMBED_SHA256` when given. The default model's
//! checksums are compiled in. Files are written to a temporary name and
//! renamed only after the checksum matches; a `verified.json` next to them
//! records what was checked, so a cached model starts with no network at all.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use sha1::Digest as _;

use crate::{Result, err};

/// The files a model needs (one safetensors file; sharded models are refused).
pub const FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

/// No model file is bigger than this (the default is 1.2 GB).
pub const MAX_FILE_BYTES: u64 = 8 << 30;

/// The default model (§8.2), pinned: revision and every file's checksum.
pub const DEFAULT_REPO: &str = "Qwen/Qwen3-Embedding-0.6B";
pub const DEFAULT_REVISION: &str = "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3";
pub const DEFAULT_FILES: &[(&str, Check)] = &[
    (
        "config.json",
        Check::GitSha1("cef2749ee93607b8f9a58ec72f4f6bfaf874e71d"),
    ),
    (
        "tokenizer.json",
        Check::Sha256("def76fb086971c7867b829c23a26261e38d9d74e02139253b38aeb9df8b4b50a"),
    ),
    (
        "model.safetensors",
        Check::Sha256("0437e45c94563b09e13cb7a64478fc406947a93cb34a7e05870fc8dcd48e23fd"),
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Sha256(&'static str),
    GitSha1(&'static str),
}

/// How a file is checked, owned (from the listing or config).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Expect {
    Sha256(String),
    GitSha1(String),
}

impl From<Check> for Expect {
    fn from(c: Check) -> Self {
        match c {
            Check::Sha256(s) => Expect::Sha256(s.into()),
            Check::GitSha1(s) => Expect::GitSha1(s.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Spec {
    /// `owner/name` on Hugging Face.
    pub repo: String,
    /// A 40-hex commit.
    pub revision: String,
    /// `FIRELIGHT_EMBED_SHA256`: file → sha256, overriding the listing.
    pub sha256: BTreeMap<String, String>,
    /// `https://huggingface.co` or a mirror.
    pub endpoint: String,
}

impl Spec {
    pub fn default_model() -> Spec {
        Spec {
            repo: DEFAULT_REPO.into(),
            revision: DEFAULT_REVISION.into(),
            sha256: BTreeMap::new(),
            endpoint: "https://huggingface.co".into(),
        }
    }

    pub fn is_default(&self) -> bool {
        self.repo == DEFAULT_REPO && self.revision == DEFAULT_REVISION
    }

    /// Refuse what can't be a pinned Hugging Face model.
    pub fn validate(&self) -> Result<()> {
        if !valid_repo(&self.repo) {
            return Err(err(format!(
                "{:?} is not a Hugging Face model id: owner/name, letters, digits, . _ -",
                self.repo
            )));
        }
        if !is_hex(&self.revision, 40) {
            return Err(err(format!(
                "the model revision must be a 40-hex commit, not {:?}: branch names move",
                self.revision
            )));
        }
        for (f, h) in &self.sha256 {
            if !FILES.contains(&f.as_str()) || !is_hex(h, 64) {
                return Err(err(format!(
                    "embed.sha256: {f:?} = {h:?} should be one of {FILES:?} and 64 hex characters"
                )));
            }
        }
        if !(self.endpoint.starts_with("https://") || self.endpoint.starts_with("http://")) {
            return Err(err("embed.hf_endpoint must be an http(s) URL"));
        }
        Ok(())
    }

    /// `<data>/models/<owner>--<name>/<revision>/`.
    pub fn dir(&self, data: &Path) -> PathBuf {
        data.join("models")
            .join(self.repo.replace('/', "--"))
            .join(&self.revision)
    }
}

pub fn valid_repo(s: &str) -> bool {
    let ok = |p: &str| {
        !p.is_empty()
            && p.len() <= 96
            && !p.starts_with('.')
            && !p.contains("..")
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    matches!(s.split_once('/'), Some((a, b)) if ok(a) && ok(b))
}

pub fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// What to check each file against, without touching the network when the
/// checksums are pinned (default model) or configured (all three files).
pub fn pinned_expectations(spec: &Spec) -> Option<BTreeMap<String, Expect>> {
    let mut out = BTreeMap::new();
    if spec.is_default() {
        for (f, c) in DEFAULT_FILES {
            out.insert((*f).to_string(), Expect::from(*c));
        }
    }
    for (f, h) in &spec.sha256 {
        out.insert(f.clone(), Expect::Sha256(h.to_ascii_lowercase()));
    }
    FILES.iter().all(|f| out.contains_key(*f)).then_some(out)
}

/// Expectations from the revision's file listing (`/api/models/<repo>/tree/<rev>`).
pub fn expectations_from_listing(
    listing: &serde_json::Value,
    spec: &Spec,
) -> Result<BTreeMap<String, Expect>> {
    let mut out = BTreeMap::new();
    let entries = listing
        .as_array()
        .ok_or_else(|| err("the model's file listing is not a list"))?;
    for f in FILES {
        if let Some(h) = spec.sha256.get(*f) {
            out.insert((*f).to_string(), Expect::Sha256(h.to_ascii_lowercase()));
            continue;
        }
        let e = entries
            .iter()
            .find(|e| e["path"].as_str() == Some(f))
            .ok_or_else(|| {
                err(format!(
                    "{} at {} has no {f} (Firelight needs {FILES:?}; sharded models aren't supported)",
                    spec.repo, spec.revision
                ))
            })?;
        let exp = if let Some(s) = e["lfs"]["oid"].as_str() {
            Expect::Sha256(s.to_ascii_lowercase())
        } else if let Some(s) = e["oid"].as_str() {
            Expect::GitSha1(s.to_ascii_lowercase())
        } else {
            return Err(err(format!("the listing gives no checksum for {f}")));
        };
        match &exp {
            Expect::Sha256(h) if !is_hex(h, 64) => {
                return Err(err(format!("bad sha256 for {f} in the listing")));
            }
            Expect::GitSha1(h) if !is_hex(h, 40) => {
                return Err(err(format!("bad git SHA-1 for {f} in the listing")));
            }
            _ => {}
        }
        out.insert((*f).to_string(), exp);
    }
    Ok(out)
}

/// [`verify_file`] on a blocking thread.
async fn verify_async(path: &Path, exp: &Expect) -> Result<bool> {
    let (p, e) = (path.to_path_buf(), exp.clone());
    tokio::task::spawn_blocking(move || verify_file(&p, &e))
        .await
        .map_err(|e| err(e.to_string()))?
}

/// Does `path` match `exp`? Reads the whole file.
pub fn verify_file(path: &Path, exp: &Expect) -> Result<bool> {
    use std::io::Read as _;
    let mut f = std::fs::File::open(path)
        .map_err(|e| err(format!("cannot read {}: {e}", path.display())))?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut buf = vec![0u8; 1 << 20];
    let mut s256 = sha2::Sha256::new();
    let mut s1 = sha1::Sha1::new();
    s1.update(format!("blob {len}\0").as_bytes());
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| err(format!("cannot read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        match exp {
            Expect::Sha256(_) => s256.update(&buf[..n]),
            Expect::GitSha1(_) => s1.update(&buf[..n]),
        }
    }
    Ok(match exp {
        Expect::Sha256(h) => hex(&s256.finalize()) == *h,
        Expect::GitSha1(h) => hex(&s1.finalize()) == *h,
    })
}

/// An error with its causes (reqwest hides the interesting part in them).
fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

const MARKER: &str = "verified.json";

/// The checked files, when a previous run verified exactly these expectations.
fn cached(dir: &Path, want: Option<&BTreeMap<String, Expect>>) -> Option<BTreeMap<String, Expect>> {
    let text = std::fs::read_to_string(dir.join(MARKER)).ok()?;
    let had: BTreeMap<String, Expect> = serde_json::from_str(&text).ok()?;
    if want.is_some_and(|w| *w != had) {
        return None;
    }
    FILES
        .iter()
        .all(|f| had.contains_key(*f) && dir.join(f).is_file())
        .then_some(had)
}

/// Make sure every model file is in `<data>/models/…` and verified; download
/// what's missing. Returns the folder. Never returns an unverified model.
pub async fn ensure(spec: &Spec, data: &Path, client: &reqwest::Client) -> Result<PathBuf> {
    spec.validate()?;
    let dir = spec.dir(data);
    let pinned = pinned_expectations(spec);
    if cached(&dir, pinned.as_ref()).is_some() {
        return Ok(dir);
    }
    let want = match pinned {
        Some(p) => p,
        None => {
            let url = format!(
                "{}/api/models/{}/tree/{}",
                spec.endpoint.trim_end_matches('/'),
                spec.repo,
                spec.revision
            );
            let listing: serde_json::Value = client
                .get(&url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|e| err(format!("cannot list the model files ({url}): {e}")))?
                .json()
                .await
                .map_err(|e| err(format!("the model file listing is not JSON: {e}")))?;
            expectations_from_listing(&listing, spec)?
        }
    };
    std::fs::create_dir_all(&dir)
        .map_err(|e| err(format!("cannot create {}: {e}", dir.display())))?;
    for f in FILES {
        let exp = &want[*f];
        let path = dir.join(f);
        if path.is_file() && verify_async(&path, exp).await? {
            continue;
        }
        let mut tries = 0;
        loop {
            tries += 1;
            match download(spec, f, &path, exp, client).await {
                Ok(()) => break,
                // A checksum mismatch is final; a broken connection is worth
                // another try.
                Err(e) if tries < 3 && !e.to_string().contains("checksum") => {
                    tracing::warn!(file = %f, error = %e, "download failed; trying again");
                    tokio::time::sleep(std::time::Duration::from_secs(2 * tries)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
    let marker = serde_json::to_string_pretty(&want).map_err(|e| err(e.to_string()))?;
    std::fs::write(dir.join(MARKER), marker)
        .map_err(|e| err(format!("cannot write {}: {e}", dir.join(MARKER).display())))?;
    Ok(dir)
}

/// Use a local folder (`FIRELIGHT_EMBED_PATH`): no network. Its files are
/// checked when checksums are pinned or configured; otherwise the operator
/// vouches for the folder.
pub fn local(spec: Option<&Spec>, dir: &Path) -> Result<PathBuf> {
    for f in FILES {
        if !dir.join(f).is_file() {
            return Err(err(format!("{} has no {f}", dir.display())));
        }
    }
    if let Some(want) = spec.and_then(pinned_expectations) {
        for (f, exp) in &want {
            if !verify_file(&dir.join(f), exp)? {
                return Err(err(format!(
                    "{} doesn't match its checksum",
                    dir.join(f).display()
                )));
            }
        }
    }
    Ok(dir.to_path_buf())
}

async fn download(
    spec: &Spec,
    file: &str,
    dest: &Path,
    exp: &Expect,
    client: &reqwest::Client,
) -> Result<()> {
    let url = format!(
        "{}/{}/resolve/{}/{}",
        spec.endpoint.trim_end_matches('/'),
        spec.repo,
        spec.revision,
        file
    );
    tracing::info!(%url, "downloading model file");
    let mut res = client
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| err(format!("cannot download {url}: {}", chain(&e))))?;
    let tmp = dest.with_extension(format!("part-{}", std::process::id()));
    let result = async {
        let mut out = std::fs::File::create(&tmp)
            .map_err(|e| err(format!("cannot write {}: {e}", tmp.display())))?;
        let mut total: u64 = 0;
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| err(format!("download of {file} broke off: {}", chain(&e))))?
        {
            total += chunk.len() as u64;
            if total > MAX_FILE_BYTES {
                return Err(err(format!("{file} is bigger than {MAX_FILE_BYTES} bytes")));
            }
            out.write_all(&chunk)
                .map_err(|e| err(format!("cannot write {}: {e}", tmp.display())))?;
        }
        out.sync_all()
            .map_err(|e| err(format!("cannot write {}: {e}", tmp.display())))?;
        if !verify_async(&tmp, exp).await? {
            return Err(err(format!(
                "{file} from {url} doesn't match its checksum; refusing to use it"
            )));
        }
        std::fs::rename(&tmp, dest)
            .map_err(|e| err(format!("cannot move {} into place: {e}", tmp.display())))
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_ids() {
        for ok in [
            "Qwen/Qwen3-Embedding-0.6B",
            "intfloat/multilingual-e5-small",
        ] {
            assert!(valid_repo(ok), "{ok}");
        }
        for bad in [
            "",
            "a",
            "a/b/c",
            "../x",
            "a/..",
            "a/.hidden",
            "a/b c",
            "/a",
            "a/",
        ] {
            assert!(!valid_repo(bad), "{bad}");
        }
    }

    #[test]
    fn revisions_must_be_commits() {
        let mut s = Spec::default_model();
        assert!(s.validate().is_ok());
        s.revision = "main".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn default_model_is_pinned() {
        let p = pinned_expectations(&Spec::default_model());
        assert_eq!(p.map(|p| p.len()), Some(FILES.len()));
        let mut other = Spec::default_model();
        other.repo = "intfloat/multilingual-e5-small".into();
        assert!(
            pinned_expectations(&other).is_none(),
            "others come from the listing"
        );
    }

    #[test]
    fn git_blob_sha1() {
        let d = std::env::temp_dir().join(format!("fl-embed-sha1-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("x");
        let _ = std::fs::write(&p, "hello\n");
        // `printf 'hello\n' | git hash-object --stdin`
        let ok = Expect::GitSha1("ce013625030ba8dba906f756967f9e9ca394464a".into());
        assert!(verify_file(&p, &ok).unwrap_or(false));
        let bad = Expect::Sha256("00".repeat(32));
        assert!(!verify_file(&p, &bad).unwrap_or(true));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn listing() {
        let l = serde_json::json!([
            {"type":"file","oid":"cef2749ee93607b8f9a58ec72f4f6bfaf874e71d","size":727,"path":"config.json"},
            {"type":"file","oid":"18a4abbf4a408dc534006a455544dcc195f356d2","size":1,"lfs":{"oid":"def76fb086971c7867b829c23a26261e38d9d74e02139253b38aeb9df8b4b50a"},"path":"tokenizer.json"},
            {"type":"file","oid":"9116264a405846ebfe4e95556b63b1ca07c892a0","size":1,"lfs":{"oid":"0437e45c94563b09e13cb7a64478fc406947a93cb34a7e05870fc8dcd48e23fd"},"path":"model.safetensors"}
        ]);
        let e = expectations_from_listing(&l, &Spec::default_model()).unwrap_or_default();
        let pinned = pinned_expectations(&Spec::default_model()).unwrap_or_default();
        assert_eq!(e, pinned, "the pinned checksums are the listing's");
        let short = serde_json::json!([l[0].clone()]);
        assert!(expectations_from_listing(&short, &Spec::default_model()).is_err());
    }
}
