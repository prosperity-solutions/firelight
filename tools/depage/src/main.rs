//! depage: the dependency-age gate (docs/HANDOVER.md §17.3, D7).
//!
//! Every crate version a change adds to `Cargo.lock`, the pinned Rust
//! toolchain, and every tool in `tools/versions.toml` must have been published
//! at least 7 days before the change's head commit. Exceptions live in
//! `.github/fresh-deps.txt`, one per line:
//!
//! ```text
//! <name> <version> <expires YYYY-MM-DD> <reason…>
//! ```
//!
//! An exception expires at most 30 days after the head commit, and an expired
//! one fails, so none lingers.
//!
//! Publish dates come from the crates.io sparse index (`pubtime`), Rust's
//! release manifests (`date`), and the `released` field of binary tools, which
//! is checked against GitHub's release when `repo` is set. Index entries
//! without `pubtime` predate the field (2025) and so are old enough.
//!
//! Test-only npm packages (`tests/browser/package-lock.json`) follow the same
//! rule, dated by the npm registry. HTTP goes through `curl`; the two TOML
//! files are a strict flat subset parsed here; JSON uses serde_json. Tests point `DEPAGE_FETCH_DIR` at a folder
//! that mirrors the URLs, so they never touch the network.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const MIN_AGE_DAYS: i64 = 7;
const MAX_EXCEPTION_DAYS: i64 = 30;
const DAY: i64 = 86_400;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(report) => {
            print!("{}", report.text);
            if report.failed {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("depage: {e}");
            ExitCode::from(2)
        }
    }
}

struct Opts {
    root: PathBuf,
    base: Option<String>,
    head: String,
    now: Option<i64>,
    warn_unresolved: bool,
}

