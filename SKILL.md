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
spelunk search "<query>" --format text|json|ndjson

# Deep search — iterative, uses LLM (requires llm_model in config)
spelunk explore "<question>"
spelunk explore "<question>" --max-steps 5
spelunk explore "<question>" --json       # {answer, sources, steps}

# Call/import graph
spelunk graph <symbol-or-file>
spelunk graph <symbol> --kind calls       # calls | imports | extends | implements
spelunk graph <file> --format text|json|ndjson

# Status and checks
spelunk status --format text|json|ndjson
spelunk check --format text|json|ndjson

# Inspect what was indexed for a file
spelunk chunks <file-path>
spelunk chunks <file-path> --format text|json|ndjson
```

Use `search` for targeted lookups. Use `explore` when the answer requires
tracing across multiple files — it runs autonomously and reports back.

---

## Plumbing commands

Plumbing commands emit NDJSON and are designed for scripts and pipelines.
Exit codes: `0` = success, `1` = no results, `2` = error. See [Plumbing and Porcelain](docs/plumbing-and-porcelain.md) for full details.

```bash
# Emit indexed chunks for a file
spelunk plumbing cat-chunks <file>

# List all indexed files (optionally filtered by prefix or staleness)
spelunk plumbing ls-files [--prefix <p>] [--stale]

# Parse a file without writing to the index
spelunk plumbing parse-file <file>

# Compute and verify file hash
spelunk plumbing hash-file <file>

# Read embedding from stdin, return nearest chunks by similarity
echo "your query" | spelunk plumbing embed --query | spelunk plumbing knn --limit 10

# Emit code graph edges (imports, calls, extends)
spelunk plumbing graph-edges --file <f> | --symbol <s>

# Emit memory entries as NDJSON
spelunk plumbing read-memory [--kind <k>] [--limit N]
```

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

# Supersede an old entry (archives the old one; creates a supersedes edge)
spelunk memory add --kind decision --title "New auth approach" --body "..." \
  --supersedes <old-id>

# Link two entries as related (creates a relates_to edge)
spelunk memory add --kind note --title "Follow-up observation" --body "..." \
  --relates-to <other-id>
```

**Kinds:** `decision` · `context` · `requirement` · `note` · `intent` · `answer` · `handoff` · `question` · `antipattern`

### Query

```bash
spelunk memory search "<question>"        # semantic search over stored entries
spelunk memory search "<q>" --expand-graph  # also include 1-hop relates_to neighbours
spelunk memory list                       # recent entries
spelunk memory list --kind decision       # filter by kind
spelunk memory list --kind decision --limit 10
spelunk memory list --as-of 2026-01-01   # point-in-time snapshot
spelunk memory show <id>                  # full entry + relationships
spelunk memory graph <id>                 # relationship graph for an entry
spelunk memory timeline "<topic>"         # topic evolution across all entries (ASC time)
spelunk memory since <epoch>              # poll for entries newer than Unix timestamp
spelunk memory watch                      # stream new entries as they arrive (SSE; requires memory_server_url)
spelunk memory search "<q>" --format json
spelunk memory failures                   # list all antipatterns (shortcut for list --kind antipattern)
spelunk memory failures --limit 30
```

### Harvest from git history or Claude Code history

```bash
spelunk memory harvest                    # analyse HEAD~10..HEAD
spelunk memory harvest --git-range v0.1.0..HEAD
spelunk memory harvest --branch main      # full branch history
spelunk memory harvest --source claude-code --confirm  # extract from ~/.claude/history.jsonl
spelunk memory harvest --source failures  # extract antipatterns from revert/bugfix commits
spelunk memory harvest --source failures --git-range v0.4.0..HEAD
```

Extracts decisions, requirements, and non-obvious notes. From git, analyzes commit messages.
From `claude-code`, reads agent session transcripts from `~/.claude/history.jsonl`.
Run at the start of a session on a new repo, or after a batch of significant commits.
Requires `llm_model` in config. The `--source claude-code` requires `--confirm` flag.

---

## Status & registry

```bash
spelunk status                 # index health for current project
spelunk status --all           # all registered projects
spelunk status --list          # one-line table
spelunk status --format json   # machine-readable output

spelunk check                  # verify index is fresh; shows active intents and file-overlap warnings
spelunk check --format json    # machine-readable output

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
spelunk check                              # includes active intents and overlapping work
spelunk memory list --kind decision --limit 10
spelunk memory list --kind handoff --limit 3
spelunk memory list --kind intent          # see what teammates are working on
spelunk memory list --kind question
spelunk memory failures                    # check antipatterns — things to avoid
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
