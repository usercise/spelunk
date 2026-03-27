pub mod db;
pub mod handlers;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use utoipa::OpenApi;

use db::ServerDb;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<tokio::sync::Mutex<ServerDb>>,
    pub api_key: Option<String>,
}

// ── OpenAPI spec ──────────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title = "spelunk-server",
        version = "0.1.0",
        description = "Shared memory server for spelunk. Stores decisions, requirements, \
                        and context for a team and serves them over HTTP. Clients embed \
                        locally and send pre-computed vectors; the server stores and searches them.",
        contact(name = "spelunk", url = "https://github.com/usercise/spelunk"),
        license(name = "MIT"),
    ),
    paths(
        handlers::health,
        handlers::list_projects,
        handlers::add_note,
        handlers::list_notes,
        handlers::get_note,
        handlers::search_notes,
        handlers::delete_note,
        handlers::archive_note,
        handlers::supersede_note,
        handlers::project_stats,
        handlers::harvested_shas,
    ),
    components(schemas(
        handlers::AddNoteRequest,
        handlers::AddNoteResponse,
        handlers::ListQuery,
        handlers::SearchRequest,
        handlers::BoolResponse,
        handlers::CountResponse,
        handlers::SupersedeRequest,
        db::Project,
        db::ServerNote,
        db::ProjectStats,
    )),
    tags(
        (name = "health", description = "Liveness"),
        (name = "projects", description = "Project management"),
        (name = "memory", description = "Memory CRUD and semantic search"),
    ),
    security(
        ("bearer_auth" = [])
    ),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer_auth",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("API key")
                    .description(Some(
                        "Pass as `Authorization: Bearer <key>`. \
                         Not required when no key is configured on the server.",
                    ))
                    .build(),
            ),
        );
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

/// Build the axum router with all routes.
pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/v1/projects", get(handlers::list_projects))
        .route("/v1/projects/{project_id}/memory", post(handlers::add_note))
        .route(
            "/v1/projects/{project_id}/memory",
            get(handlers::list_notes),
        )
        .route(
            "/v1/projects/{project_id}/memory/search",
            post(handlers::search_notes),
        )
        .route(
            "/v1/projects/{project_id}/memory/harvested-shas",
            get(handlers::harvested_shas),
        )
        .route(
            "/v1/projects/{project_id}/memory/{note_id}",
            get(handlers::get_note),
        )
        .route(
            "/v1/projects/{project_id}/memory/{note_id}",
            delete(handlers::delete_note),
        )
        .route(
            "/v1/projects/{project_id}/memory/{note_id}/archive",
            post(handlers::archive_note),
        )
        .route(
            "/v1/projects/{project_id}/memory/{note_id}/supersede",
            post(handlers::supersede_note),
        )
        .route(
            "/v1/projects/{project_id}/stats",
            get(handlers::project_stats),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .route("/v1/health", get(handlers::health))
        .route("/api-docs/openapi.json", get(openapi_spec))
        .merge(protected)
        .with_state(state)
}

// ── OpenAPI spec endpoint ─────────────────────────────────────────────────────

/// Serve the OpenAPI spec as JSON. Import into Postman via
/// `File → Import → Link` using the server URL + `/api-docs/openapi.json`.
async fn openapi_spec() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

// ── Auth middleware ───────────────────────────────────────────────────────────

/// Bearer token auth middleware. Pass-through if no API key is configured.
async fn auth_middleware(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if let Some(expected_key) = &state.api_key {
        let auth = request
            .headers()
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match auth {
            Some(token) if token == expected_key => next.run(request).await,
            _ => (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
        }
    } else {
        next.run(request).await
    }
}

// ── Error type ────────────────────────────────────────────────────────────────

/// Map anyhow errors to HTTP responses.
pub enum AppError {
    NotFound,
    Internal(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "Not found").into_response(),
            AppError::Internal(e) => {
                let msg = e.to_string();
                // Surface dimension-mismatch and other user-facing errors as 400.
                if msg.contains("mismatch") || msg.contains("required") {
                    (StatusCode::BAD_REQUEST, msg).into_response()
                } else {
                    tracing::error!("internal error: {e:#}");
                    (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response()
                }
            }
        }
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError::Internal(e.into())
    }
}