fn parse_args(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        root: PathBuf::from("."),
        base: None,
        head: "HEAD".into(),
        now: None,
        warn_unresolved: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--root" => o.root = PathBuf::from(val("--root")?),
            "--base" => o.base = Some(val("--base")?).filter(|s| !s.is_empty()),
            "--head" => o.head = val("--head")?,
            "--now" => o.now = Some(parse_time(&val("--now")?)?),
            "--warn-unresolved" => o.warn_unresolved = true,
            "-h" | "--help" => {
                return Err(
                    "usage: depage [--root DIR] [--base REV] [--head REV] [--warn-unresolved]\n\
                     Checks what changed between --base and --head (no --base: everything)."
                        .into(),
                );
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(o)
}

pub struct Report {
    pub text: String,
    pub failed: bool,
}

fn run(args: &[String]) -> Result<Report, String> {
    let o = parse_args(args)?;
    let head_time = match o.now {
        Some(t) => t,
        None => parse_time(git(&o.root, &["log", "-1", "--format=%cI", &o.head])?.trim())?,
    };
    let read_head = |p: &str| git_show(&o.root, &o.head, p);
    let read_base = |p: &str| match &o.base {
        Some(b) => git_show(&o.root, b, p),
        None => Ok(None),
    };

    let mut items: Vec<Item> = Vec::new();

    // Crates added or changed in Cargo.lock.
    let head_lock = lock_packages(&read_head("Cargo.lock")?.unwrap_or_default());
    let base_lock = lock_packages(&read_base("Cargo.lock")?.unwrap_or_default());
    for (name, version) in head_lock.difference(&base_lock) {
        items.push(Item::Crate {
            name: name.clone(),
            version: version.clone(),
        });
    }

    // The Rust toolchain.
    let tc = |s: Option<String>| {
        s.and_then(|s| flat_toml(&s).ok())
            .and_then(|t| t.get("toolchain").and_then(|m| m.get("channel")).cloned())
    };
    if let Some(ch) = tc(read_head("rust-toolchain.toml")?)
        && tc(read_base("rust-toolchain.toml")?).as_ref() != Some(&ch)
    {
        items.push(Item::Toolchain { version: ch });
    }

    // Pinned tools.
    let tools = |s: Option<String>| s.map(|s| flat_toml(&s)).transpose();
    let head_tools = tools(read_head("tools/versions.toml")?)?.unwrap_or_default();
    let base_tools = tools(read_base("tools/versions.toml")?)?.unwrap_or_default();
    for (name, t) in &head_tools {
        if base_tools.get(name) == Some(t) {
            continue;
        }
        let version = t
            .get("version")
            .cloned()
            .ok_or(format!("tools/versions.toml: [{name}] has no version"))?;
        match t.get("kind").map(String::as_str) {
            Some("cargo") => items.push(Item::Crate {
                name: name.clone(),
                version,
            }),
            Some("bin") => items.push(Item::Bin {
                name: name.clone(),
                version,
                released: t.get("released").cloned(),
                repo: t.get("repo").cloned(),
                tag: t.get("tag").cloned(),
            }),
            other => {
                return Err(format!(
                    "tools/versions.toml: [{name}] kind must be \"cargo\" or \"bin\", not {other:?}"
                ));
            }
        }
    }

    // Test-only npm packages.
    let npm = |s: Option<String>| s.map_or(Ok(BTreeSet::new()), |s| npm_packages(&s));
    let head_npm = npm(read_head(NPM_LOCK)?)?;
    // A base without a usable lockfile only means every package counts as new.
    let base_npm = npm(read_base(NPM_LOCK)?).unwrap_or_default();
    for (name, version) in head_npm.difference(&base_npm) {
        items.push(Item::Npm {
            name: name.clone(),
            version: version.clone(),
        });
    }

    let exceptions = parse_exceptions(&read_head(".github/fresh-deps.txt")?.unwrap_or_default())?;
    Ok(check(
        &items,
        &exceptions,
        head_time,
        o.warn_unresolved,
        &Fetcher::from_env(),
    ))
}

#[derive(Debug, Clone)]
pub enum Item {
    Crate {
        name: String,
        version: String,
    },
    Toolchain {
        version: String,
    },
    Bin {
        name: String,
        version: String,
        released: Option<String>,
        repo: Option<String>,
        tag: Option<String>,
    },
    /// A test-only npm package (tests/browser/package-lock.json).
    Npm {
        name: String,
        version: String,
    },
}

impl Item {
    fn key(&self) -> (String, String) {
        match self {
            Item::Crate { name, version }
            | Item::Bin { name, version, .. }
            | Item::Npm { name, version } => (name.clone(), version.clone()),
            Item::Toolchain { version } => ("toolchain".into(), version.clone()),
        }
    }
    fn label(&self) -> String {
        match self {
            Item::Crate { name, version } => format!("crate {name} {version}"),
            Item::Toolchain { version } => format!("Rust toolchain {version}"),
            Item::Bin { name, version, .. } => format!("tool {name} {version}"),
            Item::Npm { name, version } => format!("npm package {name} {version}"),
        }
    }
}

pub struct Exception {
    name: String,
    version: String,
    expires: i64,
    line: usize,
}

fn parse_exceptions(s: &str) -> Result<Vec<Exception>, String> {
    let mut out = Vec::new();
    for (i, raw) in s.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 {
            return Err(format!(
                ".github/fresh-deps.txt:{}: want `<name> <version> <expires YYYY-MM-DD> <reason>`",
                i + 1
            ));
        }
        out.push(Exception {
            name: f[0].into(),
            version: f[1].into(),
            expires: parse_time(&format!("{}T00:00:00Z", f[2]))
                .map_err(|e| format!(".github/fresh-deps.txt:{}: {e}", i + 1))?,
            line: i + 1,
        });
    }
    Ok(out)
}

