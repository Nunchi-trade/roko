# Defense in Depth as Pipeline

> Depth for [16-SECURITY.md](../../unified/16-SECURITY.md). Re-derives the 7-layer defense model as a Pipeline Graph of Verify Cells. Each layer is a Cell conforming to the Verify protocol with early-exit semantics. Addresses the critical integration gap where SafetyLayer is wired in routed paths but not universally from all execution branches.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, taint), [02-CELL](../../unified/02-CELL.md) (Cell, Verify protocol, Pipeline pattern), [03-GRAPH](../../unified/03-GRAPH.md) (Graph wiring, TOML definition), [16-SECURITY](../../unified/16-SECURITY.md) (capability intersection, 5-head corrigibility, sandboxing)

---

## 1. The Core Insight

Every safety check in Roko -- path validation, rate limiting, secret scrubbing, permission verification, sandboxing, taint inspection, corrigibility ordering -- is structurally identical: it receives a proposed action as a Signal, evaluates it against a criterion, and emits a Verdict. That is the Verify protocol. See [02-CELL.md](../../unified/02-CELL.md) SS3 for the protocol definition.

The defense-in-depth model is therefore not seven ad-hoc subsystems stitched together with glue code. It is a **Pipeline Graph of seven Verify Cells**, where each Cell has early-exit semantics: any Cell can reject, and rejection terminates the Pipeline before execution begins. The Pipeline processes a `ProposedAction` Signal and emits either an `ExecutionPermit` Signal (all layers passed) or a `Rejection` Signal (some layer rejected).

This framing eliminates the "god file" anti-pattern where safety checks are scattered across multiple code paths. It also makes defense ordering explicit and auditable -- the Pipeline TOML definition is a runtime artifact that agents and operators can inspect.

---

## 2. The Seven Verify Cells

Each layer maps to a Cell with a distinct `Criterion` type. The layers are ordered from cheapest/fastest to most expensive, following the principle that cheap checks should filter before expensive ones run.

| Layer | Cell ID | Protocol | Criterion Type | What It Checks | Cost |
|---|---|---|---|---|---|
| 1 | `path-sandbox` | Verify | `PathCriterion` | Filesystem access within authorized worktree; symlink resolution; escape prevention | O(1) string ops |
| 2 | `rate-limiter` | Verify | `RateCriterion` | Sliding-window counter per (role, tool) pair; burst protection | O(1) counter check |
| 3 | `permission-gate` | Verify | `CapabilityCriterion` | Three-layer capability intersection: Cell declaration, Graph allow-list, Space grant | O(k) intersection |
| 4 | `policy-chain` | Verify | `PolicyCriterion` | Content validation: BashPolicy, GitPolicy, NetworkPolicy deny patterns | O(n) regex matching |
| 5 | `taint-barrier` | Verify | `TaintCriterion` | Taint lattice check: reject or escalate when tainted data flows to high-risk destinations | O(1) lattice compare |
| 6 | `corrigibility-chain` | Verify | `CorrigibilityCriterion` | 5-head lexicographic ordering: deference > switch > truth > impact > task | O(5) head evaluation |
| 7 | `contract-bounds` | Verify | `ContractCriterion` | Agent contract limits: max files modified, max lines changed, cost cap, duration cap | O(1) bound checks |

Post-execution, a separate Pipeline handles output validation:

| Post-Layer | Cell ID | Protocol | What It Checks |
|---|---|---|---|
| P1 | `output-truncate` | Verify | Output size within `max_result_bytes` |
| P2 | `secret-scrub` | Verify | Regex-based secret redaction (9 default patterns + custom) |
| P3 | `taint-assign` | Verify | Assign taint to output Signal based on input provenance |
| P4 | `audit-emit` | Verify | Emit SecurityEvent Signal to Store and Pulse to Bus |

---

## 3. The Pipeline Graph Definition

