# The Cognitive Loop as Graph

> Depth for [05-EXECUTION-ENGINE.md](../../unified/05-EXECUTION-ENGINE.md). Redesigns the 7-step universal cognitive loop as a concrete Hot Graph with typed Cells, derives resumability from the Workflow/Activity split, and shows what emerges when loops compose.

---

## 1. The Loop Is Not a Metaphor

The 7-step cognitive loop (SENSE, ASSESS, COMPOSE, ACT, VERIFY, PERSIST/BROADCAST, REACT) is described in [09-universal-cognitive-loop.md](../../docs/00-architecture/09-universal-cognitive-loop.md) as a sequence. But in the unified architecture, sequences are Graphs. The loop is therefore a Hot Graph -- a resident Graph that re-fires on each tick of the Agent's adaptive clock, with state retained between ticks.

This is not a metaphor or a "conceptual mapping." The claim is concrete: each step is a Cell with typed I/O, the transitions between steps are edges with conditions, and the Engine that executes task plans is the same Engine that fires the loop. One runtime. One execution model.

### Why this matters

If the loop were special-cased, every improvement to the execution engine (retry strategies, snapshot resumability, budget enforcement, failure decomposition) would need separate implementation for the cognitive pipeline. By making the loop a Graph, those capabilities are inherited for free.

---

## 2. Each Step as a Cell

Every Cell declares its input schema, output schema, capabilities, and protocol conformance (see [02-CELL.md](../../unified/02-CELL.md) SS1). Here are the 7 loop steps re-derived as concrete Cells.

### 2.1 SENSE Cell

Gathers input from three sources: Store queries (durable Signals), Bus subscriptions (live Pulses), and external I/O (HTTP, filesystem, subprocess).

```rust
struct SenseCell {
    store: Arc<dyn Store>,
    bus: Arc<dyn Bus>,
    external: Vec<Box<dyn ExternalSource>>,
    predicates: Vec<SensePredicate>,
}

// Input schema: CorticalSnapshot (previous tick's state)
// Output schema: SensedMaterial { signals: Vec<Signal>, pulses: Vec<Pulse>, external: Vec<Signal> }
// Capabilities: { store_read, bus_subscribe }
// Protocols: [Observe]

impl Cell for SenseCell {
    fn input_schema(&self) -> Option<&TypeSchema> {
        Some(&CORTICAL_SNAPSHOT_SCHEMA)
    }

    fn output_schema(&self) -> Option<&TypeSchema> {
        Some(&SENSED_MATERIAL_SCHEMA)
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let cortical = CorticalSnapshot::from_signals(&input)?;

        // Store query: durable Signals matching predicates
        let mut signals = Vec::new();
        for pred in &self.predicates {
            signals.extend(self.store.query(pred.as_query()).await?);
        }

        // Bus drain: Pulses since last sequence number
        let pulses = drain_bus_since(&self.bus, cortical.last_seq).await?;

        // External I/O: filesystem watches, HTTP, subprocess
        let mut external = Vec::new();
        for src in &self.external {
            if let Ok(batch) = src.poll(ctx.cancel()).await {
                external.extend(batch);
            }
        }

        Ok(SensedMaterial { signals, pulses, external }.into_signals())
    }
}
```

**Execution class**: Workflow. SENSE is deterministic given the same Store and Bus state. On replay, it re-executes and produces the same result (assuming Store and Bus are restored from snapshot). In practice, SENSE often has external I/O, making it an Activity in real deployments -- but the Cell itself is categorized by the Graph author at definition time.

### 2.2 ASSESS Cell

Combines Score protocol and Route protocol into a single decision point. Scores candidates across 7 axes (relevance, confidence, urgency, novelty, salience, coherence, surprise), then routes to determine what gets acted on now.

