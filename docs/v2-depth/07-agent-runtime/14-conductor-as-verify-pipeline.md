# Conductor as Verify Pipeline

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How agent supervision emerges as a Verify Pipeline of watcher Cells with OODA Loop feedback, rather than a bespoke Conductor struct.

---

## 1. The Problem with a Bespoke Conductor

The current `roko-conductor` crate defines a `Conductor` struct that manually iterates over 10 watcher implementations, collects their outputs, and feeds them through an `InterventionPolicy`. This works, but it duplicates patterns that already exist in the kernel:

- Each watcher is a `React` impl that scans `&[Signal]` and emits intervention Signals. This is exactly what a **Verify Cell** does: inspect Signals, produce a Verdict.
- The Conductor runs all watchers in sequence and picks the worst severity. This is exactly what a **Pipeline Graph** does: linear chain of Cells, outputs forwarded.
- The policy that maps worst-severity to a decision (Continue/Restart/Fail) is exactly what a **Route Cell** does: select among alternatives based on input.
- The three timescale loops (gamma/theta/delta) are exactly what **nested Loop Graphs** do: a feedback edge from the exit back to the entry, parameterized by tick frequency.
- The 8 cognitive signals (Pause, Resume, Escalate, Cooldown, etc.) are not function calls -- they are exactly **Pulse kinds on Bus**.

The redesign eliminates the bespoke `Conductor` struct. The conductor becomes a composition of standard kernel primitives: Verify Cells, Pipeline Graphs, Route Cells, Loop Graphs, and Bus Pulses. No new abstractions are needed.

---

## 2. Watchers as Verify Cells

### The Verify protocol

Every watcher implements the same `Verify` protocol that code gates use. The `Verify` protocol consumes Signals and produces a `Verdict`. There is no difference between "verify that this code compiles" and "verify that this agent is not stuck in a loop" -- both are Verify Cells with different input schemas and detection logic.

```rust
/// The Verify protocol. Implemented by code gates AND agent supervision
/// watchers. Same trait, same Verdict type, same Bus integration.
///
/// A Verify Cell consumes Signals (code artifacts OR agent trajectory
/// events) and produces a Verdict (pass/fail with severity, evidence,
/// and suggested remediation).
pub trait Verify: Cell {
    /// Inspect input Signals and produce a Verdict.
    ///
    /// For code gates: input is compilation output, test results, diffs.
    /// For agent watchers: input is the agent's Signal stream (turns,
    /// costs, file changes, gate results).
    fn verify(&self, input: &[Signal], ctx: &CellContext) -> Verdict;
}

/// Universal verdict type. Used by both code gates and agent watchers.
pub struct Verdict {
    /// Did the verification pass?
    pub passed: bool,

    /// Severity if failed: Info (log only), Warning (restart phase),
    /// Critical (abort plan).
    pub severity: Severity,

    /// Which Cell produced this verdict.
    pub source: CellId,

    /// Human-readable explanation.
    pub reason: String,

    /// Evidence Signals that support the verdict (e.g., the ghost turn
    /// events, the cost metrics, the compile error output).
    pub evidence: Vec<Signal>,

    /// Optional remediation hint for the Route Cell downstream.
    pub remediation: Option<Remediation>,

    /// Numeric metric for threshold learning (e.g., consecutive ghost
    /// turns = 3, cost ratio = 1.25, context usage = 0.82).
    pub metric: Option<f64>,
}

/// What the system should do in response to a failed verdict.
pub enum Remediation {
    /// Continue but emit a Pulse (Info-level).
    LogOnly,
    /// Restart the agent with the given context injection.
    Restart { context: String },
    /// Abort the plan with the given reason.
    Abort { reason: String },
    /// Escalate to a higher model tier.
    Escalate { to_tier: u32 },
    /// Reduce pressure (extend budgets, slow down).
    Cooldown { factor: f64 },
    /// Try an alternative approach.
    Explore { budget_multiplier: f64 },
}
```

### Mapping the 10 watchers

Each of the 10 existing watchers becomes a Verify Cell. The Cell's `input_schema` declares what Signal kinds it consumes. The Cell's `output_schema` declares that it produces a `Verdict`. The detection logic is identical to the existing watcher implementations -- only the trait boundary changes.

| Watcher | Verify Cell name | Input Signals | Detection logic | Default threshold |
|---|---|---|---|---|
| Ghost Turn | `verify.ghost_turn` | `conductor.ghost_turn` events | Count consecutive turns with `output_meaningful=false` and `net_new_changes=0` | MAX_GHOST_TURNS=3 |
| Compile Fail Repeat | `verify.compile_fail` | `GateVerdict` + `PlanPhase` with `event=GateFailed` | Count consecutive compile gate failures | MAX_COMPILE_FAIL_REPEAT=3 |
| Cost Overrun | `verify.cost_overrun` | `Metric` signals with `name=plan_cost` and `name=plan_budget` | Compare accumulated cost against budget | DEFAULT_BUDGET_USD=10.0 |
| Iteration Loop | `verify.iteration_loop` | `GateVerdict` + `PlanPhase` | Count gate-fail-retry cycles without progress | MAX_ITERATION_LOOP=3 |
| Review Loop | `verify.review_loop` | `PlanPhase` with review-related events | Count review-revise cycles | MAX_REVIEW_CYCLES=3 |
| Spec Drift | `verify.spec_drift` | `Metric` signals with spec hashes | Compare output divergence from spec | MAX_SPEC_DRIFT_RATIO=0.25 |
| Stuck Pattern | `verify.stuck_pattern` | Agent output Signals | Detect repeated identical outputs | MAX_STUCK_REPEATS=4 |
| Test Failure Budget | `verify.test_budget` | `GateVerdict` for test gates | Track test failure count trajectory | MIN_FAILURE_INCREASE=1 |
| Time Overrun | `verify.time_overrun` | `Metric` signals with timestamps | Compare elapsed time against deadline | ALERT_THRESHOLD=0.80 |
| Context Window Pressure | `verify.context_pressure` | `Metric` signals with context usage | Compare context usage against limit | MAX_CONTEXT_USAGE_RATIO=0.80 |

