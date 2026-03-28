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
        db.apply_spec_migration()?;
        db.apply_fts_migration()?;
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

    /// Create the specs and spec_links tables. Idempotent (`IF NOT EXISTS`).
    pub fn apply_spec_migration(&self) -> Result<()> {
        self.conn
            .execute_batch(include_str!("../../migrations/006_specs.sql"))
            .context("running spec migration")?;
        Ok(())
    }

    /// Create the FTS5 virtual table and sync triggers. Idempotent (`IF NOT EXISTS`).
    /// Also backfills any existing chunks not yet in the FTS index.
    pub fn apply_fts_migration(&self) -> Result<()> {
        self.conn
            .execute_batch(include_str!("../../migrations/007_fts.sql"))
            .context("running FTS migration")?;
        // Backfill existing chunks that predate the FTS table.
        self.conn
            .execute_batch(
                "INSERT INTO chunks_fts(rowid, name, content, node_type)
                 SELECT id, name, content, node_type FROM chunks
                 WHERE id NOT IN (SELECT rowid FROM chunks_fts);",
            )
            .context("backfilling FTS index")?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Files
    // -----------------------------------------------------------------------

    pub fn upsert_file(&self, path: &str, language: Option<&str>, hash: &str) -> Result<i64> {
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
        let mut stmt = self
            .conn
            .prepare_cached("SELECT hash FROM files WHERE path = ?1")?;
        let mut rows = stmt.query(rusqlite::params![path])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    // -----------------------------------------------------------------------
    // Chunks
    // -----------------------------------------------------------------------

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
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO chunks (file_id, node_type, name, start_line, end_line, content, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                file_id,
                node_type,
                name,
                start_line as i64,
                end_line as i64,
                content,
                metadata
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
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Look up the file id for a given path, or None if not indexed.
    pub fn file_id_for_path(&self, path: &str) -> Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id FROM files WHERE path = ?1")?;
        let mut rows = stmt.query(rusqlite::params![path])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// Return all chunk IDs and their content for a given file id.
    pub fn chunks_content_for_file_id(&self, file_id: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, content FROM chunks WHERE file_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map(rusqlite::params![file_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    // -----------------------------------------------------------------------
    // Embeddings (sqlite-vec)
    // -----------------------------------------------------------------------

    /// Insert or replace an embedding for a chunk.
    ///
    /// `blob` must be raw little-endian F32 bytes (see `embeddings::vec_to_blob`).
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
        // Hard cap: prevents resource exhaustion regardless of call site.
        let limit = limit.min(1_000);
        // sqlite-vec requires k to appear as a literal or bound value in the
        // WHERE clause of the vec0 scan. We inject it as a format arg since
        // it is a validated usize, then bind the query vector blob normally.
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
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// FTS5 full-text search. Returns results ranked by BM25 (best match first).
    ///
    /// BM25 in FTS5 returns negative values (more negative = better match).
    /// We negate the score so that higher `distance` values indicate better matches,
    /// consistent with the convention used in `SearchResult`.
    pub fn search_text(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::search::SearchResult>> {
        let limit = limit.min(1_000);
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.node_type, c.name,
                    CAST(c.start_line AS INTEGER), CAST(c.end_line AS INTEGER),
                    c.content, f.path, f.language,
                    bm25(chunks_fts) AS score
             FROM chunks_fts
             JOIN chunks c ON chunks_fts.rowid = c.id
             JOIN files  f ON c.file_id = f.id
             WHERE chunks_fts MATCH ?1
             ORDER BY score
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![query, limit as i64], |row| {
            let bm25_score: f64 = row.get(8)?;
            Ok(crate::search::SearchResult {
                chunk_id: row.get(0)?,
                node_type: row.get(1)?,
                name: row.get(2)?,
                start_line: row.get::<_, i64>(3)? as usize,
                end_line: row.get::<_, i64>(4)? as usize,
                content: row.get(5)?,
                file_path: row.get(6)?,
                language: row.get(7)?,
                // Negate so that more-relevant results have a lower (closer to 0) distance,
                // matching the ascending-distance convention of vector search.
                distance: (-bm25_score) as f32,
                from_graph: false,
                governing_specs: vec![],
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Hybrid search: fuses FTS5 BM25 ranking with vector KNN via Reciprocal Rank Fusion.
    ///
    /// RRF score: `Σ 1 / (k + rank_i)` where `k = 60` and `rank_i` is 1-based rank
    /// within each result list. Candidates are merged by chunk ID, scores summed,
    /// and the top `limit` returned in descending RRF score order.
    pub fn search_hybrid(
        &self,
        query: &str,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<crate::search::SearchResult>> {
        use std::collections::HashMap;

        let candidates = (limit * 3).max(20);
        let query_blob = crate::embeddings::vec_to_blob(embedding);

        let vec_results = self.search_similar(&query_blob, candidates)?;
        let text_results = self.search_text(query, candidates).unwrap_or_default();

        const K: f64 = 60.0;

        // Map chunk_id -> RRF score accumulator and the SearchResult to return.
        let mut scores: HashMap<i64, f64> = HashMap::new();
        let mut by_id: HashMap<i64, crate::search::SearchResult> = HashMap::new();

        for (rank, result) in vec_results.into_iter().enumerate() {
            let rrf = 1.0 / (K + (rank + 1) as f64);
            *scores.entry(result.chunk_id).or_insert(0.0) += rrf;
            by_id.entry(result.chunk_id).or_insert(result);
        }

        for (rank, result) in text_results.into_iter().enumerate() {
            let rrf = 1.0 / (K + (rank + 1) as f64);
            *scores.entry(result.chunk_id).or_insert(0.0) += rrf;
            by_id.entry(result.chunk_id).or_insert(result);
        }

        // Sort by descending RRF score, take top `limit`.
        let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit);

        let results = ranked
            .into_iter()
            .filter_map(|(id, rrf_score)| {
                by_id.remove(&id).map(|mut r| {
                    // Store the inverted RRF score as `distance` so that callers
                    // can still sort ascending (lower = better).
                    r.distance = (1.0 / rrf_score) as f32;
                    r
                })
            })
            .collect();

        Ok(results)
    }

    // -----------------------------------------------------------------------
    // Graph edges
    // -----------------------------------------------------------------------

    /// Insert a batch of edges for one file. Existing rows for that file are
    /// removed first (called during re-index).
    pub fn replace_edges(
        &self,
        file_path: &str,
        edges: &[crate::indexer::graph::Edge],
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM graph_edges WHERE source_file = ?1",
            rusqlite::params![file_path],
        )?;
        for e in edges {
            self.conn.execute(
                "INSERT INTO graph_edges (source_file, source_name, target_name, kind, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    e.source_file,
                    e.source_name,
                    e.target_name,
                    e.kind.to_string(),
                    e.line as i64
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
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
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
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Return chunk IDs of symbols that are called-by or call the given chunk names.
    /// Used by `spelunk ask` to enrich context with graph neighbours.
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
                 WHERE source_name IN ({placeholders}) AND kind = 'calls'
                 UNION
                 SELECT source_name FROM graph_edges
                 WHERE target_name IN ({placeholders}) AND kind = 'calls'
             )"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            names.iter().map(|n| n as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params.as_slice(), |r| r.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Return all chunks for a file path (exact match or LIKE suffix).
    /// Used by the `chunks` subcommand.
    pub fn chunks_for_file(&self, path: &str) -> Result<Vec<crate::search::SearchResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.node_type, c.name,
                    CAST(c.start_line AS INTEGER), CAST(c.end_line AS INTEGER),
                    c.content, f.path, f.language
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
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// List all indexed file paths under the given root prefix.
    pub fn file_paths_under(&self, root: &str) -> Result<Vec<(i64, String)>> {
        let prefix = format!("{root}%");
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, path FROM files WHERE path LIKE ?1")?;
        let rows = stmt.query_map(rusqlite::params![prefix], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Delete a file record and all its chunks, embeddings, and graph edges.
    pub fn delete_file(&self, file_id: i64, file_path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM embeddings WHERE chunk_id IN (SELECT id FROM chunks WHERE file_id = ?1)",
            rusqlite::params![file_id],
        )?;
        self.conn.execute(
            "DELETE FROM chunks WHERE file_id = ?1",
            rusqlite::params![file_id],
        )?;
        self.conn.execute(
            "DELETE FROM graph_edges WHERE source_file = ?1",
            rusqlite::params![file_path],
        )?;
        self.conn.execute(
            "DELETE FROM files WHERE id = ?1",
            rusqlite::params![file_id],
        )?;
        Ok(())
    }

    /// Return all indexed file paths and their stored hashes.
    /// Used by `spelunk check` to detect stale files without re-embedding.
    pub fn all_file_hashes(&self) -> Result<std::collections::HashMap<String, String>> {
        let mut stmt = self.conn.prepare_cached("SELECT path, hash FROM files")?;
        let map = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
        Ok(map)
    }

    // -----------------------------------------------------------------------
    // Staleness probe
    // -----------------------------------------------------------------------

    /// Sample up to `n` random files and compare their on-disk blake3 hashes
    /// against the stored hashes to estimate index staleness.
    ///
    /// Designed to be fast (<10 ms for n=20): only a small random sample is
    /// hashed, not the entire index.
    pub fn sample_staleness_check(&self, n: usize) -> Result<StalenessReport> {
        let last_indexed_at: Option<i64> = self
            .conn
            .query_row("SELECT MAX(indexed_at) FROM files", [], |r| r.get(0))
            .ok()
            .flatten();

        let mut stmt = self
            .conn
            .prepare("SELECT id, path, hash FROM files ORDER BY RANDOM() LIMIT ?1")?;
        let rows = stmt.query_map(rusqlite::params![n as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;

        let sampled_rows: Vec<(i64, String, String)> =
            rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let sampled = sampled_rows.len();

        let mut stale = 0usize;
        let mut stale_paths: Vec<String> = Vec::new();

        for (_id, path, stored_hash) in &sampled_rows {
            let is_stale = match std::fs::read(path) {
                Ok(bytes) => {
                    let current = format!("{}", blake3::hash(&bytes));
                    current != *stored_hash
                }
                Err(_) => true, // missing file counts as stale
            };
            if is_stale {
                stale += 1;
                stale_paths.push(path.clone());
            }
        }

        let estimated_stale_pct = if sampled == 0 {
            0.0
        } else {
            stale as f32 / sampled as f32 * 100.0
        };

        Ok(StalenessReport {
            sampled,
            stale,
            stale_paths,
            estimated_stale_pct,
            last_indexed_at,
        })
    }

    // -----------------------------------------------------------------------
    // Stats
    // -----------------------------------------------------------------------

    pub fn stats(&self) -> Result<IndexStats> {
        let file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let chunk_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        let embedding_count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))?;
        let last_indexed: Option<i64> = self
            .conn
            .query_row("SELECT MAX(indexed_at) FROM files", [], |r| r.get(0))
            .ok()
            .flatten();
        Ok(IndexStats {
            file_count,
            chunk_count,
            embedding_count,
            last_indexed,
        })
    }

    // -----------------------------------------------------------------------
    // Drift detection
    // -----------------------------------------------------------------------

    /// Files that haven't changed while the rest of the project has.
    ///
    /// `min_days_behind`: how many days behind the newest indexed file a file
    /// must be to qualify.  `caller_count` is the number of distinct files in
    /// the graph that reference any symbol defined in this file — a high count
    /// means a change here has wide blast radius.
    pub fn drift_candidates(
        &self,
        min_days_behind: i64,
        limit: usize,
    ) -> Result<Vec<DriftCandidate>> {
        let newest: i64 = self
            .conn
            .query_row("SELECT MAX(indexed_at) FROM files", [], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .ok()
            .flatten()
            .unwrap_or(0);

        if newest == 0 {
            return Ok(vec![]);
        }

        // Files lagging behind, with a caller-count: number of distinct source
        // files that reference any named symbol defined in this file.
        let mut stmt = self.conn.prepare(
            "SELECT
                 f.path,
                 (:newest - f.indexed_at) / 86400 AS days_behind,
                 (SELECT COUNT(DISTINCT e.source_file)
                  FROM graph_edges e
                  JOIN chunks c ON c.file_id = f.id AND c.name = e.target_name
                  WHERE e.source_file != f.path) AS caller_count
             FROM files f
             WHERE days_behind >= :min_days
             ORDER BY days_behind DESC
             LIMIT :lim",
        )?;

        let candidates = stmt
            .query_map(
                rusqlite::named_params! {
                    ":newest":   newest,
                    ":min_days": min_days_behind,
                    ":lim":      limit as i64,
                },
                |row| {
                    Ok(DriftCandidate {
                        path: row.get(0)?,
                        days_behind: row.get(1)?,
                        caller_count: row.get(2)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(candidates)
    }

    // -----------------------------------------------------------------------
    // Specs
    // -----------------------------------------------------------------------

    /// Register or update a spec file. Returns the spec id.
    pub fn upsert_spec(&self, path: &str, title: &str, is_auto: bool) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO specs (path, title, is_auto)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET
                title   = excluded.title,
                is_auto = excluded.is_auto",
            rusqlite::params![path, title, is_auto as i64],
        )?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM specs WHERE path = ?1",
            rusqlite::params![path],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    /// Look up a spec by its path. Returns None if not registered.
    pub fn spec_by_path(&self, path: &str) -> Result<Option<SpecRecord>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, path, title, is_auto FROM specs WHERE path = ?1")?;
        let mut rows = stmt.query(rusqlite::params![path])?;
        if let Some(row) = rows.next()? {
            Ok(Some(SpecRecord {
                id: row.get(0)?,
                path: row.get(1)?,
                title: row.get(2)?,
                is_auto: row.get::<_, i64>(3)? != 0,
            }))
        } else {
            Ok(None)
        }
    }

    /// Return all registered specs with their linked paths.
    pub fn all_specs(&self) -> Result<Vec<SpecRecord>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, path, title, is_auto FROM specs ORDER BY path")?;
        let rows = stmt.query_map([], |r| {
            Ok(SpecRecord {
                id: r.get(0)?,
                path: r.get(1)?,
                title: r.get(2)?,
                is_auto: r.get::<_, i64>(3)? != 0,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Return all linked paths for a spec.
    pub fn spec_links(&self, spec_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT linked_path FROM spec_links WHERE spec_id = ?1 ORDER BY linked_path",
        )?;
        let rows = stmt.query_map(rusqlite::params![spec_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Add a link from a spec to a path prefix (idempotent).
    pub fn add_spec_link(&self, spec_id: i64, linked_path: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO spec_links (spec_id, linked_path) VALUES (?1, ?2)",
            rusqlite::params![spec_id, linked_path],
        )?;
        Ok(())
    }

    /// Remove a specific link from a spec.
    pub fn remove_spec_link(&self, spec_id: i64, linked_path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM spec_links WHERE spec_id = ?1 AND linked_path = ?2",
            rusqlite::params![spec_id, linked_path],
        )?;
        Ok(())
    }

    /// Delete a spec and all its links.
    pub fn delete_spec(&self, spec_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM specs WHERE id = ?1",
            rusqlite::params![spec_id],
        )?;
        Ok(())
    }

    /// For a set of file paths, return the specs that govern them (via linked_path prefix match).
    /// Returns deduplicated (spec_path, title) pairs.
    pub fn specs_for_files(&self, file_paths: &[String]) -> Result<Vec<(String, String)>> {
        if file_paths.is_empty() {
            return Ok(vec![]);
        }
        // We match each file path against each spec_link prefix using LIKE.
        // SQLite doesn't support IN with a subquery on parameterised lists easily,
        // so we query all links and filter in Rust.
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.path, s.title, sl.linked_path
             FROM spec_links sl
             JOIN specs s ON s.id = sl.spec_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;

        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for row in rows {
            let (spec_path, title, linked_path) = row?;
            for fp in file_paths {
                if fp.starts_with(&linked_path) || fp == &linked_path {
                    if seen.insert(spec_path.clone()) {
                        result.push((spec_path.clone(), title.clone()));
                    }
                    break;
                }
            }
        }
        Ok(result)
    }

    /// Return specs whose linked code has been indexed more recently than the spec itself.
    /// Uses the files table indexed_at to compare timestamps.
    pub fn stale_specs(&self) -> Result<Vec<StaleSpec>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT s.id, s.path, s.title, sl.linked_path,
                    sf.indexed_at  AS spec_indexed_at,
                    lf.indexed_at  AS code_indexed_at
             FROM specs s
             JOIN spec_links sl ON sl.spec_id = s.id
             JOIN files sf      ON sf.path = s.path
             JOIN files lf      ON (lf.path = sl.linked_path
                                    OR lf.path LIKE sl.linked_path || '%')
             WHERE lf.indexed_at > sf.indexed_at
             ORDER BY s.path",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(StaleSpec {
                spec_id: r.get(0)?,
                spec_path: r.get(1)?,
                title: r.get(2)?,
                linked_path: r.get(3)?,
                spec_indexed_at: r.get(4)?,
                code_indexed_at: r.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

/// A registered spec file.
#[derive(Debug, serde::Serialize)]
pub struct SpecRecord {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub is_auto: bool,
}

/// A spec that is potentially out-of-date with its linked code.
#[derive(Debug, serde::Serialize)]
pub struct StaleSpec {
    pub spec_id: i64,
    pub spec_path: String,
    pub title: String,
    pub linked_path: String,
    pub spec_indexed_at: i64,
    pub code_indexed_at: i64,
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
        kind: row.get(3)?,
        line: row.get::<_, i64>(4)? as usize,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct IndexStats {
    pub file_count: i64,
    pub chunk_count: i64,
    pub embedding_count: i64,
    pub last_indexed: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
pub struct DriftCandidate {
    pub path: String,
    /// Days behind the most recently indexed file in the project.
    pub days_behind: i64,
    /// Number of distinct files that call/import symbols from this file.
    pub caller_count: i64,
}

/// Result of a lightweight random-sample staleness probe.
#[derive(Debug, serde::Serialize)]
pub struct StalenessReport {
    /// Number of files sampled.
    pub sampled: usize,
    /// Number of sampled files whose on-disk hash differs from the stored hash
    /// (or that are missing from disk entirely).
    pub stale: usize,
    /// Paths of the stale files in the sample.
    pub stale_paths: Vec<String>,
    /// Estimated percentage of files in the full index that are stale,
    /// extrapolated from the sample (0–100).
    pub estimated_stale_pct: f32,
    /// Unix timestamp of the most recently indexed file, or `None` if the
    /// index is empty.
    pub last_indexed_at: Option<i64>,
}
