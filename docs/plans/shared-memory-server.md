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
┌─────────────────────────────────────────┐
│  Developer A                            │
│  spelunk CLI → LM Studio (embed)        │──POST /memory → ┐
│  spelunk CLI → local index.db (search)  │                  │
└─────────────────────────────────────────┘                  ▼
                                                  ┌──────────────────────┐
                                                  │  spelunk-server       │
                                                  │  shared memory.db    │
                                                  │  REST API            │
                                                  └──────────────────────┘
┌─────────────────────────────────────────┐                  ▲
│  Developer B                            │                  │
│  spelunk CLI → LM Studio (embed)        │──POST /memory → ┘
│  spelunk CLI → local index.db (search)  │
└─────────────────────────────────────────┘
```

## Configuration

```toml
# ~/.config/spelunk/config.toml  (personal, gitignored)
memory_server_url = "http://spelunk.internal:7777"
memory_server_key = "your-shared-api-key"

# .spelunk/config.toml  (checked in, team-wide default)
memory_server_url = "http://spelunk.internal:7777"
```

Environment variable override: `SPELUNK_MEMORY_SERVER_URL`, `SPELUNK_MEMORY_SERVER_KEY`.

No `memory_server_url` set → local mode (current behaviour, unchanged).

## Auth

Single shared API key per server instance. Passed as `Authorization: Bearer <key>` header.
Network-level access control (VPN / private subnet) is the real security boundary for a
behind-firewall deployment.

**v2 consideration**: per-user tokens for audit trails and granular revocation. Not in scope
for v1 — don't let it block shipping.

## Server

A new `spelunk-server` binary (or `spelunk serve` subcommand) that exposes a REST API:

```
POST   /memory              — add entry (receives pre-embedded vector from client)
GET    /memory              — list entries (kind, limit, offset)
GET    /memory/:id          — get single entry
POST   /memory/search       — KNN search (client sends query vector)
DELETE /memory/:id          — delete entry
GET    /health              — liveness check
```

The server holds `memory.db` (SQLite, WAL mode). No LLM, no embedding model — it only
stores and retrieves vectors that clients computed.

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

### Phase 2: Server
- [ ] Add `memory_server_url` + `memory_server_key` to `Config`
- [ ] Abstract memory store behind a `MemoryBackend` trait (local SQLite vs remote HTTP)
- [ ] Implement `RemoteMemoryBackend` (HTTP client wrapping the REST API)
- [ ] Implement `spelunk serve` subcommand (or separate binary)
- [ ] REST API: add, list, get, search, delete, health
- [ ] Server auth middleware (Bearer token)
- [ ] Store + enforce embedding dimension on server
- [ ] `spelunk memory push` — migrate local entries to server

### Phase 3: Packaging
- [ ] `Dockerfile` for `spelunk-server`
- [ ] `docker-compose.yml` (minimal: server + volume)
- [ ] `docker-compose.full.yml` (server + Ollama with GPU compose profile)
- [ ] Document server setup in `docs/server.md`
- [ ] Update `docs/getting-started.md` with team setup section
