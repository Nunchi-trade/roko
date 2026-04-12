# Runtime Harness (PlanRunner)

> **Module**: `roko-cli/src/orchestrate.rs`
> **Key type**: `PlanRunner`
> **CLI entry**: `roko plan run <dir>` → calls `PlanRunner::run()`


> **Implementation**: Shipping

---

## Overview

The `PlanRunner` is the effectful runtime harness that connects the pure
`ParallelExecutor` state machine to real side effects: spawning agent processes,
running compilation gates, merging git branches, and persisting results. It is
the bridge between the orchestrator's abstract actions and the concrete
operating system.

While the `ParallelExecutor` decides *what* should happen, the `PlanRunner`
decides *how* it happens. It owns all the stateful subsystems — the learning
runtime, the Daimon affect engine, the skill library, the knowledge store, the
process supervisor, the conductor, and the MCP server state.

---

## Structure

`PlanRunner` holds over 30 fields spanning every subsystem:

### Core orchestration

| Field | Type | Purpose |
|-------|------|---------|
| `executor` | `ParallelExecutor` | Pure state machine |
| `event_log` | `EventLog` | Hash-chained audit log |
| `worktrees` | `WorktreeManager` | Per-plan git worktree lifecycle |
| `post_merge` | `PostMergeRunner` | Post-merge regression detection |
| `task_trackers` | `HashMap<String, TaskTracker>` | Per-plan task progress |

### Agent management

| Field | Type | Purpose |
|-------|------|---------|
| `supervisor` | `ProcessSupervisor` | Agent process lifecycle tracking |
| `cancel` | `CancelToken` | Root cancellation token for coordinated shutdown |
| `mcp_state` | `Mutex<McpServerState>` | MCP server clients and lease counts |
| `tool_registry` | `Option<Arc<DynamicToolRegistry>>` | Static + MCP-discovered tools |

### Learning and adaptation

| Field | Type | Purpose |
|-------|------|---------|
| `learning` | `LearningRuntime` | Episode logger, model router, experiments |
| `daimon` | `DaimonState` | PAD affect vector for dispatch modulation |
| `skill_library` | `SkillLibrary` | Reusable task patterns from prior successes |
| `knowledge_store` | `KnowledgeStore` | Durable knowledge queried per-task |
| `adaptive_thresholds` | `AdaptiveThresholds` | Per-gate-rung pass rate tracking |
| `format_bandit` | `ProfileBandit` | Adaptive tool-call format per model/role |
| `crate_familiarity_tracker` | `CrateFamiliarityTracker` | Per-crate success rates |
| `attribution_tracker` | `ContextAttributionTracker` | Context usage tracking |
| `context_average_tracker` | `ContextAverageTracker` | Rolling EMA of reference rates |

### Monitoring

| Field | Type | Purpose |
|-------|------|---------|
| `conductor` | `Arc<Conductor>` | Anomaly detection, watchers |
| `conductor_signals` | `Vec<Signal>` | Signals for conductor evaluation |
| `metrics` | `Arc<MetricRegistry>` | Prometheus-style counters/histograms |
| `health_probes` | `ProbeRegistry` | Readiness/liveness probes |
| `obs_sinks` | `FsObservabilitySinks` | File-backed traces and metrics |

### Cost tracking

| Field | Type | Purpose |
|-------|------|---------|
| `plan_costs` | `HashMap<String, f64>` | Cumulative USD per plan |
| `task_costs` | `HashMap<String, f64>` | Cumulative USD per task dispatch |
| `efficiency_events` | `Vec<AgentEfficiencyEvent>` | In-memory efficiency log |

---

## The Dispatch Loop

The `PlanRunner::run()` method implements the core orchestration loop:

```rust
loop {
    // 1. Get next actions from the state machine
    let actions = self.executor.tick();

    // 2. Dispatch each action
    for action in actions {
        match action {
            ExecutorAction::DispatchPlan { plan_id } => {
                self.dispatch_plan(&plan_id).await?;
            }
            ExecutorAction::SpawnAgent { plan_id, role, task } => {
                self.spawn_agent(&plan_id, role, &task).await?;
            }
            ExecutorAction::RunGate { plan_id, rung } => {
                self.run_gate(&plan_id, rung).await?;
            }
            ExecutorAction::MergeBranch { plan_id } => {
                self.merge_branch(&plan_id).await?;
            }
            // ... other actions
        }

        // 3. Auto-save periodically
        self.actions_since_save += 1;
        if self.actions_since_save >= AUTOSAVE_INTERVAL {
            self.save_snapshot().await?;
            self.actions_since_save = 0;
        }
    }

    // 4. Check if all plans are terminal
    if self.all_plans_terminal() {
        break;
    }
}
```

### Auto-save interval

The executor snapshot is saved every `AUTOSAVE_INTERVAL` (5) actions. This
means at most 5 actions of work can be lost in a crash. The snapshot is written
atomically (write-to-temp + rename) to prevent corruption.

---

## Agent Dispatch

When the executor requests `SpawnAgent`, the runtime builds a complete agent
configuration:

### 1. Model selection via CascadeRouter

The `CascadeRouter` (from `roko-learn`) selects the model based on:

- Task complexity band (Fast / Standard / Complex)
- Agent role (Implementer, Strategist, Auditor, etc.)
- Iteration count (higher iterations → more capable models)
- Prior gate failure (failures → model escalation)
- Crate familiarity (low familiarity → better model)
- Affect confidence from Daimon state

This implements the dual-process architecture described in
`refactoring-prd/02-five-layers.md`: T0 (no LLM) → T1 (fast model) → T2 (deep
model) cascade, where the system starts with the cheapest option and escalates
on failure.

### 2. System prompt assembly via RoleSystemPromptSpec

The 6-layer system prompt builder constructs role-specific prompts:

```rust
let spec = RoleSystemPromptSpec {
    role,
    plan_id: plan_id.clone(),
    task_id: task_id.clone(),
    plan_context: plan_artifacts,
    task_context: task_ctx,
    learned_context: learned,
    feedback_context: feedback,
    operating_constraints: constraints,
};
let system_prompt = spec.build();
```

### 3. Agent configuration

```rust
struct AgentRunConfig {
    command: String,           // "claude" or custom command
    exec_dir: PathBuf,         // plan worktree path
    model: String,             // from CascadeRouter
    timeout_ms: u64,           // from config
    bare_mode: bool,           // --print for non-interactive
    effort: String,            // "low" | "medium" | "high"
    system_prompt: String,     // from RoleSystemPromptSpec
    allowed_tools_csv: String, // role-specific tool whitelist
    mcp_config: Option<PathBuf>, // MCP server config
    fallback_model: Option<String>,
    env_vars: Vec<(String, String)>,
    read_args: Vec<String>,    // --read file paths
    extra_args: Vec<String>,
    resume_session: Option<String>,
    prompt: String,            // task prompt
    skip_permissions: bool,
}
```

### 4. Parallel execution

Agents run in a Tokio `JoinSet`, enabling parallel execution within and across
plans. The `run_prepared_agent()` function takes an owned `AgentRunConfig` (no
borrows from `PlanRunner`) so multiple agents can run concurrently:

```rust
async fn run_prepared_agent(cfg: AgentRunConfig) -> AgentResult {
    if cfg.command == "claude" {
        let agent = ClaudeCliAgent::new(...)
            .with_system_prompt(cfg.system_prompt)
            .with_tools(cfg.allowed_tools_csv)
            // ...
        agent.run(&prompt_signal, &ctx).await
    } else {
        let agent = ExecAgent::new(...)
        agent.run(&prompt_signal, &ctx).await
    }
}
```

---

## Task Tracking

The `TaskTracker` manages per-task progress within a plan:

### State tracking

- `completed: Vec<String>` — successfully completed task IDs
- `failed: Vec<String>` — terminally failed task IDs
- `skipped: Vec<String>` — skipped task IDs
- `current_group_index: usize` — current parallel group

### Ready task computation

`ready_tasks()` returns tasks where:
1. Not completed, failed, or skipped
2. All intra-plan dependencies satisfied (in `completed` list)
3. All cross-plan dependencies satisfied (in `completed_plans` list)

### Priority modulation

