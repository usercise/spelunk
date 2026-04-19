# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| latest (`main`) | Yes |
| older releases | Best-effort patch backport for critical issues |

## Reporting a Vulnerability

**Please do not file public GitHub issues for security vulnerabilities.**

Report security issues privately via GitHub's built-in private vulnerability reporting:
**Security → Report a vulnerability** on the [spelunk repository](https://github.com/usercise/spelunk/security/advisories/new).

### What to include

- Description of the vulnerability and its impact
- Steps to reproduce (spelunk version, OS, minimal reproduction)
- Any suggested fix or relevant code references

### Response SLA

| Severity | Acknowledgement | Patch target |
|----------|:-:|:-:|
| Critical (CVSS ≥ 9.0) | 48 hours | 7 days |
| High (CVSS 7.0–8.9) | 7 days | 30 days |
| Medium / Low | 7 days | Next minor release |

We will credit reporters in the release notes unless you prefer to remain anonymous.

## Scope

spelunk is a **local single-user CLI tool**. It has no network listeners, no
authentication system, and no multi-user access model. The most relevant
security concerns are:

- **Credential leakage** — secrets present in indexed source files being stored
  in the vector index or sent to the local LLM
- **Dependency vulnerabilities** — transitive Rust crate advisories
- **Data integrity** — corruption of the local SQLite index or memory database

Network-based attacks and authentication bypass are out of scope as spelunk
makes no outbound connections except to `127.0.0.1` (a local server the user
controls).

## Security Controls

- Secret scanning runs before any chunk is stored in the index
  (`src/indexer/secrets.rs`)
- `.env*`, `*.pem`, `*.key`, and similar sensitive file patterns are excluded
  from indexing unconditionally
- All database writes use parameterised queries — no SQL string concatenation
- LLM prompts use XML delimiter isolation with angle-bracket escaping of all
  retrieved context
- `cargo audit` and `cargo deny` run in CI and block merges on unaddressed
  advisories

## Security Program

Full security program documentation: [`docs/security/SECURITY-PROGRAM.md`](docs/security/SECURITY-PROGRAM.md)  
Threat model: [`docs/security/THREAT-MODEL.md`](docs/security/THREAT-MODEL.md)
