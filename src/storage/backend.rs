use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;

use super::memory::{MemoryStore, Note};

/// Input for adding a note. Owned to avoid lifetime issues across async boundaries.
pub struct NoteInput {
    pub kind: String,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub linked_files: Vec<String>,
    /// Raw embedding blob (little-endian f32 bytes). `None` = no embedding stored.
    pub embedding: Option<Vec<u8>>,
}

/// Abstraction over local SQLite and remote HTTP memory stores.
#[async_trait]
pub trait MemoryBackend: Send {
    async fn add(&self, input: NoteInput) -> Result<i64>;
    async fn search(&self, query_blob: &[u8], limit: usize) -> Result<Vec<Note>>;
    async fn list(
        &self,
        kind_filter: Option<&str>,
        limit: usize,
        include_archived: bool,
    ) -> Result<Vec<Note>>;
    async fn get(&self, id: i64) -> Result<Option<Note>>;
    async fn count(&self) -> Result<i64>;
    async fn archive(&self, id: i64) -> Result<bool>;
    async fn supersede(&self, old_id: i64, new_id: i64) -> Result<bool>;
    async fn harvested_shas(&self) -> Result<HashSet<String>>;
}

// ── Local SQLite backend ──────────────────────────────────────────────────────

/// Wraps `MemoryStore` in a `tokio::sync::Mutex` so `LocalMemoryBackend: Send + Sync`,
/// satisfying the `async-trait` Send constraint without needing spawn_blocking.
pub struct LocalMemoryBackend {
    store: tokio::sync::Mutex<MemoryStore>,
}

impl LocalMemoryBackend {
    pub fn new(store: MemoryStore) -> Self {
        Self {
            store: tokio::sync::Mutex::new(store),
        }
    }
}

#[async_trait]
impl MemoryBackend for LocalMemoryBackend {
    async fn add(&self, input: NoteInput) -> Result<i64> {
        let store = self.store.lock().await;
        let tags: Vec<&str> = input.tags.iter().map(String::as_str).collect();
        let files: Vec<&str> = input.linked_files.iter().map(String::as_str).collect();
        let id = store.add_note(&input.kind, &input.title, &input.body, &tags, &files)?;
        if let Some(blob) = &input.embedding {
            store.insert_embedding(id, blob)?;
        }
        Ok(id)
    }

    async fn search(&self, query_blob: &[u8], limit: usize) -> Result<Vec<Note>> {
        self.store.lock().await.search(query_blob, limit)
    }

    async fn list(
        &self,
        kind_filter: Option<&str>,
        limit: usize,
        include_archived: bool,
    ) -> Result<Vec<Note>> {
        self.store
            .lock()
            .await
            .list(kind_filter, limit, include_archived)
    }

    async fn get(&self, id: i64) -> Result<Option<Note>> {
        self.store.lock().await.get(id)
    }

    async fn count(&self) -> Result<i64> {
        self.store.lock().await.count()
    }

    async fn archive(&self, id: i64) -> Result<bool> {
        self.store.lock().await.archive(id)
    }

    async fn supersede(&self, old_id: i64, new_id: i64) -> Result<bool> {
        self.store.lock().await.supersede(old_id, new_id)
    }

    async fn harvested_shas(&self) -> Result<HashSet<String>> {
        self.store.lock().await.harvested_shas()
    }
}
