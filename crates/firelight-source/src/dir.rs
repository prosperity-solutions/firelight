//! Read a `.firelight/` folder from disk, for `check` and `dir` sources.
//!
//! Only `.firelight/` is read, and the limits hold while reading, not after:
//! - symlinks are never followed: they come back as [`FileKind::Symlink`], and
//!   a file swapped for a symlink between the check and the read is caught
//!   (the opened file must be the one that was checked);
//! - FIFOs, sockets and devices are never opened ([`FileKind::Special`]);
//! - a file over the per-file limit, or past the per-source byte limit, is not
//!   read (its size is passed on, so the format reports it);
//! - reading stops one file past the file-count limit, and folders deeper than
//!   [`MAX_DEPTH`] are not entered.

use std::io::Read;
use std::path::Path;

use firelight_format::repo::FileKind;
use firelight_format::{InputFile, Limits};

/// `.firelight/` needs two levels (`islands/x.yaml`); this leaves room for
/// mistakes to be reported without walking an arbitrarily deep tree.
pub const MAX_DEPTH: usize = 8;

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
    let mut w = Walk {
        limits,
        out: Vec::new(),
        total: 0,
    };
    w.walk(&base, "", 0)?;
    w.out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(w.out)
}

struct Walk<'a> {
    limits: &'a Limits,
    out: Vec<InputFile>,
    /// Bytes read so far.
    total: u64,
}

fn io(source: std::io::Error, p: &Path) -> DirError {
    DirError::Io {
        path: p.display().to_string(),
        source,
    }
}

impl Walk<'_> {
    fn not_read(&mut self, path: String, kind: FileKind, size: u64) {
        self.out.push(InputFile {
            path,
            kind,
            contents: None,
            size,
        });
    }

    fn walk(&mut self, dir: &Path, rel: &str, depth: usize) -> Result<(), DirError> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| io(e, dir))?
            .collect::<Result<_, _>>()
            .map_err(|e| io(e, dir))?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            // One past the limit is enough for the format to report it.
            if self.out.len() as u64 > self.limits.max_files {
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
            let ft = m.file_type();
            if ft.is_symlink() {
                self.not_read(path, FileKind::Symlink, 0);
            } else if ft.is_dir() {
                if depth < MAX_DEPTH {
                    self.walk(&p, &path, depth + 1)?;
                }
            } else if !ft.is_file() {
                self.not_read(path, FileKind::Special, 0);
            } else if m.len() > self.limits.max_file_bytes
                || self.total + m.len() > self.limits.max_source_bytes
            {
                self.not_read(path, FileKind::Regular, m.len());
            } else {
                match read_checked(&p, &m, self.limits.max_file_bytes)? {
                    Some(b) => {
                        self.total += b.len() as u64;
                        self.out.push(InputFile::regular(path, b));
                    }
                    None => self.not_read(path, FileKind::Symlink, 0),
                }
            }
        }
        Ok(())
    }
}

/// Read a regular file, making sure the file opened is the one checked
/// (same device and inode): `None` if it was swapped meanwhile. Reads at most
/// `max + 1` bytes, so a file that grew is still capped.
fn read_checked(
    p: &Path,
    checked: &std::fs::Metadata,
    max: u64,
) -> Result<Option<Vec<u8>>, DirError> {
    let f = std::fs::File::open(p).map_err(|e| io(e, p))?;
    let opened = f.metadata().map_err(|e| io(e, p))?;
    if !opened.is_file() || !same_file(checked, &opened) {
        return Ok(None);
    }
    let mut buf = Vec::new();
    f.take(max + 1)
        .read_to_end(&mut buf)
        .map_err(|e| io(e, p))?;
    Ok(Some(buf))
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.len() == b.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("fl-dir-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join(".firelight/guides")).unwrap_or_else(|e| panic!("{e}"));
        p
    }

    #[test]
    fn limits_hold_while_reading() {
        let d = tmp("limits");
        for i in 0..5 {
            std::fs::write(
                d.join(format!(".firelight/guides/g{i}.md")),
                vec![b'a'; 1000],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        let limits = Limits {
            max_files: 1000,
            max_source_bytes: 2500,
            max_file_bytes: 4096,
        };
        let files = read_firelight_dir(&d, &limits).unwrap_or_else(|e| panic!("{e}"));
        let read: u64 = files
            .iter()
            .filter_map(|f| f.contents.as_ref())
            .map(|c| c.len() as u64)
            .sum();
        assert!(read <= 2500, "read {read} bytes past the source limit");
        assert_eq!(files.iter().filter(|f| f.contents.is_none()).count(), 3);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_fifos_are_not_read() {
        let d = tmp("special");
        std::os::unix::fs::symlink("/etc/hosts", d.join(".firelight/guides/link.md"))
            .unwrap_or_else(|e| panic!("{e}"));
        let fifo = d.join(".firelight/guides/fifo.md");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|s| s.success());
        let files = read_firelight_dir(&d, &Limits::default()).unwrap_or_else(|e| panic!("{e}"));
        let kind = |n: &str| {
            files
                .iter()
                .find(|f| f.path == n)
                .map(|f| (f.kind, f.contents.is_some()))
        };
        assert_eq!(kind("guides/link.md"), Some((FileKind::Symlink, false)));
        if made {
            assert_eq!(kind("guides/fifo.md"), Some((FileKind::Special, false)));
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
