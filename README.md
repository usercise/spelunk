# spelunk

**A local context engine for AI coding agents.** Grep finds strings — spelunk finds meaning.

spelunk indexes your codebase using tree-sitter AST parsing and semantic embeddings, then serves that context to AI agents (or you) via fast CLI queries. Your code never leaves your machine.

```bash
spelunk index .                                    # index the project
spelunk search "how does authentication work"      # semantic search
spelunk graph validate_token                       # callers, callees, imports
```

## Why spelunk?

AI coding agents are only as good as the context they can see. Most context tools either upload your code to a cloud service or rely on grep, which only finds exact strings.

spelunk is different:

- **Semantic search** — find code by what it *does*, not what it's called. Ask for "authentication" and find `validate_hmac_sha256` even though the word "authentication" appears nowhere in the file.
- **Code graph** — trace callers, callees, and imports across file boundaries. Understand not just *where* code is, but *how it connects*.
- **Multi-project search** — link local dependency projects and search across your entire stack in one query.
- **100% local** — no cloud, no API keys for core features, no telemetry. The index is a SQLite file in your project directory.
- **Any embedding model** — runs against any OpenAI-compatible embedding endpoint (LM Studio, Ollama, vLLM). Swap in a better model and get better results without waiting for a spelunk release.
- **Agent-native** — built to be called by AI agents, not to replace them. JSON output, git hooks for auto-indexing, and a structured memory system for cross-session context.

### When to use spelunk vs grep

| You want to... | Use |
|---|---|
| Find an exact function name | `rg "fn validate_token"` |
| Find code related to a concept | `spelunk search "request authentication"` |
| See what calls a function | `spelunk graph validate_token` |
| Search across linked projects | `spelunk search "connection pooling"` |
| Store a design decision for future sessions | `spelunk memory add --kind decision ...` |

spelunk complements agentic search tools (grep, file reading) — it handles the queries they can't.

## Quick start

**1. Install**

```bash
cargo install spelunk
```

> Or download a binary from the [releases page](https://github.com/usercise/spelunk/releases). See [Getting Started](docs/getting-started.md) for full instructions.

**2. Start an embedding model**

spelunk needs an embedding model running on any OpenAI-compatible endpoint. The easiest option is [LM Studio](https://lmstudio.ai/):

```bash
# Load google/embeddinggemma-300m-qat in LM Studio and start the server (port 1234)
```

**3. Index and search**

```bash
spelunk index .
spelunk search "error handling in the HTTP layer"
spelunk search "database migrations" --graph    # include callers/callees
```

## Core features

### Semantic search

```bash
spelunk search "how are errors propagated to the user"
spelunk search "database connection pooling" --graph --format json
```

Tree-sitter extracts functions, structs, classes, and methods as discrete chunks — not naive line splits. Each chunk is embedded and stored in a local SQLite database with the [sqlite-vec](https://github.com/asg017/sqlite-vec) extension for fast KNN search.

### Code graph

```bash
spelunk graph RagPipeline                        # all edges for a symbol
spelunk graph src/storage/db.rs --kind imports   # imports in a file
```

spelunk extracts import, call, extends, and implements edges from the AST. Use `--graph` on search to automatically expand results with 1-hop callers and callees.

### Project memory

Store decisions, requirements, and context that persist across agent sessions:

```bash
spelunk memory add --kind decision --title "Chose sqlite-vec over pgvector" \
  --body "Must run without a Postgres server. Revisit if we need filtering + ANN."
spelunk memory search "why did we choose this database"
spelunk memory harvest   # auto-extract decisions from recent commits
```

Memory entries are embedded and retrieved semantically — each query gets only the entries relevant to the current task, not the entire context file.

### Multi-project search

```bash
spelunk link ../shared-utils
spelunk search "connection pooling"   # searches both projects, merges by relevance
```

### Agent integration

Set `AGENT=true` for JSON output on every command. Install git hooks for automatic indexing:

```bash
spelunk hooks install   # post-commit: auto-index + auto-harvest memory
AGENT=true spelunk search "auth flow" | jq '.[0].file_path'
```

spelunk ships with a [Claude Code skill](SKILL.md) and [agent guide](docs/agent-guide.md) for integration with AI coding agents.

## Supported languages

Tree-sitter AST-aware chunking for: **Rust**, **Go**, **Python**, **TypeScript**, **JavaScript**, **JSX**, **TSX**, **Java**, **C**, **C++**, **Ruby**, **Swift**, **Kotlin**, **JSON**, **HTML**, **CSS**, **HCL**, **Proto**, **SQL**, **Markdown**.

All other file types are indexed as plain text with a sliding-window chunker.

## Documentation

- [Getting Started](docs/getting-started.md) — install, configure, index your first project
- [Commands](docs/commands.md) — full reference for every subcommand
- [Memory](docs/memory.md) — decisions, context, and requirements across sessions
- [Agent Guide](docs/agent-guide.md) — using spelunk with AI coding agents
- [Architecture](docs/architecture.md) — system design for contributors
- [Examples](docs/examples/) — real-world workflows

## Contributing

Contributions welcome. See [Building from source](docs/building.md) for setup instructions.

## License

[MIT](LICENSE)
