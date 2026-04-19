#!/usr/bin/env python3
"""Baseline SWE-bench agent — Claude with basic file tools only (no spelunk).

The agent is given read_file, run_bash, and write_file tools. It attempts to
produce a patch that fixes the given issue. Resolution is determined externally
by the SWE-bench Docker evaluation harness.

Usage:
    python bench/swebench/agent_baseline.py \
        --task-id django__django-11099 \
        --repo-path /path/to/repo \
        --issue "Issue description text..." \
        [--model claude-sonnet-4-6] \
        [--max-turns 20]
"""

import argparse
import json
import os
import subprocess
import time
from pathlib import Path

import anthropic

# Tools available to the baseline agent (no spelunk)
TOOLS = [
    {
        "name": "read_file",
        "description": "Read the contents of a file within the repository.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file, relative to the repo root.",
                }
            },
            "required": ["path"],
        },
    },
    {
        "name": "run_bash",
        "description": (
            "Run a shell command in the repository directory. "
            "Output is truncated to 10 000 characters."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute.",
                }
            },
            "required": ["command"],
        },
    },
    {
        "name": "write_file",
        "description": "Write content to a file within the repository.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file, relative to the repo root.",
                },
                "content": {
                    "type": "string",
                    "description": "Full content to write to the file.",
                },
            },
            "required": ["path", "content"],
        },
    },
]

MAX_OUTPUT_CHARS = 10_000


def read_file(repo_path: Path, path: str) -> str:
    target = (repo_path / path).resolve()
    # Prevent path traversal outside the repo
    if not str(target).startswith(str(repo_path.resolve())):
        return "Error: path is outside the repository."
    try:
        return target.read_text(errors="replace")
    except Exception as e:
        return f"Error reading file: {e}"


def run_bash(repo_path: Path, command: str) -> str:
    try:
        result = subprocess.run(
            command,
            shell=True,
            cwd=repo_path,
            capture_output=True,
            text=True,
            timeout=60,
        )
        output = result.stdout + result.stderr
    except subprocess.TimeoutExpired:
        output = "Error: command timed out after 60 seconds."
    except Exception as e:
        output = f"Error running command: {e}"
    # Truncate long output so it doesn't blow up the context window
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


def dispatch_tool(repo_path: Path, tool_name: str, tool_input: dict) -> str:
    if tool_name == "read_file":
        return read_file(repo_path, tool_input["path"])
    elif tool_name == "run_bash":
        return run_bash(repo_path, tool_input["command"])
    elif tool_name == "write_file":
        return write_file(repo_path, tool_input["path"], tool_input["content"])
    else:
        return f"Unknown tool: {tool_name}"


def run_agent(
    task_id: str,
    repo_path: Path,
    issue_text: str,
    model: str,
    max_turns: int,
) -> dict:
    client = anthropic.Anthropic()

    system_prompt = (
        "You are an expert software engineer. You are given a GitHub issue and a "
        "repository checkout. Your goal is to produce a minimal patch that fixes the "
        "issue. Use the available tools to explore the codebase, understand the problem, "
        "and apply the fix. When you are done, briefly summarise what you changed."
    )

    user_message = (
        f"Repository path: {repo_path}\n\n"
        f"Issue:\n{issue_text}\n\n"
        "Please investigate the issue and apply a fix."
    )

    messages = [{"role": "user", "content": user_message}]

    turns = 0
    input_tokens = 0
    output_tokens = 0
    start = time.monotonic()

    while turns < max_turns:
        response = client.messages.create(
            model=model,
            max_tokens=4096,
            system=system_prompt,
            tools=TOOLS,
            messages=messages,
        )

        input_tokens += response.usage.input_tokens
        output_tokens += response.usage.output_tokens
        turns += 1

        # Append the assistant turn
        messages.append({"role": "assistant", "content": response.content})

        # If the model stopped without requesting a tool, we're done
        if response.stop_reason == "end_turn":
            break

        # Process tool calls and collect results
        tool_results = []
        for block in response.content:
            if block.type == "tool_use":
                result = dispatch_tool(repo_path, block.name, block.input)
                tool_results.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": block.id,
                        "content": result,
                    }
                )

        if not tool_results:
            # No tools requested and stop_reason wasn't end_turn — stop anyway
            break

        messages.append({"role": "user", "content": tool_results})

    wall_seconds = time.monotonic() - start

    return {
        "task_id": task_id,
        # Resolution is always false here — the SWE-bench harness determines this
        "resolved": False,
        "turns": turns,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "wall_seconds": round(wall_seconds, 2),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Baseline SWE-bench agent (no spelunk).")
    parser.add_argument("--task-id", required=True, help="SWE-bench task instance ID.")
    parser.add_argument("--repo-path", required=True, help="Path to the repository checkout.")
    parser.add_argument("--issue", required=True, help="Issue description text.")
    parser.add_argument("--model", default="claude-sonnet-4-6", help="Claude model ID.")
    parser.add_argument("--max-turns", type=int, default=20, help="Maximum agent turns.")
    args = parser.parse_args()

    repo_path = Path(args.repo_path).resolve()
    if not repo_path.is_dir():
        parser.error(f"repo-path does not exist or is not a directory: {repo_path}")

    result = run_agent(
        task_id=args.task_id,
        repo_path=repo_path,
        issue_text=args.issue,
        model=args.model,
        max_turns=args.max_turns,
    )

    print(json.dumps(result))


if __name__ == "__main__":
    main()
