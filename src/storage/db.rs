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
        db.apply_graph_migration()?;
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

    /// Create the graph_edges table. Idempotent (`IF NOT EXISTS`).
    pub fn apply_graph_migration(&self) -> Result<()> {
        self.conn
            .execute_batch(include_str!("../../migrations/003_graph.sql"))
            .context("running graph migration")?;
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

    /// Fetch full `SearchResult` rows for a list of chunk IDs (used for graph
    /// neighbour enrichment in `ask`).
    pub fn chunks_by_ids(&self, ids: &[i64]) -> Result<Vec<crate::search::SearchResult>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT c.id, 0.0, c.node_type, c.name,
                    CAST(c.start_line AS INTEGER), CAST(c.end_line AS INTEGER),
                    c.content, f.path, f.language
             FROM chunks c
             JOIN files f ON f.id = c.file_id
             WHERE c.id IN ({placeholders})"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), |row| {
            Ok(crate::search::SearchResult {
                chunk_id:   row.get(0)?,
                distance:   row.get(1)?,
                node_type:  row.get(2)?,
                name:       row.get(3)?,
                start_line: row.get::<_, i64>(4)? as usize,
                end_line:   row.get::<_, i64>(5)? as usize,
                content:    row.get(6)?,
                file_path:  row.get(7)?,
                language:   row.get(8)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    /// Return all chunk IDs and their embedding text for a given file.
    #[allow(dead_code)]
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
    // Vector search
    // -----------------------------------------------------------------------

    /// K-nearest-neighbour search using sqlite-vec.
    ///
    /// `query_blob` must be raw little-endian F32 bytes produced by
    /// `vec_to_blob`. Returns results ordered by ascending distance
    /// (closest first).
    pub fn search_similar(
        &self,
        query_blob: &[u8],
        limit: usize,
    ) -> Result<Vec<crate::search::SearchResult>> {
        // sqlite-vec requires k to appear as a literal or bound value in the
        // WHERE clause of the vec0 scan. We inject it as a format arg since
        // it is a trusted usize, then bind the query vector blob normally.
        let sql = format!(
            "WITH knn AS (
                 SELECT chunk_id, distance
                 FROM   embeddings
                 WHERE  embedding MATCH ?1
                   AND  k = {limit}
             )
             SELECT  k.chunk_id,
                     CAST(k.distance AS REAL),
                     c.node_type,
                     c.name,
                     CAST(c.start_line AS INTEGER),
                     CAST(c.end_line   AS INTEGER),
                     c.content,
                     f.path,
                     f.language
             FROM knn k
             JOIN chunks c ON c.id = k.chunk_id
             JOIN files  f ON f.id = c.file_id
             ORDER BY k.distance"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![query_blob], |row| {
            Ok(crate::search::SearchResult {
                chunk_id:   row.get(0)?,
                distance:   row.get(1)?,
                node_type:  row.get(2)?,
                name:       row.get(3)?,
                start_line: row.get::<_, i64>(4)? as usize,
                end_line:   row.get::<_, i64>(5)? as usize,
                content:    row.get(6)?,
                file_path:  row.get(7)?,
                language:   row.get(8)?,
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Graph edges
    // -----------------------------------------------------------------------

    /// Insert a batch of edges for one file. Existing rows for that file are
    /// removed first (called during re-index).
    pub fn replace_edges(&self, file_path: &str, edges: &[crate::indexer::graph::Edge]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM graph_edges WHERE source_file = ?1",
            rusqlite::params![file_path],
        )?;
        for e in edges {
            self.conn.execute(
                "INSERT INTO graph_edges (source_file, source_name, target_name, kind, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    e.source_file, e.source_name, e.target_name,
                    e.kind.to_string(), e.line as i64
                ],
            )?;
        }
        Ok(())
    }

    /// All edges where `name` appears as source_name OR target_name.
    pub fn edges_for_symbol(&self, name: &str) -> Result<Vec<GraphEdge>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_file, source_name, target_name, kind, line
             FROM graph_edges
             WHERE source_name = ?1 OR target_name = ?1
             ORDER BY kind, target_name",
        )?;
        let rows = stmt.query_map(rusqlite::params![name], row_to_edge)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    /// All edges originating from `file_path`.
    pub fn edges_for_file(&self, file_path: &str) -> Result<Vec<GraphEdge>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_file, source_name, target_name, kind, line
             FROM graph_edges
             WHERE source_file = ?1
             ORDER BY kind, target_name",
        )?;
        let rows = stmt.query_map(rusqlite::params![file_path], row_to_edge)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    /// Return chunk IDs of symbols that are called-by or call the given chunk names.
    /// Used by `ca ask` to enrich context with graph neighbours.
    pub fn graph_neighbor_chunks(&self, names: &[&str]) -> Result<Vec<i64>> {
        if names.is_empty() {
            return Ok(vec![]);
        }
        // Build a parameterised IN clause at runtime (trusted internal data).
        let placeholders = names
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT DISTINCT c.id
             FROM chunks c
             WHERE c.name IN (
                 SELECT target_name FROM graph_edges
                 WHERE source_name IN ({ph}) AND kind = 'calls'
                 UNION
                 SELECT source_name FROM graph_edges
                 WHERE target_name IN ({ph}) AND kind = 'calls'
             )",
            ph = placeholders
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = names
            .iter()
            .map(|n| n as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
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

/// A graph edge as returned by query methods.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphEdge {
    pub source_file: String,
    pub source_name: Option<String>,
    pub target_name: String,
    pub kind: String,
    pub line: usize,
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<GraphEdge> {
    Ok(GraphEdge {
        source_file: row.get(0)?,
        source_name: row.get(1)?,
        target_name: row.get(2)?,
        kind:        row.get(3)?,
        line:        row.get::<_, i64>(4)? as usize,
    })
}

#[derive(Debug)]
pub struct IndexStats {
    pub file_count: i64,
    pub chunk_count: i64,
    pub embedding_count: i64,
}
