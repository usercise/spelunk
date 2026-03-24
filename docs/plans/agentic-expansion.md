# Agentic Engineering Plan

Analysis of Bassim Eledath's *8 Levels of Agentic Engineering* and the Dispatch
architecture paper against the current `spelunk` codebase, followed by a concrete
feature roadmap.

---

## The key insight: `spelunk` is the shared state layer

Most tools described in the levels framework — Dispatch, Inspect, Claude Code
itself — are orchestration tools. They plan, delegate, and coordinate. `spelunk` is
something different: it is the **knowledge persistence and retrieval layer**
that makes those tools more effective. Without it, every new agent session
starts blind. With it, sessions inherit the accumulated understanding of every
session before.

The Dispatch article sharpens this. Dispatch's architecture rests on three
design choices: a markdown checklist as the live state of in-progress work,
filesystem-based IPC for worker Q&A, and model-agnostic routing so any CLI can
be a worker. In that system, `spelunk` is the fourth pillar — the semantic index and
decision store that every worker reads before starting and writes to before
finishing.

That's a precise statement of what `spelunk` is for. The right question isn't "how
do we use agentic practices to improve `spelunk`?" but "how do we make `spelunk` the best
possible shared state layer for teams operating at levels 5–7?"

---

## Where `spelunk` sits today

### Level 3 — Context Engineering: solid

- `CLAUDE.md` is compact and navigable (module map, design decisions, commands).
  No dead weight.
- `SKILL.md` gives agents a precise instruction set, including when to search,
  what to store, and how to interpret results.
- `spelunk ask` separates untrusted RAG context from the question with XML
  delimiters — preventing prompt injection and improving information density.
- EmbeddingGemma task prefixes (`task: code retrieval | query: …`,
  `title: … | text: …`) are the right context for the right job, not a
  one-size-fits-all embedding.
- Secret scanning keeps credential noise out of context entirely.
- `AGENT=true` produces clean machine-readable output so agents don't waste
  tokens parsing human-formatted text.

This is genuinely good context engineering. The right context is present at
the right time.

### Level 4 — Compounding Engineering: partially closed

The levels framework describes a **plan → delegate → assess → codify** loop
where the codify step is what makes the loop compound. `spelunk memory` is the
codify mechanism. SKILL.md instructs agents to always store key decisions and
context. The architecture is right.

But the loop isn't fully closed yet. Codification is manual and high-friction.
An agent that forgets to run `spelunk memory add` leaves nothing behind. The
compounding effect only works if the store actually gets populated — and right
now that depends entirely on the agent following instructions. Instructions
get skipped; mechanisms don't.

### Level 5 — MCP and Skills: present

`spelunk` is deliberately a CLI rather than an MCP server. The Dispatch article
makes the trade-off explicit: MCP servers inject full tool schemas into context
on every turn whether the agent uses them or not. CLI tools only push the
output of the command the agent actually ran. `spelunk` keeps context lean by design.

SKILL.md is the skill reference. AGENT=true makes it pipeable. This level
is covered.

### Level 6 — Harness Engineering: significant gaps

The levels framework defines harness engineering as building "the entire
environment, tooling, and feedback loops that let agents do reliable work
without you intervening." The key concept is **backpressure**: automated
feedback mechanisms that let agents detect and correct mistakes without
human intervention.

`spelunk` has no backpressure today. There is no programmatic staleness signal.
There is no exit-code check agents can wire into CI or pre-commit hooks.
There is no self-verification path — an agent can't use `spelunk` to check whether
its own change is coherent with the existing codebase before committing.
Documentation freshness is mentioned in SKILL.md as an obligation but is not
enforced by any mechanism.

### Level 7 — Background Agents: gaps in the handoff layer

Background agents and agent-to-agent handoffs require that memory be
*designed* for inter-agent transfer, not just per-session note-taking. Right
now `spelunk memory` has no handoff kind, no structured way for one agent to tell
the next "here is what I did, what remains, and what you should check first."
There is also no automatic memory harvesting from git history — so every
newly spawned agent on an existing project starts with empty memory even
though years of decisions are recorded in commit messages.

---

## Patterns from the Dispatch architecture

Dispatch introduces three architectural patterns that directly illuminate what
`spelunk` is missing. Each maps to a concrete feature gap.

### Pattern 1: Checklist-as-state

> "The plan file IS the progress tracker. No databases, no signal files. Just
> a markdown checklist that the worker updates in place."

Dispatch uses a plain markdown file — human-readable, debuggable, git-friendly
— as the entire state management layer for a parallel workstream. Workers
check items off. The orchestrator reads the file to know what's done.

Right now there is a structural gap in `spelunk`'s mental model. `spelunk memory` stores
the *past* (decisions already made). The *present* — what is being worked on
right now, what steps remain, what a background worker should do next — has no
home.