pub fn check(
    items: &[Item],
    exceptions: &[Exception],
    head_time: i64,
    warn_unresolved: bool,
    f: &Fetcher,
) -> Report {
    let mut text = String::new();
    let mut failed = false;
    for e in exceptions {
        if e.expires < head_time {
            failed = true;
            let _ = writeln!(
                text,
                "❌ .github/fresh-deps.txt:{}: the exception for {} {} expired on {}. Remove the line.",
                e.line,
                e.name,
                e.version,
                fmt_date(e.expires)
            );
        } else if e.expires > head_time + MAX_EXCEPTION_DAYS * DAY {
            failed = true;
            let _ = writeln!(
                text,
                "❌ .github/fresh-deps.txt:{}: the exception for {} {} runs until {}, more than {MAX_EXCEPTION_DAYS} days out.",
                e.line,
                e.name,
                e.version,
                fmt_date(e.expires)
            );
        }
    }
    let cutoff = head_time - MIN_AGE_DAYS * DAY;
    for item in items {
        let (name, version) = item.key();
        let published = match published(item, f) {
            Ok(p) => p,
            Err(err) => {
                if warn_unresolved {
                    let _ = writeln!(
                        text,
                        "⚠️  {}: could not resolve its publish date ({err}).",
                        item.label()
                    );
                } else {
                    failed = true;
                    let _ = writeln!(
                        text,
                        "❌ {}: could not resolve its publish date ({err}).",
                        item.label()
                    );
                }
                continue;
            }
        };
        let Some(t) = published else {
            let _ = writeln!(
                text,
                "✅ {}: published before the index recorded dates.",
                item.label()
            );
            continue;
        };
        if t <= cutoff {
            let _ = writeln!(text, "✅ {}: published {}.", item.label(), fmt_date(t));
            continue;
        }
        if exceptions
            .iter()
            .any(|e| e.name == name && e.version == version && e.expires >= head_time)
        {
            let _ = writeln!(
                text,
                "✅ {}: published {}, allowed by .github/fresh-deps.txt.",
                item.label(),
                fmt_date(t)
            );
            continue;
        }
        failed = true;
        let _ = writeln!(
            text,
            "❌ {}: published {}, less than {MIN_AGE_DAYS} days before the head commit ({}). \
             Take an older version (scripts/dev/cargo-update.sh picks them), or add a line to \
             .github/fresh-deps.txt naming the advisory it fixes.",
            item.label(),
            fmt_date(t),
            fmt_date(head_time)
        );
    }
    if items.is_empty() && exceptions.is_empty() {
        text.push_str("depage: nothing new to check.\n");
    } else if !failed {
        let _ = writeln!(text, "depage: {} item(s) old enough.", items.len());
    }
    Report { text, failed }
}

/// The publish time of an item; `None` when the source predates recorded dates.
fn published(item: &Item, f: &Fetcher) -> Result<Option<i64>, String> {
    match item {
        Item::Npm { name, version } => {
            let doc = f.get(&format!("https://registry.npmjs.org/{name}"))?;
            let v: serde_json::Value =
                serde_json::from_str(&doc).map_err(|e| format!("registry JSON: {e}"))?;
            let t = v["time"][version.as_str()]
                .as_str()
                .ok_or_else(|| format!("{name} {version} is not in the npm registry"))?;
            parse_time(t).map(Some)
        }
        Item::Crate { name, version } => {
            let index = f.get(&format!("https://index.crates.io/{}", index_path(name)))?;
            for line in index.lines() {
                if json_str(line, "vers").as_deref() == Some(version.as_str()) {
                    return json_str(line, "pubtime")
                        .map(|t| parse_time(&t))
                        .transpose();
                }
            }
            Err(format!("{name} {version} is not in the crates.io index"))
        }
        Item::Toolchain { version } => {
            let m = f.get(&format!(
                "https://static.rust-lang.org/dist/channel-rust-{version}.toml"
            ))?;
            let date = m
                .lines()
                .find_map(|l| l.strip_prefix("date = \""))
                .and_then(|l| l.strip_suffix('"'))
                .ok_or("the release manifest has no date")?;
            parse_time(&format!("{date}T00:00:00Z")).map(Some)
        }
        Item::Bin {
            released,
            repo,
            tag,
            version,
            ..
        } => {
            let released = released
                .as_deref()
                .ok_or("tools/versions.toml has no `released` date for it")?;
            let t = parse_time(&format!("{released}T00:00:00Z"))?;
            if let Some(repo) = repo {
                let tag = tag.clone().unwrap_or_else(|| format!("v{version}"));
                let rel = f.get(&format!(
                    "https://api.github.com/repos/{repo}/releases/tags/{tag}"
                ))?;
                let at =
                    json_str(&rel, "published_at").ok_or("GitHub release has no published_at")?;
                let real = parse_time(&at)?;
                // The written date may round down to the day, never up.
                if t > real || real - t >= DAY {
                    return Err(format!(
                        "`released = \"{released}\"` does not match GitHub's {}",
                        fmt_date(real)
                    ));
                }
                return Ok(Some(real));
            }
            Ok(Some(t))
        }
    }
}

pub struct Fetcher {
    dir: Option<PathBuf>,
}

