# Threat Simulation and Nightmares

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). Redesigns threat simulation as Verify Cells that stress-test the knowledge Store, FMEA/FTA as Score Cells that rate threats, the nightmare detection pipeline as a Verify Pipeline, and nightmare containment as a React Cell that quarantines and emits Pulses.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, Kind::Warning, demurrage, HDC fingerprints), [02-CELL](../../unified/02-CELL.md) (Verify protocol, Score protocol, React protocol, Pipeline pattern), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Pipeline specialization), [17-SECURITY-MODEL](../../unified/17-SECURITY-MODEL.md) (capability model, fail-closed), [verify-cells-and-pipeline.md](../02-block/verify-cells-and-pipeline.md) (Verify Cell catalog, Pipeline Graph, Verdict lattice), [09-consolidation-and-staging.md](09-consolidation-and-staging.md) (staging partition, confirmation boost, promotion)

**Source docs**: `docs/10-dreams/09-threat-simulation.md`, `docs/10-dreams/17-advanced-dream-concepts.md`

---

## 1. Threat Simulation IS a Verify Cell

Threat simulation is a Verify Cell that stress-tests the knowledge Store. It does not test code or run subprocesses like CompileGate or TestGate. It tests knowledge: given the agent's current Store contents, are there failure modes the agent is unprepared for? The verdict is not "code compiles" but "knowledge is defensively adequate."

The Verify protocol contract holds: the Cell always returns a Verdict, never panics, and is side-effect free on the Store (it reads the Store but writes only to the staging partition via the standard staging write path).

### The Threat Simulation Verify Cell

```rust
/// Threat simulation as a Verify Cell.
///
/// Stress-tests the knowledge Store by generating hypothetical failures
/// and checking if the agent's current knowledge contains defenses.
/// Returns a Verdict: Pass if defenses are adequate, Fail if gaps found.
///
/// This Cell does not modify the main Store. It reads the Store for
/// existing knowledge and writes threat-generated Warning Signals to
/// the staging partition via the standard staging write path.
pub struct ThreatSimulationCell {
    /// Three-tier threat taxonomy (Revonsuo TST, 2000).
    pub tier_config: ThreatTierConfig,

    /// FMEA/FTA scoring sub-cells (see S2).
    pub fmea_scorer: FmeaScorer,
    pub fta_scorer: FtaScorer,

    /// Adversarial generation model tier.
    pub adversarial_model: ModelTier,       // default: T1 (Sonnet-class)

    /// Maximum threats to generate per simulation cycle.
    pub max_threats_per_cycle: usize,       // default: 30

    /// Gap analysis: minimum coverage before the Cell returns Pass.
    pub min_coverage_fraction: f64,         // default: 0.60
}

impl ThreatSimulationCell {
    /// Verify the Store's defensive adequacy.
    ///
    /// The simulation follows Revonsuo's three-tier taxonomy:
    ///   Tier 1: Known threats (replay past failures)
    ///   Tier 2: Anticipated threats (infer from existing knowledge)
    ///   Tier 3: Novel threats (creative recombination of failure patterns)
    pub async fn verify(
        &self,
        store: &dyn Store,
        episodes: &[Episode],
        ctx: &Context,
    ) -> Verdict {
        let start = Instant::now();
        let mut all_threats = Vec::new();
        let mut gaps = Vec::new();

        // ---- Tier 1: Known Threats ----
        // Replay past failures and check if defenses exist
        let failure_episodes: Vec<_> = episodes.iter()
            .filter(|e| !e.succeeded)
            .collect();

        for failure in &failure_episodes {
            let threat = KnownThreat {
                source_episode: failure.id.clone(),
                failure_class: failure.failure_class.clone(),
                description: failure.error_summary.clone(),
            };

            // Check if Store contains a Warning or Heuristic
            // that addresses this failure class
            let defenses = store.query(
                HdcQuery::similar_to(
                    &failure.hdc_fingerprint,
                    0.50,
                ).with_kinds(&[Kind::Warning, Kind::Heuristic]),
            ).await;

            if defenses.is_empty() {
                gaps.push(ThreatGap {
                    threat_id: threat.id(),
                    tier: ThreatTier::Known,
                    description: format!(
                        "No defense for known failure: {}",
                        failure.failure_class
                    ),
                });
            }

            all_threats.push(Threat::Known(threat));
        }

        // ---- Tier 2: Anticipated Threats ----
        // Infer threats from existing heuristics' boundary conditions
        let heuristics = store.query(
            Query::by_kind(Kind::Heuristic)
                .partition(Partition::Promoted)
                .limit(50),
        ).await;

        for heuristic in &heuristics {
            let anticipated = self.generate_anticipated_threat(
                heuristic, ctx
            ).await;

            if let Some(threat) = anticipated {
                let defense_exists = store.query(
                    HdcQuery::similar_to(&threat.hdc_fingerprint, 0.50)
                        .with_kinds(&[Kind::Warning]),
                ).await;

                if defense_exists.is_empty() {
                    gaps.push(ThreatGap {
                        threat_id: threat.id(),
                        tier: ThreatTier::Anticipated,
                        description: format!(
                            "No defense for anticipated threat from heuristic: {}",
                            heuristic.ref_().short()
                        ),
                    });
                }

                all_threats.push(Threat::Anticipated(threat));
            }
        }

        // ---- Tier 3: Novel Threats ----
        // Creative recombination of failure patterns
        if failure_episodes.len() >= 2 {
            let novel_threats = self.generate_novel_threats(
                &failure_episodes,
                &self.tier_config.novel,
                ctx,
            ).await;

            for threat in novel_threats {
                all_threats.push(Threat::Novel(threat));
                // Novel threats are always gaps by definition
                gaps.push(ThreatGap {
                    threat_id: threat.id(),
                    tier: ThreatTier::Novel,
                    description: format!(
                        "Novel compound threat: {}",
                        threat.description
                    ),
                });
            }
        }

        // ---- Emit Warning Signals for each threat ----
        for threat in &all_threats {
            let response = self.rehearse_response(threat, ctx).await;
            let warning = self.threat_to_staging_signal(threat, &response);
            store.staging_write(warning).await;
        }

        // ---- Coverage analysis ----
        let total_threat_space = all_threats.len();
        let defended = total_threat_space - gaps.len();
        let coverage = if total_threat_space > 0 {
            defended as f64 / total_threat_space as f64
        } else {
            1.0
        };

        let elapsed = start.elapsed().as_millis() as u64;

        if coverage >= self.min_coverage_fraction {
            Verdict::pass("threat-simulation")
                .with_detail(format!(
                    "Coverage {:.0}%: {}/{} threats defended. {} warnings staged.",
                    coverage * 100.0, defended, total_threat_space, all_threats.len(),
                ))
                .with_duration(elapsed)
        } else {
            Verdict::fail(
                "threat-simulation",
                format!(
                    "Coverage {:.0}% below minimum {:.0}%: {} undefended gaps",
                    coverage * 100.0,
                    self.min_coverage_fraction * 100.0,
                    gaps.len(),
                ),
            )
            .with_detail(serde_json::to_string_pretty(&gaps).unwrap_or_default())
            .with_duration(elapsed)
        }
    }

    /// Convert a threat + rehearsed response into a staging Signal.
    ///
    /// Warning Signals enter staging at confidence 0.25 (source spec)
    /// with the aggressive staging demurrage schedule.
    fn threat_to_staging_signal(
        &self,
        threat: &Threat,
        response: &ThreatResponse,
    ) -> Signal {
        let mut signal = Signal::new(
            Kind::Warning,
            format!(
                "Threat [{}]: {}\n\nEarly warning: {}\nResponse: {}\nRecovery: {}\nPrevention: {}",
                threat.tier(),
                threat.description(),
                response.early_warning_signs.join(", "),
                response.immediate_response,
                response.recovery_strategy,
                response.prevention_heuristic,
            ),
        );
        signal.balance = 0.25; // Below staging initial (0.30) -- threats start lower
        signal.confidence = 0.25;
        signal.partition = Partition::Staging;
        signal.hdc_fingerprint = Some(threat.hdc_fingerprint());
        signal.provenance = Provenance::Dream {
            source_phase: SourcePhase::ThreatSimulation,
            source_episodes: threat.source_episodes(),
        };
        signal
    }
}

/// Three-tier threat taxonomy from Revonsuo's Threat Simulation Theory.
pub enum ThreatTier {
    Known,       // Tier 1: experienced before
    Anticipated, // Tier 2: inferred from existing knowledge
    Novel,       // Tier 3: creative recombination
}

pub struct ThreatGap {
    pub threat_id: String,
    pub tier: ThreatTier,
    pub description: String,
}

pub struct ThreatResponse {
    pub early_warning_signs: Vec<String>,
    pub immediate_response: String,
    pub recovery_strategy: String,
    pub prevention_heuristic: String,
}
```

