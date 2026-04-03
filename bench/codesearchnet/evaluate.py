#!/usr/bin/env python3
"""Evaluate spelunk retrieval accuracy using the CodeSearchNet dataset.

For each sampled (docstring query, function) pair, runs `spelunk search`
and checks whether the ground-truth function name appears in any of the
top-10 results. Computes MRR@10, Recall@5, and Recall@10.

Prerequisites:
  - spelunk must be in PATH and the target project must already be indexed.
  - pip install datasets tqdm numpy

Usage:
    python bench/codesearchnet/evaluate.py \
        [--languages python,java,go] \
        [--samples 1000] \
        [--out bench/results/csn-run.json]
"""

import argparse
import json
import subprocess
import time
from datetime import datetime, timezone

import numpy as np
from datasets import load_dataset
from tqdm import tqdm


def get_spelunk_version() -> str:
    """Return the installed spelunk version string."""
    try:
        result = subprocess.run(
            ["spelunk", "--version"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        return result.stdout.strip().split()[-1] if result.stdout.strip() else "unknown"
    except Exception:
        return "unknown"


def spelunk_search(query: str, limit: int = 10) -> list[dict]:
    """Run spelunk search and return parsed results."""
    cmd = ["spelunk", "search", query, "--limit", str(limit), "--format", "json"]
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=30,
        )
        if result.returncode != 0 or not result.stdout.strip():
            return []
        return json.loads(result.stdout)
    except (subprocess.TimeoutExpired, json.JSONDecodeError, Exception):
        return []


def find_rank(results: list[dict], func_name: str) -> int | None:
    """Return 1-based rank of the first result matching func_name, or None."""
    for i, item in enumerate(results):
        # Results may have a 'name', 'symbol', or 'chunk_name' field
        name = (
            item.get("name")
            or item.get("symbol")
            or item.get("chunk_name")
            or ""
        )
        if func_name and func_name in name:
            return i + 1
    return None


def evaluate_language(language: str, samples: int) -> tuple[list[float], list[float], list[float], list[float]]:
    """Return (reciprocal_ranks, recall5_hits, recall10_hits, wall_times) for one language."""
    print(f"Loading CodeSearchNet dataset for language: {language}...")
    # load_dataset returns a DatasetDict; use the test split for evaluation
    dataset = load_dataset("code_search_net", language, split="test", trust_remote_code=True)

    # Sample uniformly; cap at dataset size
    n = min(samples, len(dataset))
    indices = np.random.choice(len(dataset), size=n, replace=False).tolist()
    sampled = dataset.select(indices)

    reciprocal_ranks = []
    recall5_hits = []
    recall10_hits = []
    wall_times = []

    for item in tqdm(sampled, desc=language, unit="query"):
        query = item.get("func_documentation_string") or item.get("docstring") or ""
        # Ground-truth function name
        func_name = item.get("func_name") or item.get("function_tokens", [""])[0]

        if not query.strip():
            continue

        start = time.monotonic()
        results = spelunk_search(query, limit=10)
        elapsed = time.monotonic() - start
        wall_times.append(elapsed)

        rank = find_rank(results, func_name)

        # MRR@10
        reciprocal_ranks.append(1.0 / rank if rank is not None else 0.0)
        # Recall@5 and @10
        recall5_hits.append(1.0 if rank is not None and rank <= 5 else 0.0)
        recall10_hits.append(1.0 if rank is not None else 0.0)

    return reciprocal_ranks, recall5_hits, recall10_hits, wall_times


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Evaluate spelunk retrieval accuracy using CodeSearchNet."
    )
    parser.add_argument(
        "--languages",
        default="python",
        help="Comma-separated list of CodeSearchNet languages (default: python).",
    )
    parser.add_argument(
        "--samples",
        type=int,
        default=1000,
        help="Number of (query, function) pairs to sample per language (default: 1000).",
    )
    parser.add_argument(
        "--out",
        default=None,
        help="Output JSON file path. If not set, prints to stdout.",
    )
    args = parser.parse_args()

    languages = [lang.strip() for lang in args.languages.split(",") if lang.strip()]
    spelunk_version = get_spelunk_version()
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")

    all_rr: list[float] = []
    all_r5: list[float] = []
    all_r10: list[float] = []
    all_wall: list[float] = []

    for lang in languages:
        rr, r5, r10, wall = evaluate_language(lang, args.samples)
        all_rr.extend(rr)
        all_r5.extend(r5)
        all_r10.extend(r10)
        all_wall.extend(wall)

    mrr_at_10 = float(np.mean(all_rr)) if all_rr else 0.0
    recall_at_5 = float(np.mean(all_r5)) if all_r5 else 0.0
    recall_at_10 = float(np.mean(all_r10)) if all_r10 else 0.0
    median_wall = float(np.median(all_wall)) if all_wall else 0.0

    output = {
        "run_id": timestamp,
        "benchmark": "codesearchnet",
        "condition": "spelunk",
        "spelunk_version": spelunk_version,
        "languages": languages,
        "samples": len(all_rr),
        "mrr_at_10": round(mrr_at_10, 4),
        "recall_at_5": round(recall_at_5, 4),
        "recall_at_10": round(recall_at_10, 4),
        "median_wall_seconds": round(median_wall, 3),
    }

    if args.out:
        import os
        os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
        with open(args.out, "w") as f:
            json.dump(output, f, indent=2)
        print(f"Results written to: {args.out}")
    else:
        print(json.dumps(output, indent=2))

    # Print a brief summary to stderr so it shows up even when stdout is redirected
    print(f"\nMRR@10:      {mrr_at_10:.4f}", flush=True)
    print(f"Recall@5:    {recall_at_5:.4f}")
    print(f"Recall@10:   {recall_at_10:.4f}")
    print(f"Median wall: {median_wall:.3f}s")


if __name__ == "__main__":
    main()