### Watcher families as metadata

Each Verify Cell carries a `family` tag in its metadata, used by the compound pattern detector downstream:

```rust
/// Watcher family. Attached as Cell metadata. The pattern detector
/// uses family membership for cross-family escalation.
pub enum WatcherFamily {
    /// Cost, time, and context window pressure.
    Resource,
    /// Compile, test, and spec drift.
    Quality,
    /// Ghost turn, iteration loop, stuck pattern, review loop.
    Progress,
}

/// Example: the ghost turn watcher as a Verify Cell.
pub struct GhostTurnVerifyCell {
    /// Consecutive ghost turns before firing.
    max_ghost_turns: usize,
}

impl Cell for GhostTurnVerifyCell {
    fn id(&self) -> CellId {
        CellId::from_name_version("verify.ghost_turn", "1.0.0")
    }

    fn name(&self) -> &str { "verify.ghost_turn" }

    fn input_schema(&self) -> Option<&TypeSchema> {
        // Accepts: conductor.ghost_turn events
        Some(&GHOST_TURN_SCHEMA)
    }

    fn output_schema(&self) -> Option<&TypeSchema> {
        // Produces: Verdict
        Some(&VERDICT_SCHEMA)
    }

    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Verify]
    }

    fn metadata(&self) -> &CellMetadata {
        // family = Progress
        &self.meta
    }

    fn estimated_cost(&self) -> MicroCents {
        MicroCents(0) // Pure computation, no LLM calls
    }
}

impl Verify for GhostTurnVerifyCell {
    fn verify(&self, input: &[Signal], _ctx: &CellContext) -> Verdict {
        // Count consecutive ghost turns from the end of the stream.
        let mut consecutive = 0usize;
        for signal in input.iter().rev() {
            if !is_ghost_turn(signal) { break; }
            if signal.body.get("output_meaningful") == Some(true) { break; }
            if signal.body.get("net_new_changes") != Some(0) { break; }
            consecutive += 1;
        }

        if consecutive >= self.max_ghost_turns {
            Verdict {
                passed: false,
                severity: Severity::Warning,
                source: self.id(),
                reason: format!(
                    "{consecutive} consecutive ghost turns with no meaningful output"
                ),
                evidence: input.iter().rev().take(consecutive).cloned().collect(),
                remediation: Some(Remediation::Restart {
                    context: "Ghost turn pattern detected. Try a different approach.".into(),
                }),
                metric: Some(consecutive as f64),
            }
        } else {
            Verdict::pass(self.id())
        }
    }
}
```

The key insight: this Cell is indistinguishable from a code gate Cell to the runtime. The Pipeline Graph does not care whether a Verify Cell is checking compilation output or agent trajectory -- it just runs Verify Cells in order and collects Verdicts.

---

## 3. The Conductor as a Pipeline Graph

### Pipeline structure

The Conductor is a Pipeline Graph of 10 Verify Cells followed by a Route Cell that selects the intervention. The Pipeline pattern is a linear chain: each Cell receives the Signals produced by its predecessor, plus the original input stream.

```
                         Agent Signal Stream
                               |
                               v
                    +---------------------+
                    | verify.ghost_turn   |---> Verdict
                    +---------------------+
                               |
                    +---------------------+
                    | verify.compile_fail |---> Verdict
                    +---------------------+
                               |
                    +---------------------+
                    | verify.cost_overrun |---> Verdict
                    +---------------------+
                               |
                    +---------------------+
                    | verify.iter_loop    |---> Verdict
                    +---------------------+
                               |
                    +---------------------+
                    | verify.review_loop  |---> Verdict
                    +---------------------+
                               |
                    +---------------------+
                    | verify.spec_drift   |---> Verdict
                    +---------------------+
                               |
                    +---------------------+
                    | verify.stuck_pattern|---> Verdict
                    +---------------------+
                               |
                    +---------------------+
                    | verify.test_budget  |---> Verdict
                    +---------------------+
                               |
                    +---------------------+
                    | verify.time_overrun |---> Verdict
                    +---------------------+
                               |
                    +---------------------+
                    | verify.ctx_pressure |---> Verdict
                    +---------------------+
                               |
                      all 10 Verdicts
                               |
                               v
                    +---------------------+
                    | pattern_detector    |---> CompoundVerdicts (optional escalation)
                    +---------------------+
                               |
                    +---------------------+
                    | route.intervention  |---> ConductorDecision
                    +---------------------+
```

