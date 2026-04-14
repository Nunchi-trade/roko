# Roko Quickstart

> Four levels of depth. Start at Level 1, go as deep as you need.

---

## Level 1: What Is Roko?

Roko is a Rust toolkit for building agents that build themselves. It provides a cognitive
architecture — not just prompt chaining — where agents plan work, execute it via LLMs,
verify results through gate pipelines, learn from outcomes, and persist everything as
content-addressed, decaying data (called "Engrams" in docs, `Signal` in code). The core
loop is wired end-to-end: Roko already uses itself to develop itself.

**18 crates, ~177K LOC, 1,568 tests.**

---

## Level 2: Architecture in 60 Seconds

**One noun, six verbs.**

The noun is the **Engram** (`Signal` in Rust) — a universal content-addressed datum with
BLAKE3 hashing, 7-axis scoring, four decay models, lineage tracking, and provenance stamps.
Every event, output, verdict, and knowledge entry is an Engram.

The six verbs are traits that operate on Engrams:

| Trait | What It Does |
|-------|-------------|
| **Substrate** | Store and query Engrams (JSONL files, in-memory, chain-backed) |
| **Scorer** | Score Engrams on relevance, confidence, urgency, etc. |
| **Gate** | Verify Engrams (compile, test, lint, diff, semantic checks) |
| **Router** | Select which model/tier handles a task (CascadeRouter: T0→T1→T2) |
| **Composer** | Assemble context for the LLM (6-layer prompt builder, token budgets) |
| **Policy** | React to events (circuit breakers, escalation, safety enforcement) |

**Universal loop**: query → score → route → compose → act → verify → persist → react.

Three cognitive speeds:
- **Gamma** (~5-15s): Reactive — handle immediate tasks
- **Theta** (~75s): Reflective — evaluate progress, adjust strategy
- **Delta** (hours): Consolidation — offline learning, knowledge distillation

---

## Level 3: Use It (Self-Hosting Workflow)

```bash
# Setup
cd /path/to/your/project
rustup update stable          # Need 1.91+ for alloy deps
cargo build --workspace

# The self-hosting loop:

# 1. Capture a work item
cargo run -p roko-cli -- prd idea "Add retry logic to agent dispatch"

# 2. Draft a PRD from the idea (agent-driven)
cargo run -p roko-cli -- prd draft new "retry-logic"

# 3. Research for context (optional)
cargo run -p roko-cli -- research enhance-prd retry-logic

# 4. Generate implementation plan + tasks from the PRD
cargo run -p roko-cli -- prd plan retry-logic

# 5. Execute the plan (agents run tasks, gates validate, state persists)
cargo run -p roko-cli -- plan run plans/

# 6. Resume if interrupted
cargo run -p roko-cli -- plan run plans/ --resume .roko/state/executor.json

# 7. Watch progress
cargo run -p roko-cli -- dashboard

# 8. Check status
cargo run -p roko-cli -- status
```

### Key commands

| Command | Purpose |
|---------|---------|
| `roko init` | Initialize `.roko/` directory and `roko.toml` |
| `roko run "<prompt>"` | Single prompt through the universal loop |
| `roko plan run <dir>` | Execute a plan (the main orchestration loop) |
| `roko prd idea/draft/plan` | PRD lifecycle management |
| `roko research topic/enhance-prd` | Deep research with citations |
| `roko status` | Query signals, report counts |
| `roko config show/edit/set` | Configuration management |

### Configuration

Agent behavior is configured in `roko.toml`:

```toml
[agent]
model = "claude-opus-4-6"        # Default LLM
mcp_config = ".mcp.json"      # MCP server config (auto-discovered)
max_turns = 25                 # Max agent turns per task
timeout_seconds = 600          # Per-task timeout

[gate]
pipeline = ["compile", "test", "clippy", "diff"]  # Gate rungs to apply

[learn]
cascade_router = true          # Enable T0/T1/T2 model routing
experiments = true             # Enable prompt A/B testing
```

---

## Level 4: Navigate the PRD Corpus

The docs are organized into 22 sections. Here's the reading order for different goals:

### "I want to understand the architecture"
1. [`00-architecture/`](00-architecture/INDEX.md) — Start here. Signal/Engram, 6 traits, universal loop.
2. [`16-heartbeat/`](16-heartbeat/INDEX.md) — The cognitive clock (Gamma/Theta/Delta speeds).
3. [`00-architecture/12-five-layer-taxonomy.md`](00-architecture/12-five-layer-taxonomy.md) — How crates are layered.
4. [`00-architecture/15-crate-map.md`](00-architecture/15-crate-map.md) — Every crate, its status, and its role.

### "I want to understand how agents work"
1. [`02-agents/`](02-agents/INDEX.md) — Agent types, LLM backends, dispatch.
2. [`03-composition/`](03-composition/INDEX.md) — How prompts are assembled (6-layer builder).
3. [`01-orchestration/`](01-orchestration/INDEX.md) — Plan DAG execution, parallel scheduling.
4. [`04-verification/`](04-verification/INDEX.md) — Gate pipeline (how outputs are verified).

### "I want to understand the cognitive subsystems"
1. [`09-daimon/`](09-daimon/INDEX.md) — Affect engine (PAD vectors, behavioral states, somatic markers).
2. [`06-neuro/`](06-neuro/INDEX.md) — Knowledge management (6 types, 4 tiers, HDC encoding).
3. [`10-dreams/`](10-dreams/INDEX.md) — Offline learning (NREM replay, REM imagination).
4. [`05-learning/`](05-learning/INDEX.md) — Episodes, playbooks, bandits, experiments.

### "I want to see what's implemented vs. specified"
→ [`STATUS.md`](STATUS.md) — The master status matrix.

### "I want to implement something"
1. Check [`STATUS.md`](STATUS.md) for the current tier of your target.
2. Read the relevant section's INDEX.md for the full doc list.
3. Read the section's status/gaps doc (if it has one) for known blockers.
4. Check `CLAUDE.md` at the repo root for critical development rules.

---

## Key Directories

| Path | What |
|------|------|
| `crates/` | All 18+ Rust crates |
| `crates/roko-cli/src/` | CLI entry point and subcommands |
| `crates/roko-cli/src/orchestrate.rs` | The main plan-execute-gate-persist loop |
| `crates/roko-core/` | Kernel: Signal, 6 traits, config |
| `.roko/` | Runtime data (signals, episodes, state, dreams, learn) |
| `.roko/prd/` | PRD storage |
| `.roko/state/` | Executor snapshots for resume |
| `docs/` | This PRD corpus (375+ documents, 22 sections) |