impl Fetcher {
    fn from_env() -> Self {
        Fetcher {
            dir: std::env::var_os("DEPAGE_FETCH_DIR").map(PathBuf::from),
        }
    }
    /// GET a URL. With DEPAGE_FETCH_DIR set, read `<dir>/<host>/<path>` instead.
    fn get(&self, url: &str) -> Result<String, String> {
        if let Some(dir) = &self.dir {
            let rel = url.split_once("://").map_or(url, |(_, r)| r);
            return std::fs::read_to_string(dir.join(rel)).map_err(|e| format!("{url}: {e}"));
        }
        let mut cmd = Command::new("curl");
        cmd.args([
            "-fsSL",
            "--retry",
            "3",
            "--max-time",
            "30",
            "-A",
            "firelight-depage",
        ]);
        if url.starts_with("https://api.github.com/")
            && let Ok(tok) = std::env::var("GH_TOKEN")
        {
            cmd.args(["-H", &format!("Authorization: Bearer {tok}")]);
        }
        let out = cmd.arg(url).output().map_err(|e| format!("curl: {e}"))?;
        if !out.status.success() {
            return Err(format!("GET {url} failed ({})", out.status));
        }
        String::from_utf8(out.stdout).map_err(|e| format!("{url}: {e}"))
    }
}

pub const NPM_LOCK: &str = "tests/browser/package-lock.json";

/// (name, version) of every package in an npm lockfile (v2/v3).
/// `(name, version)` of every package in an npm lockfile (v2 or v3). Anything
/// else is an error, so new packages can never pass unchecked.
pub fn npm_packages(lock: &str) -> Result<BTreeSet<(String, String)>, String> {
    let v: serde_json::Value =
        serde_json::from_str(lock).map_err(|e| format!("{NPM_LOCK} is not valid JSON: {e}"))?;
    if v["lockfileVersion"].as_u64().is_none_or(|n| n < 2) {
        return Err(format!("{NPM_LOCK} needs lockfileVersion 2 or 3"));
    }
    let Some(pkgs) = v["packages"].as_object() else {
        return Err(format!("{NPM_LOCK} has no \"packages\" object"));
    };
    Ok(pkgs
        .iter()
        .filter_map(|(path, p)| {
            let name = path
                .rsplit("node_modules/")
                .next()
                .filter(|n| !n.is_empty() && path.contains("node_modules/"))?;
            Some((name.to_string(), p["version"].as_str()?.to_string()))
        })
        .collect())
}

/// The sparse index path of a crate name.
pub fn index_path(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    match n.len() {
        1 => format!("1/{n}"),
        2 => format!("2/{n}"),
        3 => format!("3/{}/{n}", &n[..1]),
        _ => format!("{}/{}/{n}", &n[..2], &n[2..4]),
    }
}

/// A top-level string field of a compact JSON object (index lines and the
/// GitHub release object). Good enough for the fields depage reads: none of
/// them repeats inside a nested object before the top-level one.
pub fn json_str(json: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":");
    let start = json.find(&pat)? + pat.len();
    let rest = json[start..].trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// (name, version) of every crates.io package in a Cargo.lock.
pub fn lock_packages(lock: &str) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    let mut cur: BTreeMap<&str, &str> = BTreeMap::new();
    let mut flush = |cur: &mut BTreeMap<&str, &str>| {
        if let (Some(n), Some(v), Some(s)) =
            (cur.get("name"), cur.get("version"), cur.get("source"))
            && s.starts_with("registry+https://github.com/rust-lang/crates.io-index")
                | s.starts_with("sparse+https://index.crates.io/")
        {
            out.insert((n.to_string(), v.to_string()));
        }
        cur.clear();
    };
    for line in lock.lines() {
        if line.starts_with('[') {
            flush(&mut cur);
            continue;
        }
        if let Some((k, v)) = line.split_once(" = ")
            && let Some(v) = v.strip_prefix('"').and_then(|v| v.strip_suffix('"'))
        {
            cur.insert(k.trim(), v);
        }
    }
    flush(&mut cur);
    out
}

/// A strict flat TOML subset: `[section]` headers, `key = "string"` lines and
/// comments. Anything else is an error, so a typo cannot pass silently.
pub fn flat_toml(s: &str) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section = String::new();
    for (i, raw) in s.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_string();
            out.entry(section.clone()).or_default();
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or(format!("line {}: expected key = \"value\"", i + 1))?;
        let v = v.trim();
        let v = if let Some(q) = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            q.to_string()
        } else if v.starts_with('[') {
            // Arrays (rust-toolchain.toml's components) are kept raw.
            v.to_string()
        } else {
            return Err(format!("line {}: values must be quoted strings", i + 1));
        };
        out.entry(section.clone())
            .or_default()
            .insert(k.trim().to_string(), v);
    }
    Ok(out)
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

