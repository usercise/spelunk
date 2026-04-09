#!/usr/bin/env bash
# Run the SWE-bench agent benchmark.
#
# This script drives agent_baseline.py or agent_with_spelunk.py over the
# pinned task list (tasks_50.json) and writes a result JSON file.
#
# NOTE: "resolved" and "resolve_rate" in the output are always 0/0.0 —
# actual resolution requires running the SWE-bench Docker evaluation harness
# against the patches produced here. See:
#   https://github.com/princeton-nlp/SWE-bench
#
# Usage:
#   bash bench/swebench/run.sh --condition baseline|spelunk [options]
#
# Options:
#   --condition  baseline|spelunk  (required)
#   --tasks      N                 number of tasks to run (default: 50)
#   --model      MODEL             Claude model ID (default: claude-sonnet-4-6)
#   --out        DIR               output directory (default: bench/results)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

usage() {
    grep '^#' "$0" | grep -v '#!/' | sed 's/^# \?//'
    exit 1
}

# Defaults
CONDITION=""
TASKS=50
MODEL="claude-sonnet-4-6"
OUT_DIR="${BENCH_DIR}/results"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --condition) CONDITION="$2"; shift 2 ;;
        --tasks)     TASKS="$2";     shift 2 ;;
        --model)     MODEL="$2";     shift 2 ;;
        --out)       OUT_DIR="$2";   shift 2 ;;
        -h|--help)   usage ;;
        *) echo "Unknown argument: $1" >&2; usage ;;
    esac
done

if [[ -z "$CONDITION" ]]; then
    echo "Error: --condition is required." >&2
    usage
fi

if [[ "$CONDITION" != "baseline" && "$CONDITION" != "spelunk" ]]; then
    echo "Error: --condition must be 'baseline' or 'spelunk'." >&2
    exit 1
fi

if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
    echo "Error: ANTHROPIC_API_KEY is not set." >&2
    exit 1
fi

# Select the agent script
if [[ "$CONDITION" == "baseline" ]]; then
    AGENT_SCRIPT="${SCRIPT_DIR}/agent_baseline.py"
else
    AGENT_SCRIPT="${SCRIPT_DIR}/agent_with_spelunk.py"
fi

TASKS_FILE="${SCRIPT_DIR}/tasks_50.json"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT_FILE="${OUT_DIR}/swebench-${CONDITION}-${TIMESTAMP}.json"

mkdir -p "$OUT_DIR"

# Get spelunk version (may not be available if not installed)
SPELUNK_VERSION="unknown"
if command -v spelunk &>/dev/null; then
    SPELUNK_VERSION="$(spelunk --version 2>&1 | head -1 | awk '{print $NF}')"
fi

