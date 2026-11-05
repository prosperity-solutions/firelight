//! The rules .github/workflows/ci.yml must keep (docs/HANDOVER.md §17.3,
//! maintenance-manager §17.2). zizmor and actionlint run beside these in `lint`.

use harness::{read, root, yaml};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const CI: &str = ".github/workflows/ci.yml";

/// Jobs that publish. Only they may hold write permissions or OIDC.
fn is_release_job(name: &str) -> bool {
    matches!(
        name,
        "version" | "publish" | "attest" | "release" | "verify" | "site" | "pages"
    ) || name.starts_with("build-")
}

/// Jobs without harden-runner: attest (first-party actions only) and pages
/// (exactly one deploy step), as maintenance-manager §17.2; and the two macOS
/// builds, where harden-runner's block mode doesn't enforce (§17.3).
const NO_HARDEN: &[&str] = &["attest", "pages", "build-macos-arm64", "build-macos-amd64"];
const MACOS: &[&str] = &["build-macos-arm64", "build-macos-amd64"];

/// The only third-party actions allowed (§17.3); fixed for the run.
const THIRD_PARTY: &[&str] = &["step-security/harden-runner", "zizmorcore/zizmor-action"];

fn jobs() -> BTreeMap<String, Value> {
    let ci = yaml(CI);
    ci["jobs"]
        .as_object()
        .unwrap_or_else(|| panic!("ci.yml has no jobs"))
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn steps(job: &Value) -> Vec<Value> {
    job["steps"].as_array().cloned().unwrap_or_default()
}

fn needs(job: &Value) -> Vec<String> {
    match &job["needs"] {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => vec![],
    }
}

#[test]
fn only_one_workflow() {
    let dir = root().join(".github/workflows");
    let names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    assert_eq!(
        names,
        vec!["ci.yml".to_string()],
        "ci.yml is the only workflow"
    );
}

#[test]
fn triggers_are_push_and_pull_request_only() {
    let ci = yaml(CI);
    let on = ci["on"]
        .as_object()
        .unwrap_or_else(|| panic!("on: must be a map"));
    let keys: BTreeSet<&str> = on.keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["push", "pull_request"]));
    assert_eq!(on["push"]["branches"], serde_json::json!(["main"]));
}

#[test]
fn top_level_permissions_are_empty() {
    let ci = yaml(CI);
    assert_eq!(
        ci["permissions"],
        serde_json::json!({}),
        "top-level permissions: {{}}"
    );
}

#[test]
fn every_uses_is_pinned_and_allowed() {
    let text = read(CI);
    let mut n = 0;
    for (i, line) in text.lines().enumerate() {
        let l = line.trim_start().trim_start_matches("- ");
        let Some(rest) = l.strip_prefix("uses: ") else {
            continue;
        };
        n += 1;
        let (spec, comment) = rest.split_once(" # ").unwrap_or((rest, ""));
        let (action, sha) = spec
            .split_once('@')
            .unwrap_or_else(|| panic!("ci.yml:{}: uses without @", i + 1));
        assert!(
            sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
            "ci.yml:{}: {action} must be pinned to a full commit SHA",
            i + 1
        );
        assert!(
            comment.starts_with('v'),
            "ci.yml:{}: add a `# vX.Y.Z` comment",
            i + 1
        );
        let owner = action.split('/').next().unwrap_or("");
        let repo: String = action.split('/').take(2).collect::<Vec<_>>().join("/");
        assert!(
            owner == "actions" || owner == "github" || THIRD_PARTY.contains(&repo.as_str()),
            "ci.yml:{}: {action} is not an allowed action (GitHub-owned, or {THIRD_PARTY:?})",
            i + 1
        );
    }
    assert!(n > 0);
    assert!(!text.contains("docker://"), "no docker:// actions");
}

