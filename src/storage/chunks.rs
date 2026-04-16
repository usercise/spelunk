use anyhow::Result;

use super::Database;

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub fn insert_chunk(
        &self,
        file_id: i64,
        node_type: &str,
        name: Option<&str>,
        start_line: usize,
        end_line: usize,
        content: &str,
        metadata: Option<&str>,
        token_count: usize,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO chunks (file_id, node_type, name, start_line, end_line, content, metadata, token_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                file_id,
                node_type,
                name,
                start_line as i64,
                end_line as i64,
                content,
                metadata,
                token_count as i64,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Backfill token_count for all chunks where it is still 0 (existing indexes).
    /// Returns the number of rows updated.
    pub fn backfill_token_counts(&self) -> Result<usize> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, content FROM chunks WHERE token_count = 0")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let pairs: Vec<(i64, String)> = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        let count = pairs.len();
        for (id, content) in &pairs {
            let tc = crate::search::tokens::estimate_tokens(content) as i64;
            self.conn.execute(
                "UPDATE chunks SET token_count = ?1 WHERE id = ?2",
                rusqlite::params![tc, id],
            )?;
        }
        Ok(count)
    }

    pub fn delete_chunks_for_file(&self, file_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM chunks WHERE file_id = ?1",
            rusqlite::params![file_id],
        )?;
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
                    c.content, f.path, f.language, c.token_count
             FROM chunks c
             JOIN files f ON f.id = c.file_id
             WHERE c.id IN ({placeholders})"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params.as_slice(), |row| {
            Ok(crate::search::SearchResult {
                chunk_id: row.get(0)?,
                distance: row.get(1)?,
                node_type: row.get(2)?,
                name: row.get(3)?,
                start_line: row.get::<_, i64>(4)? as usize,
                end_line: row.get::<_, i64>(5)? as usize,
                content: row.get(6)?,
                file_path: row.get(7)?,
                language: row.get(8)?,
                from_graph: false,
                governing_specs: vec![],
                token_count: row.get::<_, i64>(9)? as usize,
                project_name: None,
                project_path: None,
                summary: None,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Return all chunks for a file path (exact match or LIKE suffix).
    /// Used by the `chunks` subcommand and `cat-chunks` plumbing command.
    pub fn chunks_for_file(&self, path: &str) -> Result<Vec<crate::search::SearchResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.node_type, c.name,
                    CAST(c.start_line AS INTEGER), CAST(c.end_line AS INTEGER),
                    c.content, f.path, f.language, c.token_count
             FROM chunks c
             JOIN files f ON f.id = c.file_id
             WHERE f.path = ?1 OR f.path LIKE '%' || ?1
             ORDER BY c.start_line",
        )?;
        let rows = stmt.query_map(rusqlite::params![path], |row| {
            Ok(crate::search::SearchResult {
                chunk_id: row.get(0)?,
                distance: 0.0,
                node_type: row.get(1)?,
                name: row.get(2)?,
                start_line: row.get::<_, i64>(3)? as usize,
                end_line: row.get::<_, i64>(4)? as usize,
                content: row.get(5)?,
                file_path: row.get(6)?,
                language: row.get(7)?,
                from_graph: false,
                governing_specs: vec![],
                token_count: row.get::<_, i64>(8)? as usize,
                project_name: None,
                project_path: None,
                summary: None,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Fetch chunks that have no summary yet, up to `limit` rows.
    /// Returns `(id, name, kind, content)`.
    pub fn chunks_without_summaries(
        &self,
        limit: usize,
    ) -> Result<Vec<(i64, String, String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, COALESCE(name, ''), node_type, content
             FROM chunks
             WHERE summary IS NULL
             ORDER BY id
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Update the LLM-generated summary for a single chunk.
    pub fn update_chunk_summary(&self, chunk_id: i64, summary: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE chunks SET summary = ?1 WHERE id = ?2",
            rusqlite::params![summary, chunk_id],
        )?;
        Ok(())
    }

    /// Return all (chunk_id, name) pairs for chunks that have a name.
    /// Used to map PageRank scores back to chunk IDs.
    pub fn chunks_with_names(&self) -> Result<Vec<(i64, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, name FROM chunks WHERE name IS NOT NULL")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Update the graph_rank score for a single chunk.
    pub fn update_graph_rank(&self, chunk_id: i64, score: f32) -> Result<()> {
        self.conn.execute(
            "UPDATE chunks SET graph_rank = ?1 WHERE id = ?2",
            rusqlite::params![score, chunk_id],
        )?;
        Ok(())
    }

    /// Batch-update graph_rank scores inside a transaction for performance.
    pub fn update_graph_ranks(&self, scores: &[(i64, f32)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (chunk_id, score) in scores {
            tx.execute(
                "UPDATE chunks SET graph_rank = ?1 WHERE id = ?2",
                rusqlite::params![score, chunk_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
