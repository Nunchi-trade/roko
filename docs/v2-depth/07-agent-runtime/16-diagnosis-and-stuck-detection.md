# Diagnosis and Stuck Detection

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How error classification and stuck detection emerge as Route and Observe Cells rather than pattern-matching functions.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal/Pulse duality), [02-CELL](../../unified/02-CELL.md) (9 protocols, predict-publish-correct, Verify redesign), [05-AGENT](../../unified/05-AGENT.md) (Agent lifecycle, cognitive timescales, CorticalState), [07-LEARNING](../../unified/07-LEARNING.md) (L1-L4 loop taxonomy, predict-publish-correct)

**Source docs**: `docs/07-conductor/04-diagnosis-engine.md`, `docs/07-conductor/05-stuck-detection.md`, `docs/07-conductor/08-good-regulator-self-model.md`

---

## 1. The Redesign Thesis

The original diagnosis engine is a function: `fn diagnose(&str) -> Vec<Diagnosis>`. The original stuck detector is a struct with six threshold comparisons. Both work. Neither composes with anything else. Neither learns. Neither participates in predict-publish-correct.

The unified redesign makes three structural changes:

1. **Diagnosis becomes a Route Cell.** It accepts error Signals, scores candidate categories, and routes to the appropriate intervention Cell. It conforms to the Route protocol, so every mechanism that works with Route Cells (EFE routing, Thompson sampling, LinUCB bandits) works with diagnosis.

2. **Stuck detection becomes a Lens (Observe Cell).** It observes the agent's trajectory via Bus subscription. It publishes observations as Pulses on telemetry topics. It does not take action. Downstream React Cells consume its observations and decide interventions.

3. **MetaCognition becomes a Loop.** Observe, classify, decide, act, and then feed the outcome back to update the classification model. This is predict-publish-correct applied to supervision rather than task execution.

The consequence: diagnosis and stuck detection are no longer special subsystems. They are Cells in a Graph, wired with edges, subject to the same execution semantics as everything else. They compose. They snapshot. They resume. They appear in telemetry as first-class Cells with input/output types and cost estimates.

---

## 2. Error Categories as Signal Kinds

The 20 error categories become a Kind taxonomy within the Signal system. An error is not a string to be parsed -- it is a Signal with a typed Kind that carries structured context.

```rust
/// Error category as a Signal Kind discriminant.
/// Each variant maps 1:1 to the original ErrorCategory enum,
/// but now participates in the Signal algebra (hashing, lineage, scoring).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorKind {
    // Rust compiler errors (6)
    CompileError,
    TypeMismatch,
    BorrowCheckerError,
    LifetimeError,
    ImportError,
    ClippyWarning,
    // Test and verification (1)
    TestFailure,
    // Filesystem (3)
    MissingFile,
    PermissionDenied,
    DiskFull,
    // Infrastructure (3)
    NetworkError,
    TimeoutError,
    OomError,
    // LLM provider (3)
    LlmRateLimit,
    LlmContextOverflow,
    LlmRefusal,
    // Process (2)
    ProcessCrash,
    LoopDetected,
    // VCS (1)
    GitConflict,
    // Dependencies (1)
    DependencyError,
}
```

When a Verify Cell (CompileGate, TestGate, etc.) produces a failing Verdict, the orchestrator wraps the error text into an error Signal:

```rust
/// Construct an error Signal from raw gate output.
/// The Signal carries the raw text as payload and ErrorKind::Unknown
/// until the DiagnosisRoute Cell classifies it.
fn error_signal_from_verdict(verdict: &Verdict, task_id: &str) -> Signal {
    Signal::builder(Kind::Error(ErrorKind::Unknown))
        .payload(json!({
            "raw_text": verdict.error_digest(),
            "gate_name": verdict.gate_name(),
            "task_id": task_id,
            "duration_ms": verdict.duration_ms(),
        }))
        .tag("source", "gate-pipeline")
        .build()
}
```

The `ErrorKind::Unknown` is the initial classification. The DiagnosisRoute Cell reclassifies it. This two-phase approach means the error Signal exists in the system immediately (for logging, for telemetry) without waiting for classification to complete.

---

## 3. Pattern Matching as a Score Cell

The 34 substring patterns are not a function -- they are a Score Cell. The Score protocol rates an input Signal along dimensions. Here, the dimension is "confidence that this error belongs to category X."

