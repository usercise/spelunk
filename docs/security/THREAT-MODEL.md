# spelunk Threat Model

**Method:** Lightweight threat modeling (STRIDE-informed)  
**Last reviewed:** April 2026  
**Reviewed by:** Architect  
**Next review:** v1.0 release or after any new network-facing feature

---

## System Overview

spelunk is a local CLI tool that:
1. Walks source trees, parses files with tree-sitter, and stores chunks in SQLite
2. Embeds chunks by calling a local OpenAI-compatible HTTP server (127.0.0.1:1234)
3. Runs KNN search over stored embeddings via sqlite-vec
4. Optionally sends context + a user question to a local LLM for `spelunk ask`
5. Maintains a `memory.db` of structured notes with semantic search

**Trust boundary:** The user's machine. There are no remote clients, no shared databases, no authentication tokens stored by spelunk itself.

---

## Assets

| Asset | Confidentiality | Integrity | Availability |
|-------|:-:|:-:|:-:|
| Source code chunks in index | Medium | High | Medium |
| Credentials accidentally present in source | High | — | — |
| Memory notes (decisions, handoffs) | Medium | High | Medium |
| Embedding vectors | Low | Medium | Low |
| spelunk config (`~/.config/spelunk/config.toml`) | Medium | High | Medium |

---

## Trust Boundaries and Data Flows

```
User filesystem
  │
  ├─ spelunk index ─► [secret scanner] ─► SQLite index.db (chunks + vectors)
  │
  ├─ spelunk ask/search
  │     ├─► embed query via HTTP ─► local embedding server (127.0.0.1:1234)
  │     ├─► KNN search ─► index.db
  │     └─► LLM prompt ─► local LLM server (127.0.0.1:1234)
  │           └─ context assembled from: code chunks + spec files + memory notes
  │
  └─ spelunk memory ─► memory.db (SQLite)
```

The only network egress is to `127.0.0.1:1234` — a local server the user controls. No data leaves the machine.

---

## Threat Analysis (STRIDE)

### S — Spoofing
**No applicable threats.** No authentication; the only callers are the user and local processes.

### T — Tampering

| Threat | Likelihood | Impact | Mitigation |
|--------|-----------|--------|-----------|
| Malicious file in indexed project injects SQL via chunk content | Low | High | All DB writes use rusqlite parameterised queries — string formatting into SQL is forbidden by policy |
| `memory.db` edited directly to corrupt supersession state | Low | Medium | Atomic transactions in `insert_with_supersession()` and `supersede()` (issue #136) |
| Embedding server returns malformed vectors | Low | Low | Dimension validation on KNN input; errors exit 2 |

### R — Repudiation
**Not applicable.** No audit log requirement for a single-user local tool.

### I — Information Disclosure

| Threat | Likelihood | Impact | Mitigation |
|--------|-----------|--------|-----------|
| Credentials in source code indexed into vector DB | Medium | High | `secrets.rs` scanner drops matching chunks before storage; `.env*`/`*.pem`/`*.key` files excluded from traversal |
| Credentials in chunk content sent to local LLM | Low | Medium | Same scanner runs before embedding; secrets never reach the embedding API |
| Indexed content includes credentials not matched by current patterns | Medium | Medium | Pattern coverage issue — tracked in #138 (add OpenAI, Anthropic, Stripe, NPM, database URL patterns) |

### E — Elevation of Privilege
**No applicable threats.** spelunk runs as the user, writes only to user-owned paths.

### D — Denial of Service
**Out of scope.** Local tool; no SLA.

---

## Prompt Injection

Treated separately because it is spelunk-specific:

| Threat | Likelihood | Impact | Mitigation |
|--------|-----------|--------|-----------|
| Indexed source file contains adversarial LLM instructions | Low | Medium | XML delimiter isolation in `ask.rs`; angle-bracket escaping of retrieved context (issue #137) |
| User query contains injection payload | Low | Low | Pre-flight check against known injection patterns (`ask.rs` line 155–174) |

**Residual risk:** The pre-flight only blocks known string patterns. Novel injection payloads in indexed content could influence the local LLM's response. Accepted as low-severity given local-only operation.

---

## Out-of-Scope Threats

- Network-based attacks (spelunk has no open ports)
- Multi-user access control (single-user tool)
- Authentication bypass (no authentication exists)
- Remote code execution via the embedding/LLM server (that server is user-controlled)

---

## Security Requirement Derivations

From this threat model, the following requirements are binding:

1. **No SQL string formatting.** All DB operations use rusqlite parameterised queries.
2. **Secret scanner must run before every DB write of chunk content.** Currently enforced in `parse_phase.rs` lines 195, 249, 340 and `snapshot.rs` line 196.
3. **LLM context must use XML delimiters.** `<code_context>`, `<spec_context>`, `<memory_context>`, `<question>` with angle-bracket escaping of all retrieved content.
4. **Atomic transactions for memory state transitions.** `supersede()` and `insert_with_supersession()` must both be wrapped in BEGIN/COMMIT/ROLLBACK.
5. **CI must gate on `cargo audit` and `cargo deny`.** No merge with unaddressed RUSTSEC advisories (unless explicitly ignored with reason in `audit.toml`).
