# Testing Strategy

## Current state

58 tests across 9 test files. The suite covers unit logic, SQLite integration,
HTTP mock tests, server handler tests, and CLI end-to-end tests.

```
tests/
  common/               — shared helpers (TempDb, sqlite-vec registration)
  unit_chunker.rs       — sliding-window chunker logic
  unit_embeddings.rs    — vec_to_blob / blob_to_vec roundtrips; ChunkKind parsing
  unit_graph.rs         — EdgeKind parsing and display
  integration_db.rs     — Database CRUD, KNN search, graph edges (real SQLite)
  integration_server.rs — axum server handlers (in-process, no port binding)
  mock_openai_compat.rs — OpenAiCompatEmbedder against a wiremock server
  mock_lmstudio.rs      — legacy stub (kept for reference; superseded by mock_openai_compat.rs)
  e2e_cli.rs            — CLI binary smoke tests via assert_cmd
```

Plus `#[cfg(test)]` blocks in:
- `src/utils.rs` — ANSI stripping
- `src/indexer/secrets.rs` — credential pattern detection
- `src/search/tokens.rs` — token count estimation

---

## Test categories

### Unit tests (no I/O, no mocks)

Pure-logic functions. Cheapest to write and maintain.

| File | What is tested |
|------|---------------|
| `tests/unit_chunker.rs` | `sliding_window`: empty source, single chunk, overlap boundaries, verbatim content |
| `tests/unit_embeddings.rs` | `vec_to_blob` / `blob_to_vec` roundtrip; empty vec; multi-value; blob length |
| `tests/unit_graph.rs` | `EdgeKind` display and parse; unknown kind falls back to `Imports` |
| `src/utils.rs` | `strip_ansi`: clean strings, colour codes, OSC sequences, C0 controls, newline/tab preservation |
| `src/indexer/secrets.rs` | AWS key, GitHub PAT, PEM header detection; clean code and placeholders not flagged |
| `src/search/tokens.rs` | `estimate_tokens`: empty string returns 1; chars/4 heuristic |

### Integration tests (real SQLite, no external HTTP)

Use a temp-file database with the sqlite-vec extension registered via
`tests/common/mod.rs`. Run serially where needed (`#[serial]` from `serial_test`).

| File | What is tested |
|------|---------------|
| `tests/integration_db.rs` | `upsert_file` stable IDs and hash round-trips; `insert_chunk` / `delete_chunks_for_file`; `search_similar` KNN ordering and limit; `replace_edges` stale-edge removal |
| `tests/integration_server.rs` | `GET /v1/health`; `POST /v1/projects/{id}/memory` creates note; `GET` lists; `POST .../search` returns KNN; archive; delete; project stats; auth middleware (valid token, missing token) |

### HTTP mock tests (wiremock)

External HTTP is mocked with `wiremock`. No real inference server required.

| File | What is tested |
|------|---------------|
| `tests/mock_openai_compat.rs` | Successful embed (EOS token appended, correct request shape); empty data array error; 500 error handling; multiple vectors returned |

### CLI end-to-end tests (assert_cmd)

Invoke the compiled `spelunk` binary as a subprocess. These run in CI on every
push and do not require an inference server.

| File | What is tested |
|------|---------------|
| `tests/e2e_cli.rs` | `--version`; `--help`; invalid command error; `languages` output; `status` on empty project; `index` + `status` round-trip against a fixture directory |

---

## sqlite-vec in tests

`sqlite3_auto_extension` is process-global and must only be registered once.
A `OnceLock` helper in `tests/common/mod.rs` handles this:

```rust
pub fn open_test_db() -> Database {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        sqlite_vec::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
    let path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    Database::open(&path).unwrap()
}
```

Tests that open a database call `common::open_test_db()` and are annotated
`#[serial]` to avoid races on the global extension state.

---

## Running the tests

```bash
# All tests
cargo test

# A specific test file
cargo test --test integration_db

# With output (useful for debugging)
cargo test -- --nocapture

# E2E tests require the binary to be built first
cargo build && cargo test --test e2e_cli
```

---

## What is intentionally not tested

| Area | Reason |
|------|--------|
| Interactive `$EDITOR` (memory add without --body) | Requires a TTY; test manually |
| Real inference server output quality | Non-deterministic; use E2E heuristics at best |
| sqlite-vec KNN ranking precision | Depends on embedding geometry; covered by integration smoke tests |
| Concurrent SQLite writes under load | sqlite WAL handles this; benchmark separately if needed |
| PDF text extraction accuracy | Depends on PDF structure; smoke-test with a known fixture |

---

## Planned additions

| Area | Issue |
|------|-------|
| Property-based tests for chunker, PageRank, and token-budget packing | #24 |
| Fuzzing the parser and secret scanner | #23 |
