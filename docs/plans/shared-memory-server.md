# Plan: Shared Memory Server

Enable teams to share spelunk memory (decisions, context, requirements) across
developers without siloing knowledge per-user. The code index remains local.

Branch: `feat/shared-memory-server`

## Problem

Memory is currently per-user. Two developers working on the same codebase each
maintain their own `memory.db`. When one records "we chose sqlite-vec over pgvector
because we need to run without a Postgres server", the other never sees it.

The feedback from early users: "this is great but the information is siloed per user."

## Design principles

1. **Code index stays local** — each developer indexes their own checkout. Fast, offline,
   no code leaves the machine. The server stores no source code.
2. **Memory is shared** — the server is a shared memory store. Decisions, context,
   requirements, and handoffs are visible to the whole team.
3. **Local mode still works** — no `memory_server_url` in config → local SQLite, exactly
   as today. Solo developers and air-gapped machines are unaffected.
4. **Behind the firewall** — self-hosted only. No cloud backend.
5. **Clients embed, server stores** — clients compute embeddings locally (via LM Studio)
   before sending to the server. The server stores vectors and runs KNN search. It never
   calls an LLM itself, so it needs no GPU.

## Architecture

```
┌─────────────────────────────────────────────┐
│  Developer A  (project: frontend-app)        │
│  .spelunk/config.toml → project_id, url      │
│  spelunk CLI → LM Studio (embed locally)     │──┐
│  spelunk CLI → local index.db (code search)  │  │
└─────────────────────────────────────────────┘  │  POST /v1/projects/frontend-app/memory
                                                   ▼
                                        ┌─────────────────────────┐
                                        │      spelunk-server      │
                                        │                          │
                                        │  projects/               │
                                        │    frontend-app/  ───────│── memory entries
                                        │    payments-api/  ───────│── memory entries
                                        │    data-pipeline/ ───────│── memory entries
                                        │                          │
                                        │  spelunk.db (WAL)        │
                                        │  REST API :7777          │
                                        └─────────────────────────┘
                                                   ▲
┌─────────────────────────────────────────────┐  │  POST /v1/projects/frontend-app/memory
│  Developer B  (project: frontend-app)        │──┘
│  .spelunk/config.toml → same project_id      │
│  spelunk CLI → LM Studio (embed locally)     │
│  spelunk CLI → local index.db (code search)  │
└─────────────────────────────────────────────┘
```

## Configuration

Config loads in layers, later layers override earlier ones:

```
1. ~/.config/spelunk/config.toml   global personal defaults
2. .spelunk/config.toml            project-level, checked in, team-wide
3. env vars                        CI, scripts, temporary overrides
```

```toml
# .spelunk/config.toml  — committed to the repo, shared by the whole team
# Safe to check in: contains no secrets
memory_server_url = "http://spelunk.internal:7777"
project_id        = "my-awesome-app"
```

```toml
# ~/.config/spelunk/config.toml  — personal, never committed
# Secrets and personal preferences live here
memory_server_key  = "your-shared-api-key"
embedding_model    = "text-embedding-embeddinggemma-300m-qat"
llm_model          = "google/gemma-3n-e4b"
```

Environment variable overrides: `SPELUNK_SERVER_URL`, `SPELUNK_SERVER_KEY`, `SPELUNK_PROJECT_ID`.

No `memory_server_url` configured → local mode, unchanged behaviour.

spelunk discovers `.spelunk/config.toml` by walking up from the current directory to
the git root (same strategy as `.gitignore` discovery). This means the file can live at
the repo root and be found from any subdirectory.

## Auth

Single shared API key per server instance. Passed as `Authorization: Bearer <key>` header.
Network-level access control (VPN / private subnet) is the real security boundary for a
behind-firewall deployment.

**v2 consideration**: per-user tokens for audit trails and granular revocation. Not in scope
for v1 — don't let it block shipping.

## Multi-project server

The server supports multiple projects. Each project has its own memory namespace —
a team running spelunk across five repos gets five independent memory stores, all
served by the same spelunk-server instance.

### Project identity

`project_id` is a short slug set in `.spelunk/config.toml` (e.g. `my-awesome-app`).
It is scoped to a server instance — two different companies can use `api-server` as
their project_id on their own separate servers without conflict.

On first write for an unknown `project_id`, the server auto-creates the project.
No explicit registration step required.

### REST API

All routes are scoped under `/v1/projects/{project_id}`:

```
GET    /v1/health                                   — liveness (no auth required)
GET    /v1/projects                                 — list projects on this server
POST   /v1/projects/{project_id}/memory             — add entry (pre-embedded vector)
GET    /v1/projects/{project_id}/memory             — list (kind, limit, offset)
GET    /v1/projects/{project_id}/memory/{id}        — get single entry
POST   /v1/projects/{project_id}/memory/search      — KNN (client sends query vector)
DELETE /v1/projects/{project_id}/memory/{id}        — delete entry
GET    /v1/projects/{project_id}/stats              — entry counts, embedding dimension
```