A `spelunk plan` subsystem bridges this gap. Plan files live in `docs/plans/` inside
the project, visible alongside the code they describe, committed to git, and
readable by any agent or human without querying a database. The plan file is
the in-progress state; `spelunk memory` stores what was decided while completing it.
Together they give a complete picture of a project across time.

### Pattern 2: Filesystem IPC for worker Q&A

> "Workers write questions to numbered files, a lightweight sentinel detects
> them, you answer, and the worker continues with full context preserved. No
> restart. No re-explaining. No lost work."

The Dispatch article identifies a failure mode every agent user has hit: the
agent gets stuck, can't resolve the ambiguity, and either halts (losing all
context) or continues on a bad assumption (generating work that has to be
thrown away). The solution is an explicit question channel — the agent writes
the question down, stays running, and the answer gets routed back.

For `spelunk`, this maps to a `question` memory kind. A stuck agent writes
`spelunk memory add --kind question --title "should X use approach A or B?"`. The
human or orchestrator finds it with `spelunk memory list --kind question` (titles
only, to avoid context saturation), inspects a specific entry with
`spelunk memory show <id>`, answers it with `spelunk memory add --kind answer`, and the
next agent session reads the answer before resuming. No IPC daemon required —
`spelunk memory` is already the persistent channel between sessions.

The key insight from Dispatch's design: this is not a nice-to-have. Without
it, a background agent's only options when stuck are to hallucinate or halt.
Neither is acceptable for reliable autonomous work.

### Pattern 3: Model-agnostic routing (not a `spelunk` concern)

> "Any CLI that accepts a prompt can be a worker. Adding a new model is one
> line."

Dispatch routes different task types to different models. This is the
orchestrator's concern, not `spelunk`'s. `spelunk` is designed to work with any
downstream agent running any persona or model — it exposes a stable CLI
interface and lets the caller decide what sits above it. The model configured
in `config.toml` is the embedding and chat backend for `spelunk`'s own retrieval
work; which model the calling agent uses for its primary task is out of scope.

The relevant implication for `spelunk` is not a `--model` flag but the MCP server
mode (feature 9): when agents run in isolated sandboxed environments they need
a network-accessible `spelunk` instance rather than a local binary, so the
orchestrator can wire `spelunk` as a shared tool without bundling it into every
worker container.

### The context window problem (Dispatch's lens on `spelunk ask`)

The core problem Dispatch solves is context window saturation. "By the third
or fourth task, the model starts losing nuance on the earlier ones." Dispatch's
solution: give each task its own fresh window instead of cramming everything
into one.

This exact problem exists *inside* `spelunk ask` today. When `context_chunks` is
high, the LLM receives a large prompt that spans unrelated parts of the
codebase. At 50+ chunks the model starts losing the thread — the same
saturation dynamic Dispatch is designed to prevent, at smaller scale.

The Dispatch architecture suggests a two-stage approach: a cheap/fast
dispatcher selects the best chunks from a large candidate set, then the
expensive model only sees the curated subset. Applied to `spelunk ask`, this is
re-ranking: after KNN search returns N candidates, a second lightweight pass
selects the K chunks most relevant to the specific question before the LLM
call. The main model gets a focused, high-density context instead of a
firehose.

---

## Feature proposals

Features are grouped by the gap they close. Within each group, highest
leverage first.

---

### Group 1: Close the compounding loop

**1. `spelunk check` — exit-code staleness guard**

```
spelunk check          # exit 0 if fresh, exit 1 if stale
spelunk check --json   # {"fresh":false,"stale_files":3,"stale":[...]}
```

Compares the blake3 hash of every indexed file against the current on-disk
content. Returns exit 0 if all tracked files are current, exit 1 if any have
changed since the last index. Does no re-embedding — pure hash comparison.

This is pure backpressure — a constraint, not an instruction. A pre-commit
hook reading `spelunk check || spelunk index .` enforces freshness mechanically.

**2. `spelunk memory harvest [--git-range <rev-range>]`**

```
spelunk memory harvest                     # scan recent unparsed commits
spelunk memory harvest --git-range HEAD~50..HEAD
```

Reads git commit messages in the given range, uses the LLM to classify each
one (decision? requirement? context? note?), and auto-generates memory entries
tagged with the commit SHA to prevent duplicates.

This closes the compounding loop without requiring manual `spelunk memory add`
calls. Run it on an existing repo and years of decisions become searchable
immediately.

**3. `spelunk hooks install` — mechanical re-index obligation**

```
spelunk hooks install          # writes .git/hooks/post-commit
spelunk hooks install --ci     # prints a GitHub Actions workflow step
spelunk hooks uninstall
```

Writes a post-commit hook that:
- Checks if `spelunk` is in `PATH` — if not, exits 0 silently so other developers
  on the team are completely unaffected
- Runs `spelunk index <project-root>` to keep the index in sync
- Runs `spelunk memory harvest --git-range HEAD~1..HEAD` to capture any decisions
  from the new commit automatically

