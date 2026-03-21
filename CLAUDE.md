# CLAUDE.md — codeanalysis

Developer guide for AI agents (and humans) working on this codebase.

---

## What This Project Is

`codeanalysis` (`ca`) is a Rust CLI that indexes a source tree using
tree-sitter AST chunking, embeds every chunk with EmbeddingGemma, stores
vectors in SQLite, and answers natural language questions via a local RAG
pipeline backed by Gemma 3n.

Target platform: macOS / Apple Silicon (initially). The inference backend is
modular so other platforms can be added without touching core logic.

---

## Module Map

```
src/
  main.rs          — entry point: parse CLI, dispatch to commands
  cli.rs           — clap structs (Cli, Command, *Args)
  cli/commands.rs  — async handler for each subcommand
  config.rs        — Config struct; load from ~/.config/codeanalysis/config.toml
  error.rs         — domain error types (IndexError, EmbeddingError, SearchError)
  backends.rs      — re-exports ActiveEmbedder / ActiveLlm based on feature flags

  embeddings/
    mod.rs         — EmbeddingBackend trait + EMBEDDING_DIM constant
    candle.rs      — CandleEmbedder (EmbeddingGemma, Metal GPU) [backend-metal]

  llm/
    mod.rs         — LlmBackend trait + Token type
    candle.rs      — CandleLlm (Gemma 3n, Metal GPU) [backend-metal]

  indexer/
    mod.rs         — re-exports Chunk, ChunkKind, SourceParser
    chunker.rs     — Chunk / ChunkKind structs; sliding_window fallback
    parser.rs      — SourceParser (tree-sitter); detect_language; SUPPORTED_LANGUAGES

  storage/
    mod.rs         — re-exports Database
    db.rs          — Database struct; open/migrate; typed CRUD methods

  search/
    mod.rs         — SearchResult struct
    rag.rs         — RagPipeline<E,L>: search + ask methods

migrations/
  001_initial.sql  — files, chunks, embeddings (sqlite-vec virtual table)
```

---

## Inference Backend Design

**Rule: nothing outside `src/embeddings/`, `src/llm/`, and `src/backends.rs`
should import a concrete backend type.**

Every backend sits behind a trait:
- `EmbeddingBackend` (`src/embeddings/mod.rs`)
- `LlmBackend` (`src/llm/mod.rs`)

Each implementation is in its own module gated by a feature flag:

| Feature flag     | Module                   | Hardware      |
|------------------|--------------------------|---------------|
| `backend-metal`  | `embeddings/candle.rs`   | Apple GPU (Metal) |
| `backend-metal`  | `llm/candle.rs`          | Apple GPU (Metal) |
| _(future)_       | `embeddings/coreml.rs`   | Apple Neural Engine |
| _(future)_       | `embeddings/cpu.rs`      | Any CPU       |

`src/backends.rs` re-exports `ActiveEmbedder` and `ActiveLlm` for the enabled
feature. Command handlers import from `backends`, never from concrete modules.

To add a new backend:
1. Add a feature flag in `Cargo.toml`
2. Implement both traits in new submodule files
3. Add a `#[cfg(feature = "...")]` re-export in `backends.rs`

---

## Development Phases

| Phase | Status | Scope |
|-------|--------|-------|
| 1 | done | Scaffolding, CLI skeleton, SQLite schema, git |
| 2 | next | Tree-sitter AST walk, per-language chunk extraction |
| 3 | — | EmbeddingGemma via candle + Metal; `index` command end-to-end |
| 4 | — | sqlite-vec KNN query; `search` command end-to-end |
| 5 | — | Gemma 3n inference; `ask` command with streamed output |
| 6 | — | Incremental indexing, config file polish, streaming UX |

---

## Key Design Decisions

### Chunking strategy
Tree-sitter extracts **named semantic nodes** (functions, structs, impls, etc.)
rather than naive line splits. This keeps each chunk semantically coherent
and improves retrieval quality. Sliding-window chunking is the fallback for
unsupported file types.

### Embedding input format
Follows the EmbeddingGemma cookbook convention:
```
Represent this code: <optional docstring>\n<source>
```
See `Chunk::embedding_text()` in `src/indexer/chunker.rs`.

### SQLite + sqlite-vec
No separate vector DB process. The sqlite-vec extension adds a virtual table
(`embeddings USING vec0`) that supports KNN queries directly in SQL.
The extension must be loaded before the migrations run (see `db.rs`).

### Incremental indexing
Each file is hashed with blake3. On re-index, files whose hash hasn't
changed are skipped. Changed files: delete old chunks + embeddings, reparse.

### RAG prompt format
Uses the Gemma chat template (`<start_of_turn>user ... <end_of_turn>`).
Update `search/rag.rs::build_prompt` if the model changes.

---

## Common Commands

```bash
# Build (Metal backend, default)
cargo build

# Build without Metal (e.g. on CI)
cargo build --no-default-features

# Check all features compile
cargo check --all-features

# Run the CLI
cargo run -- index ./some/project
cargo run -- search "how does authentication work"
cargo run -- ask "explain the error handling strategy"
cargo run -- status
cargo run -- languages

# Verbose logging
RUST_LOG=debug cargo run -- index .
```

---

## Dependency Notes

- Tree-sitter language crate versions must be compatible with `tree-sitter`
  core. If you bump the core, check all `tree-sitter-*` crates too.
- `candle-core/metal` requires Xcode Command Line Tools and a Metal-capable
  Apple device.
- `sqlite-vec` is loaded at runtime via `rusqlite`'s extension loading API
  (see `storage/db.rs`). The extension binary is bundled by the crate.
- `tokenizers` uses the `onig` feature to avoid requiring a system regex
  library on non-Linux targets.
