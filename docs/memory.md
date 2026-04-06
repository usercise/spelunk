# Project Memory

`spelunk memory` is a per-project knowledge store. Use it to capture decisions, context, requirements, questions, and handoff notes that would otherwise live only in chat history or someone's head.

Memory entries are stored in a local SQLite database alongside your index and are searchable by meaning (semantic similarity), not just keywords.

## Why memory?

Code tells you *what* the system does. Memory tells you *why* it was built that way.

Examples of things worth storing:

- "We chose sqlite-vec over pgvector because the project must run without a Postgres server."
- "The embedding format is `title: {name} | text: {content}` — changing this invalidates all stored embeddings."
- "Current question: should `spelunk verify` re-embed from disk or from the stored chunk content?"
- "Handoff to next session: the graph migration is done, secrets scanner is next."

## Memory kinds

| Kind | Use for |
|------|---------|
| `decision` | Architecture or design choices with rationale |
| `context` | Background information that helps understand the codebase |
| `requirement` | Product or technical requirements |
| `note` | General observations (default) |
| `question` | Open questions that need an answer |
| `answer` | Answers to previously stored questions |
| `handoff` | State transfer between work sessions or agents |

## Storing a note

```bash
# Quick note with body inline
spelunk memory add --title "Chunker uses 120-line sliding window as fallback" \
              --body "This applies to unsupported file types and binary-adjacent files." \
              --kind context \
              --tags chunker,indexer

# Open your $EDITOR for the body (omit --body)
spelunk memory add --title "Decision: use blake3 for file hashing" --kind decision

# Link to specific files
spelunk memory add --title "Auth middleware refactored" \
              --body "Moved session validation to src/auth/middleware.rs" \
              --files "src/auth/middleware.rs,src/auth/session.rs"

# Record when a decision became valid (ISO 8601)
spelunk memory add --title "Adopted monorepo layout" --kind decision \
              --valid-at 2026-01-15

# Supersede an old entry — archives it and records a supersedes edge
spelunk memory add --title "New auth approach" --kind decision --body "..." \
              --supersedes <old-id>

# Mark two entries as related — creates a relates_to edge
spelunk memory add --title "Follow-up note" --kind note --body "..." \
              --relates-to <other-id>
```

When `--body` is omitted, `spelunk` opens `$VISUAL` or `$EDITOR` (falling back to `vi`). Lines starting with `#` are stripped (comment convention).

## Pulling in context from a URL

`--from-url` fetches content from a GitHub issue, Linear ticket, or any web page and stores it as a memory entry. The title is inferred from the page automatically.

```bash
# GitHub issue — uses `gh api` for clean structured content
spelunk memory add --from-url https://github.com/owner/repo/issues/42

# Override the inferred title
spelunk memory add --from-url https://github.com/owner/repo/issues/42 \
              --title "Auth: session token storage compliance issue" \
              --kind requirement

# Any URL — fetches page title and strips HTML
spelunk memory add --from-url https://linear.app/myteam/issue/ENG-1234/... \
              --kind context

# Combine with tags
spelunk memory add --from-url https://github.com/owner/repo/issues/99 \
              --tags auth,security --kind requirement
```

For GitHub issues, `spelunk` calls `gh api` to get structured issue data (requires the [GitHub CLI](https://cli.github.com/) and `gh auth login`). For all other URLs it does an HTTP GET and extracts readable text.

## Searching memory

```bash
# Semantic search — finds entries by meaning
spelunk memory search "why did we choose sqlite"
spelunk memory search "authentication decisions" --limit 5

# Also surface 1-hop relates_to neighbours of each result
spelunk memory search "authentication decisions" --expand-graph

# Search mode: hybrid (default), semantic, text
spelunk memory search "auth" --mode semantic
spelunk memory search "auth" --mode text

# Point-in-time: only entries that were valid at this date
spelunk memory search "auth decisions" --as-of 2026-01-01
```

## Tracking topic evolution

`spelunk memory timeline` returns all entries related to a topic, sorted by the time they became valid — useful for understanding how a decision or understanding evolved.

```bash
spelunk memory timeline "authentication strategy"
spelunk memory timeline "database choice" --limit 30
spelunk memory timeline "auth" --format json
```

## Listing entries

```bash
# List recent entries (newest first)
spelunk memory list

# Filter by kind
spelunk memory list --kind decision
spelunk memory list --kind question

# More entries
spelunk memory list --limit 50

# Point-in-time snapshot — only entries valid at a given date
spelunk memory list --as-of 2026-01-01

# Filter by commit SHA (exact or prefix)
spelunk memory list --source-ref abc1234
```

`question` and `answer` entries show titles only in list view to avoid context saturation. Use `spelunk memory show <id>` to read the full body.

## Showing a single entry

```bash
spelunk memory show 42
spelunk memory show 42 --format json
```

`memory show` displays the full body plus any incoming and outgoing relationship edges (supersedes, relates_to, contradicts) with linked entry titles.

## Relationship graph

```bash
# Show all edges for an entry (text)
spelunk memory graph 42

# Machine-readable
spelunk memory graph 42 --format json
```

## Harvesting from git history

`spelunk memory harvest` reads your git log, sends commit messages to the LLM, and automatically extracts significant entries:

```bash
# Default: last 10 commits
spelunk memory harvest

# Custom range
spelunk memory harvest --git-range HEAD~30..HEAD
spelunk memory harvest --git-range v1.0..HEAD
```

Already-harvested commits are skipped (tracked via a `git:<sha>` tag). Routine commits ("fix typo", "wip", etc.) are ignored by the LLM.

### Automatic harvesting

Install the git hook and harvesting happens on every commit:

```bash
spelunk hooks install
```

## Using memory as context

`spelunk memory search` results are best consumed alongside `spelunk search` results — they answer the *why* while the code search answers the *how*. Pass both to your reasoning model for a complete picture.

## Machine-readable output

All memory commands support `--format json`, and setting `AGENT=true` forces JSON mode globally:

```bash
AGENT=true spelunk memory list --kind question
AGENT=true spelunk memory search "database decisions"
```

## Tips

- **Store the "why", not just the "what"** — the code already captures what was built.
- **Use `question` kind actively** — when you hit a decision point you're unsure about, store it. Come back with `spelunk memory list --kind question` at the start of the next session.
- **Use `handoff` kind** at the end of a long session to summarise the current state for your next session (or for another agent).
- **Tag entries** — tags like `auth`, `database`, `performance` make `spelunk memory list` more scannable and improve search relevance.
- **Use `--supersedes` when updating a decision** — it archives the old entry, sets its invalidation time, and creates a traceable edge so you can always follow the chain of reasoning.
- **Use `--relates-to` for non-superseding connections** — linking a follow-up note or a contradicting observation lets `memory graph` and `--expand-graph` surface related context automatically.
- **Use `--as-of` for archaeology** — `spelunk memory list --as-of 2026-01-01` shows the knowledge state at that date, which is useful for post-mortems or understanding old decisions in context.
