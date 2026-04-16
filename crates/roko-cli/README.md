# roko-cli

The `roko` binary. Drives every Roko subsystem from the terminal: the
universal loop (`roko run`), plan execution (`roko plan run`), PRD
lifecycle (`roko prd …`), research (`roko research …`), HTTP control
plane (`roko serve`), per-agent chat (`roko chat`), and an interactive
ratatui dashboard (`roko dashboard`).

If it's a Roko feature, `roko-cli` is the way you reach it from a
command line.

## Install

From a clone of this workspace:

```bash
cargo install --path crates/roko-cli              # release build into ~/.cargo/bin/roko
# or for iteration:
cargo install --path crates/roko-cli --debug --force
```

The binary is named `roko` (not `roko-cli`), so `roko --help` works globally once installed.

For developing on the CLI itself, a shell function that always uses the latest source is simpler than reinstalling on every change:

```zsh
roko() {
  cargo run --quiet --manifest-path ~/path/to/roko/Cargo.toml -p roko-cli -- "$@"
}
```

## Subcommands

### Core loop

```
roko init [path]              # create .roko/signals.jsonl and a starter roko.toml
roko run <prompt>             # run the universal loop once; prints prompt id, verdicts, episode id
roko status                   # signal counts by kind, latest episode, gate pass/fail totals
roko replay <hash>            # walk the lineage DAG rooted at a signal hash
roko config <cmd>             # manage global + project config (see below)
```

### Plan execution (orchestrator)

```
roko plan list                 # list discovered plans
roko plan show <id>            # show plan details
roko plan create               # scaffold a new plan
roko plan run <dir>            # execute plans end-to-end
roko plan run <dir> --resume .roko/state/executor.json   # resume after interruption
```

### PRD lifecycle (self-hosting workflow)

```
roko prd idea "<text>"         # capture an idea
roko prd list                  # list PRDs
roko prd status                # coverage report (plans/tasks/done ratio)
roko prd draft new "<title>"   # agent-assisted draft
roko prd draft promote         # promote draft to published
roko prd plan <slug>           # generate implementation plan + tasks.toml
roko prd consolidate           # consolidate overlapping PRDs
```

### Research

```
roko research topic "<topic>"            # deep research with citations
roko research enhance-prd <slug>         # enrich a PRD with research
roko research enhance-plan <plan>        # optimise a plan
roko research enhance-tasks <plan>       # split / rebalance tasks
roko research analyze                    # analyse execution data
```

### HTTP control plane + chat

```
roko serve                     # start HTTP API on :6677 (see crates/roko-serve/README.md)
roko serve --bind 0.0.0.0 --port 9090
roko chat --agent <id>         # interactive REPL with a running agent (via sidecar)
roko chat --agent <id> --serve-url http://localhost:6677
```

### Dashboard

```
roko dashboard                 # interactive ratatui TUI: 7 tabs, F1–F7
```

Tabs: **F1** Dashboard — **F2** Plans — **F3** Agents — **F4** Git —
**F5** Logs — **F6** Config — **F7** Inspect. Keys: `q` quit, `?` help,
`Tab`/`Shift+Tab` cycle panels, `Enter` drill in, `i` inject, `Ctrl-C`
force-quit (always, even through modals — see T17).

Every subcommand except `init` reads config from the layered
global+project chain described below.

## Config system

Config lives in two layered TOML files, merged field-by-field. Project values shadow global values; missing fields fall through to global, then to built-in defaults.

| Layer | Path | Purpose |
| --- | --- | --- |
| Global | `$XDG_CONFIG_HOME/roko/config.toml` (or `~/.config/roko/config.toml`) | Machine-wide defaults: agent backend, token budget, role persona |
| Project | `./roko.toml` (or nearest ancestor dir containing one) | Per-repo overrides |
| Env | `$ROKO_CONFIG=/abs/path` | Bypass both layers and use this single file |

The `gates` array is a full replacement, not a merge — a project that defines `[[gate]]` replaces the whole global gate list.

### `roko config init` — interactive wizard

Detects which LLM CLIs are on your `PATH` (claude, ollama, mods, llm, aichat, cat), prompts for a model (if ollama), token budget, role persona, and default gates, then writes a global config:

```
$ roko config init
Detected agent backends:
  [1] claude — Claude CLI (anthropic)
  [2] ollama — Ollama (local models)
  [3] cat    — cat (echo; smoke tests only)
Pick an agent backend (number or name) [claude]: 2
Ollama model (e.g. llama3, codellama, qwen2.5-coder) [llama3]: qwen2.5-coder
Token budget for prompt composition [8000]:
System role / persona [You are a Roko agent…]:
Enable default cargo gates (compile + clippy)? [y/N]: y

--- generated config ---
[agent]
command = "ollama"
args = ["run", "qwen2.5-coder"]

[prompt]
token_budget = 8000
role = "You are a Roko agent — concise, precise, and correct."

[[gate]]
kind = "compile"
build_system = "cargo"
timeout_ms = 600000

[[gate]]
kind = "clippy"
build_system = "cargo"
timeout_ms = 600000
--- end config ---

Write to /Users/you/.config/roko/config.toml? [Y/n]:
wrote /Users/you/.config/roko/config.toml
```

For CI / scripts:

```bash
roko config init --non-interactive --agent ollama --model qwen2.5-coder --budget 8000 --enable-gates --yes
```

### `roko config show` — effective config with provenance

```
$ roko config show
effective config:
  agent.command      = "ollama" [global]
  agent.args         = ["run", "qwen2.5-coder"] [global]
  agent.timeout_ms   = 120000 [default]
  prompt.token_budget= 16000 [project]
  prompt.role        = "You are a Roko agent." [global]
  gates              = 2 entries [global]

sources:
  global : /Users/you/.config/roko/config.toml
  project: /Users/you/work/myproject/roko.toml
```

Each tag (`[global]`, `[project]`, `[default]`, `[env]`) shows exactly which layer supplied that field.

### `roko config path`

Prints the resolved global path (and whether it exists), the nearest project `roko.toml`, and any `ROKO_CONFIG` env override.

### `roko config edit` / `roko config set`

```bash
roko config edit                       # opens $EDITOR on project roko.toml if present, else global
roko config edit --global              # force global
roko config edit --project             # force project (creates roko.toml if missing)

roko config set agent.command ollama                    # writes to global by default
roko config set agent.args '["run","llama3"]' --global
roko config set prompt.token_budget 16000 --project
```

Supported keys: `agent.command`, `agent.args`, `agent.timeout_ms`, `prompt.token_budget`, `prompt.role`.

## Empty-directory workflow

From scratch, with no LLM set up yet:

```bash
$ mkdir /tmp/fresh && cd /tmp/fresh
$ roko run "hello"
no config found — using built-in `cat` agent. run `roko config init` to set up a model.
running agent `cat` with 0 gate(s)
---
agent        : cat (success=true)
prompt_id    : 7a6567df22…
agent_output : 1b770372d4…
gates        : (none configured)
episode      : b039b24ee1…
signals      : 6
```

The built-in `cat` fallback runs the full 7-primitive loop without any external dependency — useful for verifying that Roko is wired correctly and that your terminal understands the signal output format. Once you've run `roko config init`, the same command invokes a real model.

## Signal layout

A single `roko run` produces these signals under `.roko/signals.jsonl`, all linked via lineage:

```
PromptSection(role)  ─┐
PromptSection(file)* ─┼→ Prompt ─→ AgentOutput ─→ Task(gate input) ─→ GateVerdict*  ─┐
PromptSection(task)  ─┘    (cleaned)       │                                         ├→ Episode
                                           └→ AgentMessage (raw stdout, if cleaned)  ┘
```

`roko status` aggregates these into counts. `roko replay <hash>` walks the DAG from any starting signal.

## Running against ollama

Point `[agent].command` at `ollama run` and drop the model name into `args`.
Tested against the following local models:

| Model | Size | Output behavior |
|---|---|---|
| `llama3.2:latest` | 2 GB | Clean code blocks, no reasoning trace |
| `gemma4:26b-a4b-it-q8_0` | 28 GB | Reasoning trace + terminal escapes |
| `gemma4:26b-moe-nothink` | 28 GB | No reasoning trace variant |
| `glm-4.7-flash:latest` | 19 GB | Long reasoning trace |

### Non-reasoning model

```toml
[agent]
command = "ollama"
args = ["run", "llama3.2:latest"]
timeout_ms = 300000
```

```bash
$ roko run "write a Rust function that returns the string hi"
```

Output lands directly in the `AgentOutput` signal — no post-processing needed.

### Reasoning model (auto-cleaned by default)

Reasoning models emit chain-of-thought AND, when run through `ollama run`,
cursor-movement escapes from ollama's progress spinner. The CLI's
`clean_output = true` (default) strips both and keeps the canonical answer.

```toml
[agent]
command = "ollama"
args = ["run", "gemma4:26b-a4b-it-q8_0"]
timeout_ms = 300000
clean_output = true                  # default; strip ANSI + thinking
```

```bash
$ roko run "what is the BLAKE3 hash of the empty string?"
```

