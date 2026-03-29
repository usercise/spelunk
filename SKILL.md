# spelunk — AI Agent Skill Reference

How to use `spelunk` as an AI agent to understand, search, and query
a codebase — and to build up a persistent memory of why it was built
the way it was.

spelunk is a **context retrieval tool** for AI agents. You are the reasoning
engine. Use spelunk to find the right code and prior decisions, then reason
over that context yourself.

---

## Prerequisites

- `spelunk` installed and in PATH (`cargo install --path .` or copy the binary)
- Any OpenAI-compatible server (LM Studio, Ollama, vLLM, …) running with an
  **embedding model** loaded (required for all search)
- Default endpoint: `http://127.0.0.1:1234` (override with `api_base_url`
  in `~/.config/spelunk/config.toml`)

---

## Indexing a project

```bash
# Index a project (run once; subsequent runs are incremental)
spelunk index <path>

# Force full re-index (after changing embedding model or config)
spelunk index <path> --force
```

The index is stored at `<path>/.spelunk/index.db` and auto-discovered
when running any other command from inside the project directory.

---

## Core commands

### Search — find relevant code

```bash
spelunk search "<query>"                     # top 10 results
spelunk search "<query>" --limit 20          # more results (max 100)
spelunk search "<query>" --format json       # machine-readable output
spelunk search "<query>" --graph             # enrich with call-graph neighbours
```

Returns ranked chunks with file path, line range, language, symbol name, and
a content preview. Use `--format json` for programmatic access.

**This is the primary tool.** Use it to retrieve context, then reason over the
results yourself — the same way `spelunk ask` does internally, but you are the
reasoning engine.

### Explore — agentic deep search

```bash
spelunk explore "<question>"                 # iterative tool-use loop, prints answer
spelunk explore "<question>" --verbose       # show tool calls on stderr
spelunk explore "<question>" --max-steps 5   # limit iterations
spelunk explore "<question>" --json          # {answer, sources, steps}
```

Use `explore` when a single `search` call is unlikely to be enough — for example when the answer requires tracing through several files or understanding why something was built a certain way. The LLM calls `search`, `graph`, `read_chunk`, and `read_file` iteratively until it can answer confidently.

Requires `llm_model` to be set in config. For fast, targeted lookups, prefer `search`.

### Chunks — inspect what was indexed

```bash
spelunk chunks <file-path>                   # exact or suffix match
spelunk chunks <file-path> --format json
```

Use this when search results seem wrong — shows exactly what the indexer
extracted and what text each chunk embeds as.

### Graph — call/import relationships

```bash
spelunk graph <symbol-or-file>
spelunk graph <symbol> --kind calls          # filter: calls, imports, extends, implements
spelunk graph <file.rs> --format json
```

### Status — index health

```bash
spelunk status                   # current project
spelunk status --all             # all registered projects
spelunk status --list            # brief one-line-per-project table
```

---

## Project memory

The memory store is a semantic database of decisions, context, and
requirements that persists alongside the code index. It answers the question
"why was this built this way?" rather than "what does this code do?".

Memory lives at `<project>/.spelunk/memory.db`, separate from the code
index, and is never overwritten by re-indexing.

### Storing a memory entry

```bash
# Record an architectural decision
spelunk memory add \
  --kind decision \
  --title "Use sqlite-vec for KNN instead of a separate vector DB" \
  --body "Evaluated Qdrant and Chroma. Chose sqlite-vec to keep the tool
self-contained with no external process dependency. Acceptable at our
scale (<1M chunks). Revisit if we need filtering + ANN at the same time." \
  --tags "architecture,storage,embeddings" \
  --files "src/storage/db.rs,migrations/002_vectors.sql"

# Record context or requirements from a human
spelunk memory add \
  --kind context \
  --title "Target users are solo developers and small teams" \
  --body "Primary use case is a single developer understanding a codebase
they didn't write. Multi-user / concurrent write is out of scope for v1."

# Record a requirement
spelunk memory add \
  --kind requirement \
  --title "Must work offline — no cloud API calls during search" \
  --body "All inference must be local. LM Studio is acceptable as a local
server. No Anthropic/OpenAI API keys should be required."

# General note
spelunk memory add \
  --kind note \
  --title "Tree-sitter grammar version pinning" \
  --body "Grammar versions must match the tree-sitter core version. Bumping
core without checking grammars produces silent parse failures."
```

**Kinds:**
- `decision` — an architectural or design choice and the reasoning behind it
- `context` — background, constraints, or requirements from a human
- `requirement` — a hard constraint the codebase must satisfy
- `note` — anything that doesn't fit the above but should be remembered

### Querying memory

```bash
# Semantic search — finds entries by meaning, not keywords
spelunk memory search "why did we choose this database"
spelunk memory search "authentication approach"
spelunk memory search "what constraints did the user specify"

# List recent entries
spelunk memory list
spelunk memory list --kind decision
spelunk memory list --kind context --limit 5

# Show full content of an entry
spelunk memory show 3

# Machine-readable output
spelunk memory search "storage decisions" --format json
spelunk memory list --format json
```

