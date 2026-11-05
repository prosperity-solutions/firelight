//! Entry points for the fuzz targets in `fuzz/` (cargo-fuzz, nightly, local)
//! and for the seed corpus, which runs on stable as ordinary tests
//! (`tests/fuzz_seeds.rs`, §13.1). They must never panic, whatever the bytes.

use crate::catalog::{self, Strictness};
use crate::{InputFile, Limits, parse_repo};

const REPO: &[u8] = b"format: 1\nid: fuzz\nname: Fuzz\nsummary: Fuzzing.\n";

fn run(files: Vec<InputFile>) {
    let (repo, _) = parse_repo(&files, None, &Limits::default());
    if let Some(r) = repo {
        let _ = catalog::build(vec![r], Strictness::Lenient);
    }
}

fn file(path: &str, data: &[u8]) -> InputFile {
    InputFile {
        path: path.into(),
        contents: Some(data.to_vec()),
        size: data.len() as u64,
    }
}

/// Arbitrary bytes as each YAML file of the format.
pub fn yaml(data: &[u8]) {
    run(vec![file("repo.yaml", data)]);
    for p in ["links.yaml", "islands/x.yaml", "golden.yaml", "spaces.yaml"] {
        run(vec![file("repo.yaml", REPO), file(p, data)]);
    }
}

/// Arbitrary bytes as an entry (front matter and markdown) and as about.md.
pub fn markdown(data: &[u8]) {
    run(vec![
        file("repo.yaml", REPO),
        file("guides/x.md", data),
        file("about.md", data),
    ]);
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = crate::markdown::scan(s, "fuzz", "x.md", 1);
        let _ = crate::template::check_url(s, &["v"]);
    }
}