#[test]
fn jobs_follow_the_rules() {
    let text = read(CI);
    assert!(
        !text.contains("secrets."),
        "no secrets. expressions anywhere"
    );
    for (name, job) in jobs() {
        assert!(
            job["timeout-minutes"].is_number(),
            "{name}: timeout-minutes"
        );
        if name != "gate" {
            assert!(
                !needs(&job).is_empty(),
                "{name}: must need gate (directly or transitively)"
            );
        }
        let st = steps(&job);
        // harden-runner first, in block mode, with an allow-list.
        if !NO_HARDEN.contains(&name.as_str()) {
            let first = st.first().unwrap_or_else(|| panic!("{name}: no steps"));
            let uses = first["uses"].as_str().unwrap_or("");
            assert!(
                uses.starts_with("step-security/harden-runner@"),
                "{name}: the first step must be harden-runner"
            );
            assert_eq!(
                first["with"]["egress-policy"], "block",
                "{name}: egress-policy: block"
            );
            assert!(
                first["with"]["allowed-endpoints"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty()),
                "{name}: allowed-endpoints"
            );
        } else if MACOS.contains(&name.as_str()) {
            assert!(
                job["runs-on"].as_str().unwrap_or("").starts_with("macos-"),
                "{name}: only macOS jobs may skip harden-runner"
            );
        } else {
            for s in &st {
                assert!(s["run"].is_null(), "{name}: no run: steps");
                let u = s["uses"].as_str().unwrap_or("");
                assert!(
                    u.starts_with("actions/"),
                    "{name}: first-party actions only"
                );
                assert!(!u.starts_with("actions/checkout@"), "{name}: no checkout");
            }
        }
        for s in &st {
            let uses = s["uses"].as_str().unwrap_or("");
            if uses.starts_with("actions/checkout@") {
                assert_eq!(
                    s["with"]["persist-credentials"],
                    Value::Bool(false),
                    "{name}: checkout needs persist-credentials: false"
                );
            }
            if let Some(run) = s["run"].as_str() {
                assert!(
                    !run.contains("${{"),
                    "{name}: no ${{{{ }}}} in run: (pass values through env:)"
                );
                for l in run.lines() {
                    let l = l.trim();
                    if let Some(rest) = l
                        .split("cargo ")
                        .nth(1)
                        .filter(|_| l.starts_with("cargo ") || l.contains(" cargo "))
                    {
                        let sub = rest.split_whitespace().next().unwrap_or("");
                        if matches!(
                            sub,
                            "build"
                                | "test"
                                | "clippy"
                                | "run"
                                | "install"
                                | "doc"
                                | "check"
                                | "about"
                                | "deny"
                        ) {
                            assert!(l.contains("--locked"), "{name}: `{l}` needs --locked");
                        }
                    }
                }
            }
            if !is_release_job(&name) {
                continue;
            }
            assert!(
                !uses.starts_with("actions/cache"),
                "{name}: release jobs use no cache"
            );
        }
        // Privileged permissions only in release jobs.
        let perms = &job["permissions"];
        for (k, v) in perms.as_object().into_iter().flatten() {
            let w = v.as_str() == Some("write");
            if w && matches!(
                k.as_str(),
                "contents" | "packages" | "id-token" | "attestations" | "pages"
            ) {
                assert!(
                    is_release_job(&name),
                    "{name}: {k}: write is for release jobs only"
                );
            }
        }
        if is_release_job(&name) {
            let cond = job["if"].as_str().unwrap_or("");
            assert!(
                cond.contains("!cancelled()"),
                "{name}: needs an explicit !cancelled() if:"
            );
        }
    }
}

#[test]
fn every_job_skips_drafts_through_gate() {
    let jobs = jobs();
    let gate = &jobs["gate"];
    assert!(
        gate["if"].as_str().unwrap_or("").contains("draft == false"),
        "gate must skip drafts"
    );
    // Every job reaches gate through needs.
    fn reaches(name: &str, jobs: &BTreeMap<String, Value>, seen: &mut BTreeSet<String>) -> bool {
        if name == "gate" {
            return true;
        }
        if !seen.insert(name.to_string()) {
            return false;
        }
        needs(&jobs[name]).iter().any(|n| reaches(n, jobs, seen))
    }
    for name in jobs.keys() {
        assert!(
            reaches(name, &jobs, &mut BTreeSet::new()),
            "{name} must need gate"
        );
    }
}

#[test]
fn all_green_needs_every_check_job() {
    let jobs = jobs();
    let ag = &jobs["all-green"];
    assert_eq!(ag["if"], "always()");
    let n: BTreeSet<String> = needs(ag).into_iter().collect();
    for name in jobs.keys() {
        if name == "all-green" || is_release_job(name) {
            continue;
        }
        assert!(n.contains(name), "all-green must need {name}");
    }
}