```rust
struct AssessCell {
    scorer: Arc<dyn ScoreProtocol>,
    router: Arc<dyn RouteProtocol>,
    daimon_bias: Option<Arc<dyn DaimonBias>>,
}

// Input schema: SensedMaterial
// Output schema: Assessment { selected: Vec<Signal>, route: RouteDecision, tier: CognitiveTier }
// Capabilities: { }
// Protocols: [Score, Route]

impl Cell for AssessCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let material = SensedMaterial::from_signals(&input)?;

        // Score all candidates
        let mut scored: Vec<(Signal, f32)> = Vec::new();
        for sig in material.all_signals() {
            let score = self.scorer.score(&sig, ctx)?;

            // Daimon cross-cut: bias ASSESS via PAD vector
            let biased = match &self.daimon_bias {
                Some(d) => d.bias_score(score, ctx.cortical()),
                None => score,
            };
            scored.push((sig, biased.effective()));
        }

        // Route: which candidates win, at what tier
        let decision = self.router.route(&scored, ctx)?;

        Ok(Assessment {
            selected: decision.selected,
            route: decision,
            tier: decision.tier,
        }.into_signals())
    }
}
```

**Execution class**: Workflow. Scoring and routing are deterministic functions of input.

### 2.3 COMPOSE Cell

Assembles a prompt Signal under budget. This is where the context window is shaped, trimmed, and ordered. The VCG auction (8+ bidders competing for token budget) runs here.

```rust
struct ComposeCell {
    composer: Arc<dyn ComposeProtocol>,
    bidders: Vec<Box<dyn AttentionBidder>>,
    token_budget: usize,
}

// Input schema: Assessment + CorticalSnapshot
// Output schema: ComposedPrompt { sections: Vec<Section>, token_count: usize, cost_estimate: Cost }
// Capabilities: { store_read }
// Protocols: [Compose]

impl Cell for ComposeCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let assessment = Assessment::from_signals(&input)?;

        // Collect bids from all bidders
        let mut bids = Vec::new();
        for bidder in &self.bidders {
            bids.extend(bidder.bid(&assessment.into_task_context()));
        }

        // VCG auction: allocate token budget to highest-value sections
        let allocated = vcg_allocate(&bids, self.token_budget);

        // Assemble prompt under budget
        let composed = self.composer.compose(
            &allocated,
            &Budget::tokens(self.token_budget),
            ctx,
        )?;

        Ok(composed.into_signals())
    }
}
```

**Execution class**: Workflow. Composition is deterministic given scored bids and budget.

### 2.4 ACT Cell

Executes the selected work -- typically an LLM turn, but the same Cell interface covers tool calls, chain actions, and subprocess execution. This is the only step that calls an external provider.

```rust
struct ActCell {
    dispatcher: Arc<dyn AgentDispatcher>,
}

// Input schema: ComposedPrompt + RouteDecision
// Output schema: ActionResult { response: Signal, pulses: Vec<Pulse>, cost: Cost }
// Capabilities: { llm_call, tool_execute, bus_publish }
// Protocols: []

impl Cell for ActCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let prompt = ComposedPrompt::from_signals(&input)?;
        let route = RouteDecision::from_signals(&input)?;

        // Dispatch to the model/tool selected by Route
        let result = self.dispatcher.dispatch(
            &prompt,
            &route,
            ctx.cancel(),
        ).await?;

        // Publish live Pulses for stream observers
        for pulse in &result.pulses {
            ctx.bus().publish(pulse.clone()).await?;
        }

        Ok(result.into_signals())
    }
}
```

**Execution class**: Activity. LLM calls are non-deterministic. On replay, the recorded output is returned without re-executing. This is the load-bearing distinction in the Workflow/Activity split.

### 2.5 VERIFY Cell

Runs the Verify protocol pipeline: Signal-gates on durable output, stream-gates on live Pulses. Produces a Verdict Signal.

