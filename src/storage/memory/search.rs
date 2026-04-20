use anyhow::Result;

use super::notes::{row_to_note, row_to_note_with_distance};
use super::{MemoryStore, Note};

impl MemoryStore {
    /// Semantic KNN search. Returns active notes ordered by ascending distance.
    /// When `as_of` is `Some(ts)`, only entries valid at that Unix timestamp are returned.
    pub fn search(&self, query_blob: &[u8], limit: usize, as_of: Option<i64>) -> Result<Vec<Note>> {
        let limit = limit.min(100);
        let as_of_clause = if as_of.is_some() {
            "AND (n.valid_at IS NULL OR n.valid_at <= ?2) AND (n.invalid_at IS NULL OR n.invalid_at > ?2)"
        } else {
            ""
        };
        let sql = format!(
            "WITH knn AS (
                 SELECT note_id, distance
                 FROM   note_embeddings
                 WHERE  embedding MATCH ?1
                   AND  k = {limit}
             )
             SELECT n.id, n.kind, n.title, n.body, n.tags, n.linked_files,
                    n.created_at, n.status, n.superseded_by, n.source_ref,
                    n.valid_at, n.invalid_at, CAST(k.distance AS REAL)
             FROM   knn k
             JOIN   notes n ON n.id = k.note_id
             WHERE  n.status = 'active'
             {as_of_clause}
             ORDER  BY k.distance"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let notes = if let Some(ts) = as_of {
            stmt.query_map(rusqlite::params![query_blob, ts], row_to_note_with_distance)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(rusqlite::params![query_blob], row_to_note_with_distance)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(notes)
    }

    /// BM25 full-text search over notes (title, body, tags).
    /// Returns active notes ordered by descending relevance.
    /// When `as_of` is `Some(ts)`, only entries valid at that Unix timestamp are returned.
    pub fn search_text(&self, query: &str, limit: usize, as_of: Option<i64>) -> Result<Vec<Note>> {
        let limit = limit.min(1_000);
        let as_of_clause = if as_of.is_some() {
            "AND (n.valid_at IS NULL OR n.valid_at <= ?2) AND (n.invalid_at IS NULL OR n.invalid_at > ?2)"
        } else {
            ""
        };
        let sql = format!(
            "SELECT n.id, n.kind, n.title, n.body, n.tags, n.linked_files,
                    n.created_at, n.status, n.superseded_by, n.source_ref,
                    n.valid_at, n.invalid_at, bm25(memory_fts) AS bm25_score
             FROM memory_fts
             JOIN notes n ON memory_fts.rowid = n.id
             WHERE memory_fts MATCH ?1
               AND n.status = 'active'
             {as_of_clause}
             ORDER BY bm25_score
             LIMIT {limit}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let notes = if let Some(ts) = as_of {
            stmt.query_map(rusqlite::params![query, ts], |row| {
                let bm25_score: f64 = row.get(12)?;
                let mut note = row_to_note(row)?;
                note.distance = Some(-bm25_score);
                Ok(note)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(rusqlite::params![query], |row| {
                let bm25_score: f64 = row.get(12)?;
                let mut note = row_to_note(row)?;
                // Negate so that higher relevance → lower distance (ascending convention).
                note.distance = Some(-bm25_score);
                Ok(note)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(notes)
    }

    /// Hybrid search: fuses FTS5 BM25 ranking with vector KNN via Reciprocal Rank Fusion.
    ///
    /// RRF score: `Σ 1 / (k + rank_i)` where `k = 60` (standard default).
    /// Candidates from both lists are merged by note ID, scores summed, then the top
    /// `limit` are returned in descending RRF score order.
    /// When `as_of` is `Some(ts)`, only entries valid at that timestamp are considered.
    pub fn search_hybrid(
        &self,
        query_blob: &[u8],
        query: &str,
        limit: usize,
        as_of: Option<i64>,
    ) -> Result<Vec<Note>> {
        use std::collections::HashMap;

        let candidates = (limit * 3).max(20);

        let vec_results = self.search(query_blob, candidates, as_of)?;
        let text_results = self
            .search_text(query, candidates, as_of)
            .unwrap_or_default();

        const K: f64 = 60.0;

        let mut scores: HashMap<i64, f64> = HashMap::new();
        let mut by_id: HashMap<i64, Note> = HashMap::new();

        for (rank, note) in vec_results.into_iter().enumerate() {
            let rrf = 1.0 / (K + (rank + 1) as f64);
            *scores.entry(note.id).or_insert(0.0) += rrf;
            by_id.entry(note.id).or_insert(note);
        }

        for (rank, note) in text_results.into_iter().enumerate() {
            let rrf = 1.0 / (K + (rank + 1) as f64);
            *scores.entry(note.id).or_insert(0.0) += rrf;
            by_id.entry(note.id).or_insert(note);
        }

        // Sort descending by RRF score, take top `limit`.
        let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit);

        let results = ranked
            .into_iter()
            .filter_map(|(id, rrf_score)| {
                by_id.remove(&id).map(|mut n| {
                    n.score = Some(rrf_score);
                    // Keep distance as inverted RRF so callers can sort ascending.
                    n.distance = Some(1.0 / rrf_score);
                    n
                })
            })
            .collect();

        Ok(results)
    }

    /// Semantic search over ALL notes regardless of status (for timeline view).
    /// Returns notes ordered by `COALESCE(valid_at, created_at) ASC`.
    pub fn search_timeline(&self, query_blob: &[u8], limit: usize) -> Result<Vec<Note>> {
        let limit = limit.min(200);
        let sql = format!(
            "WITH knn AS (
                 SELECT note_id, distance
                 FROM   note_embeddings
                 WHERE  embedding MATCH ?1
                   AND  k = {limit}
             )
             SELECT n.id, n.kind, n.title, n.body, n.tags, n.linked_files,
                    n.created_at, n.status, n.superseded_by, n.source_ref,
                    n.valid_at, n.invalid_at, CAST(k.distance AS REAL)
             FROM   knn k
             JOIN   notes n ON n.id = k.note_id
             ORDER  BY COALESCE(n.valid_at, n.created_at) ASC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let notes = stmt
            .query_map(rusqlite::params![query_blob], row_to_note_with_distance)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(notes)
    }
}
