-- Vector index for snapshot embeddings.
-- Applied after sqlite-vec extension is loaded (same pattern as 002_vectors.sql).
CREATE VIRTUAL TABLE IF NOT EXISTS snapshot_embeddings USING vec0(
    chunk_id  INTEGER PRIMARY KEY,
    embedding FLOAT[768]
);
