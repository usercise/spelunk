# spelunk-server — Shared Memory Server

`spelunk-server` lets your team share project memory (decisions, context,
requirements) without sharing code. Each developer's code index stays local;
only memory entries travel to the server.

## Quick start (Docker)

```bash
# Clone and build
git clone https://github.com/usercise/spelunk
cd spelunk

# Start the server (no auth — dev only)
docker compose up -d

# Verify
curl http://localhost:7777/v1/health
# → ok
```

## With an API key (recommended)

```bash
# Generate a key
export SPELUNK_SERVER_KEY=$(openssl rand -hex 32)

# Start
SPELUNK_SERVER_KEY=$SPELUNK_SERVER_KEY docker compose up -d

# Save the key — you'll need to distribute it to your team
echo "SPELUNK_SERVER_KEY=$SPELUNK_SERVER_KEY"
```

## Client configuration

Each developer adds a `.spelunk/config.toml` at the project root (commit it):

```toml
# .spelunk/config.toml — commit this, it's not a secret
memory_server_url = "http://spelunk.internal:7777"
project_id        = "my-awesome-app"
```

Personal config (`~/.config/spelunk/config.toml` — never commit):

```toml
# ~/.config/spelunk/config.toml
memory_server_key = "your-shared-api-key"
```

Or use the environment variable:

```bash
export SPELUNK_SERVER_KEY=your-shared-api-key
```

## Migrating existing local memory

If team members have existing local `memory.db` entries, push them to the server:

```bash
# Make sure .spelunk/config.toml is set up first, then:
spelunk memory push
```

This reads your local `memory.db` and sends all active entries to the server.
Archived entries are skipped by default; pass `--include-archived` to push them.

## Multiple projects

One server instance supports multiple projects. Each project has its own
namespace — entries from `project_id = "api"` are invisible to clients
configured with `project_id = "frontend"`.

Projects are auto-created on first write — no registration step required.

## Embedding dimension

All clients writing to the same project must use the same embedding model.
The server records the embedding dimension on the first write and rejects
subsequent writes with a different dimension.

Default: 768 dimensions (EmbeddingGemma 300M).

If your team uses a different model, configure the server at startup:

```bash
docker compose run spelunk-server --embedding-dim 1024
```

Or via compose environment:

```yaml
environment:
  SPELUNK_EMBEDDING_DIM: "1024"
```

## Production deployment

`docker-compose.yml` is the recommended minimal deployment — just
`spelunk-server` plus a named volume for the SQLite database.

Key considerations:
- Put the server behind a VPN or private subnet (the API key is the app-level
  guard; network-level access control is the real security boundary)
- The SQLite WAL-mode database handles 2–20 concurrent writers comfortably
- Back up the volume (`spelunk.db`) with your normal database backup process
- For large teams or heavy write loads, see the plan for Postgres support

## Full stack with Ollama (Linux/NVIDIA only)

`docker-compose.full.yml` adds Ollama for server-side LLM inference. This
requires Linux + NVIDIA GPU + nvidia-container-toolkit. It does not work on
Apple Silicon (Docker runs in a Linux VM without GPU passthrough).

```bash
SPELUNK_SERVER_KEY=your-key docker compose -f docker-compose.full.yml up -d
```

## Running without Docker

```bash
# Build
cargo build --release --bin spelunk-server

# Run
./target/release/spelunk-server \
  --db /var/lib/spelunk/spelunk.db \
  --port 7777 \
  --key your-api-key
```

## API reference

All routes require `Authorization: Bearer <key>` except `/v1/health`.

```
GET    /v1/health
GET    /v1/projects
POST   /v1/projects/{project_id}/memory
GET    /v1/projects/{project_id}/memory           ?kind=&limit=&archived=
GET    /v1/projects/{project_id}/memory/{id}
POST   /v1/projects/{project_id}/memory/search
DELETE /v1/projects/{project_id}/memory/{id}
POST   /v1/projects/{project_id}/memory/{id}/archive
POST   /v1/projects/{project_id}/memory/{id}/supersede
GET    /v1/projects/{project_id}/stats
```
