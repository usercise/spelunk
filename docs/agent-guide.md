# Agent Guide

`spelunk` is designed to work as infrastructure for AI coding agents, not just as a human developer tool. This guide covers the patterns that make agents most effective when paired with `spelunk`.

**The key mental model**: spelunk retrieves context; you reason over it. Use `spelunk search` to find the right code, read the results, then synthesise the answer yourself — just as you would after reading documentation. spelunk is a fast, semantic grep with memory, not an oracle.

## The core loop

A productive agentic session with `spelunk` looks like this:

1. **Orient** — read memory and check index health
2. **Plan** — generate a checklist for the current task
3. **Execute** — make code changes, delegating sub-tasks as needed
4. **Verify** — check semantic coherence after changes
5. **Codify** — store decisions and context in memory

This loop compounds: each session leaves better context for the next, whether that's the same agent resuming or a different one picking up.

## Machine-readable output

Set `AGENT=true` and every `spelunk` command returns JSON:

```bash
export AGENT=true

spelunk search "error handling"          # → JSON array of results
spelunk status                           # → { files, chunks, embeddings, ... }
spelunk memory list                      # → JSON array of notes
spelunk memory search "auth decisions"   # → JSON array of notes with distance scores
```

You can also use `--format json` on individual commands.

## Starting a session

At the start of a session, orient yourself:

```bash
# Check the index is fresh
spelunk check

# Review open questions from previous sessions
AGENT=true spelunk memory list --kind question

# Review recent handoff notes
AGENT=true spelunk memory list --kind handoff --limit 5

# Understand what's been decided
AGENT=true spelunk memory search "architecture decisions"
```

## Searching before writing

Before modifying any file, search for related code:

```bash
# Find relevant chunks
AGENT=true spelunk search "authentication middleware" --graph

# Get the raw chunks for a specific file
AGENT=true spelunk chunks src/auth/middleware.rs

# Understand the call graph around a symbol
AGENT=true spelunk graph validate_token
```

The `--graph` flag on `spelunk search` adds 1-hop callers and callees to the result set, which is often exactly the context needed to understand blast radius before a change.

## Retrieving targeted context

Use `spelunk search` with a focused query, then read the returned chunks and reason over them yourself:

```bash
# Find what touches the embedding format
AGENT=true spelunk search "embedding format storage" --graph --format json

# Trace call chains across the request lifecycle
AGENT=true spelunk graph handle_request
AGENT=true spelunk search "request lifecycle middleware" --limit 20 --format json
```

For open-ended questions that require tracing through several files, use `spelunk explore` instead. It runs the same search/graph/read tools in an LLM-driven loop and returns a synthesised answer:

```bash
# Let spelunk drive the search loop; get a final answer + sources
AGENT=true spelunk explore "how does incremental indexing decide which files to skip?"

# Limit steps if you want a fast, shallow answer
AGENT=true spelunk explore "where is the embedding model loaded?" --max-steps 3
```

`explore` is slower than `search` (multiple LLM calls) — use `search` for targeted lookups and `explore` for questions that need synthesis across multiple code paths.

## Creating plans

Before a significant change, generate a plan:

```bash
spelunk plan create "add rate limiting to the API layer"
# → writes docs/plans/add-rate-limiting.md with a - [ ] checklist

# Check progress
spelunk plan status
```

Check off items as you complete them by editing the markdown file directly (`- [ ]` → `- [x]`). `spelunk plan status` reads the file and shows completion percentages.

## After making changes

```bash
# Verify a modified file is still semantically retrievable
spelunk verify src/auth/middleware.rs

# Re-index to incorporate changes
spelunk index .
```

## Storing decisions

Every non-obvious choice should be stored:

```bash
spelunk memory add \
  --title "Chose sqlite-vec over hnswlib for vector search" \
  --body "No C++ dependency, single file, good enough performance for <1M vectors. Revisit if we need ANN at scale." \
  --kind decision \
  --tags storage,embeddings
```

Doing this consistently means future agents (and future you) can retrieve the rationale:

```bash
spelunk memory search "why did we choose sqlite-vec"
```

## Storing questions for async resolution

When you hit a decision point mid-task:

```bash
spelunk memory add \
  --title "Should verify re-embed from disk or from stored chunk content?" \
  --kind question \
  --tags verify,indexer
```