```toml
# Graph: defense-pipeline
# Seven Verify Cells in a linear Pipeline with early-exit semantics.
# Any Cell can reject, terminating the Pipeline before execution.
#
# Signal flow:
#   proposed-action -> path-sandbox -> rate-limiter -> permission-gate
#     -> policy-chain -> taint-barrier -> corrigibility-chain
#     -> contract-bounds -> [EXECUTE] -> output-truncate
#     -> secret-scrub -> taint-assign -> audit-emit -> result

[graph]
id = "defense-pipeline"
description = "7-layer defense-in-depth as Pipeline of Verify Cells"
pattern = "Pipeline"

# --- Pre-execution layers (reject before execution) ---

[[graph.cells]]
id = "path-sandbox"
protocol = "Verify"
description = "Layer 1: filesystem sandbox - worktree boundaries, path canonicalization"
config = { worktree = "${SPACE_ROOT}", deny_symlinks = false, deny_absolute = true }

[[graph.cells]]
id = "rate-limiter"
protocol = "Verify"
description = "Layer 2: sliding-window rate limiter per (role, tool) pair"
config = { max_calls_per_window = 60, window_secs = 60 }

[[graph.cells]]
id = "permission-gate"
protocol = "Verify"
description = "Layer 3: three-layer capability intersection"

[[graph.cells]]
id = "policy-chain"
protocol = "Verify"
description = "Layer 4: content validation (BashPolicy, GitPolicy, NetworkPolicy)"

[[graph.cells]]
id = "taint-barrier"
protocol = "Verify"
description = "Layer 5: taint lattice check - escalate on tainted data to high-risk targets"

[[graph.cells]]
id = "corrigibility-chain"
protocol = "Verify"
description = "Layer 6: 5-head lexicographic corrigibility ordering"

[[graph.cells]]
id = "contract-bounds"
protocol = "Verify"
description = "Layer 7: agent contract bounds (files, lines, cost, duration)"

# --- Post-execution layers (validate output) ---

[[graph.cells]]
id = "output-truncate"
protocol = "Verify"
description = "Post-1: enforce output size limits"

[[graph.cells]]
id = "secret-scrub"
protocol = "Verify"
description = "Post-2: regex-based secret redaction"

[[graph.cells]]
id = "taint-assign"
protocol = "Verify"
description = "Post-3: assign output taint from input provenance"

[[graph.cells]]
id = "audit-emit"
protocol = "Verify"
description = "Post-4: emit SecurityEvent Signal for audit trail"

# --- Edges: linear Pipeline ---

[[graph.edges]]
from = "path-sandbox.out"
to = "rate-limiter.in"

[[graph.edges]]
from = "rate-limiter.out"
to = "permission-gate.in"

[[graph.edges]]
from = "permission-gate.out"
to = "policy-chain.in"

[[graph.edges]]
from = "policy-chain.out"
to = "taint-barrier.in"

[[graph.edges]]
from = "taint-barrier.out"
to = "corrigibility-chain.in"

[[graph.edges]]
from = "corrigibility-chain.out"
to = "contract-bounds.in"

# execution boundary

[[graph.edges]]
from = "contract-bounds.out"
to = "output-truncate.in"

[[graph.edges]]
from = "output-truncate.out"
to = "secret-scrub.in"

[[graph.edges]]
from = "secret-scrub.out"
to = "taint-assign.in"

[[graph.edges]]
from = "taint-assign.out"
to = "audit-emit.in"
```

---

## 4. Early-Exit Semantics

The Pipeline pattern has a critical property: **any Cell can reject, and rejection terminates the chain**. This is not a soft vote. Rejection is absolute. A Signal that fails the path sandbox never reaches the rate limiter. A Signal that exceeds the rate limit never reaches permission checks. This ordering is intentional -- cheap failures are caught cheaply.

