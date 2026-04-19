//! Component tests for `spelunk plumbing embed`.
//!
//! Tests use a `wiremock::MockServer` that responds to `POST /v1/embeddings`
//! with a fixed 768-dimensional zero vector, so no real embedding server is needed.

mod plumbing_helpers;
use plumbing_helpers::parse_ndjson;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a config.toml that points `api_base_url` at the given mock server URI.
fn write_config(dir: &TempDir, mock_uri: &str) -> std::path::PathBuf {
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "api_base_url = \"{mock_uri}\"\n\
             embedding_model = \"test-model\"\n"
        ),
    )
    .unwrap();
    config
}

/// Build a valid OpenAI-compatible embeddings response JSON with one embedding
/// of `dims` dimensions (all zeros).
fn embedding_response(dims: usize) -> serde_json::Value {
    json!({
        "object": "list",
        "data": [
            {
                "object": "embedding",
                "index": 0,
                "embedding": vec![0.0f32; dims]
            }
        ],
        "model": "test-model",
        "usage": { "prompt_tokens": 5, "total_tokens": 5 }
    })
}

// ── exit 0: no stdin piped (empty pipe) ──────────────────────────────────────

#[test]
fn embed_exits_0_with_empty_piped_stdin() {
    let tmp = TempDir::new().unwrap();
    // Config pointing at a port that won't be called (no input lines).
    let config = tmp.path().join("config.toml");
    std::fs::write(
        &config,
        "api_base_url = \"http://127.0.0.1:19998\"\nembedding_model = \"test-model\"\n",
    )
    .unwrap();

    // Pipe empty stdin — command should succeed (no lines to embed).
    Command::cargo_bin("spelunk")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .arg("plumbing")
        .arg("embed")
        .write_stdin("")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

// ── happy path: single line → one JSON embedding ──────────────────────────────

#[tokio::test]
async fn embed_document_mode_produces_ndjson_vector() {
    const DIMS: usize = 768;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response(DIMS)))
        .mount(&mock)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = write_config(&tmp, &mock.uri());

    let output = Command::cargo_bin("spelunk")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .arg("plumbing")
        .arg("embed")
        .write_stdin("fn greet(name: &str) -> String { format!(\"Hello, {}!\", name) }\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let rows = parse_ndjson(&output);
    assert_eq!(rows.len(), 1, "one stdin line → one embedding");

    let row = &rows[0];
    assert!(row.get("model").is_some(), "missing 'model'");
    assert!(row.get("dimensions").is_some(), "missing 'dimensions'");
    assert!(row.get("vector").is_some(), "missing 'vector'");

    let dims = row["dimensions"].as_u64().unwrap_or(0);
    assert!(dims > 0, "dimensions should be positive");

    let vec_len = row["vector"].as_array().map(|a| a.len()).unwrap_or(0);
    assert_eq!(
        vec_len, dims as usize,
        "vector length must match dimensions"
    );
}

#[tokio::test]
async fn embed_query_mode_produces_ndjson_vector() {
    const DIMS: usize = 768;
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response(DIMS)))
        .mount(&mock)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = write_config(&tmp, &mock.uri());

    let output = Command::cargo_bin("spelunk")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .arg("plumbing")
        .arg("embed")
        .arg("--query")
        .write_stdin("how does greet work?\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let rows = parse_ndjson(&output);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get("vector").is_some(), "missing 'vector'");
}

#[tokio::test]
async fn embed_multiple_lines_produce_multiple_vectors() {
    const DIMS: usize = 768;
    let mock = MockServer::start().await;
    // The embed command embeds one line at a time, so we expect 3 POST calls.
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(embedding_response(DIMS)))
        .mount(&mock)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = write_config(&tmp, &mock.uri());

    let output = Command::cargo_bin("spelunk")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .arg("plumbing")
        .arg("embed")
        .write_stdin("first line\nsecond line\nthird line\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let rows = parse_ndjson(&output);
    assert_eq!(rows.len(), 3, "three stdin lines → three embeddings");
}

// ── error path: bad API URL ───────────────────────────────────────────────────

#[test]
fn embed_exits_nonzero_when_server_unreachable() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("config.toml");
    std::fs::write(
        &config,
        "api_base_url = \"http://127.0.0.1:19999\"\nembedding_model = \"test-model\"\n",
    )
    .unwrap();

    Command::cargo_bin("spelunk")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .arg("plumbing")
        .arg("embed")
        .write_stdin("some text\n")
        .assert()
        .failure();
}