/// A file at a revision; `None` when it doesn't exist there.
fn git_show(root: &Path, rev: &str, path: &str) -> Result<Option<String>, String> {
    git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .map_err(|_| format!("unknown revision {rev}"))?;
    match git(root, &["show", &format!("{rev}:{path}")]) {
        Ok(s) => Ok(Some(s)),
        Err(_) => Ok(None),
    }
}

/// RFC 3339 → Unix seconds. Accepts `Z` and `±HH:MM` offsets and fractions.
pub fn parse_time(s: &str) -> Result<i64, String> {
    let bad = || format!("not an RFC 3339 time: {s}");
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return Err(bad());
    }
    let num =
        |r: std::ops::Range<usize>| s.get(r).and_then(|x| x.parse::<i64>().ok()).ok_or_else(bad);
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "Z" | "z" => 0,
        r if r.len() == 6 && (r.starts_with('+') || r.starts_with('-')) => {
            let oh: i64 = r[1..3].parse().map_err(|_| bad())?;
            let om: i64 = r[4..6].parse().map_err(|_| bad())?;
            let o = oh * 3600 + om * 60;
            if r.starts_with('+') { o } else { -o }
        }
        _ => return Err(bad()),
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return Err(bad());
    }
    Ok(days_from_civil(y, mo, d) * DAY + h * 3600 + mi * 60 + se - offset)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn fmt_date(t: i64) -> String {
    let z = t.div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> i64 {
        parse_time(s).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn times() {
        assert_eq!(t("1970-01-01T00:00:00Z"), 0);
        assert_eq!(t("2026-11-05T12:00:00+01:00"), t("2026-11-05T11:00:00Z"));
        assert_eq!(t("2026-07-18T23:05:13.123Z"), t("2026-07-18T23:05:13Z"));
        assert_eq!(fmt_date(t("2024-02-29T10:00:00Z")), "2024-02-29");
        assert!(parse_time("2026-11-05").is_err());
        assert!(parse_time("2026-13-05T00:00:00Z").is_err());
    }

    #[test]
    fn index_paths() {
        assert_eq!(index_path("a"), "1/a");
        assert_eq!(index_path("ab"), "2/ab");
        assert_eq!(index_path("abc"), "3/a/abc");
        assert_eq!(index_path("Serde"), "se/rd/serde");
    }

    #[test]
    fn lockfile() {
        let lock = r#"
version = 4

[[package]]
name = "firelight"
version = "0.0.0"

[[package]]
name = "serde"
version = "1.0.229"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc"

[[package]]
name = "gitdep"
version = "0.1.0"
source = "git+https://example.com/x"
"#;
        let p = lock_packages(lock);
        assert_eq!(p.len(), 1);
        assert!(p.contains(&("serde".to_string(), "1.0.229".to_string())));
    }

    #[test]
    fn flat_toml_is_strict() {
        let ok = flat_toml("[a]\nversion = \"1\" \n# c\n[b]\nx = [\"y\"]\n").unwrap_or_default();
        assert_eq!(ok["a"]["version"], "1");
        assert!(flat_toml("[a]\nversion = 1\n").is_err());
        assert!(flat_toml("[a]\nnonsense\n").is_err());
    }

    fn fixture() -> (tempdir::Dir, Fetcher) {
        let d = tempdir::Dir::new();
        d.write(
            "index.crates.io/se/rd/serde",
            "{\"name\":\"serde\",\"vers\":\"1.0.0\",\"deps\":[]}\n\
             {\"name\":\"serde\",\"vers\":\"1.0.1\",\"deps\":[],\"pubtime\":\"2026-10-01T00:00:00Z\"}\n\
             {\"name\":\"serde\",\"vers\":\"1.0.2\",\"deps\":[{\"req\":\"^1\"}],\"pubtime\":\"2026-11-03T00:00:00Z\"}\n",
        );
        d.write(
            "static.rust-lang.org/dist/channel-rust-1.99.1.toml",
            "manifest-version = \"2\"\ndate = \"2026-10-15\"\n",
        );
        d.write(
            "api.github.com/repos/o/tool/releases/tags/v1.2.3",
            "{\"tag_name\":\"v1.2.3\",\"published_at\":\"2026-10-01T09:30:00Z\"}",
        );
        let f = Fetcher {
            dir: Some(d.path().to_path_buf()),
        };
        (d, f)
    }

    fn krate(v: &str) -> Item {
        Item::Crate {
            name: "serde".into(),
            version: v.into(),
        }
    }

    #[test]
    fn ages() {
        let (_d, f) = fixture();
        let now = t("2026-11-05T00:00:00Z");
        assert!(!check(&[krate("1.0.0"), krate("1.0.1")], &[], now, false, &f).failed);
        let r = check(&[krate("1.0.2")], &[], now, false, &f);
        assert!(r.failed, "{}", r.text);
        assert!(r.text.contains("less than 7 days"));
        // Unknown version: fails on PRs, warns on main.
        assert!(check(&[krate("9.9.9")], &[], now, false, &f).failed);
        assert!(!check(&[krate("9.9.9")], &[], now, true, &f).failed);
        let tc = Item::Toolchain {
            version: "1.99.1".into(),
        };
        assert!(!check(std::slice::from_ref(&tc), &[], now, false, &f).failed);
        assert!(check(&[tc], &[], t("2026-10-20T00:00:00Z"), false, &f).failed);
    }

    #[test]
    fn npm() {
        let lock = r#"{"lockfileVersion":3,"packages":{"":{"name":"x"},"node_modules/@playwright/test":{"version":"1.64.1"},"node_modules/a/node_modules/b":{"version":"2.0.0"}}}"#;
        let p = npm_packages(lock).unwrap_or_else(|e| panic!("{e}"));
        for bad in [
            "not json",
            "{}",
            r#"{"lockfileVersion":1,"dependencies":{}}"#,
            r#"{"lockfileVersion":3}"#,
        ] {
            assert!(npm_packages(bad).is_err(), "{bad}");
        }
        assert!(p.contains(&("@playwright/test".to_string(), "1.64.1".to_string())));
        assert!(p.contains(&("b".to_string(), "2.0.0".to_string())));
        assert_eq!(p.len(), 2);
        let (d, f) = fixture();
        d.write(
            "registry.npmjs.org/@playwright/test",
            r#"{"time":{"1.64.1":"2026-10-14T11:58:13.696Z","1.65.0":"2026-11-04T00:00:00.000Z"}}"#,
        );
        let now = t("2026-11-05T00:00:00Z");
        let item = |v: &str| Item::Npm {
            name: "@playwright/test".into(),
            version: v.into(),
        };
        assert!(!check(&[item("1.64.1")], &[], now, false, &f).failed);
        assert!(check(&[item("1.65.0")], &[], now, false, &f).failed);
    }

    #[test]
    fn exceptions() {
        let (_d, f) = fixture();
        let now = t("2026-11-05T00:00:00Z");
        let ex = parse_exceptions("serde 1.0.2 2026-11-20 RUSTSEC-0000-0000\n").unwrap_or_default();
        assert!(!check(&[krate("1.0.2")], &ex, now, false, &f).failed);
        let expired = parse_exceptions("serde 1.0.2 2026-11-01 old\n").unwrap_or_default();
        assert!(check(&[], &expired, now, false, &f).failed);
        let long = parse_exceptions("serde 1.0.2 2027-01-01 too long\n").unwrap_or_default();
        assert!(check(&[], &long, now, false, &f).failed);
        assert!(parse_exceptions("serde 1.0.2\n").is_err());
    }

    #[test]
    fn bin_tools() {
        let (_d, f) = fixture();
        let now = t("2026-11-05T00:00:00Z");
        let bin = |released: &str| Item::Bin {
            name: "tool".into(),
            version: "1.2.3".into(),
            released: Some(released.into()),
            repo: Some("o/tool".into()),
            tag: None,
        };
        assert!(!check(&[bin("2026-10-01")], &[], now, false, &f).failed);
        // A date that doesn't match GitHub's release is refused.
        assert!(check(&[bin("2026-10-30")], &[], now, false, &f).failed);
    }

    /// A tiny temp dir, so the tests need no crate.
    mod tempdir {
        use std::path::{Path, PathBuf};
        pub struct Dir(PathBuf);
        impl Dir {
            pub fn new() -> Self {
                let p = std::env::temp_dir().join(format!(
                    "depage-test-{}-{:?}",
                    std::process::id(),
                    std::thread::current().id()
                ));
                let _ = std::fs::remove_dir_all(&p);
                std::fs::create_dir_all(&p).unwrap_or_else(|e| panic!("{e}"));
                Dir(p)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
            pub fn write(&self, rel: &str, s: &str) {
                let p = self.0.join(rel);
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{e}"));
                }
                std::fs::write(p, s).unwrap_or_else(|e| panic!("{e}"));
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