Without cleaning, the persisted `AgentOutput` was **1935 bytes of thinking
trace + ANSI escapes**. With cleaning, it's **64 bytes**:
`af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`. The raw
output is preserved as a separate `AgentMessage` trace so nothing is lost.

**What the cleaner does** (see `src/clean.rs`):
- Emulates a line-buffered terminal for `CSI {n}D`, `CSI K`, `CSI {n}C`
  (cursor-back + clear-to-EOL is how `ollama run` line-wraps its output)
- Strips colors, OSC title sequences, and other CSI params
- Removes `Thinking... … ...done thinking.` blocks
- Removes `<think>…</think>` and `<thinking>…</thinking>` tags
- Trims leading/trailing whitespace

If you want to disable cleaning and see the raw text, set `clean_output = false`.

## Env vars for the agent subprocess

Some models / CLIs are tunable via environment:

```toml
[agent]
command = "ollama"
args = ["run", "llama3.2:latest"]
env = [
  ["OLLAMA_HOST", "http://localhost:11434"],
  ["OLLAMA_NOPROGRESS", "1"],        # if your ollama honors it
]
```

Use the same mechanism to pass API keys to remote LLM CLIs:

```toml
[agent]
command = "claude"
args = ["-p"]
env = [["ANTHROPIC_API_KEY", "sk-ant-..."]]  # or read from your .env
```

## File injection: `[[prompt.files]]`

Inject file contents into the prompt as additional `PromptSection`s. This
is how you feed issue descriptions, buggy source, workspace context, etc.
into the agent.

```toml
[[prompt.files]]
path = "issue.md"                  # relative to workdir (or absolute)
name = "issue"                     # section label; defaults to the path
priority = "high"                  # low | normal | high | critical
# hard_cap = 2000                  # optional per-file token cap (truncates)
```

Each file is wrapped with a `File `path`:` header and passed through the
composer under the configured `token_budget`. Critical-priority files
will never be dropped; lower priorities drop first under budget pressure.

## SWE-bench-style workflow

With `[[prompt.files]]` and the `test` gate, you can build a single-shot
issue-to-fix loop entirely from config.

```
/tmp/swe-fib/
├── Cargo.toml
├── src/lib.rs           # buggy fib() function
├── issue.md             # bug report text
└── roko.toml
```

