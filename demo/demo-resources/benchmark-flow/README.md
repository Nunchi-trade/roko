# SWE-bench Proxy + C-Factor Demo

Reusable flow for exercising the native benchmark path without Docker, HuggingFace, or a live LLM.

This demo uses `roko bench swe` against the built-in two-task SWE-bench-style smoke dataset. It proves the process wiring:

1. Generate or receive a patch for each benchmark instance.
2. Validate patch format.
3. Run `git apply --check`.
4. Apply the patch.
5. Run the task test command.
6. Persist score rows under `.roko/bench`.
7. Persist learning episodes, efficiency events, and C-factor snapshots under `.roko/learn`.

This is fast proxy scoring, not official SWE-bench Docker scoring and not comparable to swebench.com.

## Quick Start

From the repo root:

```bash
cargo build -p roko-cli
bash demo/demo-resources/benchmark-flow/demo-benchmark.sh
```

To reuse a specific workspace:

```bash
bash demo/demo-resources/benchmark-flow/demo-benchmark.sh /tmp/roko-bench-demo
```

You can also run it through the demo wrapper:

```bash
bash demo/demo-resources/bin/roko-demo run bench
```

## What The Script Runs

`demo-benchmark.sh` runs three batches in the same workspace:

| Batch | Agent mode | Purpose | Expected proxy score |
|---|---|---:|---:|
| Gold oracle | `--agent-mode gold` | Positive control; uses the dataset patch | `2/2` |
| Empty patch | `--agent-mode empty` | Negative control; proves failed patches do not pass | `0/2` |
| Command adapter | `--agent-mode command` | Process adapter smoke; command receives instance JSON on stdin and prints a patch | `2/2` |

The command adapter in this demo is intentionally an oracle command:

```bash
python3 -c 'import sys,json; print(json.load(sys.stdin)["patch"], end="")'
```

Replace it with your own agent command when testing a real patch-producing process.

## Artifacts

Given `WORKDIR=/tmp/roko-bench-demo`, the demo writes:

```text
$WORKDIR/
├── .roko/
│   ├── bench/
│   │   ├── scores.jsonl          # aggregate score row per run
│   │   └── runs/*.json           # full per-instance details
│   └── learn/
│       ├── episodes.jsonl        # benchmark episodes
│       ├── efficiency.jsonl      # benchmark efficiency events
│       └── c-factor.jsonl        # C-factor snapshots
├── predictions-gold.jsonl
└── predictions-command.jsonl
```

Inspect the aggregate rows:

```bash
tail -n 3 /tmp/roko-bench-demo/.roko/bench/scores.jsonl
```

Inspect the C-factor and contributors:

```bash
./target/debug/roko status --workdir /tmp/roko-bench-demo --cfactor
```

Expected pattern:

- gold succeeds and creates the initial C-factor snapshot;
- empty fails and drops C-factor;
- command succeeds and partially recovers C-factor;
- `swe-bench-empty` shows as a negative C-factor contributor.

## Custom Dataset JSONL

Pass a local dataset file directly to `roko bench swe`:

```bash
./target/debug/roko bench swe \
  --dataset ./my-swe-smoke.jsonl \
  --batch-size 5 \
  --agent-mode command \
  --agent-command './my-agent-command' \
  --workdir /tmp/roko-bench-custom
```

Each JSONL row should have this shape:

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

`repo_path` is copied into an isolated benchmark workdir before patch validation. Relative `repo_path` values are resolved relative to the dataset file.

## Agent Modes

| Mode | Flag | Use |
|---|---|---|
| Gold | `--agent-mode gold` | Harness positive control |
| Empty | `--agent-mode empty` | Harness negative control |
| Prediction file | `--agent-mode prediction-file --predictions predictions.jsonl` | Replay existing SWE-bench-style predictions |
| Command | `--agent-mode command --agent-command '<cmd>'` | Wrap a real agent or script |

For command mode, Roko writes one instance JSON object to stdin and expects a unified diff on stdout.

## Official SWE-bench

Use this demo for fast local harness verification and learning telemetry. For publishable SWE-bench numbers, export predictions and run the official Python/Docker harness separately:

```bash
./target/debug/roko bench swe \
  --batch-size 300 \
  --agent-mode command \
  --agent-command './my-agent-command' \
  --export-predictions /tmp/predictions.jsonl

python -m swebench.harness.run_evaluation \
  --predictions_path /tmp/predictions.jsonl \
  --dataset_name princeton-nlp/SWE-bench_Lite \
  --run_id roko_proxy_export
```

## Troubleshooting

| Issue | Fix |
|---|---|
| `roko binary not found` | Run `cargo build -p roko-cli` or set `ROKO=/path/to/roko` |
| `git apply` fails for gold mode | Rebuild Roko and rerun; gold mode should always pass the built-in smoke dataset |
| Empty mode raises C-factor | Rebuild Roko; C-factor overall should be bounded by benchmark gate pass rate |
| Command mode hangs | Your command likely did not close stdout or is waiting for input; it receives exactly one JSON object on stdin |
| Official SWE-bench score differs from proxy | Expected; proxy validates format/apply/tests locally, while official scoring uses task containers and SWE-bench test metadata |
