//! Firelight: the docs your repos write about themselves.
//!
//! Bootstrap stub (docs/HANDOVER.md §22, M1 replaces it): only `version`, so
//! the toolchain, lints and `cargo deny` have a package to work on.

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("version") | Some("--version") | Some("-V") => {
            println!("firelight {VERSION}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: firelight version");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_semver() {
        let parts: Vec<&str> = super::VERSION.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.parse::<u64>().is_ok()));
    }
}
