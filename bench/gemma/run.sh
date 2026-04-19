#!/usr/bin/env bash
# Orchestrate local Gemma benchmarks.
#
# Usage:
#   bash bench/gemma/run.sh --suite crosscodeeval --condition spelunk --repo-path /path/to/repo
#   bash bench/gemma/run.sh --suite swebench_local --condition spelunk
#   bash bench/gemma/run.sh --suite all --condition spelunk --repo-path /path/to/repo
#
# Options:
#   --suite        crosscodeeval|swebench_local|all   (required)
#   --condition    baseline|spelunk                   (required)
#   --repo-path    PATH      path to indexed repo (required for spelunk condition)
#   --samples      N         CrossCodeEval samples per language (default: 200)
#   --tasks        N         SWE-bench tasks (default: 50)
#   --model        MODEL     (default: gemma-4-e2b-it)
#   --api-base-url URL       (default: http://127.0.0.1:1234/v1)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
    grep '^#' "$0" | grep -v '#!/' | sed 's/^# \?//'
    exit 1
}

SUITE=""
CONDITION=""
REPO_PATH=""
SAMPLES=200
TASKS=50
MODEL="gemma-4-e2b-it"
API_BASE_URL="http://127.0.0.1:1234/v1"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --suite)        SUITE="$2";        shift 2 ;;
        --condition)    CONDITION="$2";    shift 2 ;;
        --repo-path)    REPO_PATH="$2";    shift 2 ;;
        --samples)      SAMPLES="$2";      shift 2 ;;
        --tasks)        TASKS="$2";        shift 2 ;;
        --model)        MODEL="$2";        shift 2 ;;
        --api-base-url) API_BASE_URL="$2"; shift 2 ;;
        -h|--help)      usage ;;
        *) echo "Unknown argument: $1" >&2; usage ;;
    esac
done

if [[ -z "$SUITE" || -z "$CONDITION" ]]; then
    echo "Error: --suite and --condition are required." >&2; usage
fi

COMMON_ARGS=(--condition "$CONDITION" --model "$MODEL" --api-base-url "$API_BASE_URL")
REPO_ARGS=()
if [[ -n "$REPO_PATH" ]]; then
    REPO_ARGS+=(--repo-path "$REPO_PATH")
fi

run_crosscodeeval() {
    echo "=== CrossCodeEval ==="
    bash "${SCRIPT_DIR}/crosscodeeval/run.sh" \
        "${COMMON_ARGS[@]}" \
        --samples "$SAMPLES" \
        "${REPO_ARGS[@]}"
}

run_swebench_local() {
    echo "=== SWE-bench local ==="
    bash "${SCRIPT_DIR}/swebench_local/run.sh" \
        "${COMMON_ARGS[@]}" \
        --tasks "$TASKS"
}

case "$SUITE" in
    crosscodeeval)  run_crosscodeeval ;;
    swebench_local) run_swebench_local ;;
    all)
        run_crosscodeeval
        echo ""
        run_swebench_local
        ;;
    *) echo "Error: unknown suite '${SUITE}'. Must be crosscodeeval, swebench_local, or all." >&2; exit 1 ;;
esac
