# Testing Strategy

## Current state

Coverage is minimal: 13 tests across 2 files (`src/utils.rs` and
`src/indexer/secrets.rs`). Everything else is untested. The goal of this
document is to define a realistic path to solid coverage without trying to
test everything at once.

---

## Test categories

### Unit tests (no I/O, no mocks)

Pure-logic functions that take inputs and return outputs. These are the
cheapest tests to write and maintain.

| Area | What to test |
|------|-------------|
| `src/indexer/chunker.rs` | `sliding_window`: empty file, single line, exact window size, overlap boundaries |
| `src/indexer/parser.rs` | Per-language chunk extraction (Rust, Python, JS, Markdown at minimum); fallback to sliding-window on parse failure |
| `src/indexer/graph.rs` | Edge extraction per language (imports, calls, extends); deduplication |
| `src/embeddings/mod.rs` | `vec_to_blob` / `blob_to_vec` roundtrip; empty vec; float precision |
| `src/storage/memory.rs` | `split_csv`: empty, single, whitespace, trailing commas |
| `src/utils.rs` | ✅ Done |
| `src/indexer/secrets.rs` | ✅ Done |

### Integration tests (real SQLite, no external HTTP)

These use a real in-memory or temp-file database. `sqlite-vec` must be
registered before these run (same as `main.rs` — add a `#[ctor]` or call it
in a `once_cell`).

| Area | What to test |
|------|-------------|
| `src/config.rs` | Default config; global file override; project-level `.spelunk/config.toml` walk-up; env var overrides (`SPELUNK_SERVER_URL`, `SPELUNK_PROJECT_ID`, `SPELUNK_SERVER_KEY`); validation error when URL set without project_id |
| `src/storage/db.rs` | `upsert_file` + `file_hash` (change detection); `insert_chunk` / `delete_chunks_for_file`; `search_similar` with a real embedding blob; `replace_edges` + `edges_for_symbol` |
| `src/storage/memory.rs` | `add_note` / `list` / `get` / `count`; `archive`; `supersede`; `list --include_archived`; KNN `search` returns closest entry |
| `src/storage/backend.rs` | `LocalMemoryBackend` wraps `MemoryStore` correctly under async concurrent access |
| `src/server/db.rs` | `upsert_project` auto-creates; dimension mismatch returns error; `add_note` / `list_notes` / `search_notes`; `archive` / `supersede` lifecycle |
| `src/registry.rs` | Register + find by root; walk-up path discovery; `add_dep` / `remove_dep` / `get_deps`; autoclean removes stale entries |

### HTTP mock tests (wiremock)

External HTTP is mocked with `wiremock`. No real LM Studio or server needed.

| Area | What to test |
|------|-------------|
| `src/embeddings/openai_compat.rs` | Successful embed (verify `<eos>` appended, correct request shape); batch handling; 500 / timeout error handling |
| `src/llm/openai_compat.rs` | SSE stream parsing; complete message assembly; error mid-stream |
| `src/storage/remote.rs` | `add` / `search` / `list` / `get` / `archive` / `supersede`; auth header sent; 404 on missing note |
| `src/server/mod.rs` | Auth middleware: valid token passes, invalid token → 401, no key configured → passes |

### Server handler tests (axum test harness)

Use `axum::serve` with `tokio_test::io::Builder` or `tower::ServiceExt::oneshot`
to drive the router in-process without binding a port.

| Area | What to test |
|------|-------------|
| `src/server/handlers.rs` | `GET /v1/health`; `POST /v1/projects/{id}/memory` creates note + enforces dimension; `GET /v1/projects/{id}/memory` lists; `POST /v1/projects/{id}/memory/search` returns KNN; `POST …/archive`; `POST …/supersede`; `DELETE`; `GET /v1/projects/{id}/stats` |

### End-to-end tests (optional, not in CI by default)

Require an OpenAI-compatible server running at `http://127.0.0.1:1234`. Gate behind a
`#[cfg(feature = "e2e")]` feature flag or a `RUN_E2E=1` env var guard so they
never run in CI unless explicitly enabled.

