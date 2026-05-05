# Task 024: Wire agents_instructions_section() for All 7 Templates

```toml
id = 24
title = "Wire agents_instructions_section() into all role templates"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-compose/src/templates/implementer.rs",
    "crates/roko-compose/src/templates/reviewer.rs",
    "crates/roko-compose/src/templates/strategist.rs",
    "crates/roko-compose/src/templates/scribe.rs",
    "crates/roko-compose/src/templates/quick.rs",
    "crates/roko-compose/src/templates/integration.rs",
    "crates/roko-compose/src/templates/task_impl.rs",
]
exclusive_files = []
estimated_minutes = 45
```

## Context

`agents_instructions_section()` was built but not wired for all 7 templates. Some role
templates may exceed token budgets when instructions are added.

Sources:
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md` - W14-A: `agents_instructions_section()` partial
- `tmp/solutions/demo-running/archive/original-docs/IMPROVEMENTS.md` - 10.4 DRY violation
- `tmp/solutions/demo-running/archive/batches-executed/W14-A-compose-sections.md` - original compose/template fix batch

## Background

Read these before editing:

1. `crates/roko-compose/src/templates/common.rs`
   - `agents_instructions_section(agents_md: &str)` is the canonical constructor.
   - It must build section name `agents_instructions` with `SectionPriority::Critical`,
     `CacheLayer::Role`, and `Placement::Start`.
2. `crates/roko-compose/src/templates/mod.rs`
   - `RolePromptTemplate` is the template trait; all target templates return `Vec<PromptSection>`.
3. Target template functions:
   - `implementer.rs` - `ImplementerTemplate::sections_with_context_window`
   - `reviewer.rs` - `ReviewerTemplate::sections_with_context_window`
   - `strategist.rs` - `StrategistTemplate::sections_with_context_window`
   - `scribe.rs` - `ScribeTemplate::sections_with_context_window`
   - `quick.rs` - `QuickReviewerTemplate::sections_with_context_window`
   - `integration.rs` - `IntegrationTemplate::sections_with_context_window`
   - `task_impl.rs` - `push_base_sections`
4. `crates/roko-compose/src/templates/assembly.rs`
   - Runtime/library chain is `PromptAssembler::assemble_from*()` ->
     `RolePromptTemplate::sections*()` -> rendered prompt text.

Current-state check:

```bash
grep -rn 'agents_instructions_section' crates/roko-compose/src/templates/ --include='*.rs' | grep -v target/
grep -rn 'PromptSection::new("agents_instructions"' crates/roko-compose/src/templates/ --include='*.rs' | grep -v target/
```

If the first command already shows the seven target files and the second command shows
only `templates/common.rs`, the implementation part is already done and this task is
verification/test hardening only.

Scope note: "all 7 templates" means the seven target files listed in `touches`, with
`quick.rs` represented by `QuickReviewerTemplate`. Do not retrofit `QuickFixTemplate`
unless this task's touch list and all its callsites are explicitly expanded, because
`QuickFixInput` intentionally has no `agents_md` field.

## What to Change

1. In each target template, replace any hand-built `PromptSection::new("agents_instructions", ...)`
   block with:

   ```rust
   sections.push(common::agents_instructions_section(&input.agents_md));
   ```

2. Place the `agents_instructions` push first, before plan/task/workspace sections, so
   primacy/cache placement stays stable.

3. Keep existing per-template section capacities correct. If adding the section changes
   `Vec::with_capacity(...)`, update that capacity in the same file only.

4. Do not add local truncation around `input.agents_md`. The canonical section is a critical
   role section; budget enforcement belongs to the existing assembler/composer hard-cap paths.
   If a template-specific test exposes over-budget rendering, update the test expectation to
   prove the existing budget mechanism truncates, not the template itself.

5. Add/update tests in each changed template module:
   - golden section-name test includes `agents_instructions` as the first section
   - section metadata test checks `Critical`, `Role`, `Start`
   - existing budget-capped render tests still pass

6. Add a grep-based verification note to the Status Log showing exactly which seven files
   call `common::agents_instructions_section`.

## What NOT to Do

- Don't change the instructions content.
- Don't change the template structure.
- Don't duplicate the section constructor in individual templates.
- Don't add filesystem reads in templates; `agents_md` must continue arriving through input structs.
- Don't touch `conductor.rs`, `researcher.rs`, or `refactorer.rs`; they are outside this task's
  seven-template scope.
- Don't add `agents_md` to `QuickFixInput` in this task.

## Wire Target

```bash
# Check that all templates include agent instructions
grep -rn 'common::agents_instructions_section' crates/roko-compose/src/templates/ --include='*.rs' | grep -v target/
```

Expected observable behavior:

- The grep shows calls in exactly these seven target files: `implementer.rs`, `reviewer.rs`,
  `strategist.rs`, `scribe.rs`, `quick.rs`, `integration.rs`, and `task_impl.rs`.
- No target template manually constructs `PromptSection::new("agents_instructions", ...)`.
- Rendering any target template includes an `agents_instructions` section first, with role-layer
  cache metadata.

Useful runtime/library path to exercise:

```bash
cargo test -p roko-compose render_golden -- --nocapture
cargo test -p roko-compose budget_capped_render -- --nocapture
```

## Verification

- [ ] `cargo build -p roko-compose`
- [ ] `cargo test -p roko-compose templates::`
- [ ] `cargo test -p roko-compose render_golden -- --nocapture`
- [ ] `cargo test -p roko-compose budget_capped_render -- --nocapture`
- [ ] `cargo clippy -p roko-compose --no-deps -- -D warnings`
- [ ] `grep -rn 'common::agents_instructions_section' crates/roko-compose/src/templates/ --include='*.rs' | grep -v target/` shows all 7 target files
- [ ] `grep -rn 'PromptSection::new("agents_instructions"' crates/roko-compose/src/templates/ --include='*.rs' | grep -v target/` shows only `templates/common.rs`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