```rust
/// Execute the defense Pipeline. Each Cell is a Verify Cell.
/// Early exit on first rejection.
pub async fn execute_defense_pipeline(
    action: Signal,     // Kind::ProposedAction
    pipeline: &[Box<dyn VerifyCell>],
    ctx: &CellContext,
) -> Result<Signal, Rejection> {
    let mut current = action;

    for cell in pipeline {
        let verdict = cell.verify(&current, ctx).await?;

        match verdict {
            Verdict::Pass { reward, evidence } => {
                // Annotate the Signal with pass evidence
                current.metadata.push_evidence(cell.name(), evidence);
                current.metadata.push_reward(cell.name(), reward);

                // Emit pass Pulse for telemetry
                ctx.bus().publish(Pulse::new(
                    topic!("safety.pipeline.pass"),
                    PipelinePassEvent {
                        cell: cell.name().to_string(),
                        action_hash: current.hash(),
                        reward,
                    },
                ));
            }
            Verdict::Reject { reason, head, evidence } => {
                // Emit rejection Signal (persisted to Store)
                let rejection = Signal::new(Kind::SecurityEvent, SecurityEvent::Rejection {
                    cell: cell.name().to_string(),
                    action_hash: current.hash(),
                    reason: reason.clone(),
                    head,
                    evidence,
                });
                ctx.store().put(&rejection).await?;

                // Emit rejection Pulse for telemetry
                ctx.bus().publish(Pulse::new(
                    topic!("safety.pipeline.reject"),
                    PipelineRejectEvent {
                        cell: cell.name().to_string(),
                        action_hash: current.hash(),
                        reason,
                    },
                ));

                return Err(Rejection {
                    cell: cell.name().to_string(),
                    reason,
                    head,
                });
            }
        }
    }

    // All layers passed: emit ExecutionPermit
    Ok(Signal::new(Kind::ExecutionPermit, ExecutionPermit {
        action_hash: current.hash(),
        layers_passed: pipeline.iter().map(|c| c.name().to_string()).collect(),
        effective_capabilities: ctx.effective_capabilities().clone(),
    }))
}
```

---

## 5. Every Safety Check Is a Verify Cell

The existing safety guards in `crates/roko-agent/src/safety/` are already structurally compatible with the Verify protocol. Expressing them as Verify Cells is not a refactor -- it is a relabeling that makes the composition explicit.

### PathPolicy as Verify Cell

```rust
/// Layer 1: Path sandbox. Verifies that filesystem access stays within
/// the authorized worktree boundary.
pub struct PathSandboxCell {
    worktree: PathBuf,
    deny_symlinks: bool,
    deny_absolute: bool,
}

impl VerifyCell for PathSandboxCell {
    fn name(&self) -> &str { "path-sandbox" }

    async fn verify(
        &self,
        action: &Signal,
        _ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        let paths = extract_file_paths(action)?;

        for path in &paths {
            // Canonicalize and check containment
            let canonical = path.canonicalize()
                .map_err(|_| Verdict::reject("path canonicalization failed"))?;

            if !canonical.starts_with(&self.worktree) {
                return Ok(Verdict::reject(format!(
                    "path {} escapes worktree {}",
                    canonical.display(),
                    self.worktree.display(),
                )));
            }

            if self.deny_symlinks && path.is_symlink() {
                return Ok(Verdict::reject(format!(
                    "symlink {} denied by policy",
                    path.display(),
                )));
            }

            if self.deny_absolute && path.is_absolute() {
                return Ok(Verdict::reject(format!(
                    "absolute path {} denied by policy",
                    path.display(),
                )));
            }
        }

        Ok(Verdict::pass(1.0, Evidence::PathCheck {
            paths_checked: paths.len(),
            worktree: self.worktree.clone(),
        }))
    }
}
```

### RateLimiter as Verify Cell

