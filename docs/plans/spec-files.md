# Plan: Spec Files as First-Class Citizens

Make markdown spec documents (PRDs, architecture docs, API contracts, design docs)
explicitly linkable to the code index and semantically searchable alongside code.

## Background

A useful pattern emerging in AI-assisted development is to treat specs as
infrastructure — living documents that agents consult during implementation to stay
aligned with intent. The pattern is: write the spec first, implement against it,
keep both in sync.

spelunk has `spelunk plan create` (one-shot LLM-generated checklists) but no concept
of a spec: a human-authored document that persists, stays linked to relevant code,
and is retrieved when an agent is about to modify something the spec governs.

The user's work team is already working with specs. When spelunk usage expands to
that team, agents need to be able to find and apply the relevant spec before making
changes — without the human having to manually include it in every prompt.

## What a spec is (vs a plan vs memory)

| | Spec | Plan | Memory |
|---|---|---|---|
| Author | Human | LLM (from description) | Human or harvested |
| Lifecycle | Long-lived, versioned | Disposable after completion | Permanent |
| Content | Intent, constraints, API contracts | Implementation checklist | Decisions, context |
| Indexed alongside | Code | — (in docs/plans/) | — (in memory.db) |
| Retrieved when | Agent touches related code | Agent checks progress | Agent needs context |

## Proposed design

### Storage
Specs live as markdown files anywhere in the project (user-controlled location,
e.g. `docs/specs/`, `specs/`, or project root). spelunk tracks them in a `specs`
table (path, title, linked_paths, last_indexed_at) — similar to how files are
tracked but with explicit linking metadata.

### Linking
A spec is linked to files/directories it governs:
```bash
spelunk spec link docs/specs/auth-redesign.md src/auth/
spelunk spec link docs/specs/api-contract.md src/api/ src/handlers/
```
Links are stored in the index. When `spelunk search` returns results from a linked
path, the governing specs are included in the result metadata.

### Auto-discovery (optional)
Spelunk can auto-detect spec files by frontmatter (`spelunk_spec: true`) or by
directory convention (`docs/specs/*.md`). Configurable in `config.toml`.

### Retrieval
- `spelunk ask` automatically includes relevant spec content when the question
  touches linked files — same injection pattern as memory context.
- `spelunk search --specs` includes spec chunks in results.
- `spelunk spec list` shows all tracked specs and their link coverage.

### Versioning / staleness
Specs are re-indexed on change (same BLAKE3 hash mechanism as code files).
`spelunk spec check` can flag specs that haven't been updated while their linked
code has changed significantly (uses the same drift signal as `spelunk status`).

## Discussion questions before implementing

- [ ] **Linking model**: explicit `spelunk spec link` vs frontmatter vs directory convention?
  - Explicit linking is precise but requires a command per spec.
  - Frontmatter is ergonomic for teams already writing specs in markdown.
  - Directory convention is zero-config but coarse.
  - Recommendation: directory convention as default, frontmatter to override, explicit
    link for fine-grained control. All three can coexist.

- [ ] **Chunking**: chunk spec files the same way as code (heading-based, like Markdown)?
  - Heading-based chunking already exists for Markdown via `ChunkKind::Section`.
  - Spec sections can be retrieved individually — only the relevant part is injected,
    not the whole spec. This avoids context saturation on large specs.

- [ ] **Injection in `spelunk ask`**: how many spec chunks alongside code chunks?
  - Separate budget: e.g. up to 5 spec chunks + up to 20 code chunks.
  - Or unified ranking: spec and code chunks compete on distance, same pool.
  - Separate budget is more predictable; unified ranking is more accurate.

- [ ] **`spelunk spec check`**: should this be part of `spelunk check` or separate?
  - Adding to `spelunk check` makes CI integration automatic.
  - Separate command gives more control over what blocks a build.

- [ ] **Team workflow**: how do specs get into the repo in the first place?
  - `spelunk spec create <description>` — LLM-generated spec draft (like `plan create`)
  - Human-authored from scratch, then `spelunk spec add` to register it
  - Both are valid; `spec create` has higher leverage for the agent use case

## Tasks

- [ ] Design `specs` and `spec_links` table schema (migration 004 or 005)
- [ ] Implement spec auto-discovery (directory convention + frontmatter)
- [ ] Wire spec indexing into `spelunk index` (reuse Markdown chunker)
- [ ] `spelunk spec link/unlink <spec> <path...>`
- [ ] `spelunk spec list [--format json]`
- [ ] `spelunk spec check` — flag specs whose linked code has drifted
- [ ] Inject spec context into `spelunk ask` (separate budget, configurable)
- [ ] Include governing specs in `spelunk search` result metadata
- [ ] Optional: `spelunk spec create <description>` — LLM-drafted spec
- [ ] Update `SKILL.md`, `docs/agent-guide.md`, `docs/commands.md`
