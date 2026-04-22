#!/usr/bin/env python3
"""Command adapter for `roko bench swe --agent-mode command`.

The benchmark harness sends one instance JSON object on stdin and expects a
unified diff on stdout. This adapter runs `roko run` against an isolated copy of
the benchmark repo, then prints the resulting `git diff`.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]


def run(cmd, cwd=None, input_text=None, timeout=120):
    return subprocess.run(
        cmd,
        cwd=cwd,
        input=input_text,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )


def file_context(root):
    chunks = []
    for path in sorted(root.iterdir()):
        if not path.is_file() or path.name == "roko.toml":
            continue
        try:
            text = path.read_text()
        except UnicodeDecodeError:
            continue
        chunks.append(f"### {path.name}\n```text\n{text}\n```")
    return "\n\n".join(chunks)


def query_knowledge(roko_bin, knowledge_workdir, topic):
    if not knowledge_workdir:
        return ""
    result = run(
        [
            str(Path(roko_bin).resolve()),
            "knowledge",
            "query",
            topic,
            "--workdir",
            str(Path(knowledge_workdir).resolve()),
        ],
        timeout=30,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        return ""
    return result.stdout.strip()


def write_roko_config(root, model, test_cmd):
    escaped_test = json.dumps(["-lc", test_cmd])
    root.joinpath("roko.toml").write_text(
        f"""[agent]
command = "ollama"
model = "{model}"
timeout_ms = 120000
bare_mode = true
clean_output = true

[prompt]
token_budget = 6000
role = "implementer"

[[gate]]
kind = "shell"
program = "sh"
args = {escaped_test}
timeout_ms = 60000
"""
    )


def build_prompt(instance, workdir, mode, roko_bin, knowledge_workdir):
    problem = instance.get("problem_statement", "")
    test_cmd = instance.get("test_cmd") or instance.get("test_command") or "true"
    prompt = (
        "Fix this small repository so the benchmark test passes. "
        "Edit the implementation files directly. Do not modify tests unless the problem explicitly asks for it.\n\n"
        f"Problem:\n{problem}\n\n"
        f"Validation command:\n{test_cmd}\n"
    )
    if mode in {"context", "neuro"}:
        context = file_context(workdir)
        if context:
            prompt += "\nRelevant repository context:\n" + context
    if mode == "neuro":
        topic = f"benchmark code repair {problem}"
        knowledge = query_knowledge(roko_bin, knowledge_workdir, topic)
        if knowledge:
            prompt += "\n\nRelevant learned knowledge:\n" + knowledge
        prompt += (
            "\n\nBenchmark guidance:\n"
            "- Make the smallest implementation change that satisfies the stated failing behavior.\n"
            "- Preserve public function names and test files.\n"
            "- Run or reason against the validation command before stopping.\n"
        )
    return prompt


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=["minimal", "context", "neuro"], default="context")
    parser.add_argument(
        "--model",
        default=os.environ.get("ROKO_OLLAMA_MODEL", "llama3.2:latest"),
    )
    parser.add_argument(
        "--roko-bin",
        default=os.environ.get("ROKO_BIN") or os.environ.get("ROKO") or str(REPO_ROOT / "target/debug/roko"),
    )
    parser.add_argument(
        "--knowledge-workdir",
        default=os.environ.get("ROKO_KNOWLEDGE_WORKDIR", str(REPO_ROOT)),
        help="Workdir whose .roko/neuro store should be queried in neuro mode.",
    )
    args = parser.parse_args()

    instance = json.load(sys.stdin)
    source = Path(instance["repo_path"]).resolve()
    test_cmd = instance.get("test_cmd") or instance.get("test_command") or "true"

    with tempfile.TemporaryDirectory(prefix="roko-bench-agent-") as tmp:
        workdir = Path(tmp) / "repo"
        shutil.copytree(source, workdir)
        run(["git", "init", "-q"], cwd=workdir)
        run(["git", "add", "."], cwd=workdir)
        write_roko_config(workdir, args.model, test_cmd)

        prompt = build_prompt(instance, workdir, args.mode, args.roko_bin, args.knowledge_workdir)
        result = run(
            [
                str(Path(args.roko_bin).resolve()),
                "--config",
                str(workdir / "roko.toml"),
                "--repo",
                str(workdir),
                "--quiet",
                "run",
                prompt,
            ],
            cwd=workdir,
            timeout=180,
        )
        if result.returncode != 0:
            sys.stderr.write(result.stderr)

        diff = run(["git", "diff", "--no-ext-diff", "--", "."], cwd=workdir).stdout
        sys.stdout.write(diff)


if __name__ == "__main__":
    main()