### Regime-Aware Scheduling

The ThreatSimulationCell's allocation within the dream cycle varies by operational context. This is controlled by a Trigger Cell that adjusts the simulation budget:

| Context | Threat simulation allocation | Rationale |
|---|---|---|
| Normal operations, low error rate | 10% of REM phase | Maintenance-level vigilance |
| Recent failures detected | 30% of REM phase | Active threat learning |
| Novel environment (new task types) | 25% of REM phase | Anticipatory threat modeling |
| Post-crisis recovery | 40% of REM phase | Intensive threat rehearsal |

---

## 2. FMEA and FTA ARE Score Cells

FMEA (bottom-up failure enumeration) and FTA (top-down failure decomposition) are **Score Cells** -- they rate threats along severity, occurrence, and detection dimensions. They are not Verify Cells because they do not produce pass/fail verdicts. They produce scores that downstream consumers (the threat simulation Verify Cell, the prioritization Route Cell) use to make decisions.

### FMEA as a Score Cell

FMEA proceeds component-by-component, asking "what could go wrong here?" for each element. Each failure mode is scored on three 1-10 scales.

```rust
/// FMEA as a Score Cell.
///
/// Rates each failure mode along three dimensions:
///   Severity (S):   1-10, how bad is the outcome
///   Occurrence (O): 1-10, how likely is it
///   Detection (D):  1-10, how hard to detect before harm (10 = hardest)
///
/// Risk Priority Number: RPN = S * O * D
///
/// RPN thresholds:
///   >= 200  -> immediate mitigation required
///   100-199 -> schedule for next dream cycle
///   < 100   -> monitor, no immediate action
pub struct FmeaScorer {
    /// RPN threshold for immediate action.
    pub rpn_immediate: u16,                 // default: 200

    /// RPN threshold for scheduled action.
    pub rpn_scheduled: u16,                 // default: 100

    /// Maximum failure modes to enumerate per component.
    pub max_modes_per_component: usize,     // default: 10

    /// Impact override: severity >= this triggers Critical regardless of O*D.
    pub severity_override: u8,              // default: 9
}

impl FmeaScorer {
    /// Score a set of failure modes.
    ///
    /// Returns scored modes sorted by RPN descending.
    pub fn score(&self, modes: &[FailureMode]) -> Vec<ScoredFailureMode> {
        let mut scored: Vec<ScoredFailureMode> = modes.iter()
            .map(|mode| {
                let rpn = mode.severity as u16
                    * mode.occurrence as u16
                    * mode.detection as u16;

                let priority = if mode.severity >= self.severity_override {
                    // Severity override: automatic Critical
                    FmeaPriority::Immediate
                } else if rpn >= self.rpn_immediate {
                    FmeaPriority::Immediate
                } else if rpn >= self.rpn_scheduled {
                    FmeaPriority::Scheduled
                } else {
                    FmeaPriority::Monitor
                };

                ScoredFailureMode {
                    mode: mode.clone(),
                    rpn,
                    priority,
                    risk_score: rpn as f64 / 1000.0, // normalized to [0, 1]
                }
            })
            .collect();

        scored.sort_by(|a, b| b.rpn.cmp(&a.rpn));
        scored
    }
}

pub struct FailureMode {
    pub id: String,
    pub component: String,
    pub description: String,
    pub severity: u8,       // 1-10
    pub occurrence: u8,     // 1-10
    pub detection: u8,      // 1-10
    pub tier: ThreatTier,
    pub early_warning_signs: Vec<String>,
}

pub struct ScoredFailureMode {
    pub mode: FailureMode,
    pub rpn: u16,
    pub priority: FmeaPriority,
    pub risk_score: f64,
}

pub enum FmeaPriority {
    Immediate,  // RPN >= 200 OR severity >= 9
    Scheduled,  // RPN 100-199
    Monitor,    // RPN < 100
}
```

