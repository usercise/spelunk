pub mod db;
pub mod handlers;

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};

use db::ServerDb;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<tokio::sync::Mutex<ServerDb>>,
    pub api_key: Option<String>,
}

/// Build the axum router with all routes.
pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/v1/projects",                                           get(handlers::list_projects))
        .route("/v1/projects/{project_id}/memory",                      post(handlers::add_note))
        .route("/v1/projects/{project_id}/memory",                      get(handlers::list_notes))
        .route("/v1/projects/{project_id}/memory/search",               post(handlers::search_notes))
        .route("/v1/projects/{project_id}/memory/{note_id}",            get(handlers::get_note))
        .route("/v1/projects/{project_id}/memory/{note_id}",            delete(handlers::delete_note))
        .route("/v1/projects/{project_id}/memory/{note_id}/archive",    post(handlers::archive_note))
        .route("/v1/projects/{project_id}/memory/{note_id}/supersede",  post(handlers::supersede_note))
        .route("/v1/projects/{project_id}/stats",                       get(handlers::project_stats))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    Router::new()
        .route("/v1/health", get(handlers::health))
        .merge(protected)
        .with_state(state)
}

/// Bearer token auth middleware. Pass-through if no API key is configured.
async fn auth_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
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
