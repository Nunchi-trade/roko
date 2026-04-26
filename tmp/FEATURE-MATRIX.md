# Roko Feature Matrix

> **Audit warning (2026-04-26)**: this matrix contains stale claims. The current
> source audit is in
> [`tmp/mori-diffs/28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md`](mori-diffs/28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md).
> Notable corrections: `roko-serve` currently passes `cargo check -p roko-serve --lib`;
> runner v2 is the default for `roko plan run`; cascade-router persistence is wired
> and candidates now come from effective model config, but all-provider routing is
> not proven; runner v2 uses `max_gate_rung` for the current compile/clippy/test
> path but does not yet execute the full advertised 7-rung gate ladder; adaptive
> thresholds, replan-on-gate-failure, universal
> safety, dreams auto-trigger, and daimon state loading remain open.

> **Last audited**: 2026-04-26 source re-check in `tmp/mori-diffs/28-FEATURE-MATRIX-DOGFOOD-UX-AUDIT.md`
> **Build status**: ✅ `cargo check -p roko-serve --lib` passed without warnings on 2026-04-26 after removing stale unused imports.
> **Test count**: not re-counted in the 2026-04-26 source re-check.

This is an inventory and historical feature matrix. Treat the source audit linked above as the current implementation-status reference until this file is fully reconciled.

---

## Status Legend

| Symbol | Meaning |
|--------|---------|
| ✅ WIRED | Fully connected in the active runtime path, produces/consumes data |
| ⚠️ PARTIAL | Infrastructure exists, partially connected, loop not fully closed |
| ❌ NOT WIRED | Code exists in crates but never called from the active execution path |
| 🔨 BROKEN | Was wired but currently fails (compile error, runtime bug) |

---

## 1. Core Execution Loop

The active runtime for `roko plan run` is **runner v2** at `crates/roko-cli/src/runner/event_loop.rs`. The legacy `orchestrate.rs` (21.5K lines) still exists but is NOT called from plan execution.

### 1.1 Plan DAG Execution

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-cli/src/runner/event_loop.rs` + `crates/roko-orchestrator/` |
| **What it does** | Topological sort of tasks.toml → parallel dispatch → gate after each task → retry on failure |
| **Entry point** | `roko plan run <dir>` → `commands/plan.rs:305` → `runner::event_loop::run()` |
| **Parallelism** | Respects `depends_on` in tasks.toml; independent tasks run concurrently |
| **Resume** | `--resume .roko/state/executor.json` skips completed tasks |
| **Dry run** | `--dry-run` shows execution plan without running |

**How to test:**
```bash
# Create a minimal plan
mkdir -p /tmp/test-plan && cat > /tmp/test-plan/tasks.toml << 'EOF'
[[task]]
id = "hello"
title = "Say hello"
prompt = "Create a file called hello.txt containing 'hello world'"
role = "implementer"
EOF

# Run it
cargo run -p roko-cli -- plan run /tmp/test-plan/

# Verify
cat .roko/state/executor.json | jq '.completed_tasks'
```

**How to verify it's working:**
- `.roko/state/executor.json` exists with task completion records
- `.roko/episodes.jsonl` has entries for each task
- Agent output appears in stdout/TUI

---

### 1.2 Agent Dispatch

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-cli/src/runner/agent_stream.rs` |
| **What it does** | Spawns Claude CLI (or other backend) with composed system prompt + task prompt |
| **Model selection** | task `model_hint` > CascadeRouter > config default |
| **Streaming** | Real-time stdout parsing of agent output |
| **Process isolation** | Process group set, PID tracked, kill on cancel with grace period |

**How to test:**
```bash
# Single-shot execution
cargo run -p roko-cli -- run "Create a file called test.txt with 'it works'"

# Verify agent was dispatched
ls -la test.txt
cat .roko/episodes.jsonl | tail -1 | jq '.agent_model'
```

---

### 1.3 Gate Pipeline

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-cli/src/runner/gate_dispatch.rs` + `crates/roko-gate/` |
| **What it does** | After each agent turn: compile → test → clippy → custom verify steps |
| **Rung system** | 7 rungs from fastest (syntax) to slowest (integration test) |
| **Timeout** | Configurable per gate (default: 300s) |
| **Failure handling** | Retry up to `max_retries` (default: 3) with backoff |

**How to test:**
```bash
# Run a plan that modifies Rust code (gates will fire)
cargo run -p roko-cli -- plan run plans/ --dry-run  # See which gates would run

