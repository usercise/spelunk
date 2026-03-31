# spelunk — AI Agent Skill Reference

spelunk is a **context retrieval tool** for AI agents. Use it to find relevant
code and prior decisions, then reason over the results yourself.

---

## Setup

- `spelunk` in PATH
- OpenAI-compatible server at `http://127.0.0.1:1234` with an **embedding model** loaded
  (override with `api_base_url` in `~/.config/spelunk/config.toml`)

---

## Indexing

```bash
spelunk index <path>           # index (subsequent runs are incremental)
spelunk index <path> --force   # full re-index (after changing embedding model)
spelunk check                  # verify the index is fresh before starting work
```

---

## Code search

```bash
# Semantic search — primary lookup
spelunk search "<query>"
spelunk search "<query>" --limit 20
spelunk search "<query>" --graph          # include call-graph neighbours
spelunk search "<query>" --format json

# Deep search — iterative, uses LLM (requires llm_model in config)
spelunk explore "<question>"
spelunk explore "<question>" --max-steps 5
spelunk explore "<question>" --json       # {answer, sources, steps}

# Call/import graph
spelunk graph <symbol-or-file>
spelunk graph <symbol> --kind calls       # calls | imports | extends | implements
spelunk graph <file> --format json

# Inspect what was indexed for a file
spelunk chunks <file-path>
spelunk chunks <file-path> --format json
```

Use `search` for targeted lookups. Use `explore` when the answer requires
tracing across multiple files — it runs autonomously and reports back.

---

## Memory

Stores decisions, context, and requirements that persist across sessions.
Answers "why was this built this way?" alongside the code index.

### Add an entry

```bash
spelunk memory add \
  --kind decision \
  --title "Chose sqlite-vec over Qdrant" \
  --body "Keeps spelunk self-contained; no external process. Revisit if >1M chunks." \
  --tags "architecture,storage" \
  --files "src/storage/db.rs"
```

**Kinds:** `decision` · `context` · `requirement` · `note`

### Query

```bash
spelunk memory search "<question>"        # semantic search over stored entries
spelunk memory list                       # recent entries
spelunk memory list --kind decision       # filter by kind
spelunk memory list --kind decision --limit 10
spelunk memory show <id>                  # full entry
spelunk memory search "<q>" --format json
```

### Harvest from git history

```bash
spelunk memory harvest                    # analyse HEAD~10..HEAD
spelunk memory harvest --git-range v0.1.0..HEAD
spelunk memory harvest --branch main      # full branch history
```

Extracts decisions, requirements, and non-obvious notes from commit messages.
Run at the start of a session on a new repo, or after a batch of significant commits.
Requires `llm_model` in config.

---

## Status & registry

```bash
spelunk status                 # index health for current project
spelunk status --all           # all registered projects
spelunk status --list          # one-line table

spelunk autoclean              # remove stale registry entries (deleted/moved projects)
spelunk link <path>            # include another project's index in searches
spelunk unlink <path>
```

---

## Git worktrees

Worktrees automatically share the main worktree's index. Just run
`spelunk index .` from inside the worktree — spelunk creates the link for you:

```bash
git worktree add ../my-feature my-feature-branch
cd ../my-feature
spelunk index .    # links to main worktree's index; all commands work immediately
```

Run `spelunk autoclean` after removing a worktree to tidy up the registry.

---

## Agent mode

Set `AGENT=true` for clean machine-readable output on all commands:

```bash
AGENT=true spelunk search "authentication flow"
AGENT=true spelunk memory search "storage decisions"
AGENT=true spelunk graph src/storage/db.rs
```

---

## Agent workflow

**Start of every session:**
```bash
spelunk check
spelunk memory list --kind decision --limit 10
spelunk memory list --kind handoff --limit 3
spelunk memory list --kind question
```

**Understanding code:**
1. `spelunk search "<topic>"` — find relevant chunks
2. Read reported file/line ranges
3. `spelunk graph <symbol>` — trace call chains
4. `spelunk memory search "<topic>"` — check recorded context for *why*

**Making changes:**
1. Search and read before changing
2. Store significant decisions: `spelunk memory add --kind decision …`
3. Store constraints the human states: `spelunk memory add --kind requirement …`
4. After committing: `spelunk index <project-root>`

**End of session:**
```bash
spelunk memory add --kind handoff --title "Handoff: <summary>" \
  --body "what's done, what's next, open questions"
spelunk index .
```

**Writing good memory entries:**
- **Title**: one sentence — past tense for decisions, present tense for context
- **Body**: include *why*, what alternatives were rejected, what breaks if ignored
- **Tags**: keep consistent so `list --kind decision` stays useful
- **Files**: link affected files so entries surface in related searches

---

## Tips

- All commands except `spelunk index` can be run from any subdirectory — the index is found automatically.
- After changing the embedding model, run `spelunk index <path> --force`.
- `spelunk search` only needs the embedding model; `spelunk explore`, `spelunk memory harvest`, and LLM summaries also require `llm_model`.
