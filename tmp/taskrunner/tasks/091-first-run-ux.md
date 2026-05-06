# Task 091: First-Run UX + CLI Polish

```toml
id = 91
title = "Refuse chat without valid provider, init validation, plan progress, status --quick, doctor fixes"
track = "cli-redesign"
wave = "wave-2"
priority = "high"
blocked_by = [1]
touches = [
    "crates/roko-cli/src/main.rs",
    "crates/roko-cli/src/chat.rs",
    "crates/roko-cli/src/commands/init.rs",
    "crates/roko-cli/src/commands/status.rs",
    "crates/roko-cli/src/commands/doctor.rs",
    "crates/roko-cli/src/commands/config_cmd.rs",
]
exclusive_files = []
estimated_minutes = 300
```

## Context

Blocked by task 001 (unified config loader) because this task calls `load_config_unified` from
multiple entry points and must be consistent — the pre-check in `chat.rs`, the init validation,
and the `status --quick` path must all call the same loader.

New users running roko for the first time encounter five distinct friction points:

1. **`roko chat` starts the REPL then fails mid-session.** No pre-check runs before entering the
   interactive loop. The `claude` binary absent or `ANTHROPIC_API_KEY` missing surfaces only when
   the user types their first message. The error appears inside the session, not before it.

2. **`roko init` declares success with no working provider.** After writing `roko.toml`, init
   prints "initialized roko workspace" and exits 0. Users then run `roko chat` and hit failure (1).
   Init should confirm provider status as the final step.

3. **`roko plan run` is silent for minutes.** The runner loop dispatches tasks with no per-task
   progress line. There is no feedback that work is happening. Users cannot tell if the process
   is hung or running.

4. **`roko status` output is too dense for a quick health check.** Full `roko status` queries the
   substrate and renders a multi-section report. There is no `--quick` flag for a three-line
   health summary suitable for CI or shell prompts.

5. **`roko doctor` identifies problems but offers no fixes.** Each check prints `[fail]` or `[warn]`
   with a problem description. No fix command is printed. Users must consult docs to know what to do.

Sources:
- `tmp/redesign-plan.md` Phase 11 (11.1, 11.2, 11.3, 11.5, 11.6, 11.7)
- `tmp/infrastructure-audit.md` §10 (S10 reproducibility, `roko doctor`, `roko config show --effective`)

## Background

Read these files before writing any code:

1. `crates/roko-cli/src/chat.rs` lines 1-80 — The `roko chat` entry point. Identify where the
   REPL loop begins and where the first provider call is made. The guard must be inserted before
   the REPL prompt is shown. Also look at `chat_inline.rs` for the inline path.

2. `crates/roko-cli/src/auth_detect.rs` lines 1-120 — `AuthMethod` enum (including `NeedsSetup`),
   `detect_auth_from_config()`, `detect_auth_from_env()`. After Task 090 lands, `NeedsSetup` will
   carry a `problems: Vec<String>` field. If Task 090 is not yet merged, add the field yourself
   or call `check_provider_readiness` from Task 090 directly. Do not duplicate detection logic —
   call the existing `detect_auth_from_config` function.

3. `crates/roko-cli/src/commands/init.rs` (or `commands/util.rs`) — `cmd_init()`. Find the point
   after `roko.toml` is written where provider status should be validated and printed. The function
   must continue to return `Ok(())` regardless; the provider check is informational.

4. `crates/roko-cli/src/commands/status.rs` (or `commands/util.rs`) — `cmd_status()`. Understand
   the full output path so the `--quick` early-return does not accidentally share code with the
   verbose path. The quick path must perform no substrate I/O.

5. `crates/roko-cli/src/main.rs` — The `Status` subcommand definition and its argument list.
   A `--quick` boolean flag must be added and threaded through to `cmd_status()`.

6. `crates/roko-cli/src/commands/doctor.rs` — `cmd_doctor()`. Each check should end with a
   "fix:" line. Find the existing check structure and understand what error type each check
   produces so you can pair it with the right fix command.

7. `crates/roko-cli/src/runner/event_loop.rs` lines ~340-420 — The main runner loop. Find where
   each task agent is dispatched. This is where the per-task start line must be emitted.

