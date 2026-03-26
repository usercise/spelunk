use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::backend::{MemoryBackend, NoteInput};
use super::memory::Note;
use crate::embeddings::blob_to_vec;

/// HTTP client for the spelunk-server REST API.
///
/// All routes are scoped under `/v1/projects/{project_id}/`.
pub struct RemoteMemoryBackend {
    pub client: reqwest::Client,
    pub base_url: String,
    pub project_id: String,
    pub api_key: Option<String>,
}

impl RemoteMemoryBackend {
    fn url(&self, path: &str) -> String {
        format!("{}/v1/projects/{}/{}", self.base_url.trim_end_matches('/'), self.project_id, path)
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(key) = &self.api_key {
            req.header("Authorization", format!("Bearer {key}"))
        } else {
            req
        }
    }
}

// ── Wire types (match server JSON schema) ─────────────────────────────────────

#[derive(Serialize)]
struct AddNoteRequest {
    kind: String,
    title: String,
    body: String,
    tags: Vec<String>,
    linked_files: Vec<String>,
    embedding: Option<Vec<f32>>,
}

#[derive(Deserialize)]
struct AddNoteResponse {
    id: i64,
}

#[derive(Deserialize)]
struct NoteResponse {
    id: i64,
    kind: String,
    title: String,
    body: String,
    tags: Vec<String>,
    linked_files: Vec<String>,
    created_at: i64,
    status: String,
    superseded_by: Option<i64>,
    #[serde(default)]
    distance: Option<f64>,
}

impl From<NoteResponse> for Note {
    fn from(r: NoteResponse) -> Self {
        Note {
            id: r.id,
            kind: r.kind,
            title: r.title,
            body: r.body,
            tags: r.tags,
            linked_files: r.linked_files,
            created_at: r.created_at,
            status: r.status,
            superseded_by: r.superseded_by,
            distance: r.distance,
        }
    }
}

#[derive(Serialize)]
struct SearchRequest {
    embedding: Vec<f32>,
    limit: usize,
}

#[derive(Serialize)]
struct SupersedeRequest {
    new_id: i64,
}

#[derive(Deserialize)]
struct BoolResponse {
    changed: bool,
}

#[derive(Deserialize)]
struct CountResponse {
    count: i64,
}

// ── Trait implementation ──────────────────────────────────────────────────────

#[async_trait]
impl MemoryBackend for RemoteMemoryBackend {
    async fn add(&self, input: NoteInput) -> Result<i64> {
        let embedding = input.embedding.as_deref().map(blob_to_vec);
        let body = AddNoteRequest {
            kind: input.kind,
            title: input.title,
            body: input.body,
            tags: input.tags,
            linked_files: input.linked_files,
            embedding,
        };
        let resp = self
            .authed(self.client.post(self.url("memory")))
            .json(&body)
            .send()
            .await
            .context("POST /memory")?
            .error_for_status()
            .context("server returned error for POST /memory")?
            .json::<AddNoteResponse>()
            .await
            .context("parsing POST /memory response")?;
        Ok(resp.id)
    }

    async fn search(&self, query_blob: &[u8], limit: usize) -> Result<Vec<Note>> {
        let embedding = blob_to_vec(query_blob);
        let body = SearchRequest { embedding, limit };
        let resp = self
            .authed(self.client.post(self.url("memory/search")))
            .json(&body)
            .send()
            .await
            .context("POST /memory/search")?
            .error_for_status()
            .context("server returned error for POST /memory/search")?
            .json::<Vec<NoteResponse>>()
            .await
            .context("parsing search response")?;
        Ok(resp.into_iter().map(Into::into).collect())
    }

    async fn list(
        &self,
        kind_filter: Option<&str>,
        limit: usize,
        include_archived: bool,
    ) -> Result<Vec<Note>> {
        let mut req = self.client.get(self.url("memory")).query(&[
            ("limit", limit.to_string().as_str()),
            ("archived", if include_archived { "true" } else { "false" }),
        ]);
        if let Some(kind) = kind_filter {
            req = req.query(&[("kind", kind)]);
        }
        let resp = self
            .authed(req)
            .send()
            .await
            .context("GET /memory")?
            .error_for_status()
            .context("server returned error for GET /memory")?
            .json::<Vec<NoteResponse>>()
            .await
            .context("parsing list response")?;
        Ok(resp.into_iter().map(Into::into).collect())
    }

    async fn get(&self, id: i64) -> Result<Option<Note>> {
        let resp = self
            .authed(self.client.get(self.url(&format!("memory/{id}"))))
            .send()
            .await
            .context("GET /memory/{id}")?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let note = resp
            .error_for_status()
            .context("server returned error for GET /memory/{id}")?
            .json::<NoteResponse>()
            .await
            .context("parsing get response")?;
        Ok(Some(note.into()))
    }

    async fn count(&self) -> Result<i64> {
        let resp = self
            .authed(self.client.get(self.url("stats")))
            .send()
            .await
            .context("GET /stats")?
            .error_for_status()
            .context("server returned error for GET /stats")?
            .json::<CountResponse>()
            .await
            .context("parsing stats response")?;
        Ok(resp.count)
    }

    async fn archive(&self, id: i64) -> Result<bool> {
        let resp = self
            .authed(self.client.post(self.url(&format!("memory/{id}/archive"))))
            .send()
            .await
            .context("POST /memory/{id}/archive")?
            .error_for_status()
            .context("server returned error for POST /memory/{id}/archive")?
            .json::<BoolResponse>()
            .await
            .context("parsing archive response")?;
        Ok(resp.changed)
    }

    async fn supersede(&self, old_id: i64, new_id: i64) -> Result<bool> {
        let body = SupersedeRequest { new_id };
        let resp = self
            .authed(self.client.post(self.url(&format!("memory/{old_id}/supersede"))))
            .json(&body)
            .send()
            .await
            .context("POST /memory/{id}/supersede")?
            .error_for_status()
            .context("server returned error for POST /memory/{id}/supersede")?
            .json::<BoolResponse>()
            .await
            .context("parsing supersede response")?;
        Ok(resp.changed)
    }

    async fn harvested_shas(&self) -> Result<HashSet<String>> {
        // The server doesn't expose a harvested-SHA query endpoint yet.
        // Return empty set — remote mode won't deduplicate harvest across runs.
        // TODO(Phase 3): add GET /memory?tags_contains=git: to server API.
        Ok(HashSet::new())
    }
}