```rust
struct VerifyCell {
    pipeline: Vec<Arc<dyn VerifyProtocol>>,
}

// Input schema: ActionResult
// Output schema: VerifyResult { verdict: Verdict, evidence: Vec<Signal> }
// Capabilities: { execute_command (for compile/test gates) }
// Protocols: [Verify]

impl Cell for VerifyCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let action = ActionResult::from_signals(&input)?;

        let mut verdicts = Vec::new();
        for gate in &self.pipeline {
            let v = gate.verify_post(&action.response, ctx).await?;
            verdicts.push(v);
            // Stream gate: can halt early if hard failure
            if v.is_hard_fail() {
                break;
            }
        }

        let composite = Verdict::compose(&verdicts);
        Ok(VerifyResult { verdict: composite, evidence: verdicts }.into_signals())
    }
}
```

**Execution class**: Activity. Gates run external processes (compile, test, clippy) whose output may differ across runs.

### 2.6 PERSIST/BROADCAST Cell

Writes Signals to Store and publishes Pulses on Bus. These are co-equal operations in a single Cell.

```rust
struct PersistBroadcastCell {
    store: Arc<dyn Store>,
    bus: Arc<dyn Bus>,
}

// Input schema: ActionResult + VerifyResult
// Output schema: PersistResult { stored_refs: Vec<SignalRef>, published_topics: Vec<Topic> }
// Capabilities: { store_write, bus_publish }
// Protocols: [Store]

impl Cell for PersistBroadcastCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let action = ActionResult::from_signals(&input)?;
        let verdict = VerifyResult::from_signals(&input)?;

        // PERSIST: write to Store with lineage
        let action_ref = self.store.put(action.response.clone()).await?;
        let verdict_ref = self.store.put(verdict.verdict.clone().into_signal()).await?;

        // BROADCAST: publish Pulses on Bus
        let topic = Topic::new("verify.verdict.emitted");
        let pulse = verdict.verdict.to_pulse(topic, ctx.source());
        self.bus.publish(pulse).await?;

        Ok(PersistResult {
            stored_refs: vec![action_ref, verdict_ref],
            published_topics: vec![topic],
        }.into_signals())
    }
}
```

**Execution class**: Activity. Store writes and Bus publishes have side effects.

### 2.7 REACT Cell

React protocol Cells consume the new state and emit further Signals or Pulses. This is where episode consolidation, circuit-breaking, routing feedback, and task follow-up happen.

```rust
struct ReactCell {
    policies: Vec<Arc<dyn ReactProtocol>>,
}

// Input schema: PersistResult + VerifyResult + CorticalSnapshot
// Output schema: ReactOutput { signals: Vec<Signal>, pulses: Vec<Pulse> }
// Capabilities: { store_write, bus_publish }
// Protocols: [React]

impl Cell for ReactCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let persist = PersistResult::from_signals(&input)?;
        let verdict = VerifyResult::from_signals(&input)?;

        let mut all_signals = Vec::new();
        let mut all_pulses = Vec::new();

        for policy in &self.policies {
            let out = policy.react(&persist, &verdict, ctx).await?;
            all_signals.extend(out.signals);
            all_pulses.extend(out.pulses);
        }

        // Publish emitted Pulses
        for pulse in &all_pulses {
            ctx.bus().publish(pulse.clone()).await?;
        }

        Ok(ReactOutput { signals: all_signals, pulses: all_pulses }.into_signals())
    }
}
```

**Execution class**: Workflow (most policies are deterministic functions of their input). Individual policy Cells that have side effects can be marked Activity.

---

## 3. The Graph TOML

The 7-step loop as a concrete Graph definition. This is what the Engine interprets.