```toml
# roko.toml
[agent]
command = "ollama"
args = ["run", "llama3.2:latest"]
timeout_ms = 300000
clean_output = true

[prompt]
token_budget = 20000
role = """You are a senior Rust engineer fixing a bug in a small crate.
You will see an issue report and the current contents of the buggy file.
Respond with the complete fixed file contents inside a ```rust code block.
Be concise - no long explanations."""
files = [
  { path = "issue.md", name = "issue", priority = "high" },
  { path = "src/lib.rs", name = "buggy_file", priority = "high" },
]

[[gate]]
kind = "test"
build_system = "cargo"
timeout_ms = 300000
```

```bash
$ cargo test                       # confirm the bug
test result: FAILED. 0 passed; 2 failed; ...

$ roko run "Fix the bug described in the issue."
running agent `ollama` with 1 gate(s)
---
agent        : ollama (success=true)
prompt_id    : d23549d5...
agent_output : 315c9d38...
gates:
  [FAIL] test:cargo                # the file hasn't been patched yet
episode      : 4a263905...
signals      : 10
```

The agent's cleaned output (visible via `roko replay <episode_id>`) contains
the fixed `lib.rs` inside a ```rust block. Paste it into `src/lib.rs`, rerun,
and the gate passes:

```bash
$ roko run "verify the fix"
gates:
  [PASS] test:cargo
```

**Verified working**: llama3.2:latest produces a correct fix for this
fib()-off-by-one scenario on the first shot.

### What this flow **does** cover

- ✅ Injects issue text + buggy source as content-addressed prompt sections
- ✅ Persists the LLM response as an `AgentOutput` signal with full lineage
- ✅ Runs `cargo test` as a gate against the (possibly-patched) workdir
- ✅ Emits an Episode signal recording everything — prompt, output, verdicts
- ✅ Works with any local ollama model (and any stdin/stdout LLM CLI)

### What it does **not** yet cover

- ❌ **Automatic patch application.** The agent's response is persisted
  verbatim; you must apply it to the filesystem manually. The natural next
  step is an `apply_patch = true` flag that extracts ```diff or ```rust
  blocks from the output and runs `git apply` (or writes the file) before
  the gates run.
- ❌ **Agentic loops.** One prompt → one response → one gate. No retry,
  no multi-turn, no reflection.
- ❌ **Tool calls.** `ExecAgent` is stdin/stdout only; models can't call
  tools through it.

### Actual SWE-bench scoring

If you want to produce scores that compare to the
[SWE-bench leaderboard](https://www.swebench.com/), there's a driver
script at **[`scripts/swebench_run.py`](scripts/README.md)** that:

1. Loads SWE-bench-Lite (300 real GitHub issues) from HuggingFace
2. Oracle-retrieves the files touched by the gold patch
3. Clones the target repo at the correct commit into a workdir
4. Invokes `roko run` with the issue + oracle files injected as sections
5. Extracts a unified diff from the model's response
6. Writes `predictions.jsonl` in SWE-bench's expected format

You then score with the official harness
(`python -m swebench.harness.run_evaluation`), which runs each task's
tests inside a Docker container and reports a resolved-instance rate
comparable to [swebench.com](https://www.swebench.com) entries.

```bash
# Quick smoke test (5 tasks, llama3.2:latest, ~5 min):
pip install datasets
cd crates/roko-cli/scripts
python3 swebench_run.py --model llama3.2:latest --limit 5 \
  --output /tmp/preds.jsonl

# Score:
pip install swebench          # requires Docker
python -m swebench.harness.run_evaluation \
  --predictions_path /tmp/preds.jsonl \
  --dataset_name princeton-nlp/SWE-bench_Lite \
  --run_id llama3.2_smoke \
  --max_workers 4
```

**Be honest about expectations.** Local ollama models in the 2-30GB range
typically score 0-3% on SWE-bench-Lite with single-shot oracle retrieval.
Frontier models with tool use score 55-80%. If you see a handful of
resolved instances with a 7-30B local model, that is genuinely decent for
that class of setup.

### Does the harness actually help? (A/B testing)

Three scripts compose to answer "is roko-cli adding measurable value
beyond the underlying LLM?":

- `scripts/swebench_run.py` — full roko-cli pipeline (treatment)
- `scripts/swebench_baseline.py` — raw `ollama run`, no roko (control)
- `scripts/swebench_validate.py` — local, Docker-free scorer

```bash
# Same tasks, same model — one with harness, one without:
python3 swebench_run.py --model llama3.2:latest --limit 10 \
  --output /tmp/harness.jsonl --suffix full-harness
python3 swebench_baseline.py --model llama3.2:latest --limit 10 \
  --output /tmp/baseline.jsonl

# Compare: prints format-valid / apply-check / touches-oracle deltas
python3 swebench_validate.py \
  --predictions /tmp/harness.jsonl /tmp/baseline.jsonl
```

The validator reports a **per-metric delta** between any two prediction
files. If the harness helps, `format_valid` and `apply_check` rates
increase. If they don't, you've learned something useful: for that model
+ task combination, prompt structure isn't the bottleneck.

`swebench_run.py` also has ablation flags — `--no-clean-output`,
`--no-file-injection`, `--no-hard-cap`, `--minimal-role` — for
attributing the delta to specific harness features one at a time.

See `scripts/README.md` for full usage, the ablation recipe, and a
detailed breakdown of what this driver does (and does not) replicate
from more sophisticated SWE-bench agents.

## What it is not

- **Not the only entry point.** `roko-serve` exposes the same feature
  surface over HTTP for dashboards and external integrations.
- **Not a generic MCP client.** MCP integration happens inside the agent
  crate via `agent.mcp_config`; the CLI just passes the file through.

## Exit codes

`roko run` exits `0` when the agent succeeded and every configured gate passed, `1` otherwise. This makes the CLI usable as a check in a pipeline.

## Testing

```bash
cargo test -p roko-cli                 # unit + integration tests (~40 tests)
cargo clippy -p roko-cli --all-targets
```

Integration tests (`tests/e2e.rs`) spawn the binary via `assert_cmd` and
cover:
- `init` + `run "hello"` with `cat` produces all expected signal kinds
- A failing shell gate causes non-zero exit but persists signals
- `[[prompt.files]]` entries are loaded and reach the agent
- `clean_output = true` strips thinking traces and preserves the raw trace

## Known limitations

- **Approximate tokenization.** Budgets use a 4-bytes/token heuristic; real
  tokenizers aren't hooked in.
- **No patch application in `roko run`.** See "What it does not yet cover"
  above. `roko plan run` *does* apply patches via the worktree manager.
- **Outstanding gaps catalogued.** ~67 follow-up items from the
  post-PR-13 audit live in `tmp/ux-followup/` at the workspace root;
  start with `00-INDEX.md` for a severity-sorted view.