8. `crates/roko-cli/src/runner/inline_output.rs` — `RunnerInlineTerminal` and its existing
   methods (`warm_cache_started`, `warm_cache_completed`, etc.). Extend this struct with a new
   `task_started` method rather than adding ad-hoc `eprintln!` calls in `event_loop.rs`.

9. `crates/roko-cli/src/commands/config_cmd.rs` — Find `cmd_config_show()` or the equivalent
   `config show` handler. The `--effective` flag adds a dump of the fully-resolved config after
   env var and ancestor-path overrides.

## What to Change

### 1. Refuse `roko chat` without a valid provider

In `crates/roko-cli/src/chat.rs`, before entering the REPL loop, call
`detect_auth_from_config(&workdir)`. If the result is `AuthMethod::NeedsSetup`, print to stderr
and return `Err(...)` with exit code 1 — do not print a REPL prompt:

```
error: no LLM provider is configured or available.

To set one up:
  roko config providers available    # see all supported provider kinds
  roko config providers add claude   # interactive setup (if available)
  export ANTHROPIC_API_KEY=sk-ant-... && roko init

See roko.toml [providers.*] for manual configuration.
```

Apply the same guard to `chat_inline.rs`. Do not duplicate detection — the inline path receives
the `auth` argument from `cmd_unified_chat`; check it there before entering any work.

The guard must not fire when a valid provider exists. Use `AuthMethod::NeedsSetup` as the sole
signal, not any other variant. Test both paths (no config, config with missing env var) in the
verification step.

### 2. Provider validation at the end of `roko init`

In `cmd_init()`, after `roko.toml` is written (and after the interrupted-session snapshot check),
call `detect_auth_from_config(&target)`:

- **`AuthMethod::NeedsSetup`**: print a warning block and continue:
  ```
  warning: no provider credentials found.

  The workspace is initialized but roko cannot dispatch agents yet.
  Next step:
    roko config providers available   # see what providers are supported
    export ANTHROPIC_API_KEY=sk-ant-...
  Or add a [providers.*] block to roko.toml with an API key.
  ```
- **Any other `AuthMethod` variant**: print a confirmation line:
  ```
  provider: <auth.label()> — ready
  ```

Do not return a non-zero exit code from `cmd_init()` regardless of provider status. The warning
is informational; `roko init` succeeds if the workspace was created successfully.

### 3. ALREADY EXISTS — verify only: Per-task start line in `roko plan run`

**Status**: `task_started()` already exists in the codebase:
- `crates/roko-cli/src/runner/inline_output.rs` line 90: `pub(crate) fn task_started(...)` defined.
- `crates/roko-cli/src/runner/event_loop.rs` line 2767: called as
  `.task_started(&task_id, role, &task_def.title, attempt_num)`.
- `crates/roko-cli/src/runner/event_loop.rs` line 3082: called in another dispatch path.
- `crates/roko-cli/src/runner/tui_bridge.rs` line 39: TUI variant also has `task_started`.
- `crates/roko-cli/src/runner/output_sink.rs` lines 36, 101: trait method defined and implemented.

**Do not reimplement.** The method exists with a slightly different signature than what was
originally specified here (it takes `task_id`, `role`, `title`, `attempt_num` rather than
`index`, `total`, `task_id`, `model`), but it is wired and functional.

**Verification only**: confirm that `roko plan run plans/` actually emits per-task progress
lines to stderr during execution. If the output format needs adjustment (e.g., adding
`[N/M]` prefix or model slug), that is a minor enhancement, not a from-scratch implementation.

### 4. `roko status --quick`

Add a `--quick` flag to the `Status` subcommand in `crates/roko-cli/src/main.rs`:

```rust
/// Print a compact 3-line health summary (provider, learning state, workspace).
#[arg(long)]
quick: bool,
```

Thread the flag through to `cmd_status()`. Add an early-return branch at the top of `cmd_status()`
when `quick == true`:

```rust
if quick {
    let auth = detect_auth_from_config(&workdir);
    let provider_line = match &auth {
        AuthMethod::NeedsSetup { .. } => {
            "provider:   NONE — run `roko config providers available`".to_string()
        }
        other => format!("provider:   {}", other.label()),
    };
    let learn_line = if workdir.join(".roko/learn/cascade-router.json").exists() {
        "learning:   active (cascade router data present)"
    } else {
        "learning:   no data yet"
    };
    let workspace_line = if workdir.join("roko.toml").exists() {
        format!("workspace:  {}", workdir.display())
    } else {
        "workspace:  no roko.toml — run `roko init`".to_string()
    };
    println!("{provider_line}");
    println!("{learn_line}");
    println!("{workspace_line}");
    return Ok(());
}
```