```rust
/// Score Cell: rates an error Signal against all 34 known patterns.
/// Produces a scored list of candidate ErrorKinds, ordered by confidence.
///
/// Conforms to: Score protocol (see 02-CELL.md S3)
/// Input: Signal with Kind::Error (raw error text in payload)
/// Output: Signal with Kind::Scored carrying Vec<CategoryScore>
pub struct PatternMatchScore {
    id: CellId,
    /// 34 patterns, each mapping a substring to a category + base confidence.
    patterns: Vec<ErrorPattern>,
}

pub struct ErrorPattern {
    /// Case-insensitive substring to match.
    substring: &'static str,
    /// The error category this pattern indicates.
    category: ErrorKind,
    /// Base confidence when this pattern matches (0.0 to 1.0).
    /// Specific patterns (error codes) get 0.95.
    /// Generic patterns ("cannot find") get 0.70.
    base_confidence: f64,
}

pub struct CategoryScore {
    pub category: ErrorKind,
    pub confidence: f64,
    pub matched_pattern: String,
}
```

The Score Cell's `execute` implementation:

```rust
impl Cell for PatternMatchScore {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "pattern-match-score" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn capabilities(&self) -> &Capabilities { Capabilities::pure() }
    fn estimated_cost(&self) -> Cost { Cost::ZERO }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut results = Vec::new();

        for signal in &input {
            let raw_text = signal.payload_str("raw_text")
                .unwrap_or_default();
            let lower = raw_text.to_lowercase();

            let mut scores: Vec<CategoryScore> = self.patterns.iter()
                .filter(|p| lower.contains(&p.substring.to_lowercase()))
                .map(|p| CategoryScore {
                    category: p.category,
                    confidence: p.base_confidence,
                    matched_pattern: p.substring.to_string(),
                })
                .collect();

            // Sort by confidence descending
            scores.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal));

            // If no patterns matched, emit Unknown with low confidence
            if scores.is_empty() {
                scores.push(CategoryScore {
                    category: ErrorKind::CompileError,
                    confidence: 0.30,
                    matched_pattern: "(no pattern matched)".into(),
                });
            }

            results.push(Signal::builder(Kind::Scored)
                .payload(json!({ "scores": scores }))
                .source(signal.id)
                .build());
        }

        Ok(results)
    }
}
```

**Why a Score Cell and not a function.** As a Cell, PatternMatchScore:

- Has a cost estimate (`Cost::ZERO` -- pure string matching, no I/O).
- Has typed input/output (error Signal in, scored Signal out).
- Can be replaced with a learned scorer (an LLM-based classifier, a trained model) without changing the downstream Route Cell.
- Participates in telemetry (execution time, input/output counts).
- Can be composed in parallel with other Score Cells (e.g., an LLM-based scorer for ambiguous cases) via FanOut, with results merged.

### 3.1 The 34 Patterns Organized by Category

The pattern set derives from production data (March-April 2026 batch runs). Coverage is approximately 95% of observed errors by frequency.

| Category | Patterns | Confidence | Production Frequency |
|---|---|---|---|
| ImportError | `"error[E0432]"`, `"error[E0433]"`, `"cannot find"`, `"unresolved import"` | 0.70-0.95 | 35% |
| CompileError | `"error[E0063]"`, `"could not compile"`, `"aborting due to"` | 0.85-0.90 | 20% |
| TypeMismatch | `"error[E0308]"`, `"expected"` + `"found"` | 0.70-0.95 | 15% |
| TestFailure | `"test result: FAILED"`, `"panicked at"`, `"assertion failed"` | 0.85-0.90 | 12% |
| BorrowCheckerError | `"error[E0382]"`, `"error[E0505]"`, `"error[E0507]"` | 0.95 | 5% |
| LifetimeError | `"error[E0106]"`, `"error[E0495]"`, `"error[E0621]"` | 0.95 | 4% |
| LlmRateLimit | `"rate limit"`, `"429"`, `"quota exceeded"` | 0.80-0.90 | 3% |
| All others | Remaining ~12 patterns across 13 categories | 0.80-0.95 | 6% |

### 3.2 Confidence-Weighted Routing

The Score Cell produces confidence values that the downstream Route Cell uses for tiered dispatch:

| Confidence Range | Routing Behavior |
|---|---|
| > 0.9 | Route directly to intervention. No review needed. |
| 0.6 - 0.9 | Route to intervention but flag for review in telemetry. |
| < 0.6 | Fall back to RestartAgent (the safest generic intervention). |

This tiered routing reduces false-positive auto-fixes without losing the cost savings on high-confidence classifications.

---

## 4. Diagnosis as a Route Cell

