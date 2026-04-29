# 27 — Orchestrator

> Plan runner v2: event-driven design (~2,400 LOC replacing 21K). 12 mori parity gaps with full specifications. Current state reconciliation showing what exists vs what needs building. Every runner component is a Cell processing Signals through Bus and Store. The orchestrator is the Engine's concrete realization for plan execution.

**Status**: SPEC DRAFT
**Replaces**: `crates/roko-cli/src/orchestrate.rs` (21,478 lines)
**Target**: `crates/roko-cli/src/runner/` (~2,500 lines across 10 files)

**Depends on**: [02-CELL](02-CELL.md) (Cell protocol), [03-GRAPH](03-GRAPH.md) (Graph composition), [04-EXECUTION](04-EXECUTION.md) (Engine, Flow, Activity recording), [05-AGENT](05-AGENT.md) (Agent lifecycle), [06-MEMORY](06-MEMORY.md) (Knowledge Store for context injection), [07-LEARNING](07-LEARNING.md) (Episodes, CascadeRouter, efficiency events), [15-TELEMETRY](15-TELEMETRY.md) (StateHub, Lenses)

---

## 1. Why Rewrite

`orchestrate.rs` has correct crate boundaries but broken wiring. Every fix reveals another layer of tangled assumptions:

- `plans_dir()` resolves to the wrong directory (3 bugs from this alone)
- Enrichment pipeline overwrites user's `tasks.toml`
- `discover_plans` treats enrichment artifacts as plans
- Agent output captured batch-only (no streaming)
- All persistence buffered in memory (crash = total loss)
- 250 methods on PlanRunner across 15K lines in 3 impl blocks
- TUI gets nothing during agent execution

Mori solved all of these in ~2,500 lines. The existing roko crate APIs are solid. The problem is entirely the glue layer.

---

## 2. Design Principles

Taken from mori's architecture + unified spec:

### P1: Single-threaded event loop, async I/O
No race conditions on state. TUI rendering atomic per frame. All mutations in one place. Mori's `sequential.rs` uses `tokio::select!` over 4 channels — proven pattern.

### P2: Channels as event bus
Four independent channels: agent events, executor actions, gate results, TUI input. Decouples subsystems. Non-blocking recv. Easy to extend.