This removes the friction from both the re-index obligation and the memory
harvest obligation by making them automatic side-effects of committing.

---

### Group 2: Harness and self-verification

**4. `spelunk status --format json` — machine-readable health**

```
spelunk status --format json
# → {"file_count":…, "chunk_count":…, "embedding_count":…,
#    "last_indexed_unix":…, "memory_entry_count":…}
```

An orchestrator agent inspects this at the start of a session to decide
whether to re-index, harvest memory, or proceed directly. `spelunk status` currently
has no `--format` flag.

**5. `spelunk verify <file-or-symbol>`**

```
spelunk verify src/storage/db.rs
spelunk verify search_similar --json
```

Re-embeds the current file/symbol content without storing it, searches the
existing index for semantically nearest neighbours, and reports whether the
code is still coherent with its surroundings. Flags chunks that are
semantically isolated after a change — a signal the refactor may have moved
logic out of its natural home in the codebase.

---

### Group 3: Dispatch-native coordination

**6. `spelunk plan create/status` — checklist-as-state**

```
spelunk plan create "add rate limiting to the API"
# Queries index + memory, generates docs/plans/add-rate-limiting.md

spelunk plan status                        # all plans + completion %
spelunk plan status add-rate-limiting      # specific plan
spelunk plan status --format json
```

Plan files are standard markdown checklists in `docs/plans/` (visible,
committed, git-friendly — not hidden in `.spelunk/`). The `create`
command earns its keep by querying the codebase and memory before generating
the checklist, producing an informed plan grounded in the actual code state.

**7. `spelunk memory add --kind question/answer/handoff`**

Three new first-class memory kinds alongside `decision | context | requirement
| note`:

- **`question`**: A blocking ambiguity a worker needs resolved before
  continuing. `spelunk memory list --kind question` shows titles only (not body
  previews) to avoid context saturation when there are many open questions.
  Use `spelunk memory show <id>` to read a specific question's body.
- **`answer`**: Resolution to an open question, linked by title convention.
- **`handoff`**: Written at session end — what was done, what remains, what
  the next agent should check first. The next session's opening move:
  `spelunk memory list --kind handoff --limit 3`.

For human ergonomics, when `--body` is omitted from `spelunk memory add`, `spelunk`
opens `$EDITOR` (falling back to `$VISUAL`, then `vi`) so longer entries
can be written comfortably.

---

### Group 4: Retrieval quality

**8. Two-stage retrieval in `spelunk ask` (re-ranking) — Phase 3**

```
spelunk ask "…" --rerank
```

After KNN search returns N candidates, a fast lightweight pass selects the K
most relevant chunks before the expensive LLM call. This applies the Dispatch
context window insight at the retrieval level: the main model gets a focused,
high-density context instead of a firehose of marginally relevant chunks.

---

### Group 5: Ecosystem

**9. MCP server mode (`spelunk serve --mcp`) — Phase 4**

```
spelunk serve --mcp
```

Exposes search, ask, graph, memory_search, memory_add, and plan_status as MCP
tools. When agents run in isolated sandboxed VMs (the cloud-hosted Dispatch
model), they need a network-accessible `spelunk` instance. The MCP layer is a thin
adapter over the same command handlers. The CLI-first approach remains default.

---

## Sequencing

```
Phase 1 (close the compounding loop)
  spelunk check                        — pure backpressure, smallest change
  spelunk status --format json         — orchestrator harness decisions
  spelunk hooks install                — mechanical re-index + harvest
  spelunk memory harvest               — bootstraps the decision store from git

Phase 2 (Dispatch-native coordination)
  spelunk memory --kind question       — async worker Q&A; unblocks stuck agents
  spelunk memory --kind answer         — resolution channel
  spelunk memory --kind handoff        — structured session transfer
  $EDITOR fallback for body       — human ergonomics for longer entries
  spelunk plan create/status           — checklist-as-state in docs/plans/
  spelunk verify                       — self-correction loop for background agents

Phase 3 (quality)
  two-stage retrieval (--rerank)  — focused context for complex questions
  spelunk memory auto                  — derived memory from high-confidence asks

Phase 4 (ecosystem)
  spelunk serve --mcp                  — shared spelunk instance for isolated workers
```

Phases 1 and 2 require no new dependencies and only minor schema changes. Phase
3 extends existing retrieval. Phase 4 is the largest infrastructure investment
but enables `spelunk` to serve genuinely parallel, sandboxed agent workstreams.

---

## What this does not change

The current architecture — sqlite-vec, LM Studio API, single-process,
self-contained binary — is the right substrate for all of the above. Every
proposal reuses the existing embedder, KNN search, and storage layer. The goal
is not to redesign `spelunk` but to make it a complete coordination substrate for
the Dispatch-style, Level 7 workflows that are becoming the prevailing pattern
for serious AI-assisted development.
