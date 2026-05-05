# Task 046: Make PRD Promote Atomic and Fix Frontmatter Parser

```toml
id = 46
title = "Atomic PRD promote + proper YAML frontmatter parser"
track = "infrastructure"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-cli/src/prd.rs",
]
exclusive_files = ["crates/roko-cli/src/prd.rs"]
estimated_minutes = 90
```

## Context

Two PRD lifecycle issues from the audit (S14.1, S14.3):

1. **Promote is write-then-delete, not atomic** (S14.1): `cmd_promote` at line ~760
   does `fs::write(&dst, &content)` then `fs::remove_file(&src)`. A crash between
   leaves both files. Next promote silently overwrites (already fixed by S14.2 check,
   but the dual-file window remains).

2. **Frontmatter parser is a line scanner, not YAML** (S14.3): `PrdMeta::parse` at
   lines ~490-527 manually parses with `strip_prefix("title:")` etc. This breaks on:
   - Values with colons: `title: Wire: the thing` → truncated to ` Wire`
   - Quoted values: `title: "My PRD"` → includes quotes
   - Multi-word values are OK but YAML list syntax for tags fails
   - `depends_on` as YAML list is not handled

## Background

Read: `crates/roko-cli/src/prd.rs`
- `PrdMeta::parse()` at lines ~490-527 (frontmatter parser)
- `cmd_promote()` at lines ~727-775 (promote flow)
- `generate_plan_from_prd_with_outcome()` write path around lines 1200-1228
  (`tasks.toml` and `plan.md` writes)
- `update_prd_plans_generated()` around lines 1733-1771
- Existing tests in the same file: `parse_frontmatter()` around line 2549,
  `promote_moves_file()` around line 2704, and
  `replace_in_frontmatter_only_affects_frontmatter()` around line 2896
- `crates/roko-core/src/io.rs` — `atomic_write`, `atomic_write_str`, and
  `read_optional` already exist
- `crates/roko-cli/Cargo.toml` already depends on `serde_yaml_ng`; do not add
  `serde_yaml`

Runtime chain:

```text
roko prd draft promote <slug>
  -> main.rs command dispatch
  -> prd::cmd_promote()
  -> replace_in_frontmatter()
  -> roko_core::io::atomic_write[_str]()
  -> maybe_generate_plan_after_promote()
  -> generate_plan_from_prd_with_outcome()
  -> update_prd_plans_generated()
```

## What to Change

### 1. Atomic Promote (S14.1)

Replace the write-then-delete pattern:

```rust
// BEFORE:
std::fs::write(&dst, &content)?;
std::fs::remove_file(&src)?;

// AFTER — use roko_core::io::atomic_write_str + remove:
roko_core::io::atomic_write_str(&dst, &content)?;
// Only remove source AFTER destination is confirmed on disk
std::fs::remove_file(&src)?;
```

The `atomic_write` function does write-to-tmp-then-rename, which is already available
in `roko_core::io`. This makes the destination write crash-safe. The source removal
is idempotent — if it fails after the destination was written, the S14.2 check
catches the duplicate on next promote.

### 2. YAML Frontmatter Parser (S14.3)

Replace the manual line scanner with a YAML-based parser using the existing
`serde_yaml_ng` dependency.

```rust
impl PrdMeta {
    pub fn parse(content: &str) -> Option<Self> {
        let content = content.trim();
        if !content.starts_with("---") {
            return None;
        }
        let end = content[3..].find("---")?;
        let yaml_str = &content[3..3 + end];
        // Parse as YAML mapping, then extract known fields
        let map: serde_yaml_ng::Mapping =
            serde_yaml_ng::from_str(yaml_str).ok()?;
        // ... extract fields from map with proper type coercion
    }
}
```

Extract these fields from `serde_yaml_ng::Value`:
- Scalars: `id`, `title`, `status`, `created`, `updated`, `plan_template`
- Numbers: `version` (`u32`, default 1 on bad/missing), `coverage` (`f64`,
  default 0.0 on bad/missing)
