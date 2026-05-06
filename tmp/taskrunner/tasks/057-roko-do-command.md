# Task 057: `roko do` Command — Universal Entry Point with Progressive Formality

```toml
id = 57
title = "Add 'roko do' command: unified entry point that collapses prd idea → draft → plan → run into one command"
track = "cli-redesign"
wave = "wave-2"
priority = "critical"
blocked_by = [56]
touches = [
    "crates/roko-cli/src/main.rs",
    "crates/roko-cli/src/commands/util.rs",
    "crates/roko-cli/src/commands/prd.rs",
    "crates/roko-cli/src/commands/plan.rs",
    "crates/roko-cli/src/scope_resolver.rs",
    "crates/roko-cli/src/lib.rs",
    "crates/roko-cli/src/run.rs",
    "crates/roko-cli/src/prd.rs",
    "crates/roko-cli/src/plan_generate.rs",
]
exclusive_files = []
estimated_minutes = 360
```

## Context

The current workflow to go from intent to working code requires 4 separate commands:

```bash
roko prd idea "Wire SystemPromptBuilder into orchestrate.rs"
roko prd draft new "system-prompt-wiring"
roko prd plan system-prompt-wiring
roko plan run plans/
```

This is a 4-step pipeline that requires the user to know internal concepts (PRDs, drafts, plan
directories). The `roko do` command collapses this into a single intent-driven entry point:

```bash
roko do "Fix the typo in README.md"                    # → single agent run
roko do "Add rate limiting to the API"                 # → auto-detects complexity → PRD pipeline
roko do "Refactor the database layer for sharding"     # → full PRD → plan → execute
```

**Progressive formality**: The command classifies the prompt by complexity and chooses the
right execution path automatically. Small tasks skip the PRD pipeline entirely.

Sources:
- `tmp/solutions/demo-running/CURRENT-STATE.md` — "The CLI Problem" section
- `tmp/redesign-plan.md` — CLI simplification priority
- CLAUDE.md — self-hosting workflow section

## Current Branch Status - 2026-05-05

Status: **wired, not fully implemented as originally scoped**.

Implemented on `wp-arch2`:
- `Command::Do` exists in `crates/roko-cli/src/main.rs`.
- Complexity classification lives in `crates/roko-cli/src/scope_resolver.rs` and uses the
  existing `PlanComplexity` shape (`Trivial`, `Simple`, `Standard`, `Complex`).
- Execution calls the existing WorkflowEngine path via
  `run_workflow_engine_report_with_hub()` in `crates/roko-cli/src/commands/util.rs`.
- `roko run <prompt>` routes to `cmd_do` for the common no-serve/no-share path.
- Dry-run smoke verified on this branch:
  `cargo run -p roko-cli --bin roko -- do "Fix typo in README.md" --dry-run`.

Not implemented yet:
- No `crates/roko-cli/src/commands/do_cmd.rs`; implementation is currently in
  `commands/util.rs`.
- The medium PRD/plan and complex PRD -> draft -> plan -> execute pipelines described below
  are not wired. The current implementation selects WorkflowEngine templates
  (`mechanical`, `focused`, `integrative`, `architectural`).
- `--compare` is preview-only. `--continue` reports resumable state and points at
  `roko resume`; it is not a complete work-item resume surface.
- `--yes` appears in the CLI surface, but approval bypass behavior still needs an audit.
- CLI reference docs were stale before this audit and must describe `roko do` as the preferred
  classified entry point.

## Background

Read these files:
1. `crates/roko-cli/src/main.rs` — Command enum (line 307+), `dispatch_subcommand()` (line 2054)
2. `crates/roko-cli/src/commands/util.rs` — `cmd_run()` at line 221 (current `roko run` impl)
3. `crates/roko-cli/src/prd.rs` — PRD idea/draft/plan functions
4. `crates/roko-cli/src/plan_generate.rs` — plan generation from PRD
5. `crates/roko-cli/src/run.rs` — `run_workflow_engine_report_with_hub()` (v2 engine entry)
6. `crates/roko-cli/src/commands/prd.rs` — `cmd_prd()` dispatcher

Understand the existing paths:
```bash
# How does roko run work today?
grep -n 'run_workflow_engine\|run_once' crates/roko-cli/src/run.rs | head -20

# How does prd idea → plan flow?
grep -n 'fn.*idea\|fn.*draft\|fn.*plan' crates/roko-cli/src/prd.rs | head -20
```

