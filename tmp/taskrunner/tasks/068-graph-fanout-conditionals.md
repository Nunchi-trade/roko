# Task 068: Graph Engine — Fan-Out/Fan-In and Conditional Edges

```toml
id = 68
title = "Add fan-out/fan-in parallel execution and conditional edge evaluation to Graph Engine"
track = "graph-engine"
wave = "wave-3"
priority = "medium"
blocked_by = [67]
touches = [
    "crates/roko-graph/src/lib.rs",
    "crates/roko-graph/src/engine.rs",
    "crates/roko-graph/src/types.rs",
    "crates/roko-graph/src/condition.rs",
    "crates/roko-graph/tests/fanout_condition.rs",
    "examples/graphs/parallel-gates.toml",
    "examples/graphs/conditional-branch.toml",
]
exclusive_files = [
    "crates/roko-graph/src/condition.rs",
    "crates/roko-graph/tests/fanout_condition.rs",
    "examples/graphs/parallel-gates.toml",
    "examples/graphs/conditional-branch.toml",
]
estimated_minutes = 300
```

## Context

This task adds the first two Engine features from Phase 2C: fan-out/fan-in (P2-9) and
conditional edges (P2-10). These are the two features that make graphs more than linear
pipelines. Without them, the Engine is just a sequential list executor.

Fan-out: a node with multiple outgoing edges runs its successors concurrently (up to
`GraphPolicy.max_parallelism`). Fan-in: a node with multiple incoming edges waits for all
predecessors before executing. The topo sort already handles ordering — this task adds the
concurrent execution within a topo level.

Conditional edges: an edge with a `condition` expression is only followed if the expression
evaluates to true against the predecessor's output signals. This enables branching (e.g.,
gate pass -> continue, gate fail -> retry path).

## Background

Read these files before writing any code:

1. `tmp/v2-refactoring/07-GRAPH-ENGINE.md` — Fan-out/fan-in design, condition expression
   format, the Mapping enum for signal transformation on edges
2. `crates/roko-graph/src/engine.rs` — Current sequential Engine from task 067. You will
   modify `execute_graph` to support parallelism and conditions.
3. `crates/roko-graph/src/types.rs` — Edge struct has `condition: Option<String>` and
   `mapping: Option<Mapping>`. These fields exist but are ignored by the sequential engine.
4. `crates/roko-core/src/signal.rs` (or wherever Signal is defined) — Signal structure.
   Condition expressions reference signal fields.

## What to Change

### 1. Implement condition evaluation in `condition.rs`

Create `crates/roko-graph/src/condition.rs` with:

```rust
pub fn evaluate_condition(expr: &str, signals: &[Signal]) -> Result<bool>
```

Start with a **minimal expression language** — do NOT build a full expression parser. Support:

- `"true"` / `"false"` — literal booleans
- `"verdict.hard_pass == true"` — dot-path field access on signal body (JSON) with equality
- `"kind == \"gate-result\""` — signal kind comparison
- `"any_pass"` — returns true if any input signal has `verdict.hard_pass == true`
- `"all_pass"` — returns true if all input signals have `verdict.hard_pass == true`

Implementation approach: parse the expression as a simple pattern match, not a recursive
descent parser. Use `serde_json::Value` for field access via dot paths. If the expression
does not match any supported pattern, return an error with the unrecognized expression text.

### 2. Add parallel execution to Engine

Modify `execute_graph` in `engine.rs`:

1. Group nodes by topo level (nodes at the same depth in the DAG can run concurrently)
2. Within each level, spawn nodes concurrently using `tokio::JoinSet` or `futures::join_all`
3. Respect `GraphPolicy.max_parallelism` — use a `tokio::sync::Semaphore` to limit concurrency
4. Wait for all nodes in a level to complete before proceeding to the next level

The topo sort from task 066 returns a flat list. You need to derive levels from it:
- Level 0: nodes with no incoming edges (entry nodes)
- Level N: nodes whose predecessors are all in levels < N

Add a helper: `fn topo_levels(nodes: &[Node], edges: &[Edge]) -> Result<Vec<Vec<NodeId>>>`

### 3. Wire condition evaluation into edge traversal

In `execute_graph`, when collecting inputs for a node from its incoming edges:

```rust
let inputs = graph.incoming_edges(node_id)
    .iter()
    .filter(|edge| {
        match &edge.condition {
            None => true,  // unconditional edge: always follow
            Some(expr) => {
                let predecessor_outputs = node_outputs.get(&edge.from)
                    .cloned().unwrap_or_default();
                evaluate_condition(expr, &predecessor_outputs)
                    .unwrap_or(false)  // on eval error, skip edge (log warning)
            }
        }
    })
    .flat_map(|edge| /* collect signals */)
    .collect();
```

If ALL incoming edges to a node are conditional and ALL evaluate to false, skip the node
entirely (it has no inputs to process). Log this as a debug message.

### 4. Wire Mapping application on edges

When signals flow along an edge, apply the `Mapping` if present:

- `Mapping::Identity` — pass all signals unchanged (default)
- `Mapping::FilterKind(kinds)` — only pass signals whose `kind` is in the list
- `Mapping::Project(fields)` — for each signal, extract only the named fields from its body

Implement `fn apply_mapping(mapping: &Option<Mapping>, signals: Vec<Signal>) -> Vec<Signal>`

### 5. Write example graphs

**`examples/graphs/parallel-gates.toml`** — A graph where 3 gates run in parallel after a
single entry node, then fan into a single exit node:

```
entry -> compile-gate  \
entry -> clippy-gate    -> exit
entry -> test-gate     /
```

**`examples/graphs/conditional-branch.toml`** — A graph where a gate's output determines the
next step:

```
compile -> test (condition: "verdict.hard_pass == true")
compile -> report-failure (condition: "verdict.hard_pass == false")
```

Use a NoopCell for `report-failure` if no failure-reporting cell exists.

### 6. Add tests

- Unit test: `evaluate_condition("true", &[])` returns true
- Unit test: `evaluate_condition("verdict.hard_pass == true", &signals)` with pass/fail signals
- Unit test: `topo_levels` returns correct level groupings for a diamond DAG
- Integration test: run `parallel-gates.toml` and verify all 3 gates execute
- Integration test: run `conditional-branch.toml` and verify only the matching branch executes

## What NOT to Do

- Do NOT build a full expression parser (yacc, pest, nom, etc.). Simple pattern matching only.
- Do NOT implement sub-graph execution. That is task 069.
- Do NOT implement snapshots/resume. That is task 069.
- Do NOT implement budget/deadline enforcement. That is task 070.
- Do NOT add new Cell implementations (AgentCell, etc.). That is task 071.
- Do NOT break the existing sequential execution path — it should still work for linear graphs.
- Do NOT add `unsafe` code for the semaphore or parallelism.

## Wire Target

```bash
cargo run -p roko-cli -- graph run examples/graphs/parallel-gates.toml
# Expected: All 3 gates run (potentially concurrently), results printed

cargo run -p roko-cli -- graph run examples/graphs/conditional-branch.toml
# Expected: Only the branch matching the gate verdict executes
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo run -p roko-cli -- graph run examples/graphs/parallel-gates.toml` — all 3 gates execute
- [ ] `cargo run -p roko-cli -- graph run examples/graphs/conditional-branch.toml` — correct branch executes
- [ ] Existing linear graph examples from task 067 still work unchanged
- [ ] Unit tests for condition evaluation pass
- [ ] Unit tests for topo level grouping pass
- [ ] `grep -rn 'evaluate_condition' crates/roko-graph/ --include='*.rs' | grep -v target/` — called from engine
- [ ] `grep -rn 'topo_levels\|JoinSet\|Semaphore' crates/roko-graph/ --include='*.rs' | grep -v target/` — parallel execution wired
- [ ] No `TODO`, `FIXME`, or `unimplemented!()` in any file

## Implementation Detail

### Current source facts to account for

- `Signal` is currently `roko_core::Engram`. It has `kind: Kind` and `body: Body`; it does not have
  a `topic` field. Compare kind values with `signal.kind.as_str()`.
- `Body` is an enum. Condition field access must inspect only `Body::Json(value)`; text/bytes/empty
  bodies do not contain dot-path fields.
- Gate verdict JSON from task 036 should be a serialized `roko_core::Verdict` with `passed`, not
  `verdict.hard_pass`. For compatibility with the design docs, support both `hard_pass` and
  `passed` as aliases when the path is `verdict.hard_pass`.
- Add `pub mod condition;` and useful re-exports in `crates/roko-graph/src/lib.rs`; otherwise the
  new sibling file will not compile or be testable.

### Mechanical implementation steps

