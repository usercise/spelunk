#!/usr/bin/env bash
# Fetch SWE-bench task metadata and clone repos at the correct base commits.
#
# Each task directory under REPOS_DIR will contain the repo source tree at the
# pre-fix commit, plus an ISSUE.txt with the problem statement.
#
# Usage:
#   bash bench/setup_repos.sh [options]
#
# Options:
#   --tasks-file FILE   path to tasks JSON array  (default: bench/swebench/tasks_50.json)
#   --tasks N           only set up first N tasks  (default: all)
#   --repos-dir DIR     checkout root              (default: bench/repos)
#   --dataset SLUG      HuggingFace dataset        (default: princeton-nlp/SWE-bench_Verified)
#   -h|--help

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

TASKS_FILE="${SCRIPT_DIR}/swebench/tasks_50.json"
TASKS=0   # 0 = all
REPOS_DIR="${SCRIPT_DIR}/repos"
DATASET="princeton-nlp/SWE-bench_Verified"

usage() {
    grep '^#' "$0" | grep -v '#!/' | sed 's/^# \?//'
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tasks-file) TASKS_FILE="$2"; shift 2 ;;
        --tasks)      TASKS="$2";      shift 2 ;;
        --repos-dir)  REPOS_DIR="$2";  shift 2 ;;
        --dataset)    DATASET="$2";    shift 2 ;;
        -h|--help)    usage ;;
        *) echo "Unknown argument: $1" >&2; usage ;;
    esac
done

mkdir -p "$REPOS_DIR"

echo "Tasks file:  ${TASKS_FILE}"
echo "Repos dir:   ${REPOS_DIR}"
echo "Dataset:     ${DATASET}"
echo ""

# Fetch metadata for all requested task IDs and emit NDJSON lines:
#   {"instance_id": "...", "repo": "owner/name", "base_commit": "abc123", "problem_statement": "..."}
METADATA_NDJSON="$(uv run --with datasets --with huggingface_hub python3 - <<PYEOF
import json, sys
from datasets import load_dataset

with open('${TASKS_FILE}') as f:
    task_ids = json.load(f)

limit = int('${TASKS}')
if limit > 0:
    task_ids = task_ids[:limit]

task_set = set(task_ids)

ds = load_dataset('${DATASET}', split='test')
for row in ds:
    if row['instance_id'] in task_set:
        print(json.dumps({
            'instance_id':       row['instance_id'],
            'repo':              row['repo'],
            'base_commit':       row['base_commit'],
            'problem_statement': row['problem_statement'],
        }))
PYEOF
)"

TOTAL="$(echo "$METADATA_NDJSON" | grep -c . || true)"
echo "Fetched metadata for ${TOTAL} tasks."
echo ""

IDX=0
while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    IDX=$((IDX + 1))

    INSTANCE_ID="$(echo "$line" | python3 -c "import json,sys; print(json.load(sys.stdin)['instance_id'])")"
    REPO="$(echo        "$line" | python3 -c "import json,sys; print(json.load(sys.stdin)['repo'])")"
    BASE_COMMIT="$(echo "$line" | python3 -c "import json,sys; print(json.load(sys.stdin)['base_commit'])")"
    PROBLEM="$(echo     "$line" | python3 -c "import json,sys; print(json.load(sys.stdin)['problem_statement'])")"

    DEST="${REPOS_DIR}/${INSTANCE_ID}"
    echo "[${IDX}/${TOTAL}] ${INSTANCE_ID}"

    if [[ -f "${DEST}/ISSUE.txt" ]] && git -C "$DEST" rev-parse --verify HEAD &>/dev/null; then
        CURRENT="$(git -C "$DEST" rev-parse HEAD)"
        if [[ "$CURRENT" == "$BASE_COMMIT" ]]; then
            echo "  Already set up at ${BASE_COMMIT:0:12} — skipping."
            continue
        fi
    fi

    CLONE_URL="https://github.com/${REPO}.git"

    if [[ -d "$DEST/.git" ]]; then
        echo "  Repo exists, fetching and checking out ${BASE_COMMIT:0:12}..."
        git -C "$DEST" fetch --quiet origin
    else
        echo "  Cloning ${CLONE_URL}..."
        # Partial (blobless) clone — much faster than a full clone
        git clone --filter=blob:none --no-checkout --quiet "$CLONE_URL" "$DEST"
    fi

    git -C "$DEST" checkout --quiet "$BASE_COMMIT"
    printf '%s\n' "$PROBLEM" > "${DEST}/ISSUE.txt"
    echo "  Done: checked out ${BASE_COMMIT:0:12}"

done <<< "$METADATA_NDJSON"

echo ""
echo "Setup complete. ${IDX} repos ready under ${REPOS_DIR}/"