## Implementation Context

Current CLI/runtime call chain:

```text
main.rs::dispatch_subcommand
  Command::Do
    -> commands::util::cmd_do(...)
      -> scope_resolver::ScopeResolver::resolve(...)
      -> workflow_template_for_complexity(...)
      -> run::run_workflow_engine_report_with_hub(...)
      -> roko_runtime::WorkflowEngine::run(...)

main.rs::Command::Run (common no serve/share/max_retries path)
  -> commands::util::cmd_do(...)
```

Current `Command::Do` already includes `--plan`, `--complexity`, `--dry-run`,
`--workdir`, `--provider`, `--yes`, `--ghost`, `--compare`, `--continue`,
`--no-cascade`, and `prompt: Vec<String>`. Preserve that surface unless a flag is proven
dead. The remaining work is not "add the command"; it is to replace the current template
selection for medium/complex tasks with the promised PRD/plan execution pipeline.

Useful existing APIs:
- `prd.rs::cmd_idea()` writes an idea entry.
- `prd.rs::generate_plan_from_prd()` and `generate_plan_from_prd_with_model()` generate
  plans from PRDs.
- PRD draft creation and plan command dispatch currently live partly in
  `commands/prd.rs` and `commands/plan.rs`. If a reusable helper is needed, extract it
  into `prd.rs` or `plan_generate.rs` instead of shelling out to the CLI.
- `run.rs::run_plan_tasks_with_workflow_engine()` and related `run_plan_prompts_*`
  helpers are the v2 execution targets for generated plans.

## What to Change

### 1. Extend complexity classification

Extend the existing classifier in `crates/roko-cli/src/scope_resolver.rs`; do not create a
second classifier. It already returns the existing `PlanComplexity` shape:

- **Simple** — single-file fix, typo, small addition. Indicators: short prompt (<100 chars),
  mentions specific file, "fix", "typo", "rename", "update" language.
- **Medium** — feature addition, test writing, API endpoint. Indicators: mentions a concept
  but not a fundamental redesign.
- **Complex** — architectural change, multi-crate refactor, new subsystem. Indicators: "refactor",
  "redesign", "sharding", "migrate", multi-sentence description, mentions multiple components.

This does NOT need LLM classification. Use keyword heuristics and prompt length. The user can
override with `--complexity simple|medium|complex`.

### 2. Audit the existing `Command::Do` variant

`Command::Do` is already present in `main.rs`. Compare the implemented flags to the intended
behavior and update help text/examples if needed. Do not paste in a second enum variant.

Required flag audit:
- `prompt: Vec<String>` joins into the natural-language request.
- `--complexity simple|medium|complex` maps to `PlanComplexity` without an LLM.
- `--dry-run`/`--ghost`/`--compare` render the selected path without unexpected execution.
- `--plan`, `--yes`, `--continue`, and `--no-cascade` must either affect the pipeline or
  print a truthful "not implemented yet" message with tests.
- `--workdir` and `--provider` must flow through to config/model selection.

### 3. Implement the command handler

Current branch note: the handler currently lives in `commands/util.rs`, not
`commands/do_cmd.rs`. Creating a dedicated file is optional cleanup; do not duplicate the
existing handler unless you are moving it.

Implement `cmd_do()` with three execution paths:

**Simple path** (direct agent run):
- Call the v2 WorkflowEngine directly with the prompt (same as `roko run`)
- No PRD, no plan generation
- Stream output to stderr in real-time

**Medium path** (plan + execute):
- Generate a plan from the prompt via existing plan-generation helpers (skip PRD creation)
- Write the plan under the existing `.roko/plans/<slug>/` layout; do not invent a new plan
  format or temp-only location
- Execute the generated plan via v2 (`run_plan_tasks_with_workflow_engine()` or the same
  helper used by `roko plan run`)
- Stream output to stderr

**Complex path** (full pipeline):
- Create a PRD idea from the prompt
- Draft the PRD using the existing draft machinery; if it is trapped in `commands/prd.rs`,
  extract a small reusable helper
- Generate implementation plan from the PRD via `prd.rs::generate_plan_from_prd*`
- Execute the generated plan via the v2 engine
- Stream output to stderr

Each path must print what it is doing:
```
▸ Complexity: simple (auto-detected)
▸ Running single agent...
```
or
```
▸ Complexity: complex (auto-detected, override with --complexity simple)
▸ Step 1/4: Creating PRD...
▸ Step 2/4: Drafting PRD...
▸ Step 3/4: Generating plan (12 tasks)...
▸ Step 4/4: Executing plan...
```