The DiagnosisRoute Cell takes scored error Signals and routes them to the appropriate intervention Cell. It conforms to the Route protocol (see [02-CELL.md](../../unified/02-CELL.md)).

```rust
/// Route Cell: given a scored error Signal, select the intervention Cell.
///
/// Conforms to: Route protocol (see 02-CELL.md S4)
/// Input: Signal with Kind::Scored (from PatternMatchScore)
/// Output: Signal with Kind::Intervention carrying the selected action
///
/// The routing decision is a simple lookup table today,
/// replaceable with a learned policy (bandit, EFE routing) tomorrow.
pub struct DiagnosisRoute {
    id: CellId,
    /// Category-to-intervention mapping. Static today, learned tomorrow.
    intervention_map: HashMap<ErrorKind, InterventionKind>,
}

/// The 9 interventions as Cell-addressable actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InterventionKind {
    RetryWithContext,
    AutoFix,
    RestartAgent,
    AbortPlan,
    BackoffRetry,
    MergeResolution,
    ReduceContext,
    SwitchModel,
    WarnAndContinue,
}
```

The Route Cell's execute:

```rust
impl Cell for DiagnosisRoute {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "diagnosis-route" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }
    fn capabilities(&self) -> &Capabilities { Capabilities::pure() }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut results = Vec::new();

        for signal in &input {
            let scores: Vec<CategoryScore> = signal.payload_vec("scores")?;
            let best = scores.first()
                .ok_or_else(|| CellError::MissingInput("no scores"))?;

            // Look up the intervention for this category
            let intervention = self.intervention_map
                .get(&best.category)
                .copied()
                .unwrap_or(InterventionKind::RetryWithContext);

            // Apply confidence-based routing tier
            let effective_intervention = if best.confidence < 0.6 {
                InterventionKind::RestartAgent  // low confidence -> safe fallback
            } else {
                intervention
            };

            results.push(Signal::builder(Kind::Intervention)
                .payload(json!({
                    "intervention": effective_intervention,
                    "category": best.category,
                    "confidence": best.confidence,
                    "pattern": best.matched_pattern,
                    "raw_text": signal.payload_str("raw_text").unwrap_or_default(),
                }))
                .source(signal.id)
                .build());
        }

        Ok(results)
    }
}
```

### 4.1 Category-to-Intervention Mapping

The static mapping encodes production-calibrated heuristics:

| Category | Intervention | Rationale |
|---|---|---|
| CompileError | RetryWithContext | Agent may fix with error details |
| TestFailure | RetryWithContext | Agent needs test output |
| TypeMismatch | RetryWithContext | Agent needs expected/found types |
| BorrowCheckerError | RestartAgent | Borrow errors require fresh approach |
| LifetimeError | RestartAgent | Lifetime errors are structurally difficult |
| ImportError | AutoFix | Missing imports are cheap to fix (95% auto-fix rate) |
| MissingFile | RetryWithContext | Agent may need to create the file |
| PermissionDenied | AbortPlan | Cannot fix permissions from agent context |
| NetworkError | BackoffRetry | Likely transient |
| TimeoutError | BackoffRetry | Likely transient |
| OomError | AbortPlan | Resource exhaustion requires operator action |
| DiskFull | AbortPlan | Resource exhaustion requires cleanup |
| LlmRateLimit | BackoffRetry | Wait for rate limit window |
| LlmContextOverflow | ReduceContext | Compact and retry |
| LlmRefusal | SwitchModel | Try a different model |
| ProcessCrash | RestartAgent | Agent died; respawn |
| LoopDetected | RestartAgent | Agent is stuck; fresh context needed |
| ClippyWarning | WarnAndContinue | Non-blocking |
| GitConflict | MergeResolution | Spawn merge resolver |
| DependencyError | RetryWithContext | Agent may fix with dependency info |

### 4.2 Why Route and Not a Match Statement

A `match` statement on ErrorKind would produce the same mapping. The Route Cell provides three things a match statement does not:

1. **Replaceability.** The Route Cell is wired by Graph edges. Swapping it for a learned policy (ConductorBandit, Thompson sampling) requires changing the Graph definition, not the code. The upstream Score Cell and downstream intervention Cells remain unchanged.

2. **Predict-publish-correct.** The Route Cell can publish its routing prediction as a Pulse before the intervention executes. When the intervention outcome is known, a CalibrationPolicy joins prediction and outcome to compute routing error. Over time, the routing learns which interventions actually work for which categories (see [07-LEARNING.md](../../unified/07-LEARNING.md)).

