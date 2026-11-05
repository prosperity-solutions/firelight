//! The SQLite cache (docs/HANDOVER.md §7). Every SQL statement lives here.
//!
//! One file, WAL mode. **Git is the truth; deleting the file loses nothing.**
//! Migrations are forward-only SQL files under `migrations/`, embedded and run
//! at startup in a transaction; the version is `PRAGMA user_version`. A
//! database newer than this binary (after a rollback) is deleted and rebuilt
//! from git, which is allowed because it is a cache.
//!
//! The API is synchronous; async callers use `spawn_blocking`.

use std::path::{Path, PathBuf};

use firelight_format::InputFile;
use firelight_format::repo::FileKind;
use rusqlite::{Connection, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};

/// The migrations, in order. Append only.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001_sources.sql"),
    include_str!("../migrations/002_embeddings.sql"),
];

pub mod fts;
pub use fts::FtsIndex;

fn migrations() -> Migrations<'static> {
    Migrations::new(MIGRATIONS.iter().map(|s| M::up(s)).collect())
}

/// The schema version this binary writes.
pub fn latest_version() -> usize {
    MIGRATIONS.len()
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database {path}: {source}")]
    Sqlite {
        path: String,
        source: rusqlite::Error,
    },
    #[error(
        "database {path}: migration failed: {source}. Delete the file to rebuild it from git (it is only a cache)"
    )]
    Migration {
        path: String,
        source: rusqlite_migration::Error,
    },
    #[error("database {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    conn: Connection,
    path: Option<PathBuf>,
}

/// A source's served snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub commit: String,
    pub committed_at: Option<String>,
    pub files: Vec<InputFile>,
    pub history: Vec<EntryHistory>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryHistory {
    /// Relative to the repo root, e.g. `.firelight/guides/x.md`.
    pub entry_path: String,
    pub entry_changed: Option<String>,
    pub covers_changed: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceState {
    pub source_id: String,
    pub served_commit: Option<String>,
    pub served_committed_at: Option<String>,
    pub newest_commit: Option<String>,
    /// JSON array of diagnostics when the newest commit failed validation.
    pub newest_diagnostics: Option<String>,
    pub last_pull_at: Option<String>,
    pub last_pull_ms: Option<i64>,
    pub last_pull_error: Option<String>,
}

fn kind_str(k: FileKind) -> &'static str {
    match k {
        FileKind::Regular => "regular",
        FileKind::Symlink => "symlink",
        FileKind::Special => "special",
    }
}

fn kind_of(s: &str) -> FileKind {
    match s {
        "symlink" => FileKind::Symlink,
        "special" => FileKind::Special,
        _ => FileKind::Regular,
    }
}