```rust
/// Layer 2: Sliding-window rate limiter. Prevents tool-call loops
/// and cost runaway by limiting calls per (role, tool) pair.
pub struct RateLimiterCell {
    policy: RateLimitPolicy,
    state: Mutex<HashMap<RateLimitKey, VecDeque<Instant>>>,
}

impl VerifyCell for RateLimiterCell {
    fn name(&self) -> &str { "rate-limiter" }

    async fn verify(
        &self,
        action: &Signal,
        _ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        let key = RateLimitKey {
            role: extract_role(action)?,
            tool: extract_tool_name(action)?,
        };

        let mut state = self.state.lock();
        let window = state.entry(key).or_insert_with(VecDeque::new);
        let now = Instant::now();
        let cutoff = now - self.policy.window_duration;

        // Evict expired entries
        while window.front().map_or(false, |t| *t < cutoff) {
            window.pop_front();
        }

        if window.len() >= self.policy.max_calls_per_window {
            return Ok(Verdict::reject(format!(
                "rate limit exceeded: {} calls in {}s window",
                window.len(),
                self.policy.window_duration.as_secs(),
            )));
        }

        window.push_back(now);
        Ok(Verdict::pass(1.0, Evidence::RateCheck {
            current_count: window.len(),
            limit: self.policy.max_calls_per_window,
        }))
    }
}
```

The same pattern applies to BashPolicy, GitPolicy, NetworkPolicy, ScrubPolicy, and every other safety guard. They all receive input, evaluate a criterion, and emit pass/reject. The Verify protocol unifies them.

---

## 6. The Critical Integration Gap

The defense Pipeline is only effective if it wraps **every execution path**. The critical integration gap documented in `docs/11-safety/16-critical-integration-gap.md` identifies where this invariant is currently violated.

### Current State

The SafetyLayer is fully wired into the **routed/provider-backed** execution path:

```
orchestrate.rs -> provider resolution -> ToolLoop + ToolDispatcher
                                           |
                                    SafetyLayer.check_pre_execution()
                                    SafetyLayer.scrub_output()
                                           |
                                    audit emissions at each stage
```

This path covers: OpenAI-compatible providers, Anthropic API, Gemini-compat, Gemini-native (tool-capable), and Perplexity (tool-capable).

The SafetyLayer is **not wired** into the **subprocess/specialty branches**:

```
orchestrate.rs -> ExecAgent::new() -> subprocess (Claude CLI)
                                        |
                                  Claude CLI handles its own
                                  tool dispatch internally
                                        |
                                  Raw output returned -- no Roko
                                  SafetyLayer, no audit emissions
```

### Why This Matters as a Pipeline

In Pipeline terms, the subprocess branch skips all seven layers. No path sandboxing, no rate limiting, no permission checks, no policy validation, no taint barriers, no corrigibility ordering, no contract bounds. The agent's tool calls execute inside a subprocess that Roko does not mediate.

### Resolution: Universal Pipeline Wrapper

The defense Pipeline must be the **outermost wrapper** for all execution paths. Three concrete integration points:

**Integration Point 1: Pre/Post hooks in orchestrate.rs**

At `crates/roko-cli/src/orchestrate.rs`, wrap every `agent.run()` call:

```rust
// Before: raw agent execution
let result = agent.run(&prompt).await?;

// After: Pipeline-wrapped execution
let safety = SafetyLayer::from_config(&config.safety);
let pre_action = Signal::new(Kind::ProposedAction, ProposedAction {
    prompt: prompt.clone(),
    role: task.role,
    agent_id: agent.id(),
});

// Run pre-execution Pipeline (layers 1-7)
let permit = execute_defense_pipeline(pre_action, &pre_pipeline, &ctx).await?;

// Execute with permit
let result = agent.run(&prompt).await?;

// Run post-execution Pipeline (layers P1-P4)
let validated = execute_defense_pipeline(
    Signal::new(Kind::ActionResult, result),
    &post_pipeline,
    &ctx,
).await?;
```

**Integration Point 2: Claude CLI settings passthrough**