### FTA as a Score Cell

FTA starts from an undesired top-level event and decomposes it through AND/OR logic gates until reaching basic, independently quantifiable events. The Score Cell computes propagated probabilities.

```rust
/// FTA as a Score Cell.
///
/// Builds a fault tree from a top-level event, computes propagated
/// probabilities through AND/OR gates, and identifies Minimal Cut Sets.
///
/// Gate logic:
///   OR gate:  P(top) = 1 - product(1 - P(input_i))
///   AND gate: P(top) = product(P(input_i))
pub struct FtaScorer {
    /// Maximum tree depth.
    pub max_depth: usize,                   // default: 5

    /// Whether to compute Minimal Cut Sets.
    pub compute_mcs: bool,                  // default: true
}

impl FtaScorer {
    /// Score a fault tree by propagating probabilities from leaves to root.
    pub fn score(&self, tree: &FaultTree) -> FtaScore {
        // Propagate probabilities bottom-up
        let top_probability = self.propagate(&tree.root, &tree.basic_events);

        // Compute Minimal Cut Sets if requested
        let minimal_cut_sets = if self.compute_mcs {
            self.find_minimal_cut_sets(tree)
        } else {
            Vec::new()
        };

        FtaScore {
            top_event: tree.top_event.clone(),
            top_probability,
            minimal_cut_sets,
            tree_depth: self.measure_depth(&tree.root),
        }
    }

    /// Recursive probability propagation through the fault tree.
    fn propagate(
        &self,
        node: &FaultNode,
        basic_events: &HashMap<String, f64>,
    ) -> f64 {
        match node {
            FaultNode::BasicEvent { id } => {
                *basic_events.get(id).unwrap_or(&0.01)
            }
            FaultNode::Gate { gate_type, inputs } => {
                let input_probs: Vec<f64> = inputs.iter()
                    .map(|input| self.propagate(input, basic_events))
                    .collect();

                match gate_type {
                    FaultGateType::Or => {
                        // P(any) = 1 - product(1 - P(i))
                        1.0 - input_probs.iter()
                            .map(|p| 1.0 - p)
                            .product::<f64>()
                    }
                    FaultGateType::And => {
                        // P(all) = product(P(i))
                        input_probs.iter().product()
                    }
                }
            }
        }
    }

    /// Find Minimal Cut Sets: smallest combinations of basic event
    /// failures sufficient to cause the top event.
    fn find_minimal_cut_sets(&self, tree: &FaultTree) -> Vec<MinimalCutSet> {
        let raw_sets = self.enumerate_cut_sets(&tree.root);

        // Minimize: remove supersets
        let mut minimal: Vec<MinimalCutSet> = Vec::new();
        for set in &raw_sets {
            let dominated = minimal.iter().any(|m| m.events.is_subset(&set.events));
            if !dominated {
                minimal.retain(|m| !set.events.is_subset(&m.events));
                minimal.push(set.clone());
            }
        }

        // Sort by combined probability (highest risk first)
        minimal.sort_by(|a, b| b.combined_probability
            .partial_cmp(&a.combined_probability).unwrap());
        minimal
    }
}

pub struct FaultTree {
    pub top_event: String,
    pub root: FaultNode,
    pub basic_events: HashMap<String, f64>,
}

pub enum FaultNode {
    BasicEvent { id: String },
    Gate {
        gate_type: FaultGateType,
        inputs: Vec<FaultNode>,
    },
}

pub enum FaultGateType {
    Or,
    And,
}

pub struct FtaScore {
    pub top_event: String,
    pub top_probability: f64,
    pub minimal_cut_sets: Vec<MinimalCutSet>,
    pub tree_depth: usize,
}

pub struct MinimalCutSet {
    pub events: HashSet<String>,
    pub combined_probability: f64,
}
```

### FMEA + FTA Integration

The two Score Cells feed the ThreatSimulationCell as complementary inputs:

```
FMEA (bottom-up)                   FTA (top-down)
    |                                   |
    v                                   v
ScoredFailureMode[]              FtaScore[]
    |                                   |
    +------------ merge ----------------+
                    |
                    v
        ThreatSimulationCell (Verify)
                    |
                    v
            Verdict + Warning Signals -> staging
```

FMEA discovers failure modes that FTA might miss (component-level surprises). FTA reveals systemic vulnerabilities that FMEA might miss (multi-component cascading failures where each component looks safe in isolation but the combination is dangerous).

---

## 3. Nightmare Detection IS a Verify Pipeline

The nightmare detector is a **Pipeline Graph** of four Verify Cells, identical in structure to the 7-rung verification pipeline from [verify-cells-and-pipeline.md](../02-block/verify-cells-and-pipeline.md). It is a linear chain where each Cell can reject (short-circuit) or pass through. The pipeline operates on dream-generated hypotheses before they enter the staging partition.

### The Pipeline

```
hypothesis
    |
    v
[Stage 1: HarmClassifierCell]        Verify Cell (lightweight LLM classifier)
    | pass                   | fail -> QUARANTINE
    v
[Stage 2: CbrnCheckCell]             Verify Cell (domain vocabulary + embedding match)
    | pass                   | fail -> QUARANTINE
    v
[Stage 3: CapabilityDeltaCell]       Verify Cell (capability gap analysis)
    | pass                   | fail -> QUARANTINE
    v
[Stage 4: EscalationCell]            Verify Cell (entropy-based human escalation)
    | pass                   | fail -> ESCALATE (human review queue)
    v
BENIGN -> staging partition write
```

