# Getting Started

## 1. Install spelunk

Download the latest binary for your platform from the [releases page](https://github.com/usercise/spelunk/releases) and put it somewhere on your `$PATH`:

```bash
# macOS (Apple Silicon)
curl -L https://github.com/usercise/spelunk/releases/latest/download/spelunk-aarch64-apple-darwin \
  -o spelunk && chmod +x spelunk && sudo mv spelunk /usr/local/bin/

# Verify
spelunk --version
```

> Building from source? See [Building](building.md).

## 2. Set up LM Studio

spelunk runs inference locally via [LM Studio](https://lmstudio.ai/). Download it and load two models:

1. **Embedding model** — recommended: `google/embeddinggemma-300m-qat` (fast, 300M params, low VRAM)
2. **Chat model** — any instruction-tuned model; `google/gemma-3-4b-it` is a good starting point on 8 GB RAM

Start the LM Studio local server (the toggle in the top toolbar, default port `1234`). spelunk will connect automatically.

## 3. Configuration

`spelunk` looks for a config file at `~/.config/spelunk/config.toml`. If it doesn't exist, all defaults apply.

```toml
# ~/.config/spelunk/config.toml

# LM Studio server address
lmstudio_base_url = "http://127.0.0.1:1234"

# Must match the "API Identifier" shown in LM Studio for each model
embedding_model = "text-embedding-embeddinggemma-300m-qat"

# Optional: set a chat model to enable `memory harvest` and `plan create`
# llm_model = "google/gemma-3n-e4b"

# Embedding batch size — lower this if you run out of memory
batch_size = 32

# Default database location (default: ~/.local/share/spelunk/<project-slug>.db)
# db_path = "/custom/path/myproject.db"

# Directory (relative to project root) where `spelunk plan create` writes plan files
# plans_dir = "docs/plans"

# Directory (relative to project root) where spec markdown files are discovered
# during `spelunk index` and where `spelunk spec link` defaults to looking
# specs_dir = "docs/specs"
```

You can also override the database path per-command with `--db <path>`.

## 4. Index your first project

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

## 5. Try it out

```bash
# Semantic search — finds code by meaning
spelunk search "error handling in the HTTP layer"

# With call-graph enrichment
spelunk search "authentication" --graph

# Return JSON instead of text
spelunk search "database migrations" --format json
```

## 6. Check index health

```bash
# Show statistics for the current project
spelunk status

# Check whether the index is up to date (exits 1 if stale)
spelunk check
```

## 7. Set up automatic indexing

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
- [Building from source](building.md) — for contributors and platform builders

## Team setup (shared memory)

Working with teammates? Run `spelunk-server` so the whole team shares memory
instead of each person siloing their own decisions and context.

Add a `.spelunk/config.toml` at your repo root and commit it:

```toml
# .spelunk/config.toml — commit this, it contains no secrets
memory_server_url = "http://spelunk.internal:7777"
project_id        = "my-awesome-app"
```

Each developer adds the API key to their personal config:

```toml
# ~/.config/spelunk/config.toml — never commit
memory_server_key = "shared-team-key"
```

After that, all `spelunk memory` commands transparently use the server. Push
any existing local entries with `spelunk memory push`.

→ **[Server setup guide](server.md)** — Docker, API reference, production tips
