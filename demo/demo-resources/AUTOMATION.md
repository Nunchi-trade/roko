# Demo Automation

This directory has two layers:

- `bin/common.sh` contains shared shell helpers for paths, HTTP JSON requests,
  JSON assertions, temporary workspaces, free ports, and background serve
  lifecycle.
- `bin/roko-demo` is the reusable command entrypoint for smoke tests, seeding,
  disposable local verification, and the older interactive demo scripts.

The compatibility scripts `run-all.sh`, `smoke-test.sh`, `bin/roko-up.sh`,
`bin/roko-smoke.sh`, and `bin/roko-demo.sh` route through that shared path.

## Fast Start

```bash
# Check prerequisites.
demo/demo-resources/bin/roko-demo doctor

# Build the roko binary used by demos.
demo/demo-resources/bin/roko-demo build

# Start an isolated serve, seed agents, and verify dashboard-facing APIs.
demo/demo-resources/bin/roko-demo verify-local
```

`verify-local` is the best default proof that roko-serve can back the dashboard.
It creates a temporary workspace, starts `roko serve` on a free local port,
seeds agents, warms the agent aggregator cache, registers a new smoke agent, and
verifies that the dashboard-facing APIs can see and use it.

## Existing Server

Use these against a serve you already started:

```bash
demo/demo-resources/bin/roko-demo seed-agents http://127.0.0.1:6677
demo/demo-resources/bin/roko-demo dashboard-smoke http://127.0.0.1:6677

# Compatibility entrypoints:
bash demo/demo-resources/smoke-test.sh http://127.0.0.1:6677
bash demo/demo-resources/run-all.sh http://127.0.0.1:6677
```

`dashboard-smoke` registers one `demo-smoke-*` agent and creates one
`dashboard smoke lifecycle` job in the target workspace. Use `verify-local`
when you want those writes confined to a disposable workspace.

## Commands

| Command | Purpose |
|---|---|
| `help` | Print usage and environment variables. |
| `list` | Show short names for wrapped demo scripts. |
| `doctor` | Check Python, Cargo, the roko binary, repo layout, and optional serve reachability. |
| `build` | Run `cargo build -p roko-cli`. |
| `serve [workdir] [port]` | Run `roko serve` in the foreground for a workspace. |
| `seed-agents [base-url]` | Register the reusable five-agent demo fleet. |
| `dashboard-smoke [base-url]` | Verify dashboard-facing APIs on an already-running serve. |
| `verify-local [port]` | Start disposable serve, seed agents, and run `dashboard-smoke`. |
| `bench [workdir]` | Run the SWE-bench proxy gold/empty/command controls and C-factor proof. |
| `run <name> [args...]` | Execute an existing demo script by short name. |
| `all [base-url]` | Seed agents, then run the benchmark, PRD, research, and full-loop demos. |

## Wrapped Demo Names

```bash
demo/demo-resources/bin/roko-demo list
demo/demo-resources/bin/roko-demo run match
demo/demo-resources/bin/roko-demo run lifecycle
demo/demo-resources/bin/roko-demo run single-agent
demo/demo-resources/bin/roko-demo run multi-agent
demo/demo-resources/bin/roko-demo run prd
demo/demo-resources/bin/roko-demo run prd-api
demo/demo-resources/bin/roko-demo run research
demo/demo-resources/bin/roko-demo run fleet
demo/demo-resources/bin/roko-demo run full
demo/demo-resources/bin/roko-demo run smoke
demo/demo-resources/bin/roko-demo run bench
demo/demo-resources/bin/roko-demo run bench-controls --workdir /tmp/roko-bench-controls
demo/demo-resources/bin/roko-demo run coding-bench --model gemma4:26b-moe-nothink
demo/demo-resources/bin/roko-demo run bench-summary --workdir /tmp/roko-bench-controls
```

Some wrapped scripts remain interactive or require extra service/API-key setup.
The reusable `seed-agents`, `dashboard-smoke`, `verify-local`, `run-all.sh`, and
`smoke-test.sh` paths avoid `curl` and use Python's standard library for HTTP.