Short-circuit behavior: a failure at any stage skips all subsequent stages and routes the hypothesis to the NightmareContainmentCell (see S4). This is the same short-circuit semantics as the GatePipeline.

### Pipeline Graph Definition

```toml
[graph]
id = "nightmare-detection-pipeline"
kind = "Pipeline"
description = "Four-stage safety filter for dream-generated hypotheses"
short_circuit = true

[[graph.cells]]
id = "harm-classifier"
protocol = "Verify"
description = "Stage 1: binary harmful/benign classification"

[[graph.cells]]
id = "cbrn-check"
protocol = "Verify"
description = "Stage 2: CBRN and cybersecurity domain check"

[[graph.cells]]
id = "capability-delta"
protocol = "Verify"
description = "Stage 3: capability gap analysis"

[[graph.cells]]
id = "escalation-check"
protocol = "Verify"
description = "Stage 4: entropy-based human escalation"

[[graph.edges]]
from = "harm-classifier.pass"
to = "cbrn-check.in"

[[graph.edges]]
from = "cbrn-check.pass"
to = "capability-delta.in"

[[graph.edges]]
from = "capability-delta.pass"
to = "escalation-check.in"

# All fail edges route to the containment React Cell
[[graph.edges]]
from = "harm-classifier.fail"
to = "nightmare-containment.in"

[[graph.edges]]
from = "cbrn-check.fail"
to = "nightmare-containment.in"

[[graph.edges]]
from = "capability-delta.fail"
to = "nightmare-containment.in"

[[graph.edges]]
from = "escalation-check.fail"
to = "nightmare-containment.in"
```

### Stage 1: Harm Classifier Cell

```rust
/// Stage 1: Harm classifier.
///
/// Uses a lightweight LLM (T0/Haiku-class for speed) to classify
/// dream-generated hypotheses as harmful or benign. Binary verdict.
///
/// Follows the Constitutional Classifiers approach (Anthropic, 2025):
/// the classifier evaluates against a constitution of natural language
/// safety rules defined in .roko/safety/dream-constitution.toml.
pub struct HarmClassifierCell {
    /// Model tier for the classifier (fast, cheap).
    pub model_tier: ModelTier,              // default: T0

    /// Path to the constitutional rules file.
    pub constitution_path: PathBuf,

    /// Self-critique rounds before external classification.
    pub self_critique_rounds: usize,        // default: 2

    /// Self-critique temperature (low = more conservative).
    pub self_critique_temperature: f64,     // default: 0.3
}

impl HarmClassifierCell {
    pub async fn verify(
        &self,
        hypothesis: &Signal,
        ctx: &Context,
    ) -> Verdict {
        let start = Instant::now();

        // Phase 1: Self-critique (Constitutional AI)
        let self_critique_safe = self.run_self_critique(hypothesis, ctx).await;
        if !self_critique_safe {
            return Verdict::fail(
                "harm-classifier",
                "Self-critique identified harmful content",
            ).with_detail(NightmareClass::HarmfulStrategyGeneration.to_string())
             .with_duration(start.elapsed().as_millis() as u64);
        }

        // Phase 2: External classifier
        let classification = self.classify(hypothesis, ctx).await;
        let elapsed = start.elapsed().as_millis() as u64;

        match classification {
            HarmClassification::Benign { confidence } => {
                Verdict::pass("harm-classifier")
                    .with_detail(format!("benign (confidence: {:.2})", confidence))
                    .with_duration(elapsed)
            }
            HarmClassification::Harmful { class, confidence } => {
                Verdict::fail(
                    "harm-classifier",
                    format!("Classified as {:?} (confidence: {:.2})", class, confidence),
                )
                .with_detail(class.to_string())
                .with_duration(elapsed)
            }
        }
    }
}

/// Nightmare classes. Four-class taxonomy from source spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NightmareClass {
    HarmfulStrategyGeneration,
    DangerousToolChainDiscovery,
    SafetyConstraintBypass,
    PolicyViolation,
}

enum HarmClassification {
    Benign { confidence: f64 },
    Harmful { class: NightmareClass, confidence: f64 },
}
```

### Stage 2: CBRN Check Cell

```rust
/// Stage 2: Domain-specific CBRN and cybersecurity check.
///
/// Structured vocabulary matching + embedding similarity against
/// known dangerous domains: chemical, biological, radiological,
/// nuclear, cybersecurity exploitation.
///
/// This is a deterministic check -- no LLM call. It uses constrained
/// vocabulary lists and HDC embedding similarity against a curated
/// library of dangerous pattern fingerprints.
pub struct CbrnCheckCell {
    /// HDC fingerprints of known dangerous patterns.
    pub dangerous_patterns: Vec<HdcVector>,

    /// Similarity threshold: above this, the hypothesis is flagged.
    pub similarity_threshold: f64,          // default: 0.65

    /// Vocabulary blocklist: keywords that trigger immediate flagging.
    pub blocklist: HashSet<String>,
}

impl CbrnCheckCell {
    pub async fn verify(
        &self,
        hypothesis: &Signal,
        _ctx: &Context,
    ) -> Verdict {
        let start = Instant::now();

        // Check 1: Vocabulary blocklist (fast, deterministic)
        let content_lower = hypothesis.content().to_lowercase();
        for blocked_term in &self.blocklist {
            if content_lower.contains(blocked_term) {
                return Verdict::fail(
                    "cbrn-check",
                    format!("Blocked vocabulary match: [REDACTED]"),
                )
                .with_detail(NightmareClass::PolicyViolation.to_string())
                .with_duration(start.elapsed().as_millis() as u64);
            }
        }

        // Check 2: HDC similarity to dangerous pattern library
        if let Some(fp) = &hypothesis.hdc_fingerprint {
            for pattern in &self.dangerous_patterns {
                let similarity = fp.similarity(pattern);
                if similarity > self.similarity_threshold {
                    return Verdict::fail(
                        "cbrn-check",
                        "HDC similarity to dangerous pattern above threshold",
                    )
                    .with_detail(NightmareClass::PolicyViolation.to_string())
                    .with_duration(start.elapsed().as_millis() as u64);
                }
            }
        }

        Verdict::pass("cbrn-check")
            .with_duration(start.elapsed().as_millis() as u64)
    }
}
```

