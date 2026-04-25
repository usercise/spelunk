#!/usr/bin/env python3
"""
LinearRAG vs KNN evaluation script.
Runs 20 queries, collects top-10 results + latency for both algorithms,
writes results JSON for labelling, then computes metrics if labels exist.

Usage:
  python3 bench/linearrag/run_eval.py collect   # collect results
  python3 bench/linearrag/run_eval.py metrics   # compute metrics from labelled results
"""

import json
import subprocess
import sys
import time
import os
from pathlib import Path

BINARY = "./target/release/spelunk"
OUT_DIR = Path("bench/linearrag")
RESULTS_FILE = OUT_DIR / "results.json"
LABELS_FILE = OUT_DIR / "labels.json"

# 20 queries: 5 structural, 10 semantic multi-hop, 5 cross-cutting
QUERIES = [
    # --- Structural (expected: KNN ≥ LinearRAG) ---
    {"id": "S1", "cat": "structural", "q": "where is compute_pagerank called"},
    {"id": "S2", "cat": "structural", "q": "what functions call replace_edges"},
    {"id": "S3", "cat": "structural", "q": "where is insert_chunk called"},
    {"id": "S4", "cat": "structural", "q": "what calls search_hybrid"},
    {"id": "S5", "cat": "structural", "q": "where is embed_query_vec called"},

    # --- Semantic multi-hop (expected: LinearRAG > KNN) ---
    {"id": "M1",  "cat": "multihop", "q": "how does error handling propagate from storage to CLI"},
    {"id": "M2",  "cat": "multihop", "q": "what code is involved in the embedding pipeline"},
    {"id": "M3",  "cat": "multihop", "q": "how does incremental indexing work with file hashing"},
    {"id": "M4",  "cat": "multihop", "q": "how are tree-sitter parse results converted to chunks"},
    {"id": "M5",  "cat": "multihop", "q": "how does the search result get formatted and output to the terminal"},
    {"id": "M6",  "cat": "multihop", "q": "how does the KNN vector search query work end to end"},
    {"id": "M7",  "cat": "multihop", "q": "what happens when a secret credential is detected in a chunk"},
    {"id": "M8",  "cat": "multihop", "q": "how does the project registry track and link multiple projects"},
    {"id": "M9",  "cat": "multihop", "q": "how does graph-based neighbour expansion enrich search results"},
    {"id": "M10", "cat": "multihop", "q": "how are chunk embeddings stored and retrieved from sqlite-vec"},

    # --- Cross-cutting (expected: LinearRAG ≥ KNN) ---
    {"id": "C1", "cat": "crosscut", "q": "code that touches both database storage and vector embeddings"},
    {"id": "C2", "cat": "crosscut", "q": "what handles both CLI argument parsing and config loading"},
    {"id": "C3", "cat": "crosscut", "q": "code involved in both source parsing and chunk storage"},
    {"id": "C4", "cat": "crosscut", "q": "what connects search results to graph edge traversal"},
    {"id": "C5", "cat": "crosscut", "q": "code that links memory entries to vector similarity search"},
]

LATENCY_REPS = 5  # repeat each query N times to get stable latency


def run_query(query: str, algo: str, limit: int = 10) -> tuple[list, float]:
    """Run a query and return (results, latency_ms)."""
    cmd = [
        BINARY, "search", query,
        "--retrieval", algo,
        "--limit", str(limit),
        "--format", "json",
        "--no-stale-check",
    ]
    start = time.perf_counter()
    result = subprocess.run(cmd, capture_output=True, text=True)
    elapsed_ms = (time.perf_counter() - start) * 1000

    if result.returncode != 0:
        return [], elapsed_ms
    try:
        return json.loads(result.stdout), elapsed_ms
    except json.JSONDecodeError:
        return [], elapsed_ms


