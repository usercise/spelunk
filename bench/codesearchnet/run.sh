#!/usr/bin/env bash
# Thin wrapper around evaluate.py that adds a timestamp to the output filename.
#
# Usage:
#   bash bench/codesearchnet/run.sh [--languages python,java] [--samples 1000] [--out FILE]
#
# All arguments are forwarded to evaluate.py. If --out is not given, a
# timestamped filename is generated under bench/results/.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

usage() {
    grep '^#' "$0" | grep -v '#!/' | sed 's/^# \?//'
    exit 1
}

# Check for -h/--help before forwarding args
for arg in "$@"; do
    case "$arg" in
        -h|--help) usage ;;
    esac
done

# Build default output path with timestamp if --out not supplied
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
DEFAULT_OUT="${BENCH_DIR}/results/codesearchnet-spelunk-${TIMESTAMP}.json"

# Check whether the caller passed --out
HAS_OUT=0
for arg in "$@"; do
    if [[ "$arg" == "--out" ]]; then
        HAS_OUT=1
        break
    fi
done

if [[ "$HAS_OUT" -eq 0 ]]; then
    mkdir -p "${BENCH_DIR}/results"
    exec python3 "${SCRIPT_DIR}/evaluate.py" "$@" --out "$DEFAULT_OUT"
else
    exec python3 "${SCRIPT_DIR}/evaluate.py" "$@"
fi
