use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;
use std::fs;

#[test]
fn test_help_output() {
    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: spelunk [OPTIONS] <COMMAND>"))
        .stdout(predicate::str::contains("Commands:"));
}

#[test]
fn test_version_output() {
    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("spelunk 0.1.0"));
}

#[test]
fn test_invalid_command() {
    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.arg("nonexistent-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error: unrecognized subcommand 'nonexistent-command'"));
}

#[test]
fn test_languages_output() {
    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.arg("languages")
        .assert()
        .success()
        .stdout(predicate::str::contains("Supported languages:"))
        .stdout(predicate::str::contains("rust"))
        .stdout(predicate::str::contains("python"))
        .stdout(predicate::str::contains("javascript"));
}

#[test]
fn test_status_empty_project() {
    let temp = tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    fs::write(&config_path, "llm_model = \"test-model\"\n").unwrap();

    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.current_dir(temp.path())
        .arg("--config")
        .arg(&config_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No index found for the current directory"));
}

use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

#[tokio::test]
async fn test_index_and_status() {
    let mock_server = MockServer::start().await;
    
    // Mock for index (1 file -> 1 chunk -> 1 request)
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({
                "data": [{ "embedding": vec![0.1; 768], "index": 0 }],
                "model": "test-model",
                "object": "list",
                "usage": { "prompt_tokens": 10, "total_tokens": 10 }
            })))
        .mount(&mock_server)
        .await;

    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("my-project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join("main.rs"), "fn main() { println!(\"hello\"); }").unwrap();

    let config_path = temp.path().join("config.toml");
    let db_path = temp.path().join("test_index.db");
    
    fs::write(&config_path, format!(
        "db_path = {:?}\nlmstudio_base_url = {:?}\nembedding_model = \"test-model\"\nllm_model = \"test-chat-model\"\n",
        db_path, mock_server.uri()
    )).unwrap();

    // 1. Index the project
    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.arg("--config")
        .arg(&config_path)
        .arg("index")
        .arg(&project_dir)
        .assert()
        .success();

    // 2. Check status
    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.current_dir(&project_dir)
        .arg("--config")
        .arg(&config_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Project:"))
        .stdout(predicate::str::contains("my-project"))
        .stdout(predicate::str::contains("Files:      1"))
        .stdout(predicate::str::contains("Chunks:     1"));

    // 3. Search for the function
    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.current_dir(&project_dir)
        .arg("--config")
        .arg(&config_path)
        .arg("search")
        .arg("hello")
        .assert()
        .success()
        .stdout(predicate::str::contains("main.rs"))
        .stdout(predicate::str::contains("fn main()"));

    // 4. Ask a question
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string("data: {\"choices\": [{\"delta\": {\"content\": \"The main function \"}}]}\n\ndata: {\"choices\": [{\"delta\": {\"content\": \"prints hello.\"}}]}\n\ndata: [DONE]\n\n"))
        .mount(&mock_server)
        .await;

    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.current_dir(&project_dir)
        .arg("--config")
        .arg(&config_path)
        .arg("ask")
        .arg("what does the main function do?")
        .assert()
        .success()
        .stdout(predicate::str::contains("The main function prints hello."));
}