Pick it up later:

```bash
AGENT=true spelunk memory list --kind question
```

When resolved:

```bash
spelunk memory add \
  --title "verify re-embeds from stored chunk content" \
  --body "Avoids file I/O and keeps behaviour consistent with what was originally indexed. Disk content may have changed since last index." \
  --kind answer \
  --tags verify,indexer
```

## Handing off between sessions

At the end of a session, write a handoff note:

```bash
spelunk memory add \
  --title "Handoff: rate limiting plan 60% done" \
  --body "Implemented token bucket in src/ratelimit/bucket.rs. Next: wire middleware, add tests, update docs. Open question: should limits be per-IP or per-API-key?" \
  --kind handoff
```

At the start of the next session, read it:

```bash
AGENT=true spelunk memory list --kind handoff --limit 3
```

## Cross-project search

If your project depends on shared libraries you've indexed separately:

```bash
spelunk link ../shared-utils
spelunk link ../api-contracts
```

Now `spelunk search` queries all three indexes and merges results by distance.

## CI integration

```bash
# Fail the build if the index is stale
spelunk check || { echo "Run spelunk index"; exit 1; }

# Print a GitHub Actions workflow hook
spelunk hooks install --ci
```

## Plumbing Commands

Plumbing commands emit NDJSON to stdout and follow a strict exit-code convention, making them safe to use in scripts and pipelines. See [Plumbing and Porcelain](plumbing-and-porcelain.md) for a full explanation of the design philosophy.

Exit codes across all plumbing commands:
- **0** — success, results emitted
- **1** — no results (empty set, not an error)
- **2** — hard error (bad flags, missing DB, I/O failure) — diagnostics on stderr

### cat-chunks

```
spelunk plumbing cat-chunks <file>
```

Emit all indexed chunks for a given file as NDJSON.

| Flag | Description |
|------|-------------|
| `<file>` | Project-relative path of the file to retrieve chunks for (required). |

Exit codes: `0` = chunks found, `1` = file has no indexed chunks, `2` = error.

Example:

```bash
spelunk plumbing cat-chunks src/indexer/chunker.rs \
  | jq '{name: .name, lines: "\(.start_line)-\(.end_line)"}'
```

```json
{"name":"sliding_window","lines":"45-78"}
{"name":"Chunk","lines":"12-32"}
```

---

### ls-files

```
spelunk plumbing ls-files [--prefix <prefix>] [--stale] [--root <dir>]
```

List every indexed file as NDJSON. With `--stale`, only files whose on-disk blake3 hash differs from the stored hash are emitted.

| Flag | Description |
|------|-------------|
| `--prefix <prefix>` | Restrict output to files whose path starts with this string. |
| `--stale` | Only emit files that are out of date (on-disk hash ≠ stored hash). |
| `--root <dir>` | Project root for resolving relative paths (defaults to CWD). |

Exit codes: `0` = at least one file emitted, `1` = no files matched, `2` = error.

Example:

```bash
spelunk plumbing ls-files --stale --root .
```

```json
{"path":"src/indexer/chunker.rs","language":"rust","chunk_count":12,"indexed_at":1713528000,"stale":true}
```

---

### parse-file

```
spelunk plumbing parse-file <file>
```

Parse a file with tree-sitter and emit chunks as NDJSON without writing anything to the index. Useful for previewing how spelunk will chunk a file.

| Flag | Description |
|------|-------------|
| `<file>` | Path to the file to parse (required). |

Exit codes: `0` = chunks emitted, `1` = unsupported file type or empty parse result, `2` = read error.

Example:

```bash
spelunk plumbing parse-file src/config.rs | jq '{kind, name, start_line}'
```

```json
{"kind":"struct","name":"Config","start_line":8}
{"kind":"impl","name":"Config","start_line":42}
```

---

### hash-file

```
spelunk plumbing hash-file <file>
```

Compute the blake3 hash of a file and check whether it matches the hash stored in the index, emitting a single JSON object.

| Flag | Description |
|------|-------------|
| `<file>` | Path to the file to hash (required). |

Exit codes: `0` = always (unless read error), `2` = file not readable.

Example:

```bash
spelunk plumbing hash-file src/config.rs
```

