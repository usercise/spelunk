#!/usr/bin/env bash
# Run SWE-bench with the local Gemma model.
#
# Uses the same tasks_50.json as the Claude variant so results are directly
# comparable. Repo checkouts are expected at bench/repos/<task_id>/ or
# at the path given by REPOS_DIR.
#
# NOTE: "resolved" and "resolve_rate" in output are always 0 — actual
# resolution requires the SWE-bench Docker harness. See:
#   https://github.com/princeton-nlp/SWE-bench
#
# Usage:
#   bash bench/gemma/swebench_local/run.sh --condition baseline|spelunk [options]
#
# Options:
#   --condition    baseline|spelunk          (required)
#   --tasks        N                         number of tasks (default: 50)
#   --model        MODEL                     (default: gemma-4-e2b-it)
#   --api-base-url URL                       (default: http://127.0.0.1:1234/v1)
#   --out          DIR                       output directory (default: bench/results)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REPO_ROOT="$(cd "${BENCH_DIR}/.." && pwd)"
BASELINES_DIR="${REPO_ROOT}/baselines"
TASKS_FILE="${BENCH_DIR}/swebench/tasks_50.json"

usage() {
    grep '^#' "$0" | grep -v '#!/' | sed 's/^# \?//'
    exit 1
}

CONDITION=""
TASKS=50
MODEL="gemma-4-e2b-it"
API_BASE_URL="http://127.0.0.1:1234/v1"
OUT_DIR="${BENCH_DIR}/results"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --condition)    CONDITION="$2";    shift 2 ;;
        --tasks)        TASKS="$2";        shift 2 ;;
        --model)        MODEL="$2";        shift 2 ;;
        --api-base-url) API_BASE_URL="$2"; shift 2 ;;
        --out)          OUT_DIR="$2";      shift 2 ;;
        -h|--help)      usage ;;
        *) echo "Unknown argument: $1" >&2; usage ;;
    esac
done

if [[ -z "$CONDITION" ]]; then
    echo "Error: --condition is required." >&2; usage
fi
if [[ "$CONDITION" != "baseline" && "$CONDITION" != "spelunk" ]]; then
    echo "Error: --condition must be 'baseline' or 'spelunk'." >&2; exit 1
fi

if [[ "$CONDITION" == "baseline" ]]; then
    AGENT_SCRIPT="${SCRIPT_DIR}/agent_baseline.py"
else
    AGENT_SCRIPT="${SCRIPT_DIR}/agent_spelunk.py"
fi

TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT_FILE="${OUT_DIR}/swebench-local-${CONDITION}-${TIMESTAMP}.json"
mkdir -p "$OUT_DIR"

SPELUNK_VERSION="unknown"
if command -v spelunk &>/dev/null; then
    SPELUNK_VERSION="$(spelunk --version 2>&1 | head -1 | awk '{print $NF}')"
fi

SCAFFOLD_HASH="$(git -C "${REPO_ROOT}" log -1 --format="%H" -- bench/ 2>/dev/null || echo "unknown")"

# Warn if committed baseline is stale (spelunk condition only)
if [[ "$CONDITION" == "spelunk" ]]; then
    BASELINE_FILE="${BASELINES_DIR}/swebench-local-gemma-4-e2b-it-baseline.json"
    if [[ -f "$BASELINE_FILE" ]]; then
        BASELINE_HASH="$(python3 -c "import json; d=json.load(open('${BASELINE_FILE}')); print(d.get('scaffold_hash','unknown'))")"
        if [[ "$BASELINE_HASH" != "$SCAFFOLD_HASH" && "$BASELINE_HASH" != "unknown" ]]; then
            echo "Warning: committed baseline scaffold_hash (${BASELINE_HASH}) does not match"
            echo "         current bench/ HEAD (${SCAFFOLD_HASH})."
            echo "         Consider regenerating: bash $0 --condition baseline"
            echo ""
        fi
    else
        echo "Warning: no committed baseline found at ${BASELINE_FILE}."
        echo "         Run with --condition baseline first, then commit the result."
        echo ""
    fi
fi

ALL_TASKS=()
while IFS= read -r line; do
    ALL_TASKS+=("$line")