At `crates/roko-cli/src/orchestrate.rs`, pass safety configuration to subprocess agents:

```rust
let mut agent = ExecAgent::new(&config.agent.command, args);
// Translate Roko safety config to Claude CLI --allowed-tools
agent.with_settings(safety_to_claude_settings(&config.safety));
```

**Integration Point 3: In-process dispatch migration**

Replace `ExecAgent` (subprocess) with in-process `ClaudeCliAgent` where the ToolDispatcher mediates every tool call. This is the architecturally correct solution tracked as a separate priority.

---

## 7. Composition with Other Pipelines

The defense Pipeline composes with other Pipeline Graphs by fractal nesting (see [03-GRAPH.md](../../unified/03-GRAPH.md)):

- **Immune Pipeline** (from [immune-system-as-graph.md](immune-system-as-graph.md)): Runs on Signals crossing trust boundaries. The defense Pipeline and immune Pipeline operate at different scopes -- defense validates proposed actions, immune validates knowledge integrity.

- **Gate Pipeline** (from [16-SECURITY.md](../../unified/16-SECURITY.md) SS5): Runs after execution to verify output quality (compile, test, clippy, diff). The defense Pipeline is pre-execution; the Gate Pipeline is post-execution. Together they form a bracket around every action.

- **Corrigibility Chain** (from [16-SECURITY.md](../../unified/16-SECURITY.md) SS5): The 5-head corrigibility chain is itself a Pipeline of 5 Verify Cells. It is embedded as Layer 6 of the defense Pipeline. A Pipeline within a Pipeline is just a Pipeline (fractal composition).

```
Defense Pipeline (pre-execution)
  -> [execution]
  -> Gate Pipeline (post-execution quality)
  -> Post-execution Pipeline (scrub, taint, audit)
```

---

## 8. Sandbox Layers as Verify Cells

The tiered sandboxing model maps directly to Verify Cells with different `Criterion` types:

| Sandbox Tier | Verify Cell | Criterion |
|---|---|---|
| Rust (in-tree) | No sandbox Cell (process-level trust) | N/A |
| WASM | `wasm-sandbox` | `WasmCriterion { fuel_limit, memory_limit_mb, table_limit }` |
| Script | `script-sandbox` | `ScriptCriterion { timeout, working_dir, env_filter, stdin_mode }` |
| Composition | Inherited from constituent Cells | Union of constituent criteria |

```rust
/// WASM sandbox as a Verify Cell. Checks resource limits before
/// permitting WASM Cell execution.
pub struct WasmSandboxCell {
    pub fuel_limit: u64,         // default: 100_000_000
    pub memory_limit_mb: u32,    // default: 64
    pub table_limit: u32,        // default: 10_000
    pub instance_limit: u32,     // default: 4
}

impl VerifyCell for WasmSandboxCell {
    fn name(&self) -> &str { "wasm-sandbox" }

    async fn verify(
        &self,
        action: &Signal,
        ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        let wasm_req = extract_wasm_requirements(action)?;

        if wasm_req.estimated_fuel > self.fuel_limit {
            return Ok(Verdict::reject(format!(
                "WASM fuel {} exceeds limit {}",
                wasm_req.estimated_fuel, self.fuel_limit,
            )));
        }

        if wasm_req.memory_pages * 64 * 1024 > self.memory_limit_mb as u64 * 1024 * 1024 {
            return Ok(Verdict::reject("WASM memory exceeds limit"));
        }

        Ok(Verdict::pass(1.0, Evidence::WasmCheck {
            fuel_budget: self.fuel_limit - wasm_req.estimated_fuel,
            memory_budget: self.memory_limit_mb,
        }))
    }
}
```

When a WASM Cell is part of the Graph, the WASM sandbox Verify Cell is automatically inserted into the defense Pipeline at the appropriate position. The Pipeline adapts to the Cell types it protects.