### Stage 3: Capability Delta Cell

```rust
/// Stage 3: Capability delta check.
///
/// Measures whether the hypothesis implies capabilities significantly
/// beyond the agent's current tool set. A hypothesis that describes
/// actions the agent cannot currently take is suspicious -- it may
/// be synthesizing a novel capability escalation path.
///
/// capability_delta = implied_capability_set - known_capability_set
/// If |delta| / |known| > threshold, the hypothesis is flagged.
pub struct CapabilityDeltaCell {
    /// Threshold for capability delta. Above this, the hypothesis is flagged.
    pub delta_threshold: f64,               // default: 0.50

    /// The agent's current known capability set (tool names, permissions).
    pub known_capabilities: HashSet<String>,
}

impl CapabilityDeltaCell {
    pub async fn verify(
        &self,
        hypothesis: &Signal,
        ctx: &Context,
    ) -> Verdict {
        let start = Instant::now();

        // Extract implied capabilities from hypothesis content
        let implied = self.extract_implied_capabilities(hypothesis, ctx).await;

        // Compute delta: capabilities implied but not known
        let delta: HashSet<_> = implied.difference(&self.known_capabilities).collect();
        let delta_ratio = if self.known_capabilities.is_empty() {
            1.0
        } else {
            delta.len() as f64 / self.known_capabilities.len() as f64
        };

        let elapsed = start.elapsed().as_millis() as u64;

        if delta_ratio > self.delta_threshold {
            Verdict::fail(
                "capability-delta",
                format!(
                    "Capability delta {:.2} exceeds threshold {:.2}: {} new capabilities implied",
                    delta_ratio, self.delta_threshold, delta.len(),
                ),
            )
            .with_detail(NightmareClass::SafetyConstraintBypass.to_string())
            .with_duration(elapsed)
        } else {
            Verdict::pass("capability-delta")
                .with_detail(format!("delta_ratio: {:.2}", delta_ratio))
                .with_duration(elapsed)
        }
    }
}
```

### Stage 4: Escalation Cell

```rust
/// Stage 4: Entropy-based human escalation.
///
/// Any hypothesis that passed Stages 1-3 but scores above the
/// uncertainty threshold on any stage's internal confidence is
/// escalated to human review. This catches edge cases where the
/// classifiers are uncertain -- the fail-safe is human judgment.
///
/// The entropy measure: H = -sum(p_i * log2(p_i)) across the
/// classifier's output distribution. High entropy means the
/// classifier is uncertain.
pub struct EscalationCell {
    /// Entropy threshold. Above this, escalate to human review.
    pub entropy_threshold: f64,             // default: 0.40

    /// Aggregated confidence scores from prior stages.
    /// Populated by the pipeline as each stage runs.
    pub stage_confidences: Vec<f64>,
}

impl EscalationCell {
    pub async fn verify(
        &self,
        hypothesis: &Signal,
        _ctx: &Context,
    ) -> Verdict {
        let start = Instant::now();

        // Compute entropy across stage confidence scores
        let entropy = self.compute_entropy();
        let elapsed = start.elapsed().as_millis() as u64;

        if entropy > self.entropy_threshold {
            // High uncertainty: escalate to human review
            Verdict::fail(
                "escalation-check",
                format!(
                    "Aggregate entropy {:.3} exceeds threshold {:.3}: escalating to human review",
                    entropy, self.entropy_threshold,
                ),
            )
            .with_detail("human_escalation_required")
            .with_duration(elapsed)
        } else {
            Verdict::pass("escalation-check")
                .with_detail(format!("entropy: {:.3}", entropy))
                .with_duration(elapsed)
        }
    }

    fn compute_entropy(&self) -> f64 {
        if self.stage_confidences.is_empty() {
            return 0.0;
        }
        // Normalize confidences to a probability distribution
        let sum: f64 = self.stage_confidences.iter().sum();
        if sum == 0.0 {
            return 0.0;
        }
        let probs: Vec<f64> = self.stage_confidences.iter()
            .map(|c| c / sum)
            .collect();
        -probs.iter()
            .filter(|p| **p > 0.0)
            .map(|p| p * p.log2())
            .sum::<f64>()
    }
}
```

### Verdict Aggregation

The nightmare pipeline uses the same Verdict lattice as the standard verification pipeline:

```
Skip < Warn < Pass < Fail

Pipeline verdict = max(stage verdicts)  [with short-circuit]
```

A Fail at any stage dominates the entire pipeline verdict. The Verdict carries `detail` with the `NightmareClass` that triggered the failure, which the containment React Cell (S4) uses for triage.

---

## 4. Nightmare Containment IS a React Cell

When the nightmare pipeline produces a Fail verdict, the **NightmareContainmentCell** fires. It is a React Cell: it watches the Bus for nightmare verdict Pulses and executes the containment protocol.