3. **EFE-aware routing.** When vitality is low, the Route Cell can weight cost more heavily: AutoFix ($0.01) over RestartAgent ($2.00). When vitality is high, it can prefer RestartAgent for reliability. This is the same EFE mechanism that drives T0/T1/T2 tier selection (see [cognitive-timescales.md](cognitive-timescales.md)), applied to intervention selection.

---

## 5. Auto-Fix as a Lightweight Compose Cell

The AutoFix intervention is the system's highest-leverage optimization. Production data shows: ImportError at 35% frequency, 95% auto-fixable, $0.01 per fix vs. $2.00+ per full re-implementation. For an 8-error batch, routing import errors to auto-fix saves approximately $11.94.

AutoFix is a Compose Cell: it assembles a minimal fix prompt and routes to a cheap model (Haiku-class).

```rust
/// Compose Cell: generates a lightweight fix Signal for auto-fixable errors.
///
/// Conforms to: Compose protocol (see 02-CELL.md S5)
/// Input: Signal with Kind::Intervention where intervention == AutoFix
/// Output: Signal with Kind::Action containing the fix prompt
///
/// Cost: ~$0.01 (Haiku-class model, minimal context)
/// Compared to: ~$2.00+ for full agent re-dispatch
pub struct AutoFixCompose {
    id: CellId,
    /// Model to use for auto-fix. Defaults to cheapest available.
    model_tier: ModelTier,
    /// Maximum tokens for auto-fix prompt.
    max_prompt_tokens: usize,
    /// Maximum tokens for auto-fix response.
    max_response_tokens: usize,
}

impl Cell for AutoFixCompose {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "auto-fix-compose" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }
    fn capabilities(&self) -> &Capabilities {
        Capabilities::builder()
            .requires(Capability::LlmCall)  // needs a model call
            .build()
    }
    fn estimated_cost(&self) -> Cost { Cost::microcents(1000) } // ~$0.01

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut results = Vec::new();

        for signal in &input {
            let category: ErrorKind = signal.payload_parse("category")?;
            let raw_text: String = signal.payload_str("raw_text")
                .unwrap_or_default().to_string();

            // Generate a minimal fix prompt based on the error category
            let fix_prompt = match category {
                ErrorKind::ImportError => format!(
                    "Fix the following import error. Add only the necessary \
                     `use` statement. Do not modify any other code.\n\n\
                     Error:\n{raw_text}"
                ),
                ErrorKind::CompileError => format!(
                    "Fix the following compile error. Make the minimal change \
                     to resolve it. Do not refactor.\n\n\
                     Error:\n{raw_text}"
                ),
                ErrorKind::TypeMismatch => format!(
                    "Fix the following type mismatch. Add a conversion or \
                     change the type annotation. Minimal change only.\n\n\
                     Error:\n{raw_text}"
                ),
                _ => format!(
                    "Fix the following error with the minimal possible change.\n\n\
                     Error:\n{raw_text}"
                ),
            };

            results.push(Signal::builder(Kind::Action)
                .payload(json!({
                    "action": "auto_fix",
                    "prompt": fix_prompt,
                    "model_tier": self.model_tier,
                    "max_tokens": self.max_response_tokens,
                    "category": category,
                }))
                .source(signal.id)
                .build());
        }

        Ok(results)
    }
}
```

### 5.1 Auto-Fix Economics

The cost differential is the reason the entire diagnosis system exists. Without diagnosis, every error goes through full agent re-dispatch. With diagnosis:

| Error Type | Frequency | Auto-Fix Rate | Auto-Fix Cost | Re-Dispatch Cost | Savings per Error |
|---|---|---|---|---|---|
| ImportError | 35% | 95% | $0.01 | $2.00 | $1.99 |
| CompileError (E0063) | ~5% | 80% | $0.01 | $2.00 | $1.99 |
| TypeMismatch | 15% | 50% | $0.01 | $2.00 | $1.99 (when fixable) |

For an 8-error batch with typical distribution (3 ImportError, 2 CompileError, 1 TypeMismatch, 1 TestFailure, 1 other):
- Without diagnosis: 8 x $2.00 = $16.00
- With diagnosis: 3 x $0.01 + 1 x $0.01 + 1 x $0.01 + 3 x $2.00 = $6.05
- Savings: $9.95 per batch, dominated by the ImportError category

The savings compound across plans. A 50-task plan averaging 3 errors per task = 150 errors. At typical distribution, ~52 are ImportErrors routed to auto-fix, saving ~$103.

