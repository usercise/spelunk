# spelunk Security Program

**Framework:** OWASP SAMM v2  
**Target maturity:** Level 1 across all 15 practices (pre-launch baseline)  
**Aspirational:** Level 2 in Secure Build and Security Testing by v1.0  
**Review cadence:** Each major release milestone

---

## Threat Model Summary

spelunk is a **local single-user developer CLI**. It has no network listeners,
no authentication layer, and no multi-tenancy. The relevant threats are:

| Threat | Likelihood | Impact | Controls |
|--------|-----------|--------|----------|
| Credential leakage into vector index | Medium | High | `secrets.rs` scanner; file exclusion in `ignore` traversal |
| Prompt injection via indexed source code | Low | Medium | XML delimiters in `ask.rs`; angle-bracket escaping in context |
| Data integrity corruption (memory DB) | Low | Medium | Atomic transactions in `storage/memory.rs` |
| Dependency vulnerability | Medium | Medium | `cargo audit` + `cargo deny` in CI |
| Supply chain compromise | Low | High | `cargo deny` license/source policy; `Cargo.lock` committed |

Out of scope: network attacks, multi-user access control, authentication bypass.
Full threat model: `docs/security/THREAT-MODEL.md`.

---

## SAMM v2 Posture

### Current State (April 2026)

| Business Function | Practice | Current Level | Target Level |
|-------------------|----------|:---:|:---:|
| **Governance** | Strategy & Metrics | 1 | 1 |
| | Policy & Compliance | 0 | 1 |
| | Education & Guidance | 1 | 1 |
| **Design** | Threat Assessment | 1 | 1 |
| | Security Requirements | 0 | 1 |
| | Secure Architecture | 1 | 1 |
| **Implementation** | Secure Build | 2 | 2 |
| | Secure Deployment | 1 | 1 |
| | Defect Management | 1 | 1 |
| **Verification** | Architecture Assessment | 1 | 1 |
| | Requirements-driven Testing | 1 | 1 |
| | Security Testing | 1 | 2 |
| **Operations** | Incident Management | 0 | 1 |
| | Environment Management | 2 | 2 |
| | Operational Management | 1 | 1 |

### Gaps to Close Before Launch

1. **Policy & Compliance L1** — Publish a `SECURITY.md` with responsible disclosure process and a brief secure coding policy reference in `CLAUDE.md`. Owned by: Docs Writer.
2. **Security Requirements L1** — Define a minimal security acceptance checklist for issues and PRs (secret handling, SQL parameterisation, input validation at boundaries). Owned by: Architect.
3. **Incident Management L1** — `SECURITY.md` must include a private vulnerability reporting contact and a defined response SLA (acknowledge within 7 days, patch within 30 for critical). Owned by: Docs Writer.

---

## Security Controls Inventory

### Build-time controls
| Control | Where | CI gate? |
|---------|-------|----------|
| Secret scanning before indexing | `src/indexer/secrets.rs` | No (runtime) |
| Dependency advisory scan | `cargo audit` | Yes — blocks merge |
| Dependency license/source policy | `cargo deny` | Yes — blocks merge |
| Static analysis | `cargo clippy -D warnings` | Yes — blocks merge |

### Design controls
| Control | Where |
|---------|-------|
| Parameterised SQL (no string formatting) | All `src/storage/*.rs` |
| XML delimiter isolation for LLM prompts | `src/cli/cmd/ask.rs` |
| Angle-bracket escaping in RAG context | `src/cli/cmd/ask.rs` (issue #137) |
| Atomic transactions for memory state | `src/storage/memory.rs` |

### Operational controls
| Control | Where |
|---------|-------|
| `.env*`, `*.pem`, `*.key` excluded from indexing | `src/cli/cmd/index/mod.rs` |
| RUSTSEC advisory monitoring | `audit.toml` |

---

## Secure Development Lifecycle Touchpoints

### Per-feature (every GitHub issue)
- Architect includes security acceptance criteria in the issue body
- Implementer runs `cargo audit` before every commit
- Test Engineer writes at least one adversarial/boundary test per security-sensitive path
- QA Reviewer checks for SQL string concatenation, unsanitised input, secret patterns

### Per-PR
- QA Reviewer runs the security checklist (see `agent-personas/qa-reviewer.md`)
- CI must pass: `cargo clippy`, `cargo audit`, `cargo deny`

### Per-release
- Full `cargo audit` clean (no unignored advisories)
- Re-run secret scanning patterns against the test fixture corpus
- Update `SAMM-POSTURE.md` with any practice level changes
- Check `SECURITY.md` contact details are still valid

---

## Responsible Disclosure

See `SECURITY.md` at the repo root.

---

## References

- [OWASP SAMM v2](https://owaspsamm.org/model/)
- [OWASP Top 10:2025 — Establishing a Modern AppSec Program](https://owasp.org/Top10/2025/0x03_2025-Establishing_a_Modern_Application_Security_Program/)
- [OWASP ASVS](https://owasp.org/www-project-application-security-verification-standard/) (L1 subset relevant to local CLI tools)
- `docs/security/THREAT-MODEL.md`
- `agent-comms/PROTOCOL.md`
