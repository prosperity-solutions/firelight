//! The real default model against its model card's reference numbers (§8.2,
//! D2). Needs the network once (the model job, §13.4): run with
//! `FIRELIGHT_MODEL_TEST=1 cargo test -p firelight-embed --release --test reference -- --ignored`.
//! Models are cached in `$FIRELIGHT_MODEL_CACHE` (default `target/models`).

use std::path::PathBuf;
use std::time::Instant;

use firelight_embed::candle::Candle;
use firelight_embed::model::{self, Spec};
use firelight_embed::{Embedder, dot};

#[test]
#[ignore = "downloads the default model (1.2 GB); the model job runs it"]
fn qwen3_matches_the_model_card() {
    if std::env::var_os("FIRELIGHT_MODEL_TEST").is_none() {
        return;
    }
    let cache = std::env::var_os("FIRELIGHT_MODEL_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/models"));
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rt = tokio::runtime::Runtime::new().unwrap_or_else(|e| panic!("{e}"));
    let spec = Spec::default_model();
    let t = Instant::now();
    let dir = rt
        .block_on(model::ensure(&spec, &cache, &reqwest::Client::new()))
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("model ready in {:?}", t.elapsed());
    let t = Instant::now();
    let m = Candle::load(&dir, &format!("{}@{}", spec.repo, spec.revision))
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("loaded in {:?}, dims {}", t.elapsed(), m.dims());
    let q = ["What is the capital of China?", "Explain gravity"];
    let d = [
        "The capital of China is Beijing.",
        "Gravity is a force that attracts two bodies towards each other. It gives weight to physical objects and is responsible for the movement of planets around the sun.",
    ];
    let t = Instant::now();
    let qv: Vec<Vec<f32>> = q
        .iter()
        .map(|x| m.embed_query(x).unwrap_or_default())
        .collect();
    let dv = m.embed_documents(&d).unwrap_or_default();
    eprintln!("4 texts in {:?}", t.elapsed());
    let want = [[0.764_556_8, 0.141_425_09], [0.135_497_36, 0.599_954_96]];
    for i in 0..2 {
        for j in 0..2 {
            let got = dot(&qv[i], &dv[j]);
            eprintln!("q{i}·d{j} = {got:.4} (card {:.4})", want[i][j]);
            assert!(
                (got - want[i][j]).abs() < 1e-3,
                "q{i}·d{j} = {got}, the card says {}",
                want[i][j]
            );
        }
    }
}