```rust
/// Nightmare containment as a React Cell.
///
/// Watches Bus topic "nightmare.verdict" for Fail verdicts from
/// the nightmare detection pipeline. On Fail:
///   1. Quarantine the hypothesis (never written to staging)
///   2. Log to .roko/dreams/nightmares.jsonl
///   3. Terminate the current REM phase early
///   4. Queue human review request
///   5. Reduce adversarial dreaming intensity for cooldown_cycles
///   6. Emit containment Pulse on Bus
///
/// The containment is non-bypassable by the agent. The Cell runs
/// outside the agent's control loop, in the safety sidecar.
pub struct NightmareContainmentCell {
    /// Path for the nightmare log (append-only JSONL).
    pub log_path: PathBuf,                  // default: .roko/dreams/nightmares.jsonl

    /// Number of dream cycles to reduce adversarial intensity after containment.
    pub cooldown_cycles: usize,             // default: 3

    /// Current cooldown counter (decremented each cycle).
    cooldown_remaining: usize,

    /// Quarantine buffer: hypothesis refs that were blocked.
    quarantined: Vec<SignalRef>,

    /// Pending human review queue.
    pending_reviews: Vec<NightmareReport>,
}

impl NightmareContainmentCell {
    /// React to a nightmare verdict Pulse.
    pub async fn react(
        &mut self,
        pulse: &Pulse,
        bus: &dyn Bus,
    ) -> ContainmentAction {
        let verdict: &Verdict = pulse.payload();
        let hypothesis_ref = pulse.source_ref();
        let nightmare_class = NightmareClass::from_detail(verdict.detail());
        let detection_stage = self.infer_stage(verdict.gate_name());

        // 1. Quarantine: block the hypothesis from staging
        self.quarantined.push(hypothesis_ref.clone());

        // 2. Build nightmare report
        let report = NightmareReport {
            id: generate_id(),
            cycle_id: pulse.context_id(),
            agent_id: pulse.agent_id(),
            detected_at: Utc::now(),
            hypothesis_summary: verdict.reason().to_string(),
            detection_stage,
            nightmare_class: nightmare_class.clone(),
            classifier_confidence: verdict.score().unwrap_or(0.0),
            human_reviewed: false,
            human_decision: None,
        };

        // 3. Append to JSONL log
        if let Ok(line) = serde_json::to_string(&report) {
            let _ = tokio::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&self.log_path)
                .await
                .and_then(|mut f| {
                    use tokio::io::AsyncWriteExt;
                    Box::pin(async move {
                        f.write_all(line.as_bytes()).await?;
                        f.write_all(b"\n").await
                    })
                }).await;
        }

        // 4. Queue for human review
        self.pending_reviews.push(report.clone());

        // 5. Set cooldown
        self.cooldown_remaining = self.cooldown_cycles;

        // 6. Emit containment Pulse on Bus
        let containment_pulse = Pulse::new(
            Topic::parse("nightmare.contained"),
            ContainmentEvent {
                hypothesis_ref: hypothesis_ref.clone(),
                nightmare_class,
                detection_stage,
                cooldown_cycles: self.cooldown_cycles,
                timestamp: Utc::now(),
            },
        );
        bus.publish(containment_pulse).await;

        ContainmentAction::Quarantined {
            hypothesis_ref,
            report,
        }
    }

    /// Called at the start of each dream cycle.
    /// Decrements cooldown and returns the current adversarial intensity modifier.
    pub fn cycle_tick(&mut self) -> f64 {
        if self.cooldown_remaining > 0 {
            self.cooldown_remaining -= 1;
            // During cooldown, adversarial dreaming runs at reduced intensity.
            // The modifier is a multiplier on the threat simulation allocation.
            0.25 // 25% of normal adversarial intensity during cooldown
        } else {
            1.0 // Full intensity
        }
    }

    /// Check if a hypothesis ref is quarantined (blocked from staging).
    pub fn is_quarantined(&self, signal_ref: &SignalRef) -> bool {
        self.quarantined.contains(signal_ref)
    }

    /// Process a human review decision.
    pub fn process_human_decision(
        &mut self,
        report_id: &str,
        decision: NightmareDecision,
    ) {
        if let Some(report) = self.pending_reviews.iter_mut()
            .find(|r| r.id == report_id)
        {
            report.human_reviewed = true;
            report.human_decision = Some(decision.clone());
        }

        // If approved-as-is, remove from quarantine
        if matches!(decision, NightmareDecision::ApprovedAsIs) {
            self.quarantined.retain(|ref_| {
                self.pending_reviews.iter()
                    .find(|r| r.id == report_id)
                    .map(|r| r.hypothesis_ref != *ref_)
                    .unwrap_or(true)
            });
        }
    }
}

pub struct NightmareReport {
    pub id: String,
    pub cycle_id: String,
    pub agent_id: String,
    pub detected_at: DateTime<Utc>,
    pub hypothesis_summary: String,
    pub detection_stage: u8,
    pub nightmare_class: NightmareClass,
    pub classifier_confidence: f64,
    pub human_reviewed: bool,
    pub human_decision: Option<NightmareDecision>,
}

pub enum NightmareDecision {
    Rejected,
    ApprovedWithModification { modified_content: String },
    ApprovedAsIs,
}

pub struct ContainmentEvent {
    pub hypothesis_ref: SignalRef,
    pub nightmare_class: NightmareClass,
    pub detection_stage: u8,
    pub cooldown_cycles: usize,
    pub timestamp: DateTime<Utc>,
}

pub enum ContainmentAction {
    Quarantined {
        hypothesis_ref: SignalRef,
        report: NightmareReport,
    },
}
```

### The Full Nightmare Pipeline as a Graph

```
                    Dream-generated hypothesis
                              |
                              v
            +--------------------------------------+
            | NightmareDetectionPipeline (Verify)  |
            |                                      |
            |  Stage 1: HarmClassifierCell         |
            |       |                              |
            |  Stage 2: CbrnCheckCell              |
            |       |                              |
            |  Stage 3: CapabilityDeltaCell        |
            |       |                              |
            |  Stage 4: EscalationCell             |
            +--------------------------------------+
                    |                    |
                   pass                 fail
                    |                    |
                    v                    v
          Staging partition    NightmareContainmentCell (React)
          (standard write)           |
                              +------+------+------+
                              |      |      |      |
                         quarantine  log   cooldown  Pulse on Bus
                                                     "nightmare.contained"
```

### Containment Guarantees

The containment protocol provides five guarantees, all enforced by the architecture:

| Guarantee | Mechanism |
|---|---|
| No nightmare content enters staging | `NightmareContainmentCell.is_quarantined()` check on the staging write path |
| Every nightmare is logged | Append-only JSONL at `self.log_path` -- the Cell writes synchronously before returning |
| REM phase terminates early | The containment Pulse on topic `"nightmare.contained"` is watched by the dream cycle scheduler, which aborts the current REM phase |
| Human review is always queued | `pending_reviews` is checked by the dashboard TUI and HTTP control plane (`/api/nightmares/pending`) |
| Adversarial intensity is reduced post-nightmare | `cycle_tick()` returns a reduced multiplier for `cooldown_cycles` after containment |

