-- Server-side note relationship edges.
-- kind is constrained to: supersedes, relates_to, contradicts
-- ON DELETE CASCADE ensures edges are removed when either endpoint is deleted.

CREATE TABLE IF NOT EXISTS note_edges (
    from_id    INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    to_id      INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL CHECK(kind IN ('supersedes', 'relates_to', 'contradicts')),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (from_id, to_id, kind)
);

CREATE INDEX IF NOT EXISTS idx_note_edges_from ON note_edges(from_id);
CREATE INDEX IF NOT EXISTS idx_note_edges_to   ON note_edges(to_id);
