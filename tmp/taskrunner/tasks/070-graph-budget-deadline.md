# Task 070: Graph Engine — Budget and Deadline Enforcement

```toml
id = 70
title = "Add budget tracking and deadline enforcement to Graph Engine execution"
track = "graph-engine"
wave = "wave-3"
priority = "medium"
blocked_by = [68]
touches = [
    "crates/roko-graph/src/lib.rs",
    "crates/roko-graph/src/engine.rs",
    "crates/roko-graph/src/types.rs",
    "crates/roko-graph/src/budget.rs",
    "crates/roko-graph/tests/budget_deadline.rs",
    "examples/graphs/budget-limited.toml",
    "examples/graphs/deadline-limited.toml",
]
exclusive_files = [
    "crates/roko-graph/src/budget.rs",
    "crates/roko-graph/tests/budget_deadline.rs",
    "examples/graphs/budget-limited.toml",
    "examples/graphs/deadline-limited.toml",
]
estimated_minutes = 180
```

## Context

This task adds budget enforcement (P2-13) and deadline enforcement (P2-14) from Phase 2C.
These are the two GraphPolicy constraints that make the Engine safe for production use — without
them, a runaway graph could burn unlimited LLM credits or hang indefinitely.

Both features modify the Engine's execution loop to check constraints before and after each
node execution. Budget tracking uses `Cell::estimated_cost()` (already on the trait) plus
actual cost from execution. Deadline enforcement uses `tokio::time::timeout`.

This task is independent from task 069 (sub-graphs/snapshots) — they can be worked in parallel
since they touch different Engine concerns. Both depend on task 068 (fan-out engine) because
budget/deadline must work with parallel execution.

## Background

Read these files before writing any code:

1. `tmp/v2-refactoring/07-GRAPH-ENGINE.md` — GraphPolicy struct with max_budget, deadline,
   max_parallelism, failure_strategy
2. `crates/roko-graph/src/engine.rs` — Current Engine. You will add budget/deadline checks
   into the execution loop.
3. `crates/roko-graph/src/types.rs` — GraphPolicy already has max_budget and deadline fields.
   They are defined but not enforced.
4. `crates/roko-core/src/cell.rs` — Cell trait has `estimated_cost() -> Option<f64>` and
   `estimated_duration() -> Option<Duration>`. Use these for pre-execution checks.
5. `crates/roko-runtime/src/run_ledger.rs` — RunLedger tracks per-task costs. Check if this
   can be reused or if a simpler approach is better for the Engine.
6. `crates/roko-cli/src/runner/event_loop.rs` — How Runner v2 handles budget/deadline
   (if it does). Don't duplicate, but learn from the pattern.

## What to Change

### 1. Create budget tracker in `budget.rs`

```rust
#[derive(Debug, Clone)]
pub struct BudgetTracker {
    max_budget: Option<f64>,
    spent: AtomicF64,  // or use Mutex<f64> if AtomicF64 isn't available
}
```

Note: `AtomicF64` does not exist in std. Use `Arc<Mutex<f64>>` or `AtomicU64` with
`f64::to_bits()`/`f64::from_bits()` for lock-free tracking.

Methods:
- `new(max_budget: Option<f64>) -> Self`
- `record_cost(&self, cost: f64)` — add to spent total
- `total_spent(&self) -> f64`
- `remaining(&self) -> Option<f64>` — None if no budget limit
- `is_exceeded(&self) -> bool`
- `check(&self) -> Result<()>` — returns error if exceeded

### 2. Wire budget enforcement into Engine execution

In `execute_graph`, before each node:

```rust
// Pre-execution budget check
if let Some(estimated) = cell.estimated_cost() {
    if let Some(remaining) = budget.remaining() {
        if estimated > remaining {
            return Err(anyhow!(
                "Node '{}' estimated cost ${:.4} exceeds remaining budget ${:.4}",
                node_id, estimated, remaining
            ));
        }
    }
}
```

After each node:

```rust
// Post-execution cost recording
// If the cell returns cost info in its output signals, extract it.
// Otherwise, use the estimated_cost as actual.
let actual_cost = extract_cost_from_output(&output)
    .or_else(|| cell.estimated_cost())
    .unwrap_or(0.0);
budget.record_cost(actual_cost);
budget.check()?;
```

When budget is exceeded:
- If `failure_strategy` is `FailFast`: cancel the flow immediately
- If `failure_strategy` is `ContinueOnFailure`: skip remaining Activity (non-deterministic)
  nodes but allow Workflow (deterministic) nodes to complete
- Log the budget exhaustion with total spent and limit

### 3. Wire deadline enforcement into Engine execution

Wrap the entire `execute_graph` call in a `tokio::time::timeout`:

```rust
pub async fn start(&self, graph: &Graph, input: Vec<Signal>) -> Result<FlowId> {
    let deadline = graph.policy.deadline;
    let exec_future = self.execute_graph(graph.clone(), input, ctx);

    let handle = tokio::spawn(async move {
        match deadline {
            Some(d) => match tokio::time::timeout(d, exec_future).await {
                Ok(result) => result,
                Err(_) => Err(anyhow!("Graph '{}' exceeded deadline of {:?}", graph.name, d)),
            },
            None => exec_future.await,
        }
    });
    // ...
}
```

