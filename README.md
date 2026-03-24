# spelunk — local code intelligence

spelunk gives individual developers and AI agents context-aware code understanding without sending your code anywhere. Your codebase, your models, your machine.

It indexes your source tree with [tree-sitter](https://tree-sitter.github.io/) AST chunking, stores vector embeddings in a SQLite file you own, and answers questions via a RAG pipeline backed by a locally-running LLM in [LM Studio](https://lmstudio.ai/).

## Why local-first?

Cloud-based code intelligence tools require your source code to leave your machine and be indexed on someone else's servers. That's a non-starter for proprietary codebases, regulated industries, or anyone who simply doesn't want their code analysed by a third party.

spelunk keeps everything local:
- **Your index is a SQLite file** — lives in your project, backed up with your repo, belongs to you
- **Your models run locally** — via LM Studio; no API keys, no usage billing, no rate limits
- **No vendor lock-in** — switch models, move machines, or stop using spelunk without losing anything
- **Works offline** — no internet required after installation

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
- **[Agent Guide](docs/agent-guide.md)** — using spelunk as infrastructure for AI coding agents
- **[Examples](docs/examples/)** — real-world usage patterns

## Features

**Code intelligence**
- Semantic search over code by meaning, not keywords
- Natural language Q&A with source citations
- AST-based chunking (tree-sitter) — functions, classes, structs, not naive line-splits
- Incremental re-indexing via BLAKE3 hashing
- Call-graph awareness: enrich search results with callers/callees
- Cross-project search: link multiple indexed repos

**Project memory** — a structured alternative to CLAUDE.md files

> Research shows that static context files like CLAUDE.md [reduce agent task success rates](https://arxiv.org/abs/2501.12599) — agents misread them, ignore irrelevant sections, or get confused by stale information. spelunk memory fixes this: instead of a single file agents must parse in full, context is retrieved semantically — each agent call gets only the entries most relevant to the current task.

- Store decisions, context, requirements, questions, and handoff notes
- Semantically searchable: retrieved by meaning at query time, not dumped wholesale
- Pull in context from GitHub issues, Linear tickets, or any URL
- Auto-harvest memory from git commit history
- Git hooks: auto-index and harvest on every commit

**Agent-ready**
- `AGENT=true` env var forces JSON output on every command — no extra flags
- `spelunk plan create` — LLM-generated implementation plans as markdown checklists
- `spelunk verify` — semantic coherence check after code changes
- `spelunk check` — exits 1 if the index is stale; use in CI or pre-flight scripts

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
