use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

pub struct MemoryStore {
    conn: Connection,
}

#[derive(Debug, Serialize)]
pub struct MemoryEdge {
    pub from_id: i64,
    pub to_id: i64,
    pub kind: String,
    pub created_at: i64,
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
    /// Git commit SHA for harvested entries; NULL for manually created entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    /// When this entry became valid (unix epoch). None = treat as created_at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<i64>,
    /// When this entry was invalidated/superseded (unix epoch). None = still valid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_at: Option<i64>,
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
            // Migration 012: commit provenance
            "ALTER TABLE notes ADD COLUMN source_ref TEXT",
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
        // Migration 014: temporal fields — valid_at and invalid_at.
        for stmt in [
            "ALTER TABLE notes ADD COLUMN valid_at INTEGER",
            "ALTER TABLE notes ADD COLUMN invalid_at INTEGER",
            "CREATE INDEX IF NOT EXISTS idx_memory_invalid_at ON notes(invalid_at)",
        ] {
            match self.conn.execute_batch(stmt) {
                Ok(_) => {}
                Err(e) if e.to_string().contains("duplicate column name") => {}
                Err(e) => return Err(e).context("running memory temporal migration"),
            }
        }
        // Migration 015: memory edge table.
        self.conn
            .execute_batch(include_str!("../../migrations/015_memory_edges.sql"))
            .context("running memory edges migration")?;
        Ok(())
    }

    /// Insert a note and return its id. Does not store an embedding —
    /// call `insert_embedding` afterwards if the embedder is available.
    #[allow(clippy::too_many_arguments)]
    pub fn add_note(
        &self,
        kind: &str,
        title: &str,
        body: &str,
        tags: &[&str],
        linked_files: &[&str],
        source_ref: Option<&str>,
        valid_at: Option<i64>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO notes (kind, title, body, tags, linked_files, source_ref, valid_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                kind,
                title,
                body,
                tags.join(","),
                linked_files.join(","),
                source_ref,
                valid_at,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a note in a transaction that also sets `invalid_at` on the superseded entry.
    /// Returns the new note's id.
    #[allow(clippy::too_many_arguments)]
    pub fn add_note_superseding(
        &self,
        kind: &str,
        title: &str,
        body: &str,
        tags: &[&str],
        linked_files: &[&str],
        valid_at: Option<i64>,
        supersedes_id: i64,
    ) -> Result<i64> {
        self.conn.execute_batch("BEGIN")?;
        let result = (|| -> Result<i64> {
            self.conn.execute(
                "INSERT INTO notes (kind, title, body, tags, linked_files, valid_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    kind,
                    title,
                    body,
                    tags.join(","),
                    linked_files.join(","),
                    valid_at,
                ],
            )?;
            let new_id = self.conn.last_insert_rowid();
            self.conn.execute(
                "UPDATE notes
                 SET    status = 'archived',
                        superseded_by = ?2,
                        invalid_at = CASE WHEN invalid_at IS NULL THEN unixepoch() ELSE invalid_at END
                 WHERE  id = ?1 AND status = 'active'",
                rusqlite::params![supersedes_id, new_id],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO memory_edges (from_id, to_id, kind) VALUES (?1, ?2, 'supersedes')",
                rusqlite::params![new_id, supersedes_id],
            )?;
            Ok(new_id)
        })();
        match result {
            Ok(id) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(id)
            }
            Err(e) => {
                self.conn.execute_batch("ROLLBACK").ok();
                Err(e)
            }
        }
    }

    pub fn insert_embedding(&self, note_id: i64, blob: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO note_embeddings (note_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![note_id, blob],
        )?;
        Ok(())
    }

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

    /// List notes, optionally filtered by kind, newest first.
    /// When `include_archived` is false only active entries are returned.
    pub fn list(
        &self,
        kind_filter: Option<&str>,
        limit: usize,
        include_archived: bool,
    ) -> Result<Vec<Note>> {
        self.list_filtered(kind_filter, None, limit, include_archived, None)
    }

    /// List notes with optional kind, source_ref (prefix), and as_of filters.
    /// When `as_of` is `Some(ts)`, only entries valid at that Unix timestamp are returned.
    pub fn list_filtered(
        &self,
        kind_filter: Option<&str>,
        source_ref_prefix: Option<&str>,
        limit: usize,
        include_archived: bool,
        as_of: Option<i64>,
    ) -> Result<Vec<Note>> {
        let limit = limit.min(500);
        let status_clause = if include_archived {
            ""
        } else {
            "AND status = 'active'"
        };

        // Safety: only string literals and bind-param placeholders are appended to
        // `conditions`; all user-supplied values (kind, source_ref, as_of) are bound
        // via rusqlite params![...], never interpolated into the query string.
        let mut conditions = format!("WHERE 1=1 {status_clause}");
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];

        if let Some(kind) = kind_filter {
            conditions.push_str(&format!(" AND kind = ?{}", params.len() + 1));
            params.push(Box::new(kind.to_string()));
        }
        if let Some(prefix) = source_ref_prefix {
            conditions.push_str(&format!(" AND source_ref LIKE ?{}", params.len() + 1));
            params.push(Box::new(format!("{prefix}%")));
        }
        if let Some(ts) = as_of {
            conditions.push_str(&format!(
                " AND (valid_at IS NULL OR valid_at <= ?{p}) AND (invalid_at IS NULL OR invalid_at > ?{p})",
                p = params.len() + 1
            ));
            params.push(Box::new(ts));
        }

        let sql = format!(
            "SELECT id, kind, title, body, tags, linked_files, created_at, status, superseded_by, source_ref, valid_at, invalid_at
             FROM notes {conditions} ORDER BY created_at DESC LIMIT {limit}"
        );

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
    /// Sets `invalid_at` to now if not already set.
    pub fn supersede(&self, old_id: i64, new_id: i64) -> Result<bool> {
        self.conn.execute_batch("BEGIN")?;
        let result = (|| -> Result<bool> {
            let changed = self.conn.execute(
                "UPDATE notes
                 SET    status = 'archived',
                        superseded_by = ?2,
                        invalid_at = CASE WHEN invalid_at IS NULL THEN unixepoch() ELSE invalid_at END
                 WHERE  id = ?1 AND status = 'active'",
                rusqlite::params![old_id, new_id],
            )?;
            if changed > 0 {
                self.conn.execute(
                    "INSERT OR IGNORE INTO memory_edges (from_id, to_id, kind) VALUES (?1, ?2, 'supersedes')",
                    rusqlite::params![new_id, old_id],
                )?;
            }
            Ok(changed > 0)
        })();
        match result {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Insert a directed edge between two notes.
    /// `kind` must be one of: supersedes, relates_to, contradicts.
    pub fn add_edge(&self, from_id: i64, to_id: i64, kind: &str) -> Result<()> {
        const VALID_KINDS: &[&str] = &["supersedes", "relates_to", "contradicts"];
        if !VALID_KINDS.contains(&kind) {
            anyhow::bail!(
                "invalid edge kind '{kind}'; must be one of: supersedes, relates_to, contradicts"
            );
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO memory_edges (from_id, to_id, kind) VALUES (?1, ?2, ?3)",
            rusqlite::params![from_id, to_id, kind],
        )?;
        Ok(())
    }

    /// Return all outgoing and incoming edges for a note.
    /// Returns `(outgoing, incoming)`.
    pub fn get_edges(&self, id: i64) -> Result<(Vec<MemoryEdge>, Vec<MemoryEdge>)> {
        let mut stmt = self.conn.prepare(
            "SELECT from_id, to_id, kind, created_at FROM memory_edges WHERE from_id = ?1 ORDER BY created_at",
        )?;
        let outgoing = stmt
            .query_map(rusqlite::params![id], row_to_edge)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut stmt2 = self.conn.prepare(
            "SELECT from_id, to_id, kind, created_at FROM memory_edges WHERE to_id = ?1 ORDER BY created_at",
        )?;
        let incoming = stmt2
            .query_map(rusqlite::params![id], row_to_edge)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok((outgoing, incoming))
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

    /// Return all SHAs stored in source_ref (used by harvest to avoid duplicates).
    /// Also includes SHAs stored as "git:<sha>" tags for backwards compatibility.
    pub fn harvested_shas(&self) -> Result<std::collections::HashSet<String>> {
        let mut shas = std::collections::HashSet::new();

        // Primary: source_ref column (new provenance field).
        let mut stmt = self
            .conn
            .prepare_cached("SELECT source_ref FROM notes WHERE source_ref IS NOT NULL")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for row in rows {
            shas.insert(row?);
        }

        // Backwards compat: legacy "git:<sha>" tags written by older versions.
        let mut stmt2 = self
            .conn
            .prepare_cached("SELECT tags FROM notes WHERE tags LIKE '%git:%'")?;
        let rows2 = stmt2.query_map([], |r| r.get::<_, Option<String>>(0))?;
        for row in rows2 {
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

    /// Check whether any memory entry already has the given source_ref (exact match).
    /// Used by harvest for idempotency before inserting.
    pub fn has_source_ref(&self, sha: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM notes WHERE source_ref = ?1 LIMIT 1",
            rusqlite::params![sha],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn get(&self, id: i64) -> Result<Option<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, body, tags, linked_files, created_at, status, superseded_by, source_ref, valid_at, invalid_at
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
        source_ref: row.get(9)?,
        valid_at: row.get(10)?,
        invalid_at: row.get(11)?,
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
        source_ref: row.get(9)?,
        valid_at: row.get(10)?,
        invalid_at: row.get(11)?,
        distance: Some(row.get(12)?),
        score: None,
    })
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEdge> {
    Ok(MemoryEdge {
        from_id: row.get(0)?,
        to_id: row.get(1)?,
        kind: row.get(2)?,
        created_at: row.get(3)?,
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

#[cfg(test)]
mod tests {
    use super::MemoryStore;
    use std::sync::OnceLock;

    /// Register the sqlite-vec extension exactly once per test process.
    /// `MemoryStore::migrate()` creates a `vec0` virtual table, which
    /// requires the extension to be loaded before any connection is opened.
    fn register_sqlite_vec() {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            #[allow(clippy::missing_transmute_annotations)]
            unsafe {
                rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                    sqlite_vec::sqlite3_vec_init as *const (),
                )));
            }
        });
    }

    fn open_store() -> MemoryStore {
        register_sqlite_vec();
        MemoryStore::open(std::path::Path::new(":memory:"))
            .expect("failed to open in-memory MemoryStore")
    }

    fn count_edges(store: &MemoryStore, from_id: i64, to_id: i64, kind: &str) -> i64 {
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memory_edges WHERE from_id = ?1 AND to_id = ?2 AND kind = ?3",
                rusqlite::params![from_id, to_id, kind],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    // ── supersede() ──────────────────────────────────────────────────────────

    #[test]
    fn supersede_happy_path() {
        let store = open_store();

        let old_id = store
            .add_note("decision", "Old decision", "old body", &[], &[], None, None)
            .unwrap();
        let new_id = store
            .add_note("decision", "New decision", "new body", &[], &[], None, None)
            .unwrap();

        let changed = store.supersede(old_id, new_id).unwrap();
        assert!(changed, "supersede() should return true on first call");

        // (a) old note must be archived with superseded_by set
        let old_note = store.get(old_id).unwrap().expect("old note must exist");
        assert_eq!(old_note.status, "archived");
        assert_eq!(old_note.superseded_by, Some(new_id));

        // (b) a memory_edges row must exist linking new → old
        assert_eq!(
            count_edges(&store, new_id, old_id, "supersedes"),
            1,
            "expected exactly one supersedes edge"
        );
    }

    #[test]
    fn supersede_idempotent() {
        let store = open_store();

        let old_id = store
            .add_note("note", "Alpha", "body", &[], &[], None, None)
            .unwrap();
        let new_id = store
            .add_note("note", "Beta", "body", &[], &[], None, None)
            .unwrap();

        let first = store.supersede(old_id, new_id).unwrap();
        assert!(first);

        // Second call on an already-archived note must return false
        let second = store.supersede(old_id, new_id).unwrap();
        assert!(
            !second,
            "supersede() should return false when note is already archived"
        );

        // Must not have inserted a duplicate edge
        assert_eq!(
            count_edges(&store, new_id, old_id, "supersedes"),
            1,
            "duplicate supersedes edge must not be inserted"
        );
    }

    // ── add_edge() ───────────────────────────────────────────────────────────

    #[test]
    fn add_edge_valid_kinds_accepted() {
        let store = open_store();
        let a = store
            .add_note("note", "A", "", &[], &[], None, None)
            .unwrap();
        let b = store
            .add_note("note", "B", "", &[], &[], None, None)
            .unwrap();

        for kind in ["supersedes", "relates_to", "contradicts"] {
            store
                .add_edge(a, b, kind)
                .unwrap_or_else(|e| panic!("add_edge with kind '{kind}' failed: {e}"));
        }
    }

    #[test]
    fn add_edge_invalid_kind_returns_err() {
        let store = open_store();
        let a = store
            .add_note("note", "A", "", &[], &[], None, None)
            .unwrap();
        let b = store
            .add_note("note", "B", "", &[], &[], None, None)
            .unwrap();

        let err = store
            .add_edge(a, b, "invented")
            .expect_err("add_edge with invalid kind must return Err");
        assert!(
            err.to_string().contains("invented"),
            "error message must mention the invalid kind; got: {err}"
        );
    }

    #[test]
    fn add_edge_duplicate_silently_ignored() {
        let store = open_store();
        let a = store
            .add_note("note", "A", "", &[], &[], None, None)
            .unwrap();
        let b = store
            .add_note("note", "B", "", &[], &[], None, None)
            .unwrap();

        store.add_edge(a, b, "relates_to").unwrap();
        store.add_edge(a, b, "relates_to").unwrap(); // second call must not error

        assert_eq!(
            count_edges(&store, a, b, "relates_to"),
            1,
            "duplicate edge must not produce a second row"
        );
    }
}
