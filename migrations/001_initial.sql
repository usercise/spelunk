-- Source files tracked in the index
CREATE TABLE IF NOT EXISTS files (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    path       TEXT    UNIQUE NOT NULL,
    language   TEXT,
    hash       TEXT    NOT NULL,  -- blake3 hex; used for incremental re-indexing
    indexed_at INTEGER NOT NULL   -- unix timestamp
);

-- AST-derived code chunks extracted from each file
CREATE TABLE IF NOT EXISTS chunks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id    INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    node_type  TEXT    NOT NULL,  -- "function", "struct", "class", "method", etc.
    name       TEXT,              -- symbol name (NULL for anonymous/fallback chunks)
    start_line INTEGER NOT NULL,
    end_line   INTEGER NOT NULL,
    content    TEXT    NOT NULL,
    metadata   TEXT              -- JSON: docstring, parent_scope, etc.
);

-- Vector index — one embedding row per chunk
-- Requires sqlite-vec to be loaded before this statement executes.
CREATE VIRTUAL TABLE IF NOT EXISTS embeddings USING vec0(
    chunk_id INTEGER PRIMARY KEY,
    embedding FLOAT[768]
);

CREATE INDEX IF NOT EXISTS idx_chunks_file_id ON chunks(file_id);
CREATE INDEX IF NOT EXISTS idx_files_path     ON files(path);
