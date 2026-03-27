# Plan: Convention Extraction

Auto-detect coding conventions during indexing and store them as memory context,
so agents understand team patterns without requiring manual `spelunk memory add` calls.

## Background

AI coding tools that learn team patterns automatically from the codebase are raising
the bar. spelunk's memory is entirely manual today — high-value but high-friction.
Auto-extracting conventions closes this gap for a class of context that is highly
repetitive to document by hand and tends to be consistent across a project.

Conventions worth extracting:
- Error handling patterns (e.g. `anyhow::Result`, `thiserror`, custom `AppError`)
- Naming conventions (snake_case functions, PascalCase types, prefix conventions)
- Module/file organisation patterns (one struct per file, flat vs nested modules)
- Test conventions (unit tests in same file, integration tests in `tests/`, fixture patterns)
- Documentation conventions (doc comments on public API, inline comments for non-obvious logic)
- Async patterns (tokio, async-std, blocking in spawn_blocking)
- Logging conventions (tracing macros, log levels in use)

## Discussion questions before implementing

- [ ] **Trigger**: run on every `spelunk index`, or as a separate `spelunk conventions` command?
  - On-index is zero-friction but adds latency; separate command is explicit but forgettable.
  - A middle path: run automatically but only when the project has never been analysed, then
    on-demand or after major re-indexes.

- [ ] **Storage**: conventions as `memory` entries (kind=`context`, tagged `convention`)?
  - Pros: immediately searchable alongside decisions, shows up in `spelunk ask` context.
  - Cons: pollutes `spelunk memory list` with auto-generated noise; harder to distinguish
    from human-written entries.
  - Alternative: a separate `conventions.db` or a `conventions` table, surfaced via
    `spelunk conventions list` and injected into `spelunk ask` context automatically.

- [ ] **LLM vs heuristic extraction**:
  - Heuristic: count patterns in AST nodes (fast, no LLM required, deterministic).
  - LLM: richer descriptions but adds latency and requires LM Studio to be running during index.
  - Hybrid: heuristic pass to gather evidence, LLM pass to write natural-language summaries.

- [ ] **Staleness**: conventions should be refreshed when enough of the codebase has changed.
  What threshold triggers a re-extraction? (e.g. >20% of files re-indexed since last run)

- [ ] **Scope**: per-language or project-wide?
  - A Rust project might have Rust conventions + shell script conventions.
  - Per-language is more useful but more complex to present.

## Proposed approach (to validate in discussion)

1. Heuristic AST pass during `spelunk index` (no LLM dependency):
   - Count node types and patterns across all chunks
   - Detect: error type names, async usage, test file patterns, doc comment coverage
   - Runs in the same tree-sitter pass as chunking — no extra file reads

2. Store results in a `conventions` table (new migration):
   - `(language, category, description, evidence_count, extracted_at)`
   - Separate from `memory` to avoid noise, but injected into `spelunk ask` context

3. `spelunk conventions` command:
   - `spelunk conventions list` — show extracted conventions
   - `spelunk conventions refresh` — re-run extraction

4. Optional LLM summary pass (requires LM Studio):
   - Takes raw heuristic output, produces human-readable convention descriptions
   - Run as `spelunk conventions refresh --summarise`

## Tasks

- [ ] Design `conventions` table schema (migration 004)
- [ ] Implement heuristic extraction in the indexer (AST pass)
- [ ] Wire into `spelunk index` (post-chunking, pre-embedding)
- [ ] Inject top conventions into `spelunk ask` context alongside memory
- [ ] Add `spelunk conventions list [--format json]`
- [ ] Add `spelunk conventions refresh [--summarise]`
- [ ] Update `SKILL.md` and `docs/agent-guide.md`
