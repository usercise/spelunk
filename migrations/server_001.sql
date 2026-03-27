-- spelunk-server schema
-- One server instance can host multiple projects.

CREATE TABLE IF NOT EXISTS projects (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    slug          TEXT    NOT NULL UNIQUE,
    embedding_dim INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS notes (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id    INTEGER NOT NULL REFERENCES projects(id),
    kind          TEXT    NOT NULL DEFAULT 'note',
    title         TEXT    NOT NULL,
    body          TEXT    NOT NULL,
    tags          TEXT,
    linked_files  TEXT,
    created_at    INTEGER NOT NULL DEFAULT (unixepoch()),
    status        TEXT    NOT NULL DEFAULT 'active',
    superseded_by INTEGER REFERENCES notes(id)
);

CREATE INDEX IF NOT EXISTS idx_notes_project ON notes(project_id);
CREATE INDEX IF NOT EXISTS idx_notes_status  ON notes(project_id, status);
