use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

/// Wraps the SQLite connection and provides typed access to the schema.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) the database at `path` and run all migrations.
    ///
    /// Assumes `sqlite3_auto_extension` has already been called in `main` to
    /// load the sqlite-vec extension into every new connection.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating db directory {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("opening database at {}", path.display()))?;

        // WAL mode: better concurrent read performance
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        let db = Self { conn };
        db.migrate()?;
        db.apply_vector_migration()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn
            .execute_batch(include_str!("../../migrations/001_initial.sql"))
            .context("running base migrations")?;
        Ok(())
    }

    /// Create the sqlite-vec virtual table. Idempotent (`IF NOT EXISTS`).
    pub fn apply_vector_migration(&self) -> Result<()> {
        self.conn
            .execute_batch(include_str!("../../migrations/002_vectors.sql"))
            .context("running vector migration (is the sqlite-vec extension loaded?)")?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Files
    // -----------------------------------------------------------------------

    pub fn upsert_file(
        &self,
        path: &str,
        language: Option<&str>,
        hash: &str,
    ) -> Result<i64> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn.execute(
            "INSERT INTO files (path, language, hash, indexed_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET
                language   = excluded.language,
                hash       = excluded.hash,
                indexed_at = excluded.indexed_at",
            rusqlite::params![path, language, hash, now],
        )?;

        // ON CONFLICT UPDATE doesn't reset last_insert_rowid; fetch it explicitly.
        let id: i64 = self.conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            rusqlite::params![path],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    /// Returns the stored hash for a file path, or None if not indexed.
    pub fn file_hash(&self, path: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT hash FROM files WHERE path = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![path])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    // -----------------------------------------------------------------------
    // Chunks
    // -----------------------------------------------------------------------

    pub fn insert_chunk(
        &self,
        file_id: i64,
        node_type: &str,
        name: Option<&str>,
        start_line: usize,
        end_line: usize,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO chunks (file_id, node_type, name, start_line, end_line, content, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                file_id, node_type, name,
                start_line as i64, end_line as i64,
                content, metadata
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_chunks_for_file(&self, file_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM chunks WHERE file_id = ?1",
            rusqlite::params![file_id],
        )?;
        // Cascade in SQL handles embeddings deletion via chunk_id FK
        Ok(())
    }

    /// Return all chunk IDs and their embedding text for a given file.
    pub fn chunks_for_embedding(
        &self,
        file_id: i64,
    ) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, content FROM chunks WHERE file_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![file_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Embeddings (sqlite-vec)
    // -----------------------------------------------------------------------

    /// Insert or replace an embedding for a chunk.
    ///
    /// `blob` must be raw little-endian F32 bytes (`vec_to_blob` in candle.rs).
    pub fn insert_embedding(&self, chunk_id: i64, blob: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO embeddings (chunk_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![chunk_id, blob],
        )?;
        Ok(())
    }

    /// Delete all embeddings associated with chunks of a given file.
    pub fn delete_embeddings_for_file(&self, file_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM embeddings WHERE chunk_id IN (
                 SELECT id FROM chunks WHERE file_id = ?1
             )",
            rusqlite::params![file_id],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Stats
    // -----------------------------------------------------------------------

    pub fn stats(&self) -> Result<IndexStats> {
        let file_count: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let chunk_count: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        let embedding_count: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))?;
        Ok(IndexStats { file_count, chunk_count, embedding_count })
    }
}

#[derive(Debug)]
pub struct IndexStats {
    pub file_count: i64,
    pub chunk_count: i64,
    pub embedding_count: i64,
}