```toml
[graph]
name = "cognitive-loop"
version = "1.0.0"
hot = true                           # Stays resident, re-fires each tick
clock = { kind = "adaptive" }        # Bound to Agent's adaptive clock

[graph.policy]
max_parallelism = 1                  # Sequential pipeline (no parallelism within a tick)
failure_strategy = "retry_with_escalation"
snapshot_interval_secs = 300         # Checkpoint every 5 minutes
budget_scope = "agent"               # Draws from Agent budget

# ── Nodes ──────────────────────────────────────────────────────────

[[graph.nodes]]
id = "sense"
cell = "roko.cognitive.sense"
execution_class = "workflow"         # Deterministic in replay

[[graph.nodes]]
id = "assess"
cell = "roko.cognitive.assess"
execution_class = "workflow"

[[graph.nodes]]
id = "compose"
cell = "roko.cognitive.compose"
execution_class = "workflow"

[[graph.nodes]]
id = "act"
cell = "roko.cognitive.act"
execution_class = "activity"         # LLM call: non-deterministic, recorded for replay

[[graph.nodes]]
id = "verify"
cell = "roko.cognitive.verify"
execution_class = "activity"         # Runs external gate processes

[[graph.nodes]]
id = "persist_broadcast"
cell = "roko.cognitive.persist_broadcast"
execution_class = "activity"         # Side effects: store + bus

[[graph.nodes]]
id = "react"
cell = "roko.cognitive.react"
execution_class = "workflow"

# ── Edges (sequential pipeline) ───────────────────────────────────

[[graph.edges]]
from = "sense"
to = "assess"

[[graph.edges]]
from = "assess"
to = "compose"

[[graph.edges]]
from = "compose"
to = "act"

[[graph.edges]]
from = "act"
to = "verify"

[[graph.edges]]
from = "verify"
to = "persist_broadcast"

[[graph.edges]]
from = "persist_broadcast"
to = "react"

# ── Feedback edge (react output feeds next tick's sense) ──────────

[[graph.edges]]
from = "react"
to = "sense"
kind = "feedback"                    # Marks this as a Hot Graph feedback loop
```

The feedback edge from REACT back to SENSE is what makes this a Loop. On each tick, REACT's output is retained in memory and becomes part of SENSE's input on the next tick.

---

## 4. Hot Graph Tick Semantics

When the Engine registers this Graph as a Hot Graph (via `Engine::register_hot()`), the execution model changes:

```
Tick N:
  1. SENSE receives: (cortical_snapshot_N, react_output_{N-1})
  2. Pipeline executes: SENSE -> ASSESS -> COMPOSE -> ACT -> VERIFY -> PERSIST/BROADCAST -> REACT
  3. REACT output is retained in memory for tick N+1
  4. Tick Pulse published: agent:{id}.tick.completed

Tick N+1:
  1. SENSE receives: (cortical_snapshot_{N+1}, react_output_N)
  2. ... same pipeline ...
```

Between ticks:
- Node outputs from the previous tick are retained in memory.
- CorticalState is updated atomically (single-writer / multi-reader).
- The adaptive clock determines when the next tick fires based on regime.
- Budget accounting is continuous (demurrage costs accumulate between ticks).

### T0 Short-Circuit

Most ticks (~80%) short-circuit at ASSESS. When the ASSESS Cell determines that all 16 T0 probes report "no change" and EFE selects T0 (zero-cost reflex), the remaining Cells do not execute. The pipeline publishes a minimal `tick.completed` Pulse and returns.

```rust
// Inside AssessCell::execute, the T0 short-circuit:
if decision.tier == CognitiveTier::T0Reflex && decision.selected.is_empty() {
    // Nothing interesting. Skip ACT/VERIFY/PERSIST/REACT.
    return Ok(Assessment::noop().into_signals());
}
```

This short-circuit is expressed as a conditional edge in the Graph:

```toml
[[graph.edges]]
from = "assess"
to = "compose"
condition = "assessment.tier != T0 || assessment.has_selected"

[[graph.edges]]
from = "assess"
to = "react"
condition = "assessment.tier == T0 && !assessment.has_selected"
label = "T0 short-circuit"
```

When the T0 short-circuit fires, REACT still runs (to update counters, check deadlines, emit heartbeats) but ACT/VERIFY/PERSIST are skipped entirely. Cost: $0.

---

## 5. Composing Loops: Nested Graphs

What happens when you compose multiple loops? The Agent has three concurrent timescales (gamma, theta, delta), each running the same 7-step loop at different speeds. In Graph terms, these are three Hot Graphs sharing the same CorticalState but with different clock bindings:

```toml
# Gamma loop: fast perception (~5-15s)
[graph]
name = "cognitive-loop-gamma"
hot = true
clock = { kind = "adaptive", timescale = "gamma" }

# Theta loop: reflective (~75s)
[graph]
name = "cognitive-loop-theta"
hot = true
clock = { kind = "adaptive", timescale = "theta" }

# Delta loop: consolidation (~hours)
[graph]
name = "cognitive-loop-delta"
hot = true
clock = { kind = "adaptive", timescale = "delta" }
```

Each loop is an independent Hot Flow managed by the Engine. They share:
- The same CorticalState (atomic reads, single writer per loop)
- The same Store (serialized writes via the Store protocol)
- The same Bus (Pulses from one loop are visible to others)
- The same budget (VitalityTracker with atomic accounting)

They do NOT share:
- Node outputs (each loop retains its own tick state)
- Clock timing (each fires independently)
- Active Cell instances (each loop has its own Cell configuration)

### Inter-loop communication

The loops communicate through two channels:

1. **Bus Pulses**: Theta publishes `agent:{id}.theta.replanned` when it decides to change strategy. Gamma's SENSE Cell receives this on the next tick.

2. **CorticalState**: Delta updates `cortical.confidence` after dream consolidation. Gamma reads it on every tick and adjusts its EFE thresholds.

```rust
// Gamma loop reads theta's replan signal via Bus
fn sense_bus_drain(bus: &dyn Bus, last_seq: u64) -> Vec<Pulse> {
    let pulses = bus.drain_since(last_seq);
    pulses.into_iter()
        .filter(|p| p.topic.matches("agent:*.theta.replanned")
                  || p.topic.matches("agent:*.delta.consolidated"))
        .collect()
}
```

### The nesting emerges naturally

A Theta tick can itself spawn a sub-Graph (a plan for re-planning). That sub-Graph is a standard Flow managed by the same Engine. The Theta loop's ACT Cell submits the sub-Graph via `Engine::start()` and awaits its completion. This is Graph nesting: a Hot Graph's Cell spawns a standard Graph as a child.

```
Agent
  |
  +-- Gamma Hot Graph (fires every ~10s)
  |     |
  |     +-- [T0 short-circuit most ticks]
  |     +-- [T1/T2 ACT Cell: spawns task sub-Graphs]
  |
  +-- Theta Hot Graph (fires every ~75s)
  |     |
  |     +-- ACT Cell: spawns replan sub-Graph
  |           |
  |           +-- Standard Flow: analyze_progress -> replan -> validate
  |
  +-- Delta Hot Graph (fires every ~hours)
        |
        +-- ACT Cell: spawns dream consolidation sub-Graph
              |
              +-- Standard Flow: nrem_replay -> rem_imagination -> integration
```

---

## 6. Flow Snapshots and Resumability

The Workflow/Activity split is the key to resumability. See [05-EXECUTION-ENGINE.md](../../unified/05-EXECUTION-ENGINE.md) SS5.

### Why the split matters for the cognitive loop

In a cognitive loop tick, the deterministic steps (SENSE with fixed Store state, ASSESS, COMPOSE, REACT) are Workflow Cells. The non-deterministic steps (ACT with LLM calls, VERIFY with external gate processes, PERSIST/BROADCAST with side effects) are Activity Cells.

When the Engine resumes from a snapshot:

```
Snapshot taken after VERIFY completed in tick 47:

Tick 47 replay:
  SENSE:             Workflow -> re-execute (same result)
  ASSESS:            Workflow -> re-execute (same result)
  COMPOSE:           Workflow -> re-execute (same result)
  ACT:               Activity -> load recorded output from activity log
  VERIFY:            Activity -> load recorded output from activity log
  PERSIST/BROADCAST: Activity -> re-execute (state may have been partially persisted)
  REACT:             Workflow -> re-execute (same result)
```

