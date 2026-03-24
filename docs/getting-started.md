# Getting Started

## Prerequisites

### Rust

Install via [rustup.rs](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### LM Studio

Download [LM Studio](https://lmstudio.ai/) and load two models:

1. **Embedding model** — recommended: `google/embeddinggemma-300m-qat` (fast, 300M params)
2. **Chat model** — any instruction-tuned model works; `google/gemma-3-4b-it` is a good starting point for 8 GB RAM

Start the local server (default port: `1234`). The server icon in the toolbar shows when it's running.

## Installation

Clone and build:

```bash
git clone https://github.com/usercise/spelunk
cd spelunk
cargo build --release
```

Copy the binary somewhere on your `$PATH`:

```bash
cp target/release/spelunk ~/.local/bin/
# or
sudo cp target/release/spelunk /usr/local/bin/
```

Verify:

```bash
spelunk --version
```

## Configuration

`spelunk` looks for a config file at `~/.config/spelunk/config.toml`. If it doesn't exist, all defaults apply.

```toml
# ~/.config/spelunk/config.toml

# LM Studio server address
lmstudio_base_url = "http://127.0.0.1:1234"

# Must match the "API Identifier" shown in LM Studio for each model
embedding_model = "text-embedding-embeddinggemma-300m-qat"
llm_model       = "google/gemma-3n-e4b"

# Embedding batch size — lower this if you run out of memory
batch_size = 32

# Default database location (default: ~/.local/share/spelunk/<project-slug>.db)
# db_path = "/custom/path/myproject.db"
```

You can also override the database path per-command with `--db <path>`.

## Indexing your first project

```bash
spelunk index /path/to/your/project
```

This will:
1. Walk the file tree, respecting `.gitignore`
2. Parse supported source files with tree-sitter
3. Embed each chunk via your embedding model
4. Store everything in a SQLite database

On subsequent runs, only changed files are re-processed.

```bash
# Force a full re-index
spelunk index /path/to/your/project --force
```

## Searching

```bash
# Semantic search — finds code by meaning
spelunk search "error handling in the HTTP layer"

# With call-graph enrichment
spelunk search "authentication" --graph

# Return JSON instead of text
spelunk search "database migrations" --format json
```

## Asking questions

```bash
spelunk ask "How does the incremental indexing work?"
spelunk ask "What files handle user authentication?" --json
```

## Checking index health

```bash
# Show statistics for the current project
spelunk status

# Check whether the index is up to date (exits 1 if stale)
spelunk check
```

## Setting up automatic indexing

Install a git post-commit hook so `spelunk` indexes and harvests memory on every commit:

```bash
spelunk hooks install
```

This adds a hook that runs `spelunk index` and `spelunk memory harvest` after each commit. Other developers without `spelunk` installed are unaffected — the hook checks for the binary first.

To remove:

```bash
spelunk hooks uninstall
```

## Next steps

- [Commands reference](commands.md) — every flag and option
- [Memory](memory.md) — storing project context across sessions
- [Agent Guide](agent-guide.md) — using `spelunk` with AI coding agents
