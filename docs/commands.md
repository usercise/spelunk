# Commands Reference

All commands accept `--config <path>` to override the default config file location.

---

## spelunk index

Index a codebase directory.

```
spelunk index <path> [options]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--db <path>` | auto | Override database path |
| `--batch-size <n>` | 32 | Embedding batch size |
| `--force` | false | Re-index all files regardless of hash |

**Example:**

```bash
spelunk index ./myproject
spelunk index ./myproject --force --batch-size 16
```

---

## spelunk search

Semantic search over indexed code.

```
spelunk search <query> [options]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-l, --limit <n>` | 10 | Number of results (max 100) |
| `--format text\|json` | text | Output format |
| `-g, --graph` | false | Enrich results with 1-hop call-graph neighbours |
| `--graph-limit <n>` | 10 | Max graph-expanded results to add |
| `-d, --db <path>` | auto | Override database path |

**Example:**

```bash
spelunk search "where is the JWT token validated"
spelunk search "database schema migration" --limit 5 --format json
spelunk search "authentication middleware" --graph
```

---

## spelunk ask

Answer a natural language question using your indexed codebase.

```
spelunk ask <question> [options]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--context-chunks <n>` | 20 | Number of chunks to retrieve as context |
| `--json` | false | Return structured JSON: `{ answer, relevant_files, confidence }` |
| `-d, --db <path>` | auto | Override database path |

**Example:**

```bash
spelunk ask "How does the indexer handle binary files?"
spelunk ask "What is the retry strategy for failed embeddings?" --json
```

Setting `AGENT=true` in the environment forces JSON output regardless of `--json`.

---

## spelunk status

Show indexing statistics for the current project (or all projects).

```
spelunk status [options]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-a, --all` | false | Show all registered projects |
| `-l, --list` | false | One-line-per-project format (implies `--all`) |
| `--format text\|json` | text | Output format |

**Example:**

```bash
spelunk status
spelunk status --all --format json
```

---

## spelunk check

Check whether the index is in sync with the source tree. Exits with code 1 if the index is stale.

```
spelunk check [options]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--format text\|json` | text | Output format |
| `-d, --db <path>` | auto | Override database path |

Useful in CI or as a pre-commit guard:

```bash
spelunk check || echo "Index is stale — run spelunk index"
```

---

## spelunk verify

Re-embed a file or symbol and show its nearest semantic neighbours. Useful for checking that a piece of code is retrievable after a refactor.

```
spelunk verify <target> [options]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--neighbours <n>` | 3 | Number of nearest neighbours per chunk |
| `--format text\|json` | text | Output format |
| `-d, --db <path>` | auto | Override database path |

`<target>` is matched against indexed file paths (suffix match).

**Example:**

```bash
spelunk verify src/auth/middleware.rs
spelunk verify src/auth/middleware.rs --neighbours 5 --format json
```

---

## spelunk graph

Query the code graph: imports, function calls, class inheritance.

```
spelunk graph <symbol> [options]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--kind <type>` | all | Filter: `imports`, `calls`, `extends`, `implements` |
| `--format text\|json` | text | Output format |
| `-d, --db <path>` | auto | Override database path |

**Example:**

```bash
spelunk graph RagPipeline
spelunk graph src/storage/db.rs --kind imports
```

---

## spelunk chunks

Show the raw indexed chunks for a file. Useful for debugging or providing precise context to an agent.

```
spelunk chunks <path> [options]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--format text\|json` | text | Output format |
| `-d, --db <path>` | auto | Override database path |

**Example:**

```bash
spelunk chunks src/indexer/parser.rs
spelunk chunks src/indexer/parser.rs --format json
```

---

## spelunk languages

List all supported programming languages and their tree-sitter parsers.

```
spelunk languages
```

---

## spelunk link / spelunk unlink

Add or remove a project dependency. When linked, `spelunk search` and `spelunk ask` also query the linked project's index.

```
spelunk link <path>
spelunk unlink <path>
```

**Example:**

```bash
# Search both this project and a shared library project
spelunk link ../shared-utils
```

---

## spelunk hooks

Manage git post-commit hooks.

```
spelunk hooks install [--ci]
spelunk hooks uninstall
```

`install` writes a post-commit hook that runs `spelunk index` and `spelunk memory harvest` after each commit. Developers without `spelunk` installed are unaffected.

`--ci` prints a GitHub Actions workflow step instead of writing a hook.

---

## spelunk memory

Store and query project context, decisions, and requirements. See [Memory](memory.md) for full documentation.

```
spelunk memory add --title "..." [--body "..."] [--kind decision] [--tags auth,db] [--files src/auth.rs]
spelunk memory search <query> [--limit 10] [--format text|json]
spelunk memory list [--kind decision] [--limit 20] [--format text|json]
spelunk memory show <id> [--format text|json]
spelunk memory harvest [--git-range HEAD~10..HEAD]
```

---

## spelunk plan

Create and track implementation plans as markdown checklists in `docs/plans/`.

```
spelunk plan create <description> [--name <slug>]
spelunk plan status [<name>] [--format text|json]
```

`create` queries the codebase and memory for context, then generates a `- [ ]` checklist via the LLM. The plan is saved to `docs/plans/<slug>.md`.

**Example:**

```bash
spelunk plan create "add rate limiting to the HTTP API"
spelunk plan status
spelunk plan status add-rate-limiting
```

---

## spelunk autoclean

Remove registry entries for projects whose root path no longer exists on disk.

```
spelunk autoclean
```

---

## Environment variables

| Variable | Effect |
|----------|--------|
| `AGENT=true` | Force JSON output for all commands (same as `--format json` + `--json`) |
| `RUST_LOG=debug` | Enable verbose logging |
| `EDITOR` / `VISUAL` | Editor opened by `spelunk memory add` when `--body` is omitted |
