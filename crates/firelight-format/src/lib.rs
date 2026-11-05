//! The `.firelight/` format, v1 (docs/HANDOVER.md §5; normative reference:
//! docs/format.md).
//!
//! Types, YAML and markdown parsing, validation and ref resolution. **No I/O
//! and no async** (§4): callers hand in the files of a `.firelight/` folder
//! already read, and get back a parsed [`Repo`] plus every [`Diagnostic`].
//! [`catalog::build`] merges many repos into the page model: spaces, islands
//! with their cards and links, the Unsorted fallback, resolved refs.
//!
//! Content is untrusted (§17.1): sizes are capped before parsing, YAML runs
//! under a tight budget (no billion laughs), symlinks are refused, URL schemes
//! are allow-listed and template URLs can only fill safe positions.

pub mod catalog;
pub mod diag;
pub mod fuzz;
pub mod icons;
pub mod ids;
pub mod markdown;
pub mod model;
pub mod pos;
pub mod raw;
pub mod repo;
pub mod schema;
pub mod template;
mod yaml;

pub use diag::{Diagnostic, Severity};
pub use ids::{Ref, valid_id};
pub use model::*;
pub use repo::{InputFile, Limits, parse_repo};

/// `.firelight/` format versions this crate reads.
pub const FORMAT_VERSIONS: &[u32] = &[1];

/// Reserved repo id: Firelight's own docs (§5.4, §6.4).
pub const SELF_REPO: &str = "firelight";

/// Reserved space ids (§5.7).
pub const SPACE_ALL: &str = "all";
pub const SPACE_DEFAULT: &str = "default";
pub const SPACE_FIRELIGHT: &str = "firelight";
pub const RESERVED_SPACES: &[&str] = &[SPACE_ALL, SPACE_DEFAULT, SPACE_FIRELIGHT];
