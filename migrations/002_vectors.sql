-- Phase 4: vector index for embeddings.
-- This migration is applied by Database::apply_vector_migration(), called only
-- after the sqlite-vec extension has been loaded into the connection.
CREATE VIRTUAL TABLE IF NOT EXISTS embeddings USING vec0(
    chunk_id INTEGER PRIMARY KEY,
    embedding FLOAT[768]
);