The Activity recordings are written to `runs/<run-id>/activities/<node-id>.json` immediately after each Activity completes. This is the mechanism that prevents double LLM calls on resume.

### Snapshot structure for the cognitive loop

```rust
struct CognitiveLoopSnapshot {
    // Standard FlowSnapshot fields
    run_id: RunId,
    graph_id: GraphId,
    snapshot_at: DateTime<Utc>,

    // Hot Graph-specific: state retained between ticks
    tick_count: u64,
    last_react_output: Vec<Signal>,
    cortical_snapshot: CorticalSnapshot,

    // Per-node states for the current tick
    node_states: BTreeMap<NodeId, NodeState>,

    // Activity records for replay
    activity_records: Vec<ContentHash>,

    // Budget at snapshot time
    budget_remaining: Cost,
    demurrage_accumulated: Cost,
}
```

### Crash recovery

If the process crashes mid-tick:
1. The Engine loads the latest snapshot.
2. Activity Cells with recorded outputs skip re-execution.
3. Activity Cells without recorded outputs re-execute (the LLM call is retried).
4. Workflow Cells always re-execute (deterministic, so the result matches).
5. The tick resumes from the first non-completed node.

The worst case is a duplicate LLM call (if ACT completed but the Activity record was not flushed before crash). This is acceptable: the cost is one extra API call, and the system prefers availability over exactly-once semantics.

---

## 7. The Loop Under Adversarial Conditions

What happens when a Cell in the loop is Byzantine -- producing incorrect outputs, delayed responses, or actively malicious results?

### Byzantine Cell threat model

```
Byzantine Cell B replaces the COMPOSE Cell and:
  (a) Injects malicious instructions into the prompt
  (b) Inflates token counts to exhaust budget
  (c) Returns stale/irrelevant context to degrade performance
  (d) Delays execution to starve downstream Cells
```

### Defense: Verify as a consensus gate

The VERIFY Cell acts as a BFT (Byzantine Fault Tolerant) consensus gate. Even if COMPOSE or ACT produces corrupted output, VERIFY catches it:

```rust
// Verify Cell with multi-gate consensus
struct ByzantineResilientVerifyCell {
    gates: Vec<Arc<dyn VerifyProtocol>>,
    quorum: usize,  // minimum gates that must pass
}

impl Cell for ByzantineResilientVerifyCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let action = ActionResult::from_signals(&input)?;

        let mut pass_count = 0;
        let mut fail_count = 0;
        let mut verdicts = Vec::new();

        for gate in &self.gates {
            let v = gate.verify_post(&action.response, ctx).await?;
            if v.passed() { pass_count += 1; } else { fail_count += 1; }
            verdicts.push(v);
        }

        let consensus = if pass_count >= self.quorum {
            VerdictStatus::Pass
        } else {
            VerdictStatus::Fail
        };

        Ok(VerifyResult {
            verdict: Verdict::with_status(consensus),
            evidence: verdicts,
            quorum_met: pass_count >= self.quorum,
        }.into_signals())
    }
}
```

### Defense: Budget as a kill switch

A Byzantine Cell that tries to exhaust the budget is stopped by the Engine's budget enforcement. The BudgetTracker rejects any Cell execution whose estimated cost exceeds remaining budget. Even if a Cell lies about its cost estimate, the post-execution cost tracking catches the overrun and triggers a budget warning Pulse.

### Defense: Timeout as a liveness guarantee

Every Cell execution has a node-level timeout enforced by the Engine. A Byzantine Cell that delays indefinitely is killed after the timeout. The failure strategy (Retry, RetryWithEscalation, or Decompose) then takes over.

### Defense: CaMeL IFC tagging

Extension Cells that intercept the pipeline are tagged with capability provenance. A Cell that was not granted `llm_call` capability cannot invoke the LLM. A Cell that was not granted `store_write` cannot corrupt the Store. This is enforced by the Space's capability intersection, not by the Cell's self-report.

### The residual risk