impl Store {
    /// Open (or create) the database at `path` and migrate it.
    pub fn open(path: &Path) -> Result<Store> {
        let shown = path.display().to_string();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| StoreError::Io {
                path: shown.clone(),
                source,
            })?;
        }
        let conn = Self::connect(path)?;
        let version: usize = conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .map_err(|source| StoreError::Sqlite {
                path: shown.clone(),
                source,
            })?
            .try_into()
            .unwrap_or(0);
        if version > latest_version() {
            tracing::warn!(path = %shown, found = version, supported = latest_version(), "the database is newer than this binary (a rollback?); deleting it and rebuilding from git");
            drop(conn);
            for suffix in ["", "-wal", "-shm"] {
                let p = PathBuf::from(format!("{}{suffix}", path.display()));
                if p.exists() {
                    std::fs::remove_file(&p).map_err(|source| StoreError::Io {
                        path: p.display().to_string(),
                        source,
                    })?;
                }
            }
            return Self::open(path);
        }
        let mut s = Store {
            conn,
            path: Some(path.to_path_buf()),
        };
        s.migrate()?;
        Ok(s)
    }

    /// An in-memory database, for tests.
    pub fn memory() -> Result<Store> {
        let conn = Connection::open_in_memory().map_err(|source| StoreError::Sqlite {
            path: ":memory:".into(),
            source,
        })?;
        let mut s = Store { conn, path: None };
        s.migrate()?;
        Ok(s)
    }

    fn connect(path: &Path) -> Result<Connection> {
        let shown = path.display().to_string();
        let conn = Connection::open(path).map_err(|source| StoreError::Sqlite {
            path: shown.clone(),
            source,
        })?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| StoreError::Sqlite {
                path: shown.clone(),
                source,
            })?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|source| StoreError::Sqlite {
                path: shown.clone(),
                source,
            })?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|source| StoreError::Sqlite {
                path: shown,
                source,
            })?;
        Ok(conn)
    }

    fn shown(&self) -> String {
        self.path
            .as_ref()
            .map_or(":memory:".into(), |p| p.display().to_string())
    }

    fn sql(&self, source: rusqlite::Error) -> StoreError {
        StoreError::Sqlite {
            path: self.shown(),
            source,
        }
    }

    fn migrate(&mut self) -> Result<()> {
        let path = self.shown();
        migrations()
            .to_latest(&mut self.conn)
            .map_err(|source| StoreError::Migration { path, source })
    }

    pub fn schema_version(&self) -> Result<usize> {
        let v: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|e| self.sql(e))?;
        Ok(usize::try_from(v).unwrap_or(0))
    }

    /// Bytes on disk (the main file only), or 0 in memory.
    pub fn size_bytes(&self) -> u64 {
        self.path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map_or(0, |m| m.len())
    }

    /// Replace a source's served snapshot (its last good commit).
    pub fn save_snapshot(&mut self, source_id: &str, snap: &Snapshot) -> Result<()> {
        let path = self.shown();
        let err = |source| StoreError::Sqlite {
            path: path.clone(),
            source,
        };
        let tx = self.conn.transaction().map_err(err)?;
        tx.execute(
            "DELETE FROM source_files WHERE source_id = ?1",
            params![source_id],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM entry_history WHERE source_id = ?1",
            params![source_id],
        )
        .map_err(err)?;
        {
            let mut f = tx
                .prepare("INSERT INTO source_files (source_id, path, kind, size, contents) VALUES (?1, ?2, ?3, ?4, ?5)")
                .map_err(err)?;
            for file in &snap.files {
                f.execute(params![
                    source_id,
                    file.path,
                    kind_str(file.kind),
                    file.size as i64,
                    file.contents
                ])
                .map_err(err)?;
            }
            let mut h = tx
                .prepare("INSERT INTO entry_history (source_id, entry_path, entry_changed, covers_changed) VALUES (?1, ?2, ?3, ?4)")
                .map_err(err)?;
            for e in &snap.history {
                h.execute(params![
                    source_id,
                    e.entry_path,
                    e.entry_changed,
                    e.covers_changed
                ])
                .map_err(err)?;
            }
        }
        tx.execute(
            "INSERT INTO source_state (source_id, served_commit, served_committed_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (source_id) DO UPDATE SET served_commit = excluded.served_commit, served_committed_at = excluded.served_committed_at",
            params![source_id, snap.commit, snap.committed_at],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }

    /// The served snapshot of a source, if any.
    pub fn load_snapshot(&self, source_id: &str) -> Result<Option<Snapshot>> {
        let state: Option<(Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT served_commit, served_committed_at FROM source_state WHERE source_id = ?1",
                params![source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| self.sql(e))?;
        let Some((Some(commit), committed_at)) = state else {
            return Ok(None);
        };
        let mut st = self.conn.prepare("SELECT path, kind, size, contents FROM source_files WHERE source_id = ?1 ORDER BY path").map_err(|e| self.sql(e))?;
        let files = st
            .query_map(params![source_id], |r| {
                let kind: String = r.get(1)?;
                let size: i64 = r.get(2)?;
                Ok(InputFile {
                    path: r.get(0)?,
                    kind: kind_of(&kind),
                    size: size.max(0) as u64,
                    contents: r.get(3)?,
                })
            })
            .map_err(|e| self.sql(e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| self.sql(e))?;
        let mut st = self
            .conn
            .prepare("SELECT entry_path, entry_changed, covers_changed FROM entry_history WHERE source_id = ?1 ORDER BY entry_path")
            .map_err(|e| self.sql(e))?;
        let history = st
            .query_map(params![source_id], |r| {
                Ok(EntryHistory {
                    entry_path: r.get(0)?,
                    entry_changed: r.get(1)?,
                    covers_changed: r.get(2)?,
                })
            })
            .map_err(|e| self.sql(e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| self.sql(e))?;
        Ok(Some(Snapshot {
            commit,
            committed_at,
            files,
            history,
        }))
    }

    /// Record a pull attempt: the newest commit seen and, when it couldn't be
    /// served, why (diagnostics JSON) or the pull error.
    pub fn record_pull(&mut self, s: &SourceState) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO source_state (source_id, newest_commit, newest_diagnostics, last_pull_at, last_pull_ms, last_pull_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (source_id) DO UPDATE SET newest_commit = excluded.newest_commit,
                   newest_diagnostics = excluded.newest_diagnostics, last_pull_at = excluded.last_pull_at,
                   last_pull_ms = excluded.last_pull_ms, last_pull_error = excluded.last_pull_error",
                params![s.source_id, s.newest_commit, s.newest_diagnostics, s.last_pull_at, s.last_pull_ms, s.last_pull_error],
            )
            .map_err(|e| self.sql(e))?;
        Ok(())
    }

    pub fn source_states(&self) -> Result<Vec<SourceState>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT source_id, served_commit, served_committed_at, newest_commit, newest_diagnostics, last_pull_at, last_pull_ms, last_pull_error
                 FROM source_state ORDER BY source_id",
            )
            .map_err(|e| self.sql(e))?;
        st.query_map([], |r| {
            Ok(SourceState {
                source_id: r.get(0)?,
                served_commit: r.get(1)?,
                served_committed_at: r.get(2)?,
                newest_commit: r.get(3)?,
                newest_diagnostics: r.get(4)?,
                last_pull_at: r.get(5)?,
                last_pull_ms: r.get(6)?,
                last_pull_error: r.get(7)?,
            })
        })
        .map_err(|e| self.sql(e))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| self.sql(e))
    }

    /// Drop everything about sources no longer configured.
    /// Cached vectors for `hashes` under one embedder; missing ones are absent.
    pub fn vectors(
        &self,
        model: &str,
        dims: usize,
        hashes: &[[u8; 32]],
    ) -> Result<std::collections::HashMap<[u8; 32], Vec<f32>>> {
        let mut out = std::collections::HashMap::new();
        let d = i64::try_from(dims).unwrap_or(i64::MAX);
        let mut st = self
            .conn
            .prepare_cached(
                "SELECT vec FROM embeddings WHERE model = ?1 AND dims = ?2 AND hash = ?3",
            )
            .map_err(|e| self.sql(e))?;
        for h in hashes {
            let blob: Option<Vec<u8>> = st
                .query_row(params![model, d, &h[..]], |r| r.get(0))
                .optional()
                .map_err(|e| self.sql(e))?;
            if let Some(b) = blob
                && b.len() == dims * 4
            {
                out.insert(*h, from_le(&b));
            }
        }
        Ok(out)
    }

    /// Store vectors (one transaction).
    pub fn put_vectors(&mut self, model: &str, rows: &[([u8; 32], Vec<f32>)]) -> Result<()> {
        let path = self.shown();
        let e = |source| StoreError::Sqlite {
            path: path.clone(),
            source,
        };
        let tx = self.conn.transaction().map_err(e)?;
        {
            let mut st = tx
                .prepare_cached("INSERT OR REPLACE INTO embeddings (model, dims, hash, vec) VALUES (?1, ?2, ?3, ?4)")
                .map_err(e)?;
            for (h, v) in rows {
                let dims = i64::try_from(v.len()).unwrap_or(0);
                st.execute(params![model, dims, &h[..], to_le(v)])
                    .map_err(e)?;
            }
        }
        tx.commit().map_err(e)
    }

    /// Drop every other embedder's vectors (after a model change). The active
    /// model's rows stay, so a source that is briefly missing costs nothing.
    pub fn prune_other_models(&mut self, model: &str, dims: usize) -> Result<usize> {
        let d = i64::try_from(dims).unwrap_or(i64::MAX);
        self.conn
            .execute(
                "DELETE FROM embeddings WHERE NOT (model = ?1 AND dims = ?2)",
                params![model, d],
            )
            .map_err(|e| self.sql(e))
    }

    /// Drop one embedder's vectors that no hash in `keep` uses.
    pub fn retain_vectors(&mut self, model: &str, dims: usize, keep: &[[u8; 32]]) -> Result<usize> {
        let path = self.shown();
        let e = |source| StoreError::Sqlite {
            path: path.clone(),
            source,
        };
        let d = i64::try_from(dims).unwrap_or(i64::MAX);
        let tx = self.conn.transaction().map_err(e)?;
        tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS keep_hashes (hash BLOB PRIMARY KEY); DELETE FROM keep_hashes;")
            .map_err(e)?;
        {
            let mut st = tx
                .prepare_cached("INSERT OR IGNORE INTO keep_hashes (hash) VALUES (?1)")
                .map_err(e)?;
            for h in keep {
                st.execute(params![&h[..]]).map_err(e)?;
            }
        }
        let n = tx
            .execute(
                "DELETE FROM embeddings WHERE model = ?1 AND dims = ?2 AND hash NOT IN (SELECT hash FROM keep_hashes)",
                params![model, d],
            )
            .map_err(e)?;
        tx.execute_batch("DELETE FROM keep_hashes;").map_err(e)?;
        tx.commit().map_err(e)?;
        Ok(n)
    }

    /// How many vectors the cache holds for one embedder.
    pub fn vector_count(&self, model: &str, dims: usize) -> Result<usize> {
        let d = i64::try_from(dims).unwrap_or(i64::MAX);
        let n: i64 = self
            .conn
            .query_row(
                "SELECT count(*) FROM embeddings WHERE model = ?1 AND dims = ?2",
                params![model, d],
                |r| r.get(0),
            )
            .map_err(|e| self.sql(e))?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    pub fn retain_sources(&mut self, ids: &[&str]) -> Result<()> {
        let path = self.shown();
        let err = |source| StoreError::Sqlite {
            path: path.clone(),
            source,
        };
        let tx = self.conn.transaction().map_err(err)?;
        let keep = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
        for table in ["source_files", "entry_history", "source_state"] {
            tx.execute(
                &format!(
                    "DELETE FROM {table} WHERE source_id NOT IN (SELECT value FROM json_each(?1))"
                ),
                params![keep],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)
    }
}

#[cfg(test)]
mod tests;

fn to_le(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_le(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}
