use anyhow::{Context, Result};
use rusqlite::Connection;
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
        self.conn.execute_batch(include_str!("../../migrations/004_memory.sql"))
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
            rusqlite::params![
                kind,
                title,
                body,
                tags.join(","),
                linked_files.join(","),
            ],
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

    /// List notes, optionally filtered by kind, newest first.
    /// When `include_archived` is false only active entries are returned.
    pub fn list(&self, kind_filter: Option<&str>, limit: usize, include_archived: bool) -> Result<Vec<Note>> {
        let limit = limit.min(500);
        let status_clause = if include_archived { "" } else { "AND status = 'active'" };
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
            if let Some(kind) = kind_filter {
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

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM notes WHERE status = 'active'", [], |r| r.get(0))?)
    }

    /// Return all SHA tags stored (used by harvest to avoid duplicates).
    /// Harvest stores the git SHA as a tag in the format "git:<sha>".
    pub fn harvested_shas(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare_cached("SELECT tags FROM notes WHERE tags LIKE '%git:%'")?;
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
    })
}

fn split_csv(s: Option<&str>) -> Vec<String> {
    match s {
        None | Some("") => vec![],
        Some(s) => s.split(',').map(|p| p.trim().to_string()).filter(|s| !s.is_empty()).collect(),
    }
}