### P3: Stream, don't batch
Parse `--stream-json` line by line (like mori's `parse_claude_event`). Emit `AgentEvent::MessageDelta` / `TokenUsage` / `ToolCall` in real time. TUI sees text as it arrives.

### P4: Flush after every task
Write episode + efficiency + executor snapshot after each task completes. Atomic writes (tmp + rename). Crash loses at most 1 task. Mori writes every 2 seconds.

### P5: The plan dir is the plan
When user passes a directory with `tasks.toml`, that's the plan. No scanning for `.md` files. No enrichment artifact confusion. `tasks.toml` is the source of truth and is never overwritten by the system.

### P6: Executor is pure, runner does I/O
`ParallelExecutor::tick()` returns actions. Runner dispatches them to real systems. Results fed back as events. This is already the pattern — just not cleanly implemented.

### P7: Align with unified spec
Use naming conventions from the unified spec where practical. Structure the runner as a precursor to the unified Engine. Activity recording per-node (not monolithic snapshots). Lifecycle Pulses on Bus/StateHub.

---

## 3. Architecture

```
                    +-----------------------------+
                    |         PlanRunner           |
                    |  (event loop: select!)       |
                    |                              |
  +--agent_rx-->    |  match event {               |
  |                 |    AgentEvent::* => update    |    --> StateHub (TUI)
  |  +--gate_rx-->  |    GateResult => feed executor|
  |  |              |    TuiInput => handle keys   |    --> EpisodeLogger
  |  |  +--tick-->  |    Tick => executor.tick()   |
  |  |  |           |      => dispatch actions     |    --> executor.json
  |  |  |           |  }                           |
  |  |  |           +-----------------------------+
  |  |  |                    |
  |  |  |           +--------+--------+
  |  |  |           | ParallelExecutor | (pure state machine)
  |  |  |           +--------+--------+
  |  |  |                    | ExecutorAction
  |  |  |           +--------+--------+
  |  |  |           | Action Dispatcher |
  |  |  |           +-----------------+
  |  |  |           | SpawnAgent      |--> AgentProcess (--stream-json)
  |  |  |           | RunGate         |--> gate pipeline (background task)
  |  |  |           | MergeBranch     |--> git operations
  |  |  |           | CompletePlan    |--> cleanup
  |  |  |           +-----------------+
  |  |  |
  |  |  +-- tokio interval (33ms for TUI, 2s for flush)
  |  +----- gate completion channel
  +-------- agent stdout line-by-line parsing
```

---

## 4. Module Layout

```
crates/roko-cli/src/runner/
  mod.rs            -- PlanRunner struct, public run() API              (~200 lines)
  event_loop.rs     -- tokio::select! main loop                        (~400 lines)
  agent_stream.rs   -- spawn claude, parse --stream-json per line      (~300 lines)
  agent_events.rs   -- handle AgentEvent variants, update RunState     (~250 lines)
  gate_dispatch.rs  -- spawn gate as background task, collect results  (~200 lines)
  persist.rs        -- atomic writes: executor.json, episodes, efficiency (~200 lines)
  plan_loader.rs    -- load tasks.toml, validate, no discovery magic   (~150 lines)
  state.rs          -- RunState: agent output, tokens, costs, progress (~300 lines)
  tui_bridge.rs     -- publish DashboardEvents to StateHub             (~200 lines)
  types.rs          -- AgentEvent, GateCompletion, RunConfig           (~200 lines)
```

**Total: ~2,400 lines.** Average ~240 lines/file. Largest: event_loop.rs at ~400.

---

## 5. Data Flow

### 5.1 Agent Output (Streaming)

Mori pattern adapted for roko:

```
claude --stream-json stdout
  |
  +-- {"type":"system", ...}           -> AgentEvent::SystemInit
  +-- {"type":"assistant", "message":  -> for each content block:
  |     {"content": [                      Text -> AgentEvent::MessageDelta
  |       {"type":"text", "text":"..."     ToolUse -> AgentEvent::ToolCall
  |       {"type":"tool_use", ...}
  |     ], "usage": {...}}}            -> AgentEvent::TokenUsage
  +-- {"type":"tool", ...}             -> AgentEvent::ToolOutput
  +-- {"type":"result", ...}           -> AgentEvent::TurnCompleted
                                          (session_id, total_cost, num_turns)
```

Each event flows through `agent_rx` to the event loop:

```rust
// Phase D: AgentEvent::MessageDelta → Pulse("agent.message_delta")
AgentEvent::MessageDelta { content, .. } => {
    run_state.agent.output.push_str(&content);
    state_hub.publish(DashboardEvent::AgentOutput {
        agent_id, content,
    });
}

// Phase D: AgentEvent::TokenUsage → Pulse("agent.token_usage")
AgentEvent::TokenUsage { input, output, cost, .. } => {
    run_state.cost.tokens_in = input;
    run_state.cost.tokens_out = output;
    run_state.cost.cost_usd += cost_delta;
    state_hub.publish(DashboardEvent::TokenUpdate {
        agent_id, input, output, cost,
    });
}

// Phase D: AgentEvent::TurnCompleted → Pulse("agent.turn_completed")
AgentEvent::TurnCompleted { session_id, .. } => {
    run_state.agent.active = false;
    run_state.agent.session_id = session_id;  // for --resume
    executor.apply_event(plan_id, &ExecutorEvent::AgentDone);
}
```

### 5.2 Gate Execution (Background)

Gates run as background tokio tasks, never blocking the event loop:

```rust
// Spawn gate as tokio task, don't block event loop
let gate_tx = gate_tx.clone();
tokio::spawn(async move {
    let verdicts = roko_gate::run_rung(&signal, &ctx, rung, &inputs, &config).await;
    let _ = gate_tx.send(GateCompletion { plan_id, rung, verdicts });
});
```

Event loop receives:
```rust
Some(completion) = gate_rx.recv() => {
    let passed = completion.verdicts.iter().all(|v| v.passed);
    executor.apply_event(&completion.plan_id, if passed {
        &ExecutorEvent::GatePassed
    } else {
        &ExecutorEvent::GateFailed(format_failures(&completion.verdicts))
    });
    persist.record_gate_result(&completion);
    state_hub.publish(DashboardEvent::GateResult { ... });
}
```

### 5.3 Persistence (After Every Task)

```rust
// Called after each task completion or gate result
fn flush_task_checkpoint(&self) {
    // 1. Executor snapshot (atomic write)
    let snap = self.executor.snapshot();
    atomic_write(&self.paths.executor_json, &serde_json::to_string_pretty(&snap)?)?;

    // 2. Episode (append)
    self.episode_logger.record(&episode)?;  // already flushes per-write

    // 3. Efficiency event (append + flush)
    append_jsonl(&self.paths.efficiency_jsonl, &efficiency_event)?;

    // 4. Routing decision (append)
    append_jsonl(&self.paths.routing_jsonl, &routing_log)?;
}
```

### 5.4 Plan Loading (Simple, Correct)

```rust
pub fn load_plan(plan_dir: &Path) -> Result<Plan> {
    let tasks_path = plan_dir.join("tasks.toml");
    anyhow::ensure!(tasks_path.exists(), "no tasks.toml in {}", plan_dir.display());

    let tasks_file = TasksFile::parse(&tasks_path)?;

    // NEVER overwrite tasks.toml. NEVER scan for .md files.
    // The plan is exactly what the user authored.

    Ok(Plan {
        id: plan_dir.file_name().unwrap().to_string_lossy().to_string(),
        dir: plan_dir.to_path_buf(),
        tasks: tasks_file,
        skip_enrichment: tasks_file.meta.skip_enrichment,
    })
}
```

---

## 6. RunState (TUI Data Model)

Mori's `RunState` pattern — single struct composed of focused component structs. Each component maps to a Lens type in Phase D.

```rust
// Phase D: AgentState fields → Lens("agent.{field}") projections
pub struct AgentState {
    pub active: bool,
    pub role: Option<AgentRole>,
    pub model: String,
    pub output: String,              // accumulated MessageDelta text
    pub session_id: Option<String>,
}

// Phase D: CostState fields → Lens("cost.{scope}") projections
pub struct CostState {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub cost_per_plan: HashMap<String, f64>,
    pub cost_per_task: HashMap<String, f64>,
    pub total_cost_usd: f64,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
}

// Phase D: ProgressState fields → Lens("progress.{field}") projections
pub struct ProgressState {
    pub current_plan: String,
    pub current_task: Option<String>,
    pub current_phase: String,
    pub tasks_total: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
    pub task_checklist: Vec<TaskRow>,
}

/// RunState composes component structs rather than flattening all fields.
/// Phase D: RunState → collection of Lens projections over Bus.
pub struct RunState {
    pub agent: AgentState,
    pub cost: CostState,
    pub progress: ProgressState,

    // Gate output (streamed from background task)
    pub gate_output: String,
    pub gate_running: bool,

    // Iteration tracking
    pub iteration: u32,
    pub max_iterations: u32,

    // Cumulative
    pub episodes: Vec<EpisodeSummary>,
}
```

Every field is updated inside the event loop. TUI renders `&RunState` each frame.

---

## 7. Event Types

```rust
/// Events from the agent subprocess (parsed from --stream-json).
/// Phase D: AgentEvent → Pulse("agent.{event_type}") on Bus.
pub enum AgentEvent {
    SystemInit { model: String, session_id: Option<String> },
    MessageDelta { content: String },
    ToolCall { name: String, id: String, input: Value },
    ToolOutput { content: String },
    TokenUsage { input: u64, output: u64, cost: Option<f64> },
    TurnCompleted { session_id: String, total_cost: Option<f64>, num_turns: u64 },
    Error { message: String },
    Exited { exit_code: Option<i32> },
}

/// Results from background gate tasks.
pub struct GateCompletion {
    pub plan_id: String,
    pub task_id: String,
    pub rung: u32,
    pub verdicts: Vec<Verdict>,
    pub output: String,         // gate stdout/stderr for TUI
    pub duration: Duration,
}
```

---

## 8. Mori Parity Gaps — 12 Specifications

These features exist in mori but are not yet in roko's orchestrator. The types and detection rules already exist in roko crates (see section 9 reconciliation); what is missing is the **wiring** into the orchestration loop.

### Gap 1: Structured Review Verdicts + Express Mode

**Source**: `bardo/apps/mori/src/orchestrator/review.rs`
**Target**: `crates/roko-gate/src/review_verdict.rs` (types exist) + wire into runner

Parse agent review output into structured verdicts with issue classification. Types already exist in `roko-gate`:

- `ReviewDecision` enum: `Approve | Revise | Skip`
- `ReviewIssue { severity, category, gate, rung, file, line, suggestion, blocking }`
- `ReviewVerdict { decision, summary, issues, rung_results }`
- 10 `IssueCategory` variants (Compilation, Test, TypeMismatch, MissingImpl, Docs, Style, etc.)

**Parsing fallback chain**:

```rust
fn parse_review(output: &str) -> StructuredReview {
    // 1. Try parsing entire output as JSON
    if let Ok(review) = serde_json::from_str::<StructuredReview>(output) {
        return review;
    }
    // 2. Try extracting JSON from ```json ... ``` code block
    if let Some(json_block) = extract_code_block(output, "json") {
        if let Ok(review) = serde_json::from_str::<StructuredReview>(&json_block) {
            return review;
        }
    }
    // 3. Try extracting TOML from ```toml ... ``` code block
    if let Some(toml_block) = extract_code_block(output, "toml") {
        if let Ok(review) = toml::from_str::<StructuredReview>(&toml_block) {
            return review;
        }
    }
    // 4. Fallback: treat entire output as a Revise verdict with raw text
    StructuredReview {
        verdict: ReviewDecision::Revise,
        issues: vec![],
        summary: output.chars().take(500).collect(),  // cap at 500 chars
    }
}
```

The fallback (step 4) means **parsing never fails** — worst case, the raw text becomes the summary and the orchestrator treats it as a revision request.

**Express mode**: When `all_issues_quick_fixable()` returns true (all issues are Compilation, Docs, Style, LintViolation, or Unused), skip strategist and go directly to implementer. NOT quick-fixable (even if small): TestFailure, TypeMismatch, SpecDeviation, SecurityVulnerability.

**Acceptance criteria**:
- [ ] `StructuredReview` parses from JSON agent output
- [ ] Fallback parsing handles malformed JSON gracefully (returns Revise with raw text)
- [ ] `all_issues_quick_fixable()` correctly identifies trivial-fix scenarios
- [ ] Express mode: quick-fixable review -> implementer (no strategist)
- [ ] Integration test: mock review JSON -> parsed verdict -> correct phase transition

### Gap 2: Compile Error Classification + Auto-Fix

**Source**: `bardo/apps/mori/src/orchestrator/autofix.rs`
**Target**: `crates/roko-gate/src/compile_errors.rs` (types exist) + wire into runner

Types already exist: `CompileError { category, code, message, file, line, column, suggestion }` with `ErrorCategory` enum (11 categories: Syntax, UnresolvedImport, TypeMismatch, Lifetime, MissingMember, Unused, Visibility, Macro, TraitBound, Ownership, Other).

**Remaining wiring**:

1. `apply_rustc_fixes(worktree: &Path)` — Run `cargo fix --allow-dirty` + `cargo fmt` to apply compiler-suggested fixes directly (no agent needed).
2. `collect_rustc_suggestions(json_output: &str) -> Vec<RustcSuggestion>` — Extract `children[].suggested_replacement` from diagnostic JSON.
3. In runner autofix path: first try `apply_rustc_fixes()`. If that resolves all errors, skip agent retry. Otherwise, pass classified errors (not raw cargo output) to agent.

**Merge conflict handling**: If `cargo fix --allow-dirty` exits non-zero (conflicting suggestions), skip auto-fix and fall through to agent-assisted fix. Never use `--allow-staged`.

**Acceptance criteria**:
- [ ] `apply_rustc_fixes()` runs cargo fix + fmt successfully
- [ ] Auto-fix resolves all errors -> skip agent retry
- [ ] Agent receives classified errors instead of raw output
- [ ] `cargo fix` failure -> graceful fallthrough to agent

### Gap 3: Error Pattern Sharing Between Parallel Agents

**Source**: `bardo/apps/mori/src/orchestrator/gates.rs`
**Target**: `crates/roko-gate/src/error_patterns.rs` + wire into runner

Share discovered error patterns across parallel agents so they learn from each other's failures.

- `extract_error_digest(output: &str) -> String` — Parse cargo/test output, extract `error[E...]` blocks, deduplicate via HashSet, cap at 10 unique errors, cap each at 200 chars.
- `append_discovered_pattern(repo_root, plan, error_digest)` — Write to `.roko/learn/discovered-patterns.json`.
- `read_discovered_patterns() -> Vec<DiscoveredPattern>` — Read last 5 unresolved patterns (200 chars each). Inject into agent context.
- `is_mostly_passing(results) -> bool` — >90% pass rate with >20 tests and >=1 failure = "mostly passing". Targeted fix should suffice (not full replan).

**Error deduplication**: Patterns deduplicated by **normalized error code + file path**:

```rust
fn error_key(error: &CompileError) -> String {
    format!("{}::{}", error.code, error.file.as_deref().unwrap_or("unknown"))
}
```

Two `error[E0425]` in different files ARE different patterns. Two `error[E0425]` in the same file with different line numbers are the SAME pattern.

**Acceptance criteria**:
- [ ] `extract_error_digest()` produces compact, deduped error signatures
- [ ] Patterns persisted to `.roko/learn/discovered-patterns.json`
- [ ] Parallel agents see each other's patterns (read from shared file, injected into system prompt)
- [ ] `is_mostly_passing()` returns true for 95% pass with 1 failure, false for 50%

### Gap 4: Post-Gate Reflection Loop

**Source**: `bardo/apps/mori/src/orchestrator/reflection.rs`
**Target**: New function in runner + extend Episode struct

After gate failure, spawn a lightweight agent to analyze what went wrong.

1. **Trigger**: After any gate failure (compile, test, clippy), before replanning
2. **Reflection agent**: Use cheapest model (haiku-4-5). Prompt: "Analyze this gate failure. What went wrong? What should the next attempt do differently? Gate output: {error_digest}. Files changed: {file_list}. Previous attempts: {iteration_count}."
3. **Output**: Store reflection text in episode's `reflection` field
4. **Injection**: On retry, inject last reflection into agent's system prompt as "Lessons from previous attempt: {reflection}"
5. **Deduplication**: If error_digest matches a previous reflection's error pattern, skip re-generating
6. **Cost guard**: Token-based, not price-based: max_tokens=500, model=haiku-4-5. Actual cost ~$0.0001.

**Acceptance criteria**:
- [ ] Reflection generated on gate failure (visible in episode log)
- [ ] Reflection injected into retry agent's prompt
- [ ] Deduplication: same error pattern doesn't trigger second reflection
- [ ] Cost capped: max_tokens=500, model=haiku

### Gap 5: Context Injection Scoping + KnowledgeConfig

**Source**: `bardo/apps/mori/src/orchestrator/inject.rs`
**Target**: `crates/roko-compose/src/context_scoping.rs` + wire into runner

Scope playbook rules to plan's touched files and enable per-category toggles.

**KnowledgeConfig** struct:
- `file_intel_enabled: bool` (default true), `file_intel_max_entries: usize` (default 5)
- `warnings_enabled: bool`, `warning_max_entries: usize`
- `error_patterns_enabled: bool`, `error_pattern_min_cluster: usize`
- `wave_context_enabled: bool` (read context from sibling tasks in same wave)
- `dynamic_budget_enabled: bool` (adjust context size per file difficulty)

**Role-filtered context** — different roles get different context sizes:

| Role | File intel entries | Warning entries | Error pattern entries |
|------|-------------------|-----------------|----------------------|
| Implementer | 10 (full: file path, key functions, recent changes) | 5 | 5 |
| Reviewer | 3 (summary: file path + one-line description) | 3 | 3 |
| Strategist | 0 (sees plan-level only) | 0 | 0 |

Configurable via roko.toml:

```toml
[knowledge]
file_intel_max_entries = 10
warnings_max_entries = 5
error_pattern_min_cluster = 3
```

**Acceptance criteria**:
- [ ] `KnowledgeConfig` loadable from `roko.toml` (with defaults)
- [ ] `collect_plan_playbook_scope()` narrows rule matching to plan's files
- [ ] Implementer gets full context; reviewer gets summary; verified by prompt inspection
- [ ] Config toggles actually suppress sections

### Gap 6: Warm Agent Spawning + WarmPool

**Source**: `bardo/apps/mori/src/agent/mod.rs`
**Target**: `crates/roko-runtime/src/warm_pool.rs` + wire into runner

Pre-spawn agents during gate execution for faster phase transitions.

**Cold spawn bottleneck profile**: A cold spawn (5-15s) consists of:
1. **fork + exec** (~50ms) -- OS process creation for `claude` CLI binary.
2. **Initial API auth** (~500ms-2s) -- TLS handshake + API key validation + session creation.
3. **Context loading** (~3-10s) -- System prompt assembly, MCP tool discovery, file context injection, model initialization on provider side.

Warm spawning saves steps 1-2 entirely and pre-loads step 3. The `promote_warm()` call only needs to inject the task-specific prompt (~100ms), because the process is already authenticated and context-primed.

1. **WarmPool**: `HashMap<AgentRole, WarmAgent>` where `WarmAgent` = pre-spawned process ready for promotion
2. **`pre_spawn_warm(role, effort)`**: During gate pipeline execution, spawn the next phase's agent in the background. Agent initializes but doesn't receive a task yet.
3. **`promote_warm(role) -> AgentConnection`**: Swap warm agent to active. Agent receives its task and starts working immediately. Saves 5-15s vs cold spawn.
4. **`evict_warm(role)`**: Kill warm agent on gate failure (no point keeping it if plan is replanning)

**Acceptance criteria**:
- [ ] Warm agent spawns in background during gate execution
- [ ] `promote_warm()` returns usable agent connection without re-spawn delay
- [ ] `evict_warm()` kills process and frees resources
- [ ] Timing: promote is <100ms vs 5-15s for cold spawn
- [ ] No leaked processes on gate failure path

### Gap 7: All 10 Conductor Watchers

**Status**: ALREADY IMPLEMENTED in `roko-conductor/src/watchers/`

All 10 watchers exist: GhostTurn, ReviewLoop, IterationLoop, TestFailureBudget, Silence, CompileFailRepeat, TaskStall, ContextPressure, TimeOverrun, CooldownFilter. Intervention system exists with BanditPolicy and WorstSeverityPolicy. Circuit breaker exists with Holt forecasting.

**Remaining**: Verify watcher thresholds are configurable via `[conductor]` in `roko.toml`:

```toml
[conductor]
ghost_turn_max_secs = 5
review_loop_max_consecutive = 3
iteration_loop_max = 6
test_failure_budget_pass_rate = 0.70
silence_timeout_secs = 180
compile_fail_max_consecutive = 3
task_stall_secs = 300
context_pressure_percent = 80
phase_timeout_secs = 1800
cooldown_filter_secs = 120
```

### Gap 8: Neuro Store -> Cascade Router Bias

**Target**: `crates/roko-learn/src/cascade_router.rs`

Currently the cascade router selects models based on observations (pass/fail history) but does NOT consult the neuro store.

1. At `decide()` time, query `knowledge_store.query(task_description, limit=3)` for relevant prior knowledge
2. If knowledge entries mention specific model preferences, bias model scoring by +0.1 for mentioned model
3. If knowledge entries describe failure patterns with a model, bias by -0.1
4. Add knowledge context to LinUCB feature vector (add 2 dims: `knowledge_match_score`, `knowledge_model_bias`)
5. Opt-in via `cascade_router.consult_knowledge: bool` in config (default true)

**Score clamping**: `final_score = clamp(base_score + knowledge_bias, 0.05, 1.0)`. The 0.05 floor ensures every model has a non-zero chance (exploration).

**Acceptance criteria**:
- [ ] Cascade router queries neuro store at decide time
- [ ] Knowledge bias clamped to [-0.1, +0.1], final score clamped to [0.05, 1.0]
- [ ] Config toggle works (disabled = no knowledge query)
- [ ] No performance regression: knowledge query <10ms

### Gap 9: Episode Clustering for Error Patterns

**Target**: `crates/roko-learn/src/pattern_discovery.rs`

Cluster failed episodes by error signature to recommend model fallbacks.

- `cluster_episodes(episodes: &[Episode]) -> Vec<EpisodeCluster>` — Group by `error_signature` (failures) or `file_pattern` (successes). Minimum cluster size: 3.
- `EpisodeCluster { key, count, maturity, success_rate, common_files, best_model, best_provider, avg_cost_usd, recommended_model }`
- `ClusterMaturity`: Immature (< 3 episodes, no recommendation) | Mature (>= 3, produces recommendation)
- Per cluster, compute which model has highest success_rate -> `recommended_model`

**Integration**: Feed cluster recommendations into cascade_router as soft priors. Cadence: every 10 new episodes.

**Acceptance criteria**:
- [ ] `cluster_episodes()` groups episodes with matching error signatures
- [ ] Mature clusters (3+) produce model recommendations; immature clusters stored but don't recommend
- [ ] Recommendations integrated as soft bias in cascade_router
- [ ] Test: 5 episodes with same error + model A succeeding -> recommends model A

### Gap 10: Provider Pass-Rate into Model Scoring

**Target**: `crates/roko-learn/src/cascade_router.rs`

Bias model selection toward proven providers.

1. `compute_provider_metrics(episodes)` -> per-provider: pass_rate, avg_cost, avg_duration (min 5 episodes)
2. In cascade_router Stage 2 (confidence) and Stage 3 (LinUCB): multiply model score by `provider_pass_rate`
3. Use existing ProviderHealthTracker data if available, fall back to episode-derived metrics

**Acceptance criteria**:
- [ ] Provider metrics computed from episode history
- [ ] Model scores multiplied by provider pass_rate in stages 2-3
- [ ] Provider with 0.9 pass_rate boosts its models vs provider with 0.6
- [ ] Minimum 5 episodes before provider metrics affect scoring

### Gap 11: Reflection-Derived Playbook Rules with Confidence Tracking

**Target**: `crates/roko-learn/src/playbook_rules.rs`

Auto-generate playbook rules from agent reflections (Gap 4 above).

1. After reflection stored in episode, extract actionable patterns:
   - Reflection mentions specific files -> create rule with `trigger_files` glob
   - Reflection mentions error type -> create rule with `trigger_tags`
   - Context injection = the reflection's key insight
2. **Confidence tracking**: New rules start at 0.5 (neutral). Boost +0.05 on gate pass, penalize -0.10 on gate fail. Remove rules below 0.2 confidence (unless manually created).
3. **Cadence**: Run after every 3 new reflections
4. **Persistence**: Append to `.roko/learn/playbook-rules.json` with `source: "reflection"` tag

**Acceptance criteria**:
- [ ] Reflections with file mentions -> playbook rules with trigger_files
- [ ] Confidence tracking: +0.05 on success, -0.10 on failure
- [ ] Rules below 0.2 auto-removed; manually created rules preserved
- [ ] Persistence in playbook-rules.json with `source: "reflection"` tag

### Gap 12: A-MAC 5-Factor Admission Gate for Neuro Store

**Target**: `crates/roko-neuro/src/`

Prevent hallucinated or contradictory knowledge from entering the store. 5-factor validation before any knowledge entry is stored:

1. **Similarity**: Too similar to existing knowledge? (cosine sim > 0.95 -> reject as duplicate)
2. **Novelty**: Does this add new information? (cosine sim < 0.3 to all existing -> novel)
3. **Contradiction**: Does this contradict existing high-confidence entries? Two HDC vectors per entry — one for **topic**, one for **assertion**. High topic similarity (> 0.7) + negative assertion similarity (< -0.3) = contradiction.
4. **Relevance**: Is this relevant to the agent's domain? (keyword match against domain tags)
5. **Confidence**: Does the source have sufficient credibility? (gate pass rate of the episode that generated this)

**Contradiction detection**:

```rust
fn check_contradiction(new_entry: &KnowledgeEntry, existing: &[KnowledgeEntry]) -> bool {
    for entry in existing.iter().filter(|e| e.confidence > 0.8) {
        let sim = cosine_similarity(&new_entry.hdc_vector, &entry.hdc_vector);
        // High similarity but opposite conclusion = contradiction
        if sim > 0.7 {
            let assertion_sim = cosine_similarity(
                &new_entry.assertion_vector,
                &entry.assertion_vector,
            );
            if assertion_sim < -0.3 {
                return true;  // Contradiction: same topic, opposite claim
            }
        }
    }
    false
}
```

If HDC vectors are not available (pre-HDC entries), fall back to keyword overlap for topic similarity and skip contradiction checking.

Gate result: `Admit | Reject { reason }`. Log rejections for debugging.

**Acceptance criteria**:
- [ ] Near-duplicate entries rejected (similarity > 0.95)
- [ ] Contradictory entries flagged (if existing entry has confidence > 0.8)
- [ ] Novel entries admitted with appropriate confidence score
- [ ] Rejections logged with reason
- [ ] Unit test: insert duplicate -> rejected; insert novel fact -> admitted; insert contradiction -> flagged

---

## 9. Current State Reconciliation

### Already Implemented (do NOT rebuild)

| Gap | Item | Location | Status |
|-----|------|----------|--------|
| 1 | `ReviewDecision`, `ReviewIssue`, `ReviewVerdict` types | `roko-gate/src/review_verdict.rs` | EXISTS (10 issue categories) |
| 2 | `CompileError`, `ErrorCategory` (11 categories), `classify_error_code()` | `roko-gate/src/compile_errors.rs` | EXISTS |
| 3 | `ErrorPattern` struct for cross-error pattern matching | `roko-conductor/src/diagnosis.rs` | EXISTS |
| 7 | All 10 conductor watchers | `roko-conductor/src/watchers/` | EXISTS |
| 7 | Intervention system (BanditPolicy, WorstSeverityPolicy) | `roko-conductor/src/interventions.rs` | EXISTS |
| 7 | Circuit breaker (Holt forecasting) | `roko-conductor/src/circuit_breaker.rs` | EXISTS |

### Remaining Work

| Gap | What | Status | Notes |
|-----|------|--------|-------|
| 1 | Wire ReviewVerdict parsing into runner | MISSING | Types exist, parsing agent output -> verdict not wired |
| 1 | Express mode (skip strategist when quick-fixable) | MISSING | Phase transition logic not wired |
| 2 | `apply_rustc_fixes()` auto-fix path | MISSING | `cargo fix --allow-dirty` + `cargo fmt` before agent |
| 2 | Wire classified errors into agent prompt | MISSING | Agent gets raw cargo output instead of structured |
| 3 | Error pattern sharing between parallel agents | MISSING | File exists but not injected into system prompt |
| 3 | `is_mostly_passing()` check | MISSING | Not used to decide fix strategy |
| 4 | Post-gate reflection loop | FULL GAP | Not implemented at all |
| 5 | Context injection scoping (KnowledgeConfig, role-filtered) | FULL GAP | Not implemented |
| 6 | Warm agent spawning (WarmPool) | FULL GAP | Not implemented |
| 7 | Configurable watcher thresholds via roko.toml | VERIFY | May be hardcoded |
| 8 | Neuro store -> cascade router bias | FULL GAP | Router doesn't consult knowledge |
| 9 | Episode clustering for error patterns | FULL GAP | No clustering |
| 10 | Provider pass-rate bias in model scoring | FULL GAP | Not multiplied |
| 11 | Reflection-derived playbook rules + confidence | FULL GAP | No auto-generation |
| 12 | A-MAC 5-factor admission gate | FULL GAP | No validation |

---

## 10. What Gets Rewritten vs Kept

### Rewrite (new `runner/` module)
- **Event loop** — new, based on mori's `sequential.rs` pattern
- **Agent streaming** — new, parse `--stream-json` line by line
- **Plan loading** — new, simple `tasks.toml` reader (no discovery)
- **Persistence** — new, flush after every task (atomic writes)
- **TUI bridge** — new, publish DashboardEvents with real data
- **RunState** — new, single struct for all TUI data

### Keep (existing crate APIs, called from runner)
- `ParallelExecutor` from roko-orchestrator (pure state machine)
- `run_rung()` from roko-gate (gate execution)
- `LearningRuntime` from roko-learn (episodes, routing, efficiency)
- `ProcessSupervisor` from roko-runtime (PID tracking, kill)
- `StateHub` from roko-core (TUI event publishing)
- `SafetyLayer` from roko-agent (tool authorization)
- `Conductor` from roko-conductor (health monitoring, 10 watchers)
- `RoleSystemPromptSpec` from roko-compose (prompt building)
- `CascadeRouter` from roko-learn (model selection)

### Deprecate (move to `orchestrate_legacy.rs`)
- `PlanRunner` god object (250 methods)
- `ReviewDriftReport` (2,833 lines — not needed for plan execution)
- `StaticCFactorSource` (644 lines — can be a separate file)
- `WatcherRunner` (564 lines — conductor handles this)
- `CrateFamiliarityTracker` (302 lines — enrichment-only)
- All enrichment pipeline code (not needed when `skip_enrichment = true`)
- All path resolution hacks (`plans_dir()` + fallbacks)

---

## 11. Migration Strategy

### Phase A: Build runner alongside orchestrate.rs (1-2 days)
1. Create `crates/roko-cli/src/runner/` module
2. Implement plan loading, event loop, agent streaming, persistence
3. Wire to existing crate APIs
4. Test with a real plan

### Phase B: Wire into CLI (hours)
1. Add `--runner v2` flag to `roko plan run`
2. Default to v2, `--runner legacy` falls back to orchestrate.rs
3. Verify TUI works with new runner

### Phase C: Deprecate orchestrate.rs (after validation)
1. Rename to `orchestrate_legacy.rs`
2. Extract useful pieces (CFactorSource, etc.) to separate files
3. Remove `--runner legacy` after confidence period

### Phase D: Align with unified spec (future)
1. Rename types (Engram -> Signal, etc.) per Phase 1 kernel
2. Add Activity recording per-node (unified resumability)
3. Add Pulse-based lifecycle events on Bus
4. Replace event loop with Engine interpretation of TOML Graphs

---

## 12. Relationship to Unified Spec

This plan runner v2 is a **stepping stone** toward the unified Engine:

| Unified Engine concept | Plan runner v2 equivalent |
|---|---|
| Engine interprets Graphs | Event loop dispatches ExecutorActions |
| Flow lifecycle Pulses | DashboardEvents on StateHub |
| Activity recording per-node | Episode + efficiency flush per-task |
| Workflow/Activity split | Executor (pure) / Runner (I/O) split |
| Failure strategies | Gate retry loop (iteration++) |
| Budget enforcement | Cost tracking per-plan per-task |
| Verify protocol | Gate pipeline (run_rung) |
| Agent type-state lifecycle | Agent spawn -> stream -> exit tracking |

The v2 runner will not implement the full Cell/Graph/Protocol stack, but it is structured so the unified Engine can subsume it incrementally.

---

## 13. Enrichment (Separate, Not in Plan Run)

Enrichment is a **separate command** (`roko plan enrich <dir>`), not part of `plan run`. If the user wants enrichment, they run it first. `plan run` NEVER modifies `tasks.toml`. The `skip_enrichment` flag becomes unnecessary because `plan run` doesn't enrich by default.

Enrichment artifacts go to a subdirectory (`<plan>/enrichment/`) not alongside `tasks.toml`.

---

## 14. Success Criteria

A plan run with `--approval`:

1. **One plan discovered** (not 6 phantom plans)
2. **Enrichment skipped** (no artifacts written, no agents spawned for enrichment)
3. **Tasks execute** (dispatched to correct model via CascadeRouter)
4. **Streaming output** in TUI (text appears as agent types, not after exit)
5. **Token counters update** in real time (not "-" or "0k")
6. **Model name shown** (from SystemInit event, not efficiency.jsonl)
7. **Task titles shown** (from tasks.toml, not "plan plan")
8. **executor.json written** after each task (crash-safe)
9. **Episodes written** after each task (visible in `.roko/episodes.jsonl`)
10. **Efficiency events** flushed (visible in `.roko/learn/efficiency.jsonl`)
11. **Ctrl+C** kills agent + all descendants within 3 seconds
12. **Resume** from executor.json after crash (skip completed tasks)
13. **Gate output** visible in TUI (compile/test results stream)
14. **Cost tracking** accurate per-plan and per-task

---

## Version History

| Version | Date | Changes |
|---|---|---|
| 3.0 | 2026-04-26 | Unified spec: merged plan runner v2, 12 mori parity gaps with specs, current state reconciliation, migration strategy, relationship to unified Engine. |
| 2.0 | 2026-04-24 | Plan runner v2 standalone spec + orchestrator gaps doc. |
| 1.0 | 2026-04-20 | Initial orchestrator gap analysis from mori parity checklist. |
