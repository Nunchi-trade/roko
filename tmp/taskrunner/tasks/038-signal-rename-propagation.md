# Task 038: Propagate Signal Rename to 5 Core Crates

```toml
id = 38
title = "Propagate Engram -> Signal rename to roko-agent, roko-gate, roko-learn, roko-compose, roko-orchestrator"
track = "v2-core-abstractions"
wave = "wave-2"
priority = "medium"
blocked_by = [37]
touches = [
    "crates/roko-agent/src/agent.rs",
    "crates/roko-agent/src/lifecycle.rs",
    "crates/roko-agent/src/dispatcher/mod.rs",
    "crates/roko-agent/src/claude_cli_agent.rs",
    "crates/roko-agent/src/claude_agent.rs",
    "crates/roko-agent/src/exec.rs",
    "crates/roko-agent/src/pool.rs",
    "crates/roko-agent/src/composition.rs",
    "crates/roko-agent/src/task_runner.rs",
    "crates/roko-agent/src/mock.rs",
    "crates/roko-gate/src/compile.rs",
    "crates/roko-gate/src/test_gate.rs",
    "crates/roko-gate/src/clippy_gate.rs",
    "crates/roko-gate/src/diff_gate.rs",
    "crates/roko-gate/src/gate_pipeline.rs",
    "crates/roko-gate/src/shell.rs",
    "crates/roko-gate/src/composition.rs",
    "crates/roko-learn/src/episode_logger.rs",
    "crates/roko-learn/src/verdict_scorer.rs",
    "crates/roko-learn/src/error_enrichment.rs",
    "crates/roko-compose/src/prompt.rs",
    "crates/roko-compose/src/system_prompt_builder.rs",
    "crates/roko-compose/src/context_provider.rs",
    "crates/roko-compose/src/scorer.rs",
    "crates/roko-orchestrator/src/coordination.rs",
    "crates/roko-orchestrator/src/safety/taint_propagation.rs",
]
exclusive_files = []
estimated_minutes = 180
```

## Context

Task 037 renamed `Engram` to `Signal` in roko-core and left a deprecated `type Engram = Signal`
alias. The 5 most-used downstream crates still import `Engram`. This task updates them to
import `Signal` directly, eliminating deprecated-alias warnings.

Current `Engram` usage across these 5 crates:
- **roko-agent**: ~92 occurrences across ~20 files (heaviest user)
- **roko-gate**: ~115 occurrences across ~25 files
- **roko-learn**: ~25 occurrences across ~10 files
- **roko-compose**: ~41 occurrences across ~6 files
- **roko-orchestrator**: ~9 occurrences across ~2 files

Total: ~282 occurrences. This is a mechanical find-and-replace but must be verified per-file
to avoid breaking type inference or match arms.

Checklist item: P1-8.

## Background

Read these files before starting:

1. `crates/roko-core/src/engram.rs` — to see what `Signal` (formerly `Engram`) looks like after task 037
2. `crates/roko-core/src/lib.rs` — to see what names are exported

Then scan each crate:
```bash
grep -rn 'Engram' crates/roko-agent/src/ --include='*.rs' | grep -v target/ | head -30
grep -rn 'Engram' crates/roko-gate/src/ --include='*.rs' | grep -v target/ | head -30
grep -rn 'Engram' crates/roko-learn/src/ --include='*.rs' | grep -v target/ | head -30
grep -rn 'Engram' crates/roko-compose/src/ --include='*.rs' | grep -v target/ | head -30
grep -rn 'Engram' crates/roko-orchestrator/src/ --include='*.rs' | grep -v target/ | head -30
```

## What to Change

### Per-crate migration (repeat for each of the 5 crates)

1. **Update imports**: `use roko_core::Engram` -> `use roko_core::Signal`
   - Also: `EngramBuilder` -> `SignalBuilder`
   - Also: `Datum::Engram(...)` -> `Datum::Signal(...)` in match arms

2. **Update type annotations**: `engram: Engram` -> `signal: Signal` (or keep parameter names
   if renaming parameters would be noisy — type name is what matters)

3. **Update doc comments**: "engram" -> "signal" in doc strings

4. **Update test files**: Same find-and-replace in test modules

### Specific watch-outs per crate

**roko-agent** (heaviest user):
- `lifecycle.rs` has ~92 uses — many are in function signatures and struct fields
- `agent.rs` has `AgentOutput` which wraps engrams — update the type
- `composition.rs` has engram composition logic — rename types, not logic