---

## 5. The Integrated Threat-Dream Pipeline

Threat simulation and nightmare detection are complementary: threat simulation generates adversarial content deliberately to strengthen defenses; nightmare detection catches adversarial content that should not have been generated. They form a Pipeline with a safety wrapper:

```
Dream cycle begins
    |
    v
+------ REM Imagination Phase -----------------------------------+
|                                                                  |
|   Normal REM hypotheses (counterfactual, combinational, etc.)    |
|       |                                                          |
|   Threat simulation (if scheduled, 10-40% of REM budget)        |
|       |                                                          |
|   All hypotheses collected                                       |
+------------------------------------------------------------------+
    |
    v
+------ Nightmare Detection Pipeline (mandatory, non-skippable) --+
|                                                                  |
|   [harm classifier] -> [CBRN] -> [capability delta] -> [escal.] |
|                                                                  |
+------------------------------------------------------------------+
    |                           |
   pass                        fail
    |                           |
    v                           v
Staging partition          Containment React Cell
(normal demurrage)              |
    |                      quarantine + log + cooldown
    v
Consolidation continues...
```

The critical invariant: **every hypothesis passes through the nightmare pipeline before entering staging, including threat-generated Warning Signals.** Threat simulation generates realistic adversarial scenarios, but those scenarios are themselves subject to nightmare detection. A threat scenario that crosses from "realistic failure rehearsal" into "actual harmful capability synthesis" is caught and quarantined.

---

## 6. Bayesian Threat Prioritization

After FMEA and FTA scoring, the prioritization layer uses Bayesian updating to adjust threat probabilities as evidence accumulates:

```rust
/// Bayesian threat prioritization.
///
/// Updates threat probability estimates as evidence accumulates
/// across dream cycles and waking episodes.
pub struct BayesianThreatPrioritizer {
    /// Default prior for novel threats with no evidence.
    pub default_prior: f64,                 // default: 0.10

    /// Threat probability estimates, keyed by threat ID.
    posteriors: HashMap<String, f64>,
}

impl BayesianThreatPrioritizer {
    /// Update the probability estimate for a threat given new evidence.
    ///
    /// P(threat | evidence) = P(evidence | threat) * P(threat) / P(evidence)
    pub fn update(
        &mut self,
        threat_id: &str,
        likelihood_if_threat: f64,    // P(evidence | threat)
        likelihood_marginal: f64,      // P(evidence)
    ) {
        let prior = self.posteriors
            .get(threat_id)
            .copied()
            .unwrap_or(self.default_prior);

        let posterior = (likelihood_if_threat * prior) / likelihood_marginal;
        self.posteriors.insert(threat_id.to_string(), posterior.clamp(0.0, 1.0));
    }

    /// Expected Value of Mitigation for a control defending against a threat.
    ///
    /// EVM = Impact * [P(attack) - P(attack | control)] - cost
    /// A mitigation is worth deploying when EVM > 0.
    pub fn expected_value_of_mitigation(
        &self,
        threat_id: &str,
        impact: f64,
        residual_probability: f64,
        cost: f64,
    ) -> f64 {
        let p_attack = self.posteriors
            .get(threat_id)
            .copied()
            .unwrap_or(self.default_prior);

        impact * (p_attack - residual_probability) - cost
    }

    /// Rank all threats by risk-adjusted expected loss (P * Impact).
    pub fn rank(&self, threats: &[ScoredFailureMode]) -> Vec<RankedThreat> {
        let mut ranked: Vec<RankedThreat> = threats.iter()
            .map(|t| {
                let probability = self.posteriors
                    .get(&t.mode.id)
                    .copied()
                    .unwrap_or(self.default_prior);
                let expected_loss = probability * t.mode.severity as f64;

                RankedThreat {
                    threat_id: t.mode.id.clone(),
                    probability,
                    impact: t.mode.severity as f64,
                    expected_loss,
                    rpn: t.rpn,
                    priority: t.priority.clone(),
                }
            })
            .collect();

        ranked.sort_by(|a, b| b.expected_loss
            .partial_cmp(&a.expected_loss).unwrap());
        ranked
    }
}

pub struct RankedThreat {
    pub threat_id: String,
    pub probability: f64,
    pub impact: f64,
    pub expected_loss: f64,
    pub rpn: u16,
    pub priority: FmeaPriority,
}
```

---

## 7. Attack Knowledge Persistence

Threat simulation maintains a persistent attack knowledge base that grows across dream cycles, following the AutoRedTeamer (2025) lifelong learning pattern. Attack primitives are modular and composable (MAD-MAX, 2025):

```rust
/// Persistent attack knowledge Store.
///
/// Attack primitives are Signals of Kind::AttackPrimitive stored in
/// a dedicated Store partition. They persist across dream cycles and
/// are subject to the same demurrage economics as other knowledge.
///
/// Composable attack primitives can be combined into compound threats
/// (MAD-MAX architecture). Historical success rates update via the
/// standard confirmation boost React Cell.
pub struct AttackKnowledgeStore {
    /// Store partition for attack knowledge.
    pub partition: Partition,               // Partition::AttackKnowledge

    /// Maximum composable primitives per compound threat.
    pub max_primitives_per_compound: usize, // default: 4

    /// Category coverage target: across N generated threats,
    /// at least this many distinct categories should be represented.
    pub min_category_diversity: usize,      // default: 3
}

/// A modular attack primitive -- a Signal with attack-specific metadata.
pub struct AttackPrimitive {
    /// The underlying Signal (Kind::AttackPrimitive).
    pub signal: Signal,

    /// Attack category for diversity tracking.
    pub category: AttackCategory,

    /// Historical success rate (updated by confirmation boost).
    pub success_rate: f64,

    /// IDs of primitives this composes well with.
    pub composable_with: Vec<SignalRef>,

    /// How this primitive was discovered.
    pub source: AttackSource,
}

pub enum AttackCategory {
    InputManipulation,
    TimingExploitation,
    ResourceExhaustion,
    ContextPoisoning,
    PrivilegeEscalation,
    LogicBypass,
    StateCorruption,
}

pub enum AttackSource {
    WakingFailure { episode_ref: SignalRef },
    AdversarialGeneration { cycle_id: String },
    MeshInherited { source_agent: String },
    Synthesized { parents: Vec<SignalRef> },
}
```