- Index a small fixture directory, search for a known symbol, verify it appears
  in results.
- `spelunk ask` over that index produces a non-empty answer.
- Two clients push to `spelunk-server`, both see each other's entries.

---

## Required additions to `Cargo.toml`

```toml
[dev-dependencies]
tempfile        = "3"      # temp dirs/files for DB isolation
wiremock        = "0.6"    # mock HTTP server for OpenAI-compatible API + remote memory
tokio-test      = "0.4"    # async test helpers
serial_test     = "3"      # serialize tests that share global state (sqlite-vec extension)
pretty_assertions = "1"    # coloured diffs on assertion failures
```

---

## sqlite-vec in tests

`sqlite3_auto_extension` is global and can only be called once per process.
Two options:

1. Call it in a `std::sync::OnceLock` helper in `tests/common/mod.rs` and
   invoke that helper at the top of every test that opens a database.
2. Add a `#[ctor]` attribute on a registration function (requires the `ctor`
   crate).

Option 1 is simpler and has no extra dependency.

```rust
// tests/common/mod.rs
pub fn register_sqlite_vec() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}
```

---

## Test file layout

```
src/
  utils.rs                    ← #[cfg(test)] block already here
  indexer/
    chunker.rs                ← add #[cfg(test)] block
    parser.rs                 ← add #[cfg(test)] block
    graph.rs                  ← add #[cfg(test)] block
    secrets.rs                ← #[cfg(test)] block already here
  embeddings/
    mod.rs                    ← add #[cfg(test)] block (vec_to_blob roundtrip)
    lmstudio.rs               ← add #[cfg(test)] block (wiremock)
  storage/
    memory.rs                 ← add #[cfg(test)] block
    db.rs                     ← add #[cfg(test)] block
    backend.rs                ← add #[cfg(test)] block
    remote.rs                 ← add #[cfg(test)] block (wiremock)
  server/
    db.rs                     ← add #[cfg(test)] block
    handlers.rs               ← add #[cfg(test)] block (axum oneshot)
    mod.rs                    ← add #[cfg(test)] block (auth middleware)

tests/
  common/
    mod.rs                    ← register_sqlite_vec(), TempDb, mock builders
  config.rs                   ← config layering tests (needs filesystem)
  registry.rs                 ← registry integration tests
```

Prefer `#[cfg(test)]` blocks inside the module file for unit/integration tests
that access private items. Use `tests/` top-level files only for tests that
are purely external to the module (config, registry).

---

## Suggested order of implementation

Start with the cheapest, highest-confidence tests and work outward:

1. **Chunker, parser, graph, vec roundtrip, CSV** — pure logic, zero
   infrastructure. Gets core indexing logic under test quickly.
2. **Config** — catches regressions whenever config fields change. Needs
   `tempfile` and env-var isolation (`std::env::set_var` with a lock or
   `serial_test`).
3. **MemoryStore + Database** — SQLite CRUD. Needs `tempfile` +
   `register_sqlite_vec()`.
4. **Embedding + LLM backends** — needs `wiremock`. Catches HTTP contract
   regressions.
5. **RemoteMemoryBackend** — needs `wiremock`. Can reuse the mock builders
   from step 4.
6. **Server DB + handlers** — in-process axum harness. Fast, no network.
7. **Registry** — needs `tempfile` and a fake filesystem layout.
8. **E2E** — optional, behind feature flag.

---

## What is intentionally not tested

| Area | Reason |
|------|--------|
| Interactive `$EDITOR` (memory add without --body) | Requires a TTY; test manually |
| Real inference server output quality | Model output is non-deterministic; use E2E heuristics at best |
| sqlite-vec KNN ranking precision | Depends on embedding geometry; covered by integration smoke tests |
| CLI binary output / argument parsing | clap handles most of this; add `assert_cmd` tests only if custom validation is added |
| Concurrent SQLite writes under load | sqlite WAL handles this; benchmark separately if a bottleneck |
