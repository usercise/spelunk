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
    """Format a numeric value, or return '—' if None/missing."""
    if value is None:
        return "—"
    return format(value, fmt_spec)


def make_row(result: dict) -> dict:
    benchmark = result.get("benchmark", "unknown")
    condition = result.get("condition", "unknown")
    spelunk_version = result.get("spelunk_version", "unknown")

    # SWE-bench fields
    resolve_rate = result.get("resolve_rate")
    # CodeSearchNet fields
    mrr_at_10 = result.get("mrr_at_10")
    # Shared fields
    median_tokens = result.get("median_tokens_per_task")
    median_wall = result.get("median_wall_seconds")

    return {
        "benchmark": benchmark,
        "condition": condition,
        "spelunk_version": spelunk_version,
        "resolve_rate": resolve_rate,
        "mrr_at_10": mrr_at_10,
        "median_tokens_per_task": median_tokens,
        "median_wall_seconds": median_wall,
    }


def col_width(header: str, rows: list[dict], key: str) -> int:
    """Compute column width as max of header and all formatted values."""
    values = [fmt(r[key]) if isinstance(r[key], (int, float, type(None))) else str(r[key]) for r in rows]
    return max(len(header), max((len(v) for v in values), default=0))


def print_markdown_table(rows: list[dict]) -> None:
    headers = [
        ("benchmark", "benchmark"),
        ("condition", "condition"),
        ("spelunk_version", "spelunk_version"),
        ("resolve_rate", "resolve_rate"),
        ("mrr_at_10", "mrr_at_10"),
        ("median_tokens_per_task", "median_tokens"),
        ("median_wall_seconds", "median_wall_s"),
    ]

    # Compute column widths
    widths = {}
    for key, label in headers:
        if key in ("resolve_rate", "mrr_at_10", "median_tokens_per_task", "median_wall_seconds"):
            formatted_rows = [{**r, key: fmt(r[key])} for r in rows]
        else:
            formatted_rows = rows
        widths[key] = max(len(label), max((len(str(r[key])) for r in formatted_rows), default=0))

    def cell(value, key, width) -> str:
        if isinstance(value, float):
            s = fmt(value)
        elif value is None:
            s = "—"
        else:
            s = str(value)
        return s.ljust(width)

    # Header row
    header_parts = [cell(label, key, widths[key]) for key, label in headers]
    print("| " + " | ".join(header_parts) + " |")

    # Separator row
    sep_parts = ["-" * widths[key] for key, _ in headers]
    print("|-" + "-|-".join(sep_parts) + "-|")

    # Data rows
    for row in rows:
        parts = [cell(row[key], key, widths[key]) for key, _ in headers]
        print("| " + " | ".join(parts) + " |")


def main() -> None:
    if len(sys.argv) < 3:
        print("Usage: python bench/report.py <result1.json> <result2.json> [...]", file=sys.stderr)
        print("  Prints a markdown table comparing two or more benchmark result files.", file=sys.stderr)
        sys.exit(1)

    paths = sys.argv[1:]
    rows = []
    for path in paths:
        p = Path(path)
        if not p.exists():
            print(f"Error: file not found: {path}", file=sys.stderr)
            sys.exit(1)
        result = load_result(path)
        rows.append(make_row(result))

    print_markdown_table(rows)


if __name__ == "__main__":
    main()
