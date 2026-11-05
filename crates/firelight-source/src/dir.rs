//! Read a `.firelight/` folder from disk, for `check` and `dir` sources.
//!
//! Only `.firelight/` is read. Symlinks are never followed: they come back as
//! files without contents, and the format refuses them. A file over the size
//! limit is not read at all. Reading stops past the file-count limit.

use std::path::Path;

use firelight_format::{InputFile, Limits};

#[derive(Debug, thiserror::Error)]
pub enum DirError {
    #[error("{0} has no .firelight/ folder")]
    Missing(String),
    #[error(".firelight/ in {0} is a symlink; Firelight never follows symlinks")]
    Symlink(String),
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

/// Every file under `<root>/.firelight/`, paths relative to `.firelight/`,
/// sorted.
pub fn read_firelight_dir(root: &Path, limits: &Limits) -> Result<Vec<InputFile>, DirError> {
    let base = root.join(".firelight");
    let shown = root.display().to_string();
    match std::fs::symlink_metadata(&base) {
        Err(_) => return Err(DirError::Missing(shown)),
        Ok(m) if m.file_type().is_symlink() => return Err(DirError::Symlink(shown)),
        Ok(m) if !m.is_dir() => return Err(DirError::Missing(shown)),
        Ok(_) => {}
    }
    let mut out = Vec::new();
    walk(&base, "", limits, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn walk(dir: &Path, rel: &str, limits: &Limits, out: &mut Vec<InputFile>) -> Result<(), DirError> {
    let io = |source, p: &Path| DirError::Io {
        path: p.display().to_string(),
        source,
    };
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| io(e, dir))?
        .collect::<Result<_, _>>()
        .map_err(|e| io(e, dir))?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        // One past the limit is enough for the format to report it.
        if out.len() as u64 > limits.max_files {
            return Ok(());
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name == ".DS_Store" {
            continue;
        }
        let path = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let p = e.path();
        let m = std::fs::symlink_metadata(&p).map_err(|e| io(e, &p))?;
        if m.file_type().is_symlink() {
            out.push(InputFile {
                path,
                contents: None,
                size: 0,
            });
        } else if m.is_dir() {
            walk(&p, &path, limits, out)?;
        } else if m.len() > limits.max_file_bytes {
            out.push(InputFile {
                path,
                contents: None,
                size: m.len(),
            });
        } else {
            let bytes = std::fs::read(&p).map_err(|e| io(e, &p))?;
            out.push(InputFile {
                size: bytes.len() as u64,
                path,
                contents: Some(bytes),
            });
        }
    }
    Ok(())
}
