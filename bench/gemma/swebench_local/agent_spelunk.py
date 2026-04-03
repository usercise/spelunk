#!/usr/bin/env python3
"""SWE-bench agent with spelunk — Gemma via local OpenAI-compatible API.

Identical to agent_baseline.py but adds a spelunk_search tool.

Usage:
    python bench/gemma/swebench_local/agent_spelunk.py \\
        --task-id django__django-11099 \\
        --repo-path /path/to/repo \\
        --issue "Issue description text..." \\
        [--model gemma-4-e2b-it] \\
        [--api-base-url http://127.0.0.1:1234/v1] \\
        [--max-turns 20]
"""

import argparse
import json
import subprocess
import time
from pathlib import Path

from openai import OpenAI

TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "read_file",
            "description": "Read the contents of a file within the repository.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path relative to the repo root."}
                },
                "required": ["path"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "run_bash",
            "description": "Run a shell command in the repository directory. Output is truncated to 10 000 characters.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Shell command to execute."}
                },
                "required": ["command"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "write_file",
            "description": "Write content to a file within the repository.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path relative to the repo root."},
                    "content": {"type": "string", "description": "Full content to write."},
                },
                "required": ["path", "content"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "spelunk_search",
            "description": (
                "Semantically search the codebase using spelunk. Returns the most relevant "
                "code chunks for the given query. Use this to quickly locate relevant "
                "functions, classes, or logic without manually browsing files."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Natural-language search query."},
                    "limit": {"type": "integer", "description": "Max results (default 10).", "default": 10},
                },
                "required": ["query"],
            },
        },
    },
]

MAX_OUTPUT_CHARS = 10_000

SYSTEM_PROMPT = (
    "You are an expert software engineer. You are given a GitHub issue and a "
    "repository checkout. Your goal is to produce a minimal patch that fixes the "
    "issue. You have access to spelunk_search for fast semantic code search — use "
    "it to locate relevant code before diving into files. When you are done, "
    "briefly summarise what you changed."
)


def read_file(repo_path: Path, path: str) -> str:
    target = (repo_path / path).resolve()
    if not str(target).startswith(str(repo_path.resolve())):
        return "Error: path is outside the repository."
    try:
        return target.read_text(errors="replace")
    except Exception as e:
        return f"Error reading file: {e}"


def run_bash(repo_path: Path, command: str) -> str:
    try:
        result = subprocess.run(
            command, shell=True, cwd=repo_path, capture_output=True, text=True, timeout=60
        )
        output = result.stdout + result.stderr
    except subprocess.TimeoutExpired:
        output = "Error: command timed out after 60 seconds."
    except Exception as e:
        output = f"Error running command: {e}"
    if len(output) > MAX_OUTPUT_CHARS:
        output = output[:MAX_OUTPUT_CHARS] + "\n... (output truncated)"
    return output


def write_file(repo_path: Path, path: str, content: str) -> str:
    target = (repo_path / path).resolve()
    if not str(target).startswith(str(repo_path.resolve())):
        return "Error: path is outside the repository."
    try:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
        return f"Wrote {len(content)} bytes to {path}."
    except Exception as e:
        return f"Error writing file: {e}"


def spelunk_search(repo_path: Path, query: str, limit: int = 10) -> str:
    cmd = ["spelunk", "search", query, "--limit", str(limit), "--format", "json"]
    try:
        result = subprocess.run(cmd, cwd=repo_path, capture_output=True, text=True, timeout=30)
        output = result.stdout
        if result.returncode != 0:
            return f"spelunk search failed (exit {result.returncode}): {result.stderr.strip()}"
    except FileNotFoundError:
        return "Error: spelunk not found in PATH."
    except subprocess.TimeoutExpired:
        return "Error: spelunk search timed out."
    if len(output) > MAX_OUTPUT_CHARS:
        output = output[:MAX_OUTPUT_CHARS] + "\n... (output truncated)"
    return output or "(no results)"


def dispatch_tool(repo_path: Path, name: str, arguments: str) -> str:
    args = json.loads(arguments)
    if name == "read_file":
        return read_file(repo_path, args["path"])
    elif name == "run_bash":
        return run_bash(repo_path, args["command"])
    elif name == "write_file":
        return write_file(repo_path, args["path"], args["content"])
    elif name == "spelunk_search":
        return spelunk_search(repo_path, args["query"], args.get("limit", 10))
    return f"Unknown tool: {name}"


def run_agent(
    task_id: str,
    repo_path: Path,
    issue_text: str,
    client: OpenAI,
    model: str,
    max_turns: int,
) -> dict:
    messages = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {
            "role": "user",
            "content": (
                f"Repository path: {repo_path}\n\nIssue:\n{issue_text}\n\n"
                "Please investigate the issue and apply a fix."
            ),
        },
    ]

    turns = 0
    input_tokens = 0
    output_tokens = 0
    start = time.monotonic()

    while turns < max_turns:
        response = client.chat.completions.create(
            model=model,
            max_tokens=4096,
            tools=TOOLS,
            tool_choice="auto",
            messages=messages,
        )
        msg = response.choices[0].message
        input_tokens += response.usage.prompt_tokens
        output_tokens += response.usage.completion_tokens
        turns += 1

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
            break

        for tc in msg.tool_calls:
            result = dispatch_tool(repo_path, tc.function.name, tc.function.arguments)
            messages.append({"role": "tool", "tool_call_id": tc.id, "content": result})

    return {
        "task_id": task_id,
        "resolved": False,  # determined externally by SWE-bench harness
        "turns": turns,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "wall_seconds": round(time.monotonic() - start, 2),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="SWE-bench agent with spelunk (local model).")
    parser.add_argument("--task-id", required=True)
    parser.add_argument("--repo-path", required=True)
    parser.add_argument("--issue", required=True)
    parser.add_argument("--model", default="gemma-4-e2b-it")
    parser.add_argument("--api-base-url", default="http://127.0.0.1:1234/v1")
    parser.add_argument("--max-turns", type=int, default=20)
    args = parser.parse_args()

    repo_path = Path(args.repo_path).resolve()
    if not repo_path.is_dir():
        parser.error(f"repo-path does not exist: {repo_path}")

    client = OpenAI(base_url=args.api_base_url, api_key="local")
    result = run_agent(args.task_id, repo_path, args.issue, client, args.model, args.max_turns)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
