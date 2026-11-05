//! Embeds the page assets (§18): every `web/*.css`, `web/*.js` and
//! `web/*.svg`, as `ASSETS: &[(&str, &[u8])]`. No Node, no build step: the
//! files are served exactly as written.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let web = Path::new(&manifest).join("../../web");
    println!("cargo:rerun-if-changed={}", web.display());
    let mut files: Vec<(String, String)> = std::fs::read_dir(&web)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let keep =
                        name.ends_with(".css") || name.ends_with(".js") || name.ends_with(".svg");
                    (keep && name != "theme.css").then(|| (name, e.path().display().to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut src = String::from(
        "/// The page assets, embedded at build time.\npub static ASSETS: &[(&str, &[u8])] = &[\n",
    );
    for (name, abs) in &files {
        println!("cargo:rerun-if-changed={abs}");
        let _ = writeln!(src, "    ({name:?}, include_bytes!({abs:?})),");
    }
    src.push_str("];\n");
    let out = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".into());
    if let Err(e) = std::fs::write(Path::new(&out).join("assets.rs"), src) {
        println!("cargo:warning=cannot write assets.rs: {e}");
    }
}
