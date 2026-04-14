-- Historical code snapshots: index the codebase at a specific git commit.
-- snapshot_files and snapshot_chunks mirror the live files/chunks tables
-- but are scoped per snapshot and carry no FK to the live index.

CREATE TABLE IF NOT EXISTS snapshots (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    commit_sha  TEXT    NOT NULL UNIQUE,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    file_count  INTEGER NOT NULL DEFAULT 0,
    chunk_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS snapshot_files (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    path        TEXT    NOT NULL,
    language    TEXT,
    hash        TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS snapshot_chunks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    file_id     INTEGER NOT NULL REFERENCES snapshot_files(id) ON DELETE CASCADE,
    node_type   TEXT    NOT NULL,
    name        TEXT,
    start_line  INTEGER NOT NULL,
    end_line    INTEGER NOT NULL,
    content     TEXT    NOT NULL,
    metadata    TEXT,
    token_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_snapshots_sha          ON snapshots(commit_sha);
CREATE INDEX IF NOT EXISTS idx_snapshot_files_snap    ON snapshot_files(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_snapshot_chunks_snap   ON snapshot_chunks(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_snapshot_chunks_name   ON snapshot_chunks(name);