`prioritize_ready_tasks()` uses the Daimon's arousal value to modulate task
ordering:

```rust
fn prioritize_ready_tasks(ready: Vec<String>, arousal_for_task: F) -> Vec<String> {
    // effective_priority = base_priority * (1.0 + arousal * 0.5)
    // Higher arousal → higher effective priority → runs first
}
```

This implements the Yerkes-Dodson principle: moderate arousal boosts
performance, so high-arousal tasks (urgent, time-sensitive) get dispatched
first.

### Re-planning

When a plan accumulates too many gate failures, the `TaskTracker` can trigger
re-planning:

1. `gate_failure_count` tracks consecutive gate failures
2. If `gate_failure_count > threshold`, trigger `roko prd plan <slug>` to
   regenerate the task list
3. `merge_regenerated_plan()` merges the new plan with completed tasks,
   preserving work already done
4. `reload_tasks_file()` reloads `tasks.toml` after regeneration

---

## Conductor Integration

A background `WatcherRunner` tails `.roko/signals.jsonl` and periodically
runs the conductor against recent signals:

```rust
struct WatcherRunner {
    conductor: Arc<Conductor>,
    signals_path: PathBuf,
    efficiency_path: PathBuf,
    budget_usd: Option<f64>,
    cancel: TokioCancellationToken,
}
```

The watcher runs every `WATCHER_INTERVAL_SECS` (30 seconds), reading the most
recent `WATCHER_SIGNAL_TAIL` (200) signals. Alert signals are persisted back
to the signal log for the orchestrator to act on.

The conductor provides 10 watchers including:
- Cost overrun detection (budget_usd)
- Context window pressure monitoring
- Silence detection (agents not producing output)
- Ghost turn detection (agents looping without progress)

See `11-conductor-integration.md` for full details.

---

## Learning Integration

After each agent dispatch, the runtime records learning data:

### Efficiency events

```rust
AgentEfficiencyEvent {
    plan_id, task_id, role, model,
    total_prompt_tokens, total_completion_tokens,
    cost_usd, duration_ms,
    gate_passed: bool,
}
```

Written to `.roko/learn/efficiency.jsonl` for cost tracking and model routing
feedback.

### Episode logging

Agent turns and gate results are recorded as episodes in
`.roko/episodes.jsonl`. Episodes feed into the skill extraction pipeline —
successful task patterns are extracted as reusable `Skill` entries.

### Crate familiarity

`CrateFamiliarityTracker` records per-crate success rates:

```rust
struct CrateFamiliarityTracker {
    path: PathBuf,
    stats: HashMap<String, (u64, u64)>,  // (success_count, total_count)
}
```

The familiarity score feeds into the `CascadeRouter`'s context vector for
model selection — unfamiliar crates get assigned more capable models.

### Context attribution

`ContextAttributionTracker` tracks which context types (knowledge tier ×
source type) are actually referenced by agents. This enables automatic context
demotion — if a context type is consistently unreferenced, it gets deprioritized
in future dispatches.

---

## Reporting

`PlanRunner::run()` returns an `OrchestrationReport`:

```rust
pub struct OrchestrationReport {
    pub plans: Vec<PlanRunReport>,
    pub total_agent_calls: usize,
    pub total_gate_runs: usize,
    pub fleet_cfactor: Option<FleetCFactor>,
}
```

The fleet C-Factor (Woolley et al. 2010) is computed as a collective
intelligence metric: how much better the multi-agent system performs compared
to the sum of individual agents. See `12-stigmergy-niche.md` for the
theoretical background.

---

## References

- Woolley, A. W. et al. (2010). Evidence for a collective intelligence factor.
  *Science*, 330(6004), 686–688. (C-Factor metric)
- Yerkes, R. M. & Dodson, J. D. (1908). The relation of strength of stimulus to
  rapidity of habit-formation. *Journal of Comparative Neurology and
  Psychology*, 18(5), 459–482. (Arousal-based task prioritization)
- Sumers, T. R. et al. (2023). Cognitive architectures for language agents.
  *arXiv:2309.02427*. (CoALA cognitive cycle)
- Damasio, A. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*.
  Putnam. (Somatic marker hypothesis — underpins the Daimon affect integration)