---

## 6. Stuck Detection as a Lens

Stuck detection is observation, not action. It watches the agent's trajectory and publishes what it sees. The Lens pattern (Observe protocol, see [02-CELL.md](../../unified/02-CELL.md)) is exactly right: read-only, no side effects, publishes Pulses on telemetry topics.

### 6.1 The Six Stuck Kinds

Each stuck kind is a distinct Lens Cell. Separating them allows independent thresholds, independent telemetry, and independent threshold adaptation.

```rust
/// The six modes of stuck-ness, each detected by its own Lens Cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StuckKind {
    /// Same output hash for N consecutive turns.
    OutputLoop,
    /// No file changes for N milliseconds.
    NoProgress,
    /// Gate failures oscillate without net improvement.
    GateLoop,
    /// Compile error fingerprints cycle.
    CompileLoop,
    /// Agent produces text but no tool calls or file changes.
    EmptyOutput,
    /// Same operation retried N times without approach change.
    ExcessiveRetries,
}
```

### 6.2 The Stuck Lens Cells

Each Lens Cell subscribes to Bus Pulses, maintains a small amount of state (ring buffers, counters, hashes), and publishes an observation Pulse when the stuck condition is met.

```rust
/// Lens Cell: detects output loops by hashing consecutive agent turns.
///
/// Conforms to: Observe protocol
/// Subscribes to: "agent.turn.{agent_id}" Pulses on Bus
/// Publishes to: "telemetry.stuck.output_loop"
///
/// State: ring buffer of the last N turn content hashes.
/// Cost: O(1) per turn (one hash comparison).
pub struct OutputLoopLens {
    id: CellId,
    /// Number of consecutive identical hashes before triggering.
    threshold: usize,  // default: 4
    /// Per-agent ring buffer of turn hashes.
    histories: DashMap<AgentId, VecDeque<u64>>,
}

impl Cell for OutputLoopLens {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn capabilities(&self) -> &Capabilities { Capabilities::read_only() }
    fn estimated_cost(&self) -> Cost { Cost::ZERO }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut observations = Vec::new();

        for signal in &input {
            let agent_id: AgentId = signal.payload_parse("agent_id")?;
            let content_hash: u64 = signal.payload_parse("content_hash")?;

            let mut history = self.histories
                .entry(agent_id.clone())
                .or_insert_with(VecDeque::new);

            history.push_back(content_hash);
            if history.len() > self.threshold {
                history.pop_front();
            }

            // Check if the last N hashes are all identical
            if history.len() >= self.threshold
                && history.iter().all(|h| *h == content_hash)
            {
                observations.push(Signal::pulse(
                    Kind::Telemetry,
                    topic!("telemetry.stuck.output_loop"),
                    StuckObservation {
                        agent_id,
                        kind: StuckKind::OutputLoop,
                        consecutive_count: self.threshold,
                        evidence: format!(
                            "{} consecutive identical outputs (hash: {:#x})",
                            self.threshold, content_hash
                        ),
                    },
                ));
            }
        }

        Ok(observations)
    }
}
```

The five remaining Lens Cells follow the same pattern, each with its own detection logic:

```rust
/// Lens Cell: detects absence of progress by monitoring file modification times.
/// Subscribes to: filesystem events (or polls CorticalState)
/// Publishes to: "telemetry.stuck.no_progress"
/// Threshold: 300,000 ms (5 minutes) without file change.
pub struct NoProgressLens {
    id: CellId,
    threshold_ms: u64,
    last_file_change: DashMap<AgentId, Instant>,
}

/// Lens Cell: detects gate failure oscillation.
/// Subscribes to: "gate.verdict.*" Pulses
/// Publishes to: "telemetry.stuck.gate_loop"
/// Threshold: 3 oscillation cycles (fail-fix-fail with same error set).
pub struct GateLoopLens {
    id: CellId,
    threshold: usize,
    /// Per-agent rolling set of gate failure fingerprints.
    failure_histories: DashMap<AgentId, Vec<u64>>,
}

/// Lens Cell: detects compile error fingerprint cycling.
/// Subscribes to: "gate.verdict.compile" Pulses
/// Publishes to: "telemetry.stuck.compile_loop"
/// Threshold: 3 cycles of the same error fingerprint reappearing.
pub struct CompileLoopLens {
    id: CellId,
    threshold: usize,
    error_fingerprints: DashMap<AgentId, Vec<u64>>,
}

/// Lens Cell: detects turns with text output but no tool calls or file changes.
/// Subscribes to: "agent.turn.*" Pulses
/// Publishes to: "telemetry.stuck.empty_output"
/// Threshold: 3 consecutive empty-action turns.
pub struct EmptyOutputLens {
    id: CellId,
    threshold: usize,
    empty_counts: DashMap<AgentId, usize>,
}

/// Lens Cell: detects the same operation retried without approach change.
/// Subscribes to: "agent.tool_call.*" Pulses
/// Publishes to: "telemetry.stuck.excessive_retries"
/// Threshold: 6 identical operations.
pub struct ExcessiveRetriesLens {
    id: CellId,
    threshold: usize,
    operation_counts: DashMap<(AgentId, u64), usize>,  // (agent, op_hash) -> count
}
```

