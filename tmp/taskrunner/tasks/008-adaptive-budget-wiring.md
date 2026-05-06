# Task 008: Wire AdaptiveBudget — Scale Prompts to Model Context Window

```toml
id = 8
title = "Wire adaptive_budget_for() to replace static budget_for() in prompt templates"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-compose/src/templates/common.rs",
    "crates/roko-compose/src/templates/implementer.rs",
    "crates/roko-compose/src/templates/reviewer.rs",
    "crates/roko-compose/src/templates/strategist.rs",
    "crates/roko-compose/src/templates/quick.rs",
    "crates/roko-compose/src/templates/scribe.rs",
    "crates/roko-compose/src/templates/integration.rs",
    "crates/roko-compose/src/templates/task_impl.rs",
    "crates/roko-compose/src/templates/mod.rs",
    "crates/roko-compose/src/system_prompt_builder.rs",
]
exclusive_files = ["crates/roko-compose/src/templates/common.rs"]
estimated_minutes = 60
```

## Context

`adaptive_budget_for(role, context_window)` exists in `templates/common.rs` and scales token
budgets to the model's context window. But all templates call the static `budget_for(role)` which
returns fixed constants. The dynamic scaling is never used.

Current code inspection shows several role templates and `SystemPromptBuilder` may already use
`adaptive_budget_for()`. Treat this task as "verify current wiring and fill exact gaps", not as a
request to churn already-correct call sites.

Sources:
- `tmp/v2-refactoring/10-DEAD-CODE-AUDIT.md` — AdaptiveBudget (WIRE NOW)
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md` — W15-E: AdaptiveBudget dead code

## Background

Read these files first:
1. `crates/roko-compose/src/templates/common.rs` — both `budget_for()` and `adaptive_budget_for()`
2. `crates/roko-compose/src/system_prompt_builder.rs` — where prompts are assembled
3. Any template file — see how `budget_for()` is currently called

## What to Change

1. **Replace `budget_for(role)` calls with `adaptive_budget_for(role, context_window)`** in all template files.
2. **Thread `context_window: usize`** through the template construction:
   - `SystemPromptBuilder` should accept model context window size
   - Pass it through to template rendering
3. **If context_window is unknown**, fall back to the static `budget_for()` behavior (backwards compat).

## What NOT to Do

- Don't change the adaptive budget algorithm.
- Don't add new budget categories.
- Don't change template content — only how budgets are calculated.
- Don't delete `budget_for()`: it is the static fallback and the base table used by
  `adaptive_budget_for()`.

## Implementation Notes

Current code inspection shows parts of this task may already be implemented. Before editing, run:

```bash
rg -n "budget_for|adaptive_budget_for|sections_with_context_window|context_window_tokens" crates/roko-compose/src -g '*.rs'
rg -n "show-prompt|ShowPrompt" crates/roko-cli/src/main.rs crates/roko-cli/src/commands/plan.rs
```

Files/functions to read before editing:
- `crates/roko-compose/src/templates/common.rs`: `PromptBudget`, `REFERENCE_CONTEXT_WINDOW_TOKENS`,
  `budget_for()`, `AdaptiveBudget`, `adaptive_budget_for()`, adaptive-budget tests.
- `crates/roko-compose/src/templates/mod.rs`: `RolePromptTemplate::sections_with_context_window()`
  default method.
- `crates/roko-compose/src/templates/implementer.rs`: `ImplementerTemplate::sections()` and
  `sections_with_context_window()`.
- `crates/roko-compose/src/templates/reviewer.rs`: `ReviewerTemplate::sections()` and
  `sections_with_context_window()`.
- `crates/roko-compose/src/templates/strategist.rs`: `StrategistTemplate::sections()` and
  `sections_with_context_window()`.
- Also inspect `templates/quick.rs`, `templates/scribe.rs`, `templates/integration.rs`, and
  `templates/task_impl.rs`; they already follow the same pattern and are useful examples.
- `crates/roko-compose/src/system_prompt_builder.rs`: `with_budget_profile()`,
  `with_adaptive_budget_profile()`, and `section_budget_cap()`.
- Read-only runtime context: `crates/roko-cli/src/prompting.rs` and
  `crates/roko-cli/src/dispatch_helpers.rs` show how CLI callers pass `context_window_tokens`.

Mechanical steps:
1. Do not blindly replace all text matches for `budget_for(`. Valid remaining uses include:
   - the `budget_for()` definition itself;
   - `adaptive_budget_for()` calling `budget_for()` as its base table;
   - tests that compare static and adaptive behavior;
   - `budget.rs::adjusted_budget_for()` if it intentionally remains the static fallback.
2. For each role template that applies section caps, ensure:
   - `sections(&self, input)` delegates to `sections_with_context_window(input,
     REFERENCE_CONTEXT_WINDOW_TOKENS)`;
   - `sections_with_context_window(...)` calls `adaptive_budget_for(role, context_window_tokens)`;
   - no template hardcodes static caps where the matching `PromptBudget` field exists.
3. For `SystemPromptBuilder`, ensure callers can choose either explicit
   `with_budget_profile(PromptBudget)` or model-aware `with_adaptive_budget_profile(role,
   context_window_tokens)`. Unknown context windows should use `REFERENCE_CONTEXT_WINDOW_TOKENS`,
   which preserves the old `budget_for()` behavior.
4. If the current snapshot already satisfies the above, do not churn the templates. Add or tighten
   tests only.
5. If a runtime CLI path still assembles prompts without passing selected-model context, do not edit
   unrelated `roko-cli` files unless the task metadata is expanded. Record the gap in the Status Log
   with the exact caller path.

Tests to add/update:
- In `crates/roko-compose/src/templates/common.rs`, keep the existing scale tests and add a
  regression that `adaptive_budget_for(role, REFERENCE_CONTEXT_WINDOW_TOKENS) == budget_for(role)`.
- In each touched template module, add or update tests proving a smaller context window lowers at
  least one hard cap and a larger window raises or preserves it.
- In `system_prompt_builder.rs`, add a test that `with_adaptive_budget_profile()` changes the cap
  returned by `section_budget_cap("task_context")` compared with the reference window.
- Avoid snapshot updates unless the only changed bytes are truncation caused by the new budget.

## Wire Target

```bash
cargo test -p roko-compose adaptive_budget -- --nocapture
cargo test -p roko-compose templates:: -- --nocapture
```

**Expected behavior**: adaptive-budget tests show smaller context windows reduce section caps and
larger windows increase/preserve them. Note: `roko plan show-prompt` is not present in the current
CLI, so do not add that command under this task unless the write set is expanded.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `rg -n 'budget_for\(' crates/roko-compose/src/templates crates/roko-compose/src/system_prompt_builder.rs -g '*.rs'` — no production template call should use static `budget_for()` directly
- [ ] `rg -n 'adaptive_budget_for' crates/roko-compose/src/templates crates/roko-compose/src/system_prompt_builder.rs -g '*.rs'` — shows production callers
- [ ] `rg -n 'show-prompt|ShowPrompt' crates/roko-cli/src/main.rs crates/roko-cli/src/commands/plan.rs` — if still empty, document the missing CLI prompt-inspection wire target in Status Log

## Status Log

| Time | Agent | Action |
|------|-------|--------|