# Read task IDs from the JSON array (requires python3)
mapfile -t ALL_TASKS < <(python3 -c "
import json, sys
with open('${TASKS_FILE}') as f:
    tasks = json.load(f)
for t in tasks[:${TASKS}]:
    print(t)
")

TASK_COUNT="${#ALL_TASKS[@]}"
echo "Condition:      ${CONDITION}"
echo "Model:          ${MODEL}"
echo "Tasks to run:   ${TASK_COUNT}"
echo "Output file:    ${OUT_FILE}"
echo ""

# Accumulate per-task results
TASK_RESULTS=()
TOTAL_TOKENS=()
WALL_SECONDS=()

for i in "${!ALL_TASKS[@]}"; do
    TASK_ID="${ALL_TASKS[$i]}"
    IDX=$((i + 1))
    echo "Running task ${TASK_ID} (${IDX}/${TASK_COUNT})..."

    # SWE-bench tasks require a repo checkout. For the purposes of this script,
    # we expect a directory at bench/repos/<task_id> or a REPOS_DIR env var.
    # If neither exists, record a skipped result and continue.
    REPOS_BASE="${REPOS_DIR:-${BENCH_DIR}/repos}"
    REPO_PATH="${REPOS_BASE}/${TASK_ID}"

    if [[ ! -d "$REPO_PATH" ]]; then
        echo "  Skipping: no repo checkout found at ${REPO_PATH}"
        TASK_RESULTS+=("{\"task_id\": \"${TASK_ID}\", \"resolved\": false, \"turns\": 0, \"input_tokens\": 0, \"output_tokens\": 0, \"wall_seconds\": 0, \"skipped\": true}")
        continue
    fi

    # The issue file should be at bench/repos/<task_id>/ISSUE.txt or ISSUE env var
    ISSUE_FILE="${REPO_PATH}/ISSUE.txt"
    if [[ -f "$ISSUE_FILE" ]]; then
        ISSUE_TEXT="$(cat "$ISSUE_FILE")"
    else
        ISSUE_TEXT="See task ${TASK_ID} in the SWE-bench dataset."
    fi

    # Run the agent; capture JSON output
    RESULT="$(python3 "$AGENT_SCRIPT" \
        --task-id "$TASK_ID" \
        --repo-path "$REPO_PATH" \
        --issue "$ISSUE_TEXT" \
        --model "$MODEL" \
        2>/dev/null)" || {
        echo "  Agent failed for ${TASK_ID}" >&2
        RESULT="{\"task_id\": \"${TASK_ID}\", \"resolved\": false, \"turns\": 0, \"input_tokens\": 0, \"output_tokens\": 0, \"wall_seconds\": 0, \"error\": true}"
    }

    TASK_RESULTS+=("$RESULT")

    # Extract metrics for summary (requires python3)
    TOKENS="$(echo "$RESULT" | python3 -c "import json,sys; r=json.load(sys.stdin); print(r.get('input_tokens',0)+r.get('output_tokens',0))")"
    SECS="$(echo "$RESULT" | python3 -c "import json,sys; r=json.load(sys.stdin); print(r.get('wall_seconds',0))")"
    TOTAL_TOKENS+=("$TOKENS")
    WALL_SECONDS+=("$SECS")

    echo "  Done: tokens=${TOKENS} wall=${SECS}s"
done

echo ""
echo "All tasks complete. Writing results..."

# Compute medians and summary via python3
python3 - <<PYEOF
import json, sys, statistics
from pathlib import Path

task_results = [json.loads(r) for r in [
$(printf '    %s,\n' "${TASK_RESULTS[@]}")
]]

tokens_list = [
    r.get("input_tokens", 0) + r.get("output_tokens", 0)
    for r in task_results
    if not r.get("skipped") and not r.get("error")
]
wall_list = [
    r.get("wall_seconds", 0)
    for r in task_results
    if not r.get("skipped") and not r.get("error")
]

median_tokens = round(statistics.median(tokens_list), 1) if tokens_list else 0
median_wall   = round(statistics.median(wall_list), 2)   if wall_list   else 0

# resolved / resolve_rate are always 0 here — set them via the SWE-bench harness
output = {
    "run_id":                 "${TIMESTAMP}",
    "benchmark":              "swebench-verified",
    "condition":              "${CONDITION}",
    "model":                  "${MODEL}",
    "spelunk_version":        "${SPELUNK_VERSION}",
    "tasks_run":              len(task_results),
    "resolved":               0,
    "resolve_rate":           0.0,
    "median_tokens_per_task": median_tokens,
    "median_wall_seconds":    median_wall,
    # NOTE: To obtain real resolve_rate, run the SWE-bench Docker evaluation
    # harness on the patches produced in bench/repos/<task_id>. See:
    # https://github.com/princeton-nlp/SWE-bench
    "tasks": task_results,
}

out_path = Path("${OUT_FILE}")
out_path.parent.mkdir(parents=True, exist_ok=True)
out_path.write_text(json.dumps(output, indent=2))
print(f"Results written to: ${OUT_FILE}")
print(f"Tasks run:          {len(task_results)}")
print(f"Median tokens:      {median_tokens}")
print(f"Median wall sec:    {median_wall}")
PYEOF