# Check gate results in episodes
cat .roko/episodes.jsonl | jq 'select(.gate_outcome != null) | .gate_outcome'
```

**How to verify it's working:**
- Gate verdicts appear in episode metadata (`gate_outcome` field)
- `cargo check`, `cargo test`, `cargo clippy` actually execute (visible in agent output)
- Failed gates trigger retry (visible in executor state)

---

### 1.4 Session Persistence & Resume

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-cli/src/runner/event_loop.rs` (snapshot logic) |
| **What it does** | Saves executor state after each task completion; resume skips completed tasks |
| **State file** | `.roko/state/executor.json` |
| **Resume command** | `roko plan run <dir> --resume .roko/state/executor.json` |

**How to test:**
```bash
# Start a plan, Ctrl-C mid-execution
cargo run -p roko-cli -- plan run plans/
# ^C

# Resume from where it stopped
cargo run -p roko-cli -- plan run plans/ --resume .roko/state/executor.json

# Verify no duplicate completions
cat .roko/episodes.jsonl | jq '.task_id' | sort | uniq -c | sort -rn | head
```

---

## 2. Learning & Feedback

### 2.1 CascadeRouter (Model Selection)

| Aspect | Detail |
|--------|--------|
| **Status** | ⚠️ PARTIAL — CLOSED LOOP EXISTS, ALL-PROVIDER PROOF MISSING |
| **Location** | `crates/roko-learn/src/cascade_router.rs`, called from `runner/event_loop.rs:934-978` |
| **Algorithm** | LinUCB contextual bandit (3-stage: Static < 50 obs → Confidence 50-200 → UCB > 200) |
| **State file** | `.roko/learn/cascade-router.json` |
| **Feedback** | Gate quality + cost + latency fed back as multi-objective observations |
| **Model priority** | explicit `model_hint` > cascade router candidates from effective model config > config default |

**How to test:**
```bash
# Check current router state
cargo run -p roko-cli -- learn router

# Run several tasks, then check if model selection changed
cargo run -p roko-cli -- plan run plans/
cat .roko/learn/cascade-router.json | jq '.observations | length'

# Verify routing decisions in episodes
cat .roko/episodes.jsonl | jq '{task: .task_id, model: .agent_model}' | tail -5
```

**How to verify the loop is closed:**
1. `.roko/learn/cascade-router.json` grows with observations after each run
2. Model selection changes as router learns (check episode `agent_model` field)
3. `roko learn router` shows per-model scores and selection counts

---

### 2.2 Episode Logging

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED (write path) |
| **Location** | `crates/roko-cli/src/runner/event_loop.rs:2075-2264` |
| **Output** | `.roko/episodes.jsonl` |
| **Fields** | agent_model, input_tokens, output_tokens, cost_usd, files_changed, gate_outcome, hdc_fingerprint, task_id, role, duration |
| **Feedback loop** | ❌ Episodes are NOT read back to influence future runs (one-way write) |

**How to test:**
```bash
# After any plan run
cat .roko/episodes.jsonl | jq -c '{model: .agent_model, cost: .cost_usd, gate: .gate_outcome}' | tail -5

# Check HDC fingerprints are attached
cat .roko/episodes.jsonl | jq 'select(.hdc_fingerprint != null)' | wc -l
```

---