Additionally, use the `CancellationToken` so that when deadline fires, all in-progress cells
receive cancellation. Check `ctx.cancel.is_cancelled()` between node executions.

### 4. Add budget/deadline info to CLI output

When a graph completes (or is cancelled), print a summary:

```
Graph 'my-graph' completed in 12.3s
  Budget: $0.0234 / $0.05 (46.8% used)
  Nodes: 5/5 completed
```

When budget/deadline cancels a graph:

```
Graph 'my-graph' CANCELLED: budget exceeded ($0.052 / $0.05)
  Completed: 3/5 nodes
  Last completed: "agent-step-2"
```

### 5. Write example graphs

**`examples/graphs/budget-limited.toml`**:
```toml
[graph]
name = "budget-test"
version = "0.1.0"

# ... nodes ...

[graph.policy]
max_budget = 0.0001
failure_strategy = "FailFast"
```

**`examples/graphs/deadline-limited.toml`**:
```toml
[graph]
name = "deadline-test"
version = "0.1.0"

# ... nodes ...

[graph.policy]
deadline = "1ms"
failure_strategy = "FailFast"
```

Note: deadline in TOML needs a serde deserializer for human-readable duration. Implement a
small local parser; support `"1ms"`, `"30s"`, `"5m"`, and `"1h"`.

### 6. Add tests

- Unit test: `BudgetTracker` records costs and detects exceeded budget
- Unit test: `BudgetTracker` with no limit never exceeds
- Integration test: run a graph with a very low budget ($0.0001), verify it cancels
- Integration test: run a graph with a very short deadline (1ms), verify it times out
- Integration test: run a graph with generous budget/deadline, verify it completes normally

## What NOT to Do

- Do NOT implement cost estimation based on LLM token counts. Use `Cell::estimated_cost()` only.
- Do NOT implement dynamic budget adjustment or budget negotiation between nodes.
- Do NOT add a billing system or cost persistence beyond the flow snapshot.
- Do NOT break existing graphs that have no budget/deadline set — they must still work
  with unlimited defaults.
- Do NOT implement per-node timeouts (only graph-level deadline). Per-node timeouts can be
  a follow-up.
- Do NOT add external dependencies for atomic floats — use `Mutex<f64>` or bit-casting.

## Wire Target

```bash
# Budget enforcement
cargo run -p roko-cli -- graph run examples/graphs/budget-limited.toml
# Expected: Prints CANCELLED for budget exhaustion and exits non-zero

# Deadline enforcement
cargo run -p roko-cli -- graph run examples/graphs/deadline-limited.toml
# Expected: Prints CANCELLED for deadline timeout and exits non-zero

# Existing graphs still work (no budget/deadline = unlimited)
cargo run -p roko-cli -- graph run examples/graphs/linear-gates.toml
# Expected: Works exactly as before
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- graph run examples/graphs/budget-limited.toml` — budget enforced
- [ ] `cargo run -p roko-cli -- graph run examples/graphs/deadline-limited.toml` — deadline enforced
- [ ] Existing example graphs from tasks 067-069 still work unchanged
- [ ] CLI output includes budget/time summary after graph completion
- [ ] Unit tests for BudgetTracker pass
- [ ] Integration tests for budget cancellation pass
- [ ] Integration tests for deadline timeout pass
- [ ] `grep -rn 'BudgetTracker\|budget' crates/roko-graph/src/engine.rs` — budget wired into execution
- [ ] `grep -rn 'timeout\|deadline' crates/roko-graph/src/engine.rs` — deadline wired into execution
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file

## Implementation Detail

### Current source facts to account for

- `Cell::estimated_cost()` and `Cell::estimated_duration()` are metadata methods on
  `roko_core::Cell`; existing gates generally inherit the default `None`. Do not modify gate
  crates in this task.
- `roko_runtime::run_ledger::RunLedger` records runner-level costs but is not wired into the graph
  engine. Use it as background only; implement a graph-local `BudgetTracker`.
- Runner v2 enforces plan deadlines with a fixed `tokio::time::sleep_until` branch and cancellation
  token. The graph engine can use the simpler task-level `tokio::time::timeout`, but it must cancel
  the graph token on timeout so in-flight cells see cancellation.
- If task 069 has already landed, update snapshot/status summaries with budget/deadline outcomes.
  If it has not, keep the budget/deadline state in `FlowStatus` and avoid depending on snapshot
  APIs.

### Mechanical implementation steps

1. `budget.rs`:
   - Implement `BudgetTracker` with `max_budget: Option<f64>` and `spent: Arc<std::sync::Mutex<f64>>`.
   - `record_cost(cost)` should ignore negative values by recording `cost.max(0.0)`.
   - `remaining()` returns `None` for unlimited budget and otherwise `Some((max - spent).max(0.0))`.
   - `is_exceeded()` is true only when `spent > max_budget`.
   - `check()` returns an `anyhow::Error` that includes spent and limit.
   - Add `pub mod budget;` and `pub use budget::BudgetTracker;` in `lib.rs`.

