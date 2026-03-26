use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use super::{AppError, AppState};

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AddNoteRequest {
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub linked_files: Vec<String>,
    /// Pre-computed embedding from the client (required).
    pub embedding: Option<Vec<f32>>,
}

#[derive(Serialize)]
pub struct AddNoteResponse {
    pub id: i64,
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub kind: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub archived: bool,
}
fn default_limit() -> usize { 20 }

#[derive(Deserialize)]
pub struct SearchRequest {
    pub embedding: Vec<f32>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Serialize)]
pub struct BoolResponse {
    pub changed: bool,
}

#[derive(Serialize)]
pub struct CountResponse {
    pub count: i64,
}

#[derive(Deserialize)]
pub struct SupersedeRequest {
    pub new_id: i64,
}

// ── Health ────────────────────────────────────────────────────────────────────

pub async fn health() -> &'static str { "ok" }

// ── Projects ──────────────────────────────────────────────────────────────────

pub async fn list_projects(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let projects = db.list_projects()?;
    Ok(Json(projects))
}

// ── Memory CRUD ───────────────────────────────────────────────────────────────

pub async fn add_note(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(body): Json<AddNoteRequest>,
) -> Result<impl IntoResponse, AppError> {
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

    Ok((StatusCode::CREATED, Json(AddNoteResponse { id })))
}

pub async fn list_notes(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Query(params): Query<ListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let notes = db.list_notes(project.id, params.kind.as_deref(), params.limit, params.archived)?;
    Ok(Json(notes))
}

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

pub async fn delete_note(
    State(state): State<AppState>,
    Path((project_id, note_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let changed = db.delete_note(project.id, note_id)?;
    Ok(Json(BoolResponse { changed }))
}

pub async fn archive_note(
    State(state): State<AppState>,
    Path((project_id, note_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let changed = db.archive_note(project.id, note_id)?;
    Ok(Json(BoolResponse { changed }))
}

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

pub async fn project_stats(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.lock().await;
    let project = require_project(&db, &project_id)?;
    let stats = db.stats(project.id)?;
    Ok(Json(stats))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn require_project(
    db: &super::db::ServerDb,
    slug: &str,
) -> Result<super::db::Project, AppError> {
    db.get_project(slug)?.ok_or(AppError::NotFound)
}
