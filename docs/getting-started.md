# Getting Started

## 1. Install spelunk

Download the latest binary for your platform from the [releases page](https://github.com/usercise/spelunk/releases) and put it somewhere on your `$PATH`:

```bash
# macOS (Apple Silicon) — universal binary also available
curl -L https://github.com/usercise/spelunk/releases/latest/download/spelunk-v0.3.0-aarch64-apple-darwin.tar.gz \
  | tar -xz && chmod +x spelunk spelunk-server && sudo mv spelunk spelunk-server /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/usercise/spelunk/releases/latest/download/spelunk-v0.3.0-x86_64-apple-darwin.tar.gz \
  | tar -xz && chmod +x spelunk spelunk-server && sudo mv spelunk spelunk-server /usr/local/bin/

# macOS (universal — works on both Intel and Apple Silicon)
curl -L https://github.com/usercise/spelunk/releases/latest/download/spelunk-v0.3.0-universal-apple-darwin.tar.gz \
  | tar -xz && chmod +x spelunk spelunk-server && sudo mv spelunk spelunk-server /usr/local/bin/

# Linux x86_64
curl -L https://github.com/usercise/spelunk/releases/latest/download/spelunk-v0.3.0-x86_64-unknown-linux-gnu.tar.gz \
  | tar -xz && chmod +x spelunk spelunk-server && sudo mv spelunk spelunk-server /usr/local/bin/

# Linux ARM64
curl -L https://github.com/usercise/spelunk/releases/latest/download/spelunk-v0.3.0-aarch64-unknown-linux-gnu.tar.gz \
  | tar -xz && chmod +x spelunk spelunk-server && sudo mv spelunk spelunk-server /usr/local/bin/

# Verify
spelunk --version
```

> Replace `v0.1.0` with the version you want. The URL pattern is:
> `https://github.com/usercise/spelunk/releases/latest/download/spelunk-<version>-<target>.tar.gz`

> Building from source? See [Building](building.md).

## 2. Set up an inference server

spelunk works with any **OpenAI-compatible** inference server. The easiest options:

- **[LM Studio](https://lmstudio.ai/)** — desktop app for macOS/Windows/Linux; enable the local server (default port `1234`)
- **[Ollama](https://ollama.com/)** — `ollama serve` (default port `11434`)
- **vLLM / any OpenAI proxy** — point `api_base_url` at your endpoint

Load two models in your server of choice:

1. **Embedding model** — recommended: `google/embeddinggemma-300m-qat` (fast, 300M params, low VRAM)
2. **Chat model** — any instruction-tuned model; `google/gemma-3-4b-it` is a good starting point on 8 GB RAM (optional — only needed for `memory harvest` and `plan create`)

## 3. Configuration

`spelunk` looks for a config file at `~/.config/spelunk/config.toml`. If it doesn't exist, all defaults apply.

```toml
# ~/.config/spelunk/config.toml

# Base URL for your OpenAI-compatible server
# LM Studio default:  http://127.0.0.1:1234
# Ollama default:     http://127.0.0.1:11434
api_base_url = "http://127.0.0.1:1234"

# Must match the model's API identifier on your server
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

## 4. Initialise your project

The quickest way to get started is `spelunk init`. Run it from inside your project directory:

```bash
cd /path/to/your/project
spelunk init
```

This single command:
1. Registers the project in the global spelunk registry
2. Walks the file tree (respecting `.gitignore`), parses every source file, embeds each chunk, and stores everything in SQLite
3. Prints a summary with file/chunk counts, the DB path, and suggested next commands

```
spelunk initialised for my-project

  Index:   142 files, 1 840 chunks
  DB:      ~/.local/share/spelunk/my-project.db
  Hook:    not installed — run `spelunk hooks install` to add

Next steps:
  spelunk search "your query"
  spelunk ask "how does X work?"
```

### Optional flags

```bash
# Also install the post-commit git hook in one step
spelunk init --hook

# Register without indexing (index later with `spelunk index .`)
spelunk init --no-index
```

Running `spelunk init` again is safe — it notices an existing index and won't re-register.

### Manual indexing

If you prefer to manage indexing yourself, you can skip `init` and call `spelunk index` directly:

```bash
spelunk index /path/to/your/project

# Force a full re-index (ignore change detection)
spelunk index /path/to/your/project --force
```

On subsequent runs, only changed files are re-processed (blake3 hash comparison).

## 5. Try it out

```bash
# Semantic search — finds code by meaning
spelunk search "error handling in the HTTP layer"

# Hybrid search (semantic + full-text) — the default
spelunk search "authentication" --mode hybrid

# Pure text search — no embedding model needed
spelunk search "handleRequest" --mode text

# Fit results within a token budget (useful for agent context windows)
spelunk search "database layer" --budget 4000

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

# Machine-readable output for scripts
spelunk check --porcelain

# List which files are stale
spelunk check --porcelain --files
```

## 7. Set up automatic indexing

Install a git post-commit hook so `spelunk` indexes and harvests memory on every commit (or use `spelunk init --hook` to do this at init time):

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
