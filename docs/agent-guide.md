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
