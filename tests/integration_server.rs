//! Integration tests for spelunk-server HTTP handlers using axum's oneshot testing.
//!
//! No real TCP socket is opened — requests go directly through the router.
//! sqlite-vec must be registered before any `ServerDb` is opened, so all
//! tests in this file use `#[serial]`.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serial_test::serial;
use spelunk::server::{AppState, router};
use std::sync::Arc;
use tower::ServiceExt; // for `.oneshot()`

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_state() -> AppState {
    let db = common::open_test_server_db(4);
    AppState {
        db: Arc::new(tokio::sync::Mutex::new(db)),
        api_key: None,
    }
}

fn json_body(body: impl serde::Serialize) -> Body {
    Body::from(serde_json::to_vec(&body).unwrap())
}

async fn send(
    state: AppState,
    method: &str,
    uri: &str,
    body: Body,
    content_type: bool,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if content_type {
        builder = builder.header("content-type", "application/json");
    }
    let req = builder.body(body).unwrap();
    router(state).oneshot(req).await.unwrap()
}

// ── health ───────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn health_returns_ok() {
    let resp = send(make_state(), "GET", "/v1/health", Body::empty(), false).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// ── list_projects ─────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn list_projects_empty_initially() {
    let resp = send(make_state(), "GET", "/v1/projects", Body::empty(), false).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let projects: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(projects, serde_json::json!([]));
}

// ── add_note + list_notes ─────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn add_note_creates_project_automatically() {
    let state = make_state();
    let payload = serde_json::json!({
        "kind": "decision",
        "title": "Use SQLite",
        "body": "Simpler than Postgres for local use.",
        "embedding": [0.1_f32, 0.2, 0.3, 0.4],
    });

    let resp = send(
        state.clone(),
        "POST",
        "/v1/projects/test-project/memory",
        json_body(&payload),
        true,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Project should now exist.
    let resp2 = send(state, "GET", "/v1/projects", Body::empty(), false).await;
    let bytes = axum::body::to_bytes(resp2.into_body(), usize::MAX)
        .await
        .unwrap();
    let projects: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["slug"], "test-project");
}

#[tokio::test]
#[serial]
async fn list_notes_returns_added_note() {
    let state = make_state();
    let payload = serde_json::json!({
        "kind": "note",
        "title": "First note",
        "body": "Some context.",
        "embedding": [1.0_f32, 0.0, 0.0, 0.0],
    });
    send(
        state.clone(),
        "POST",
        "/v1/projects/proj/memory",
        json_body(&payload),
        true,
    )
    .await;

    let resp = send(
        state,
        "GET",
        "/v1/projects/proj/memory",
        Body::empty(),
        false,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let notes: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(notes.as_array().unwrap().len(), 1);
    assert_eq!(notes[0]["title"], "First note");
    assert_eq!(notes[0]["status"], "active");
}

// ── get_note ──────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_note_returns_404_for_unknown_id() {
    let state = make_state();
    // Create project first.
    send(
        state.clone(),
        "POST",
        "/v1/projects/p/memory",
        json_body(serde_json::json!({"kind":"note","title":"x","embedding":[0.0_f32,0.0,0.0,0.0]})),
        true,
    )
    .await;

    let resp = send(
        state,
        "GET",
        "/v1/projects/p/memory/9999",
        Body::empty(),
        false,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── archive + supersede ───────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn archive_note_hides_it_from_list() {
    let state = make_state();
    let add =
        serde_json::json!({"kind":"decision","title":"Arch","embedding":[0.0_f32,0.0,0.0,0.0]});
    let resp = send(
        state.clone(),
        "POST",
        "/v1/projects/q/memory",
        json_body(&add),
        true,
    )
    .await;
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = created["id"].as_i64().unwrap();

    let archive_resp = send(
        state.clone(),
        "POST",
        &format!("/v1/projects/q/memory/{id}/archive"),
        Body::empty(),
        false,
    )
    .await;
    assert_eq!(archive_resp.status(), StatusCode::OK);

    // Default list excludes archived.
    let list_resp = send(state, "GET", "/v1/projects/q/memory", Body::empty(), false).await;
    let bytes = axum::body::to_bytes(list_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let notes: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(notes.as_array().unwrap().is_empty());
}

// ── delete ────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn delete_note_removes_it() {
    let state = make_state();
    let add = serde_json::json!({"kind":"note","title":"Gone","embedding":[0.0_f32,0.0,0.0,0.0]});
    let resp = send(
        state.clone(),
        "POST",
        "/v1/projects/r/memory",
        json_body(&add),
        true,
    )
    .await;
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = created["id"].as_i64().unwrap();

    let del = send(
        state.clone(),
        "DELETE",
        &format!("/v1/projects/r/memory/{id}"),
        Body::empty(),
        false,
    )
    .await;
    assert_eq!(del.status(), StatusCode::OK);

    let get = send(
        state,
        "GET",
        &format!("/v1/projects/r/memory/{id}"),
        Body::empty(),
        false,
    )
    .await;
    assert_eq!(get.status(), StatusCode::NOT_FOUND);
}

// ── auth middleware ───────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn protected_endpoint_rejects_missing_token() {
    let db = common::open_test_server_db(4);
    let state = AppState {
        db: Arc::new(tokio::sync::Mutex::new(db)),
        api_key: Some("secret".into()),
    };
    let resp = send(state, "GET", "/v1/projects", Body::empty(), false).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[serial]
async fn protected_endpoint_accepts_correct_token() {
    let db = common::open_test_server_db(4);
    let state = AppState {
        db: Arc::new(tokio::sync::Mutex::new(db)),
        api_key: Some("secret".into()),
    };
    let req = Request::builder()
        .method("GET")
        .uri("/v1/projects")
        .header("Authorization", "Bearer secret")
        .body(Body::empty())
        .unwrap();
    let resp = router(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ── search ────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn search_returns_closest_note() {
    let state = make_state();

    // Add two notes with distinct embeddings.
    for (title, emb) in [
        ("alpha", [1.0_f32, 0.0, 0.0, 0.0]),
        ("beta", [0.0_f32, 1.0, 0.0, 0.0]),
    ] {
        send(
            state.clone(),
            "POST",
            "/v1/projects/s/memory",
            json_body(serde_json::json!({"kind":"note","title":title,"embedding":emb})),
            true,
        )
        .await;
    }

    // Query near alpha.
    let search_payload = serde_json::json!({
        "embedding": [1.0_f32, 0.0, 0.0, 0.0],
        "limit": 2,
    });
    let resp = send(
        state,
        "POST",
        "/v1/projects/s/memory/search",
        json_body(&search_payload),
        true,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let notes: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(!notes.as_array().unwrap().is_empty());
    assert_eq!(notes[0]["title"], "alpha");
}

// ── stats ─────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn project_stats_returns_correct_counts() {
    let state = make_state();
    send(
        state.clone(),
        "POST",
        "/v1/projects/t/memory",
        json_body(serde_json::json!({"kind":"note","title":"a","embedding":[0.0_f32,0.0,0.0,0.0]})),
        true,
    )
    .await;

    let resp = send(state, "GET", "/v1/projects/t/stats", Body::empty(), false).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stats["count"], 1);
    assert_eq!(stats["total"], 1);
    assert_eq!(stats["embedding_dim"], 4);
}