---

## Agent obligations — keeping the index current

Re-indexing is incremental: only files whose content has changed since the
last run are re-parsed and re-embedded, so it is fast enough to run routinely.

**Re-index after:**
- Every `git commit` — even small changes move the index out of sync
- Any refactor that renames, moves, or restructures files
- Adding a significant new feature (new files, new symbols)
- Updating dependencies that change generated or vendored code

```bash
spelunk index <project-root>
```

If you changed the embedding model or prompt format, add `--force` to
regenerate all embeddings from scratch.

Not re-indexing means searches will miss new code and may surface deleted code.
Make it the last step of any commit workflow.

---

## Agent obligations — memory

**These are not optional.** Building up project memory is part of the agent's
responsibility on every session.

### At the start of a session
Before beginning any significant work, check what has already been recorded:

```bash
spelunk memory list --kind decision --limit 10
spelunk memory search "<topic you are about to work on>"
```

This prevents re-litigating decisions that have already been made and gives
context for why the code looks the way it does.

### During a session — what to store

Store a memory entry whenever any of the following occurs:

1. **A human states a constraint or requirement** — even informally.
   > "we don't want external API calls" → `spelunk memory add --kind requirement …`

2. **A significant design decision is made** — especially when alternatives
   were considered and rejected.
   > Chose X over Y because Z → `spelunk memory add --kind decision …`

3. **A surprising or non-obvious fact is discovered** — about the codebase,
   its dependencies, or its environment.
   > "tree-sitter grammar versions must match core" → `spelunk memory add --kind note …`

4. **A human provides background context** about the project, its users, or
   its goals.
   > "this is used by solo developers" → `spelunk memory add --kind context …`

**What NOT to store:**
- Things already visible in the code or git history
- Ephemeral task state ("currently working on X")
- Debugging steps that didn't lead to a conclusion

### Writing good memory entries

- **Title**: one sentence, past tense for decisions ("Chose X"), present tense
  for context ("Target users are…")
- **Body**: include *why*, not just *what*. What alternatives were considered?
  What constraint drove the choice? What will break if someone ignores this?
- **Tags**: use consistent tags so `spelunk memory list --kind decision` stays useful
- **Files**: link to the files most affected so future searches surface this
  entry when those files are relevant

---

## Recommended agent workflow

**At the start of any session:**
1. `spelunk memory list --kind decision` — review prior decisions
2. `spelunk memory search "<topic>"` — find relevant context before starting

**For code understanding questions:**
1. `spelunk search "<topic>"` — find the most relevant code chunks
2. Read the files at the reported line ranges
3. `spelunk graph <symbol>` to trace call chains or imports
4. `spelunk memory search "<topic>"` — check if there's recorded context explaining *why*
5. Synthesise an answer from the retrieved context yourself

**For code changes:**
1. Search and read before changing
2. After making a significant decision, store it: `spelunk memory add --kind decision …`
3. If the human explains a constraint that shaped your approach, store it too
4. After committing, re-index: `spelunk index <project-root>`

**For structured context retrieval (pipelines):**
```bash
spelunk search "<topic>" --format json | jq '.[].content'
spelunk memory search "<topic>" --format json | jq '.[].body'
```

---

## Multi-project search

```bash
# Make searches in project A also return results from project B
spelunk link <path-to-B>
spelunk unlink <path-to-B>
spelunk autoclean        # remove registry entries for deleted projects
```

Once linked, `spelunk search` queries both indexes and merges results by semantic
distance. Memory is always per-project.

---

## Agent mode (`AGENT=true`)

Set `AGENT=true` in the environment before running any `spelunk` command to enable
machine-readable output without extra flags:

```bash
AGENT=true spelunk search "authentication flow"        # → JSON, no spinner
AGENT=true spelunk graph src/storage/db.rs             # → JSON
AGENT=true spelunk memory search "storage decisions"   # → JSON
```

What changes when `AGENT=true` is set:
- All `--format text` defaults become `--format json` automatically.
- Progress spinners and animated progress bars are suppressed, keeping stdout
  clean for downstream parsing.

You can still pass `--format text` explicitly to override even in agent mode.

---

## Tips

- `spelunk index` must be pointed at the project root; all other commands can be
  run from any subdirectory — the DB is auto-discovered by walking up.
- After changing the embedding model, run `spelunk index <path> --force` to
  regenerate all embeddings. Also re-run `spelunk memory add` for important entries
  so their embeddings reflect the new model.
- `spelunk search` and `spelunk memory search` only need the embedding model.
- Secret-containing chunks (AWS keys, PEM headers, tokens, etc.) are
  automatically skipped during indexing and will not appear in results.