**roko-gate** (second heaviest):
- Every `impl Verify for X` has `verify(&self, engram: &Engram, ctx: &Context) -> Verdict`
  — the parameter name `engram` can stay or become `signal` (your call; consistency within
  the crate matters more than the name)
- `gate_pipeline.rs` has mock gates in test modules — update those too
- Test files in `crates/roko-gate/tests/` also need updating

**roko-learn**:
- `episode_logger.rs` — Episode struct has engram references
- `verdict_scorer.rs` — scorer operates on engrams

**roko-compose**:
- `prompt.rs` (~22 uses) — prompt assembly from engrams
- `context_provider.rs` (~12 uses)
- `scorer.rs` (~11 uses)

**roko-orchestrator**:
- Only ~9 uses, mostly in taint_propagation.rs

### Verification per crate

After updating each crate, run:
```bash
cargo build -p roko-{crate}
cargo test -p roko-{crate}
```
before moving to the next.

## What NOT to Do

- Do NOT update crates beyond these 5 (roko-cli, roko-fs, roko-std, roko-serve, roko-conductor,
  roko-chain, roko-dreams, etc. can be done in a future task).
- Do NOT remove the deprecated Engram alias from roko-core. Other crates still need it.
- Do NOT change serialization format or field names. Only Rust type names change.
- Do NOT rename function parameters if it would make the diff enormous. Type names are the
  priority; parameter names are nice-to-have.
- Do NOT fix bugs or refactor logic while renaming. Pure mechanical rename only.

## Wire Target

The workspace compiles without deprecated-alias warnings from these 5 crates:

```bash
cargo build -p roko-agent 2>&1 | grep -c 'deprecated.*Engram'
# Should output: 0

cargo build -p roko-gate 2>&1 | grep -c 'deprecated.*Engram'
# Should output: 0
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -rn 'Engram' crates/roko-agent/src/ --include='*.rs' | grep -v target/ | grep -v 'type Engram' | grep -v deprecated` — returns 0 lines
- [ ] `grep -rn 'Engram' crates/roko-gate/src/ --include='*.rs' | grep -v target/ | grep -v deprecated` — returns 0 lines
- [ ] `grep -rn 'Engram' crates/roko-learn/src/ --include='*.rs' | grep -v target/ | grep -v deprecated` — returns 0 lines
- [ ] `grep -rn 'Engram' crates/roko-compose/src/ --include='*.rs' | grep -v target/ | grep -v deprecated` — returns 0 lines
- [ ] `grep -rn 'Engram' crates/roko-orchestrator/src/ --include='*.rs' | grep -v target/ | grep -v deprecated` — returns 0 lines
- [ ] No serialization/deserialization regressions (existing JSONL still parses)

## Implementation Detail

### Dependency order

Start only after task 037 is merged in the worktree. Confirm:

```bash
rg -n "pub struct Signal|type Engram = Signal|pub struct SignalBuilder|type EngramBuilder = SignalBuilder" crates/roko-core/src/engram.rs
```

If `Signal` is still only a re-export alias from `signal.rs`, stop and mark the
task blocked by task 037.

### Current reference set

The TOML `touches` list above is narrower than the current grep result. The
implementation goal ("these 5 crates import `Signal` directly") requires
handling every file below. If task-runner touch enforcement is strict, sync the
task metadata before implementation rather than doing a partial rename.