done < <(python3 -c "
import json
with open('${TASKS_FILE}') as f:
    tasks = json.load(f)
for t in tasks[:${TASKS}]:
    print(t)
")

TASK_COUNT="${#ALL_TASKS[@]}"
echo "Condition:    ${CONDITION}"
echo "Model:        ${MODEL}"
echo "API base:     ${API_BASE_URL}"
echo "Tasks:        ${TASK_COUNT}"
echo "Output:       ${OUT_FILE}"
echo ""

TASK_RESULTS=()

for i in "${!ALL_TASKS[@]}"; do
    TASK_ID="${ALL_TASKS[$i]}"
    IDX=$((i + 1))
    echo "Running task ${TASK_ID} (${IDX}/${TASK_COUNT})..."

    REPOS_BASE="${REPOS_DIR:-${BENCH_DIR}/repos}"
    REPO_PATH="${REPOS_BASE}/${TASK_ID}"

    if [[ ! -d "$REPO_PATH" ]]; then
        echo "  Skipping: no repo checkout at ${REPO_PATH}"
        TASK_RESULTS+=("{\"task_id\": \"${TASK_ID}\", \"resolved\": false, \"turns\": 0, \"input_tokens\": 0, \"output_tokens\": 0, \"wall_seconds\": 0, \"skipped\": true}")
        continue
    fi

    ISSUE_FILE="${REPO_PATH}/ISSUE.txt"
    ISSUE_TEXT="$(cat "$ISSUE_FILE" 2>/dev/null || echo "See task ${TASK_ID} in the SWE-bench dataset.")"

    RESULT="$(uv run --with-requirements "${BENCH_DIR}/requirements.txt" \
        python3 "$AGENT_SCRIPT" \
        --task-id "$TASK_ID" \
        --repo-path "$REPO_PATH" \
        --issue "$ISSUE_TEXT" \
        --model "$MODEL" \
        --api-base-url "$API_BASE_URL" \
        2>/dev/null)" || {
        echo "  Agent failed for ${TASK_ID}" >&2
        RESULT="{\"task_id\": \"${TASK_ID}\", \"resolved\": false, \"turns\": 0, \"input_tokens\": 0, \"output_tokens\": 0, \"wall_seconds\": 0, \"error\": true}"
    }

    TASK_RESULTS+=("$RESULT")
    TOKENS="$(echo "$RESULT" | python3 -c "import json,sys; r=json.load(sys.stdin); print(r.get('input_tokens',0)+r.get('output_tokens',0))")"
    SECS="$(echo "$RESULT" | python3 -c "import json,sys; r=json.load(sys.stdin); print(r.get('wall_seconds',0))")"
    echo "  Done: tokens=${TOKENS} wall=${SECS}s"
done

echo ""
echo "All tasks complete. Writing results..."

python3 - <<PYEOF
import json, statistics
from pathlib import Path

task_results = [json.loads(r) for r in [
$(printf "    '%s',\n" "${TASK_RESULTS[@]}")
]]

ran = [r for r in task_results if not r.get("skipped") and not r.get("error")]
tokens_list = [r.get("input_tokens", 0) + r.get("output_tokens", 0) for r in ran]
wall_list   = [r.get("wall_seconds", 0) for r in ran]

output = {
    "run_id":                 "${TIMESTAMP}",
    "benchmark":              "swebench-verified",
    "condition":              "${CONDITION}",
    "model":                  "${MODEL}",
    "model_source":           "local",
    "api_base_url":           "${API_BASE_URL}",
    "spelunk_version":        "${SPELUNK_VERSION}",
    "scaffold_hash":          "${SCAFFOLD_HASH}",
    "tasks_run":              len(task_results),
    "resolved":               0,
    "resolve_rate":           0.0,
    "median_tokens_per_task": round(statistics.median(tokens_list), 1) if tokens_list else 0,
    "median_wall_seconds":    round(statistics.median(wall_list), 2)   if wall_list   else 0,
    "tasks": task_results,
}

out_path = Path("${OUT_FILE}")
out_path.parent.mkdir(parents=True, exist_ok=True)
out_path.write_text(json.dumps(output, indent=2))
print(f"Results written to: ${OUT_FILE}")
PYEOF

# Print comparison if spelunk condition and baseline exists
if [[ "$CONDITION" == "spelunk" && -f "${BASELINES_DIR}/swebench-local-gemma-4-e2b-it-baseline.json" ]]; then
    echo ""
    echo "=== Comparison vs committed baseline ==="
    python3 "${BENCH_DIR}/report.py" \
        "${BASELINES_DIR}/swebench-local-gemma-4-e2b-it-baseline.json" \
        "$OUT_FILE"
fi