- String lists: `depends_on`, `crates`, `plans_generated`, `tags`

Support both inline YAML lists (`tags: ["a", "b"]`) and block lists. Quoted
YAML scalars should be returned unquoted by the YAML parser; do not manually
strip quotes after parsing except as a fallback for non-YAML legacy content.
Update the existing `parse_frontmatter` test to use valid YAML
`plan_template: "strict"` instead of the current `plan_template = "strict"`.

### 3. Non-atomic writes in PRD

Also convert `std::fs::write` calls in `prd.rs` production paths to use
`roko_core::io::atomic_write` where the file is critical state:
- Line ~1204: `std::fs::write(plan_dir.join("tasks.toml"), &validated_toml)` →
  `roko_core::io::atomic_write_str`
- Lines ~1215 and ~1227: generated or minimal `plan.md` writes →
  `roko_core::io::atomic_write_str` (plan discovery treats this as generated
  state paired with `tasks.toml`)
- Line ~1770: `std::fs::write(prd_path, updated)` →
  `roko_core::io::atomic_write_str`

Leave scaffold/idea files with `std::fs::write` (not critical).

## Mechanical Implementation Notes

Create small local helpers inside `prd.rs`, for example:

```rust
fn yaml_get_string(map: &serde_yaml_ng::Mapping, key: &str) -> Option<String>;
fn yaml_get_string_list(map: &serde_yaml_ng::Mapping, key: &str) -> Vec<String>;
```

Key lookup should use `serde_yaml_ng::Value::String(key.to_string())`.
For scalar fields, accept string values and numeric/bool YAML values by
converting them to strings only for the known string fields. For list fields,
ignore non-string elements rather than failing the whole parse.

Keep `PrdMeta::parse()` returning `Option<Self>`; malformed frontmatter should
return `None`, preserving current caller behavior in `read_prd_entry()` and
`cmd_plan()`.

## Tests to Add or Update

In `crates/roko-cli/src/prd.rs` tests:
- Update `parse_frontmatter()` to assert `depends_on`, `crates`,
  `plans_generated`, `tags`, and `plan_template`.
- Add `parse_frontmatter_handles_colons_quotes_and_yaml_lists()` with:
  `title: "Wire: the thing"`, `tags: ["infra", "prd"]`, and a block
  `depends_on` list.
- Add/extend a promote test to assert the published content exists, the draft is
  removed only after a successful write, and body text containing
  `status: draft` remains unchanged.
- Keep `replace_in_frontmatter_only_affects_frontmatter()` unchanged except for
  assertions directly required by this task.

## What NOT to Do

- Don't change the `replace_in_frontmatter` function — it was already fixed (S14.4).
- Don't change `has_substantive_markdown_content` — already fixed (batch 34).
- Don't add new frontmatter fields.
- Don't change the promote episode logging or event emission.
- Don't add a new YAML crate; use `serde_yaml_ng`.
- Don't replace the whole PRD frontmatter writer. This task only parses metadata
  and makes existing critical writes atomic.

## Wire Target

```bash
# Test promote flow
cargo test -p roko-cli -- promote
# Test frontmatter parsing
cargo test -p roko-cli -- parse_frontmatter
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cmd_promote` uses `roko_core::io::atomic_write_str` for destination
- [ ] `PrdMeta::parse` handles `title: Wire: the thing` correctly (colon in value)
- [ ] `PrdMeta::parse` handles `title: "My PRD"` correctly (strips quotes)
- [ ] Add test for colon-in-value frontmatter parsing
- [ ] `grep -n 'std::fs::write' crates/roko-cli/src/prd.rs` shows only
      scaffold/idea/non-critical writes called out as intentionally unchanged

## Status Log

| Time | Agent | Action |
|------|-------|--------|