```json
{"path":"src/config.rs","hash":"a3f1...","indexed_hash":"a3f1...","is_current":true}
```

---

### knn

```
spelunk plumbing knn [--limit N] [--min-score F] [--lang <lang>]
```

Read a JSON embedding object from stdin (as produced by `spelunk plumbing embed`) and return the *N* nearest indexed chunks by cosine similarity.

| Flag | Description |
|------|-------------|
| `--limit N` | Maximum number of results (default: `10`). |
| `--min-score F` | Drop results with cosine similarity below this threshold (0.0–1.0, default: `0.0`). |
| `--lang <lang>` | Restrict results to chunks from files of this language (e.g. `rust`, `python`). |

Exit codes: `0` = results found, `1` = no results pass the filters, `2` = error.

Compose with `embed` for a full semantic search pipeline:

```bash
echo "authentication" | spelunk plumbing embed --query | spelunk plumbing knn --limit 5
```

Example output:

```json
{"chunk_id":42,"file_path":"src/auth/middleware.rs","language":"rust","node_type":"function","name":"validate_token","start_line":18,"end_line":54,"content":"...","distance":0.12,"score":0.88}
```

---

### embed

```
spelunk plumbing embed [--query]
```

Read lines from stdin and emit one NDJSON embedding vector per line. Each output object contains the model name, vector dimensionality, and the float vector.

| Flag | Description |
|------|-------------|
| `--query` | Apply the query retrieval prefix (`task: code retrieval | query: …`). Use this flag when the output will be piped into `knn`. Omit it when embedding document text for storage. |

Exit codes: `0` = at least one vector emitted, `2` = stdin is a terminal (not a pipe) or embedding backend unreachable.

Compose with `knn`:

```bash
echo "authentication" | spelunk plumbing embed --query | spelunk plumbing knn --limit 5
```

Example output:

```json
{"model":"text-embedding-gemma-3","dimensions":1024,"vector":[0.021,-0.043,...]}
```

---

### graph-edges

```
spelunk plumbing graph-edges --file <file> | --symbol <symbol>
```

Emit code graph edges (imports, calls, extends/implements) for a file or symbol. At least one of `--file` or `--symbol` is required. When both are provided, results are merged and deduplicated.

| Flag | Description |
|------|-------------|
| `--file <file>` | Project-relative path; emit all edges originating from this file. |
| `--symbol <symbol>` | Symbol name; emit edges where this name appears as source or target. |

Exit codes: `0` = edges found, `1` = no edges matched, `2` = neither flag supplied or DB error.

Example:

```bash
spelunk plumbing graph-edges --symbol validate_token
```

```json
{"source_file":"src/auth/middleware.rs","source_name":"handle_request","target_name":"validate_token","kind":"calls","line":28}
```

---

### read-memory

```
spelunk plumbing read-memory [--kind <kind>] [--id <n>] [--limit N]
```

Emit memory entries as NDJSON. Use `--kind` to filter by entry type or `--id` to fetch a single entry.

| Flag | Description |
|------|-------------|
| `--kind <kind>` | Filter by memory kind: `decision`, `question`, `note`, `answer`, `requirement`, `handoff`. |
| `--id <n>` | Fetch a single entry by its integer id. Exits `1` if not found. |
| `--limit N` | Maximum number of entries (default: `50`). |

Exit codes: `0` = entries found, `1` = no entries matched, `2` = error.

Example:

```bash
spelunk plumbing read-memory --kind decision --limit 5 | jq '{id, title}'
```

```json
{"id":17,"title":"Chose sqlite-vec over hnswlib for vector search"}
{"id":22,"title":"Incremental index skips unchanged files via blake3 hash"}
```

---

## Summary: agent workflow at a glance

```bash
# Session start
spelunk check
AGENT=true spelunk memory list --kind handoff --limit 3
AGENT=true spelunk memory list --kind question

# Before writing code — retrieve context, then reason yourself
AGENT=true spelunk search "<topic>" --graph --format json
AGENT=true spelunk memory search "<topic>" --format json

# Planning
spelunk plan create "<description>"

# After changes
spelunk index .
spelunk verify <changed-file>

# Session end
spelunk memory add --title "Handoff: ..." --kind handoff
spelunk memory add --title "Decision: ..." --kind decision   # for each key choice
```