## Benchmark Automation

The benchmark flow is the reusable proof path for `roko bench swe` and C-factor
telemetry. It does not need `roko serve`.

```bash
demo/demo-resources/bin/roko-demo run bench
# or:
bash demo/demo-resources/benchmark-flow/demo-benchmark.sh /tmp/roko-bench-demo
```

It runs three batches in one workspace:

| Batch | Agent mode | Purpose | Expected proxy score |
|---|---|---|---:|
| Positive control | `gold` | Uses the built-in gold patches | `2/2` |
| Negative control | `empty` | Proves empty patches fail and lower C-factor | `0/2` |
| Command adapter | `command` | Wraps a process that receives instance JSON and prints a patch | `2/2` |

The script validates the latest score rows and C-factor movement, then prints a
`roko status --cfactor` snapshot. Use `benchmark-flow/README.md` for the custom
dataset JSONL schema, artifact paths, and official SWE-bench export notes.

## Coding-Agent Benchmarks

`coding-agent-benchmarks/` turns command mode into a local coding-agent loop.
The adapter copies the benchmark repo, configures `roko run` for an Ollama
model, and prints the resulting `git diff` for scoring.

```bash
demo/demo-resources/bin/roko-demo run bench-controls --workdir /tmp/roko-bench-controls
demo/demo-resources/bin/roko-demo run coding-bench \
  --model gemma4:26b-moe-nothink \
  --workdir /tmp/roko-coding-bench \
  --knowledge-workdir .
demo/demo-resources/bin/roko-demo run bench-summary --workdir /tmp/roko-coding-bench
```

`bench-controls` validates gold `2/2` and empty `0/2` before model results are
interpreted. `coding-bench` compares `minimal`, `context`, and `neuro` modes by
default; pass `--mode minimal` or another mode to run only one slice. Set
`RUN_OLLAMA_BENCH=1` when you want `run-all.sh` to include the Ollama-backed
benchmark.

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `ROKO` | `<repo>/target/debug/roko` | CLI binary used by demos. |
| `PYTHON` | `python3` | Python executable for JSON parsing and HTTP calls. |
| `ROKO_SERVE_URL` | `http://127.0.0.1:6677` | Base URL for HTTP commands. |
| `HTTP_TIMEOUT_SECS` | `20` | Per-request timeout for HTTP helpers. |
| `BENCH_MODEL` | `gemma4:26b-moe-nothink` | Ollama model used by `coding-bench`. |
| `BENCH_BATCH_SIZE` | `2` | Number of SWE-bench proxy instances to score. |
| `ROKO_OLLAMA_MODEL` | `llama3.2:latest` | Fallback model for direct adapter calls. |
| `ROKO_KNOWLEDGE_WORKDIR` | repo root | Knowledge store queried by the adapter in neuro mode. |

## Dashboard Smoke Coverage

`dashboard-smoke` checks:

- `GET /api/health`
- `GET /api/dashboard`
- `GET /api/managed-agents`
- `GET /api/agents`
- `GET /api/agents/topology`
- `GET /api/projections/dashboard`
- `POST /api/agents/register`
- `POST /api/jobs/match`
- `POST /api/jobs`
- `POST /api/jobs/{id}/assign`
- `POST /api/jobs/{id}/start`
- `POST /api/jobs/{id}/submit`
- `POST /api/jobs/{id}/evaluate`

It intentionally calls `GET /api/agents` before registering the smoke agent and
then calls it again afterward. That catches cache invalidation failures where a
dashboard would keep showing a stale roster after agents register.

## Adding New Automation

Prefer `bin/common.sh` helpers for new scripts:

- `api_url` normalizes base URLs to `/api`.
- `http_get_json` and `http_post_json` make JSON requests without `curl`.
- `json_eval` evaluates small JSON expressions from shell scripts.
- `free_port`, `with_temp_workspace`, `start_roko_serve_bg`, and `stop_pid`
  support isolated local smoke tests.
- `run_script` resolves paths relative to this demo resource directory.