Use an internal enum such as `DoExecutionPath::{DirectWorkflow, PromptPlanWorkflow,
PrdPlanWorkflow}` so dry-run, ghost, compare, and actual execution are rendered from the
same decision. This prevents dry-run from describing a path the runtime does not use.

`--yes` should only bypass PRD/plan approval prompts that already exist in the pipeline. It
must not broaden filesystem/shell permissions or skip gates. `--compare` may remain
preview-only unless this task explicitly implements two real executions; label it as such
in output and tests. `--continue` may continue pointing at `roko resume` until work-item
resume is implemented, but the message and tests must make that limitation clear.

### 4. Wire into dispatch

The dispatch arm already exists. Keep it routed to the single handler location
(`commands::util::cmd_do` today, or a moved `commands::do_cmd::cmd_do` if you refactor).
Do not leave both handlers compiled.

### 5. Keep `roko run` as an alias

`roko run` currently delegates to `cmd_do` for the common path, so it inherits
classification/template behavior. Decide whether `run` should remain an alias to `do` or
become a strict direct WorkflowEngine invocation, then update help text and tests to match.
Do not leave docs saying one thing while dispatch does another.

### 6. Add tests around the mechanical decisions

- Unit tests in `scope_resolver.rs` for simple, standard/medium, and complex prompts plus
  `--complexity` override mapping.
- CLI/dry-run tests that verify the chosen `DoExecutionPath` for simple, medium, and complex
  prompts without contacting a provider.
- Tests for `--plan`, `--no-cascade`, `--compare`, and `--continue` output so preview mode
  remains truthful.
- A helper-level test for prompt->plan and PRD->plan path creation using temp `.roko/plans`
  directories, if helpers are extracted.

## What NOT to Do

- Don't remove `roko run`. Keep it as a documented alias or an explicit direct-run path,
  but make dispatch, help text, and tests agree.
- Don't use an LLM for complexity classification — heuristics only. LLM classification adds
  latency and cost before the user sees anything happen.
- Don't modify the PRD pipeline internals. Call existing functions.
- Don't add new PRD fields or plan formats.
- Don't implement `roko do --resume` — that stays on `roko plan run --resume` / `roko resume`.
- Don't create `commands/do_cmd.rs` while leaving the old `cmd_do` path active.
- Don't shell out to `roko prd` or `roko plan`; call Rust helpers directly so errors and
  state paths remain testable.
- Don't classify with an LLM or provider call.

## Wire Target

```bash
# Simple task — should go directly to agent:
cargo run -p roko-cli -- do "Fix the typo in README.md" --dry-run
# Output must include: complexity simple/auto-detected, direct WorkflowEngine execution,
# and execution skipped because dry-run.

# Complex task — should show full pipeline:
cargo run -p roko-cli -- do "Refactor the entire database layer to support sharding across multiple regions" --dry-run
# Output must include: complexity complex/auto-detected and PRD -> Draft -> Plan -> Execute.

# Override complexity:
cargo run -p roko-cli -- do "Big refactor" --complexity simple --dry-run
# Output must include: complexity simple/forced and the direct path.

# Actually run a simple task:
cargo run -p roko-cli -- do "Add a hello world test to roko-core"
# Should produce agent output on stderr, complete with exit code 0
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- do "test" --dry-run` — works, shows complexity classification
- [ ] `cargo run -p roko-cli -- do "test" --complexity simple --dry-run` — override works
- [ ] `cargo run -p roko-cli -- do "test" --complexity complex --dry-run` — shows full pipeline
- [ ] `cargo run -p roko-cli -- run "test" --help` — still works (not removed)
- [ ] `grep -rn 'Command::Do' crates/roko-cli/src/main.rs` — wired in dispatch
- [ ] `grep -rn 'cmd_do' crates/roko-cli/src/commands/ --include='*.rs'` — implementation exists

## Status Log

| Time | Agent | Action |
|------|-------|--------|
| 2026-05-05 | wp-arch2 audit | Partial implementation found. `roko do` is wired in `main.rs`, classification lives in `scope_resolver.rs`, and execution dispatches directly to WorkflowEngine from `commands/util.rs`. Dry-run works. The documented PRD/plan pipelines and dedicated `commands/do_cmd.rs` file are not implemented. |
