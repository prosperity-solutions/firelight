//! dir, git and GitHub sources: fetching, polling, GitHub App tokens, webhook verification.
//!
//! M2 holds the `.firelight/` folder reader; M3 adds the rest.

pub mod dir;

pub use dir::{DirError, read_firelight_dir};
