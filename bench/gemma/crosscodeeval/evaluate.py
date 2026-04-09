#!/usr/bin/env python3
"""Evaluate Gemma on CrossCodeEval with and without spelunk.

CrossCodeEval tasks each present a code file truncated at a completion point.
The ground truth is a single line that requires understanding of symbols
defined in other files — exactly what spelunk_search helps with.

Baseline condition: model sees only the truncated prompt.
Spelunk condition:  model has spelunk_search as a function-call tool and can
                    retrieve cross-file context before generating the completion.

Usage:
    python bench/gemma/crosscodeeval/evaluate.py --condition baseline
    python bench/gemma/crosscodeeval/evaluate.py --condition spelunk --repo-path /path/to/indexed/repo

    # Multiple languages, limit samples, custom output
    python bench/gemma/crosscodeeval/evaluate.py \\
        --condition spelunk \\
        --repo-path /path/to/repo \\
        --languages python,typescript \\
        --samples 200 \\
        --out bench/results/cce-spelunk.json
"""

import argparse
import difflib
import json
import re
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

import numpy as np
from datasets import load_dataset
from openai import OpenAI
from tqdm import tqdm

MAX_SEARCH_TURNS = 5
MAX_OUTPUT_CHARS = 4_000

SYSTEM_BASELINE = (
    "You are a code completion assistant. "
    "Complete the next line of the given code. "
    "Output only the completion line, nothing else — no explanation, no markdown fences."
)

SYSTEM_SPELUNK = (
    "You are a code completion assistant. "
    "Use spelunk_search to find relevant type definitions, function signatures, or constants "
    "from other files in the codebase before completing the code. "
    "Output only the completion line, nothing else — no explanation, no markdown fences."
)

SPELUNK_TOOL = {
    "type": "function",
    "function": {
        "name": "spelunk_search",
        "description": (
            "Semantically search the indexed codebase for relevant code — type definitions, "
            "function signatures, constants, or class structures from other files."
        ),
        "parameters": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Natural-language search query."},
                "limit": {"type": "integer", "description": "Max results (default 5).", "default": 5},
            },
            "required": ["query"],
        },
    },
}


def spelunk_search(repo_path: Path, query: str, limit: int = 5) -> str:
    cmd = ["spelunk", "search", query, "--limit", str(limit), "--format", "json"]
    try:
        result = subprocess.run(cmd, cwd=repo_path, capture_output=True, text=True, timeout=30)
        output = result.stdout
        if result.returncode != 0:
            return f"spelunk search failed: {result.stderr.strip()}"
    except FileNotFoundError:
        return "Error: spelunk not found in PATH."
    except subprocess.TimeoutExpired:
        return "Error: spelunk search timed out."
    if len(output) > MAX_OUTPUT_CHARS:
        output = output[:MAX_OUTPUT_CHARS] + "\n... (truncated)"
    return output or "(no results)"


def complete_baseline(client: OpenAI, model: str, prompt: str) -> tuple[str, int, int]:
    """Ask the model to complete the next line. Returns (completion, input_tokens, output_tokens)."""
    response = client.chat.completions.create(
        model=model,
        max_tokens=256,
        temperature=0.0,
        messages=[
            {"role": "system", "content": SYSTEM_BASELINE},
            {"role": "user", "content": f"Complete the next line:\n\n{prompt}"},
        ],
    )
    content = response.choices[0].message.content or ""
    return (
        content.strip(),
        response.usage.prompt_tokens,
        response.usage.completion_tokens,
    )


