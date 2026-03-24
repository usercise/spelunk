# spelunk — local code intelligence

`spelunk` is a local-first CLI that makes your codebase searchable and queryable in natural language, without sending code to the cloud.

It indexes your source tree with [tree-sitter](https://tree-sitter.github.io/) AST chunking, stores vector embeddings in SQLite, and answers questions via a RAG pipeline — all through a locally-running LLM in [LM Studio](https://lmstudio.ai/).

## Quick start

```bash
# 1. Start LM Studio with an embedding model + a chat model loaded

# 2. Index a project
spelunk index /path/to/your/project

# 3. Search
spelunk search "database connection handling"

# 4. Ask
spelunk ask "how are errors propagated in the indexer?"
```

## Installation

```bash
cargo install --path .
```

Or build and copy manually:

```bash
cargo build --release
cp target/release/spelunk ~/.local/bin/
```

**Requires**: Rust, and [LM Studio](https://lmstudio.ai/) running at `http://127.0.0.1:1234` with an embedding model and a chat model loaded.

## Documentation

- **[Getting Started](docs/getting-started.md)** — installation, configuration, first steps
- **[Commands](docs/commands.md)** — full reference for every subcommand
- **[Memory](docs/memory.md)** — persisting decisions, context, and requirements with `spelunk memory`
- **[Agent Guide](docs/agent-guide.md)** — using `spelunk` as infrastructure for AI coding agents
- **[Examples](docs/examples/)** — real-world usage patterns

## Features

- Semantic search over code by meaning, not keywords
- Natural language Q&A with source citations
- AST-based chunking (tree-sitter) — functions, classes, structs, not naive line-splits
- Incremental re-indexing via BLAKE3 hashing
- Call-graph awareness: enrich search results with callers/callees
- Cross-project search: link multiple indexed repos
- Project memory: store decisions, context, requirements, questions
- Auto-harvest memory from git commit history
- Git hooks: auto-index and harvest on every commit
- `spelunk plan create` — LLM-generated implementation plans saved as markdown checklists
- `spelunk verify` — semantic coherence check after code changes
- `AGENT=true` env var for machine-readable JSON output from any command

## Supported languages

Rust, Go, Python, TypeScript, JavaScript, JSX, TSX, Java, C, C++, Ruby, Swift, Kotlin, JSON, HTML, CSS, HCL, Proto, SQL, Markdown, plain text.

## Configuration

`~/.config/spelunk/config.toml`:

```toml
lmstudio_base_url = "http://127.0.0.1:1234"
embedding_model   = "text-embedding-embeddinggemma-300m-qat"
llm_model         = "google/gemma-3n-e4b"
batch_size        = 32
```

See [Getting Started](docs/getting-started.md) for full configuration options.

## Development

```bash
cargo build          # debug
cargo build --release
cargo test
```

## License

MIT