### 2.3 Efficiency Events

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-cli/src/runner/event_loop.rs:2266-2305` |
| **Output** | `.roko/learn/efficiency.jsonl` |
| **Fields** | model, task_wall_ms, input_tokens, output_tokens, cost_usd, gate_verdicts |
| **Consumer** | Conductor reads for load pressure; NOT fed to cascade router |

**How to test:**
```bash
cargo run -p roko-cli -- learn efficiency
cat .roko/learn/efficiency.jsonl | jq -c '{model: .model, cost: .cost_usd, wall_ms: .task_wall_ms}' | tail
```

---

### 2.4 Section Effectiveness (Prompt Learning)

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED — CLOSED LOOP |
| **Location** | `crates/roko-compose/` (Beta distribution tracker) |
| **What it does** | Tracks which prompt sections correlate with gate success via Beta(α,β) per section |
| **Feedback** | Gate pass → α++; gate fail → β++; next prompt allocates more tokens to high-α sections |
| **State** | Persisted in compose state; reloaded on next run |

**How to verify:**
- After multiple runs, prompt composition changes (more tokens to effective sections)
- Observable via `roko learn all` output

---

### 2.5 Adaptive Gate Thresholds

| Aspect | Detail |
|--------|--------|
| **Status** | ❌ NOT WIRED |
| **Location** | `crates/roko-learn/` (AdaptiveThresholds struct) |
| **State file** | `.roko/learn/gate-thresholds.json` (exists but always empty: `{"rungs": {}}`) |
| **Problem** | Thresholds are loaded but never populated; gates use hardcoded cutoffs |
| **What should happen** | EMA of pass rates per rung → tighten thresholds when pass rate high, loosen when low |

**How to verify it's broken:**
```bash
cat .roko/learn/gate-thresholds.json
# Shows: {"rungs": {}} — never populated
```

**To fix:** Wire `update_threshold(rung, passed)` call in `gate_dispatch.rs` after each gate verdict.

---

### 2.6 Playbook Store

| Aspect | Detail |
|--------|--------|
| **Status** | ⚠️ PARTIAL |
| **Location** | `crates/roko-learn/src/playbooks.rs` |
| **Query path** | ✅ Wired — queries by task title + role at dispatch time |
| **Injection** | ✅ Wired — results injected as "Learned techniques" in system prompt Layer 6 |
| **Save path** | ⚠️ Only saves on task SUCCESS — if gates fail, nothing saved |
| **Problem** | If first runs fail gates, playbook store stays empty forever |

**How to test:**
```bash
ls .roko/learn/playbooks/
# Empty if no tasks have passed gates yet