1. `condition.rs`:
   - Implement `pub fn evaluate_condition(expr: &str, signals: &[Signal]) -> Result<bool>`.
   - Trim `expr`; support exact literals `true` and `false`.
   - Support `any_pass`: true if any input signal's JSON body has `verdict.hard_pass == true`,
     `hard_pass == true`, or `passed == true`.
   - Support `all_pass`: true only when `signals` is non-empty and every input signal has one of
     those pass fields set to true.
   - Support `kind == "..."`: true if any input signal's `kind.as_str()` matches after normalizing
     hyphens to underscores. Accept `"gate-result"` and `"gate_verdict"` for `Kind::GateVerdict`.
   - Support `<dot.path> == true|false|"string"` equality against any JSON body. Dot paths should
     walk `serde_json::Value::Object`; missing fields are non-matches.
   - Return an error for unsupported expressions and include the original expression text.

2. Mapping helpers:
   - Implement `pub fn apply_mapping(mapping: &Option<Mapping>, signals: Vec<Signal>) -> Vec<Signal>`
     in `condition.rs` or `engine.rs`.
   - `None` and `Some(Mapping::Identity)` return the input unchanged.
   - `Mapping::FilterKind(kinds)` keeps signals where `kinds.contains(&signal.kind)`.
   - `Mapping::Project(fields)` keeps the signal metadata but replaces the body with a JSON object
     containing only requested top-level or dot-path fields. After changing a signal body, recompute
     `signal.id = signal.content_hash()` so persisted identity stays correct.

3. `topo_levels`:
   - Add `pub(crate) fn topo_levels(nodes: &[Node], edges: &[Edge]) -> Result<Vec<Vec<NodeId>>>`.
   - Compute levels from `topological_sort`: `depth(node) = max(depth(pred) + 1)`, or 0 for nodes
     without predecessors.
   - Bucket by depth with `BTreeMap<usize, Vec<NodeId>>` and sort each level lexically for stable
     execution and tests.
   - Return an error if an edge references a missing node, matching task 066 validation behavior.

4. Parallel execution:
   - Process one topo level at a time. Nodes inside a level can run concurrently; all nodes in the
     level must finish before the next level starts.
   - Use `tokio::task::JoinSet` plus `tokio::sync::Semaphore`. Treat `policy.max_parallelism = None`
     as unlimited and `Some(n)` as the limit; `Some(0)` should already be rejected by
     `Graph::validate()`.
   - Do not share a mutable `node_outputs` map across spawned tasks. Build each node's input before
     spawning, spawn with cloned `Arc<dyn Cell>`, cloned inputs, cloned context/cancellation token,
     then insert outputs into `node_outputs` after joins complete.
   - Join results are nondeterministic. Sort completed `(node_id, outputs)` by `node_id` before
     inserting if any later output order depends on map iteration.
   - If a non-entry node has no active inputs because every conditional incoming edge evaluated
     false, skip the node and record no output for it. Log this at debug level.

5. Edge traversal semantics:
   - For each incoming edge, evaluate its condition against the predecessor's output signals.
   - If condition evaluation returns Err, log a warning and treat that edge as inactive. Unit tests
     should still assert the evaluator returns Err directly for unsupported expressions.
   - Apply mapping after a condition passes and before appending signals to the target node input.
   - Preserve the task-067 behavior for linear graphs with unconditional edges.

6. Example graphs:
   - `parallel-gates.toml` should use a deterministic `noop` entry and `noop` fan-in/exit node so
     graph structure is testable even when local compile/test gates fail.
   - `conditional-branch.toml` should include one condition using `passed == true` or
     `verdict.hard_pass == true` and one using false. Use `noop` for the failure-reporting branch.

### Tests to add

- `condition.rs` unit tests for literal booleans, kind equality, `any_pass`, `all_pass`, bool
  equality, string equality, and unsupported-expression errors.
- `engine.rs` unit tests for `topo_levels` on a diamond DAG.
- `crates/roko-graph/tests/fanout_condition.rs`:
  - Use test cells that record their node names into an `Arc<Mutex<Vec<String>>>`.
  - Verify a diamond graph executes both parallel branches and then the fan-in node.
  - Verify a conditional graph executes only the matching branch.
  - Verify `max_parallelism = 1` still runs all nodes but serializes concurrent levels.

### Anti-patterns specific to this task

- Do not add a parser crate (`nom`, `pest`, etc.) or recursive expression parser.
- Do not evaluate conditions against stringified JSON; use `serde_json::Value` access.
- Do not let one condition parse failure abort the whole graph in this task; inactive edge plus
  warning is the intended engine behavior.
- Do not hold a `Mutex` guard or `Semaphore` permit across unrelated awaits.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
