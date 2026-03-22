-- Structural code graph: import / call / extends / implements edges.
-- Nodes are already represented by chunks; this table stores directed edges
-- between named symbols or files.

CREATE TABLE IF NOT EXISTS graph_edges (
    id          INTEGER PRIMARY KEY,
    source_file TEXT    NOT NULL,
    source_name TEXT,               -- enclosing function/class, NULL = file-level
    target_name TEXT    NOT NULL,   -- imported module or called/referenced symbol
    kind        TEXT    NOT NULL,   -- 'imports' | 'calls' | 'extends' | 'implements'
    line        INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS graph_edges_source_file ON graph_edges(source_file);
CREATE INDEX IF NOT EXISTS graph_edges_source_name ON graph_edges(source_name);
CREATE INDEX IF NOT EXISTS graph_edges_target_name ON graph_edges(target_name);
CREATE INDEX IF NOT EXISTS graph_edges_kind        ON graph_edges(kind);
