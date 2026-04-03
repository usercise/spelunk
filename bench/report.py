#!/usr/bin/env python3
"""Compare spelunk benchmark results across two or more result JSON files.

Usage:
    python bench/report.py results/run-a.json results/run-b.json [results/run-c.json ...]
"""

import json
import sys
from pathlib import Path


def load_result(path: str) -> dict:
    with open(path) as f:
        return json.load(f)


def fmt(value, fmt_spec=".3f") -> str:
    if value is None:
        return "—"
    return format(value, fmt_spec)


def make_row(result: dict) -> dict:
    model = result.get("model", "unknown")
    source = result.get("model_source")
    model_label = f"{model} ({source})" if source else model

    return {
        "benchmark": result.get("benchmark", "unknown"),
        "condition": result.get("condition", "unknown"),
        "model": model_label,
        "spelunk_version": result.get("spelunk_version", "unknown"),
        # SWE-bench
        "resolve_rate": result.get("resolve_rate"),
        # CodeSearchNet
        "mrr_at_10": result.get("mrr_at_10"),
        # CrossCodeEval
        "exact_match": result.get("exact_match"),
        "edit_similarity": result.get("edit_similarity"),
        "identifier_recall": result.get("identifier_recall"),
        # Shared
        "median_tokens_per_task": result.get("median_tokens_per_task"),
        "median_wall_seconds": result.get("median_wall_seconds"),
    }


HEADERS = [
    ("benchmark",            "benchmark"),
    ("condition",            "condition"),
    ("model",                "model"),
    ("spelunk_version",      "spelunk_ver"),
    ("resolve_rate",         "resolve_rate"),
    ("mrr_at_10",            "mrr@10"),
    ("exact_match",          "exact_match"),
    ("edit_similarity",      "edit_sim"),
    ("identifier_recall",    "id_recall"),
    ("median_tokens_per_task", "med_tokens"),
    ("median_wall_seconds",  "med_wall_s"),
]

NUMERIC_KEYS = {
    "resolve_rate", "mrr_at_10", "exact_match", "edit_similarity",
    "identifier_recall", "median_tokens_per_task", "median_wall_seconds",
}


def cell_str(value, key: str) -> str:
    if value is None:
        return "—"
    if key in NUMERIC_KEYS and isinstance(value, float):
        return format(value, ".3f")
    return str(value)


def print_markdown_table(rows: list[dict]) -> None:
    # Only include columns that have at least one non-None value
    active_headers = [
        (key, label) for key, label in HEADERS
        if any(r.get(key) is not None for r in rows)
    ]

    widths = {
        key: max(len(label), max(len(cell_str(r.get(key), key)) for r in rows))
        for key, label in active_headers
    }

    def row_line(r: dict) -> str:
        parts = [cell_str(r.get(key), key).ljust(widths[key]) for key, _ in active_headers]
        return "| " + " | ".join(parts) + " |"

    header_parts = [label.ljust(widths[key]) for key, label in active_headers]
    print("| " + " | ".join(header_parts) + " |")
    print("|-" + "-|-".join("-" * widths[key] for key, _ in active_headers) + "-|")
    for row in rows:
        print(row_line(row))


def main() -> None:
    if len(sys.argv) < 3:
        print("Usage: python bench/report.py <result1.json> <result2.json> [...]", file=sys.stderr)
        sys.exit(1)

    rows = []
    for path in sys.argv[1:]:
        p = Path(path)
        if not p.exists():
            print(f"Error: file not found: {path}", file=sys.stderr)
            sys.exit(1)
        rows.append(make_row(load_result(path)))

    print_markdown_table(rows)


if __name__ == "__main__":
    main()