```text
crates/roko-agent/src/agent.rs
crates/roko-agent/src/claude_agent.rs
crates/roko-agent/src/claude_cli_agent.rs
crates/roko-agent/src/codex_agent.rs
crates/roko-agent/src/composition.rs
crates/roko-agent/src/cursor_agent.rs
crates/roko-agent/src/dispatcher/mod.rs
crates/roko-agent/src/exec.rs
crates/roko-agent/src/gemini/compat.rs
crates/roko-agent/src/gemini/embed.rs
crates/roko-agent/src/gemini/native.rs
crates/roko-agent/src/lifecycle.rs
crates/roko-agent/src/metamorphosis.rs
crates/roko-agent/src/mock.rs
crates/roko-agent/src/model_call_service.rs
crates/roko-agent/src/multi_pool.rs
crates/roko-agent/src/ollama/agent.rs
crates/roko-agent/src/openai_agent.rs
crates/roko-agent/src/perplexity/adapter.rs
crates/roko-agent/src/perplexity/chat.rs
crates/roko-agent/src/perplexity/deep_research.rs
crates/roko-agent/src/perplexity/tool_loop.rs
crates/roko-agent/src/pool.rs
crates/roko-agent/src/process/group.rs
crates/roko-agent/src/provider/anthropic_api.rs
crates/roko-agent/src/provider/claude_cli.rs
crates/roko-agent/src/provider/cursor_acp.rs
crates/roko-agent/src/provider/mod.rs
crates/roko-agent/src/provider/openai_compat.rs
crates/roko-agent/src/task_runner.rs
crates/roko-agent/src/testutil.rs
crates/roko-agent/src/tool_loop/agent_wrapper.rs
crates/roko-compose/src/compaction.rs
crates/roko-compose/src/context_provider.rs
crates/roko-compose/src/lib.rs
crates/roko-compose/src/prompt.rs
crates/roko-compose/src/scorer.rs
crates/roko-compose/src/symbol_resolver.rs
crates/roko-compose/src/system_prompt_builder.rs
crates/roko-gate/src/benchmark_gate.rs
crates/roko-gate/src/clippy_gate.rs
crates/roko-gate/src/code_exec.rs
crates/roko-gate/src/compile.rs
crates/roko-gate/src/composition.rs
crates/roko-gate/src/diff_gate.rs
crates/roko-gate/src/fact_check.rs
crates/roko-gate/src/format_check_gate.rs
crates/roko-gate/src/gate_pipeline.rs
crates/roko-gate/src/gate_service.rs
crates/roko-gate/src/generated_test_gate.rs
crates/roko-gate/src/integration_gate.rs
crates/roko-gate/src/llm_judge_gate.rs
crates/roko-gate/src/payload.rs
crates/roko-gate/src/property_test_gate.rs
crates/roko-gate/src/rung_dispatch.rs
crates/roko-gate/src/security_scan_gate.rs
crates/roko-gate/src/shell.rs
crates/roko-gate/src/symbol_gate.rs
crates/roko-gate/src/test_gate.rs
crates/roko-gate/src/verify_chain_gate.rs
crates/roko-gate/tests/compile_real_project.rs
crates/roko-gate/tests/gate_truth.rs
crates/roko-gate/tests/rungs.rs
crates/roko-learn/src/cascade/tests.rs
crates/roko-learn/src/episode_logger.rs
crates/roko-learn/src/error_enrichment.rs
crates/roko-learn/src/oracles/chain.rs
crates/roko-learn/src/oracles/coding.rs
crates/roko-learn/src/oracles/research.rs
crates/roko-learn/src/quality_judge.rs
crates/roko-learn/src/skill_library.rs
crates/roko-learn/src/verdict_scorer.rs
crates/roko-orchestrator/src/coordination.rs
crates/roko-orchestrator/src/safety/taint_propagation.rs
```

Regenerate the list before editing with:

```bash
rg -l "\bEngram\b|\bEngramBuilder\b|Datum::Engram" \
  crates/roko-agent/src crates/roko-gate/src crates/roko-gate/tests \
  crates/roko-learn/src crates/roko-compose/src crates/roko-orchestrator/src \
  --glob '*.rs' | sort
```

### Mechanical migration rules

Apply these rules only in the five target crates and `crates/roko-gate/tests/`:

1. Imports:
   - `use roko_core::{..., Engram, ...};` -> `Signal`
   - `use roko_core::{..., EngramBuilder, ...};` -> `SignalBuilder`
   - Qualified paths `roko_core::Engram` -> `roko_core::Signal`
   - Qualified paths `roko_core::EngramBuilder` -> `roko_core::SignalBuilder`

2. Type positions:
   - `Engram` -> `Signal`
   - `Vec<Engram>` -> `Vec<Signal>`
   - `&Engram` -> `&Signal`
   - `EngramBuilder` -> `SignalBuilder`
   - `impl Iterator<Item = ContentHash>` and other non-Engram types stay
     unchanged.

3. Constructors and associated functions:
   - `Engram::builder(...)` -> `Signal::builder(...)`
   - `Engram::from_pulse_synthetic(...)` -> `Signal::from_pulse_synthetic(...)`
   - `Engram::from_pulses(...)` -> `Signal::from_pulses(...)`
   - Any `serde_json::from_str::<Engram>(...)` -> `::<Signal>(...)`

