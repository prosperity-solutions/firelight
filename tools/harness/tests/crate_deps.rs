//! The workspace dependency rules (docs/HANDOVER.md §4), read from
//! `cargo metadata`.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

fn metadata() -> serde_json::Value {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--locked", "--format-version", "1"])
        .current_dir(harness::root())
        .output()
        .unwrap_or_else(|e| panic!("cargo metadata: {e}"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}"))
}

/// Every package each workspace member pulls in through normal (non-dev)
/// dependencies, transitively.
fn closure() -> BTreeMap<String, BTreeSet<String>> {
    let m = metadata();
    let names: BTreeMap<String, String> = m["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            (
                p["id"].as_str().unwrap_or("").to_string(),
                p["name"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let mut edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in m["resolve"]["nodes"].as_array().into_iter().flatten() {
        let id = n["id"].as_str().unwrap_or("").to_string();
        let deps = n["deps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| {
                d["dep_kinds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|k| k["kind"].is_null() || k["kind"] == "build")
            })
            .filter_map(|d| d["pkg"].as_str().map(String::from))
            .collect();
        edges.insert(id, deps);
    }
    let mut out = BTreeMap::new();
    for member in m["workspace_members"].as_array().into_iter().flatten() {
        let start = member.as_str().unwrap_or("").to_string();
        let mut seen = BTreeSet::new();
        let mut stack = vec![start.clone()];
        while let Some(id) = stack.pop() {
            for d in edges.get(&id).into_iter().flatten() {
                if seen.insert(names.get(d).cloned().unwrap_or_default()) {
                    stack.push(d.clone());
                }
            }
        }
        out.insert(names.get(&start).cloned().unwrap_or_default(), seen);
    }
    out
}

#[test]
fn dependency_rules() {
    let c = closure();
    let format = c
        .get("firelight-format")
        .unwrap_or_else(|| panic!("no firelight-format"));
    for banned in [
        "tokio",
        "reqwest",
        "rusqlite",
        "hyper",
        "axum",
        "candle-core",
    ] {
        assert!(
            !format.contains(banned),
            "firelight-format must not depend on {banned} (§4)"
        );
    }
    assert!(
        !format.iter().any(|d| d.starts_with("firelight-")),
        "firelight-format depends on nothing in the workspace (§4)"
    );
    for (member, deps) in &c {
        if member != "firelight-store" {
            assert!(
                !deps.contains("rusqlite"),
                "{member}: only firelight-store may depend on rusqlite (§4)"
            );
        }
        if member != "firelight-embed" {
            assert!(
                !deps.iter().any(|d| d.starts_with("candle-")),
                "{member}: only firelight-embed may depend on candle (§4)"
            );
        }
        for banned in [
            "openssl",
            "openssl-sys",
            "aws-lc-rs",
            "aws-lc-sys",
            "native-tls",
        ] {
            assert!(
                !deps.contains(banned),
                "{member}: no {banned}; TLS is rustls with ring (§19)"
            );
        }
    }
}

#[test]
fn every_package_is_unpublished_and_2024() {
    let m = metadata();
    let members: BTreeSet<&str> = m["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    for p in m["packages"].as_array().into_iter().flatten() {
        if !members.contains(p["id"].as_str().unwrap_or("")) {
            continue;
        }
        let name = p["name"].as_str().unwrap_or("");
        assert_eq!(p["edition"], "2024", "{name}: edition 2024");
        assert_eq!(
            p["publish"],
            serde_json::json!([]),
            "{name}: publish = false"
        );
    }
}