# After a successful task:
cat .roko/learn/playbooks/*.json | jq '.title'
```

---

### 2.7 Knowledge/Neuro Store

| Aspect | Detail |
|--------|--------|
| **Status** | ❌ NOT WIRED (broken initialization) |
| **Location** | `crates/roko-neuro/src/` |
| **Write path** | Code exists to ingest episodes into neuro store |
| **Problem** | `.roko/neuro/` directory never created → writes fail silently |
| **Query path** | Never queried at dispatch time in runner v2 |
| **What should happen** | Successful patterns stored as knowledge Signals → queried for similar tasks → injected into prompts |

**How to verify it's broken:**
```bash
ls .roko/neuro/ 2>&1
# "No such file or directory"
```

**To fix:** Add `std::fs::create_dir_all(".roko/neuro/")` in initialization + wire query into dispatch.

---

### 2.8 Daimon/Affect System

| Aspect | Detail |
|--------|--------|
| **Status** | ⚠️ PARTIAL (instantiated but default) |
| **Location** | `crates/roko-daimon/` + `runner/event_loop.rs:961` |
| **Problem** | `DaimonPolicy::default()` used — never loaded from disk state |
| **What should happen** | PAD vector (pleasure/arousal/dominance) loaded from `.roko/daimon/` → influences model routing (high arousal → prefer faster models), prompt flavoring (affect guidance layer), and exploration rate |
| **Affect on prompts** | Layer 8 of system prompt builder accepts affect guidance — but gets zero-vector |

**How to verify it's broken:**
```bash
cat .roko/episodes.jsonl | jq '.affect_state'
# All null or default values
```

---

### 2.9 Dreams/Consolidation

| Aspect | Detail |
|--------|--------|
| **Status** | ❌ NOT WIRED (never triggered automatically) |
| **Location** | `crates/roko-dreams/` |
| **Manual trigger** | `roko knowledge dream run` works (runs NREM replay + REM imagination + integration) |
| **Automatic trigger** | Never called after plan completion or on schedule |
| **What should happen** | After plan completion: consolidate episodes → extract patterns → promote knowledge tiers → generate routing advice |

**How to test manually:**
```bash
cargo run -p roko-cli -- knowledge dream run
cargo run -p roko-cli -- knowledge dream report
```

---

## 3. Prompt Engineering

### 3.1 System Prompt Builder (9 Layers)

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-compose/src/system_prompt_builder.rs` + `templates/` |
| **Layers** | 1: Role identity, 2: Conventions, 3: Domain context, 4: Task context, 5: Tool instructions, 6: Skills + playbooks, 7: Anti-patterns, 8: Affect guidance, 9: Constraints |
| **Dynamic content** | Task context, anti-patterns from prior failures, playbooks (if any exist) |
| **Templates** | Static .md files in `crates/roko-compose/src/templates/` per role |

**How to test:**
```bash
# The prompt is visible in verbose/debug output during plan run
RUST_LOG=debug cargo run -p roko-cli -- run "test prompt" 2>&1 | grep -A5 "system_prompt"
```

---

### 3.2 Anti-Pattern Injection

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `runner/agent_stream.rs:396-402` |
| **What it does** | If prior attempt on same task failed, failure context injected into prompt Layer 7 |
| **Format** | "Previous attempt failed because: [gate output]. Avoid: [specific anti-pattern]" |

---

## 4. Safety & Security

### 4.1 Safety Layer (6 Guards)

| Aspect | Detail |
|--------|--------|
| **Status** | ❌ NOT WIRED in runner v2 |
| **Location** | `crates/roko-agent/src/safety/` |
| **Guards** | BashPolicy, GitPolicy, NetworkPolicy, PathPolicy, ScrubPolicy, RateLimit |
| **Problem** | Runner v2 dispatches agents without calling SafetyLayer — tool calls go unchecked |
| **Where it IS wired** | Old `orchestrate.rs` (not active) and `roko-agent` routed dispatch path |
| **Impact** | No per-tool-call permission enforcement, no secret scrubbing, no rate limiting |

**Critical gap.** The #1 priority fix.

---

### 4.2 MCP Config Passthrough

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `runner/event_loop.rs:212-237`, `agent_stream.rs:241-259` |
| **What it does** | MCP server configs from `roko.toml` → `--mcp-config` flag on agent spawn |
| **Supports** | Multiple MCP servers, environment variable injection |

**How to test:**
```bash
# Add to roko.toml:
# [[agent.mcp_servers]]
# name = "code-intelligence"
# command = "roko"
# args = ["mcp", "code-intelligence"]

cargo run -p roko-cli -- run "Search for the Engram struct"
# Agent should have access to code search tools
```

---

### 4.3 Process Supervision

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `runner/agent_stream.rs:85-114` |
| **Features** | PID tracking, process group isolation, kill on cancel with grace, orphan cleanup at startup |

---

## 5. Infrastructure

### 5.1 HTTP Control Plane (roko-serve)

| Aspect | Detail |
|--------|--------|
| **Status** | 🔨 BROKEN (compile error) |
| **Location** | `crates/roko-serve/src/` |
| **Routes** | ~85 REST routes + SSE + WebSocket on :6677 |
| **Error** | Missing fields `cross_episode_report` and `routing_recommendations` in `DreamCycleReport` (4 instances in `dreams.rs`) |
| **Port** | 6677 (configurable) |

**How to fix:**
```bash
# Add missing fields to DreamCycleReport initializers in crates/roko-serve/src/dreams.rs
# Lines 556, 613, 642, 711
```

---

### 5.2 Interactive TUI (Dashboard)

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-cli/src/tui/` |
| **Features** | 6 tabs (Health, Trends, Plans, Episodes, Knowledge, Config), 60fps ratatui, file watcher |
| **Command** | `roko dashboard` |

**How to test:**
```bash
cargo run -p roko-cli -- dashboard
# Or text mode:
cargo run -p roko-cli -- dashboard --text
```

---

### 5.3 Per-Agent Sidecar

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-agent-server/` |
| **Routes** | 13 routes: /message (real LLM dispatch), /stream (WebSocket), /predictions, /research, /tasks |
| **Command** | `roko agent serve` |

---

### 5.4 Research Agent

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-cli/src/commands/research.rs` |
| **Backends** | Perplexity (`sonar-deep-research`) for topic research, direct search |
| **Output** | `.roko/research/<topic>.md` |

**How to test:**
```bash
cargo run -p roko-cli -- research topic "active inference in agent systems"
cat .roko/research/active-inference-in-agent-systems.md
```

---

### 5.5 PRD Lifecycle

| Aspect | Detail |
|--------|--------|
| **Status** | ✅ WIRED |
| **Location** | `crates/roko-cli/src/commands/prd.rs` |
| **Flow** | idea → draft → promote → plan |
| **Storage** | `.roko/prd/` |

**How to test:**
```bash
cargo run -p roko-cli -- prd idea "Implement feature X"
cargo run -p roko-cli -- prd draft new feature-x
cargo run -p roko-cli -- prd plan feature-x
ls plans/feature-x/tasks.toml
```

---

## 6. NOT WIRED — The 6 Critical Gaps

### Gap 1: Safety Pipeline Not Universal

| Aspect | Detail |
|--------|--------|
| **Severity** | HIGH |
| **Impact** | Tool calls bypass permission checks, secret scrubbing, rate limiting |
| **Fix location** | `crates/roko-cli/src/runner/agent_stream.rs` |
| **Fix size** | ~50 lines — wrap tool dispatch in `SafetyLayer::check()` |
| **Runner prompt** | M153 |

---

### Gap 2: Knowledge Store Not Initialized

| Aspect | Detail |
|--------|--------|
| **Severity** | MEDIUM |
| **Impact** | Learned patterns never persist; system can't improve from experience |
| **Fix location** | `crates/roko-cli/src/runner/event_loop.rs` (init) + dispatch (query) |
| **Fix size** | ~30 lines — create dir + wire query at dispatch time |
| **Runner prompt** | M147 (partial) |

---

### Gap 3: Gate Thresholds Never Learn

| Aspect | Detail |
|--------|--------|
| **Severity** | MEDIUM |
| **Impact** | Gates are always equally strict; can't adapt to project quality level |
| **Fix location** | `crates/roko-cli/src/runner/gate_dispatch.rs` |
| **Fix size** | ~40 lines — call `threshold.update(rung, passed)` after each verdict |
| **Runner prompt** | M148 (partial) |

---

### Gap 4: Dreams Never Triggered

| Aspect | Detail |
|--------|--------|
| **Severity** | LOW (manual trigger works) |
| **Impact** | No automatic consolidation of learned patterns |
| **Fix location** | `crates/roko-cli/src/runner/event_loop.rs` (end of plan) |
| **Fix size** | ~20 lines — call `DreamRunner::consolidate_now()` after plan completion |
| **Runner prompt** | M142 (partial) |

---

### Gap 5: Daimon Always Default

| Aspect | Detail |
|--------|--------|
| **Severity** | LOW |
| **Impact** | Affect doesn't modulate behavior (exploration rate, model selection, prompt tone) |
| **Fix location** | `crates/roko-cli/src/runner/event_loop.rs:961` |
| **Fix size** | ~15 lines — load from `.roko/daimon/affect.json` instead of `default()` |
| **Runner prompt** | None yet (trivial fix) |

---

### Gap 6: Replan on Gate Failure

| Aspect | Detail |
|--------|--------|
| **Severity** | MEDIUM |
| **Impact** | Failed tasks retry blindly instead of generating revised approach |
| **Fix location** | `crates/roko-cli/src/runner/event_loop.rs` (retry logic) |
| **Fix size** | ~80 lines — after N failures, generate revised task prompt via agent |
| **Runner prompt** | None in current set (would be M172) |

---

## 7. Complete CLI Command Reference

### Fully Working Commands

```bash
# Core workflow
roko init                                    # Create .roko/ and roko.toml
roko run "<prompt>"                          # Single prompt → compose → agent → gate → persist
roko status                                  # Query signals, report counts
roko doctor                                  # Diagnose workspace state

# Planning
roko plan list                               # List discovered plans
roko plan show <dir>                         # Show plan details
roko plan create                             # Create new plan directory
roko plan run <dir>                          # Execute plan (main orchestration loop)
roko plan run <dir> --resume <state.json>    # Resume interrupted plan
roko plan run <dir> --dry-run                # Show what would execute
roko plan validate <dir>                     # Lint tasks.toml without executing
roko plan generate "<prompt>"                # Generate plan from prompt
roko plan regenerate <dir>                   # Regenerate plan from context

# PRDs
roko prd idea "<text>"                       # Capture work item
roko prd list                                # List PRDs
roko prd status                              # Coverage report
roko prd draft new <slug>                    # Create PRD draft
roko prd draft edit <slug>                   # Edit existing draft
roko prd draft promote <slug>                # Promote draft → published
roko prd plan <slug>                         # Generate tasks.toml from PRD
roko prd consolidate                         # Scan for gaps/duplicates

# Research
roko research topic "<topic>"                # Deep research with citations
roko research search "<query>"               # Direct web search
roko research enhance-prd <slug>             # Enhance PRD with research
roko research enhance-plan <dir>             # Enhance plan with research
roko research analyze                        # Analyze execution data

# Knowledge
roko knowledge query "<topic>"               # Search knowledge store
roko knowledge stats                         # Store statistics
roko knowledge gc                            # Garbage collection
roko knowledge backup                        # Backup with genomic bottleneck
roko knowledge restore <archive>             # Restore with confidence decay
roko knowledge sync <peer>                   # Mesh sync
roko knowledge dream run                     # Run dream consolidation
roko knowledge dream report                  # Show dream report
roko knowledge dream journal                 # Dream journal entries
roko knowledge custody list/show/verify      # Audit chain

# Learning
roko learn all                               # Inspect all learning state
roko learn router                            # Show cascade router state
roko learn experiments                       # Show A/B experiments
roko learn efficiency                        # Show cost/performance data
roko learn episodes                          # Show episode history
roko learn tune gates                        # Tune gate thresholds
roko learn tune routing                      # Tune routing policy

# Agents
roko agent create --name X --domain Y        # Create agent manifest
roko agent start --name X                    # Start long-running agent
roko agent stop --name X                     # Stop agent
roko agent list                              # List with status
roko agent status --name X                   # Detailed health
roko agent serve                             # Start HTTP sidecar
roko agent chat --agent X                    # Interactive REPL

# Configuration
roko config show                             # Show resolved config
roko config set <key> <value>                # Set config value
roko config validate                         # Validate schema
roko config providers list                   # List LLM providers
roko config providers health                 # Provider health check
roko config models list                      # List available models
roko config secrets set <key> <value>        # Set secret

# Server & deployment
roko serve                                   # HTTP control plane (:6677)
roko dashboard                               # Interactive TUI
roko daemon start/stop/status/logs           # Background daemon

# Utilities
roko replay <hash>                           # Walk signal DAG
roko index build                             # Build code intelligence index
roko index search "<query>"                  # Search code index
roko new <type> <name>                       # Scaffold boilerplate
roko explain <topic>                         # Concept explainer
roko completions <shell>                     # Shell completions
```

---

## 8. Self-Hosting Loop (End-to-End Verification)

This is the complete loop that verifies roko can develop itself:

```bash
# Step 1: Initialize (if fresh)
cargo run -p roko-cli -- init

# Step 2: Capture idea
cargo run -p roko-cli -- prd idea "Add adaptive gate threshold learning"

# Step 3: Draft PRD
cargo run -p roko-cli -- prd draft new adaptive-gates

# Step 4: Research the topic
cargo run -p roko-cli -- research enhance-prd adaptive-gates

# Step 5: Generate plan
cargo run -p roko-cli -- prd plan adaptive-gates

# Step 6: Verify plan makes sense
cargo run -p roko-cli -- plan validate plans/adaptive-gates/

# Step 7: Execute
cargo run -p roko-cli -- plan run plans/adaptive-gates/

# Step 8: Check results
cargo run -p roko-cli -- status
cargo run -p roko-cli -- learn all

# Step 9: If interrupted, resume
cargo run -p roko-cli -- plan run plans/adaptive-gates/ --resume .roko/state/executor.json

# Step 10: Consolidate learnings (manual until auto-trigger wired)
cargo run -p roko-cli -- knowledge dream run
```

**Expected outcomes after a successful run:**
- `.roko/episodes.jsonl` has new entries with full metadata
- `.roko/learn/cascade-router.json` has new observations
- `.roko/learn/efficiency.jsonl` has cost/token events
- `.roko/state/executor.json` shows completed tasks
- Git has new commits from agent work (if gates passed)
- `roko learn router` shows updated model scores

---

## 9. Configuration (roko.toml)

Minimal working configuration:

```toml
[agent]
default_model = "claude-sonnet-4-6"
max_turns = 25
max_retries = 3

[gates]
pipeline = ["compile", "test", "clippy"]
timeout_seconds = 300

[learning]
cascade_router_enabled = true
efficiency_tracking = true
episode_logging = true

# Optional: MCP servers
# [[agent.mcp_servers]]
# name = "code-intelligence"
# command = "roko"
# args = ["mcp", "code-intelligence"]
```

---

## 10. File Map (State & Learning)

```
.roko/
├── roko.toml                          # Configuration
├── signals.jsonl                      # Signal log (durable audit trail)
├── episodes.jsonl                     # Agent turn records ✅
├── state/
│   └── executor.json                  # Plan execution snapshot ✅
├── learn/
│   ├── cascade-router.json            # Model routing state ✅ (CLOSED LOOP)
│   ├── efficiency.jsonl               # Cost/token events ✅
│   ├── efficiency-summaries.jsonl     # Aggregate snapshots ✅
│   ├── gate-thresholds.json           # Adaptive thresholds ❌ (always empty)
│   ├── gate-ratchet.json              # CUSUM ratcheting ❌ (stub)
│   ├── conductor.json                 # Operational pressure ✅
│   ├── costs.jsonl                    # Per-model cost tracking ✅
│   ├── provider-model-outcomes.jsonl  # Success rates by model ✅
│   ├── latency-stats.json            # Response time tracking ✅
│   ├── skills.json                    # Learned prompt techniques ✅
│   ├── task-metrics.jsonl             # Per-task performance ✅
│   ├── c-factor.jsonl                 # Collective intelligence ✅
│   ├── local-rewards.json             # Reward definitions ✅
│   ├── experiment-winners.json        # A/B results ❌ (empty)
│   └── playbooks/                     # Reusable task patterns ⚠️ (empty until gates pass)
├── neuro/                             # Knowledge store ❌ (dir not created)
├── prd/                               # PRD storage ✅
├── research/                          # Research artifacts ✅
└── daimon/                            # Affect state ⚠️ (never written by runner)
```

---

## 11. What "Fully Self-Improving" Requires

Today's closed loops:
1. ⚠️ **CascadeRouter** — model selection improves with use, but all-provider routing proof is still missing
2. ✅ **Section effectiveness** — prompt allocation improves with use
3. ✅ **Anti-pattern injection** — prior failures inform next attempt

Missing loops (6 fixes, ~235 LOC total):
1. ❌ Safety Pipeline → universal enforcement
2. ❌ Knowledge Store → create dir + query at dispatch
3. ❌ Gate Thresholds → update after each verdict
4. ❌ Dreams → trigger after plan completion
5. ⚠️ Daimon → loaded for model routing; persistence and prompt/episode propagation still missing
6. ❌ Replan → revise task on repeated failure

Once these 6 are wired, every run produces data that makes the next run better across ALL dimensions (model selection, prompt content, gate strictness, knowledge retrieval, affect modulation, and failure recovery).

---

## 12. Known Bugs

| Bug | Location | Impact | Fix |
|-----|----------|--------|-----|
| `DreamCycleReport` missing fields | `crates/roko-serve/src/dreams.rs:556,613,642,711` | Blocks `cargo test --workspace` | Add `cross_episode_report: None, routing_recommendations: vec![]` |
| Bardo path hardcoding | PRD generation prompts | References `/Users/will/dev/uniswap/bardo/prd/` | Make dynamic |
| ExtensionChain always empty | `orchestrate.rs` extension call sites | Extensions never loaded | Wire extension loader/factory |

---

## Appendix: Quick Health Check Script

```bash
#!/bin/bash
# Save as: scripts/health-check.sh
set -e
cd "$(git rev-parse --show-toplevel)"

echo "=== Build ==="
cargo check -p roko-cli 2>&1 | tail -3

echo "=== State Files ==="
for f in .roko/episodes.jsonl .roko/learn/cascade-router.json .roko/learn/efficiency.jsonl .roko/state/executor.json; do
  if [ -f "$f" ]; then
    echo "  ✅ $f ($(wc -l < "$f") lines)"
  else
    echo "  ❌ $f MISSING"
  fi
done

echo "=== Learning State ==="
if [ -f .roko/learn/cascade-router.json ]; then
  echo "  Router observations: $(cat .roko/learn/cascade-router.json | python3 -c 'import json,sys; d=json.load(sys.stdin); print(len(d.get("observations",[])))' 2>/dev/null || echo 'parse error')"
fi

echo "=== Known Gaps ==="
[ -d .roko/neuro ] && echo "  ✅ Neuro dir exists" || echo "  ❌ Neuro dir MISSING"
[ -s .roko/learn/gate-thresholds.json ] && echo "  ⚠️  Gate thresholds: $(cat .roko/learn/gate-thresholds.json)" || echo "  ❌ Gate thresholds empty/missing"
[ -d .roko/learn/playbooks ] && echo "  Playbooks: $(ls .roko/learn/playbooks/ 2>/dev/null | wc -l) files" || echo "  ❌ Playbooks dir MISSING"
```
