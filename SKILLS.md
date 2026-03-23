# ca — AI Agent Skill Reference

How to use `ca` as an AI agent to understand, search, and answer questions
about a codebase.

---

## Prerequisites

- `ca` installed and in PATH (`cargo install --path .` or copy the binary)
- LM Studio running with an embedding model and a chat model loaded
- Default endpoint: `http://127.0.0.1:1234` (override with `lmstudio_base_url`
  in `~/.config/codeanalysis/config.toml`)

---

## Indexing a project

```bash
# Index a project (run once; subsequent runs are incremental)
ca index <path>

# Force full re-index (after changing embedding model or config)
ca index <path> --force
```

The index is stored at `<path>/.codeanalysis/index.db` and auto-discovered
when running any other command from inside the project directory.

---

## Core commands

### Search — find relevant code

```bash
ca search "<query>"                     # top 10 results
ca search "<query>" --limit 20          # more results (max 100)
ca search "<query>" --format json       # machine-readable output
ca search "<query>" --graph             # enrich with call-graph neighbours
```

Returns ranked chunks with file path, line range, language, symbol name, and
a content preview. Use `--format json` for programmatic access.

### Ask — answer a question

```bash
ca ask "<question>"
ca ask "<question>" --context-chunks 30     # retrieve more context (max 100)
ca ask "<question>" --json                  # structured output
```

`--json` returns: `{"answer": "...", "relevant_files": [...], "confidence": "high|medium|low"}`

### Chunks — inspect what was indexed

```bash
ca chunks <file-path>                   # exact or suffix match
ca chunks <file-path> --format json
```

Use this when search results seem wrong — shows exactly what the indexer
extracted and what text each chunk embeds as.

### Graph — call/import relationships

```bash
ca graph <symbol-or-file>
ca graph <symbol> --kind calls          # filter: calls, imports, extends, implements
ca graph <file.rs> --format json
```

### Status — index health

```bash
ca status                   # current project
ca status --all             # all registered projects
ca status --list            # brief one-line-per-project table
```

---

## Multi-project search

```bash
# Make searches in project A also return results from project B
ca link <path-to-B>

# Remove the dependency
ca unlink <path-to-B>

# Remove registry entries for deleted projects
ca autoclean
```

Once linked, `ca search` and `ca ask` query both indexes and merge results
by semantic distance.

---

## Recommended agent workflow

**For code understanding questions:**
1. `ca search "<topic>"` — find the most relevant chunks
2. Read the files at the reported line ranges
3. `ca graph <symbol>` if you need to trace call chains or imports
4. `ca ask "<question>"` for a synthesised answer when the question is
   conceptual rather than locational

**For locating a specific function/type:**
- `ca search "<name or description>"` is faster than `ca ask` and doesn't
  require the LLM to be loaded

**For structured answers (agent pipelines):**
```bash
ca ask "<question>" --json | jq '.answer'
ca ask "<question>" --json | jq '.relevant_files[]'
```

**For bulk inspection:**
```bash
ca search "<topic>" --format json | jq '.[].file_path' | sort -u
ca chunks src/some/file.rs --format json | jq '.[].content'
```

---

## Tips

- `ca index` must be pointed at the project root; all other commands can be
  run from any subdirectory — the DB is auto-discovered by walking up.
- After changing the embedding model, run `ca index <path> --force` to
  regenerate all embeddings with the new model.
- `ca search` does not require LM Studio's chat model — only the embedding
  model is needed. `ca ask` requires both.
- Secret-containing chunks (AWS keys, PEM headers, tokens, etc.) are
  automatically skipped during indexing and will not appear in results.