### FanOut for parallel evaluation

In practice, all 10 watcher Cells are independent: they each read the same input stream and produce independent Verdicts. The Graph uses a **FanOut** node to run them in parallel, then a **FanIn** node to collect all Verdicts before passing them to the pattern detector and Route Cell.

```rust
/// The conductor Pipeline Graph, defined in TOML (shown here as Rust
/// construction for clarity).
fn build_conductor_graph(watchers: Vec<Box<dyn Verify>>) -> Graph {
    let mut graph = Graph::new("conductor", "1.0.0");

    // Entry: receives the agent Signal stream.
    let entry = graph.add_node(Node::noop("entry"));

    // FanOut: broadcast input to all watcher Cells in parallel.
    let fanout = graph.add_node(Node::fanout("watcher_fanout"));
    graph.add_edge(entry, fanout);

    // Add all 10 watcher Verify Cells.
    let mut watcher_nodes = Vec::new();
    for watcher in &watchers {
        let node = graph.add_node(Node::cell(watcher.id()));
        graph.add_edge(fanout, node);
        watcher_nodes.push(node);
    }

    // FanIn: collect all Verdicts.
    let fanin = graph.add_node(Node::fanin("verdict_collect"));
    for node in &watcher_nodes {
        graph.add_edge(*node, fanin);
    }

    // Pattern detector: cross-family escalation.
    let pattern = graph.add_node(Node::cell(
        CellId::from_name("pattern_detector"),
    ));
    graph.add_edge(fanin, pattern);

    // Route Cell: select intervention based on worst severity.
    let route = graph.add_node(Node::cell(
        CellId::from_name("route.intervention"),
    ));
    graph.add_edge(pattern, route);

    // Exit: emit ConductorDecision.
    graph.set_entry(vec![entry]);
    graph.set_exit(vec![route]);

    graph
}
```

### The pattern detector as a Verify Cell

The compound pattern detector (`PatternDetector` in the current code) becomes another Verify Cell in the pipeline. It receives the 10 individual Verdicts and looks for cross-family correlations:

- **Resource exhaustion**: 2+ Resource-family watchers fired.
- **Quality degradation**: 2+ Quality-family watchers fired.
- **Progress stall**: 2+ Progress-family watchers fired.
- **Total resource exhaustion**: all 3 Resource watchers fired.
- **Progressive degradation**: ghost_turn + iteration_loop + stuck_pattern all fired.

When a compound pattern is detected, the pattern detector emits a new Verdict with escalated severity (Warning + Warning from different families escalates to Critical). This Verdict is forwarded alongside the individual Verdicts to the Route Cell.

```rust
impl Verify for PatternDetectorCell {
    fn verify(&self, input: &[Signal], _ctx: &CellContext) -> Verdict {
        // input contains the Verdicts from all 10 watcher Cells.
        let verdicts: Vec<Verdict> = input.iter()
            .filter_map(|s| s.body.deserialize::<Verdict>().ok())
            .collect();

        let failed = verdicts.iter().filter(|v| !v.passed).collect::<Vec<_>>();
        if failed.is_empty() {
            return Verdict::pass(self.id());
        }

        // Group by family.
        let mut family_fires: HashMap<WatcherFamily, Vec<&Verdict>> = HashMap::new();
        for v in &failed {
            if let Some(family) = self.family_of(&v.source) {
                family_fires.entry(family).or_default().push(v);
            }
        }

        // Cross-family escalation: Warning+Warning from different families.
        let warning_families: Vec<WatcherFamily> = family_fires.iter()
            .filter(|(_, vs)| vs.iter().any(|v| v.severity >= Severity::Warning))
            .map(|(f, _)| *f)
            .collect();

        if warning_families.len() >= 2 {
            return Verdict {
                passed: false,
                severity: Severity::Critical,
                source: self.id(),
                reason: format!(
                    "Cross-family escalation: warnings in {} families",
                    warning_families.len()
                ),
                evidence: failed.iter().flat_map(|v| v.evidence.clone()).collect(),
                remediation: Some(Remediation::Abort {
                    reason: "Multiple watcher families firing simultaneously".into(),
                }),
                metric: Some(warning_families.len() as f64),
            };
        }

        // Same-family compound patterns.
        for (family, vs) in &family_fires {
            if vs.len() >= 2 {
                let pattern_name = match family {
                    WatcherFamily::Resource => "resource_exhaustion",
                    WatcherFamily::Quality => "quality_degradation",
                    WatcherFamily::Progress => "progress_stall",
                };
                return Verdict {
                    passed: false,
                    severity: Severity::Critical,
                    source: self.id(),
                    reason: format!("Compound pattern: {pattern_name}"),
                    evidence: vs.iter().flat_map(|v| v.evidence.clone()).collect(),
                    remediation: Some(Remediation::Abort {
                        reason: format!("{pattern_name} detected").into(),
                    }),
                    metric: Some(vs.len() as f64),
                };
            }
        }

        Verdict::pass(self.id())
    }
}
```

### The Route Cell as intervention selector

