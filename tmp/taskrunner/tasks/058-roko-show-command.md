# Task 058: `roko show` Command — Unified Inspect Surface

```toml
id = 58
title = "Add 'roko show' command: unified inspection replacing status, learn all, plan list, dashboard text mode"
track = "cli-redesign"
wave = "wave-2"
priority = "high"
blocked_by = [56]
touches = [
    "crates/roko-cli/src/main.rs",
    "crates/roko-cli/src/commands/show.rs",
    "crates/roko-cli/src/commands/status.rs",
    "crates/roko-cli/src/commands/learn.rs",
    "crates/roko-cli/src/tui/dashboard.rs",
    "crates/roko-cli/src/lib.rs",
]
exclusive_files = ["crates/roko-cli/src/commands/show.rs"]
estimated_minutes = 240
```

## Context

Inspecting workspace state currently requires knowing which of several commands to run:

- `roko status` — signal counts, recent episode, gate pass/fail
- `roko learn all` — dumps raw JSONL learning state
- `roko learn route` — dumps JSON cascade router state
- `roko learn efficiency` — dumps efficiency metrics
- `roko plan list` — lists plans
- `roko dashboard --text` — text-mode dashboard
- `roko doctor` — workspace bootstrap diagnostics

No single command gives a useful overview. `roko learn all` dumps raw data with no summary.
`roko status` shows one snapshot with no trend. A user who wants "what is going on in my
workspace?" has to run 3-4 commands and mentally merge the output.

`roko show` replaces this with a single context-aware command:

```bash
roko show                  # Smart default: workspace health summary
roko show plans            # Plan listing with status
roko show learning         # Learning summary (router, efficiency, episodes)
roko show runs             # Recent runs with outcomes
roko show config           # Effective configuration summary
```

Sources:
- `tmp/solutions/demo-running/CURRENT-STATE.md` — "The CLI Problem", "Learning is invisible"
- CLAUDE.md — `roko status` and `roko learn` command descriptions

## Background

Read these files:
1. `crates/roko-cli/src/main.rs` — `Command::Status`, `Command::Learn`, `Command::Dashboard`
   definitions and their dispatch in `dispatch_subcommand()`
2. `crates/roko-cli/src/commands/status.rs` — current `cmd_status()` implementation
3. `crates/roko-cli/src/commands/learn.rs` — current learn subcommand dispatch
4. `crates/roko-cli/src/status.rs` — `SessionStatus` computation
5. `crates/roko-cli/src/commands/show.rs` — current partial `roko show` implementation
6. `crates/roko-cli/src/tui/dashboard.rs` — `DashboardData::load_best_effort()` data source

Understand the current data sources:
```bash
# What data does status read?
grep -n 'signals.jsonl\|episodes.jsonl\|executor.json\|learn/' crates/roko-cli/src/status.rs | head -20

# What does learn dump?
grep -n 'fn dispatch_learn\|fn cmd_learn' crates/roko-cli/src/commands/learn.rs | head -10
```

## Current Branch Status - 2026-05-05

Status: **command exists, incomplete relative to this spec**.

Implemented now:
- `Command::Show` exists in `main.rs` with `--live`, `--workdir`, and optional `subject`.
- Dispatch calls `commands::show::cmd_show(...)`; the module is
  `crates/roko-cli/src/commands/show.rs`, not `show_cmd.rs`.
- Supported text topics are `overview`/`summary`, `costs`, `agents`, `knowledge`/`neuro`,
  `plans`, `learning`/`router`/`routing`, `history`/`events`/`log`, and work IDs.
- `--live` delegates to `dashboard::cmd_dashboard`.
- Data comes from `RokoLayout`, `DashboardData::load_best_effort()`, `.roko/state`,
  `.roko/learn`, and `.roko/neuro/knowledge.jsonl`.

Missing now:
- Global `--json` is not honored by `cmd_show`; `roko --json show` still renders text.
- No `config` or `health` topics.
- `roko status`, `roko learn`, and `roko plan list` remain separate commands and do not
  delegate to `show`.

## What to Change

### 1. Audit the existing `Command::Show` variant

`Command::Show` is already present. Update help text/examples and flags rather than adding
a duplicate. Use the existing global `cli.json` flag for JSON output unless there is a
strong reason to add a show-specific flag.

Required flag audit:
- `subject: Option<String>` selects the topic or work ID.
- `--workdir` resolves the workspace before loading `.roko` state.
- `--live` continues to delegate to the dashboard.
- Global `--json` (for example `roko --json show plans`) switches every topic to JSON.

### 2. Extend `commands/show.rs`

Implement `cmd_show()` with topic dispatch:

**`summary` (default)**: Combine the best of `status` and `doctor`:
- Workspace root, git branch, config file location
- Signal count, episode count, last run timestamp
- Gate pass rate (last 10 runs)
- Active plans count, completed/failed/pending task counts
- Learning trend: cost per task (last 10 vs previous 10), router confidence
- Provider health: green/yellow/red per provider
- One-line format, color-coded. Example:
  ```
  Workspace:  /Users/will/dev/nunchi/roko/roko (branch: wp-arch2)
  Signals:    847 total, 12 since last run
  Episodes:   143 recorded
  Last run:   2m ago — 8/8 tasks completed, 0 failed
  Gate rate:  95% pass (last 10 runs)
  Cost trend: $0.42/task avg (↓12% from previous)
  Providers:  anthropic ● healthy | openai ● degraded | ollama ○ offline
  ```

