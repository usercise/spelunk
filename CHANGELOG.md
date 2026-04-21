# Changelog

All notable changes to spelunk are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
spelunk uses [Semantic Versioning](https://semver.org/).

---

## [0.5.0] — 2026-04-21

### Added

- **Unix plumbing/porcelain architecture** — 8 new `spelunk spelunk` plumbing
  subcommands emit machine-readable NDJSON to stdout and use conventional exit
  codes (0 = ok, 1 = no results, 2 = error). All porcelain commands now accept
  `--format text|json|ndjson` for structured output in scripts and agents.
  Plumbing commands: `cat-chunks`, `embed`, `graph-edges`, `hash-file`, `knn`,
  `ls-files`, `parse-file`, `read-memory`.

- **`spelunk memory harvest --source claude-code`** — mines Claude Code session
  history files (`.claude/projects/*/sessions/*.jsonl`) for decisions, notes,
  and requirements; deduplicates against already-stored entries; stores the
  results directly in the memory index.

- **`intent` memory kind** — agents record work-in-progress intent entries so
  collaborating agents (and humans) can see what is actively being changed.
  `spelunk check` now shows active agent sessions alongside the index health
  summary, and warns when any intent's linked files overlap with files recently
  modified in the current worktree.

- **Server-side conflict detection** — `spelunk-server` runs a KNN similarity
  search before storing each new memory entry; entries that closely contradict
  an existing active entry are flagged with a `contradicts` edge and the HTTP
  response includes a `409 Conflict` status with the conflicting entry IDs.
  A `--conflict-threshold` flag controls the cosine-distance trigger.

- **`spelunk memory since` / `spelunk memory watch`** — incremental memory feed
  (`since`) and a long-running SSE stream (`watch`) for agents that want to be
  notified of new memory entries in real time. _(coming soon in 0.5.x — not
  yet merged as of this release)_

- **Benchmark scripts** (`bench/`) for evaluating search quality across
  indexing configurations.

### Changed

- `--format text|json` standardised across all porcelain commands (`ask`,
  `explore`, `search`, `graph`, `memory list`, `memory search`). The legacy
  `--json` flag is kept as a hidden deprecated alias.

- `storage/memory.rs` split into focused sub-modules
  (`storage/memory/`, `storage/db/`) to reduce file size and improve
  navigability, as part of the broader Unix-architecture refactor.

### Fixed / Security

- **XML escaping in LLM prompts** — spec titles and paths interpolated into
  `<spec_context>` blocks are now escaped with `escape_xml()`, closing a
  prompt-injection vector.

- **Expanded secret scanner** — `src/indexer/secrets.rs` now recognises
  OpenAI, Anthropic, and Stripe API keys; npm automation tokens; and database
  connection URLs containing inline credentials. Patterns compile once via
  `OnceLock`.

- **Atomic memory transactions** — `NoteStore` archive and supersede operations
  now run inside a single SQLite transaction; partial writes on crash are no
  longer possible.

- Resolved all security-audit findings from `cargo audit` (#136, #137, #138,
  #145) by upgrading affected dependency versions.

---

## [0.4.1] — 2026-03-21

Initial public release.