4. Enum variants:
   - If task 037 renamed `Datum::Engram` to `Datum::Signal`, update every
     downstream match/construction to `Datum::Signal`.
   - Current grep shows no downstream `Datum::Engram` hits, but verify again.

5. Public function names:
   - Do not rename public functions just because they contain lowercase
     `engram` unless the function name itself causes a compiler/deprecation
     issue. For example, `AgentOutput::all_engrams()` can remain as a
     compatibility method if it returns `Vec<Signal>`.
   - Update doc comments and user-facing prose from capitalized `Engram` to
     `Signal` so the verification greps are clean.

6. Serialization:
   - Do not rename JSON fields, serde aliases, persisted file names, or database
     keys unless they literally contain the Rust type token in code.
   - Keep existing JSONL compatibility. Type names are not serialized for the
     core `Signal` struct.

### Crate-specific notes

- `roko-agent/src/agent.rs` imports both `Engram` and `EngramBuilder`; migrate
  both to `Signal` and `SignalBuilder`. `derived_output(...)` should return
  `SignalBuilder`. `all_engrams()` may stay as a compatibility method name.
- `roko-agent/src/pool.rs` uses qualified `roko_core::Engram`; convert those
  to `roko_core::Signal`.
- `roko-agent/src/lifecycle.rs` has many domain comments and docs using
  capitalized `Engram`. Update prose, but do not redesign lifecycle data
  structures.
- `roko-gate` gates all implement `Verify::verify(&self, signal: &Engram, ...)`;
  after task 037 the trait signature accepts `&Signal`, so update every gate
  signature and helper signal constructors. Do not change gate behavior.
- `roko-gate/tests/*.rs` compile under `cargo test -p roko-gate`; update them
  in the same pass.
- `roko-compose/src/prompt.rs` has public APIs named `into_signal` and
  `from_signal`; keep those names and change only the underlying type.
- `roko-compose/src/symbol_resolver.rs` contains generated/example text with
  `Engram`; update the string if the final grep requires zero capitalized
  `Engram` hits.
- `roko-learn/src/cascade/tests.rs` is under `src`, so it is covered by the
  crate build and grep even though it is test-only code.
- `roko-orchestrator/src/safety/taint_propagation.rs` should switch imports and
  doc comments from `Engram` to `Signal`; do not change taint semantics.

### Suggested workflow

1. Run the per-crate `rg -l` command and save the file list in your status log.
2. Migrate one crate at a time, then run:
   ```bash
   cargo build -p roko-agent
   cargo test -p roko-agent
   cargo build -p roko-gate
   cargo test -p roko-gate
   cargo build -p roko-learn
   cargo test -p roko-learn
   cargo build -p roko-compose
   cargo test -p roko-compose
   cargo build -p roko-orchestrator
   cargo test -p roko-orchestrator
   ```
3. After all five crates pass, run the workspace verification from the main
   task checklist.

### Final grep checks

Use token-boundary greps so lowercase compatibility names such as
`all_engrams` do not create false positives:

```bash
rg -n "\bEngram\b|\bEngramBuilder\b|Datum::Engram" crates/roko-agent/src --glob '*.rs'
rg -n "\bEngram\b|\bEngramBuilder\b|Datum::Engram" crates/roko-gate/src crates/roko-gate/tests --glob '*.rs'
rg -n "\bEngram\b|\bEngramBuilder\b|Datum::Engram" crates/roko-learn/src --glob '*.rs'
rg -n "\bEngram\b|\bEngramBuilder\b|Datum::Engram" crates/roko-compose/src --glob '*.rs'
rg -n "\bEngram\b|\bEngramBuilder\b|Datum::Engram" crates/roko-orchestrator/src --glob '*.rs'
```

Each command should return zero lines unless the line is an intentional
backwards-compatibility comment that the implementation agent explicitly
documents in the status log.

### Anti-patterns

- Do not use `#[allow(deprecated)]` in these five crates to hide old `Engram`
  imports.
- Do not run a workspace-wide rename that touches `roko-cli`, `roko-fs`,
  `roko-std`, `roko-serve`, `roko-conductor`, `roko-chain`, or other crates.
- Do not rename lowercase public compatibility method names unless a compile
  error forces it.
- Do not alter gate, agent, compose, or learning behavior while doing the type
  rename.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
