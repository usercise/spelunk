# spelunk Benchmarks

Developer-only scripts for measuring whether spelunk improves agent task completion and retrieval accuracy. Run manually before releases or after significant changes to the indexer, chunker, or embedding pipeline.

---

## Overview

| Tier | Benchmark | Model | Cadence |
|------|-----------|-------|---------|
| **Primary** | CrossCodeEval | gemma-4-e2b-it (local) | Pre-release |
| **Primary** | SWE-bench (local) | gemma-4-e2b-it (local) | Pre-release |
| Secondary | SWE-bench (Claude) | claude-sonnet-4-6 | Major releases only |
| Retrieval | CodeSearchNet | model-agnostic | On indexer/chunker changes |

---

## Committed baselines

Baseline results (no-spelunk condition) live in `baselines/` at the repo root and are committed to git. Normal runs execute only the spelunk condition and auto-compare against the baseline. See `baselines/README.md` for when and how to regenerate.

---

## Prerequisites

```bash
uv pip install -r bench/requirements.txt
```

**For Gemma benchmarks (primary):**
- Local OpenAI-compatible server at `http://127.0.0.1:1234` with `gemma-4-e2b-it` loaded
- `spelunk` in PATH (build: `cargo build --release`)
- Docker (SWE-bench only)

**For Claude benchmarks (secondary):**
- `ANTHROPIC_API_KEY` in environment
- Docker

---

## CrossCodeEval

Measures whether `spelunk_search` helps complete lines that require symbols from other files. Each task presents a code file truncated at a completion point; the ground truth requires cross-file understanding.

```bash
# Spelunk condition — compares against committed baseline automatically
bash bench/gemma/crosscodeeval/run.sh --condition spelunk --repo-path /path/to/indexed/repo

# Regenerate baseline (run once after scaffold changes)
bash bench/gemma/crosscodeeval/run.sh --condition baseline --samples 400
```

**Options:** `--languages python,typescript` · `--samples 200` · `--model gemma-4-e2b-it` · `--api-base-url http://127.0.0.1:1234/v1`

**Metrics:** `exact_match`, `edit_similarity`, `identifier_recall`

---

## SWE-bench (local model)

Measures whether `spelunk_search` helps fix real GitHub issues. Uses the same 50-task slice as the Claude variant so results are directly comparable.

```bash
# Spelunk condition — compares against committed baseline automatically
bash bench/gemma/swebench_local/run.sh --condition spelunk

# Regenerate baseline
bash bench/gemma/swebench_local/run.sh --condition baseline --tasks 50
```

Repo checkouts are expected at `bench/repos/<task_id>/` or `$REPOS_DIR/<task_id>/`. Each directory should contain an `ISSUE.txt`. The `resolved` field in results is always `0` — run the [SWE-bench Docker harness](https://github.com/princeton-nlp/SWE-bench) on the patches to determine real resolution rates.

**Metrics:** `resolve_rate` (via harness), `median_tokens_per_task`, `median_wall_seconds`

---

## SWE-bench (Claude) — secondary

```bash
bash bench/swebench/run.sh --condition baseline --tasks 50
bash bench/swebench/run.sh --condition spelunk  --tasks 50
```

Requires `ANTHROPIC_API_KEY`. Results go to `bench/results/swebench-{condition}-{timestamp}.json`.

---

## CodeSearchNet — retrieval quality

Model-agnostic. Measures how accurately `spelunk search` retrieves relevant code for natural-language queries.

```bash
bash bench/codesearchnet/run.sh --languages python --samples 1000
```

The target repo must be indexed before running: `spelunk index /path/to/repo`.

**Metrics:** `mrr_at_10`, `recall_at_5`, `recall_at_10`

---

## Comparing results

```bash
python bench/report.py baselines/crosscodeeval-gemma-4-e2b-it-baseline.json bench/results/crosscodeeval-spelunk-<ts>.json
```

The spelunk run scripts print this comparison automatically when a baseline exists. You can also compare any two result files directly.

Example output:
```
| benchmark      | condition | model                    | exact_match | edit_sim | id_recall | med_wall_s |
|----------------|-----------|--------------------------|-------------|----------|-----------|------------|
| crosscodeeval  | baseline  | gemma-4-e2b-it (local)  | 0.170       | 0.481    | 0.224     | 3.800      |
| crosscodeeval  | spelunk   | gemma-4-e2b-it (local)  | 0.210       | 0.541    | 0.291     | 4.100      |
```

---

## Metrics reference

| Metric | Benchmark | Meaning |
|--------|-----------|---------|
| `exact_match` | CrossCodeEval | Fraction of completions that exactly match ground truth |
| `edit_similarity` | CrossCodeEval | Average SequenceMatcher ratio between prediction and ground truth |
| `identifier_recall` | CrossCodeEval | Fraction of identifiers in ground truth that appear in the prediction |
| `resolve_rate` | SWE-bench | Fraction of tasks where the patch passes all tests (set by harness) |
| `mrr_at_10` | CodeSearchNet | Mean Reciprocal Rank at 10 |
| `recall_at_5/10` | CodeSearchNet | Fraction of queries where ground truth appears in top 5/10 results |
| `median_tokens_per_task` | SWE-bench | Median total tokens per task |
| `median_wall_seconds` | All | Median wall-clock seconds per task/query |