The final node in the pipeline is a Route Cell that maps the collected Verdicts to a `ConductorDecision`. This replaces both `WorstSeverityPolicy` and `BanditPolicy` with a single Route Cell that can be swapped via configuration.

```rust
/// The intervention Route Cell. Selects the conductor decision based
/// on the worst severity across all Verdicts. Replaces the bespoke
/// InterventionPolicy trait.
pub struct InterventionRouteCell {
    /// After warmup, blend with Thompson Sampling bandit.
    bandit: Option<ConductorBandit>,
    /// Adaptive threshold learner.
    threshold_learner: ThresholdLearner,
}

impl Cell for InterventionRouteCell {
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Route]
    }
    // ...
}

impl Route for InterventionRouteCell {
    fn route(&self, input: &[Signal], ctx: &CellContext) -> RouteDecision {
        let verdicts: Vec<Verdict> = extract_verdicts(input);

        // Find worst severity.
        let worst = verdicts.iter()
            .filter(|v| !v.passed)
            .max_by_key(|v| v.severity);

        let static_decision = match worst {
            None => ConductorDecision::Continue,
            Some(v) => match v.severity {
                Severity::Info => ConductorDecision::Continue,
                Severity::Warning => ConductorDecision::Restart {
                    reason: v.reason.clone(),
                },
                Severity::Critical => ConductorDecision::Fail {
                    reason: v.reason.clone(),
                },
            },
        };

        // Blend with bandit after warmup.
        let decision = if let Some(ref bandit) = self.bandit {
            if bandit.is_warmed_up() {
                blend_with_bandit(bandit, &verdicts, static_decision, ctx)
            } else {
                static_decision
            }
        } else {
            static_decision
        };

        // Derive cognitive Pulses from verdicts.
        let pulses = derive_cognitive_pulses(&verdicts);
        for pulse in &pulses {
            ctx.bus().publish(pulse.clone());
        }

        RouteDecision::single(decision.into_signal())
    }
}
```

---

## 4. The OODA Loop as a Loop Graph

### OODA mapped to kernel patterns

The conductor's evaluation cycle is an OODA loop: Observe (read Signal stream), Orient (watchers classify), Decide (policy selects intervention), Act (orchestrator executes). In the unified kernel, this is a **Loop Graph** -- a Graph with a feedback edge from the exit node back to the entry node.

```
                    +---> Observe ----+
                    |                 |
                    |                 v
                Act <---- Orient ----+
                    |                 |
                    |                 v
                    +---- Decide <----+
```

The Loop Graph wraps the Verify Pipeline from Section 3:

```rust
/// The conductor OODA Loop. A Loop Graph wrapping the Verify Pipeline.
fn build_ooda_loop(pipeline: Graph) -> Graph {
    let mut loop_graph = Graph::new("conductor.ooda", "1.0.0");

    // Observe: Lens Cell that reads the agent Signal stream from Store.
    let observe = loop_graph.add_node(Node::cell(
        CellId::from_name("observe.agent_stream"),
    ));

    // Orient + Decide: the Verify Pipeline from Section 3.
    let pipeline_node = loop_graph.add_node(Node::subgraph(pipeline));

    // Act: a React Cell that publishes the ConductorDecision as a Pulse
    // and optionally writes intervention Signals to Store.
    let act = loop_graph.add_node(Node::cell(
        CellId::from_name("react.conductor_act"),
    ));

    // Forward edges.
    loop_graph.add_edge(observe, pipeline_node);
    loop_graph.add_edge(pipeline_node, act);

    // Feedback edge: the Loop pattern. Act's output feeds back to
    // Observe for the next tick. The feedback edge carries the
    // ConductorDecision and any cognitive Pulses, which Observe uses
    // to filter the next window of the Signal stream.
    loop_graph.add_feedback_edge(act, observe);

    // Loop termination condition: the Loop exits when the
    // ConductorDecision is Fail (terminal) or when the agent's
    // Flow completes (external cancellation via Bus).
    loop_graph.set_policy(GraphPolicy {
        loop_condition: LoopCondition::UntilSignal {
            kind: "conductor.terminal".into(),
        },
        max_iterations: Some(10_000), // Safety bound
        ..Default::default()
    });

    loop_graph.set_entry(vec![observe]);
    loop_graph.set_exit(vec![act]);

    loop_graph
}
```

### The Observe Cell (Lens)

The Observe Cell is an **Observe protocol** Cell (a Lens). It reads the agent's Signal stream from Store and filters it into the window that the Verify Pipeline needs. The Lens does not modify Signals -- it selects.

```rust
/// The Observe Cell for the conductor OODA loop. Reads the agent's
/// Signal stream from Store and produces a windowed view for the
/// Verify Pipeline.
pub struct AgentStreamLens {
    /// How many recent Signals to include in the window.
    window_size: usize,
    /// Agent ID whose stream to observe.
    agent_id: AgentId,
}

impl Observe for AgentStreamLens {
    fn observe(&self, ctx: &CellContext) -> Vec<Signal> {
        // Read from Store: the agent's Signal partition.
        let stream = ctx.store().query(
            StoreQuery::by_agent(self.agent_id)
                .latest(self.window_size)
        );

        // Also read the previous ConductorDecision from the feedback
        // edge (if this is not the first iteration).
        let prev_decision = ctx.feedback_signals()
            .iter()
            .find(|s| s.kind == "conductor.decision");

        // If the previous decision was Restart, include the restart
        // reason in the output so the Verify Cells can track
        // post-restart behavior.
        let mut output = stream;
        if let Some(decision) = prev_decision {
            output.push(decision.clone());
        }

        output
    }
}
```

