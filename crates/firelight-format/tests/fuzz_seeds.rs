//! The fuzz seed corpus on stable (§13.1): every seed through the same entry
//! points the cargo-fuzz targets use. A panic here is a bug.

use std::path::Path;

fn seeds(kind: &str) -> Vec<std::path::PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/corpus")
        .join(kind);
    let mut v: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    assert!(v.len() >= 5, "the {kind} corpus is too small");
    v
}

#[test]
fn yaml_seeds() {
    for p in seeds("yaml") {
        firelight_format::fuzz::yaml(&std::fs::read(&p).unwrap_or_default());
    }
}

#[test]
fn markdown_seeds() {
    for p in seeds("markdown") {
        firelight_format::fuzz::markdown(&std::fs::read(&p).unwrap_or_default());
    }
}
