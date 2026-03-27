//! Tests for the LM Studio embedding backend using wiremock.
//!
//! These tests spin up a local mock HTTP server so no real LM Studio
//! instance is needed.  They exercise the JSON contract and error handling
//! of `LmStudioEmbedder`.

use spelunk::config::Config;
use spelunk::embeddings::{EmbeddingBackend, lmstudio::LmStudioEmbedder};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Build a minimal Config pointing at the mock server.
fn config_for(base_url: &str) -> Config {
    Config {
        lmstudio_base_url: base_url.to_string(),
        ..Config::default()
    }
}

// ── happy path ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn embed_returns_vectors_from_server() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"embedding": [0.1_f32, 0.2, 0.3]},
                {"embedding": [0.4_f32, 0.5, 0.6]},
            ]
        })))
        .mount(&server)
        .await;

    let cfg = config_for(&server.uri());
    let embedder = LmStudioEmbedder::load(&cfg).await.unwrap();
    let result = embedder.embed(&["hello", "world"]).await.unwrap();

    assert_eq!(result.len(), 2);
    assert_eq!(result[0], vec![0.1, 0.2, 0.3]);
    assert_eq!(result[1], vec![0.4, 0.5, 0.6]);
}

#[tokio::test]
async fn embed_appends_eos_token_to_input() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"embedding": [1.0_f32]}]
        })))
        .mount(&server)
        .await;

    let cfg = config_for(&server.uri());
    let embedder = LmStudioEmbedder::load(&cfg).await.unwrap();
    embedder.embed(&["test"]).await.unwrap();

    // Verify the request body contained the <eos> suffix.
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    let inputs = body["input"].as_array().unwrap();
    assert_eq!(inputs.len(), 1);
    assert!(
        inputs[0].as_str().unwrap().ends_with("<eos>"),
        "expected <eos> suffix, got: {}",
        inputs[0]
    );
}

// ── error handling ────────────────────────────────────────────────────────────

#[tokio::test]
async fn embed_errors_on_server_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let cfg = config_for(&server.uri());
    let embedder = LmStudioEmbedder::load(&cfg).await.unwrap();
    let result = embedder.embed(&["x"]).await;
    assert!(result.is_err(), "expected error on HTTP 500");
}

#[tokio::test]
async fn embed_errors_on_empty_data_array() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[]})))
        .mount(&server)
        .await;

    let cfg = config_for(&server.uri());
    let embedder = LmStudioEmbedder::load(&cfg).await.unwrap();
    let result = embedder.embed(&["x"]).await;
    assert!(result.is_err(), "expected error when data array is empty");
}
