# spelunk Benchmarks

Developer-only scripts for measuring whether spelunk improves agent task completion quality and retrieval accuracy versus baseline.

---

## Purpose

These benchmarks answer two questions:

1. **SWE-bench**: Does giving a Claude agent access to `spelunk search` improve its ability to resolve real GitHub issues, compared to an agent with only basic file tools?
2. **CodeSearchNet**: How accurately does spelunk retrieve relevant code for natural-language queries?

Run these before releases or after significant changes to the indexer, chunker, or embedding pipeline.

---

## Prerequisites

- **Docker** — required by the SWE-bench evaluation harness
- **`spelunk` in PATH** — build with `cargo build --release` and add `target/release` to PATH, or install globally
- **`ANTHROPIC_API_KEY`** — set in your environment for SWE-bench agent scripts
- **Python 3.10+**
- **Python dependencies**:

```bash
pip install -r bench/requirements.txt
```

---

## SWE-bench

Evaluates whether spelunk helps a Claude agent fix real GitHub issues.

### How it works

1. The agent is given an issue description and a repository checkout.
2. It uses file tools (and optionally `spelunk search`) to explore the codebase and produce a patch.
3. The patch is written to disk; actual resolution is determined by running the SWE-bench Docker evaluation harness separately (see [SWE-bench repo](https://github.com/princeton-nlp/SWE-bench)).

### Running

```bash
# Baseline (no spelunk)
bash bench/swebench/run.sh --condition baseline --tasks 50

# With spelunk
bash bench/swebench/run.sh --condition spelunk --tasks 50

# Custom model or output dir
bash bench/swebench/run.sh --condition spelunk --model claude-opus-4-5 --out bench/results
```

Results are written to `bench/results/swebench-{condition}-{timestamp}.json`.

### Evaluating resolution

After both runs complete, run the SWE-bench Docker harness on the patches to populate `resolved` and `resolve_rate`. See https://github.com/princeton-nlp/SWE-bench for harness setup instructions.

The pinned task list lives at `bench/swebench/tasks_50.json` — do not change it between paired runs.

---

## CodeSearchNet

Evaluates spelunk's retrieval accuracy on the public CodeSearchNet dataset.

### How it works

1. Downloads the CodeSearchNet dataset from HuggingFace.
2. For each sampled (docstring query, function) pair, runs `spelunk search` and checks whether the ground-truth function appears in the top results.
3. Computes MRR@10, Recall@5, and Recall@10.

### Prerequisites

The repository being evaluated must already be indexed with spelunk:

```bash
spelunk index /path/to/repo
```

### Running

```bash
# Default: 1000 Python samples
bash bench/codesearchnet/run.sh

# Multiple languages, more samples
bash bench/codesearchnet/run.sh --languages python,java,go --samples 2000 --out bench/results/csn-run.json
```

Results are written to `bench/results/codesearchnet-spelunk-{timestamp}.json` (or the path you specify).

---

## Comparing Results

Use `report.py` to print a markdown comparison table across two or more result files:

```bash
python bench/report.py results/run-a.json results/run-b.json
```

Example output:

```
| benchmark           | condition | spelunk_version | resolve_rate | mrr_at_10 | median_tokens | median_wall_s |
|---------------------|-----------|-----------------|--------------|-----------|---------------|---------------|
| swebench-verified   | baseline  | 0.2.1           | 0.12         | —         | 24500         | 142.3         |
| swebench-verified   | spelunk   | 0.2.1           | 0.18         | —         | 31200         | 167.8         |
| codesearchnet       | spelunk   | 0.2.1           | —            | 0.43      | —             | 2.1           |
```

---

## Metrics

| Metric | Benchmark | Meaning |
|--------|-----------|---------|
| `resolve_rate` | SWE-bench | Fraction of tasks where the agent's patch passes all tests in the SWE-bench harness |
| `mrr_at_10` | CodeSearchNet | Mean Reciprocal Rank: average of 1/rank for each query where ground truth appears in top 10 |
| `recall_at_5` | CodeSearchNet | Fraction of queries where ground truth appears in top 5 results |
| `recall_at_10` | CodeSearchNet | Fraction of queries where ground truth appears in top 10 results |
| `median_tokens_per_task` | SWE-bench | Median total tokens (input + output) consumed per task |
| `median_wall_seconds` | Both | Median wall-clock time per task/query in seconds |

Higher `resolve_rate`, `mrr_at_10`, and recall values are better. Token and time costs are secondary — higher cost is acceptable if accuracy improves meaningfully.
