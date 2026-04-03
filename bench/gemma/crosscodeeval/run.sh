#!/usr/bin/env bash
# Run CrossCodeEval for Gemma — one condition at a time.
#
# Normal usage (spelunk condition against committed baseline):
#   bash bench/gemma/crosscodeeval/run.sh --condition spelunk --repo-path /path/to/repo
#
# Regenerate the committed baseline:
#   bash bench/gemma/crosscodeeval/run.sh --condition baseline --samples 400
#
# Options:
#   --condition    baseline|spelunk          (required)
#   --repo-path    PATH                      path to indexed repo (required for spelunk)
#   --languages    python,typescript         comma-separated (default: python,typescript)
#   --samples      N                         samples per language (default: 200)
#   --model        MODEL                     model name (default: gemma-4-e2b-it)
#   --api-base-url URL                       (default: http://127.0.0.1:1234/v1)
#   --out          FILE                      output path (default: bench/results/...)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REPO_ROOT="$(cd "${BENCH_DIR}/.." && pwd)"
BASELINES_DIR="${REPO_ROOT}/baselines"

usage() {
    grep '^#' "$0" | grep -v '#!/' | sed 's/^# \?//'
    exit 1
}

# Defaults
CONDITION=""
REPO_PATH=""
LANGUAGES="python,typescript"
SAMPLES=200
MODEL="gemma-4-e2b-it"
API_BASE_URL="http://127.0.0.1:1234/v1"
OUT_FILE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --condition)    CONDITION="$2";    shift 2 ;;
        --repo-path)    REPO_PATH="$2";    shift 2 ;;
        --languages)    LANGUAGES="$2";    shift 2 ;;
        --samples)      SAMPLES="$2";      shift 2 ;;
        --model)        MODEL="$2";        shift 2 ;;
        --api-base-url) API_BASE_URL="$2"; shift 2 ;;
        --out)          OUT_FILE="$2";     shift 2 ;;
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

TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
if [[ -z "$OUT_FILE" ]]; then
    mkdir -p "${BENCH_DIR}/results"
    OUT_FILE="${BENCH_DIR}/results/crosscodeeval-${CONDITION}-${TIMESTAMP}.json"
fi

# Compute scaffold_hash from last commit touching bench/
SCAFFOLD_HASH="$(git -C "${REPO_ROOT}" log -1 --format="%H" -- bench/ 2>/dev/null || echo "unknown")"

# Warn if the committed baseline is stale (spelunk condition only)
if [[ "$CONDITION" == "spelunk" ]]; then
    BASELINE_FILE="${BASELINES_DIR}/crosscodeeval-gemma-4-e2b-it-baseline.json"
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

EXTRA_ARGS=()
if [[ -n "$REPO_PATH" ]]; then
    EXTRA_ARGS+=(--repo-path "$REPO_PATH")
fi

echo "Condition:    ${CONDITION}"
echo "Languages:    ${LANGUAGES}"
echo "Samples:      ${SAMPLES} per language"
echo "Model:        ${MODEL}"
echo "API base:     ${API_BASE_URL}"
echo "Bench base:   ${BENCH_DIR}"
echo "Output:       ${OUT_FILE}"
echo ""

uv run --with-requirements "${BENCH_DIR}/requirements.txt" \
    python3 "${SCRIPT_DIR}/evaluate.py" \
    --condition "$CONDITION" \
    --languages "$LANGUAGES" \
    --samples "$SAMPLES" \
    --model "$MODEL" \
    --api-base-url "$API_BASE_URL" \
    --scaffold-hash "$SCAFFOLD_HASH" \
    --out "$OUT_FILE" \
    ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}

# If spelunk condition and baseline exists, print comparison
if [[ "$CONDITION" == "spelunk" && -f "${BASELINES_DIR}/crosscodeeval-gemma-4-e2b-it-baseline.json" ]]; then
    echo ""
    echo "=== Comparison vs committed baseline ==="
    uv run --with-requirements "${BENCH_DIR}/requirements.txt" \
    python3 "${BENCH_DIR}/report.py" \
        "${BASELINES_DIR}/crosscodeeval-gemma-4-e2b-it-baseline.json" \
        "$OUT_FILE"
fi
