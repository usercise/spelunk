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
spelunk graph <symbol>            # trace callers/callees when needed
```

spelunk retrieves context — you synthesise the answer.

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
tree-sitter AST chunking, embeds every chunk via any OpenAI-compatible API
(EmbeddingGemma 300M by default), and stores vectors in SQLite for semantic search.

It is a **context retrieval engine** for AI agents like you. You search with
spelunk, then reason over the results yourself.

**Requirement**: Any OpenAI-compatible server (LM Studio, Ollama, vLLM, etc.)
running at `http://127.0.0.1:1234` (configurable via `api_base_url`) with an
**embedding model** loaded. A chat model is optional (enables `memory harvest`
and `plan create`).

---

## Module Map

```
src/
  main.rs          — entry point: parse CLI, dispatch to commands
  lib.rs           — crate root; re-exports public modules
  error.rs         — SpelunkError enum
  config.rs        — Config struct; load from ~/.config/spelunk/config.toml
  backends.rs      — re-exports ActiveEmbedder / ActiveLlm (LM Studio)
  utils.rs         — strip_ansi(), misc helpers
  registry.rs      — global project registry (~/.config/spelunk/registry.db)

  cli/
    mod.rs         — clap structs (Cli, Command, *Args)
    cmd/
      mod.rs       — re-exports one pub fn per subcommand
      ask.rs       — `spelunk ask` handler
      check.rs     — `spelunk check` handler
      explore.rs   — `spelunk explore` handler
      graph.rs     — `spelunk graph` handler
      helpers.rs   — shared output / progress helpers
      history.rs   — `spelunk history` handler
      hooks.rs     — `spelunk hooks` handler
      init.rs      — `spelunk init` handler
      link.rs      — `spelunk link/unlink/autoclean` handlers
      links.rs     — `spelunk links` handler
      misc.rs      — `spelunk chunks` / `spelunk languages` handlers
      plan.rs      — `spelunk plan` handler
      search.rs    — `spelunk search` handler
      snapshot.rs  — `spelunk snapshot` handler
      spec.rs      — `spelunk spec` handler
      status.rs    — `spelunk status` handler
      verify.rs    — `spelunk verify` handler
      ui.rs        — TUI helpers (private)
      index/
        mod.rs         — `spelunk index` entry point
        embed_phase.rs — embedding phase of indexing
        parse_phase.rs — parse/chunk phase of indexing
        summaries.rs   — AI summary generation during index
        worktree.rs    — git worktree handling for index
      memory/
        mod.rs         — `spelunk memory` dispatch
        add.rs         — memory add subcommand
        archive.rs     — memory archive subcommand
        graph_cmd.rs   — memory graph subcommand
        harvest.rs     — memory harvest (LLM extraction)
        list.rs        — memory list subcommand
        push.rs        — memory push subcommand
        search.rs      — memory search subcommand
        show.rs        — memory show subcommand
        supersede.rs   — memory supersede subcommand
        timeline.rs    — memory timeline subcommand
      plumbing/
        mod.rs         — PlumbingArgs/PlumbingCommand; dispatch; exit-2 on error
        cat_chunks.rs  — emit indexed chunks for a file as NDJSON
        embed_cmd.rs   — read stdin lines, emit embedding vectors as NDJSON
        graph_edges.rs — emit code graph edges as NDJSON
        hash_file.rs   — blake3 hash a file; check index currency
        knn.rs         — KNN vector search, NDJSON output
        ls_files.rs    — list indexed files as NDJSON; exit 1 if no results
        parse_file.rs  — parse a file and emit chunks as NDJSON (no DB write)
        read_memory.rs — emit memory entries as NDJSON

  embeddings/
    mod.rs         — EmbeddingBackend trait, vec_to_blob/blob_to_vec helpers
    openai_compat.rs — OpenAiCompatEmbedder: calls /v1/embeddings

  llm/
    mod.rs         — LlmBackend trait, Message struct, Token type
    openai_compat.rs — OpenAiCompatLlm: calls /v1/chat/completions (SSE streaming)

  indexer/
    mod.rs         — re-exports Chunk, ChunkKind, SourceParser
    chunker.rs     — Chunk / ChunkKind structs; sliding_window fallback
    docparser.rs   — document-level parsing helpers
    pagerank.rs    — PageRank over the code graph
    pdf.rs         — PDF text extraction
    secrets.rs     — contains_secret(): regex scanner, drops credential chunks
    summariser.rs  — LLM-based chunk summarisation
    graph/
      mod.rs       — re-exports EdgeExtractor
      edges.rs     — EdgeExtractor: import/call/extends edges via tree-sitter
      builtins.rs  — built-in symbol skip-list
    parser/
      mod.rs       — SourceParser; detect_language; SUPPORTED_LANGUAGES
      text.rs      — plain-text / sliding-window parser
      ts_walker.rs — tree-sitter AST walker

  storage/
    mod.rs         — re-exports Database
    db.rs          — Database struct; open/migrate; connection setup
    files.rs       — file record CRUD (insert, lookup, delete)
    chunks.rs      — chunk CRUD (insert, fetch, delete by file)
    search.rs      — KNN search queries against sqlite-vec
    graph.rs       — graph_edges CRUD
    snapshots.rs   — snapshot save/restore
    specs.rs       — spec record CRUD
    stats.rs       — aggregate statistics queries
    memory.rs      — NoteStore: memory entries CRUD + list_filtered
    backend.rs     — StorageBackend trait (local vs remote)
    remote.rs      — remote storage backend (HTTP)

  search/
    mod.rs         — SearchResult struct
    rag.rs         — RagPipeline<E,L>: search + ask (dead code, kept for future)
    explore.rs     — interactive exploration pipeline
    tokens.rs      — token-budget helpers
    tools.rs       — tool-call helpers for LLM search

migrations/
  001_initial.sql  — files, chunks tables
  002_vectors.sql  — embeddings (sqlite-vec virtual table)
  003_graph.sql    — graph_edges table
```

---

## Inference Backend

The only backend is the **OpenAI-compatible API** client (`backend-lmstudio`
feature flag, always enabled). Both `ActiveEmbedder` and `ActiveLlm` are
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
Query-side prefix: `task: code retrieval | query: {q}`

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
dependency links. `spelunk search` automatically queries all linked project DBs
and merges results by distance.

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