def complete_spelunk(
    client: OpenAI, model: str, prompt: str, repo_path: Path
) -> tuple[str, int, int]:
    """Run a tool-use loop: model may call spelunk_search before returning the completion."""
    messages = [
        {"role": "system", "content": SYSTEM_SPELUNK},
        {"role": "user", "content": f"Complete the next line:\n\n{prompt}"},
    ]
    input_tokens = 0
    output_tokens = 0

    for _ in range(MAX_SEARCH_TURNS):
        response = client.chat.completions.create(
            model=model,
            max_tokens=512,
            temperature=0.0,
            tools=[SPELUNK_TOOL],
            tool_choice="auto",
            messages=messages,
        )
        msg = response.choices[0].message
        input_tokens += response.usage.prompt_tokens
        output_tokens += response.usage.completion_tokens

        # Build assistant entry — include tool_calls only if present
        assistant_entry: dict = {"role": "assistant", "content": msg.content or ""}
        if msg.tool_calls:
            assistant_entry["tool_calls"] = [
                {
                    "id": tc.id,
                    "type": "function",
                    "function": {"name": tc.function.name, "arguments": tc.function.arguments},
                }
                for tc in msg.tool_calls
            ]
        messages.append(assistant_entry)

        if response.choices[0].finish_reason != "tool_calls" or not msg.tool_calls:
            return (msg.content or "").strip(), input_tokens, output_tokens

        # Execute tool calls
        for tc in msg.tool_calls:
            args = json.loads(tc.function.arguments)
            result = spelunk_search(repo_path, args["query"], args.get("limit", 5))
            messages.append({"role": "tool", "tool_call_id": tc.id, "content": result})

    # Fell through max turns — ask for final answer without tools
    response = client.chat.completions.create(
        model=model,
        max_tokens=256,
        temperature=0.0,
        messages=messages + [{"role": "user", "content": "Now output only the completion line."}],
    )
    input_tokens += response.usage.prompt_tokens
    output_tokens += response.usage.completion_tokens
    return (response.choices[0].message.content or "").strip(), input_tokens, output_tokens


def edit_similarity(pred: str, truth: str) -> float:
    return difflib.SequenceMatcher(None, pred, truth).ratio()


def extract_identifiers(code: str) -> set[str]:
    """Return the set of identifier tokens (word characters, length >= 2)."""
    return {t for t in re.findall(r"\b[A-Za-z_]\w+\b", code)}


def identifier_recall(pred: str, truth: str) -> float:
    """Fraction of identifiers in ground truth that appear in the prediction."""
    truth_ids = extract_identifiers(truth)
    if not truth_ids:
        return 1.0
    pred_ids = extract_identifiers(pred)
    return len(truth_ids & pred_ids) / len(truth_ids)


def get_spelunk_version() -> str:
    try:
        r = subprocess.run(["spelunk", "--version"], capture_output=True, text=True, timeout=10)
        return r.stdout.strip().split()[-1] if r.stdout.strip() else "unknown"
    except Exception:
        return "unknown"


def scaffold_hash() -> str:
    bench_dir = Path(__file__).parents[3]  # repo root / bench
    try:
        r = subprocess.run(
            ["git", "log", "-1", "--format=%H", "--", "bench/"],
            cwd=bench_dir,
            capture_output=True,
            text=True,
            timeout=10,
        )
        return r.stdout.strip() or "unknown"
    except Exception:
        return "unknown"


DATASET = "tianyang/repobench_python_v1.1"
# cross_file_first: completion requires a symbol introduced in another file that
# appears first in the cross-file context — the hardest and most relevant split
# for measuring spelunk's retrieval benefit.
VALID_SPLITS = ("cross_file_first", "cross_file_random", "in_file")