The quick path must perform no substrate I/O, no signal queries, and no async operations.
It must return in under 100 ms for any workspace. All three lines always print (never omit one).
Exit 0 if `auth` is not `NeedsSetup` and `roko.toml` exists; exit 1 otherwise.

### 5. `roko doctor` actionable fix commands

In `crates/roko-cli/src/commands/doctor.rs`, extend each check so that a failing check prints a
"fix:" line immediately after the `[fail]` or `[warn]` line:

| Failing check | Fix line |
|---|---|
| `roko.toml` not found | `fix: roko init` |
| claude CLI not on PATH | `fix: npm install -g @anthropic-ai/claude-cli && claude login` |
| `ANTHROPIC_API_KEY` not set | `fix: export ANTHROPIC_API_KEY=sk-ant-...` |
| Port 6677 already in use | `fix: kill $(lsof -ti :6677)` |
| `.roko/` directory missing | `fix: roko init` |
| `roko.toml` parse error | `fix: roko config validate  # see what is wrong` |
| Rust version below 1.91 | `fix: rustup update stable` |
| Node version below 22 | `fix: nvm install 22 && nvm use 22` |

The fix line format is: four spaces, `→ fix: `, then the command. It prints on the line
immediately after the `[fail]`/`[warn]` line. Passing checks (`[ok]`) do not print a fix line.

Add `roko dev setup` as a real command if `roko dev` does not yet exist (or add it as a no-op
stub with `// TODO: wire to roko-dev-full alias installer`). The doctor output for the missing
alias check should read:
```
[warn] roko-dev-full alias not found in shell config
       → fix: roko dev setup
```

### 6. `roko config show --effective`

Add a `--effective` flag to the `config show` subcommand (wherever `cmd_config_show()` lives in
`crates/roko-cli/src/commands/config_cmd.rs`):

```rust
/// Show the fully-resolved config after global merge and env var overrides.
#[arg(long)]
effective: bool,
```

When `--effective` is passed, load the config using the unified loader (task 001's
`load_config_unified`), then serialize it to TOML and print to stdout. The output must reflect:
- Global config merged from `~/.roko/config.toml`
- Project config from `roko.toml`
- Env var overrides applied
- Default values filled in

When `--effective` is not passed, the existing behavior is unchanged. The flag is additive.

## What NOT to Do

- Do NOT change the REPL loop logic inside `chat.rs`. Only add the guard before the prompt appears.
- Do NOT duplicate `detect_auth_from_config` calls. Call once; pass the result to sub-functions
  that need it.
- Do NOT make `roko status --quick` open the substrate, parse signals, or touch any JSONL file.
  Filesystem presence checks only.
- Do NOT remove the existing verbose `roko status` output path. The `--quick` flag is a pure
  early-return branch, not a replacement.
- Do NOT write task progress to stdout. Use `eprintln!` only (or the `self.stderr` guard in
  `RunnerInlineTerminal`).
- Do NOT change the TUI progress rendering. This task covers the CLI-only (`RunnerInlineTerminal`)
  path; the TUI handles its own task progress separately.
- Do NOT add new crates or Cargo dependencies. All required functionality exists in the workspace.

## Wire Target

