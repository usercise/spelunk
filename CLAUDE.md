# CLAUDE.md — spelunk

Developer guide for AI agents (and humans) working on this codebase.

---

## Agent workflow — use spelunk on this codebase

This project is indexed with spelunk. Use it — don't just use Read/Grep/Glob.

**At the start of every session:**
```bash
spelunk check                                    # verify index is fresh
spelunk memory list --kind decision --limit 10   # review prior decisions
spelunk memory list --kind handoff --limit 3     # pick up where last session left off
spelunk memory list --kind question              # check open questions
```

**Before reading any file, search first:**
```bash
spelunk search "<topic>"          # find relevant chunks by meaning
spelunk ask "<question>"          # get a synthesised answer with citations
spelunk graph <symbol>            # trace callers/callees when needed
```

**Store decisions as you make them** — don't wait until the end:
```bash
spelunk memory add --kind decision --title "..." --body "why, what alternatives, what breaks"
spelunk memory add --kind requirement --title "..." --body "..."   # when user states a constraint
spelunk memory add --kind note --title "..."                       # surprising/non-obvious facts
```

**At the end of every session:**
```bash
spelunk memory add --kind handoff --title "Handoff: <summary>" --body "what's done, what's next, open questions"
spelunk index .                   # re-index after any commits (hook does this, but run manually if no commit)
```

Full reference: `SKILL.md` and `docs/agent-guide.md`.

---

## What This Project Is

`spelunk` (`spelunk`) is a Rust CLI that indexes a source tree using
tree-sitter AST chunking, embeds every chunk via the LM Studio API
(EmbeddingGemma 300M), stores vectors in SQLite, and answers natural language
questions via a RAG pipeline backed by any chat model loaded in LM Studio.

**Requirement**: LM Studio running at `http://127.0.0.1:1234` (configurable)
with an embedding model and a chat model loaded.

---

## Module Map

```
src/
  main.rs          — entry point: parse CLI, dispatch to commands
  cli/
    mod.rs         — clap structs (Cli, Command, *Args)
    commands.rs    — async handler for each subcommand
  config.rs        — Config struct; load from ~/.config/spelunk/config.toml
  backends.rs      — re-exports ActiveEmbedder / ActiveLlm (LM Studio)
  utils.rs         — strip_ansi(): sanitize LLM output before printing

  embeddings/
    mod.rs         — EmbeddingBackend trait, vec_to_blob/blob_to_vec helpers
    lmstudio.rs    — LmStudioEmbedder: calls /v1/embeddings

  llm/
    mod.rs         — LlmBackend trait, Message struct, Token type
    lmstudio.rs    — LmStudioLlm: calls /v1/chat/completions (SSE streaming)

  indexer/
    mod.rs         — re-exports Chunk, ChunkKind, SourceParser
    chunker.rs     — Chunk / ChunkKind structs; sliding_window fallback
    parser.rs      — SourceParser (tree-sitter); detect_language; SUPPORTED_LANGUAGES
    graph.rs       — EdgeExtractor: extract import/call/extends edges via tree-sitter
    secrets.rs     — contains_secret(): regex-based scanner, drops chunks with credentials

  storage/
    mod.rs         — re-exports Database
    db.rs          — Database struct; open/migrate; typed CRUD + KNN search

  search/
    mod.rs         — SearchResult struct
    rag.rs         — RagPipeline<E,L>: search + ask methods

  registry.rs      — global project registry (~/.config/spelunk/registry.db)
                     project auto-discovery, cross-project link/unlink

migrations/
  001_initial.sql  — files, chunks tables
  002_vectors.sql  — embeddings (sqlite-vec virtual table)
  003_graph.sql    — graph_edges table
```

---

## Inference Backend

The only backend is **LM Studio** (`backend-lmstudio`, the default feature).
There are no other feature flags. Both `ActiveEmbedder` and `ActiveLlm` are
unconditional re-exports in `src/backends.rs`.

To add a new backend:
1. Add a feature flag in `Cargo.toml`
2. Implement `EmbeddingBackend` and `LlmBackend` in new submodule files
3. Gate the re-exports in `src/backends.rs` behind the feature flag

Nothing outside `src/embeddings/`, `src/llm/`, and `src/backends.rs` should
import a concrete backend type.

---

## Key Design Decisions

### Chunking strategy
Tree-sitter extracts **named semantic nodes** (functions, structs, impls, etc.)
rather than naive line splits. Sliding-window (120 lines, 15-line overlap) is
the fallback for unsupported file types. Markdown uses ATX heading-based
chunking (each `# Heading` + body = one `ChunkKind::Section`).

### Embedding input format
EmbeddingGemma's recommended document retrieval format:
```
title: {name | "none"} | text: {content}
```
Query-side prefixes are task-specific:
- `spelunk search` → `task: code retrieval | query: {q}`
- `spelunk ask`    → `task: question answering | query: {q}`

See `Chunk::embedding_text()` in `src/indexer/chunker.rs`.

### SQLite + sqlite-vec
No separate vector DB. The sqlite-vec extension adds a `vec0` virtual table
for KNN queries. The extension is registered via `sqlite3_auto_extension`
before any connection is opened (see `main.rs`).

### Incremental indexing
Each file is hashed with blake3. On re-index, unchanged files are skipped.
Changed files: delete old chunks + embeddings, reparse, re-embed.

### Multi-project registry
`~/.config/spelunk/registry.db` tracks all indexed projects and their
dependency links. `spelunk search` and `spelunk ask` automatically query all linked
project DBs and merge results by distance.

### Secret scanning
`src/indexer/secrets.rs` runs before each chunk is stored. Chunks matching
known credential patterns (AWS keys, PEM headers, GitHub PATs, etc.) are
silently dropped and a warning is logged — content is never echoed.

### Prompt structure
The ask prompt uses XML-style delimiters to separate untrusted RAG context
from the user's question, mitigating prompt injection:
```xml
<code_context>
{retrieved chunks}
</code_context>

<question>
{user question}
</question>
```

---

## Supported Languages

Rust, Go, Python, TypeScript, JavaScript, JSX, TSX, Java, C, C++, Ruby,
Swift, Kotlin, JSON, HTML, CSS, HCL, Proto, SQL, Markdown, plain text.

---

## Common Commands

```bash
# Build
cargo build
cargo build --release

# Run the CLI
cargo run -- index ./some/project
cargo run -- search "how does authentication work"
cargo run -- ask "explain the error handling strategy"
cargo run -- ask "what files handle auth" --json
cargo run -- status
cargo run -- status --all
cargo run -- graph <symbol>
cargo run -- chunks src/some/file.rs
cargo run -- languages

# Verbose logging
RUST_LOG=debug cargo run -- index .

# Tests
cargo test

# Security audit (requires cargo-audit)
cargo audit
```

---

## Dependency Notes

- Tree-sitter language crate versions must be compatible with the `tree-sitter`
  core. If you bump the core, check all `tree-sitter-*` crates too.
- `sqlite-vec` is loaded at runtime via `sqlite3_auto_extension` (see
  `main.rs`). The extension binary is bundled by the crate — no system install
  needed.
- `regex` is used only by `src/indexer/secrets.rs`. Patterns are compiled once
  via `OnceLock` at the start of `spelunk index`.
- `ignore` respects `.gitignore`, `.ignore`, and global gitignore rules during
  file traversal. Sensitive file patterns (`.env*`, `*.pem`, etc.) are
  excluded unconditionally via `OverrideBuilder`.
