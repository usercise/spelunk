# baselines/

Committed baseline results (no-spelunk condition) for the local Gemma benchmarks.

Keeping these outside `bench/` is intentional: the scaffold hash is computed from
`bench/` only, so committing a new baseline here does not change that hash or
spuriously invalidate itself.

## Files

| File | Benchmark | Model |
|------|-----------|-------|
| `repobench-gemma-4-e2b-it-baseline.json` | RepoBench-Python (cross_file_first) | gemma-4-e2b-it |
| `swebench-local-gemma-4-e2b-it-baseline.json` | SWE-bench (local) | gemma-4-e2b-it |

## When to regenerate

Re-run and commit a new baseline when any of these change:

1. **The model** — different version or quantization of `gemma-4-e2b-it`
2. **The agent scaffold** — system prompt, tool definitions, or `max_turns` in
   `bench/gemma/swebench_local/` or `bench/gemma/crosscodeeval/`
3. **The task/sample set** — `bench/swebench/tasks_50.json` or sample seed/count

Each baseline JSON includes a `scaffold_hash` (last git commit of `bench/`).
The run scripts warn automatically when this hash no longer matches HEAD.

## Regenerating

```bash
# RepoBench baseline (400 samples, cross_file_first split)
bash bench/gemma/crosscodeeval/run.sh --condition baseline --samples 400
# Review the output, then:
cp bench/results/repobench-baseline-<timestamp>.json baselines/repobench-gemma-4-e2b-it-baseline.json
git add baselines/repobench-gemma-4-e2b-it-baseline.json
git commit -m "bench: update RepoBench baseline"

# SWE-bench local baseline (50 tasks)
bash bench/gemma/swebench_local/run.sh --condition baseline --tasks 50
cp bench/results/swebench-local-baseline-<timestamp>.json baselines/swebench-local-gemma-4-e2b-it-baseline.json
git add baselines/swebench-local-gemma-4-e2b-it-baseline.json
git commit -m "bench: update SWE-bench local baseline"
```