def evaluate_split(
    split: str,
    samples: int,
    condition: str,
    client: OpenAI,
    model: str,
    repo_path: Path | None,
) -> tuple[list, list, list, list, list, list]:
    """Returns (exact_matches, edit_sims, id_recalls, input_tokens, output_tokens, wall_times)."""
    print(f"Loading RepoBench-Python ({split})...")
    dataset = load_dataset(DATASET, split=split)

    n = min(samples, len(dataset))
    indices = np.random.choice(len(dataset), size=n, replace=False).tolist()
    sampled = dataset.select(indices)

    exact_matches, edit_sims, id_recalls = [], [], []
    input_tokens_list, output_tokens_list, wall_times = [], [], []

    for item in tqdm(sampled, desc=f"{split} ({condition})", unit="task"):
        # RepoBench fields: cropped_code = prompt up to completion point,
        # next_line = the line to predict.
        prompt = item.get("cropped_code", "")
        truth = (item.get("next_line") or "").strip()

        if not prompt or not truth:
            continue

        start = time.monotonic()
        try:
            if condition == "baseline":
                pred, inp_tok, out_tok = complete_baseline(client, model, prompt)
            else:
                pred, inp_tok, out_tok = complete_spelunk(client, model, prompt, repo_path)
        except Exception as e:
            print(f"\nWarning: inference failed — {e}")
            continue
        elapsed = time.monotonic() - start

        exact_matches.append(1.0 if pred == truth else 0.0)
        edit_sims.append(edit_similarity(pred, truth))
        id_recalls.append(identifier_recall(pred, truth))
        input_tokens_list.append(inp_tok)
        output_tokens_list.append(out_tok)
        wall_times.append(elapsed)

    return exact_matches, edit_sims, id_recalls, input_tokens_list, output_tokens_list, wall_times


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Evaluate Gemma on RepoBench-Python cross-file completion."
    )
    parser.add_argument("--condition", choices=["baseline", "spelunk"], required=True)
    parser.add_argument(
        "--repo-path",
        default=None,
        help="Path to an indexed repo (required for --condition spelunk).",
    )
    parser.add_argument(
        "--split",
        default="cross_file_first",
        help=f"Dataset split to use. One of: {', '.join(VALID_SPLITS)} (default: cross_file_first).",
    )
    parser.add_argument("--samples", type=int, default=200)
    parser.add_argument("--model", default="gemma-4-e2b-it")
    parser.add_argument("--api-base-url", default="http://127.0.0.1:1234/v1")
    parser.add_argument("--scaffold-hash", default=None, help="Passed by run.sh; auto-computed if omitted.")
    parser.add_argument("--out", default=None)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()

    if args.split not in VALID_SPLITS:
        parser.error(f"--split must be one of: {', '.join(VALID_SPLITS)}")
    if args.condition == "spelunk" and not args.repo_path:
        parser.error("--repo-path is required for --condition spelunk")

    repo_path = Path(args.repo_path).resolve() if args.repo_path else None
    if repo_path and not repo_path.is_dir():
        parser.error(f"--repo-path does not exist: {repo_path}")

    np.random.seed(args.seed)
    client = OpenAI(base_url=args.api_base_url, api_key="local")

    all_exact, all_edit, all_id_recall = [], [], []
    all_inp_tok, all_out_tok, all_wall = [], [], []

    exact, edit, id_rec, inp_tok, out_tok, wall = evaluate_split(
        args.split, args.samples, args.condition, client, args.model, repo_path
    )
    all_exact.extend(exact)
    all_edit.extend(edit)
    all_id_recall.extend(id_rec)
    all_inp_tok.extend(inp_tok)
    all_out_tok.extend(out_tok)
    all_wall.extend(wall)

    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = {
        "run_id": timestamp,
        "benchmark": "repobench",
        "condition": args.condition,
        "model": args.model,
        "model_source": "local",
        "api_base_url": args.api_base_url,
        "spelunk_version": get_spelunk_version(),
        "scaffold_hash": args.scaffold_hash or scaffold_hash(),
        "split": args.split,
        "samples": len(all_exact),
        "exact_match": round(float(np.mean(all_exact)), 4) if all_exact else 0.0,
        "edit_similarity": round(float(np.mean(all_edit)), 4) if all_edit else 0.0,
        "identifier_recall": round(float(np.mean(all_id_recall)), 4) if all_id_recall else 0.0,
        "median_input_tokens": round(float(np.median(all_inp_tok)), 1) if all_inp_tok else 0.0,
        "median_output_tokens": round(float(np.median(all_out_tok)), 1) if all_out_tok else 0.0,
        "median_wall_seconds": round(float(np.median(all_wall)), 3) if all_wall else 0.0,
    }

    if args.out:
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(json.dumps(output, indent=2))
        print(f"Results written to: {args.out}")
    else:
        print(json.dumps(output, indent=2))

    print(f"\nExact match:       {output['exact_match']:.4f}")
    print(f"Edit similarity:   {output['edit_similarity']:.4f}")
    print(f"Identifier recall: {output['identifier_recall']:.4f}")
    print(f"Median wall:       {output['median_wall_seconds']:.3f}s")


if __name__ == "__main__":
    main()
