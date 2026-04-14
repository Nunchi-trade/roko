# 05 — Agent Pools

> Sub-doc 05 of **02-agents** · Roko Documentation
>
> This document describes the `AgentPool` (sequential, single-role) and
> `MultiAgentPool` (parallel, multi-role) execution managers, their lifecycle
> states, warm-pool pre-spawning, and how the orchestrator uses them.


> **Implementation**: Shipping

---

## Two Pool Types

Roko provides two pool implementations for agent lifecycle management:

1. **`AgentPool`** (`crates/roko-agent/src/pool.rs`) — Manages a queue of
   tasks for a single agent role. Tasks execute sequentially. If the primary
   agent fails, the pool retries with a fallback agent (different model).

2. **`MultiAgentPool`** (`crates/roko-agent/src/multi_pool.rs`) — Manages
   multiple `AgentPool` instances across roles for concurrent execution.
   Supports warm-pool pre-spawning so agents are ready to accept work
   without cold-start latency.

---

## AgentInstanceId

Every agent instance gets a unique identifier:

```rust
pub struct AgentInstanceId {
    /// The role this instance fulfils.
    pub role: AgentRole,
    /// Human-readable instance discriminator (e.g. "plan42-task3").
    pub instance: String,
}
```

The `key()` method produces a string like `"implementer-plan42-task3"` for
use in logs, metrics, and the TUI dashboard. The `matches()` method supports
plan-based filtering for bulk operations (e.g., kill all agents working on
plan 42).

```rust
impl AgentInstanceId {
    pub fn key(&self) -> String {
        format!("{}-{}", self.role.label(), self.instance)
    }

    pub fn matches(&self, needle: &str) -> bool {
        self.key().contains(needle)
    }
}
```

---

## Instance Lifecycle

Each agent instance transitions through these states:

```rust
pub enum InstanceStatus {
    Warm,       // Pre-spawned, waiting for work
    Pending,    // Queued, waiting its turn
    Running,    // Currently executing
    Completed,  // Finished successfully
    Failed,     // Finished with error
    Killed,     // Terminated externally
}
```

The lifecycle flow:

```
Warm ──work-arrives──→ Pending ──turn-comes──→ Running
                                                  │
                                          ┌───────┴───────┐
                                          ▼               ▼
                                      Completed        Failed
                                                         │
                                                    ┌────┴────┐
                                                    ▼         ▼
                                              TryFallback   Killed
```

### Warm pool

The `MultiAgentPool` supports **warm-pool pre-spawning**: agents are
constructed and held in memory before work arrives, eliminating cold-start
latency. When a task arrives for a role that has a warm agent available,
the pool promotes the warm agent to active status instead of constructing
a new one.

```rust
struct WarmEntry {
    agent: Arc<dyn Agent>,
    spawned_at: Instant,
}
```

Warm entries have a time-to-live. `evict_stale_warm` removes entries that
have been idle longer than a configurable timeout (default: 5 minutes),
preventing memory waste for unused pre-spawned agents.

### Fallback retry

When an agent fails, the `AgentPool` checks if a fallback agent is
configured for the role. If so, it retries the same task with the fallback:

```
Primary (Opus) fails → Fallback (Sonnet) retries → Final result
```

This provides automatic model tier de-escalation: if the expensive model
fails (rate limit, timeout, context overflow), the cheaper model gets a
chance before the task is marked as failed.

---

## MultiAgentPool

The multi-pool manages concurrent execution across roles:

```rust
pub struct MultiAgentPool {
    active: HashMap<AgentInstanceId, ActiveEntry>,
    warm: HashMap<(AgentRole, String), WarmEntry>,
    fallbacks: HashMap<AgentRole, Arc<dyn Agent>>,
    concurrency_limits: HashMap<AgentRole, usize>,
    default_concurrency: usize,  // Default: 4
}
```

### Concurrency control

Each role can have its own concurrency limit. This prevents expensive roles
(like `Architect` at Premium tier) from consuming too many parallel slots
while allowing cheap roles (like `Validator` at Fast tier) to fan out:

```rust
pool.set_concurrency_limit(AgentRole::Architect, 1);   // Serial
pool.set_concurrency_limit(AgentRole::Implementer, 4); // Parallel
pool.set_concurrency_limit(AgentRole::Validator, 8);    // High parallelism
```

When a role hits its concurrency limit, new tasks are queued in `Pending`
status until a running instance completes.

### Bulk operations

The pool supports bulk lifecycle operations for plan management:

- **`kill_all()`** — Terminate all active instances (used on plan completion
  or Ctrl-C shutdown).
- **`kill_by_plan(plan_id)`** — Terminate all instances whose `AgentInstanceId`
  matches the plan (used when a plan fails and its agents should stop).
- **`kill_by_role(role)`** — Terminate all instances of a specific role.

These operations work through the `ProcessSupervisor` in `bardo-runtime`
for subprocess-based agents (Claude CLI, Codex) — the supervisor sends
SIGTERM and waits for graceful shutdown before escalating to SIGKILL.

---

## How the Orchestrator Uses Pools

The `PlanRunner` in `orchestrate.rs` manages agent execution through the
`AgentRunConfig` + `run_prepared_agent` flow. Currently it does not use
`MultiAgentPool` directly — instead, it constructs agents on-demand and
tracks them via the `ProcessSupervisor`.

The pool types (`AgentPool`, `MultiAgentPool`) are designed for the future
state where `orchestrate.rs` delegates all agent lifecycle to the pool layer:

```
Current:
  orchestrate.rs → AgentRunConfig → run_prepared_agent() → ClaudeCliAgent

Future:
  orchestrate.rs → MultiAgentPool.submit(role, task) → pool handles:
    → warm-pool promotion or cold-start construction
    → create_agent_for_model() via provider adapter
    → execution with timeout + cancellation
    → fallback retry on failure
    → lifecycle state tracking
    → bulk kill on plan completion
```

This migration is tracked as a Tier 1 integration priority.

---

## AgentTask

Tasks submitted to the pool carry their full specification:

```rust
pub struct AgentTask {
    pub id: AgentInstanceId,
    pub prompt: Signal,
    pub context: Context,
    pub priority: u32,
}
```

The `priority` field enables scheduling: higher-priority tasks (e.g.,
gate validation blocking the merge queue) preempt lower-priority tasks
(e.g., documentation generation).

### TaskOutcome

When a task completes, the pool produces a `TaskOutcome`:

```rust
pub enum TaskOutcome {
    Success(AgentResult),
    Failed(AgentResult),
    Cancelled,
}
```

The `AgentResult` inside `Failed` still contains the agent's output — even
failed runs produce diagnostic information that the orchestrator logs and
uses for retry decisions.

---

## Relationship to ProcessSupervisor

The `ProcessSupervisor` in `bardo-runtime` (`crates/bardo-runtime/`) handles
the low-level process lifecycle for subprocess-based agents:

- **Spawning** — Creates the child process with the correct environment
- **Monitoring** — Watches for exit codes, stdout/stderr
- **Shutdown** — Sends SIGTERM → waits → SIGKILL for graceful termination

The pool layer sits above the supervisor: it decides *when* to spawn and
*which model* to use; the supervisor handles *how* the process runs.

```
MultiAgentPool
    │
    ├── AgentPool (per role)
    │   ├── AgentInstanceId + status tracking
    │   └── fallback retry logic
    │
    ▼
create_agent_for_model() → Box<dyn Agent>
    │
    ▼
Agent::run() → AgentResult
    │
    ├── ClaudeCliAgent → ProcessSupervisor (subprocess)
    ├── OpenAiAgent → HTTP client (no supervisor needed)
    └── OllamaAgent → HTTP client (no supervisor needed)
```

---

## Citations

1. `crates/roko-agent/src/pool.rs` — AgentPool, AgentInstanceId,
   InstanceStatus, AgentTask, TaskOutcome.
2. `crates/roko-agent/src/multi_pool.rs` — MultiAgentPool, WarmEntry,
   ActiveEntry, concurrency control.
3. `crates/bardo-runtime/` — ProcessSupervisor for subprocess lifecycle.
4. `crates/roko-cli/src/orchestrate.rs:431` — AgentRunConfig struct.
5. Refactoring PRD §05-agent-types — Agent role compositions.