### The Act Cell (React)

The Act Cell is a **React protocol** Cell. It receives the ConductorDecision and publishes it as a Pulse on Bus. It also writes the decision as a Signal to Store for persistence.

```rust
/// The Act Cell for the conductor OODA loop. Publishes the
/// ConductorDecision as a Pulse and persists it as a Signal.
pub struct ConductorActCell;

impl React for ConductorActCell {
    fn react(&self, input: &[Signal], ctx: &CellContext) -> Vec<Signal> {
        let decision = extract_decision(input);

        // Publish as Pulse on Bus (ephemeral, for the orchestrator).
        ctx.bus().publish(Pulse {
            kind: PulseKind::ConductorDecision,
            payload: decision.clone(),
            source: self.id(),
        });

        // Publish cognitive Pulses derived from the Verdicts.
        let verdicts = extract_verdicts(input);
        for pulse in derive_cognitive_pulses(&verdicts) {
            ctx.bus().publish(pulse);
        }

        // Write to Store (durable, for persistence and replay).
        let signal = Signal::from_body(decision)
            .with_kind("conductor.decision")
            .with_tag("plan_id", &ctx.plan_id());
        ctx.store().write(&signal);

        // Return the decision Signal for the feedback edge.
        vec![signal]
    }
}
```

---

## 5. Three Timescales as Nested Loops

### Gamma, Theta, Delta

The conductor operates at three frequencies, each approximately 15x slower than the one below it. In the unified kernel, these are three nested Loop Graphs, each wrapping the one below:

| Timescale | Period | What it does | Loop Graph |
|---|---|---|---|
| **Gamma** (tactical) | ~5s per tick | Per-turn watcher evaluation. Runs the full 10-watcher Verify Pipeline. | Inner loop: the OODA Loop from Section 4. |
| **Theta** (operational) | ~75s per tick | Per-task health assessment. Runs the HealthMonitor, MetaCognition stuck detector, and Yerkes-Dodson pressure adjustment. | Middle loop: wraps Gamma. |
| **Delta** (strategic) | ~20min per tick | Per-batch learning. Updates ThresholdLearner, persists circuit breaker state, triggers dream consolidation for compound patterns. | Outer loop: wraps Theta. |

```
Delta Loop (strategic, ~20min)
  |
  +---> Theta Loop (operational, ~75s)
          |
          +---> Gamma Loop (tactical, ~5s)
                  |
                  +---> Observe -> Verify Pipeline -> Route -> Act
                  |                                              |
                  +<----- feedback edge (ConductorDecision) <----+
          |                                                      |
          +<----- Theta feedback (health, pressure) <-----------+
  |                                                              |
  +<----- Delta feedback (thresholds, circuit breaker) <--------+
```

### Parameter cascade

Each outer loop adjusts parameters of the inner loops. This is the **Rack pattern** (macro knobs controlling micro behavior):

```rust
/// Theta loop: per-task operational assessment.
/// Runs every ~75s (15x gamma). Adjusts gamma loop parameters.
pub struct ThetaLoopCell {
    health_monitor: HealthMonitor,
    yerkes_dodson: YerkesDodson,
    stuck_detector: StuckDetector,
}

impl Cell for ThetaLoopCell {
    fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Collect gamma loop decisions from the last 15 ticks.
        let gamma_decisions = extract_gamma_decisions(&input);

        // Run health monitor.
        let snapshot = build_snapshot_from_signals(&input);
        let health = self.health_monitor.overall_status(&snapshot);

        // Update Yerkes-Dodson pressure.
        let pressure = compute_pressure(&gamma_decisions);
        self.yerkes_dodson.set_pressure(pressure);
        let aggressiveness = self.yerkes_dodson.intervention_aggressiveness();

        // Check for stuck patterns across multiple gamma ticks.
        let stuck = self.stuck_detector.assess(&gamma_decisions);

        // Emit parameter adjustments as Signals for the gamma loop.
        let mut output = Vec::new();

        // Adjust watcher sensitivity based on Yerkes-Dodson.
        output.push(Signal::parameter_adjustment(
            "gamma.watcher_sensitivity",
            aggressiveness,
        ));

        // If stuck at theta level, emit Explore or Escalate Pulse.
        if let Some(action) = stuck {
            ctx.bus().publish(Pulse {
                kind: PulseKind::CognitiveSignal(match action {
                    MetaCognitionAction::Explore => CognitiveSignal::Explore {
                        budget_multiplier: 2.0,
                    },
                    MetaCognitionAction::Escalate => CognitiveSignal::Escalate {
                        to_tier: 3,
                    },
                    MetaCognitionAction::Abort => CognitiveSignal::Shutdown {
                        reason: "theta-level stuck detection".into(),
                    },
                }),
                source: self.id(),
            });
        }

        output.push(Signal::health_status(health));
        Ok(output)
    }
}

/// Delta loop: per-batch strategic learning.
/// Runs every ~20min (15x theta). Updates adaptive thresholds
/// and circuit breaker state.
pub struct DeltaLoopCell {
    threshold_learner: ThresholdLearner,
}

impl Cell for DeltaLoopCell {
    fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Collect all intervention outcomes since last delta tick.
        let outcomes = extract_intervention_outcomes(&input);

        // Update adaptive thresholds.
        for outcome in &outcomes {
            self.threshold_learner.record_outcome(outcome.clone());
        }

        // Persist threshold learner state to Store.
        ctx.store().write(&Signal::from_body(&self.threshold_learner)
            .with_kind("conductor.thresholds"));

        // Check for compound patterns that should trigger dream
        // consolidation.
        let compound_patterns = extract_compound_patterns(&input);
        if !compound_patterns.is_empty() {
            ctx.bus().publish(Pulse {
                kind: PulseKind::DreamTrigger {
                    reason: "compound conductor patterns".into(),
                    patterns: compound_patterns,
                },
                source: self.id(),
            });
        }

        // Emit updated thresholds as parameter Signals for theta/gamma.
        let threshold_updates = self.threshold_learner
            .watcher_thresholds
            .iter()
            .map(|(name, threshold)| {
                Signal::parameter_adjustment(
                    &format!("gamma.{name}.threshold"),
                    threshold.ema,
                )
            })
            .collect();

        Ok(threshold_updates)
    }
}
```

