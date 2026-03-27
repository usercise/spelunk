# spelunk — local code intelligence

spelunk gives individual developers and AI agents context-aware code understanding without sending your code anywhere. Your codebase, your models, your machine.

## Why spelunk?

Most code intelligence tools require your source code to leave your machine — indexed on someone else's servers, processed by someone else's models, subject to someone else's terms. That's a non-starter for proprietary codebases, regulated industries, or anyone who simply doesn't want their IP analysed by a third party.

spelunk keeps everything local:

- **Your code never leaves your machine** — the index is a SQLite file that lives in your project, backed up with your repo, owned by you
- **Any model, your choice** — runs against any model loaded in [LM Studio](https://lmstudio.ai/); no API keys, no billing, no rate limits
- **Works offline** — no internet required after setup
- **No lock-in** — switch models, move machines, or stop using spelunk without losing anything

## What it does

**Semantic search** — find code by what it does, not what it's called. spelunk uses AST-based chunking (tree-sitter) to index functions, structs, and classes as discrete units, then embeds them for meaning-based retrieval.

```bash
spelunk search "error handling in the HTTP layer"
spelunk search "database connection pooling" --graph   # enrich with callers/callees
```

**Natural language Q&A** — ask questions about your codebase and get answers with source citations.

```bash
spelunk ask "how does incremental re-indexing work?"
spelunk ask "what would break if I changed the embedding format?" --json
```

**Project memory** — a structured alternative to CLAUDE.md files

> Research shows that static context files like CLAUDE.md [reduce agent task success rates](https://arxiv.org/abs/2501.12599) — agents misread them, ignore irrelevant sections, or get confused by stale information. spelunk memory fixes this: context is retrieved semantically at query time, so each agent call gets only the entries most relevant to the current task.

```bash
spelunk memory add --kind decision --title "Chose sqlite-vec over pgvector" \
  --body "Must run without a Postgres server. Revisit if we need filtering + ANN."
spelunk memory search "why did we choose this database"
spelunk memory add --from-url https://github.com/owner/repo/issues/42  # pull in ticket context
```

**Agent-ready** — `AGENT=true` forces JSON output on every command. Pair with git hooks to auto-index and harvest memory on every commit.

```bash
spelunk hooks install   # post-commit: auto-index + auto-harvest memory
AGENT=true spelunk search "auth flow" | jq '.[0].file_path'
```

## Getting started

→ **[Getting Started](docs/getting-started.md)** — install, configure LM Studio, index your first project

## Documentation

- [Getting Started](docs/getting-started.md)
- [Commands](docs/commands.md) — full reference for every subcommand
- [Memory](docs/memory.md) — decisions, context, and requirements across sessions
- [Agent Guide](docs/agent-guide.md) — spelunk as infrastructure for AI coding agents
- [Examples](docs/examples/)

## Supported languages

The following languages get **AST-aware indexing** — spelunk uses tree-sitter to extract semantic chunks (functions, classes, methods) rather than raw line splits:

Rust, Go, Python, TypeScript, JavaScript, JSX, TSX, Java, C, C++, Ruby, Swift, Kotlin, JSON, HTML, CSS, HCL, Proto, SQL, Markdown.

Any other file type is indexed as plain text using a sliding-window chunker, so it still shows up in search results.

## License

MIT