#[test]
fn release_binaries_come_from_four_named_jobs() {
    let jobs = jobs();
    for t in ["linux-amd64", "linux-arm64", "macos-arm64", "macos-amd64"] {
        let name = format!("build-{t}");
        let job = jobs.get(&name).unwrap_or_else(|| panic!("no {name} job"));
        assert!(
            job["outputs"]["sha256"].is_string(),
            "{name} exposes its sha256"
        );
        let up = steps(job)
            .into_iter()
            .filter(|s| {
                s["uses"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("actions/upload-artifact@")
            })
            .count();
        assert_eq!(up, 1, "{name} uploads exactly one artifact");
    }
    let attest = steps(&jobs["attest"]);
    assert_eq!(
        attest.len(),
        7,
        "attest: 4 archives, SHA256SUMS, image, chart"
    );
    for s in attest {
        assert!(
            s["with"]["subject-digest"]
                .as_str()
                .is_some_and(|d| d.contains("needs.")),
            "attest digests come from job outputs"
        );
    }
}

#[test]
fn only_our_push_targets() {
    // packages: write is used only for our image and our chart.
    let text = read(CI);
    for (i, l) in text.lines().enumerate() {
        for word in l.split_whitespace() {
            let w = word.trim_matches(|c| c == '"' || c == '\'');
            if let Some(rest) = w
                .strip_prefix("ghcr.io/")
                .or_else(|| w.strip_prefix("oci://ghcr.io/"))
            {
                let ok = rest.starts_with("prosperity-solutions/firelight")
                    || rest.starts_with("prosperity-solutions/charts")
                    || rest.starts_with("$REPO")
                    || rest.starts_with("$OWNER")
                    || rest.starts_with("%s");
                assert!(ok, "ci.yml:{}: unexpected registry target {w}", i + 1);
            }
        }
    }
}

#[test]
fn harden_runner_is_v2_16_or_later() {
    for l in read(CI)
        .lines()
        .filter(|l| l.contains("step-security/harden-runner@"))
    {
        let v = l.rsplit("# v").next().unwrap_or("");
        let mut p = v.split('.').map(|x| x.parse::<u32>().unwrap_or(0));
        let (maj, min) = (p.next().unwrap_or(0), p.next().unwrap_or(0));
        assert!(
            maj > 2 || (maj == 2 && min >= 16),
            "harden-runner ≥ v2.16: {l}"
        );
    }
}

#[test]
fn public_id_pattern_in_sync() {
    let script = read("scripts/dev/public-ids.sh");
    let pat = script
        .lines()
        .find_map(|l| {
            l.strip_prefix("PATTERN='")
                .and_then(|l| l.strip_suffix('\''))
        })
        .unwrap_or_else(|| panic!("public-ids.sh: no PATTERN='…' line"));
    assert!(
        read(CI).contains(&format!("grep -Eo '{pat}'")),
        "ci.yml's commits job must use public-ids.sh's pattern"
    );
}

#[test]
fn dependabot_has_the_cooldown() {
    let d = yaml(".github/dependabot.yml");
    let ups = d["updates"]
        .as_array()
        .unwrap_or_else(|| panic!("no updates"));
    let eco: BTreeSet<String> = ups
        .iter()
        .map(|u| {
            assert_eq!(
                u["cooldown"]["default-days"], 7,
                "cooldown default-days 7: {u}"
            );
            assert_eq!(u["schedule"]["interval"], "weekly");
            u["package-ecosystem"].as_str().unwrap_or("").to_string()
        })
        .collect();
    let mut want = BTreeSet::from(["cargo".to_string(), "github-actions".to_string()]);
    if root().join("Dockerfile").exists() {
        want.insert("docker".into());
    }
    if root().join("package-lock.json").exists()
        || root().join("tests/browser/package-lock.json").exists()
    {
        want.insert("npm".into());
    }
    assert_eq!(eco, want);
}

#[test]
fn scripts_use_locked_cargo() {
    // Every cargo command in scripts uses --locked (§17.3), except the one
    // that updates the lockfile and the toolchain-only fmt.
    for entry in walk(&root().join("scripts")) {
        let rel = entry
            .strip_prefix(root())
            .unwrap_or(&entry)
            .to_string_lossy()
            .into_owned();
        if rel.ends_with("cargo-update.sh") || rel.contains("testdata") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for (i, l) in text.lines().enumerate() {
            let t = l.trim_start();
            if t.starts_with('#') {
                continue;
            }
            for sub in ["build", "test", "clippy", "run", "install"] {
                if t.contains(&format!("cargo {sub} ")) {
                    assert!(
                        t.contains("--locked"),
                        "{rel}:{}: cargo {sub} needs --locked",
                        i + 1
                    );
                }
            }
        }
    }
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