All routes except `/v1/health` require `Authorization: Bearer <key>`.

The server holds a single `spelunk.db` (SQLite, WAL mode) with a `projects` table
and a `notes` table with a `project_id` foreign key. No LLM, no embedding model.

## Open questions

- [ ] **Single binary or separate binary?**
  - `spelunk serve` subcommand in the same binary: simpler distribution, one download.
  - Separate `spelunk-server` binary: cleaner separation, smaller server image.
  - Lean towards `spelunk serve` unless binary size becomes a concern.

- [ ] **SQLite WAL vs Postgres**
  - SQLite WAL mode handles small teams (2–20 concurrent writers) comfortably.
  - The write pattern is low-frequency: humans adding entries + harvest on commit.
  - Abstract storage behind a trait now so Postgres can be swapped in later.
  - Start with SQLite. Add Postgres support when a team hits the limits.

- [ ] **Vector search on the server**
  - sqlite-vec works fine for KNN on the server side — same approach as local.
  - The server receives a query vector from the client and runs KNN against stored vectors.
  - If we later move to Postgres, pgvector is the natural replacement.

- [ ] **Embedding dimension contract**
  - All clients writing to a shared server must use the same embedding model (same dimensions).
  - Server should store and enforce the embedding dimension on first write.
  - Clients using a different model should get a clear error, not silent corruption.
  - Consider: server records which model produced the embeddings; warn if clients differ.

- [ ] **Memory status field (lifecycle / deprecation)**
  - Active entries and archived/superseded entries are different things.
  - Add `status` field: `active` | `archived`. Archived entries excluded from search and
    `spelunk ask` context but visible via `--archived` flag.
  - Add `spelunk memory archive <id>` and `spelunk memory supersede <old> <new>` commands.
  - This is independent of the server but should land in the same migration.

## Docker packaging

`Dockerfile` for `spelunk-server`:
- Minimal Linux base (distroless or Alpine)
- Single static binary
- Volume mount for `memory.db` persistence
- Exposes port 7777

`docker-compose.yml` options:
1. **Minimal** — just `spelunk-server` + volume. Clients bring their own LM Studio.
2. **Full stack** — `spelunk-server` + Ollama (for teams that want server-side LLM for
   `spelunk ask`). Ollama supports NVIDIA GPU passthrough on Linux
   (`nvidia-container-toolkit`). Does not work on Apple Silicon (Docker runs in a Linux VM).

Note: "full stack" compose is Linux/NVIDIA only. The default compose should be minimal —
don't require GPU on the server host.

## Migration path for existing users

When `memory_server_url` is configured:
1. On first `spelunk memory add`, check if the server is reachable.
2. Offer (or automatically run) `spelunk memory push` to migrate local entries to the server.
3. After push, local `memory.db` can be kept as a read-only cache or ignored.

`spelunk memory push` command: reads local `memory.db`, sends all entries to server.

## Tasks

### Phase 1: Memory lifecycle (local, no server required)
- [ ] Add `status` field to `notes` table (`active` | `archived`)
- [ ] `spelunk memory archive <id>`
- [ ] `spelunk memory supersede <old-id> <new-id>` — marks old as archived, links to new
- [ ] Show `created_at` as human-readable age in `spelunk memory list` output
- [ ] Exclude archived entries from search and `spelunk ask` context
- [ ] `spelunk memory list --archived` to view historical entries

### Phase 2: Per-project config + client changes
- [ ] Add `.spelunk/config.toml` project-level config (walk up to git root to discover)
- [ ] Config loading order: global personal → project → env vars
- [ ] Add `memory_server_url`, `memory_server_key`, `project_id` fields to `Config`
- [ ] `project_id` required when `memory_server_url` is set; error clearly if missing
- [ ] Abstract memory store behind a `MemoryBackend` trait (local SQLite vs remote HTTP)
- [ ] Implement `RemoteMemoryBackend` (HTTP client, all routes scoped to project_id)

### Phase 3: Server
- [ ] Implement `spelunk serve` subcommand (or separate binary — decision pending)
- [ ] `projects` table in server DB: `(id, slug, embedding_dim, created_at)`
- [ ] Auto-create project on first write for unknown project_id
- [ ] REST API: all routes under `/v1/projects/{project_id}/`
- [ ] Server auth middleware (Bearer token, all routes except `/v1/health`)
- [ ] Store + enforce embedding dimension per project (error on mismatch)
- [ ] `spelunk memory push` — migrate local entries to server under correct project_id
- [ ] `GET /v1/projects` — list projects (useful for server admin)

### Phase 4: Packaging
- [ ] `Dockerfile` for `spelunk-server`
- [ ] `docker-compose.yml` (minimal: server + volume)
- [ ] `docker-compose.full.yml` (server + Ollama with GPU compose profile)
- [ ] Document server setup in `docs/server.md`
- [ ] Update `docs/getting-started.md` with team setup section
- [ ] Document `.spelunk/config.toml` format and what to commit vs gitignore
