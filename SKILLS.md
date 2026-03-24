# ca — AI Agent Skill Reference

How to use `ca` as an AI agent to understand, search, and answer questions
about a codebase — and to build up a persistent memory of why it was built
the way it was.

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

`ca ask` automatically includes both code context (HOW the system is built) and
memory context (WHAT was decided and WHY) when both are available. No extra flags
needed — if `memory.db` exists and has relevant entries, they are included.

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

## Project memory

The memory store is a semantic database of decisions, context, and
requirements that persists alongside the code index. It answers the question
"why was this built this way?" rather than "what does this code do?".

Memory lives at `<project>/.codeanalysis/memory.db`, separate from the code
index, and is never overwritten by re-indexing.

### Storing a memory entry

```bash
# Record an architectural decision
ca memory add \
  --kind decision \
  --title "Use sqlite-vec for KNN instead of a separate vector DB" \
  --body "Evaluated Qdrant and Chroma. Chose sqlite-vec to keep the tool
self-contained with no external process dependency. Acceptable at our
scale (<1M chunks). Revisit if we need filtering + ANN at the same time." \
  --tags "architecture,storage,embeddings" \
  --files "src/storage/db.rs,migrations/002_vectors.sql"

# Record context or requirements from a human
ca memory add \
  --kind context \
  --title "Target users are solo developers and small teams" \
  --body "Primary use case is a single developer understanding a codebase
they didn't write. Multi-user / concurrent write is out of scope for v1."

# Record a requirement
ca memory add \
  --kind requirement \
  --title "Must work offline — no cloud API calls during search" \
  --body "All inference must be local. LM Studio is acceptable as a local
server. No Anthropic/OpenAI API keys should be required."

# General note
ca memory add \
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
ca memory search "why did we choose this database"
ca memory search "authentication approach"
ca memory search "what constraints did the user specify"

# List recent entries
ca memory list
ca memory list --kind decision
ca memory list --kind context --limit 5

# Show full content of an entry
ca memory show 3

# Machine-readable output
ca memory search "storage decisions" --format json
ca memory list --format json
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
ca index <project-root>
```

If you changed the embedding model or prompt format, add `--force` to
regenerate all embeddings from scratch.

Not re-indexing means searches and `ca ask` will miss new code and may
surface deleted code. Make it the last step of any commit workflow.

---

## Agent obligations — memory

**These are not optional.** Building up project memory is part of the agent's
responsibility on every session.

### At the start of a session
Before beginning any significant work, check what has already been recorded:

```bash
ca memory list --kind decision --limit 10
ca memory search "<topic you are about to work on>"
```

This prevents re-litigating decisions that have already been made and gives
context for why the code looks the way it does.

### During a session — what to store

Store a memory entry whenever any of the following occurs:

1. **A human states a constraint or requirement** — even informally.
   > "we don't want external API calls" → `ca memory add --kind requirement …`

2. **A significant design decision is made** — especially when alternatives
   were considered and rejected.
   > Chose X over Y because Z → `ca memory add --kind decision …`

3. **A surprising or non-obvious fact is discovered** — about the codebase,
   its dependencies, or its environment.
   > "tree-sitter grammar versions must match core" → `ca memory add --kind note …`

4. **A human provides background context** about the project, its users, or
   its goals.
   > "this is used by solo developers" → `ca memory add --kind context …`

**What NOT to store:**
- Things already visible in the code or git history
- Ephemeral task state ("currently working on X")
- Debugging steps that didn't lead to a conclusion

### Writing good memory entries

- **Title**: one sentence, past tense for decisions ("Chose X"), present tense
  for context ("Target users are…")
- **Body**: include *why*, not just *what*. What alternatives were considered?
  What constraint drove the choice? What will break if someone ignores this?
- **Tags**: use consistent tags so `ca memory list --kind decision` stays useful
- **Files**: link to the files most affected so future searches surface this
  entry when those files are relevant

---

## Recommended agent workflow

**At the start of any session:**
1. `ca memory list --kind decision` — review prior decisions
2. `ca memory search "<topic>"` — find relevant context before starting

**For code understanding questions:**
1. `ca search "<topic>"` — find the most relevant code chunks
2. Read the files at the reported line ranges
3. `ca graph <symbol>` to trace call chains or imports
4. `ca memory search "<topic>"` — check if there's recorded context explaining *why*
5. `ca ask "<question>"` for a synthesised answer when needed

**For code changes:**
1. Search and read before changing
2. After making a significant decision, store it: `ca memory add --kind decision …`
3. If the human explains a constraint that shaped your approach, store it too
4. After committing, re-index: `ca index <project-root>`

**For structured answers (pipelines):**
```bash
ca ask "<question>" --json | jq '.answer'
ca memory search "<topic>" --format json | jq '.[].body'
```

---

## Multi-project search

```bash
# Make searches in project A also return results from project B
ca link <path-to-B>
ca unlink <path-to-B>
ca autoclean        # remove registry entries for deleted projects
```

Once linked, `ca search` and `ca ask` query both indexes and merge results
by semantic distance. Memory is always per-project.

---

## Tips

- `ca index` must be pointed at the project root; all other commands can be
  run from any subdirectory — the DB is auto-discovered by walking up.
- After changing the embedding model, run `ca index <path> --force` to
  regenerate all embeddings. Also re-run `ca memory add` for important entries
  so their embeddings reflect the new model.
- `ca search` and `ca memory search` only need the embedding model.
  `ca ask` requires both the embedding model and a chat model.
- Secret-containing chunks (AWS keys, PEM headers, tokens, etc.) are
  automatically skipped during indexing and will not appear in results.
