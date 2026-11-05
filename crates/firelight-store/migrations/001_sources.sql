-- 001: sources and their last good snapshots (docs/HANDOVER.md §7).
-- The database is a cache: git is the truth, and deleting the file loses
-- nothing. Forward-only; never edit a migration once it has shipped.

-- Per source: what is served, what is newest, and how the last pull went.
CREATE TABLE source_state (
    source_id           TEXT PRIMARY KEY NOT NULL,
    -- The last good commit: the newest one that passed validation, served.
    served_commit       TEXT,
    served_committed_at TEXT,
    -- The newest commit seen on the remote, and why it isn't served (JSON
    -- diagnostics) when it failed validation.
    newest_commit       TEXT,
    newest_diagnostics  TEXT,
    last_pull_at        TEXT,
    last_pull_ms        INTEGER,
    last_pull_error     TEXT
) STRICT;

-- The .firelight/ files of each source's served commit.
CREATE TABLE source_files (
    source_id TEXT NOT NULL,
    path      TEXT NOT NULL,
    kind      TEXT NOT NULL CHECK (kind IN ('regular', 'symlink', 'special')),
    size      INTEGER NOT NULL,
    contents  BLOB,
    PRIMARY KEY (source_id, path)
) STRICT;

-- Staleness inputs (§9.5) from git history: per entry, when the entry file
-- and when its covered paths last changed (RFC 3339, UTC).
CREATE TABLE entry_history (
    source_id     TEXT NOT NULL,
    entry_path    TEXT NOT NULL,
    entry_changed TEXT,
    covers_changed TEXT,
    PRIMARY KEY (source_id, entry_path)
) STRICT;