### 6.3 Threshold Defaults and Calibration

The defaults balance sensitivity against false positives, calibrated from production batch runs:

| Lens | Threshold | Too Low -> | Too High -> |
|---|---|---|---|
| OutputLoop | 4 turns | False positives on verification loops | Late detection (tokens wasted) |
| NoProgress | 300,000 ms (5 min) | Kills slow-but-progressing agents | Agent stalls for 10+ min |
| GateLoop | 3 cycles | Normal retry cycles flagged | Agent oscillates for 5+ cycles |
| CompileLoop | 3 cycles | Normal fix attempts flagged | Agent toggles errors for 5+ cycles |
| EmptyOutput | 3 turns | Kills agents that are reasoning | Agent describes instead of acting |
| ExcessiveRetries | 6 retries | Normal retries flagged | Agent retries 10+ times |

These thresholds participate in L1 predict-publish-correct (see section 7): each Lens publishes its threshold as a prediction, the outcome determines whether the detection was a true positive, and the threshold adapts accordingly.

### 6.4 Stuck Lens Aggregation

The six Lens Cells run as a FanOut in the theta loop (not gamma -- theta frequency matches the MetaCognitionHook's operating rate). A StuckAggregate Cell merges their observations:

```rust
/// Aggregate Cell: merges observations from all six stuck Lens Cells
/// into a single StuckAssessment per agent.
pub struct StuckAggregate {
    id: CellId,
}

pub struct StuckAssessment {
    pub agent_id: AgentId,
    pub stuck_kinds: Vec<StuckKind>,
    pub severity: StuckSeverity,
    pub recommended_action: MetaCognitionAction,
}

#[derive(Debug, Clone, Copy)]
pub enum StuckSeverity {
    /// One stuck kind detected. Likely recoverable.
    Mild,
    /// Two or more stuck kinds detected simultaneously.
    Moderate,
    /// Cycling stuck kinds (GateLoop or CompileLoop). Structural problem.
    Severe,
}

impl Cell for StuckAggregate {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Group observations by agent_id
        let mut by_agent: HashMap<AgentId, Vec<StuckKind>> = HashMap::new();
        for signal in &input {
            let obs: StuckObservation = signal.payload_parse("observation")?;
            by_agent.entry(obs.agent_id).or_default().push(obs.kind);
        }

        let mut results = Vec::new();
        for (agent_id, kinds) in by_agent {
            let severity = match kinds.len() {
                0 => continue,
                1 => {
                    if kinds.contains(&StuckKind::GateLoop)
                        || kinds.contains(&StuckKind::CompileLoop)
                    {
                        StuckSeverity::Severe
                    } else {
                        StuckSeverity::Mild
                    }
                }
                _ => StuckSeverity::Moderate,
            };

            let action = match severity {
                StuckSeverity::Mild => MetaCognitionAction::AdjustStrategy,
                StuckSeverity::Moderate => MetaCognitionAction::AdjustStrategy,
                StuckSeverity::Severe => MetaCognitionAction::Escalate,
            };

            results.push(Signal::pulse(
                Kind::Telemetry,
                topic!("telemetry.stuck.assessment"),
                StuckAssessment { agent_id, stuck_kinds: kinds, severity, action },
            ));
        }

        Ok(results)
    }
}
```

---

## 7. MetaCognition as a Loop

MetaCognition is not a hook -- it is a Loop Graph. The output of the intervention feeds back as input to the next observation cycle. This is the predict-publish-correct pattern applied to supervision.

```
MetaCognition Loop (theta frequency)
=====================================

    +-----------+     +----------------+     +----------------+
    | 6 Stuck   |---->| StuckAggregate |---->| DiagnosisRoute |
    | Lens Cells|     |                |     | (intervention  |
    | (Observe) |     | (Observe)      |     |  selection)    |
    +-----------+     +----------------+     +----------------+
         ^                                          |
         |                                          v
         |                                   +-------------+
         |                                   | Intervention |
         |                                   | Cell         |
         |                                   | (execute)    |
         |                                   +-------------+
         |                                          |
         |                                          v
         |                                   +-------------+
         |                                   | Outcome      |
         |                                   | Observer     |
         |                                   | (Lens)       |
         |                                   +-------------+
         |                                          |
         +------------------------------------------+
                    feedback edge (Loop)
```

The feedback edge carries the intervention outcome (did the restart help? did the auto-fix succeed? did the abort save resources?) back to the Stuck Lens Cells as context for the next observation cycle. This closes the loop.

### 7.1 MetaCognition Action Mapping

```rust
/// MetaCognition assessment result. Maps stuck kinds to concrete actions.
#[derive(Debug, Clone, Copy)]
pub enum MetaCognitionAction {
    /// No intervention needed. Agent is progressing normally.
    Continue,
    /// Agent needs a strategy change. Maps to Conductor Restart
    /// with enriched context from the diagnosis.
    AdjustStrategy,
    /// Problem is structural. Maps to Conductor Fail.
    /// The task needs replan, not retry.
    Escalate,
}
```

| Stuck Kind | Action | Rationale |
|---|---|---|
| OutputLoop | AdjustStrategy | Agent needs a different approach; restart with varied prompt |
| NoProgress | AdjustStrategy | Agent is stalled; refocus on the task |
| GateLoop | Escalate | Cycling indicates fundamental problem; retry will not help |
| CompileLoop | Escalate | Architectural mismatch; needs replan |
| EmptyOutput | AdjustStrategy | Agent needs more directive prompting |
| ExcessiveRetries | AdjustStrategy | Different operation or tool needed |

### 7.2 The Good Regulator Connection

The MetaCognition Loop is the system's self-model instantiated as a Graph. Per the Good Regulator Theorem (Conant & Ashby 1970): every good regulator of a system must contain a model of that system.

The six Lens Cells are the model. Each encodes an expectation about healthy execution:
- A healthy agent does not repeat the same output 4 times.
- A healthy agent changes files within 5 minutes.
- A healthy agent does not oscillate between the same gate failures.

These expectations are the system's self-model. When behavior deviates from the model, the MetaCognition Loop intervenes. The accuracy of the self-model determines the quality of intervention (see [17-adaptive-supervision-loop.md](17-adaptive-supervision-loop.md) for how the model learns).

### 7.3 Relationship to Watcher Ensemble

The six Stuck Lens Cells overlap with the 10-watcher ensemble in `roko-conductor`. This overlap is intentional and structural:

| Detection | Stuck Lens Cell | Watcher |
|---|---|---|
| Identical compile errors | CompileLoopLens | compile-fail-repeat watcher |
| Zero output | EmptyOutputLens | ghost-turn watcher |
| No file changes | NoProgressLens | (not directly covered) |
| Identical actions | OutputLoopLens | stuck-pattern watcher |
| Gate failure cycling | GateLoopLens | iteration-loop watcher |

The Lens Cells operate on unstructured agent output (raw text, hashes). The watchers operate on structured signal streams. Both feed into the MetaCognition Loop. Redundant detection at different abstraction levels increases coverage (Ashby's Law of Requisite Variety: the detector must have at least as many distinguishable states as the system has pathological states).

---

## 8. The Diagnosis Pipeline as a Graph

All the Cells above compose into a single Graph that can be defined in TOML:

```
Diagnosis Pipeline Graph (theta frequency)
============================================

                    Error Signal
                         |
                         v
               +-------------------+
               | PatternMatchScore |  (Score Cell, $0)
               +-------------------+
                         |
                         v
               +-------------------+
               | DiagnosisRoute    |  (Route Cell, $0)
               +-------------------+
                    /    |    \
                   /     |     \
                  v      v      v
           AutoFix  Retry  Restart  ... (Intervention Cells)
           ($0.01)  ($0)   ($2.00)

    Concurrently (theta frequency):

     +----------+  +----------+  +----------+
     | Output   |  | NoProgress|  | GateLoop |  ... (6 Lens Cells)
     | LoopLens |  | Lens     |  | Lens     |
     +----------+  +----------+  +----------+
          \             |             /
           \            |            /
            v           v           v
          +------------------------+
          | StuckAggregate         | (Observe Cell)
          +------------------------+
                    |
                    v
          +------------------------+
          | MetaCognition Route    | (Route Cell: action selection)
          +------------------------+
                    |
                    v
              Intervention
```

This Graph executes within the Agent's theta loop. The Engine runs it like any other Graph -- snapshot, resume, telemetry, budget accounting all come for free from the execution infrastructure.

---

## What This Enables

1. **Composition with the rest of the system.** Diagnosis and stuck detection are Cells in Graphs. They compose with other Cells via edges. They participate in FanOut, Pipeline, and Loop patterns. Adding a new error category is adding a pattern to the Score Cell. Adding a new stuck detector is adding a Lens Cell to the FanOut.

2. **Predict-publish-correct for supervision.** The DiagnosisRoute Cell publishes routing predictions. The intervention outcome publishes corrections. Over time, the routing learns which interventions actually work for which error categories. This is the same learning mechanism that drives L1 parameter tuning throughout the system.

3. **Cost-aware routing.** The Route Cell can factor intervention cost into its decision. AutoFix at $0.01 is preferred over RestartAgent at $2.00 when confidence is high. This saves approximately $10-100 per plan depending on error distribution.

4. **Independent threshold evolution.** Each Stuck Lens Cell has its own threshold that adapts independently via Bayesian learning (see [17-adaptive-supervision-loop.md](17-adaptive-supervision-loop.md)). The NoProgress threshold can tighten while the OutputLoop threshold loosens, tracking different rates of change in the underlying failure modes.

5. **Snapshot and resume.** The MetaCognition Loop's state (ring buffers, counters, hashes) is part of the Flow snapshot. If the orchestrator crashes and resumes, the stuck detection state resumes with it. No cold start.

---

## Feedback Loops

- **Score Cell -> L1**: PatternMatchScore confidence values feed calibration. When a high-confidence classification leads to a failed intervention, the confidence for that pattern decreases.
- **Route Cell -> L2**: DiagnosisRoute routing decisions feed the ConductorBandit's observation stream. The bandit learns which (category, intervention) pairs produce good outcomes.
- **Stuck Lens -> L1**: Each Lens threshold is a prediction ("this threshold separates healthy from stuck"). Outcomes (was the detection a true positive?) update the threshold via Bayesian learning.
- **MetaCognition -> L3**: Aggregate stuck detection metrics feed delta-timescale consolidation. Patterns like "agents get stuck on auth tasks 3x more than other tasks" become playbook entries.
- **AutoFix -> L1**: Auto-fix success rates per error category feed back to the Score Cell's confidence values. If ImportError auto-fix drops below 80%, confidence for that routing path decreases.
- **Escalation -> Gate Pipeline**: When MetaCognition escalates (Fail), the gate pipeline's rung selection escalates complexity on the next attempt (see [verify-cells-and-pipeline.md](../02-block/verify-cells-and-pipeline.md)).

---

## Open Questions

1. **LLM-based scoring for ambiguous errors.** The PatternMatchScore Cell covers 95% of errors with substring matching. The remaining 5% fall to the default. Should there be a second Score Cell (LLM-based) that runs in parallel for unmatched errors? The cost is ~$0.01 per classification (Haiku). The benefit is better routing for the long tail. The question is whether the long tail matters enough to justify the latency.

2. **Cross-task stuck detection.** The current Lens Cells operate per-agent within a single task. Cross-task patterns (Agent A failed 3 tasks in a row) are not detected. Should there be an L3-scope Lens Cell that observes plan-level stuck patterns? The watcher ensemble partially covers this (iteration-loop watcher), but it operates on different abstractions.

3. **Stuck detection for flow state.** The Stuck Lens Cells cannot distinguish "slow but making genuine progress" from "stuck." The NoProgress threshold (5 minutes) is coarse. Should the Lens integrate with the FlowDetector (see [17-adaptive-supervision-loop.md](17-adaptive-supervision-loop.md)) to raise thresholds when flow indicators are positive?

4. **Pattern evolution.** The 34 patterns are static. Production errors evolve as the codebase and toolchain change. Should the Score Cell accept new patterns at runtime (via Bus Pulses from L3 learning)? Or is the current approach (add patterns in code, redeploy) sufficient?

5. **Structured classification passthrough.** Today, `GateFailureClassification` from `roko-gate/src/compile_errors.rs` is serialized to JSON in `verdict.error_digest` and must be re-parsed by the Score Cell. Adding a typed `classification` field to the Verdict Signal would eliminate this lossy round-trip. The tradeoff is coupling `roko-core` Signal types to `roko-gate` classification types.