**`plans`**: List plans with task status breakdown:
- Plan ID, title, status (completed/running/pending)
- Task count: completed/failed/pending
- Last run timestamp
- Reuse existing plan listing logic from `commands/plan.rs`

**`learning`**: Summarized learning state (NOT raw JSONL):
- Cascade router: top 3 models by confidence, recent routing decisions
- Efficiency: average cost/task, average latency, trend arrows
- Episodes: count, most recent 5 summaries
- Gate thresholds: current adaptive values per rung
- Format as a readable table, not JSON dumps

**`runs`**: Recent run history:
- List recent runs from `.roko/state/` snapshots
- For each: timestamp, plan count, task count, pass/fail, duration, cost

**`config`**: Effective configuration summary:
- Active providers (with credential status: present/missing)
- Configured models (with assigned roles)
- Gate configuration
- Learning settings
- NOT a raw TOML dump — a structured summary
- Implementation target: use the same config-loading path as other CLI commands
  (`RokoConfig`, unified config loader, or existing workdir resolver). Redact credential
  values; show only present/missing and provider/model names.

**`health`**: Provider health + gate thresholds:
- Per-provider: circuit state, success rate, recent failures
- Gate thresholds: current EMA values, configured vs effective
- Implementation target: prefer `DashboardData` summaries and existing learn/provider
  health files over scanning raw logs. If data is absent, render `unknown` rather than
  failing the command.

### 3. Wire into dispatch

Dispatch is already wired to `commands::show::cmd_show`. Keep a single module. Do not create
`commands/show_cmd.rs` unless you also rename every module reference and remove `show.rs`.

### 4. Add `--json` support

Every topic must support `--json` output. When `--json` is passed, emit a structured JSON
object instead of the formatted text. This enables scripting and piping.

Mechanical shape:
- Branch in `cmd_show()` on `cli.json` before rendering text.
- Reuse the same loaded `ShowState` for text and JSON to avoid divergent data.
- Emit a top-level object with at least `topic`, `workspace`, `state_root`,
  `generated_at`, and a topic-specific `data` object.
- Work-id lookups should produce a nonzero exit with a small JSON error object when
  `--json` is set.
- Add a test or command verification using `python3 -m json.tool`.

### 5. Keep existing commands as aliases

`roko status` stays and continues to work. Internally it can delegate to `cmd_show("summary")`.
`roko learn all` stays. `roko plan list` stays. These are not removed in this task.

Given the current branch already has stable separate implementations, delegation is optional.
If you delegate, preserve existing output compatibility or update tests/docs in the same
change. Do not break `status` while adding `show`.

### 6. Add tests around parsing and rendering

- Unit tests for `ShowTarget` parsing, including aliases: `summary`, `overview`, `plans`,
  `learning`, `history`/`runs`, `config`, `health`, and work IDs.
- JSON renderer tests for default summary and at least one topic, asserting parseable JSON
  and the presence of `topic`, `workspace`, and `data`.
- Config rendering test proving secret values are redacted.
- Learning text test proving `roko show learning` summarizes state instead of dumping raw
  JSONL lines.

## What NOT to Do

- Don't remove `roko status`, `roko learn`, `roko plan list`, or `roko dashboard`. They stay
  as-is. `roko show` is a new unified surface, not a replacement delete.
- Don't add the TUI (ratatui) to `roko show`. It is text-only output. The TUI stays in
  `roko dashboard`.
- Don't add new data collection. Use existing data files (signals.jsonl, episodes.jsonl,
  learn/, state/).
- Don't parse JSONL at query time if pre-computed summaries exist. Prefer reading snapshot
  files over scanning logs.
- Don't implement all topics in a single monolithic function. One function per topic.
- Don't create `commands/show_cmd.rs` in this branch unless you intentionally migrate the
  existing `commands/show.rs` module.
- Don't print raw config secrets, auth tokens, or environment values.

## Wire Target

```bash
# Default summary:
cargo run -p roko-cli -- show
# Should print workspace health summary with color

# Plans:
cargo run -p roko-cli -- show plans
# Should list plans with task status

# Learning:
cargo run -p roko-cli -- show learning
# Should print summarized learning state (NOT raw JSONL)

# JSON mode:
cargo run -p roko-cli -- --json show
# Should output valid JSON

# JSON mode (verify parseable):
cargo run -p roko-cli -- --json show 2>/dev/null | python3 -m json.tool > /dev/null
# Should succeed (valid JSON)

# Config and health topics:
cargo run -p roko-cli -- show config
cargo run -p roko-cli -- show health
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- show` — prints workspace summary
- [ ] `cargo run -p roko-cli -- show plans` — lists plans
- [ ] `cargo run -p roko-cli -- show learning` — learning summary (not raw JSONL)
- [ ] `cargo run -p roko-cli -- show config` — effective config summary with secrets redacted
- [ ] `cargo run -p roko-cli -- show health` — provider/gate health summary
- [ ] `cargo run -p roko-cli -- --json show | python3 -m json.tool` — valid JSON
- [ ] `cargo run -p roko-cli -- status` — still works (not broken)
- [ ] `grep -rn 'Command::Show' crates/roko-cli/src/main.rs` — wired in dispatch
- [ ] `grep -rn 'cmd_show' crates/roko-cli/src/commands/ --include='*.rs'` — implementation exists

## Status Log

| Time | Agent | Action |
|------|-------|--------|