---

## What This Enables

1. **Single enforcement point**: All safety checks funnel through one Pipeline. No code path can bypass safety by using an alternative dispatch mechanism.
2. **Auditable ordering**: The Pipeline TOML definition is a runtime artifact. Operators can inspect, verify, and version-control the defense ordering.
3. **Composable safety**: Adding a new safety check means adding a new Verify Cell to the Pipeline. No other code changes required. The Pipeline is open for extension, closed for modification.
4. **Domain-specific specialization**: A chain-domain agent adds an `mev-gate` Verify Cell. A healthcare agent adds a `phi-barrier` Verify Cell. The Pipeline pattern accommodates domain safety without modifying the core.
5. **Cost-ordered evaluation**: Cheap checks (path validation: microseconds) run before expensive checks (corrigibility chain: may involve LLM calls). Rejected actions fail fast and cheap.

## Feedback Loops

- **L1**: Rate limiter thresholds adjust via EMA based on observed tool-call patterns. Agents with consistent legitimate high-throughput earn higher limits.
- **L2**: Adaptive gate thresholds (already wired at `crates/roko-learn/`) feed back into the corrigibility chain's confidence requirements.
- **L3**: Pipeline rejection patterns are stored as Signals. The immune system's anomaly detection (Layer 2) watches for rejection clustering that indicates probing or escalation attempts.
- **Memory**: Pipeline pass/reject events contribute to the agent's Episode log (`.roko/episodes.jsonl`), enabling the cascade router to learn which models/agents trigger more rejections.

## Open Questions

1. **Latency budget**: Seven layers of verification add latency. The common case (all pass) should be fast -- but how fast? Path check is O(1), rate check is O(1), capability intersection is O(k) where k is the number of capability types. Corrigibility chain is the bottleneck if it requires LLM evaluation. Should corrigibility be deferred to post-execution for speed-critical paths?

2. **Pipeline customization**: Should operators be able to reorder, remove, or add layers? Reordering risks putting expensive checks before cheap ones. Removing risks creating gaps. The safe answer may be: operators can add layers but not remove or reorder the core seven.

3. **Subprocess mediation**: The integration gap is narrowing (routed paths are wired) but subprocess branches remain. Is full in-process dispatch migration the only correct solution, or can Claude CLI's own safety settings provide adequate coverage for the remaining branches?

4. **Pipeline versioning**: When the Pipeline definition changes (new Cell added, threshold adjusted), should running Flows continue with their original Pipeline or hot-swap to the new one? Hot-swap is simpler but risks mid-action inconsistency.

## Implementation Tasks

| Task | File | What |
|---|---|---|
| Wire pre/post safety hooks universally | `crates/roko-cli/src/orchestrate.rs` | Wrap all `agent.run()` calls with SafetyLayer pre/post checks, including subprocess branches |
| Express SafetyLayer guards as Verify Cells | `crates/roko-agent/src/safety/mod.rs` | Implement `VerifyCell` trait for BashPolicy, GitPolicy, NetworkPolicy, PathPolicy, ScrubPolicy, RateLimiter |
| Add defense Pipeline TOML loader | `crates/roko-core/src/graph/` | Load defense Pipeline from TOML, validate layer ordering |
| Wire Pipeline into ToolDispatcher | `crates/roko-agent/src/dispatcher/mod.rs` | Replace the ad-hoc 7-stage dispatch pipeline with a formal Pipeline Graph |
| Add WASM sandbox Verify Cell | `crates/roko-agent/src/safety/` | Implement `WasmSandboxCell` for WASM tier enforcement |
| Emit Pipeline telemetry | `crates/roko-cli/src/orchestrate.rs` | Publish pass/reject Pulses on `safety.pipeline.*` Bus topics |
| Integration test: end-to-end rejection | `crates/roko-agent/tests/` | Test that a rejected action at Layer 1 never reaches Layer 7 or execution |
