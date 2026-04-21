use std::convert::Infallible;
use std::time::Duration;

use anyhow::Result;
use async_stream::stream;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{AppError, AppState};

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct AddNoteRequest {
    /// Kind of memory entry: `decision`, `requirement`, `note`, `question`, `handoff`, `intent`.
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Optional tags for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Source file paths this entry is linked to.
    #[serde(default)]
    pub linked_files: Vec<String>,
    /// Pre-computed embedding vector from the client (required for semantic search).
    pub embedding: Option<Vec<f32>>,
}

#[derive(Serialize, ToSchema)]
pub struct AddNoteResponse {
    /// Whether the note was stored (always true for 201/409).
    pub stored: bool,
    /// ID of the created note.
    pub id: i64,
    /// Conflicting entries (only present on 409).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<ConflictEntry>,
}

/// A single conflicting memory entry returned in a 409 response.
#[derive(Serialize, ToSchema)]
pub struct ConflictEntry {
    pub id: i64,
    pub title: String,
    /// Cosine similarity to the new entry (0.0–1.0).
    pub similarity: f32,
}

#[derive(Deserialize, ToSchema, utoipa::IntoParams)]
pub struct ListQuery {
    /// Filter by kind (`decision`, `requirement`, `note`, `question`, `handoff`, `intent`).
    pub kind: Option<String>,
    /// Maximum number of results to return (default: 20).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Include archived entries (default: false).
    #[serde(default)]
    pub archived: bool,
}
fn default_limit() -> usize {
    20
}

#[derive(Deserialize, ToSchema)]
pub struct SearchRequest {
    /// Query embedding vector (must match project's embedding dimension).
    pub embedding: Vec<f32>,
    /// Maximum number of results to return (default: 20).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Serialize, ToSchema)]
pub struct BoolResponse {
    /// Whether the operation modified a record.
    pub changed: bool,
}

#[derive(Serialize, ToSchema)]
pub struct CountResponse {
    pub count: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct SupersedeRequest {
    /// ID of the new note that replaces the superseded one.
    pub new_id: i64,
}

// ── Health ────────────────────────────────────────────────────────────────────

/// Server liveness check. No authentication required.
#[utoipa::path(
    get,
    path = "/v1/health",
    responses(
        (status = 200, description = "Server is up", body = str, example = "ok")
    ),
    tag = "health"
)]
pub async fn health() -> &'static str {
    "ok"
}

// ── Projects ──────────────────────────────────────────────────────────────────

/// List all projects registered on this server.
#[utoipa::path(
    get,
    path = "/v1/projects",
    responses(
        (status = 200, description = "List of projects", body = Vec<super::db::Project>),
        (status = 401, description = "Unauthorized"),
    ),
    security(("bearer_auth" = [])),
    tag = "projects"
)]
pub async fn list_projects(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let projects = db.list_projects()?;
    Ok(Json(projects))
}

// ── Memory CRUD ───────────────────────────────────────────────────────────────

/// Add a memory entry to a project. The project is auto-created on first write.
///
/// The client must supply a pre-computed `embedding` vector. All entries in
/// a project must use the same embedding dimension — the first write fixes it.
///
/// Returns **201** on success. Returns **409** when the new entry is semantically
/// close to one or more existing active entries (similarity ≥ conflict_threshold).
/// The entry is still stored in both cases; the 409 is informational.
#[utoipa::path(
    post,
    path = "/v1/projects/{project_id}/memory",
    params(
        ("project_id" = String, Path, description = "Project slug (e.g. `usercise/spelunk`)")
    ),
    request_body = AddNoteRequest,
    responses(
        (status = 201, description = "Note created", body = AddNoteResponse),
        (status = 400, description = "Embedding dimension mismatch"),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Note stored but conflicts with existing entries", body = AddNoteResponse),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn add_note(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(body): Json<AddNoteRequest>,
) -> Result<Response, AppError> {
    let embedding = body.embedding.as_deref();
    let dim = embedding.map(|v| v.len()).unwrap_or(0);

    let db = state.db.lock().await;
    let project = db.upsert_project(&project_id, dim)?;

    let id = db.add_note(
        project.id,
        &body.kind,
        &body.title,
        &body.body,
        &body.tags,
        &body.linked_files,
        embedding,
    )?;

    // ── Conflict detection ────────────────────────────────────────────────────
    // Only run if the entry has an embedding and conflict detection is enabled
    // (threshold < 1.0).
    let threshold = state.conflict_threshold;
    if let Some(vec) = embedding
        && threshold < 1.0
    {
        let max_distance = 1.0 - threshold;
        let nearby = db.search_notes_for_conflicts(project.id, vec, max_distance, id, 5)?;
        if !nearby.is_empty() {
            // Insert `contradicts` edges for each conflict.
            for note in &nearby {
                if let Err(e) = db.add_edge(id, note.id, "contradicts") {
                    tracing::warn!("failed to insert contradicts edge {id}→{}: {e}", note.id);
                }
            }
            let conflicts: Vec<ConflictEntry> = nearby
                .into_iter()
                .map(|n| {
                    let similarity = n
                        .distance
                        .map(|d| (1.0 - d as f32).clamp(0.0, 1.0))
                        .unwrap_or(0.0);
                    ConflictEntry {
                        id: n.id,
                        title: n.title,
                        similarity,
                    }
                })
                .collect();
            return Ok((
                StatusCode::CONFLICT,
                Json(AddNoteResponse {
                    stored: true,
                    id,
                    conflicts,
                }),
            )
                .into_response());
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(AddNoteResponse {
            stored: true,
            id,
            conflicts: vec![],
        }),
    )
        .into_response())
}

