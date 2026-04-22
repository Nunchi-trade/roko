# Ollama Coding-Agent Benchmark Loop

Reusable workflow for running local Roko coding agents against the built-in
SWE-bench proxy smoke dataset, recording benchmark scores, learning episodes,
C-factor snapshots, and durable neuro knowledge.

This flow is intentionally small and fast. It is a proxy harness for local
iteration, not official SWE-bench Docker scoring and not comparable to
published SWE-bench results.

## Prerequisites

From the repo root:

```bash
cargo build -p roko-cli
ollama list
```

You also need Python 3 and git. Use any Ollama model that Roko can call through
`roko run`; the examples below default to `gemma4:26b-moe-nothink` because it
passed the two-task local smoke set in the latest local run.

## Quick Start

Run benchmark harness controls:

```bash
bash demo/demo-resources/coding-agent-benchmarks/run-controls.sh
```

Run the Ollama coding-agent loop across `minimal`, `context`, and `neuro`:

```bash
bash demo/demo-resources/coding-agent-benchmarks/run-ollama-bench.sh \
  --model gemma4:26b-moe-nothink
```

Summarize the latest score rows and knowledge store:

```bash
bash demo/demo-resources/coding-agent-benchmarks/summarize-bench.sh
target/debug/roko knowledge query benchmark --workdir .
```

## What The Modes Compare

| Mode | What changes |
|---|---|
| `minimal` | Sends only the problem statement and validation command to `roko run`. |
| `context` | Adds direct source-file context from the isolated benchmark repo. |
| `neuro` | Adds source-file context plus matching entries from `roko knowledge query`. |

The agent adapter copies each benchmark repo into a temp directory, writes a
small `roko.toml` configured for Ollama, runs `roko run`, then emits `git diff`
to stdout. `roko bench swe --agent-mode command` scores that diff by checking
format, `git apply --check`, patch application, and the task test command.

## Controls

`run-controls.sh` executes:

| Batch | Agent mode | Expected result |
|---|---|---:|
| Gold oracle | `--agent-mode gold` | `2/2` on the built-in smoke dataset |
| Empty patch | `--agent-mode empty` | `0/2` on the built-in smoke dataset |

Run these first when changing benchmark code. If the controls do not match,
debug the harness before interpreting model numbers.

## Outputs

By default, scripts use the repo root as `--workdir` and write:

```text
.roko/
|-- bench/
|   |-- scores-*.jsonl
|   |-- predictions-*.jsonl
|   `-- runs/*.json
|-- learn/
|   |-- episodes.jsonl
|   |-- task-metrics.jsonl
|   |-- efficiency.jsonl
|   `-- c-factor.jsonl
`-- neuro/
    `-- knowledge.jsonl
```

Use `--workdir /tmp/somewhere` on the scripts to isolate a run from the repo
root. Use `--knowledge-workdir .` on `run-ollama-bench.sh` if you want neuro
mode to reuse the repo root knowledge store while writing benchmark artifacts
elsewhere.

## Environment And Flags

| Name | Default | Purpose |
|---|---|---|
| `ROKO` | `target/debug/roko` | CLI binary used by demo scripts. |
| `PYTHON` | `python3` | Python executable for the command adapter and summaries. |
| `BENCH_MODEL` | `gemma4:26b-moe-nothink` | Ollama model for `run-ollama-bench.sh`. |
| `BENCH_BATCH_SIZE` | `2` | Number of benchmark instances to run. |
| `ROKO_OLLAMA_MODEL` | `llama3.2:latest` | Fallback model for direct adapter calls. |

Script flags override environment variables:

```bash
bash demo/demo-resources/coding-agent-benchmarks/run-ollama-bench.sh \
  --model llama3.2:latest \
  --mode context \
  --batch-size 2 \
  --workdir /tmp/roko-agent-bench \
  --knowledge-workdir .
```

`--mode` can be passed multiple times to run a subset:

```bash
bash demo/demo-resources/coding-agent-benchmarks/run-ollama-bench.sh \
  --model gemma4:26b-moe-nothink \
  --mode minimal \
  --mode neuro
```

## Latest Local Smoke Results

These are example results from a local run on the built-in two-task smoke set:

| Approach | Result | Notes |
|---|---:|---|
| gold control | `2/2` | Harness positive control. |
| empty control | `0/2` | Harness negative control. |
| `llama3.2:latest` minimal | `0/2` | Produced no usable patch. |
| `llama3.2:latest` context | `0/2` | Produced no usable patch. |
| `gemma4:26b-moe-nothink` minimal | `2/2` | Passed both tasks. |
| `gemma4:26b-moe-nothink` context | `2/2` | Passed both tasks. |
| `gemma4:26b-moe-nothink` neuro | `2/2` | Passed both tasks with knowledge injection enabled. |

Treat these numbers as harness checks, not statistical proof. Add larger local
JSONL datasets before drawing model-selection conclusions.

## Custom Dataset

You can bypass the wrapper scripts and call `roko bench swe` directly:

```bash
target/debug/roko bench swe \
  --dataset ./my-swe-smoke.jsonl \
  --batch-size 5 \
  --agent-mode command \
  --agent-command 'python3 demo/demo-resources/coding-agent-benchmarks/roko-ollama-patch-agent.py --mode neuro --model gemma4:26b-moe-nothink --knowledge-workdir .'
```

Each JSONL row should include:

```json
{
  "instance_id": "local__case-1",
  "repo": "local/example",
  "repo_path": "./fixtures/case-1",
  "problem_statement": "Fix the failing behavior.",
  "patch": "diff --git a/file.py b/file.py\n...",
  "test_cmd": "python3 -m unittest"
}
```

## Troubleshooting

| Issue | Fix |
|---|---|
| `roko binary not found` | Run `cargo build -p roko-cli` or set `ROKO=/path/to/roko`. |
| Ollama model fails immediately | Check `ollama list`, start Ollama, or pass `--model <installed-model>`. |
| Command mode produces no patch | Run with `--mode context` or inspect `.roko/bench/runs/<run>.json` for stderr. |
| Neuro mode shows no knowledge | Run controls or previous benchmark runs with learning enabled, then `roko knowledge stats --workdir .`. |
| Controls fail | Fix `roko bench swe` or the built-in smoke dataset before comparing models. |