Attack primitives are subject to the same demurrage schedule as Warnings (high decay rate, short half-life). This ensures the attack knowledge base stays relevant: old attack patterns that are no longer effective decay naturally, while patterns that continue to discover real vulnerabilities get reinforced and survive.

---

## What This Enables

1. **Proactive defense**: Threat simulation stress-tests the knowledge Store during dream cycles, discovering gaps before they become real failures during waking operation. The three-tier taxonomy (known, anticipated, novel) ensures coverage across the entire threat spectrum.

2. **Graduated safety**: The four-stage nightmare pipeline provides graduated filtering. Obvious harmful content is caught cheaply at Stage 1 (lightweight classifier). Subtle domain-specific threats are caught at Stage 2 (deterministic vocabulary check). Capability escalation is caught at Stage 3 (gap analysis). Uncertain cases are escalated to humans at Stage 4. Each stage is a standard Verify Cell with the same total-function, no-panic contract.

3. **Economic threat lifecycle**: Threat-generated Warning Signals enter the same staging partition as all other dream hypotheses. They are subject to the same demurrage, the same confirmation boost, and the same promotion trigger. Warnings that are never confirmed by waking evidence expire in ~14 days. Warnings that are confirmed persist and inform future agent behavior. The economics prevent threat hoarding.

4. **Composable attack library**: Attack primitives stored as Signals in a dedicated partition can be combined into compound threats. The composability graph (which primitives combine well) is learned from historical success rates, making each dream cycle's threat generation more sophisticated than the last.

5. **Non-bypassable containment**: The nightmare containment Cell operates outside the agent's control loop, in the safety sidecar. The agent cannot suppress nightmare reports, skip cooldown, or bypass the quarantine check on the staging write path.

---

## Feedback Loops

1. **Threat simulation -> Staging -> Confirmation -> Promoted Warning -> Agent avoids failure -> Fewer threats to simulate**: The defensive loop. Threat rehearsal produces Warning Signals. Those that are confirmed by waking evidence get promoted. Promoted Warnings influence agent behavior (injected into system prompts via playbook rules), helping the agent avoid the threats. Over time, the agent becomes more resilient, and threat simulation finds fewer gaps.

2. **Nightmare detection -> Cooldown -> Reduced adversarial intensity -> Fewer nightmares -> Cooldown expires -> Normal intensity**: The safety oscillator. A nightmare triggers cooldown, which reduces the probability of generating another nightmare, which lets the cooldown expire, restoring normal adversarial intensity. This prevents a runaway nightmare spiral where one detected nightmare causes more aggressive dreaming that produces more nightmares.

3. **Attack knowledge -> Better threat generation -> More realistic threats -> Better rehearsal -> Stronger defenses -> Harder to generate effective attacks -> Drives novel attack discovery**: The red team arms race. As the agent's defenses improve, the threat simulator must generate more sophisticated attacks to find gaps. This drives the attack knowledge base toward novel, compound threats -- the same arms race dynamic observed in AutoRedTeamer (2025).

4. **FMEA bottom-up + FTA top-down -> Comprehensive coverage -> Fewer missed threats -> Better-calibrated Bayesian priors -> More accurate risk ranking -> Better mitigation allocation**: The convergence loop. FMEA and FTA provide complementary coverage. Bayesian updating sharpens probability estimates as evidence accumulates. Better estimates lead to better resource allocation for mitigations.

5. **Nightmare rate trend -> Dream journal -> Policy adjustment -> Modified constitution -> Classifier recalibration -> Changed nightmare rate**: The meta-safety loop. If the nightmare rate trends upward (detected via dream journal analysis), the safety policy is reviewed. A tighter constitution reduces false negatives. A looser constitution reduces false positives. The trend feeds back into the policy that governs the pipeline.

---

## Open Questions

1. **Constitutional rules authorship**: The harm classifier uses a constitution of natural language rules. Who writes these rules? If the agent writes its own safety constitution, there is a circularity risk (the entity being constrained defines its own constraints). If a human writes them, they may not cover agent-specific edge cases. A hybrid approach -- human-authored base rules with agent-proposed extensions that require human approval -- is likely necessary but adds operational overhead.

2. **Capability delta grounding**: The CapabilityDeltaCell measures "implied capabilities" from hypothesis content. Extracting implied capabilities from natural language is itself an LLM task and is subject to the same uncertainty as the content being checked. Should the capability set be derived from a formal tool manifest rather than extracted from text? The formal approach is more precise but cannot catch implicit capability escalation (e.g., chaining two safe tools to achieve an unsafe outcome).

3. **Cross-agent nightmare correlation**: In a fleet, if Agent A generates a nightmare and Agent B independently generates a similar nightmare (HDC similarity > 0.70), does this indicate a systemic vulnerability that the fleet should address? Or does it indicate a systematic bias in the dream generation process that produces false positives? The distinction matters for whether the response is "strengthen defenses" or "recalibrate the classifier."

4. **Adversarial dreaming ROI**: Threat simulation consumes 10-40% of the REM phase budget. Is this investment justified? The ROI depends on whether threat-generated Warnings actually prevent real failures. Measuring this requires a controlled experiment: a fleet where some agents run threat simulation and others do not, with the same task distribution. The dream journal provides the data; the experiment design is the open question.

5. **Nightmare false positive cost**: A false positive in nightmare detection quarantines a benign hypothesis that could have been valuable. The false positive rate of the four-stage pipeline is the product of per-stage false positive rates, but Stage 1 (LLM classifier) has a non-zero and hard-to-bound false positive rate. Should the pipeline include a "false positive recovery" path where quarantined hypotheses are re-evaluated after a cooling-off period, similar to how the staging partition allows expired entries to be thawed?