---

## 6. Cognitive Signals as Pulse Kinds

### No function calls, only Bus Pulses

The 8 cognitive signals are not method calls on the Conductor. They are Pulse kinds published on Bus. Any Cell in the system can subscribe to them. The orchestrator subscribes to `PulseKind::CognitiveSignal` and translates them into concrete actions.

```rust
/// The 8 cognitive signal Pulse kinds. Published on Bus by the
/// conductor Loop, consumed by the orchestrator and other subsystems.
pub enum CognitiveSignal {
    /// Pause the current agent. Subscriber: orchestrator.
    Pause,

    /// Resume a paused agent. Subscriber: orchestrator.
    Resume,

    /// Change task priority ordering. Subscriber: plan executor.
    Reprioritize { reason: String },

    /// Inject context into the agent's next prompt.
    /// Subscriber: prompt builder.
    InjectContext { context: String },

    /// Escalate to a higher model tier.
    /// Subscriber: cascade router.
    Escalate { to_tier: u32 },

    /// Reduce pressure: extend budgets, slow tick frequency.
    /// Subscriber: budget tracker, adaptive clock.
    Cooldown { factor: f64 },

    /// Try alternative approaches: increase exploration budget.
    /// Subscriber: cascade router, prompt builder.
    Explore { budget_multiplier: f64 },

    /// Terminate the agent/plan.
    /// Subscriber: orchestrator.
    Shutdown { reason: String },
}

/// Derive cognitive Pulses from watcher Verdicts.
/// Published by the Act Cell in the OODA Loop.
fn derive_cognitive_pulses(verdicts: &[Verdict]) -> Vec<Pulse> {
    let mut pulses = Vec::new();

    let has_cost_pressure = has_failed_verdict(verdicts, "verify.cost_overrun");
    let has_context_pressure = has_failed_verdict(verdicts, "verify.ctx_pressure");
    let has_time_pressure = has_failed_verdict(verdicts, "verify.time_overrun");
    let has_quality_issue = has_any_failed(verdicts, &[
        "verify.compile_fail", "verify.test_budget", "verify.spec_drift",
    ]);
    let has_stuck = has_any_failed(verdicts, &[
        "verify.ghost_turn", "verify.iter_loop", "verify.stuck_pattern",
    ]);

    // Context pressure -> suggest trimming.
    if has_context_pressure {
        pulses.push(Pulse::cognitive(CognitiveSignal::InjectContext {
            context: "Context window pressure detected. Consider trimming history.".into(),
        }));
    }

    // Cost + time pressure -> slow down.
    if has_cost_pressure || has_time_pressure {
        pulses.push(Pulse::cognitive(CognitiveSignal::Cooldown { factor: 1.3 }));
    }

    // Quality issues without being stuck -> try stronger model.
    if has_quality_issue && !has_stuck {
        pulses.push(Pulse::cognitive(CognitiveSignal::Escalate { to_tier: 2 }));
    }

    // Stuck -> explore alternatives.
    if has_stuck {
        pulses.push(Pulse::cognitive(CognitiveSignal::Explore {
            budget_multiplier: 1.5,
        }));
    }

    // Multiple resource watchers -> reprioritize.
    let resource_count = [has_cost_pressure, has_context_pressure, has_time_pressure]
        .iter().filter(|&&v| v).count();
    if resource_count >= 2 {
        pulses.push(Pulse::cognitive(CognitiveSignal::Reprioritize {
            reason: "Multiple resource watchers firing. Consider reordering queue.".into(),
        }));
    }

    pulses
}
```

### Subscription model

Cognitive Pulses flow through Bus, not through return values. Any Cell in the system can subscribe:

```rust
// In the orchestrator's setup:
bus.subscribe(PulseKind::CognitiveSignal, |pulse| {
    match pulse.cognitive_signal() {
        CognitiveSignal::Shutdown { reason } => {
            // Cancel the agent Flow.
            engine.cancel(flow_id, reason);
        }
        CognitiveSignal::Escalate { to_tier } => {
            // Tell the cascade router to use a higher tier next time.
            cascade_router.set_min_tier(to_tier);
        }
        CognitiveSignal::Cooldown { factor } => {
            // Extend the budget.
            budget_tracker.extend_by_factor(factor);
        }
        // ...
    }
});

// The cascade router subscribes independently:
bus.subscribe(PulseKind::CognitiveSignal, |pulse| {
    if let CognitiveSignal::Explore { budget_multiplier } = pulse.cognitive_signal() {
        // Increase exploration probability in LinUCB.
        self.exploration_budget *= budget_multiplier;
    }
});
```

---

## 7. Routing Bias as a Cross-Cut Functor

The conductor's routing bias (deprioritize certain models, prefer cheaper tiers) is a **Functor pattern** -- a cross-cut that enriches Signals passing through the Route protocol. Instead of the conductor maintaining a `Mutex<RoutingBias>`, it publishes a `RoutingBias` Pulse on Bus. The cascade router subscribes and adjusts its next routing decision.

```rust
/// Routing bias as a cross-cut Functor. Published on Bus by the
/// conductor, consumed by the cascade router.
pub struct RoutingBiasFunctor;

impl Functor for RoutingBiasFunctor {
    fn enrich(&self, signal: &mut Signal, ctx: &CellContext) {
        // Read the most recent RoutingBias from Bus.
        if let Some(bias) = ctx.bus().latest(PulseKind::RoutingBias) {
            signal.tags.insert("routing.prefer_cheaper", bias.prefer_cheaper);
            for model in &bias.deprioritize {
                signal.tags.append("routing.deprioritize", model);
            }
        }
    }
}
```

The cascade router reads these tags during its Route decision, rather than calling `conductor.routing_bias()`. This eliminates the shared mutable state between the conductor and router.

---

## 8. TOML Definition

The entire conductor can be defined in TOML, loaded and validated at startup, and modified without recompilation:

```toml
[graph]
name = "conductor.gamma"
version = "1.0.0"
pattern = "loop"

[graph.policy]
loop_condition = { until_signal = "conductor.terminal" }
max_iterations = 10000
tick_interval_ms = 5000

# Observe Cell (Lens)
[[graph.nodes]]
id = "observe"
cell = "observe.agent_stream"
params = { window_size = 50 }

# FanOut to all watchers
[[graph.nodes]]
id = "fanout"
kind = "fanout"

# 10 watcher Verify Cells
[[graph.nodes]]
id = "ghost_turn"
cell = "verify.ghost_turn"
params = { max_ghost_turns = 3 }

[[graph.nodes]]
id = "compile_fail"
cell = "verify.compile_fail"
params = { max_compile_fail_repeat = 3 }

[[graph.nodes]]
id = "cost_overrun"
cell = "verify.cost_overrun"
params = { budget_usd = 10.0 }

[[graph.nodes]]
id = "iter_loop"
cell = "verify.iteration_loop"
params = { max_iteration_loop = 3 }

[[graph.nodes]]
id = "review_loop"
cell = "verify.review_loop"
params = { max_review_cycles = 3 }

[[graph.nodes]]
id = "spec_drift"
cell = "verify.spec_drift"
params = { max_drift_ratio = 0.25 }

[[graph.nodes]]
id = "stuck_pattern"
cell = "verify.stuck_pattern"
params = { max_stuck_repeats = 4 }

[[graph.nodes]]
id = "test_budget"
cell = "verify.test_budget"
params = { min_failure_increase = 1 }

[[graph.nodes]]
id = "time_overrun"
cell = "verify.time_overrun"
params = { alert_threshold = 0.80 }

[[graph.nodes]]
id = "ctx_pressure"
cell = "verify.context_pressure"
params = { max_context_usage_ratio = 0.80 }

# FanIn: collect Verdicts
[[graph.nodes]]
id = "fanin"
kind = "fanin"

# Pattern detector
[[graph.nodes]]
id = "pattern_detector"
cell = "verify.pattern_detector"
params = { hysteresis_window = 2 }

# Route Cell: intervention selector
[[graph.nodes]]
id = "route"
cell = "route.intervention"
params = { policy = "worst_severity", bandit_warmup = 50 }

# Act Cell: publish decision
[[graph.nodes]]
id = "act"
cell = "react.conductor_act"

# Edges
[[graph.edges]]
from = "observe"
to = "fanout"

[[graph.edges]]
from = "fanout"
to = ["ghost_turn", "compile_fail", "cost_overrun", "iter_loop",
      "review_loop", "spec_drift", "stuck_pattern", "test_budget",
      "time_overrun", "ctx_pressure"]

[[graph.edges]]
from = ["ghost_turn", "compile_fail", "cost_overrun", "iter_loop",
        "review_loop", "spec_drift", "stuck_pattern", "test_budget",
        "time_overrun", "ctx_pressure"]
to = "fanin"

[[graph.edges]]
from = "fanin"
to = "pattern_detector"

[[graph.edges]]
from = "pattern_detector"
to = "route"

[[graph.edges]]
from = "route"
to = "act"

# Feedback edge: Loop pattern
[[graph.feedback_edges]]
from = "act"
to = "observe"
```