2. Duration serde in `types.rs`:
   - Add custom serde for `GraphPolicy.deadline: Option<Duration>` that accepts TOML strings
     `"1ms"`, `"30s"`, `"5m"`, and `"1h"`. Also accept integer seconds if TOML provides a number.
   - Serialize deadlines back to a readable string.
   - Do not add `humantime-serde`; a small local parser is enough for the required units.
   - Add validation that `max_budget` is non-negative when present.

3. Engine budget enforcement:
   - Create one `BudgetTracker` per flow from `graph.policy.max_budget`.
   - Before spawning/executing a topo level, compute positive estimated costs for all runnable
     nodes in that level. If the sum exceeds remaining budget:
     - `FailFast` and `Retry(_)`: cancel/fail the flow before spawning any node in that level.
     - `ContinueOnFailure`: skip `ExecutionClass::Activity` nodes until the estimated level cost
       fits; deterministic `Workflow` nodes may continue if their estimated cost fits.
   - Immediately before each node calls `cell.execute()`, set
     `ctx.budget_remaining = budget.remaining()` if the landed `CellContext` exposes that field,
     then call `budget.check()`.
   - After each node returns, compute actual cost with `extract_cost_from_output(&output)`, falling
     back to `cell.estimated_cost().unwrap_or(0.0)`, then `record_cost()` and `check()`.
   - `extract_cost_from_output` should look for numeric cost fields in this order:
     signal tag `"cost_usd"`, JSON body `cost_usd`, JSON body `total_cost_usd`, and JSON body
     `usage.cost_usd`.
   - For parallel levels from task 068, record costs after joins in deterministic node-id order so
     tests can assert totals.

4. Engine deadline enforcement:
   - Wrap the whole graph execution future in `tokio::time::timeout(graph.policy.deadline, ...)`
     inside `Engine::start()`/resume start, not around individual nodes.
   - On timeout, call the flow's `CancellationToken::cancel()`, update status to a cancelled/failed
     deadline state, and return an error containing graph name and configured deadline.
   - Check `ctx.cancel.is_cancelled()` between node executions and before spawning each level so
     cooperative cancellation stops promptly.
   - Do not add per-node timeouts in this task.

5. Flow status and CLI summary:
   - Extend `FlowStatus` or add a small `FlowReport` so the CLI can read:
     completed node count, total node count, elapsed duration, budget spent, and budget limit.
   - On success, print:
     `Graph '<name>' completed in <seconds>s`, `Budget: $spent / $limit` or `Budget: unlimited`,
     and `Nodes: completed/total completed`.
   - On budget/deadline cancellation, print `Graph '<name>' CANCELLED: <reason>` plus completed
     nodes and budget spent before returning a non-zero error.
   - Existing graphs with no budget/deadline must produce the same successful behavior as task 067.

6. Deterministic example cells for wire targets:
   - Add two tiny graph-local cells next to task-067's `NoopCell`, and register them in
     `build_default_registry()`:
     - `"expensive-noop"`: pass-through cell with `estimated_cost() -> Some(0.01)`.
     - `"slow-noop"`: pass-through cell whose `execute()` sleeps for 50ms and whose
       `estimated_duration() -> Some(Duration::from_millis(50))`.
   - These are allowed in this task only to make budget/deadline CLI examples deterministic. Do not
     put them in `roko-core` or `roko-std`, and do not implement AgentCell/ComposeCell.
   - `budget-limited.toml` should use `expensive-noop` with `max_budget = 0.0001` so the command
     cancels before executing or immediately after pre-check.
   - `deadline-limited.toml` should use `slow-noop` with `deadline = "1ms"` so the command times
     out deterministically.

### Tests to add

- `budget.rs` unit tests:
  - no-limit tracker never exceeds;
  - recording costs accumulates;
  - spent greater than limit makes `check()` fail;
  - negative costs do not reduce spent.
- Duration serde tests in `types.rs` or loader tests for `"1ms"`, `"30s"`, `"5m"`, `"1h"`, and a
  malformed duration error.
- `crates/roko-graph/tests/budget_deadline.rs`:
  - Low-budget graph with an expensive test cell returns an error before downstream nodes run.
  - Generous-budget graph completes and reports spent amount.
  - Short-deadline graph with a slow test cell times out and cancels.
  - No-budget/no-deadline linear graph from task 067 still completes.

### Verification command details

```bash
cargo run -p roko-cli -- graph run examples/graphs/linear-gates.toml

# These two are expected to print CANCELLED and exit non-zero.
cargo run -p roko-cli -- graph run examples/graphs/budget-limited.toml
cargo run -p roko-cli -- graph run examples/graphs/deadline-limited.toml
```

### Anti-patterns specific to this task

- Do not estimate costs from token counts or provider pricing tables.
- Do not let parallel nodes all pass pre-check independently when their combined estimated cost
  exceeds the remaining budget.
- Do not treat missing cost estimates as an error; they are zero for this task.
- Do not retry budget or deadline failures under `FailureStrategy::Retry(_)`; they are flow-level
  constraints, not transient node failures.
- Do not make budget/deadline fields required in TOML.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