def collect():
    OUT_DIR.mkdir(parents=True, exist_ok=True)

    all_results = []
    total = len(QUERIES)

    for i, qdef in enumerate(QUERIES):
        qid = qdef["id"]
        cat = qdef["cat"]
        q = qdef["q"]
        print(f"[{i+1}/{total}] {qid} ({cat}): {q[:60]}…", flush=True)

        # Warm-up run (not timed)
        run_query(q, "knn", limit=10)

        # Collect timed results
        knn_results, knn_times = [], []
        lr_results, lr_times = [], []

        for rep in range(LATENCY_REPS):
            res, ms = run_query(q, "knn")
            knn_times.append(ms)
            if rep == 0:
                knn_results = res

        for rep in range(LATENCY_REPS):
            res, ms = run_query(q, "linearrag")
            lr_times.append(ms)
            if rep == 0:
                lr_results = res

        knn_times.sort()
        lr_times.sort()

        entry = {
            "id": qid,
            "cat": cat,
            "query": q,
            "knn": {
                "results": [
                    {"rank": r+1, "chunk_id": x["chunk_id"], "file": x["file_path"],
                     "name": x.get("name"), "start": x["start_line"], "end": x["end_line"],
                     "snippet": x["content"][:120].replace("\n", " ")}
                    for r, x in enumerate(knn_results)
                ],
                "latency_p50_ms": knn_times[LATENCY_REPS // 2],
                "latency_p95_ms": knn_times[int(LATENCY_REPS * 0.95)] if LATENCY_REPS >= 20 else knn_times[-1],
            },
            "linearrag": {
                "results": [
                    {"rank": r+1, "chunk_id": x["chunk_id"], "file": x["file_path"],
                     "name": x.get("name"), "start": x["start_line"], "end": x["end_line"],
                     "snippet": x["content"][:120].replace("\n", " ")}
                    for r, x in enumerate(lr_results)
                ],
                "latency_p50_ms": lr_times[LATENCY_REPS // 2],
                "latency_p95_ms": lr_times[int(LATENCY_REPS * 0.95)] if LATENCY_REPS >= 20 else lr_times[-1],
            },
        }
        all_results.append(entry)
        print(f"  knn p50={knn_times[LATENCY_REPS//2]:.0f}ms  lr p50={lr_times[LATENCY_REPS//2]:.0f}ms", flush=True)

    with open(RESULTS_FILE, "w") as f:
        json.dump(all_results, f, indent=2)
    print(f"\nResults written to {RESULTS_FILE}")
    print("Next: label relevant chunks in bench/linearrag/labels.json, then run: python3 bench/linearrag/run_eval.py metrics")
    generate_label_template(all_results)


def generate_label_template(all_results):
    """Generate a label template JSON — fill in 1 (relevant) or 0 for each result."""
    labels = {}
    for entry in all_results:
        pool = {}
        # Pool = union of chunk_ids from both systems
        for r in entry["knn"]["results"] + entry["linearrag"]["results"]:
            cid = str(r["chunk_id"])
            if cid not in pool:
                pool[cid] = {
                    "file": r["file"],
                    "name": r["name"],
                    "start": r["start"],
                    "snippet": r["snippet"],
                    "relevant": -1,  # -1 = unlabelled; set to 0 or 1
                }
        labels[entry["id"]] = {
            "query": entry["query"],
            "pool": pool,
        }
    with open(LABELS_FILE, "w") as f:
        json.dump(labels, f, indent=2)
    print(f"Label template written to {LABELS_FILE}")


def metrics():
    if not RESULTS_FILE.exists():
        print("Run collect first.")
        sys.exit(1)
    if not LABELS_FILE.exists():
        print("Labels file not found. Run collect first.")
        sys.exit(1)

    with open(RESULTS_FILE) as f:
        all_results = json.load(f)
    with open(LABELS_FILE) as f:
        labels = json.load(f)

    # Check for unlabelled entries
    unlabelled = []
    for qid, ldata in labels.items():
        for cid, info in ldata["pool"].items():
            if info["relevant"] == -1:
                unlabelled.append((qid, cid, info["file"], info.get("name")))
    if unlabelled:
        print(f"WARNING: {len(unlabelled)} unlabelled entries. Treating as 0.")

    # Compute per-query metrics
    rows = []
    for entry in all_results:
        qid = entry["id"]
        cat = entry["cat"]
        q = entry["query"]
        ldata = labels.get(qid, {})
        pool = ldata.get("pool", {})

        def relevance(chunk_id):
            return pool.get(str(chunk_id), {}).get("relevant", 0)

        def recall_at_k(results, k=10):
            rel_in_pool = sum(1 for v in pool.values() if v.get("relevant", 0) == 1)
            if rel_in_pool == 0:
                return None
            hits = sum(1 for r in results[:k] if relevance(r["chunk_id"]) == 1)
            return hits / rel_in_pool

        def mrr(results):
            for r in results:
                if relevance(r["chunk_id"]) == 1:
                    return 1.0 / r["rank"]
            return 0.0

        def precision_at_k(results, k=10):
            hits = sum(1 for r in results[:k] if relevance(r["chunk_id"]) == 1)
            return hits / min(k, len(results)) if results else 0.0

        knn_r = entry["knn"]["results"]
        lr_r = entry["linearrag"]["results"]

        row = {
            "id": qid,
            "cat": cat,
            "query": q[:50],
            "knn_recall10": recall_at_k(knn_r),
            "lr_recall10": recall_at_k(lr_r),
            "knn_p10": precision_at_k(knn_r),
            "lr_p10": precision_at_k(lr_r),
            "knn_mrr": mrr(knn_r),
            "lr_mrr": mrr(lr_r),
            "knn_lat_ms": entry["knn"]["latency_p50_ms"],
            "lr_lat_ms": entry["linearrag"]["latency_p50_ms"],
        }
        rows.append(row)

    # Print per-query table
    print(f"\n{'ID':<4} {'Cat':<9} {'Query':<52} {'R@10-knn':>8} {'R@10-lr':>7} {'MRR-knn':>7} {'MRR-lr':>6} {'Lat-knn':>7} {'Lat-lr':>6}")
    print("-" * 120)
    for r in rows:
        rec_knn = f"{r['knn_recall10']:.2f}" if r['knn_recall10'] is not None else "  N/A"
        rec_lr  = f"{r['lr_recall10']:.2f}"  if r['lr_recall10']  is not None else "  N/A"
        print(f"{r['id']:<4} {r['cat']:<9} {r['query']:<52} {rec_knn:>8} {rec_lr:>7} "
              f"{r['knn_mrr']:>7.3f} {r['lr_mrr']:>6.3f} {r['knn_lat_ms']:>7.0f} {r['lr_lat_ms']:>6.0f}")

    # Category aggregates
    print()
    for cat in ["structural", "multihop", "crosscut"]:
        cat_rows = [r for r in rows if r["cat"] == cat]
        valid = [r for r in cat_rows if r["knn_recall10"] is not None]
        if not valid:
            print(f"{cat}: no labelled queries")
            continue
        avg_knn_r = sum(r["knn_recall10"] for r in valid) / len(valid)
        avg_lr_r  = sum(r["lr_recall10"]  for r in valid) / len(valid)
        avg_knn_mrr = sum(r["knn_mrr"] for r in cat_rows) / len(cat_rows)
        avg_lr_mrr  = sum(r["lr_mrr"]  for r in cat_rows) / len(cat_rows)
        avg_knn_lat = sum(r["knn_lat_ms"] for r in cat_rows) / len(cat_rows)
        avg_lr_lat  = sum(r["lr_lat_ms"]  for r in cat_rows) / len(cat_rows)
        lat_ratio = avg_lr_lat / avg_knn_lat if avg_knn_lat > 0 else 0
        delta_r = (avg_lr_r - avg_knn_r) / avg_knn_r * 100 if avg_knn_r > 0 else 0
        print(f"{cat:12} R@10: knn={avg_knn_r:.3f}  lr={avg_lr_r:.3f}  Δ={delta_r:+.1f}%  "
              f"MRR: knn={avg_knn_mrr:.3f}  lr={avg_lr_mrr:.3f}  "
              f"Lat: knn={avg_knn_lat:.0f}ms  lr={avg_lr_lat:.0f}ms  ratio={lat_ratio:.2f}x")

    # Overall
    valid_all = [r for r in rows if r["knn_recall10"] is not None]
    if valid_all:
        mhop = [r for r in valid_all if r["cat"] == "multihop"]
        if mhop:
            avg_knn = sum(r["knn_recall10"] for r in mhop) / len(mhop)
            avg_lr  = sum(r["lr_recall10"]  for r in mhop) / len(mhop)
            delta = (avg_lr - avg_knn) / avg_knn * 100 if avg_knn > 0 else 0
            lat_mhop_knn = sum(r["knn_lat_ms"] for r in mhop) / len(mhop)
            lat_mhop_lr  = sum(r["lr_lat_ms"]  for r in mhop) / len(mhop)
            lat_ratio = lat_mhop_lr / lat_mhop_knn if lat_mhop_knn > 0 else 0
            print(f"\nSUCCESS THRESHOLD (multi-hop): Δ Recall@10 = {delta:+.1f}% (need ≥+15%)  "
                  f"latency ratio = {lat_ratio:.2f}x (need <2.0x)")
            if delta >= 15 and lat_ratio < 2.0:
                print("✓  PASS — threshold met")
            else:
                reasons = []
                if delta < 15:
                    reasons.append(f"recall delta {delta:+.1f}% < +15%")
                if lat_ratio >= 2.0:
                    reasons.append(f"latency ratio {lat_ratio:.2f}x ≥ 2.0x")
                print(f"✗  FAIL — {', '.join(reasons)}")

    # Index size
    try:
        res = subprocess.run(
            ["sqlite3", ".spelunk/index.db",
             "SELECT kind, COUNT(*) FROM graph_edges GROUP BY kind ORDER BY kind"],
            capture_output=True, text=True
        )
        if res.returncode == 0:
            print("\nIndex graph_edges breakdown:")
            print(res.stdout.strip())
    except Exception:
        pass


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "help"
    if cmd == "collect":
        collect()
    elif cmd == "metrics":
        metrics()
    else:
        print(__doc__)
