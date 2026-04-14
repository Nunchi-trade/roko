# Plan Generation Strategy: Sufficient Context for Fresh Agents

## The Problem

When roko generates implementation plans (`roko prd plan <slug>`), each task gets assigned to a fresh agent with no prior context. The agent only knows what's in:
1. The system prompt (7 layers from `RoleSystemPromptSpec`)
2. The task definition in `tasks.toml`
3. Whatever the agent can discover via tools (grep, read_file, etc.)

For agents to succeed on the first try, the task definition must be **self-contained**.

## What a Fresh Agent Needs (Minimum)

### Always Required
- **Task title**: imperative verb phrase ("Rename Signal → Engram in roko-agent")
- **Files to modify**: explicit list, 1-2 files max per task
- **Verification commands**: shell commands with exit 0 = pass

### Highly Recommended
- **`[task.context].read_files`**: exact file paths + line ranges + "why"
- **`[task.context].symbols`**: function/type signatures the agent needs
- **`[task.context].anti_patterns`**: 3-5 specific DO NOTs
- **`[task.context].prior_failures`**: if retrying, the exact error

### What Guarantees Success
- ≤50 LOC for focused tasks (mechanical ≤20)
- 1-3 files to read, 1-2 to modify
- Executable verification only (no subjective criteria)
- Model tier alignment: haiku for mechanical, sonnet for focused, opus for architectural

## The Context Tier System

The orchestrator assembles context based on task complexity:

| Tier | Model | Token Budget | What's Included |
|------|-------|-------------|-----------------|
| Surgical | Haiku/Gemma | ≤4K | Inline files only |
| Focused | Sonnet | ≤12K | + task brief + dependency graph |
| Full | Opus | ≤24K | + plan brief + research + invariants |

## How to Write Better PRDs for Plan Generation

The plan generator agent reads the PRD and codebase to produce `tasks.toml`. The quality of the generated plan depends entirely on the PRD quality.

### PRD Must Include

1. **Specific file paths**: not "update the agent crate" but "update `crates/roko-agent/src/dispatcher/mod.rs` lines 45-80"
2. **Current state description**: what exists now, what works, what doesn't
3. **Target state description**: what should exist after, with concrete examples
4. **Naming rules**: if names are changing, the exact mapping
5. **Verification criteria**: shell commands that prove the change worked
6. **Anti-patterns**: what NOT to do (critical for preventing the "reimplement instead of wire" mistake)

### PRD Template for Refactoring Tasks

```markdown
---
id: prd-rename-bardo-runtime
title: Rename bardo-runtime to roko-runtime
status: published
crates: [bardo-runtime, roko-cli, roko-core, roko-serve]
---

# Rename bardo-runtime to roko-runtime

## Current State
- Crate at `crates/bardo-runtime/` with `name = "bardo-runtime"` in Cargo.toml
- Depended on by: roko-cli, roko-core, roko-serve, mirage-rs
- Import pattern: `use bardo_runtime::{cancel, event_bus, process, metrics, resource}`

## Target State
- Crate at `crates/roko-runtime/` with `name = "roko-runtime"`
- Same API, same modules, just renamed
- All dependents use `use roko_runtime::`

## Files to Modify
1. `crates/bardo-runtime/Cargo.toml` → rename to `crates/roko-runtime/Cargo.toml`
2. `Cargo.toml` (workspace members line)
3. `crates/roko-cli/Cargo.toml` (dependency)
4. `crates/roko-core/Cargo.toml` (dependency)
5. `crates/roko-serve/Cargo.toml` (dependency)
6. All `.rs` files containing `bardo_runtime`

## Verification
- `! grep -rn 'bardo.runtime' crates/ --include='*.toml' | grep -v target/`
- `! grep -rn 'bardo_runtime' crates/ --include='*.rs' | grep -v target/`
- `cargo check --workspace`

## Anti-patterns
- Do NOT change any API signatures — this is a rename only
- Do NOT add or remove any modules
- Do NOT update any code logic
```

## Enriching PRDs Before Plan Generation

### Manual Enrichment
For each draft PRD, append a `## Codebase Context` section with:
- Exact file paths that need modification (with line ranges)
- Naming rules from the glossary that apply
- Existing code that should be wired (not rebuilt)

### Script-Assisted Enrichment
```bash
# For each PRD, generate impact analysis
for prd in .roko/prd/published/*.md; do
  slug=$(basename $prd .md)
  # Extract crate names from frontmatter
  crates=$(grep '^crates:' $prd | sed 's/crates: \[//;s/\]//;s/,/ /g')
  echo "=== $slug ==="
  for crate in $crates; do
    echo "  Files in $crate:"
    find "crates/$crate" -name '*.rs' | head -20
  done
done
```

### Agent-Assisted Enrichment
```bash
roko research enhance-prd <slug>
```
This dispatches a research agent that reads the PRD and codebase, then adds context.

## Improving the Plan Generator

### Current Limitation
The planner base prompt is no longer generic: `plan_generate.rs` now layers naming glossary and `CLAUDE.md` excerpts into the shared builder. The remaining risk is caller drift when a planner-style path passes `PLAN_GENERATOR_SYSTEM_PROMPT` directly instead of using that builder.

### Fix: Use Shared Prompt Builders Everywhere
Planner call sites should use the shared helpers in `plan_generate.rs` so every path gets the same naming glossary and workspace-rule context.

### Included Context
The shared planner builder already includes the project's `CLAUDE.md` so agents inherit the critical rules:
- "NEVER reimplement what already exists"
- "WIRE, don't build"
- "Verify before marking done"

## Breaking Changes: Multi-Plan Strategy

For changes that span many crates (like Signal → Engram):

### Don't: One Giant Plan
```
Plan: rename-signal-to-engram
  T1: Rename in core (blocks T2-T18)
  T2: Update roko-agent
  T3: Update roko-gate
  ... T18: Update roko-chain
  T19: Full verification
```
This creates a 19-task plan with complex dependencies. One failure blocks everything.

### Do: Multiple Small Plans
```
Plan P1: core-rename (2 tasks, sequential)
  T1: Rename struct + builder in roko-core
  T2: Update exports, add compat alias

Plan P2: consumer-updates (depends_on_plan: [P1])
  T1-T18: One task per consumer crate (parallel, max_parallel = 4)

Plan P3: cleanup (depends_on_plan: [P2])
  T1: Remove compat alias
  T2: cargo check + cargo test
```

### Use `depends_on_plan` for Cross-Plan Blocking
```toml
[meta]
plan = "P2-consumer-updates"
depends_on_plan = ["P1-core-rename"]
```

## Pre-Generation Scripts

### Impact Map Generator
Before creating a PRD for a rename, generate the exact scope:

```bash
#!/bin/bash
# Generate impact map for a rename
OLD=$1  # e.g., "bardo_runtime"
NEW=$2  # e.g., "roko_runtime"

echo "=== Files containing '$OLD' ==="
grep -rn "$OLD" crates/ --include='*.rs' --include='*.toml' | grep -v target/ \
  | cut -d: -f1 | sort -u | while read f; do
    count=$(grep -c "$OLD" "$f")
    echo "  $f ($count occurrences)"
  done

echo ""
echo "=== Crates affected ==="
grep -rn "$OLD" crates/ --include='*.rs' | grep -v target/ \
  | cut -d: -f1 | sed 's|crates/\([^/]*\)/.*|\1|' | sort -u

echo ""
echo "=== Total occurrences ==="
grep -rn "$OLD" crates/ --include='*.rs' --include='*.toml' | grep -v target/ | wc -l
```

This output goes directly into the PRD so the plan generator knows the exact scope.
