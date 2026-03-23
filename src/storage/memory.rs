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

    /// Semantic KNN search. Returns notes ordered by ascending distance.
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
                    n.created_at, CAST(k.distance AS REAL)
             FROM   knn k
             JOIN   notes n ON n.id = k.note_id
             ORDER  BY k.distance"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let notes = stmt
            .query_map(rusqlite::params![query_blob], row_to_note_with_distance)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(notes)
    }

    /// List notes, optionally filtered by kind, newest first.
    pub fn list(&self, kind_filter: Option<&str>, limit: usize) -> Result<Vec<Note>> {
        let limit = limit.min(500);
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
            if let Some(kind) = kind_filter {
                (
                    format!(
                        "SELECT id, kind, title, body, tags, linked_files, created_at
                         FROM notes WHERE kind = ?1 ORDER BY created_at DESC LIMIT {limit}"
                    ),
                    vec![Box::new(kind.to_string())],
                )
            } else {
                (
                    format!(
                        "SELECT id, kind, title, body, tags, linked_files, created_at
                         FROM notes ORDER BY created_at DESC LIMIT {limit}"
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

    pub fn get(&self, id: i64) -> Result<Option<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, body, tags, linked_files, created_at
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
        distance: Some(row.get(7)?),
    })
}

fn split_csv(s: Option<&str>) -> Vec<String> {
    match s {
        None | Some("") => vec![],
        Some(s) => s.split(',').map(|p| p.trim().to_string()).filter(|s| !s.is_empty()).collect(),
    }
}
