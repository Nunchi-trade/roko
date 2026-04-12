# TUI 29-Screen Inventory

> Complete inventory of all 29 screens across 6 window regions — navigation, agent detail, plan detail, knowledge, collective intelligence, and system monitoring.

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [08-tui-main-layout.md](./08-tui-main-layout.md), [07-rosedust-design-language.md](./07-rosedust-design-language.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §4, `roko-cli/src/tui/`, `bardo-backup/prd/18-interfaces/03-tui.md`, `bardo-backup/prd/18-interfaces/01-cli.md`

---

## Abstract

The Roko TUI organizes its interface into **29 screens** distributed across **6 window regions**. Each region groups related information by domain — agent operations, plan execution, knowledge management, collective intelligence, and system health. Screens within a region are navigated using Tab/Shift+Tab, while regions are accessed using numeric shortcuts (1–6).

This progressive disclosure design ensures that the main dashboard (see [08-tui-main-layout.md](./08-tui-main-layout.md)) provides a glanceable overview, while detail screens expose full operational context on demand. Every screen renders within the same ratatui immediate-mode rendering loop at 60fps, using the ROSEDUST palette (see [07-rosedust-design-language.md](./07-rosedust-design-language.md)).

---

## Screen Map Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    ROKO TUI — 29 SCREENS                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Region 1: NAVIGATION (6)     Region 2: AGENT DETAIL (6)       │
│  ┌─ Agent List ─────────┐     ┌─ Output Stream ─────────┐      │
│  ├─ Plan List ──────────┤     ├─ Gate Results ──────────┤      │
│  ├─ Mesh Status ────────┤     ├─ Daimon State ──────────┤      │
│  ├─ Knowledge Browser ──┤     ├─ Prediction Dashboard ──┤      │
│  ├─ Episode Timeline ───┤     ├─ Tool Trace ────────────┤      │
│  └─ Settings ───────────┘     └─ Cost Breakdown ────────┘      │
│                                                                 │
│  Region 3: PLAN DETAIL (5)    Region 4: KNOWLEDGE (4)          │
│  ┌─ DAG View ───────────┐     ┌─ Neuro Explorer ────────┐      │
│  ├─ Task Detail ────────┤     ├─ Tier Progression ──────┤      │
│  ├─ Merge Queue ────────┤     ├─ Cross-Domain Map ──────┤      │
│  ├─ Timeline ───────────┤     └─ Knowledge Graph ───────┘      │
│  └─ Worktree Status ───┘                                       │
│                                                                 │
│  Region 5: COLLECTIVE (4)     Region 6: SYSTEM (4)             │
│  ┌─ C-Factor Dashboard ─┐     ┌─ Provider Health ───────┐      │
│  ├─ Agent Comparison ───┤     ├─ Resource Monitor ──────┤      │
│  ├─ Pheromone Landscape ┤     ├─ Event Log ─────────────┤      │
│  └─ Stigmergy Map ─────┘     └─ Spectre Gallery ───────┘      │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Total: 6 + 6 + 5 + 4 + 4 + 4 = 29 screens**

---

## Region 1: Navigation (6 screens)

The Navigation region occupies the left sidebar of the main layout. It provides system-wide overviews for quick orientation.

### Screen 1.1: Agent List

The primary navigation screen. Shows all agents in the current session with status indicators and behavioral state coloring.

```
┌─ AGENTS ────────────────────────────────┐
│                                          │
│  ◉ rust-implementer      Engaged    3/7 │
│    sonnet-4.6  C:+0.12  $0.34          │
│                                          │
│  ◉ reviewer-01           Focused    2/3 │
│    sonnet-4.6  C:+0.08  $0.12          │
│                                          │
│  ○ researcher-01         Coasting   ─   │
│    opus-4.6    C:+0.04  $0.56          │
│                                          │
│  ◌ architect-01          Resting    ─   │
│    sonnet-4.6  C:+0.02  $0.08          │
│                                          │
│  ─────────────────────────────────────  │
│  4 agents │ 2 active │ 1 idle │ 1 rest │
└──────────────────────────────────────────┘
```

**Data displayed per agent:**
- Status indicator: `◉` active, `○` idle, `◌` resting (Dreams active)
- Agent name (from template + instance number)
- Behavioral state label, colored by ROSEDUST state mapping:
  - Rose (`#D4778C`) — Engaged
  - Amber/Crimson (`#D4A857` / `#C45C50`) — Struggling
  - Sapphire (`#6B8FBD`) — Coasting
  - Violet (`#A08CC4`) — Exploring
  - Jade (`#5DB8A3`) — Focused
  - Dim Rose (`#A05C6E`) — Resting
- Current task progress (e.g., "3/7")
- Model name
- C-Factor contribution (signed delta)
- Session cost

**Interactions:**
- `↑`/`↓` — select agent
- `Enter` — navigate to Agent Detail (Region 2)
- `d` — jump to Daimon state for selected agent
- `s` — jump to Spectre viewport for selected agent

**Source**: `roko-cli/src/tui/views/agents.rs`, `roko-cli/src/tui/widgets/agent_grid.rs`

### Screen 1.2: Plan List

Shows all discovered plans with execution status.

```
┌─ PLANS ─────────────────────────────────┐
│                                          │
│  ▸ plan-01  Wire TUI layout       3/7  │
│    ██████░░░░  42%  2m34s  $1.23       │
│                                          │
│  ✓ plan-02  Add gate pipeline      7/7  │
│    ██████████  100%  8m12s  $3.45      │
│                                          │
│  ✗ plan-03  Refactor compose       4/6  │
│    ██████░░░░  67%  5m01s  $2.11       │
│    └ FAIL: test gate (3 failures)       │
│                                          │
│  ─────────────────────────────────────  │
│  3 plans │ 1 running │ 1 done │ 1 fail │
└──────────────────────────────────────────┘
```

**Status indicators:**
- `▸` — currently running (animated, pulses in rose)
- `✓` — completed successfully (jade)
- `✗` — failed (danger red)
- `◦` — pending (muted)
- `⏸` — paused

**Per-plan data:**
- Plan ID and title
- Task progress (completed/total)
- Progress bar with percentage
- Elapsed time
- Total cost
- Failure reason (if failed)

**Interactions:**
- `Enter` — navigate to Plan Detail (Region 3)
- `r` — retry failed plan
- `p` — pause/resume running plan

**Source**: `roko-cli/src/tui/views/plans.rs`, `roko-cli/src/tui/widgets/plan_tree.rs`

### Screen 1.3: Mesh Status

Shows Agent Mesh connectivity — peer discovery, pheromone channels, and synchronization state.

```
┌─ MESH ──────────────────────────────────┐
│                                          │
│  Peers: 4/4 connected                   │
│                                          │
│  rust-impl ──── reviewer                │
│      │    ╲        │                    │
│      │     ╲       │                    │
│  researcher ── architect                │
│                                          │
│  Pheromones:                            │
│   ≋ Wisdom     rust-impl → reviewer    │
│   ≋ Warning    researcher → ALL        │
│   ≋ Discovery  architect → rust-impl   │
│                                          │
│  Sync: 142 engrams │ lag: 0.2s         │
└──────────────────────────────────────────┘
```

**Displays:**
- Peer count and connectivity graph (ASCII topology)
- Active pheromone channels with type, source, and target
- Synchronization metrics (engram count, replication lag)

**Pheromone types** (from Stigmergy system):
- `Wisdom` — successful strategy discovered
- `Warning` — failure or hazard detected
- `Discovery` — novel insight found
- `Recruitment` — requesting collaboration
- `Completion` — task finished, resources available

**Interactions:**
- `Enter` — navigate to Collective Region (Region 5) for detailed mesh view
- `↑`/`↓` — select pheromone for detail

### Screen 1.4: Knowledge Browser

Overview of the Neuro knowledge store — total entries, tier distribution, and recent additions.

```
┌─ KNOWLEDGE ─────────────────────────────┐
│                                          │
│  Neuro Store: 284 entries               │
│                                          │
│  Persistent ████░░░░░░░░░░  45  (16%)  │
│  Working    ████████░░░░░░  134 (47%)  │
│  Transient  ██████░░░░░░░░  105 (37%)  │
│                                          │
│  Recent:                                │
│  + Insight: auth module patterns   3m   │
│  + Heuristic: test isolation       8m   │
│  + Warning: alloy dep conflict    12m   │
│  ↑ Promoted: error handling (W→P) 15m   │
│                                          │
│  Types: 89 Insight │ 67 Heuristic │    │
│         45 Warning │ 38 Causal │       │
│         29 Strategy │ 16 AntiKnowledge │
└──────────────────────────────────────────┘
```

**Displays:**
- Total entry count across all tiers
- Tier distribution bar chart (Persistent, Consolidated, Working, Transient)
- Recent knowledge events (additions, promotions, decays)
- Breakdown by knowledge type (Insight, Heuristic, Warning, CausalLink, StrategyFragment, AntiKnowledge)

**Interactions:**
- `Enter` — navigate to Knowledge Region (Region 4)
- `f` — filter by type
- `s` — search knowledge entries

### Screen 1.5: Episode Timeline

Chronological view of recent episodes (agent execution sessions) with outcomes.

```
┌─ EPISODES ──────────────────────────────┐
│                                          │
│  14:23  rust-impl  task-03  ✓  12 turns │
│         sonnet-4.6  $0.34  2m01s       │
│                                          │
│  14:18  reviewer   task-02  ✓  8 turns  │
│         sonnet-4.6  $0.12  1m23s       │
│                                          │
│  14:12  rust-impl  task-02  ✗→✓ 2 iter │
│         sonnet-4.6  $0.67  4m45s       │
│         retry: test gate failure        │
│                                          │
│  14:05  researcher  research  ✓ 15 trn  │
│         opus-4.6   $1.23  6m30s        │
│                                          │
│  Total: 23 episodes │ 87% first-try    │
└──────────────────────────────────────────┘
```

**Per-episode data:**
- Timestamp
- Agent name and task ID
- Outcome (✓ success, ✗ failure, ✗→✓ retry success)
- Turn count and iteration count
- Model used, cost, duration
- Failure/retry reason (if applicable)

**Source**: `.roko/episodes.jsonl` via `EpisodeLogger`

### Screen 1.6: Settings

Configuration overview and runtime settings.

```
┌─ SETTINGS ──────────────────────────────┐
│                                          │
│  Project: roko                          │
│  Config:  roko.toml (workspace root)    │
│                                          │
│  Models:                                │
│   T0: (16 probes, no LLM)              │
│   T1: claude-sonnet-4-6                 │
│   T2: claude-opus-4-6                   │
│                                          │
│  Gates: compile ✓ test ✓ clippy ✓       │
│         fmt ✓ diff ✓ review ○           │
│                                          │
│  Parallelism: 4 agents max              │
│  Budget: $50.00/day (used: $12.34)      │
│  Data dir: .roko/                       │
│                                          │
│  MCP Servers:                           │
│   ✓ github (stdio)                      │
│   ✓ filesystem (stdio)                  │
│   ○ slack (not configured)              │
└──────────────────────────────────────────┘
```

**Displays:**
- Project name and config file location
- Model routing configuration (T0/T1/T2 tiers)
- Active gate pipeline
- Parallelism and budget limits
- MCP server status

---

## Region 2: Agent Detail (6 screens)

The Agent Detail region shows deep information about the currently selected agent. Accessed by pressing Enter on an agent in the Agent List (Screen 1.1).

### Screen 2.1: Output Stream

Live output from the selected agent — the primary monitoring view during execution.

```
┌─ rust-implementer ─── Output Stream ────┐
│                                          │
│  [14:23:01] Analyzing auth module...     │
│  [14:23:03] Found 3 existing patterns:   │
│    - JWT validation in middleware        │
│    - Session token in cookie             │
│    - API key in header                   │
│  [14:23:05] ◆ tool: read_file            │
│    path: src/auth/middleware.rs           │
│    result: 142 lines read                │
│  [14:23:08] ◆ tool: edit_file            │
│    path: src/auth/middleware.rs           │
│    lines: 45-67 (22 lines modified)      │
│  [14:23:12] Implementing token refresh   │
│    logic based on existing JWT pattern.  │
│  [14:23:15] ◆ tool: write_file           │
│    path: src/auth/refresh.rs             │
│    result: 89 lines written              │
│                                          │
│  Turn 5/∞ │ 2,341 tokens │ $0.02       │
└──────────────────────────────────────────┘
```

**Features:**
- Timestamped output with automatic scrolling
- Tool call traces with collapsible detail (tool name, arguments, result)
- Token count and cost per turn in the status line
- Color-coded: text output in fg, tool calls in lavender, errors in danger red
- Auto-scroll to bottom, with scroll lock on manual scroll up

**Interactions:**
- `↑`/`↓` — scroll output
- `Space` — toggle auto-scroll
- `c` — copy selected text to clipboard
- `f` — search/filter output

### Screen 2.2: Gate Results

Gate pipeline status and history for the selected agent's current task.

```
┌─ rust-implementer ─── Gate Results ─────┐
│                                          │
│  Pipeline: task-03 (iteration 1)        │
│                                          │
│  Rung 1 (Format):                       │
│    ✓ fmt_check      0.3s   pass         │
│                                          │
│  Rung 2 (Compile):                      │
│    ✓ cargo_check    2.1s   pass         │
│                                          │
│  Rung 3 (Lint):                         │
│    ✓ clippy         3.4s   pass         │
│    ✓ deny_warnings  0.1s   pass         │
│                                          │
│  Rung 4 (Test):                         │
│    ✓ cargo_test     8.2s   pass (42/42) │
│    ○ integration    ─      pending      │
│                                          │
│  Rung 5 (Diff):                         │
│    ○ diff_review    ─      pending      │
│                                          │
│  Rung 6 (Review):                       │
│    ○ ai_review      ─      pending      │
│                                          │
│  Ratchet: ████████░░  80%  (4/5 rungs)  │
│  History: ✓ ✓ ✗ ✓ ✓ ✓  (6 runs)        │
│  Adaptive threshold: 0.85 (EMA)         │
└──────────────────────────────────────────┘
```

**Displays:**
- Current pipeline run with per-rung breakdown
- Individual gate status within each rung (pass/fail/pending/skip)
- Gate duration
- Test counts (passed/total)
- Ratchet progress bar (highest rung reached)
- Historical pass/fail pattern
- Adaptive threshold (from `gate-thresholds.json`)

**Source**: `roko-gate` crate, 6-rung pipeline, 11+ built-in gates

### Screen 2.3: Daimon State

Behavioral state and PAD (Pleasure-Arousal-Dominance) vector display for the selected agent.

```
┌─ rust-implementer ─── Daimon State ─────┐
│                                          │
│  State: ENGAGED                         │
│  ████████████████████████░░░░  rose     │
│                                          │
│  PAD Vector:                            │
│  Pleasure   ▓▓▓▓▓▓▓░░░  0.70          │
│  Arousal    ▓▓▓▓▓░░░░░  0.50          │
│  Dominance  ▓▓▓▓▓▓▓▓░░  0.80          │
│                                          │
│  State History (last 10 ticks):         │
│  E E E F F E E E S E                   │
│  ─────────────────────→ time            │
│                                          │
│  Behavioral Effects:                    │
│  • Tier routing: T1 preferred (moderate) │
│  • Context bid: +0.12 bonus             │
│  • Risk tolerance: 0.65 (balanced)      │
│  • Exploration rate: 0.15               │
│                                          │
│  Transitions (session):                 │
│  Engaged → Focused: 3                   │
│  Focused → Engaged: 3                   │
│  Engaged → Struggling: 1               │
│  Struggling → Engaged: 1               │
│                                          │
│  Time in state: 2m14s (45% of session)  │
└──────────────────────────────────────────┘
```

**Displays:**
- Current behavioral state with ROSEDUST color bar
- PAD vector components as horizontal bars
- State history timeline (letter abbreviations: E/S/C/X/F/R)
- Behavioral effects on agent operation:
  - Tier routing preference (T0/T1/T2 distribution)
  - Context bidding modifier (from VCG auction)
  - Risk tolerance scalar
  - Exploration rate (from LinUCB)
- State transition counts for the session
- Time spent in current state

**Source**: `roko-daimon` crate, PAD vector model

### Screen 2.4: Prediction Dashboard

Active predictions and calibration tracking for the selected agent.

```
┌─ rust-implementer ─── Predictions ──────┐
│                                          │
│  Active Predictions:                    │
│                                          │
│  Build time:  34s  (pred: 40s)  ✓  -15% │
│  Test pass:   94%  (pred: 90%)  ✓  +4%  │
│  Gate pass:   yes  (pred: 85%)  ✓       │
│  Cost/task:   $0.34 (pred: $0.50) ✓ -32%│
│  Turns:       12   (pred: 15)   ✓  -20% │
│                                          │
│  Calibration Score: 0.82                │
│  ████████░░  (well-calibrated)          │
│                                          │
│  Prediction History:                    │
│  ✓ ✓ ✗ ✓ ✓ ✓ ✓ ✗ ✓ ✓  (80% accurate)  │
│                                          │
│  Brier Score: 0.12 (lower = better)     │
│  Correction latency: ~50ns              │
│                                          │
│  Falsified:                             │
│  ✗ task-02: predicted 8 turns, took 22  │
│    → Updated: model complexity estimate │
└──────────────────────────────────────────┘
```

**Displays:**
- Active predictions with actual vs. predicted values
- Per-prediction accuracy delta
- Calibration score (0–1, from `CalibrationTracker`)
- Historical accuracy pattern
- Brier score for probabilistic calibration
- Correction latency (~50ns per the Predictive Foraging innovation)
- Falsified predictions with learning outcome

**Source**: `roko-learn` crate, Predictive Foraging (Innovation #6)

### Screen 2.5: Tool Trace

Detailed tool call history for the selected agent.

```
┌─ rust-implementer ─── Tool Trace ───────┐
│                                          │
│  Turn 5 (14:23:15):                     │
│  ┌ read_file                            │
│  │ path: src/auth/middleware.rs          │
│  │ result: 142 lines (3.2KB)            │
│  │ duration: 12ms                       │
│  └ ✓                                    │
│                                          │
│  ┌ edit_file                            │
│  │ path: src/auth/middleware.rs          │
│  │ old_string: "fn validate_token..."   │
│  │ new_string: "fn validate_token..."   │
│  │ lines_changed: 22                    │
│  │ duration: 8ms                        │
│  └ ✓                                    │
│                                          │
│  ┌ write_file                           │
│  │ path: src/auth/refresh.rs            │
│  │ lines: 89                            │
│  │ duration: 5ms                        │
│  └ ✓                                    │
│                                          │
│  Turn 4 (14:23:08):                     │
│  ┌ bash                                 │
│  │ cmd: cargo test --lib auth           │
│  │ exit_code: 0                         │
│  │ duration: 4.2s                       │
│  └ ✓                                    │
│                                          │
│  Summary: 23 calls │ 21 ✓ │ 2 ✗ │ 4.8s │
└──────────────────────────────────────────┘
```

**Displays:**
- Tool calls grouped by turn, newest first
- Per-call: tool name, arguments (truncated), result, duration, status
- Call tree indentation for nested operations
- Summary: total calls, pass/fail counts, total tool time

**Interactions:**
- `Enter` — expand tool call to show full arguments and result
- `e` — show only errors
- `↑`/`↓` — navigate calls

### Screen 2.6: Cost Breakdown

Token usage, model costs, and efficiency metrics for the selected agent.

```
┌─ rust-implementer ─── Cost Breakdown ───┐
│                                          │
│  Session Total: $0.34                   │
│                                          │
│  Per Model:                             │
│  sonnet-4.6   12 calls  $0.28  (82%)   │
│  opus-4.6     2 calls   $0.06  (18%)   │
│                                          │
│  Token Usage:                           │
│  Input:   45,230 tokens  ($0.14)        │
│  Output:  12,450 tokens  ($0.19)        │
│  Cache:   23,100 tokens  (saved $0.07)  │
│                                          │
│  Efficiency:                            │
│  $/task:  $0.11  (avg)                  │
│  $/turn:  $0.03  (avg)                  │
│  T0 suppression: 78% (saved ~$0.42)    │
│                                          │
│  Tier Distribution:                     │
│  T0 ████████████████  78%  (16 probes)  │
│  T1 ████░░░░░░░░░░░░  17%              │
│  T2 █░░░░░░░░░░░░░░░   5%              │
│                                          │
│  Budget: $12.34 / $50.00 today          │
│  ████░░░░░░  25%                        │
└──────────────────────────────────────────┘
```

**Displays:**
- Total session cost
- Per-model breakdown (calls, cost, percentage)
- Token usage (input, output, cache hits and savings)
- Efficiency metrics (cost per task, cost per turn)
- T0 probe suppression rate (from 16 T0 Probes innovation)
- Tier distribution bar chart
- Daily budget usage

**Source**: `.roko/learn/efficiency.jsonl`, CascadeRouter metrics

---

## Region 3: Plan Detail (5 screens)

The Plan Detail region shows deep information about the currently selected plan. Accessed by pressing Enter on a plan in the Plan List (Screen 1.2).

### Screen 3.1: DAG View

Visual representation of the plan's task dependency graph.

```
┌─ plan-01 ─── DAG View ─────────────────┐
│                                          │
│  Wire TUI Layout (7 tasks)              │
│                                          │
│  ┌─────┐     ┌─────┐     ┌─────┐       │
│  │ T-1 │────▸│ T-3 │────▸│ T-5 │       │
│  │  ✓  │     │  ▸  │     │  ◦  │       │
│  └─────┘     └─────┘     └──┬──┘       │
│                   │          │          │
│  ┌─────┐         │     ┌────▼──┐       │
│  │ T-2 │────────▸│     │ T-7   │       │
│  │  ✓  │         │     │  ◦    │       │
│  └─────┘     ┌───▼─┐   └───────┘       │
│              │ T-4  │                   │
│  ┌─────┐    │  ▸   │                   │
│  │ T-6 │    └──────┘                   │
│  │  ◦  │                               │
│  └─────┘                               │
│                                          │
│  Critical path: T-1 → T-3 → T-5 → T-7 │
│  Parallelism: 2 (T-3 ∥ T-4)           │
│  Est. remaining: 12m                    │
└──────────────────────────────────────────┘
```

**Displays:**
- ASCII DAG of task dependencies with directional arrows
- Per-task status: ✓ done, ▸ running, ◦ pending, ✗ failed
- Critical path highlighted in rose
- Current parallelism level
- Estimated remaining time

**Source**: `roko-orchestrator` plan DAG executor

### Screen 3.2: Task Detail

Detailed view of a specific task within the plan.

```
┌─ plan-01 / task-03 ─── Task Detail ─────┐
│                                          │
│  Title: Implement layout computation     │
│  Agent: rust-implementer                 │
│  Status: Running (iteration 1)          │
│                                          │
│  Dependencies:                          │
│    ✓ task-01: Define widget trait        │
│    ✓ task-02: Create ROSEDUST theme     │
│                                          │
│  Dependents:                            │
│    ◦ task-05: Wire Spectre viewport     │
│    ◦ task-07: Integration test          │
│                                          │
│  Files Modified:                        │
│    M src/tui/layout.rs  (+45, -12)      │
│    A src/tui/regions.rs (+89)           │
│    M src/tui/mod.rs     (+3, -0)        │
│                                          │
│  Gates Passed: 3/5 rungs                │
│  Turns: 8 │ Tokens: 12,340 │ $0.11     │
│  Started: 14:18 │ Elapsed: 5m12s       │
│                                          │
│  Prompt (first 3 lines):               │
│  "Implement the layout computation for  │
│   the TUI main dashboard, using ratatui │
│   Layout::default().direction(...)..."  │
└──────────────────────────────────────────┘
```

**Source**: `roko-cli/src/tui/modals/task_detail.rs`

### Screen 3.3: Merge Queue

Status of the plan's merge queue — worktree branches awaiting merge.

```
┌─ plan-01 ─── Merge Queue ──────────────┐
│                                          │
│  Queue: 3 branches                      │
│                                          │
│  1. task-01/define-widget-trait          │
│     ✓ gates passed │ ✓ merged           │
│     +89 -12 │ 3 files                   │
│                                          │
│  2. task-02/create-rosedust-theme       │
│     ✓ gates passed │ ✓ merged           │
│     +234 -0 │ 2 files                   │
│                                          │
│  3. task-03/implement-layout            │
│     ▸ gates running │ ○ pending merge   │
│     +134 -12 │ 3 files                  │
│                                          │
│  Merge strategy: rebase                 │
│  Conflicts: none detected               │
│  Base: main (abc1234)                   │
└──────────────────────────────────────────┘
```

**Displays:**
- Queued branches with gate and merge status
- Diff stats per branch
- Merge strategy (rebase/merge/squash)
- Conflict detection status

**Source**: `roko-orchestrator` merge queue system

### Screen 3.4: Timeline

Gantt-style timeline of plan execution.

```
┌─ plan-01 ─── Timeline ─────────────────┐
│                                          │
│  14:00    14:10    14:20    14:30       │
│  │         │         │         │        │
│  T-1 ████░░░░░░░░░░░░░░░░░░░░░░       │
│  T-2 ░░░████░░░░░░░░░░░░░░░░░░░       │
│  T-3 ░░░░░░░████████▸░░░░░░░░░░       │
│  T-4 ░░░░░░░░░░████▸░░░░░░░░░░░       │
│  T-5 ░░░░░░░░░░░░░░░░░░◦░░░░░░░       │
│  T-6 ░░░░░░░░░░░░░░░◦░░░░░░░░░░       │
│  T-7 ░░░░░░░░░░░░░░░░░░░░░░◦░░░       │
│                                          │
│  ████ = done  ▸ = active  ◦ = pending  │
│                                          │
│  Elapsed: 23m │ Est. total: 35m        │
│  Parallelism: avg 1.8 │ max 2         │
└──────────────────────────────────────────┘
```

### Screen 3.5: Worktree Status

Git worktree overview for the plan's parallel execution.

```
┌─ plan-01 ─── Worktree Status ──────────┐
│                                          │
│  Base: main (abc1234)                   │
│                                          │
│  Worktrees:                             │
│  ✓ .roko/worktrees/task-01/             │
│    branch: plan-01/task-01              │
│    status: merged, cleanup pending      │
│                                          │
│  ✓ .roko/worktrees/task-02/             │
│    branch: plan-01/task-02              │
│    status: merged, cleanup pending      │
│                                          │
│  ▸ .roko/worktrees/task-03/             │
│    branch: plan-01/task-03              │
│    status: active, 3 files modified     │
│                                          │
│  ▸ .roko/worktrees/task-04/             │
│    branch: plan-01/task-04              │
│    status: active, 1 file modified      │
│                                          │
│  Disk usage: 45MB (4 worktrees)         │
└──────────────────────────────────────────┘
```

---

## Region 4: Knowledge (4 screens)

The Knowledge region provides tools for exploring the Neuro knowledge store — the persistent memory that accumulates across agent sessions.

### Screen 4.1: Neuro Explorer

Searchable, filterable view of all knowledge entries.

```
┌─ NEURO EXPLORER ────────────────────────┐
│                                          │
│  Search: [auth pattern_____________]    │
│  Filter: Insight ✓ Heuristic ✓ All ○   │
│                                          │
│  Results: 12 entries                    │
│                                          │
│  ◉ Insight: JWT refresh pattern          │
│    Tier: Persistent │ Score: 0.92       │
│    "When implementing token refresh,     │
│     use a sliding window with 80%..."    │
│    Created: 2h ago │ Decays: never      │
│                                          │
│  ◉ Heuristic: Auth middleware ordering   │
│    Tier: Working │ Score: 0.78          │
│    "Place auth middleware before rate     │
│     limiting to avoid unnecessary..."    │
│    Created: 4h ago │ Decays: 12h        │
│                                          │
│  ○ Warning: Cookie SameSite default      │
│    Tier: Transient │ Score: 0.45        │
│    "Browser default for SameSite has     │
│     changed; explicit Lax required..."   │
│    Created: 1d ago │ Decays: 2h         │
│                                          │
│  [↓ 9 more entries]                     │
└──────────────────────────────────────────┘
```

**Features:**
- Full-text search with keyword highlighting
- Filter by knowledge type and tier
- Entry detail: type, tier, score, content preview, creation time, decay schedule
- Tier indicators: ◉ Persistent, ◎ Consolidated, ○ Working, ◌ Transient

**Source**: `roko-neuro` knowledge store

### Screen 4.2: Tier Progression

Visualization of knowledge tier promotion and decay over time.

```
┌─ TIER PROGRESSION ──────────────────────┐
│                                          │
│  Knowledge Flow (last 24h):             │
│                                          │
│         Transient → Working → Persistent│
│                                          │
│  Ingest:  +45 ──▸                       │
│  Promote:        +12 ──▸  +3            │
│  Decay:   -23          -5               │
│  Prune:   -8           -2               │
│                                          │
│  Tier Sizes Over Time:                  │
│  200│  ·····                            │
│     │ ·     ····                         │
│  150│·          ·····  Working           │
│     │                ····               │
│  100│                    ····           │
│     │────────────────────── Persistent  │
│   50│  ·                                │
│     │·  ···                              │
│    0│      ····  Transient              │
│     └──────────────────────             │
│      -24h  -18h  -12h  -6h  now        │
│                                          │
│  Promotion criteria:                    │
│  Transient→Working: score ≥ 0.30        │
│  Working→Consolidated: score ≥ 0.50     │
│  Consolidated→Persistent: score ≥ 0.70  │
│  + confidence threshold per Dreams cycle │
└──────────────────────────────────────────┘
```

### Screen 4.3: Cross-Domain Map

HDC (Hyperdimensional Computing) similarity map showing knowledge connections across domains.

```
┌─ CROSS-DOMAIN MAP ──────────────────────┐
│                                          │
│  Similarity Threshold: 0.526            │
│                                          │
│       auth ────── session               │
│        │  ╲         │                   │
│        │   ╲        │                   │
│     crypto   ╲   middleware             │
│        │      ╲     │                   │
│        │    error-handling              │
│        │          │                     │
│     testing ──── validation             │
│                                          │
│  Resonance Events (last 24h):           │
│                                          │
│  ≋ auth × error-handling  sim: 0.634   │
│    "Token expiry patterns structurally  │
│     analogous to circuit breaker reset" │
│                                          │
│  ≋ crypto × validation  sim: 0.571     │
│    "Hash verification parallels input   │
│     sanitization pipeline structure"    │
│                                          │
│  Cross-domain transfers: 7              │
│  Novel insights generated: 3           │
└──────────────────────────────────────────┘
```

**Source**: Cross-Domain Insight Resonance (Innovation #12), HDC encoding with structural analogy threshold 0.526

### Screen 4.4: Knowledge Graph

Full knowledge entry graph with lineage (parent Engram) connections.

```
┌─ KNOWLEDGE GRAPH ───────────────────────┐
│                                          │
│  Entries: 284 │ Edges: 412              │
│                                          │
│  [Cluster view — entries as nodes,      │
│   lineage as edges, colored by type,    │
│   sized by score, positioned by         │
│   HDC similarity using force-directed   │
│   layout in braille characters]         │
│                                          │
│  ⠿⠿⠿⠿        ⠿⠿⠿                      │
│  ⠿⠿⠿⠿⠿      ⠿⠿⠿⠿                     │
│   ⠿⠿⠿⠿  ────  ⠿⠿⠿                     │
│    ⠿⠿⠿        ⠿⠿                       │
│      │                                  │
│    ⠿⠿⠿⠿                                │
│    ⠿⠿⠿⠿⠿                               │
│     ⠿⠿⠿                                │
│                                          │
│  Legend:                                │
│  ◉ Insight  ◎ Heuristic  △ Warning     │
│  ◇ Causal   ▢ Strategy   ✕ Anti       │
│                                          │
│  Focus: [selected cluster detail]       │
└──────────────────────────────────────────┘
```

**Features:**
- Force-directed graph layout rendered in braille characters (`roko-cli/src/tui/widgets/braille.rs`)
- Nodes colored by knowledge type
- Node size proportional to score
- Edges represent lineage (parent Engram → child Engram)
- Clustering by HDC similarity
- Click/select a cluster to zoom into detail

---

## Region 5: Collective (4 screens)

The Collective region shows multi-agent dynamics — how agents work together and whether the collective outperforms the sum of individuals.

### Screen 5.1: C-Factor Dashboard

The primary collective intelligence monitoring view.

```
┌─ C-FACTOR DASHBOARD ───────────────────┐
│                                          │
│        C-Factor: 1.23                   │
│  ◀ 0.5    1.0 ▲ 1.5    2.0 ▶          │
│  ──────────────█──────────              │
│           superlinear ↑                 │
│                                          │
│  C-Score Components:                    │
│  gate_pass       ██████████  0.94  ×0.3 │
│  cost_efficiency ████████░░  0.82  ×0.2 │
│  speed           ███████░░░  0.76  ×0.15│
│  first_try_rate  █████████░  0.88  ×0.25│
│  knowledge_growth██████░░░░  0.65  ×0.1 │
│                                          │
│  C-Score: 0.836                         │
│                                          │
│  Diagnostics:                           │
│  Turn-taking equality:  0.91  (good)    │
│  Knowledge flow rate:   0.73  (good)    │
│  Cross-domain transfer: 0.45  (low)     │
│  Emergent coordination: 0.62  (fair)    │
│                                          │
│  Agent Contributions:                   │
│  rust-impl  ████████  0.34  (highest)   │
│  reviewer   ██████░░  0.28              │
│  researcher █████░░░  0.22              │
│  architect  ████░░░░  0.18              │
│                                          │
│  Trend (last 1h): 1.18 → 1.23 (+0.05)  │
└──────────────────────────────────────────┘
```

**Displays:**
- C-Factor ratio on a scale gauge (< 1.0 = sublinear, > 1.0 = superlinear)
- C-Score composite breakdown with component weights
- Four diagnostic signals (Woolley et al., Science 330, 2010)
- Per-agent contribution ranking
- Trend line

**Source**: C-Factor metric system, `/ws/cfactor` WebSocket endpoint

### Screen 5.2: Agent Comparison

Side-by-side comparison of agent performance metrics.

```
┌─ AGENT COMPARISON ──────────────────────┐
│                                          │
│  Metric          rust-impl  reviewer    │
│  ───────────────────────────────────    │
│  Tasks completed  3          2          │
│  Gate pass rate   100%       100%       │
│  First-try rate   67%        100%       │
│  Avg turns/task   10.3       6.5        │
│  Avg cost/task    $0.11      $0.06      │
│  T0 suppression   78%        82%        │
│  Knowledge created 12         8         │
│  C-Contribution   0.34       0.28       │
│                                          │
│  State Distribution:                    │
│  rust-impl:  E:45% F:30% S:10% X:15%  │
│  reviewer:   F:60% E:25% C:15%        │
│                                          │
│  Model Usage:                           │
│  rust-impl:  sonnet:82% opus:18%       │
│  reviewer:   sonnet:95% opus:5%        │
└──────────────────────────────────────────┘
```

### Screen 5.3: Pheromone Landscape

Real-time visualization of stigmergic communication between agents.

```
┌─ PHEROMONE LANDSCAPE ───────────────────┐
│                                          │
│  Active Pheromones: 7                   │
│                                          │
│  ≋≋≋ Wisdom (intensity: 0.8)           │
│  rust-impl → ALL                        │
│  "JWT refresh pattern is reusable"      │
│  Decay: 4h remaining                    │
│                                          │
│  ≋≋ Warning (intensity: 0.6)            │
│  researcher → rust-impl, reviewer       │
│  "alloy 0.4 breaks with tokio 1.38"    │
│  Decay: 2h remaining                    │
│                                          │
│  ≋ Discovery (intensity: 0.3)           │
│  architect → ALL                        │
│  "Layout computation can be cached"     │
│  Decay: 1h remaining                    │
│                                          │
│  Intensity Map:                         │
│  ░░▒▒▓▓██▓▓▒▒░░  (spatial heat map)   │
│                                          │
│  Pheromone history (24h):               │
│  Emitted: 23 │ Decayed: 16 │ Active: 7 │
└──────────────────────────────────────────┘
```

**Source**: Agent Mesh pheromone system, stigmergy-based indirect communication

### Screen 5.4: Stigmergy Map

Visual map of indirect coordination patterns — how agents influence each other through shared environment modifications rather than direct messaging.

```
┌─ STIGMERGY MAP ─────────────────────────┐
│                                          │
│  Coordination Events (last 1h):         │
│                                          │
│  rust-impl modified auth/middleware.rs   │
│    └ reviewer picked up change (3m)     │
│    └ reviewed and approved (5m)         │
│                                          │
│  researcher found alloy issue            │
│    └ Warning pheromone emitted          │
│    └ rust-impl adjusted approach (2m)   │
│    └ architect updated plan (4m)        │
│                                          │
│  Environment Traces:                    │
│  auth/     ████████  8 modifications    │
│  tui/      ██████░░  6 modifications    │
│  tests/    █████░░░  5 modifications    │
│  config/   ██░░░░░░  2 modifications    │
│                                          │
│  Coordination Patterns:                 │
│  Sequential handoff:  4 instances       │
│  Parallel discovery:  2 instances       │
│  Cascade correction:  1 instance        │
│                                          │
│  Emergent coordination score: 0.62      │
└──────────────────────────────────────────┘
```

---

## Region 6: System (4 screens)

The System region provides infrastructure monitoring — provider health, resource usage, event logs, and the Spectre gallery.

### Screen 6.1: Provider Health

LLM provider status and routing metrics.

```
┌─ PROVIDER HEALTH ───────────────────────┐
│                                          │
│  Active Providers:                      │
│                                          │
│  ✓ Anthropic (claude-sonnet-4-6)        │
│    Latency: 1.2s (p50) │ 2.8s (p99)   │
│    Rate limit: 45/60 RPM remaining     │
│    Error rate: 0.2% (last 100 calls)   │
│    Status: healthy                      │
│                                          │
│  ✓ Anthropic (claude-opus-4-6)          │
│    Latency: 3.4s (p50) │ 8.1s (p99)   │
│    Rate limit: 18/20 RPM remaining     │
│    Error rate: 0.0% (last 20 calls)    │
│    Status: healthy                      │
│                                          │
│  ○ OpenRouter (meta-llama-4-maverick)   │
│    Status: configured, not active       │
│                                          │
│  Routing:                               │
│  CascadeRouter state: .roko/learn/      │
│    cascade-router.json                  │
│  LinUCB arms: 4 │ Pareto frontier: 2   │
│  Anomaly detector: no anomalies        │
│                                          │
│  Circuit Breaker:                       │
│  All providers: CLOSED (healthy)        │
└──────────────────────────────────────────┘
```

**Source**: `roko-agent` provider backends, `roko-conductor` circuit breaker, CascadeRouter

### Screen 6.2: Resource Monitor

System resource usage — CPU, memory, disk, and network.

```
┌─ RESOURCE MONITOR ──────────────────────┐
│                                          │
│  CPU:    ████░░░░░░  38%               │
│  Memory: ██████░░░░  62%  (1.2GB)      │
│  Disk:   ████████░░  78%  (.roko: 45MB)│
│                                          │
│  Processes:                             │
│  roko-cli         12MB   main           │
│  roko-serve       34MB   HTTP server    │
│  agent-01         8MB    worktree       │
│  agent-02         8MB    worktree       │
│  anvil            120MB  EVM simulator  │
│                                          │
│  Network:                               │
│  API calls (1h):  145                   │
│  WS connections:  3                     │
│  SSE streams:     1                     │
│  Bandwidth:       2.3MB sent/recv       │
│                                          │
│  Storage:                               │
│  signals.jsonl:   12MB  (23,456 entries)│
│  episodes.jsonl:  2MB   (456 entries)   │
│  learn/:          8MB                   │
│  worktrees/:      23MB  (4 active)     │
└──────────────────────────────────────────┘
```

### Screen 6.3: Event Log

Real-time event stream from the internal event bus.

```
┌─ EVENT LOG ─────────────────────────────┐
│                                          │
│  Filter: [all________________] Level: ≥ │
│                                          │
│  14:23:15 INFO  agent.output             │
│    rust-impl: "Implementing refresh..."  │
│                                          │
│  14:23:12 INFO  gate.result              │
│    plan-01/task-03: clippy PASS (3.4s)  │
│                                          │
│  14:23:10 INFO  agent.tool               │
│    rust-impl: edit_file (middleware.rs)  │
│                                          │
│  14:23:08 WARN  conductor.threshold      │
│    Cost approaching daily budget (25%)   │
│                                          │
│  14:23:05 INFO  cfactor.update           │
│    C-Factor: 1.21 → 1.23 (+0.02)       │
│                                          │
│  14:23:01 INFO  agent.spawn              │
│    researcher-01 spawned (opus-4.6)     │
│                                          │
│  14:22:58 INFO  mesh.pheromone           │
│    Wisdom emitted by rust-impl          │
│                                          │
│  Events: 1,234 │ Shown: 7 │ Rate: 12/s │
└──────────────────────────────────────────┘
```

**Features:**
- Filter by event type, source, level
- Automatic scrolling with scroll lock
- Click to expand event detail
- Color-coded by level: INFO (fg), WARN (gold), ERROR (danger)

**Source**: `roko-runtime` event bus, `roko-serve` event bus

### Screen 6.4: Spectre Gallery

Gallery view showing all agent Spectre creatures side by side — a visual summary of the entire collective's cognitive state.

```
┌─ SPECTRE GALLERY ───────────────────────┐
│                                          │
│  ┌──────────┐  ┌──────────┐             │
│  │  ╭─╮     │  │   ╭╮    │             │
│  │ ╭╯ ╰╮    │  │  ╭╯╰╮   │             │
│  │ │◉ ◉│    │  │  │◉◉│   │             │
│  │ ╰───╯    │  │  ╰──╯   │             │
│  │ 0.7Hz    │  │  1.2Hz   │             │
│  │rust-impl │  │ reviewer │             │
│  │ Engaged  │  │ Focused  │             │
│  └──────────┘  └──────────┘             │
│                                          │
│  ┌──────────┐  ┌──────────┐             │
│  │    ╭╮    │  │  ╭───╮   │             │
│  │ ≋╭╯╰╮≋  │  │  │   │   │             │
│  │  │◉ ◉│   │  │  │○ ○│   │             │
│  │  ╰───╯   │  │  ╰───╯   │             │
│  │  0.9Hz   │  │  0.3Hz   │             │
│  │researcher│  │ architect│             │
│  │Exploring │  │ Resting  │             │
│  └──────────┘  └──────────┘             │
│                                          │
│  Collective harmony: 0.78               │
│  Breathing sync: partial (2/4 aligned)  │
└──────────────────────────────────────────┘
```

**Features:**
- Grid layout of all agent Spectres, rendered in miniature ASCII
- Each cell shows: Spectre creature, breathing rate, agent name, behavioral state
- Color-coded by behavioral state (ROSEDUST state colors)
- Collective harmony metric (how synchronized the collective is)
- Breathing synchronization indicator

See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre rendering details.

---

## Screen Navigation

### Keyboard Shortcuts

| Key | Action |
|---|---|
| `1`–`6` | Jump to Region 1–6 |
| `Tab` / `Shift+Tab` | Cycle screens within current region |
| `↑`/`↓` | Navigate within focused list |
| `Enter` | Drill into detail |
| `Esc` | Back to parent region |
| `q` | Quit TUI |
| `?` | Help overlay (lists all shortcuts) |
| `/` | Global search |
| `Space` | Toggle auto-scroll (in scrollable views) |

### Screen Transition Map

```
Region 1 (Navigation)
  Agent List ──Enter──▸ Region 2 (Agent Detail)
  Plan List  ──Enter──▸ Region 3 (Plan Detail)
  Mesh       ──Enter──▸ Region 5 (Collective)
  Knowledge  ──Enter──▸ Region 4 (Knowledge)
  Episodes   ──Enter──▸ Region 2 (Agent Detail, focused on episode agent)
  Settings   ──Enter──▸ (inline editing)

Region 2 (Agent Detail) ──Esc──▸ Region 1, Agent List
Region 3 (Plan Detail)  ──Esc──▸ Region 1, Plan List
Region 4 (Knowledge)    ──Esc──▸ Region 1, Knowledge Browser
Region 5 (Collective)   ──Esc──▸ Region 1, Mesh
Region 6 (System)       ──Esc──▸ Region 1, Agent List
```

---

## Current Status and Gaps

**Built (in `roko-cli/src/tui/`):**
- Agent list view (`views/agents.rs`)
- Plan list view (`views/plans.rs`)
- Dashboard scaffold (`views/dashboard.rs`)
- Config view (`views/config.rs`)
- Log view (`views/logs.rs`)
- Signal view (`views/signals.rs`)
- Agent grid widget (`widgets/agent_grid.rs`)
- Plan tree widget (`widgets/plan_tree.rs`)
- Status bar, header bar, phase bar, token bar widgets
- Task detail and plan detail modals
- Braille rendering widget (`widgets/braille.rs`)
- Scrollbar widget

**Not yet built:**
- Interactive TUI rendering (currently text-only via `--text`)
- Region-based navigation (1–6 shortcuts)
- Agent Detail screens (2.1–2.6) — data available, UI not wired
- Plan Detail screens (3.1–3.5) — DAG data available, UI not wired
- Knowledge screens (4.1–4.4) — requires Neuro store integration
- Collective screens (5.1–5.4) — C-Factor computed, UI not wired
- System screens (6.1–6.4) — event data available, UI not wired
- Spectre Gallery — requires Spectre rendering implementation

---

## Cross-references

- See [08-tui-main-layout.md](./08-tui-main-layout.md) for the main layout structure
- See [07-rosedust-design-language.md](./07-rosedust-design-language.md) for the color palette
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre rendering
- See topic [09-daimon](../09-daimon/INDEX.md) for behavioral states and PAD vector
- See topic [07-cfactor](../07-cfactor/INDEX.md) for C-Factor computation
- See topic [11-neuro](../11-neuro/INDEX.md) for the knowledge tier system
