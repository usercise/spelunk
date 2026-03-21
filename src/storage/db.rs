use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

/// Wraps the SQLite connection and provides typed access to the schema.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) the database at `path` and run migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating db directory {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("opening database at {}", path.display()))?;

        // Load the sqlite-vec extension
        // Phase 4: unsafe { sqlite_vec::load(&conn)?; }

        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Apply base schema migrations (files + chunks tables).
    fn migrate(&self) -> Result<()> {
        self.conn
            .execute_batch(include_str!("../../migrations/001_initial.sql"))
            .context("running migrations")?;
        Ok(())
    }

    /// Apply the vector table migration (Phase 4).
    /// Must be called *after* the sqlite-vec extension is loaded.
    pub fn apply_vector_migration(&self) -> Result<()> {
        self.conn
            .execute_batch(include_str!("../../migrations/002_vectors.sql"))
            .context("running vector migration")?;
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

        Ok(self.conn.last_insert_rowid())
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
        Ok(IndexStats { file_count, chunk_count })
    }
}

#[derive(Debug)]
pub struct IndexStats {
    pub file_count: i64,
    pub chunk_count: i64,
}
