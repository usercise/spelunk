# Security Review Action Plan

This action plan is designed for a developer agent to execute sequentially, prioritizing immediate safety risks (Output Sanitization) followed by preventative measures (Secret Scanning) and architectural hardening (Prompt Injection mitigation).

---

### **Phase 1: Immediate Safety & Input Hardening**
*Focus: Prevent terminal exploits and resource exhaustion.*

1.  **Task 1.1: Implement Output Sanitization**
    *   **Goal:** Prevent LLM-generated ANSI escape sequences from compromising the user's terminal.
    *   **Action:** Add a utility function in `src/utils.rs` (or similar) that strips non-printable and ANSI control characters from strings.
    *   **Integration:** Wrap the LLM token stream in `src/cli/commands.rs` (the `ask` function) with this sanitizer before printing to `stdout`.

2.  **Task 1.2: Enforce Bounds on SQL & CLI Arguments**
    *   **Goal:** Prevent Denial of Service (DoS) via resource-heavy database queries.
    *   **Action:** In `src/cli/mod.rs`, add validation logic to `SearchArgs` and `AskArgs` to cap `limit` and `context_chunks` (e.g., max 100).
    *   **Action:** Update `src/storage/db.rs` to ensure `k` is validated before being formatted into the vector search query.

---

### **Phase 2: Data Privacy & Secret Prevention**
*Focus: Ensure the RAG index does not become a source of credential leakage.*

3.  **Task 2.1: Integrate Secret Scanning in Indexer**
    *   **Goal:** Automatically skip chunks that appear to contain API keys or secrets.
    *   **Action:** Add a "Secret Scanner" module in `src/indexer/mod.rs` using high-entropy checks or common regex patterns (AWS keys, Bearer tokens, etc.).
    *   **Integration:** Update `src/indexer/parser.rs` to run the scanner on each `Chunk`. If a secret is detected, drop the chunk and log a warning (without logging the secret).

4.  **Task 2.2: Hardening File Traversal**
    *   **Goal:** Ensure the indexer strictly respects exclusion rules.
    *   **Action:** Audit `WalkBuilder` configuration in `src/cli/commands.rs`. Ensure it explicitly ignores `.git`, `.env`, and other sensitive directories by default, even if `.gitignore` is missing.

---

### **Phase 3: Prompt Integrity & Model Safety**
*Focus: Mitigate direct and indirect prompt injection.*

5.  **Task 3.1: Structural Prompt Hardening**
    *   **Goal:** Move away from simple string concatenation for prompts.
    *   **Action:** Refactor `src/cli/commands.rs` to use a more robust template for the `ask` system prompt. Explicitly delimit the "Code Context" from the "User Question" using unique, hard-to-spoof separators (e.g., XML-style tags or random UUID delimiters).
    *   **Action:** Add a "Pre-flight" check to ensure the user's question doesn't contain common injection keywords (e.g., "ignore previous instructions").

---

### **Phase 4: Automated Security Tooling (CI/CD)**
*Focus: Continuous monitoring and deep vulnerability discovery.*

6.  **Task 4.1: Integrate SAST & Audit Tools**
    *   **Action:** Create a GitHub Action (or local script) that runs `cargo-audit` on every push to check for vulnerable dependencies.
    *   **Action:** Add `cargo-deny` to the workflow to prevent "cratesquatting" and ensure license compliance.

7.  **Task 4.2: Implement Fuzz Testing**
    *   **Goal:** Find crashes or memory issues in the code parsers.
    *   **Action:** Create a `fuzz` target using `cargo-fuzz`. Focus specifically on the `SourceParser::parse` function in `src/indexer/parser.rs`.
    *   **Action:** Fuzz with a corpus of both valid and intentionally malformed code files in multiple supported languages.

---

### **Success Criteria for the Agent**
- [ ] `ca ask` output is stripped of all ANSI control characters.
- [ ] Attempting to index a file containing a mock AWS key results in that chunk being skipped.
- [ ] `cargo audit` passes with zero vulnerabilities.
- [ ] `ca search --limit 999999` is gracefully capped to a safe maximum.
- [ ] Fuzzer runs for 10 minutes without discovering a crash in the `tree-sitter` integration.