The one attack that the Graph model cannot prevent: a Byzantine ACT Cell that produces plausible but subtly wrong output that passes all Verify gates. This is the alignment problem in microcosm. The Loop's defense is calibration over time -- REACT logs episodes, the learning system tracks prediction accuracy, and persistent errors eventually trigger regime changes that escalate to T2 reasoning or human review.

---

## 8. What This Enables

- **Unified execution model**: Task plans, cognitive loops, dream cycles, and monitoring pipelines all execute on the same Engine. No special-cased loop runner.
- **Resumability for free**: The Workflow/Activity split gives the cognitive loop crash recovery without custom checkpoint logic.
- **Composable timescales**: Gamma/Theta/Delta are three Hot Graphs, not three separate scheduling systems.
- **T0 short-circuit as a conditional edge**: The 80% zero-cost tick path is a Graph edge condition, not an `if` statement in a custom loop runner.
- **Nested execution**: The Engine's ability to start sub-Graphs means that Theta replan and Delta dream consolidation are standard Flows, not separate code paths.

---

## 9. Feedback Loops

| Loop | What it observes | What it adjusts |
|---|---|---|
| **EFE adaptation** | Prediction error from ASSESS across ticks | T0/T1/T2 escalation thresholds in the Route protocol |
| **Section effects** | Gate pass/fail correlated with COMPOSE sections | Beta-distribution posteriors for VCG bidder valuations |
| **Regime transitions** | Prediction error trend over 3+ ticks | Adaptive clock multipliers (Calm/Normal/Volatile/Crisis) |
| **Vitality phases** | Budget remaining / initial | COMPOSE token budget, Verify rigor, exploration rate |
| **Reflex promotion** | T2 patterns that succeed 5+ times at >90% | T0 reflex store gains new rules; future ticks short-circuit |

Each of these Loops is itself a Graph -- a feedback edge from REACT back to ASSESS with a learning Cell in between. The Loops compose because they operate on the same Signal types through the same Bus.

---

## 10. Open Questions

1. **Tick ordering across agents**: When multiple Agents share a Bus, their cognitive loops fire independently. Is there a useful ordering guarantee (e.g., consensus on Bus sequence numbers) or is eventual consistency sufficient?

2. **Hot Graph migration**: Can a Hot Graph be migrated between processes (e.g., for load balancing)? The snapshot mechanism supports it in principle, but the CorticalState's atomic operations assume a single process. What is the serialization boundary?

3. **Activity idempotency**: The Workflow/Activity split assumes that Activity replay returns recorded output. But what if the recorded output references ephemeral state (e.g., a tool call that created a file)? Should Activities include compensating rollback logic?

4. **Graph versioning**: When the cognitive loop Graph is updated (e.g., adding a new Cell between COMPOSE and ACT), how does a running Hot Flow handle the version transition? The snapshot format includes `graph_version`, but the migration semantics are unspecified.

5. **Byzantine SENSE**: The Verify Cell catches bad ACT output, but who verifies SENSE? A corrupted Store could feed the loop with poisoned context. Is there a SENSE-level integrity check (e.g., content-hash verification of Store entries)?

---

## Cross-References

- [05-EXECUTION-ENGINE.md](../../unified/05-EXECUTION-ENGINE.md) -- Engine API, Hot Graph execution, Workflow/Activity split
- [02-CELL.md](../../unified/02-CELL.md) SS1 -- Cell trait definition and typed I/O
- [03-GRAPH.md](../../unified/03-GRAPH.md) -- Graph structure, edge conditions, Hot Graphs
- [04-SPECIALIZATIONS.md](../../unified/04-SPECIALIZATIONS.md) SS2 -- Flow as Graph-at-runtime
- [07-AGENT-RUNTIME.md](../../unified/07-AGENT-RUNTIME.md) SS8 -- The 9-step pipeline as Hot Graph
- [resilience-and-numerics.md](resilience-and-numerics.md) -- Circuit breakers, failure strategies, numerical stability