```bash
# 1. roko chat refuses to start without a provider
# (unset all keys, or set a nonexistent claude command in roko.toml)
cargo run -p roko-cli -- chat
# Expected: actionable error to stderr, exit code 1, no REPL prompt

# 2. roko init warns when no provider is ready
tmp=$(mktemp -d)
ANTHROPIC_API_KEY="" cargo run -p roko-cli -- init "$tmp"
# Expected: "warning: no provider credentials found." at end of output

# 3. roko plan run emits per-task lines
cargo run -p roko-cli -- plan run plans/ 2>&1 | grep '\[1/'
# Expected: "[1/N] Running "task-name" (model-slug)" line

# 4. roko status --quick is fast and always prints 3 lines
time cargo run -p roko-cli -- status --quick
# Expected: 3 lines in under 1 s

# 5. roko doctor prints fix commands
cargo run -p roko-cli -- doctor
# Expected: each [fail]/[warn] is followed by "    → fix: <command>"

# 6. roko config show --effective dumps merged config
cargo run -p roko-cli -- config show --effective
# Expected: TOML output including values from ~/.roko/config.toml and roko.toml
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `roko chat` exits code 1 with actionable text when `AuthMethod::NeedsSetup`
- [ ] `roko chat` enters the REPL normally when a valid provider exists (no regression)
- [ ] `roko init` in a no-credentials directory prints the provider warning block and exits 0
- [ ] `roko init` in a directory with a working provider prints "provider: ... — ready" and exits 0
- [ ] `roko plan run plans/` emits `[N/M] Running "task-id" (model)` lines to stderr per task
- [ ] `roko status --quick` prints exactly 3 lines (provider, learning, workspace) in under 100 ms
- [ ] `--quick` appears in `roko status --help`
- [ ] `roko doctor` prints `→ fix: <command>` on the line after every `[fail]` and `[warn]` check
- [ ] `roko config show --effective` prints TOML that differs from `roko config show` when a global
  config exists (demonstrates merge is applied)
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file touched by this task

## Implementation Ground Truth (Worker 18 Enrichment)

The current code layout differs from the older file names above. Treat these as the runtime paths to inspect before editing:

- Default no-subcommand chat path: `main.rs` dispatches to `roko_cli::unified::cmd_unified_chat()`, which already calls `detect_auth_from_config(&current_dir)` before `chat_inline::run_unified_inline()`. Keep this guard before `RokoBootstrap`, `ensure_workspace()`, background serve startup, and any raw-terminal setup.
- `agent chat` path: `main.rs` `Command::Agent` -> `commands/agent.rs::cmd_agent()` -> `agent_serve.rs::run(AgentCmd::Chat)` -> `chat_inline.rs::run_chat_inline()` -> `chat.rs::run_chat_repl()` for non-TTY fallback. This path is still the mechanical place to add a preflight guard if the user explicitly runs `roko agent chat`/legacy chat. Add the guard before `InlineTerminal::new()` and before the fallback line-oriented prompt in `chat.rs`.
- `cmd_init()`, `cmd_status()`, and `cmd_doctor()` live in `crates/roko-cli/src/commands/util.rs`; `commands/init.rs` is only template rendering, `commands/status.rs` re-exports `util::cmd_status`, and `commands/doctor.rs` does not exist. Doctor's actual report model/rendering is `crates/roko-cli/src/doctor.rs`.
- The config-show implementation is `crates/roko-cli/src/config_cmd.rs::cmd_show()`; `crates/roko-cli/src/commands/config_cmd.rs` only dispatches `ConfigCmd::Show`.
- `AuthMethod::NeedsSetup` is currently a unit variant in `auth_detect.rs`; if task 090 has merged, it may be a struct variant. Write matches so they compile against the state in the worktree, e.g. use the exact local variant shape and keep all other variants accepted.
- Per-task plan progress already exists in `runner/inline_output.rs::RunnerInlineTerminal::task_started()` and is called from `runner/event_loop.rs` dispatch paths. Do not replace it with direct `eprintln!`; at most adjust formatting in `RunnerInlineTerminal` if verification shows the required `[N/M]` shape is missing.

## Mechanical Implementation Steps (Worker 18 Enrichment)

1. Update CLI argument plumbing in `main.rs`.
   - Add `quick: bool` to `Command::Status`, ideally with `conflicts_with = "surfaces"` so `--quick --surfaces` is rejected by clap instead of doing surprising work.
   - Change the status dispatch to pass `quick`. Because quick mode must exit 1 without printing an extra anyhow error, change `commands::status::cmd_status` / `util::cmd_status` to return `Result<i32>` and return `EXIT_SUCCESS` or `EXIT_AGENT_FAILURE` from the command; update the existing non-quick callers to preserve exit 0 on success.
   - Add `effective: bool` to `ConfigCmd::Show` and thread it through `commands/config_cmd.rs` to `roko_cli::config_cmd::cmd_show_effective()` or a widened `cmd_show(workdir, effective)`.

2. Implement provider preflight in one helper and reuse it.
   - Add a small helper near `auth_detect.rs` or `unified.rs`, e.g. `ensure_provider_ready_for_chat(workdir: &Path) -> Result<AuthMethod>`, that calls `detect_auth_from_config(workdir)` exactly once, prints the standardized setup block on `NeedsSetup`, and returns an error/exit signal.
   - Use it in `unified::cmd_unified_chat()` only if the current guard is being changed; otherwise leave the existing guard and only update its text/tests.
   - Use it in `agent_serve.rs` before `chat_inline::run_chat_inline()`, or at the top of `run_chat_inline()` before the HTTP client and `InlineTerminal::new()`. If you guard in `run_chat_inline()`, pass/derive `workdir` consistently and make sure the non-TTY fallback does not print a prompt first.

3. Add init validation in `commands/util.rs::cmd_init()`.
   - Place it after the interrupted-session snapshot message and before `Ok(())`.
   - Use `detect_auth_from_config(&target)` and only print; never fail init because credentials are missing.
   - Do not edit `commands/init.rs` unless the generated template text itself needs to change.

4. Add quick status in `commands/util.rs::cmd_status()`.
   - Resolve `workdir`, then execute the quick branch before `FileSubstrate::open`, `Query::all`, C-Factor refresh, costs log reads, or any async substrate I/O.
   - Use only `detect_auth_from_config`, `workdir.join("roko.toml").exists()`, and `.roko/learn/cascade-router.json` existence checks.
   - Always print exactly three stdout lines. Return exit 0 only when provider is not `NeedsSetup` and `roko.toml` exists; otherwise return exit 1.

5. Add doctor fixes in `crates/roko-cli/src/doctor.rs`, not `commands/util.rs`.
   - Extend `DoctorCheck` with `fix: Option<String>` and update `render_human()` to print `       -> fix: <command>` immediately after warn/fail lines. Keep JSON output backward-compatible by using `skip_serializing_if`.
   - Add the new environment/prerequisite checks in `run_doctor()` if they are absent today: Rust version, Node version, Claude CLI on PATH, `ANTHROPIC_API_KEY`, port 6677, and `roko-dev-full` alias. Existing checks only cover workdir, config presence, `.roko` layout, serve auth, and optional serve health.
   - `roko dev setup` does not currently exist (`rg "Command::Dev|roko dev"` returns no hits). Adding it requires a `main.rs` subcommand plus a handler; if the task owner does not want that scope, keep the doctor fix line but mark the command as a follow-up in the Status Log.

6. Add `config show --effective`.
   - Existing `roko_cli::config_cmd::cmd_show()` uses `load_layered()` and source-tag rendering. Add a separate `cmd_show_effective()` that calls `roko_core::config::loader::load_config_unified(workdir)` and `roko_core::config::loader::serialize_effective(&config)` (already exists in `loader.rs`).
   - Do not remove the source-tag output for default `roko config show`.

## Focused Tests to Add (Worker 18 Enrichment)

- Parser tests in `main.rs` for `status --quick`, `config show --effective`, and `status --quick --surfaces` conflict if that conflict is added.
- Unit tests for quick status should use a temp dir and call a small pure helper if possible; do not open `FileSubstrate` in quick tests.
- Doctor tests in `crates/roko-cli/src/doctor.rs` should assert every warn/fail check renders a following `-> fix:` line, and JSON contains the optional `fix` field only when present.
- Config-show tests should set a temp `ROKO_CONFIG`/global config and project `roko.toml`, then assert `--effective` output is valid TOML and includes merged/defaulted values.
- Chat guard tests should assert no `InlineTerminal::new()`/prompt path is reached on `NeedsSetup`; if direct terminal tests are impractical, unit test the shared guard and add one CLI smoke command in Verification.

## Scope Notes (Worker 18 Enrichment)

The task metadata is stale for implementation: `commands/util.rs`, `crates/roko-cli/src/doctor.rs`, `crates/roko-cli/src/config_cmd.rs`, `crates/roko-cli/src/unified.rs`, and possibly `crates/roko-cli/src/agent_serve.rs` are required to satisfy the described behavior but are not all listed in `touches`. Do not start code work until the implementation worker either expands the task `touches` list or records an explicit owner approval in their Status Log.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