---

## 9. Mori-Diffs: What the Redesign Fixes

The existing conductor has four structural problems identified in mori-diffs:

| Problem | Old design | Unified design |
|---|---|---|
| Watchers cannot observe the active runner stream | Watchers receive a snapshot `&[Signal]` passed by the caller. If the caller forgets to update the snapshot, watchers see stale data. | The Observe Lens reads directly from Store. The agent's Signal stream is always up-to-date because the agent writes to Store in real time. |
| Per-watcher threshold config exists but is not loaded | `ThresholdLearner` is built but watcher Cells use hardcoded constants (`MAX_GHOST_TURNS=3`). | Each watcher Cell reads its threshold from the parameter cascade. The Delta loop writes updated thresholds; the Gamma loop's watcher Cells read them. |
| Two execution paths | `evaluate()` and `evaluate_full()` do mostly the same thing but diverge on cognitive signals. | One Loop Graph. One execution path. Cognitive Pulses are always derived (they are a side effect of the Act Cell, not a separate code path). |
| Routing bias via ad-hoc shared state | `Mutex<RoutingBias>` queried by the cascade router via `conductor.routing_bias()`. | RoutingBias published as a Pulse on Bus. The cascade router subscribes. No shared mutable state. |

---

## What This Enables

1. **Unified verification**: The same Verify protocol used for code gates (compile, test, clippy, diff) is used for agent supervision. A "gate pipeline" and a "conductor" are the same thing: a Pipeline of Verify Cells. This halves the trait surface.

2. **Configuration-driven supervision**: Adding a new watcher is adding a new Verify Cell to the TOML Graph definition. No code changes to the conductor. Removing a watcher is deleting a line from the TOML. Changing thresholds is editing parameters.

3. **Composable timescales**: The three nested loops are not hardcoded. A fourth timescale (epsilon, sub-second for latency-sensitive agents) is just another nested Loop Graph. A two-timescale configuration for simple agents is just removing the Delta loop from the Graph.

4. **Decoupled cognitive signals**: Cognitive Pulses on Bus can be consumed by any subsystem, not just the orchestrator. The dream consolidation system subscribes to compound patterns. The cascade router subscribes to Escalate. The prompt builder subscribes to InjectContext. None of these require changes to the conductor.

5. **Replay and audit**: Because the conductor is a Graph executed by the Engine, every tick is recorded as a Flow with Activity records. The entire conductor decision history is replayable via `roko replay`. Each Verdict is a Signal in Store with a content hash.

---

## Feedback Loops

- **Gamma -> Theta**: Gamma Verdicts accumulate into the Theta loop's health and stuck assessments. Theta adjusts Gamma's watcher sensitivity via parameter Signals.
- **Theta -> Delta**: Theta intervention outcomes feed into Delta's threshold learning. Delta adjusts Theta's pressure model and persists circuit breaker state.
- **Delta -> Gamma**: Delta's updated adaptive thresholds cascade down through Theta to modify individual watcher Cell parameters at the Gamma level.
- **Conductor -> Cascade Router**: Routing bias Pulses and Escalate signals modulate the cascade router's model selection. The router's choices produce the agent output that the conductor observes -- closing the loop.
- **Conductor -> Dreams**: Compound patterns trigger dream consolidation Pulses. Dreams produce new knowledge Signals that inform future watcher evaluations.
- **Threshold learning**: Every intervention records whether the task improved afterward. Effective interventions lower thresholds (intervene earlier). Ineffective interventions raise thresholds (intervene later). EMA smoothing with alpha=0.1. Warmup period of 10 observations before adaptive thresholds override defaults.

---

## Open Questions

1. **Watcher Cell ordering within the FanOut**: All 10 watchers run in parallel via FanOut. But the pattern detector depends on all 10 completing. Should the FanIn have a timeout? If one watcher is slow (e.g., spec drift requires diffing files), should the others proceed without it?

2. **Feedback edge serialization**: The feedback edge from Act to Observe carries the previous ConductorDecision. If the Loop is serialized to TOML and deserialized later, does the feedback state need to be persisted separately or can it be reconstructed from Store?

3. **Bandit policy cold start**: The BanditPolicy delegates to WorstSeverityPolicy during warmup (first 50 observations). When the system migrates from the old `Conductor` struct to the new Loop Graph, should the bandit's historical data be migrated, or should it restart from cold?

4. **Cross-family escalation threshold**: Currently, any 2 families firing at Warning escalates to Critical. Should this be configurable? Should it require 2 families with at least 2 watchers each? The current behavior can produce false Critical escalations when a single stressor affects both Resource and Progress families simultaneously (e.g., cost overrun + ghost turns from a slow model).

5. **Gamma tick frequency adaptation**: The current design uses a fixed 5s tick. Should the Theta loop be able to increase the Gamma tick rate when it detects the agent is making rapid progress (more events per second), and decrease it during idle periods?
