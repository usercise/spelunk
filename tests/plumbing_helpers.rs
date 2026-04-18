//! Shared helpers for plumbing command component tests.
//!
//! Every test that needs an indexed project DB should call
//! `index_fixture_project()`.  Tests that need no index still share helpers
//! for constructing `Command` instances.
#![allow(dead_code)]

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Path (relative to the workspace root) of the synthetic fixture project.
pub const FIXTURE_DIR: &str = "tests/fixtures/simple-project";

/// Build a `spelunk plumbing --db <db>` Command pre-configured to use the
/// given DB and config file.  Callers add the specific plumbing subcommand
/// args (e.g. `cmd.arg("cat-chunks").arg("src/lib.rs")`).
///
/// Note: `--db` is a flag on the `plumbing` subcommand, not the top-level
/// command.  The correct invocation shape is:
///   spelunk --config <cfg> plumbing --db <db> <subcommand> [args]
pub fn spelunk_cmd(db_path: &Path, config_path: &Path) -> Command {
    let mut cmd = Command::cargo_bin("spelunk").unwrap();
    cmd.arg("--config")
        .arg(config_path)
        .arg("plumbing")
        .arg("--db")
        .arg(db_path);
    cmd
}

/// Write a minimal config file pointing at `db_path` and an optional
/// API base URL.  Returns the config file path.
pub fn write_config(dir: &Path, db_path: &Path, api_base: &str) -> PathBuf {
    let cfg = format!(
        "db_path = {:?}\napi_base_url = {:?}\nembedding_model = \"test-model\"\nllm_model = \"test-chat\"\n",
        db_path, api_base
    );
    let config_path = dir.join("config.toml");
    std::fs::write(&config_path, cfg).expect("write config");
    config_path
}

/// Run `spelunk index <fixture_dir>` backed by a mock embedding server.
///
/// Returns `(TempDir, db_path, config_path)`.  The `TempDir` must be kept
/// alive for the duration of the test.
pub fn index_fixture_project() -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().expect("create temp dir");
    let db_path = tmp.path().join("spelunk.db");

    // Spin up a wiremock server that echoes back a 768-dim embedding.
    // The server must stay alive past the index call, so we keep it in scope.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _mock_server = rt.block_on(async {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "embedding": vec![0.1f32; 768], "index": 0 }],
                "model": "test-model",
                "object": "list",
                "usage": { "prompt_tokens": 5, "total_tokens": 5 }
            })))
            .mount(&server)
            .await;
        server
    });

    let mock_url = _mock_server.uri();
    let config_path = write_config(tmp.path(), &db_path, &mock_url);

    // Resolve the fixture directory relative to the workspace root.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR);

    // Pass `--db` explicitly so the index is written to our temp DB path,
    // not to `<fixture>/.spelunk/index.db` (the default project-local location).
    Command::cargo_bin("spelunk")
        .unwrap()
        .arg("--config")
        .arg(&config_path)
        .arg("index")
        .arg("--db")
        .arg(&db_path)
        .arg(&fixture)
        .assert()
        .success();

    (tmp, db_path, config_path)
}

/// Parse every line of `stdout` as JSON; return the parsed values.
/// Panics if any line is not valid JSON.
pub fn parse_ndjson(stdout: &[u8]) -> Vec<serde_json::Value> {
    let text = std::str::from_utf8(stdout).expect("stdout is utf-8");
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("invalid JSON line {l:?}: {e}")))
        .collect()
}