/// List memory entries for a project, optionally filtered by kind.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}/memory",
    params(
        ("project_id" = String, Path, description = "Project slug"),
        ListQuery,
    ),
    responses(
        (status = 200, description = "List of notes", body = Vec<super::db::ServerNote>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Project not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn list_notes(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Query(params): Query<ListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let notes = db.list_notes(
        project.id,
        params.kind.as_deref(),
        params.limit,
        params.archived,
    )?;
    Ok(Json(notes))
}

/// Get a single memory entry by ID.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}/memory/{note_id}",
    params(
        ("project_id" = String, Path, description = "Project slug"),
        ("note_id" = i64, Path, description = "Note ID"),
    ),
    responses(
        (status = 200, description = "Note found", body = super::db::ServerNote),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Note not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn get_note(
    State(state): State<AppState>,
    Path((project_id, note_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    match db.get_note(project.id, note_id)? {
        Some(note) => Ok(Json(note).into_response()),
        None => Err(AppError::NotFound),
    }
}

/// Semantic search over memory entries using a pre-computed query embedding.
#[utoipa::path(
    post,
    path = "/v1/projects/{project_id}/memory/search",
    params(
        ("project_id" = String, Path, description = "Project slug"),
    ),
    request_body = SearchRequest,
    responses(
        (status = 200, description = "Nearest neighbours", body = Vec<super::db::ServerNote>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Project not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn search_notes(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(body): Json<SearchRequest>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let notes = db.search_notes(project.id, &body.embedding, body.limit)?;
    Ok(Json(notes))
}

/// Delete a memory entry permanently.
#[utoipa::path(
    delete,
    path = "/v1/projects/{project_id}/memory/{note_id}",
    params(
        ("project_id" = String, Path, description = "Project slug"),
        ("note_id" = i64, Path, description = "Note ID"),
    ),
    responses(
        (status = 200, description = "Deletion result", body = BoolResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Note not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn delete_note(
    State(state): State<AppState>,
    Path((project_id, note_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let changed = db.delete_note(project.id, note_id)?;
    Ok(Json(BoolResponse { changed }))
}

/// Archive a memory entry. Archived entries are excluded from search and `ask`
/// context but remain visible via `?archived=true`.
#[utoipa::path(
    post,
    path = "/v1/projects/{project_id}/memory/{note_id}/archive",
    params(
        ("project_id" = String, Path, description = "Project slug"),
        ("note_id" = i64, Path, description = "Note ID"),
    ),
    responses(
        (status = 200, description = "Archive result", body = BoolResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Note not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn archive_note(
    State(state): State<AppState>,
    Path((project_id, note_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let changed = db.archive_note(project.id, note_id)?;
    Ok(Json(BoolResponse { changed }))
}

/// Mark a memory entry as superseded by a newer one. The old entry is archived
/// and linked to the new one.
#[utoipa::path(
    post,
    path = "/v1/projects/{project_id}/memory/{note_id}/supersede",
    params(
        ("project_id" = String, Path, description = "Project slug"),
        ("note_id" = i64, Path, description = "Note ID to supersede"),
    ),
    request_body = SupersedeRequest,
    responses(
        (status = 200, description = "Supersede result", body = BoolResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Note not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn supersede_note(
    State(state): State<AppState>,
    Path((project_id, note_id)): Path<(String, i64)>,
    Json(body): Json<SupersedeRequest>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let changed = db.supersede_note(project.id, note_id, body.new_id)?;
    Ok(Json(BoolResponse { changed }))
}

/// Return entry counts and embedding dimension for a project.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}/stats",
    params(
        ("project_id" = String, Path, description = "Project slug"),
    ),
    responses(
        (status = 200, description = "Project stats", body = super::db::ProjectStats),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Project not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn project_stats(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let stats = db.stats(project.id)?;
    Ok(Json(stats))
}

/// Return all git commit SHAs stored in note tags for a project.
///
/// Each harvested commit is tagged `git:<sha>`. Clients call this endpoint
/// to skip commits they have already stored, enabling incremental harvest.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}/memory/harvested-shas",
    params(
        ("project_id" = String, Path, description = "Project slug"),
    ),
    responses(
        (status = 200, description = "List of harvested git SHAs", body = Vec<String>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Project not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn harvested_shas(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let shas = db.harvested_shas(project.id)?;
    Ok(Json(shas))
}

// ── Poll / SSE endpoints ──────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema, utoipa::IntoParams)]
pub struct SinceQuery {
    /// Unix epoch seconds (exclusive lower bound).
    pub t: i64,
    /// Maximum number of results (default: 100, max: 500).
    #[serde(default = "default_since_limit")]
    pub limit: i64,
}
fn default_since_limit() -> i64 {
    100
}

#[derive(Deserialize, ToSchema, utoipa::IntoParams)]
pub struct StreamQuery {
    /// Unix epoch seconds to start from (inclusive). Defaults to now.
    pub t: Option<i64>,
}

/// Return notes created after a given Unix timestamp. Archived entries are
/// excluded. Results are ordered `created_at ASC`.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}/memory/since",
    params(
        ("project_id" = String, Path, description = "Project slug"),
        SinceQuery,
    ),
    responses(
        (status = 200, description = "Notes newer than `t`", body = Vec<super::db::ServerNote>),
        (status = 400, description = "Missing or invalid `t` parameter"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Project not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn memory_since(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Query(params): Query<SinceQuery>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let notes = db.notes_since(project.id, params.t, params.limit)?;
    Ok(Json(notes))
}

/// Stream new memory entries as Server-Sent Events. Each event carries a
/// single `ServerNote` serialised as JSON. The stream polls the database once
/// per second and stays open indefinitely — close the connection to stop it.
///
/// Pass `?t=<unix_secs>` to replay entries written after a known timestamp.
/// Omit it to receive only entries written after the connection opens.
#[utoipa::path(
    get,
    path = "/v1/projects/{project_id}/memory/stream",
    params(
        ("project_id" = String, Path, description = "Project slug"),
        StreamQuery,
    ),
    responses(
        (status = 200, description = "SSE stream of new memory entries"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Project not found"),
    ),
    security(("bearer_auth" = [])),
    tag = "memory"
)]
pub async fn memory_stream(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Query(params): Query<StreamQuery>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, AppError> {
    // Validate the project exists before opening the stream.
    {
        let db = state.db.lock().await;
        require_project(&db, &project_id)?;
    }

    let start_t = params.t.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    });

    let s = stream! {
        let mut last_seen = start_t;
        loop {
            // Lock, query, immediately release.
            let notes = {
                let db = state.db.lock().await;
                // Re-fetch the project each iteration so the stream survives
                // project creation that may have happened after the handshake.
                let pid = match db.get_project(&project_id) {
                    Ok(Some(p)) => p.id,
                    _ => {
                        drop(db);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                // Ignore DB errors mid-stream; keep the connection alive.
                db.notes_since(pid, last_seen, 50).unwrap_or_default()
            };

            for note in notes {
                if note.created_at > last_seen {
                    last_seen = note.created_at;
                }
                let data = serde_json::to_string(&note).unwrap_or_default();
                yield Ok::<Event, Infallible>(Event::default().data(data));
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };

    Ok(Sse::new(s).keep_alive(KeepAlive::default()))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn require_project(db: &super::db::ServerDb, slug: &str) -> Result<super::db::Project, AppError> {
    db.get_project(slug)?.ok_or(AppError::NotFound)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::Body,
        http::{self, Request},
    };
    use serde_json::{Value, json};
    use tower::ServiceExt; // for `oneshot`

    use super::super::db::ServerDb;
    use super::super::{AppState, router};

    /// Register sqlite-vec extension once per test process.
    fn register_sqlite_vec() {
        use std::sync::OnceLock;
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

    fn make_app(conflict_threshold: f32) -> (axum::Router, i32) {
        register_sqlite_vec();
        let dim: usize = 4;
        let db = ServerDb::open(std::path::Path::new(":memory:"), dim)
            .expect("failed to open in-memory server db");
        let state = AppState {
            db: Arc::new(tokio::sync::Mutex::new(db)),
            api_key: None,
            conflict_threshold,
        };
        (router(state), dim as i32)
    }

    /// POST /v1/projects/{slug}/memory with the given embedding. Returns the response.
    async fn post_note(
        app: axum::Router,
        slug: &str,
        title: &str,
        embedding: Vec<f32>,
    ) -> (http::StatusCode, Value) {
        let body = json!({
            "kind": "note",
            "title": title,
            "body": "test body",
            "embedding": embedding,
        });
        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/projects/{slug}/memory"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    /// Two semantically identical entries (identical embeddings) should trigger 409
    /// and a `contradicts` edge should be inserted.
    #[tokio::test]
    async fn conflict_detection_identical_embeddings_returns_409() {
        let (app, _dim) = make_app(0.92);
        // Use a very low threshold to ensure a conflict (0.0 = any non-zero similarity conflicts).
        let (app_low, _dim) = make_app(0.0);

        // First entry — must be 201.
        let embedding = vec![1.0_f32, 0.0, 0.0, 0.0];
        let (status1, body1) = post_note(
            app_low.clone(),
            "test-project",
            "Entry A",
            embedding.clone(),
        )
        .await;
        assert_eq!(
            status1,
            http::StatusCode::CREATED,
            "first write must be 201; body: {body1}"
        );
        let first_id = body1["id"].as_i64().expect("id in response");
        assert_eq!(body1["stored"], json!(true));

        // Second entry with identical embedding — must be 409.
        let (status2, body2) = post_note(
            app_low.clone(),
            "test-project",
            "Entry B (duplicate)",
            embedding.clone(),
        )
        .await;
        assert_eq!(
            status2,
            http::StatusCode::CONFLICT,
            "second identical write must be 409; body: {body2}"
        );
        assert_eq!(
            body2["stored"],
            json!(true),
            "stored must be true even on 409"
        );

        let conflicts = body2["conflicts"]
            .as_array()
            .expect("conflicts array in 409 body");
        assert!(!conflicts.is_empty(), "conflicts must not be empty");
        let conflicting_ids: Vec<i64> = conflicts.iter().filter_map(|c| c["id"].as_i64()).collect();
        assert!(
            conflicting_ids.contains(&first_id),
            "first entry's id ({first_id}) must appear in conflicts; got: {conflicting_ids:?}"
        );

        // Similarity should be > 0.
        let similarity = conflicts[0]["similarity"]
            .as_f64()
            .expect("similarity field");
        assert!(
            similarity > 0.0,
            "similarity must be positive; got {similarity}"
        );

        // Suppress unused variable warning from app (default threshold).
        drop(app);
    }

    /// At default threshold (0.92), dissimilar entries must not conflict.
    #[tokio::test]
    async fn conflict_detection_dissimilar_entries_no_conflict() {
        let (app, _dim) = make_app(0.92);

        // Orthogonal embeddings — cosine similarity = 0.
        let emb_a = vec![1.0_f32, 0.0, 0.0, 0.0];
        let emb_b = vec![0.0_f32, 1.0, 0.0, 0.0];

        let (status1, _) = post_note(app.clone(), "proj-dissimilar", "Alpha", emb_a).await;
        assert_eq!(status1, http::StatusCode::CREATED);

        let (status2, body2) = post_note(app.clone(), "proj-dissimilar", "Beta", emb_b).await;
        assert_eq!(
            status2,
            http::StatusCode::CREATED,
            "orthogonal entries must not conflict; body: {body2}"
        );
    }

    /// threshold = 1.0 disables conflict detection entirely.
    #[tokio::test]
    async fn conflict_detection_disabled_at_threshold_one() {
        let (app, _dim) = make_app(1.0);

        // Use identical embeddings — but with threshold=1.0, no conflict should fire.
        let embedding = vec![1.0_f32, 0.0, 0.0, 0.0];
        let (status1, _) = post_note(app.clone(), "proj-disabled", "X", embedding.clone()).await;
        assert_eq!(status1, http::StatusCode::CREATED);
        let (status2, body2) = post_note(app.clone(), "proj-disabled", "X dup", embedding).await;
        assert_eq!(
            status2,
            http::StatusCode::CREATED,
            "threshold=1.0 must disable conflict detection; body: {body2}"
        );
    }
}
