use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

pub struct MemoryStore {
    conn: Connection,
}

#[derive(Debug, Serialize)]
pub struct Note {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub linked_files: Vec<String>,
    pub created_at: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<i64>,
    /// Semantic distance — only populated by search(), None otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f64>,
    /// Fused relevance score — only populated by hybrid search, None otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

impl MemoryStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening memory DB at {}", path.display()))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        self.conn
            .execute_batch(include_str!("../../migrations/004_memory.sql"))
            .context("running memory migrations")?;
        // Migration 005: lifecycle columns — ALTER TABLE doesn't support IF NOT EXISTS,
        // so we ignore "duplicate column name" errors (idempotent re-open).
        for stmt in [
            "ALTER TABLE notes ADD COLUMN status TEXT NOT NULL DEFAULT 'active'",
            "ALTER TABLE notes ADD COLUMN superseded_by INTEGER REFERENCES notes(id)",
        ] {
            match self.conn.execute_batch(stmt) {
                Ok(_) => {}
                Err(e) if e.to_string().contains("duplicate column name") => {}
                Err(e) => return Err(e).context("running memory lifecycle migration"),
            }
        }
        // Migration 012: FTS5 full-text index for memory notes.
        self.conn
            .execute_batch(include_str!("../../migrations/012_memory_fts.sql"))
            .context("running memory FTS migration")?;
        Ok(())
    }

    /// Insert a note and return its id. Does not store an embedding —
    /// call `insert_embedding` afterwards if the embedder is available.
    pub fn add_note(
        &self,
        kind: &str,
        title: &str,
        body: &str,
        tags: &[&str],
        linked_files: &[&str],
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO notes (kind, title, body, tags, linked_files)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![kind, title, body, tags.join(","), linked_files.join(","),],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_embedding(&self, note_id: i64, blob: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO note_embeddings (note_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![note_id, blob],
        )?;
        Ok(())
    }

    /// Semantic KNN search. Returns active notes ordered by ascending distance.
    pub fn search(&self, query_blob: &[u8], limit: usize) -> Result<Vec<Note>> {
        let limit = limit.min(100);
        let sql = format!(
            "WITH knn AS (
                 SELECT note_id, distance
                 FROM   note_embeddings
                 WHERE  embedding MATCH ?1
                   AND  k = {limit}
             )
             SELECT n.id, n.kind, n.title, n.body, n.tags, n.linked_files,
                    n.created_at, n.status, n.superseded_by, CAST(k.distance AS REAL)
             FROM   knn k
             JOIN   notes n ON n.id = k.note_id
             WHERE  n.status = 'active'
             ORDER  BY k.distance"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let notes = stmt
            .query_map(rusqlite::params![query_blob], row_to_note_with_distance)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(notes)
    }

    /// BM25 full-text search over notes (title, body, tags).
    /// Returns active notes ordered by descending relevance.
    pub fn search_text(&self, query: &str, limit: usize) -> Result<Vec<Note>> {
        let limit = limit.min(1_000);
        let sql = format!(
            "SELECT n.id, n.kind, n.title, n.body, n.tags, n.linked_files,
                    n.created_at, n.status, n.superseded_by,
                    bm25(memory_fts) AS bm25_score
             FROM memory_fts
             JOIN notes n ON memory_fts.rowid = n.id
             WHERE memory_fts MATCH ?1
               AND n.status = 'active'
             ORDER BY bm25_score
             LIMIT {limit}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let notes = stmt
            .query_map(rusqlite::params![query], |row| {
                let bm25_score: f64 = row.get(9)?;
                let mut note = row_to_note(row)?;
                // Negate so that higher relevance → lower distance (ascending convention).
                note.distance = Some(-bm25_score);
                Ok(note)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(notes)
    }

    /// Hybrid search: fuses FTS5 BM25 ranking with vector KNN via Reciprocal Rank Fusion.
    ///
    /// RRF score: `Σ 1 / (k + rank_i)` where `k = 60` (standard default).
    /// Candidates from both lists are merged by note ID, scores summed, then the top
    /// `limit` are returned in descending RRF score order.
    pub fn search_hybrid(&self, query_blob: &[u8], query: &str, limit: usize) -> Result<Vec<Note>> {
        use std::collections::HashMap;

        let candidates = (limit * 3).max(20);

        let vec_results = self.search(query_blob, candidates)?;
        let text_results = self.search_text(query, candidates).unwrap_or_default();

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

    /// List notes, optionally filtered by kind, newest first.
    /// When `include_archived` is false only active entries are returned.
    pub fn list(
        &self,
        kind_filter: Option<&str>,
        limit: usize,
        include_archived: bool,
    ) -> Result<Vec<Note>> {
        let limit = limit.min(500);
        let status_clause = if include_archived {
            ""
        } else {
            "AND status = 'active'"
        };
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(kind) =
            kind_filter
        {
            (
                    format!(
                        "SELECT id, kind, title, body, tags, linked_files, created_at, status, superseded_by
                         FROM notes WHERE kind = ?1 {status_clause} ORDER BY created_at DESC LIMIT {limit}"
                    ),
                    vec![Box::new(kind.to_string())],
                )
        } else {
            (
                    format!(
                        "SELECT id, kind, title, body, tags, linked_files, created_at, status, superseded_by
                         FROM notes WHERE 1=1 {status_clause} ORDER BY created_at DESC LIMIT {limit}"
                    ),
                    vec![],
                )
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let notes = stmt
            .query_map(params_refs.as_slice(), row_to_note)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(notes)
    }

    /// Mark an entry as archived (hidden from search and ask context).
    pub fn archive(&self, id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE notes SET status = 'archived' WHERE id = ?1 AND status = 'active'",
            rusqlite::params![id],
        )?;
        Ok(changed > 0)
    }

    /// Archive `old_id` and link it to `new_id` as its replacement.
    pub fn supersede(&self, old_id: i64, new_id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE notes SET status = 'archived', superseded_by = ?2 WHERE id = ?1 AND status = 'active'",
            rusqlite::params![old_id, new_id],
        )?;
        Ok(changed > 0)
    }

    /// Retrieve the raw embedding blob for a note (for use by `memory push`).
    pub fn get_embedding(&self, note_id: i64) -> Result<Option<Vec<u8>>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT embedding FROM note_embeddings WHERE note_id = ?1",
                rusqlite::params![note_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(blob)
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM notes WHERE status = 'active'",
            [],
            |r| r.get(0),
        )?)
    }

    /// Return all SHA tags stored (used by harvest to avoid duplicates).
    /// Harvest stores the git SHA as a tag in the format "git:<sha>".
    pub fn harvested_shas(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT tags FROM notes WHERE tags LIKE '%git:%'")?;
        let rows = stmt.query_map([], |r| r.get::<_, Option<String>>(0))?;
        let mut shas = std::collections::HashSet::new();
        for row in rows {
            if let Some(tags) = row? {
                for tag in tags.split(',').map(str::trim) {
                    if let Some(sha) = tag.strip_prefix("git:") {
                        shas.insert(sha.to_string());
                    }
                }
            }
        }
        Ok(shas)
    }

    pub fn get(&self, id: i64) -> Result<Option<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, body, tags, linked_files, created_at, status, superseded_by
             FROM notes WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id], row_to_note)?;
        Ok(rows.next().transpose()?)
    }
}

// ── row mappers ──────────────────────────────────────────────────────────────

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        kind: row.get(1)?,
        title: row.get(2)?,
        body: row.get(3)?,
        tags: split_csv(row.get::<_, Option<String>>(4)?.as_deref()),
        linked_files: split_csv(row.get::<_, Option<String>>(5)?.as_deref()),
        created_at: row.get(6)?,
        status: row.get(7)?,
        superseded_by: row.get(8)?,
        distance: None,
        score: None,
    })
}

fn row_to_note_with_distance(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        kind: row.get(1)?,
        title: row.get(2)?,
        body: row.get(3)?,
        tags: split_csv(row.get::<_, Option<String>>(4)?.as_deref()),
        linked_files: split_csv(row.get::<_, Option<String>>(5)?.as_deref()),
        created_at: row.get(6)?,
        status: row.get(7)?,
        superseded_by: row.get(8)?,
        distance: Some(row.get(9)?),
        score: None,
    })
}

fn split_csv(s: Option<&str>) -> Vec<String> {
    match s {
        None | Some("") => vec![],
        Some(s) => s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    }
}
