# Architecture

This document describes spelunk's system design for contributors and anyone integrating with the codebase.

## Overview

spelunk is a Rust CLI that:

1. **Indexes** source trees using tree-sitter AST parsing
2. **Embeds** each code chunk via an external embedding model
3. **Stores** vectors, chunks, and graph edges in SQLite
4. **Serves** semantic search, graph queries, and memory retrieval via CLI

```
┌─────────────┐     ┌──────────────┐     ┌────────────────┐
│  Source tree │────>│   Indexer    │────>│   SQLite DB    │
│  (.rs, .py,  │     │  tree-sitter │     │  chunks        │
│   .ts, ...)  │     │  + chunker   │     │  embeddings    │
└─────────────┘     └──────┬───────┘     │  graph_edges   │
                           │             │  notes         │
                    ┌──────▼───────┐     └───────┬────────┘
                    │  Embedding   │             │
                    │  Backend     │     ┌───────▼────────┐
                    │  (LM Studio, │     │   Search /     │
                    │   Ollama,    │     │   Graph /      │
                    │   any OAI)   │     │   Memory       │
                    └──────────────┘     └��──────┬────────┘
                                                │
                                        ┌───────▼───────��┐
                                        │   CLI output   │
                                        │  (text / JSON) │
                                        └────────────��───┘
```

## Module structure

```
src/
  main.rs              Entry point: CLI parse, sqlite-vec init, command dispatch
  lib.rs               Library root: re-exports for tests and server binary

  cli/
    mod.rs             Clap structs (Cli, Command, *Args)
    cmd/               One file per subcommand (index.rs, search.rs, etc.)

  config.rs            Config struct, loads from ~/.config/spelunk/config.toml

  backends.rs          Re-exports ActiveEmbedder / ActiveLlm (feature-gated)

  embeddings/
    mod.rs             EmbeddingBackend trait, vec_to_blob/blob_to_vec helpers
    lmstudio.rs        LmStudioEmbedder: POST /v1/embeddings

  llm/
    mod.rs             LlmBackend trait, Message struct
    lmstudio.rs        LmStudioLlm: POST /v1/chat/completions (SSE streaming)

  indexer/
    mod.rs             Re-exports
    chunker.rs         Chunk / ChunkKind structs, embedding_text(), sliding_window
    parser.rs          SourceParser (tree-sitter), detect_language, SUPPORTED_LANGUAGES
    graph.rs           EdgeExtractor: import/call/extends edges via tree-sitter
    secrets.rs         Regex-based credential scanner, drops matching chunks

  storage/
    mod.rs             Re-exports
    db.rs              Database struct: open/migrate, CRUD, KNN search

  search/
    mod.rs             SearchResult struct
    rag.rs             RagPipeline: search + ask methods

  registry.rs          Global project registry (~/.config/spelunk/registry.db)

migrations/            SQL migration files applied in order at DB open
```

## Key design decisions

Architectural decisions are recorded in [docs/adr/](adr/). Key ones:

### Chunking: tree-sitter AST nodes, not line splits

Tree-sitter parses source code into an AST and spelunk extracts named semantic nodes (functions, structs, classes, methods, traits, impls) as individual chunks. This means each chunk is a meaningful unit of code with a name, type, and scope — not an arbitrary 100-line window.

Fallback: sliding window (120 lines, 15-line overlap) for unsupported languages. Markdown uses heading-based chunking.

### Storage: SQLite + sqlite-vec, nothing else

All data lives in a single SQLite file per project. The sqlite-vec extension adds a `vec0` virtual table for KNN vector search. No separate vector database, no separate search engine.

This is a deliberate constraint — see [ADR-001](adr/001-scope-boundaries.md). SQLite is zero-configuration, single-file, and sufficient for the scale spelunk targets.

### Incremental indexing via blake3

Each file is hashed with blake3. On re-index, unchanged files are skipped entirely. Changed files get their old chunks and embeddings deleted, then re-parsed and re-embedded.

### Embedding format

Chunks are embedded using EmbeddingGemma's recommended format:
```
title: {name} | text: {content}
```

Queries use: `task: code retrieval | query: {q}`

See `Chunk::embedding_text()` in `src/indexer/chunker.rs`.

### Backend abstraction

The `EmbeddingBackend` and `LlmBackend` traits are the only interface between spelunk and inference. The concrete implementation (LM Studio) is gated behind a feature flag and re-exported in `src/backends.rs`.

To add a new backend: implement the trait, add a feature flag, gate the re-export. Nothing outside `src/embeddings/`, `src/llm/`, and `src/backends.rs` imports a concrete backend.

### Secret scanning

`src/indexer/secrets.rs` runs regex patterns against every chunk before storage. Chunks matching known credential patterns (AWS keys, PEM headers, GitHub PATs) are silently dropped and a warning is logged.

### Multi-project registry

`~/.config/spelunk/registry.db` tracks all indexed projects. `spelunk link` connects projects so that `spelunk search` queries multiple databases and merges results by vector distance.

## Data flow: index

```
files on disk
  → SourceParser (tree-sitter AST → Chunk[])
  → SecretScanner (drop credential chunks)
  → EmbeddingBackend.embed(batch of chunk texts)
  → Database.store(chunks + embeddings)
  → EdgeExtractor (AST → graph_edges)
  → Database.store(edges)
```

## Data flow: search

```
query string
  → EmbeddingBackend.embed(formatted query)
  → Database.search_similar(query_vec, limit)  // sqlite-vec KNN
  → [optional] Database.graph_neighbor_chunks() // 1-hop expansion
  → [optional] query linked project DBs via registry
  → merge + deduplicate by (file_path, start_line, end_line)
  → return Vec<SearchResult>
```

## Adding a new language

1. Add the `tree-sitter-{lang}` crate to `Cargo.toml`
2. Register the language in `src/indexer/parser.rs` (`detect_language` + `SUPPORTED_LANGUAGES`)
3. Add extraction patterns in `src/indexer/graph/edges.rs` for graph edge support
4. Add tests
