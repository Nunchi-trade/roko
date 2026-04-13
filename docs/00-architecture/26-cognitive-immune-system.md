# Cognitive Immune System

> **Abstract:** Roko operates in adversarial environments — external data sources lie, tools fail
> silently, LLMs hallucinate confidently, and even the agent's own memories can become corrupted
> through cascading errors. The Cognitive Immune System (CIS) is a defense-in-depth architecture
> that protects knowledge integrity through five mechanisms: taint propagation tracking, anomaly
> detection on knowledge mutations, anti-knowledge quarantine, threat simulation (red-team
> probing), and immune memory that remembers past attacks. This document unifies these mechanisms
> into a single subsystem that integrates with the Gate pipeline, Neuro knowledge store, and
> Daimon affect system.

> **Implementation**: Specified

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [05-provenance-and-attestation](./05-provenance-and-attestation.md), [06-synapse-traits](./06-synapse-traits.md), [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md)
**Key sources**:
- Chen et al. 2024, NeurIPS (arXiv:2407.12784) — AgentPoison: Red-teaming LLM Agents via Memory Poisoning
- arXiv:2411.18948 (2024) — RevPRAG: Revealing Poisoning Attacks via LLM Activation Analysis
- arXiv:2601.05504 (2025) — Memory Poisoning Attack and Defense on Memory-Based LLM Agents
- arXiv:2407.10867 (2024) — Provable Robustness of GNNs Against Data Poisoning
- NIST AI 100-2e2025 — Adversarial Machine Learning: Taxonomy of Attacks and Mitigations
- Matzinger 2002, Science 296 — The Danger Model (immunology)
- Forrest et al. 1994 — Self-NonSelf Discrimination in Computer Security (negative selection)

---

## 1. Threat Model

### 1.1 Attack Surfaces

Roko's knowledge can be corrupted through five attack vectors:

| Vector | Example | Severity | Existing Defense |
|---|---|---|---|
| **Prompt injection** | Malicious instructions embedded in tool output | Critical | Safety layer pre/post checks |
| **Memory poisoning** | Agent stores hallucinated "facts" that cascade | High | Gate pipeline (partial) |
| **Knowledge drift** | Correct knowledge becomes stale but retains high tier | Medium | Decay variants (TTL, HalfLife) |
| **Cascading taint** | Tainted Engram used as input → output is also tainted | High | Provenance.tainted flag (partial) |
| **Adversarial retrieval** | Crafted Engrams that score high but contain misinformation | Critical | None currently |

Chen et al. (2024) demonstrated that RAG-based agents can be backdoored with just 0.1%
poisoning ratio, achieving 82% attack success. arXiv:2601.05504 (2025) showed that episodic
and semantic memory stores are both vulnerable. Roko's four-tier knowledge system (Transient →
Working → Consolidated → Persistent) means that a single poisoned entry, if it survives
promotion, can contaminate the agent's long-term reasoning.

### 1.2 The Biological Analogy

The CIS draws from two immunological models:

1. **Self/Non-Self (Forrest et al. 1994)**: The immune system maintains "detectors" trained on
   self-patterns. Anything that doesn't match triggers an immune response. In Roko: knowledge
   that deviates from established patterns triggers anomaly detection.

2. **Danger Model (Matzinger 2002)**: The immune system doesn't respond to "foreign" per se —
   it responds to *danger signals*. Tissue damage, not foreignness, triggers immunity. In Roko:
   the CIS responds to *knowledge damage signals* — unexpected Score drops, failed gate
   verdicts, contradicted predictions — not merely to "external" data.

---

## 2. Architecture

### 2.1 Five Defense Layers

```
┌─────────────────────────────────────────────────────┐
│                  Layer 5: IMMUNE MEMORY              │
│  Remember past attacks; inoculate against repeats    │
├─────────────────────────────────────────────────────┤
│                  Layer 4: RED TEAM PROBES            │
│  Simulate attacks against own knowledge; find gaps   │
├─────────────────────────────────────────────────────┤
│                  Layer 3: QUARANTINE                 │
│  Isolate suspicious knowledge; prevent propagation   │
├─────────────────────────────────────────────────────┤
│                  Layer 2: ANOMALY DETECTION          │
│  Statistical monitoring of knowledge mutations       │
├─────────────────────────────────────────────────────┤
│                  Layer 1: TAINT PROPAGATION          │
│  Track data lineage; flag downstream of tainted      │
└─────────────────────────────────────────────────────┘
```

### 2.2 Core Types

```rust
/// Threat classification for knowledge corruption events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThreatClass {
    /// Data from external source contradicts internal knowledge.
    ExternalContradiction,
    /// Agent's own output flagged by gate as inconsistent.
    InternalInconsistency,
    /// Knowledge mutation exceeds statistical norms.
    AnomalousMutation,
    /// Taint propagated from upstream corrupted source.
    TaintCascade,
    /// Known attack pattern matched from immune memory.
    KnownAttackPattern,
    /// Simulated red-team probe succeeded (vulnerability found).
    RedTeamSuccess,
}

/// A detected threat event in the knowledge system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatEvent {
    pub id: Uuid,
    pub timestamp: SystemTime,
    pub threat_class: ThreatClass,
    /// The Engram(s) involved.
    pub affected_engrams: Vec<ContentHash>,
    /// Confidence that this is a genuine threat (0.0 to 1.0).
    pub confidence: f64,
    /// Severity score (0.0 = informational, 1.0 = critical).
    pub severity: f64,
    /// Provenance chain that led to this detection.
    pub detection_chain: Vec<String>,
    /// Recommended response action.
    pub recommended_action: ImmuneAction,
}

/// Actions the immune system can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImmuneAction {
    /// Log but take no action (low confidence / low severity).
    Monitor,
    /// Mark Engrams as tainted; downstream consumers are warned.
    Taint,
    /// Move Engrams to quarantine zone; excluded from queries until reviewed.
    Quarantine,
    /// Demote knowledge tier (e.g., Consolidated → Working).
    Demote,
    /// Create AntiKnowledge entry to prevent future re-ingestion.
    Falsify,
    /// Trigger full re-verification of affected lineage subgraph.
    Reverify,
}
```

---

## 3. Layer 1: Taint Propagation Tracking

### 3.1 Extending Provenance

Roko's existing `Provenance` struct has a `tainted: bool` flag. The CIS extends this to a
rich taint model that tracks *how* and *why* data is tainted:

```rust
/// Extended taint information attached to Provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintInfo {
    /// Is this Engram tainted?
    pub tainted: bool,
    /// Source of taint (if tainted): the ContentHash of the first tainted ancestor.
    pub taint_origin: Option<ContentHash>,
    /// Taint depth: how many lineage hops from the original taint source.
    pub taint_depth: u32,
    /// Taint reason.
    pub taint_reason: Option<ThreatClass>,
    /// Taint timestamp.
    pub tainted_at: Option<SystemTime>,
    /// Confidence that the taint is genuine (not a false positive).
    pub taint_confidence: f64,
}

impl TaintInfo {
    pub fn clean() -> Self {
        Self {
            tainted: false,
            taint_origin: None,
            taint_depth: 0,
            taint_reason: None,
            tainted_at: None,
            taint_confidence: 0.0,
        }
    }

    /// Propagate taint to a derived Engram.
    /// Confidence degrades with depth (geometric decay).
    pub fn propagate(&self, decay_factor: f64) -> Self {
        Self {
            tainted: true,
            taint_origin: self.taint_origin,
            taint_depth: self.taint_depth + 1,
            taint_reason: self.taint_reason,
            tainted_at: Some(SystemTime::now()),
            taint_confidence: self.taint_confidence * decay_factor,
        }
    }
}
```

### 3.2 Lineage-Based Taint Propagation Algorithm

```
ALGORITHM: TaintPropagation(source_hash, reason, confidence)

1. Mark source Engram as tainted with (reason, confidence)
2. BFS over lineage DAG from source:
   For each child Engram reachable from source:
     a. Compute propagated confidence = parent_confidence × DECAY_FACTOR
     b. If propagated_confidence > TAINT_THRESHOLD (default 0.1):
        - Mark child as tainted
        - Record taint_depth = parent_depth + 1
        - Add child to BFS queue
     c. If propagated_confidence ≤ TAINT_THRESHOLD:
        - Stop propagating (taint has decayed below detection)
3. Return set of all newly tainted Engrams
4. For each newly tainted Engram in Consolidated or Persistent tier:
   - Emit ThreatEvent with severity = tier_weight × confidence
   - Recommend action based on severity matrix

PARAMETERS:
  DECAY_FACTOR = 0.8    (20% confidence loss per hop)
  TAINT_THRESHOLD = 0.1 (stop propagating below 10% confidence)
  MAX_DEPTH = 10        (circuit breaker on deep lineage chains)
```

---

## 4. Layer 2: Anomaly Detection

### 4.1 Statistical Monitoring

The CIS maintains running statistics on knowledge mutations and triggers alerts when
mutations deviate from established baselines.

```rust
/// Anomaly detector for knowledge store mutations.
pub struct KnowledgeAnomalyDetector {
    /// Exponential moving averages of mutation rates per knowledge type.
    pub mutation_ema: HashMap<KnowledgeType, ExponentialMovingAverage>,
    /// EMA of tier promotion rates.
    pub promotion_ema: ExponentialMovingAverage,
    /// EMA of tier demotion rates.
    pub demotion_ema: ExponentialMovingAverage,
    /// Z-score threshold for triggering anomaly alert.
    pub z_threshold: f64,  // default: 3.0 (3 sigma)
    /// Minimum observations before anomaly detection activates.
    pub warmup_observations: usize,  // default: 100
    /// Sliding window for variance estimation.
    pub window_size: usize,  // default: 500
}

/// Exponential moving average with variance tracking.
pub struct ExponentialMovingAverage {
    pub mean: f64,
    pub variance: f64,
    pub alpha: f64,  // smoothing factor, default 0.05
    pub observations: usize,
}

impl ExponentialMovingAverage {
    pub fn update(&mut self, value: f64) {
        self.observations += 1;
        let delta = value - self.mean;
        self.mean += self.alpha * delta;
        self.variance = (1.0 - self.alpha) * (self.variance + self.alpha * delta * delta);
    }

    pub fn z_score(&self, value: f64) -> f64 {
        let stddev = self.variance.sqrt().max(f64::EPSILON);
        (value - self.mean) / stddev
    }
}
```

### 4.2 Monitored Signals

| Signal | Baseline | Anomaly Indicator |
|---|---|---|
| Mutation rate (Engrams changed per tick) | EMA of recent ticks | Spike > 3σ above mean |
| Promotion rate (Transient→Working→...) | EMA of recent sessions | Rapid mass promotion |
| Contradiction rate (new vs. existing) | EMA over window | Sudden cluster of contradictions |
| Score distribution shift | Kolmogorov-Smirnov vs. baseline | Distribution divergence > 0.1 |
| Taint propagation fan-out | EMA of fan-out per taint event | Single source tainting > 50 Engrams |

### 4.3 Detection Inspired by RevPRAG

arXiv:2411.18948 (RevPRAG) achieves 98% TPR at 1% FPR by analyzing internal LLM activations
to detect poisoned generations. The CIS adapts this principle to Engram scoring:

```
ALGORITHM: ScoreDistributionAnomaly(new_engram, knowledge_type)

1. Compute new_engram.score.effective()
2. Get baseline distribution for knowledge_type from EMA
3. Compute z = (effective - baseline_mean) / baseline_stddev
4. If |z| > Z_THRESHOLD:
   a. If z > 0: suspiciously high score — possible adversarial inflation
   b. If z < 0: suspiciously low score — possible corruption
   c. Emit ThreatEvent(AnomalousMutation, confidence=sigmoid(|z| - Z_THRESHOLD))
5. Return (is_anomalous, z_score)
```

---

## 5. Layer 3: Quarantine

### 5.1 Quarantine Zone

Suspicious Engrams are moved to a quarantine zone — a separate Substrate partition that is
excluded from normal `query()` results but preserved for investigation.

```rust
/// Quarantine zone for suspicious knowledge.
pub struct QuarantineZone {
    /// Quarantined Engrams, indexed by ContentHash.
    entries: HashMap<ContentHash, QuarantineEntry>,
    /// Maximum quarantine duration before forced resolution.
    max_quarantine_duration: Duration,  // default: 72 hours
    /// Auto-resolve policy after max duration.
    auto_resolve: QuarantineResolution,  // default: Demote
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineEntry {
    pub engram_hash: ContentHash,
    pub quarantined_at: SystemTime,
    pub reason: ThreatClass,
    pub original_tier: KnowledgeTier,
    pub threat_event_id: Uuid,
    /// Number of reverification attempts.
    pub reverify_attempts: u32,
    /// Latest reverification result.
    pub last_reverify: Option<GateVerdict>,
}

#[derive(Debug, Clone, Copy)]
pub enum QuarantineResolution {
    /// Release back to original tier (threat was false positive).
    Release,
    /// Demote to lower tier (uncertain).
    Demote,
    /// Convert to AntiKnowledge (confirmed corrupted).
    Falsify,
    /// Delete entirely (dangerous + no learning value).
    Purge,
}
```

### 5.2 Quarantine Workflow

```
1. DETECT    → Anomaly detector or taint propagation flags an Engram
2. QUARANTINE → Move Engram to quarantine zone; exclude from queries
3. NOTIFY    → Emit ThreatEvent; raise Daimon alarm signal
4. REVERIFY  → Run affected Engram through full gate pipeline
5. RESOLVE:
   a. Gate passes + confidence recovers → RELEASE (false alarm)
   b. Gate passes but confidence low    → DEMOTE (downgrade tier)
   c. Gate fails                        → FALSIFY (create AntiKnowledge)
   d. Timeout (72h, no resolution)      → AUTO_RESOLVE per policy
6. LEARN     → Record outcome in immune memory
```

---

## 6. Layer 4: Red Team Probes

### 6.1 Self-Adversarial Testing

During Delta (consolidation) cycles, the CIS runs automated red-team probes against its own
knowledge store — testing for vulnerabilities before an adversary finds them.

```rust
/// Red team probe generator.
pub struct RedTeamProber {
    /// Probe strategies, tried in rotation.
    pub strategies: Vec<ProbeStrategy>,
    /// Maximum probes per Delta cycle.
    pub max_probes_per_cycle: usize,  // default: 10
    /// Minimum knowledge tier to probe (skip Transient).
    pub min_probe_tier: KnowledgeTier,  // default: Working
}

#[derive(Debug, Clone)]
pub enum ProbeStrategy {
    /// Test if contradictory knowledge can be injected.
    ContradictionInjection {
        /// Target knowledge type to contradict.
        target_type: KnowledgeType,
    },
    /// Test if high-scored garbage passes the gate.
    ScoreInflation {
        /// Synthetic Engram with inflated score.
        inflation_magnitude: f64,
    },
    /// Test if taint propagation correctly reaches all descendants.
    TaintCoverageCheck {
        /// Source hash to taint.
        source_hash: ContentHash,
        /// Expected descendant count.
        expected_descendants: usize,
    },
    /// Test if a known-false claim can survive promotion.
    PromotionBypass {
        /// False claim to attempt promoting.
        false_claim: String,
    },
}
```

### 6.2 Probe Execution

```
ALGORITHM: RedTeamCycle(knowledge_store, gate_pipeline)

For each strategy in strategies (up to max_probes_per_cycle):
  1. Generate synthetic adversarial Engram per strategy
  2. Attempt to insert into knowledge store (bypassing CIS for this probe only)
  3. Run query() to see if synthetic Engram appears in results
  4. Run gate pipeline on synthetic Engram
  5. Record outcome:
     a. If synthetic Engram passed gates → VULNERABILITY FOUND
        - Emit ThreatEvent(RedTeamSuccess, severity=high)
        - Record in immune memory for future detection
     b. If synthetic Engram rejected by gates → DEFENSE HOLDS
        - Record successful defense pattern
  6. Remove synthetic Engram from knowledge store (cleanup)
```

---

## 7. Layer 5: Immune Memory

### 7.1 Remembering Attacks

The immune system maintains long-term memory of attack patterns, analogous to adaptive
immunity (B-cells and T-cells in biology).

```rust
/// Long-term immune memory. Persists to .roko/learn/immune-memory.json.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImmuneMemory {
    /// Known attack signatures (threat class → detection patterns).
    pub attack_signatures: Vec<AttackSignature>,
    /// False positive record (to avoid crying wolf).
    pub false_positives: Vec<FalsePositiveRecord>,
    /// Defense effectiveness scores (which defenses work against which threats).
    pub defense_scores: HashMap<(ThreatClass, ImmuneAction), EffectivenessScore>,
    /// Total threat events processed.
    pub events_processed: u64,
    /// Last consolidation timestamp.
    pub last_consolidated: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackSignature {
    pub id: Uuid,
    pub threat_class: ThreatClass,
    /// HDC fingerprint of the attack pattern (for similarity matching).
    pub fingerprint: Vec<u8>,
    /// Score distribution anomaly pattern at time of detection.
    pub score_pattern: ScoreDistributionSnapshot,
    /// How many times this signature has been matched.
    pub match_count: u32,
    /// Effectiveness of defense when this pattern was encountered.
    pub best_defense: ImmuneAction,
    pub first_seen: SystemTime,
    pub last_seen: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalsePositiveRecord {
    pub threat_event_id: Uuid,
    pub misclassified_as: ThreatClass,
    pub actual_status: String,  // "benign", "data_quality", etc.
    pub recorded_at: SystemTime,
}

pub struct EffectivenessScore {
    pub successes: u32,
    pub failures: u32,
    pub effectiveness_rate: f64,
}
```

### 7.2 Signature Matching

When a new ThreatEvent is detected, the CIS checks immune memory for matching signatures
using HDC similarity (Kanerva 2009):

```
ALGORITHM: ImmuneRecognition(threat_event)

1. Compute HDC fingerprint of threat_event's affected Engrams
2. For each stored attack_signature:
   a. Compute cosine similarity between fingerprints
   b. If similarity > RECOGNITION_THRESHOLD (default 0.85):
      - KNOWN ATTACK: apply best_defense immediately (skip analysis)
      - Increment match_count
      - Update last_seen
      - Return early with recommended action
3. If no match: NOVEL ATTACK
   a. Run full analysis pipeline (Layers 2-4)
   b. After resolution, create new AttackSignature
   c. Store in immune memory

PARAMETERS:
  RECOGNITION_THRESHOLD = 0.85
  HDC_DIMENSION = 10240 (matches bardo-primitives)
```

---

## 8. Daimon Integration: Immune Alarm Signals

The CIS communicates threat severity to the Daimon, which adjusts the agent's affective state.
This implements Matzinger's (2002) danger model — the immune system responds to danger signals,
not merely to "foreign" content.

```rust
/// Immune alarm signal sent to the Daimon.
pub struct ImmuneAlarm {
    /// Overall threat level (0.0 = no threats, 1.0 = critical breach).
    pub threat_level: f64,
    /// PAD modulation recommended by the immune system.
    pub pad_delta: PadVector,
    /// Suggested behavioral state shift.
    pub suggested_state: Option<BehavioralState>,
}

impl ImmuneAlarm {
    /// Generate alarm from recent threat events.
    pub fn from_events(events: &[ThreatEvent]) -> Self {
        let max_severity = events.iter()
            .map(|e| e.severity)
            .fold(0.0_f64, f64::max);
        let avg_confidence = events.iter()
            .map(|e| e.confidence)
            .sum::<f64>() / events.len().max(1) as f64;

        let threat_level = max_severity * avg_confidence;

        // High threat → decrease pleasure, increase arousal, decrease dominance
        let pad_delta = PadVector {
            pleasure: -0.3 * threat_level,
            arousal: 0.5 * threat_level,
            dominance: -0.2 * threat_level,
        };

        let suggested_state = if threat_level > 0.8 {
            Some(BehavioralState::Cautious)
        } else if threat_level > 0.4 {
            Some(BehavioralState::Focused)
        } else {
            None
        };

        Self { threat_level, pad_delta, suggested_state }
    }
}
```

---

## 9. Configuration

```toml
[immune]
# Enable/disable the cognitive immune system.
enabled = true

[immune.taint]
# Confidence decay per lineage hop.
decay_factor = 0.8
# Stop propagating below this confidence.
taint_threshold = 0.1
# Maximum lineage depth to traverse.
max_depth = 10

[immune.anomaly]
# Z-score threshold for anomaly alerts.
z_threshold = 3.0
# Minimum observations before detection activates.
warmup_observations = 100
# EMA smoothing factor.
ema_alpha = 0.05

[immune.quarantine]
# Maximum quarantine duration.
max_duration_hours = 72
# Auto-resolve policy: "release", "demote", "falsify", "purge".
auto_resolve = "demote"
# Maximum concurrent quarantined Engrams.
max_quarantined = 1000

[immune.redteam]
# Maximum probes per Delta consolidation cycle.
max_probes_per_cycle = 10
# Minimum knowledge tier to probe.
min_probe_tier = "working"

[immune.memory]
# HDC similarity threshold for recognizing known attacks.
recognition_threshold = 0.85
# Maximum stored attack signatures.
max_signatures = 500
# Persist location.
path = ".roko/learn/immune-memory.json"
```

---

## 10. Integration Wiring

### 10.1 Into the Universal Cognitive Loop

| Loop Step | CIS Integration |
|---|---|
| 1. PERCEIVE | Query results filtered: quarantined Engrams excluded |
| 2. EVALUATE | Score checked against anomaly detector baseline |
| 3. ATTEND | Tainted Engrams receive Score penalty (0.5× multiplier) |
| 4. INTEGRATE | Taint info included in composed context (agents see warnings) |
| 5. ACT | No direct integration (agent unaware of CIS) |
| 6. VERIFY | Gate verdicts feed anomaly detector + trigger taint if failed |
| 7. PERSIST | New Engrams inherit taint from parent lineage |
| 8. ADAPT | Policy receives ThreatEvents; adjusts future gate strictness |
| 9. META-COGNIZE | ImmuneAlarm feeds Daimon; affects next tick's caution level |

### 10.2 Into Existing Crates

| Crate | Integration Point | Change |
|---|---|---|
| `roko-core` | `Provenance` struct | Add `taint_info: TaintInfo` field |
| `roko-neuro` | `NeuroStore::query()` | Filter out quarantined Engrams |
| `roko-gate` | `GatePipeline::verify()` | On failure → trigger taint propagation |
| `roko-daimon` | `DaimonState` | Receive `ImmuneAlarm`, adjust PAD |
| `roko-learn` | `EpisodeLogger` | Log ThreatEvents alongside episodes |
| `roko-dreams` | Delta cycle | Schedule RedTeamProber during consolidation |
| `roko-conductor` | Circuit breaker | Trigger on threat_level > 0.8 |
| `roko-fs` | `FileSubstrate` | Quarantine partition (separate JSONL file) |

---

## 11. Test Criteria

| Test | What It Validates | Type |
|---|---|---|
| `test_taint_propagates_through_lineage` | Taint reaches all descendants within MAX_DEPTH | Unit |
| `test_taint_confidence_decays` | Each hop reduces confidence by DECAY_FACTOR | Unit |
| `test_taint_stops_at_threshold` | Propagation halts when confidence < TAINT_THRESHOLD | Unit |
| `test_anomaly_z_score_detection` | Spike > 3σ triggers ThreatEvent | Unit |
| `test_anomaly_warmup_no_false_positives` | No alerts during first 100 observations | Unit |
| `test_quarantine_excludes_from_query` | Quarantined Engrams absent from query results | Integration |
| `test_quarantine_auto_resolve_on_timeout` | After 72h, quarantine auto-resolves per policy | Unit |
| `test_redteam_detects_score_inflation` | Inflated-score synthetic Engram is caught | Integration |
| `test_redteam_cleanup` | Synthetic Engrams removed after probe | Unit |
| `test_immune_memory_recognizes_repeat` | Second occurrence of known signature matches | Unit |
| `test_immune_memory_hdc_similarity` | Similarity > 0.85 triggers recognition | Unit |
| `test_false_positive_recording` | False positives tracked to reduce future alerts | Unit |
| `test_immune_alarm_shifts_pad` | High threat → pleasure decreases, arousal increases | Unit |
| `test_gate_failure_triggers_taint` | Failed gate verdict taints the Engram | Integration |

---

## 12. Theoretical Foundations

### 12.1 Biological Immune System Analogy

| Biological Component | CIS Component | Function |
|---|---|---|
| Innate immunity | Taint propagation + anomaly detection | Fast, non-specific first response |
| Adaptive immunity | Immune memory + HDC signature matching | Slow, specific, remembers past attacks |
| B-cells (antibodies) | Attack signatures | Recognize specific threat patterns |
| T-cells (killer cells) | Quarantine + Falsify actions | Neutralize identified threats |
| Fever (systemic alarm) | ImmuneAlarm → Daimon | System-wide behavioral change |
| Autoimmune disorder | False positives | Immune system attacks own knowledge |
| Immunodeficiency | Warmup period, disabled CIS | System vulnerable without defenses |

### 12.2 Negative Selection (Forrest et al. 1994)

The red team probes implement negative selection: generate random "non-self" patterns and check
if they survive the immune system. If they do, the immune system has a gap. The CIS then
creates a new detector (attack signature) to cover the gap.

### 12.3 NIST AI 100-2 Alignment

The CIS maps to NIST's adversarial ML taxonomy:

| NIST Category | CIS Defense |
|---|---|
| Data poisoning | Anomaly detection + taint propagation |
| Model evasion | Red team probes test gate bypass |
| Model extraction | Out of scope (Roko is open-source) |
| Supply chain | Provenance attestation (planned, see [05-provenance-and-attestation](./05-provenance-and-attestation.md)) |

---

## Cross-References

- [05-provenance-and-attestation](./05-provenance-and-attestation.md) — Provenance struct that CIS extends with TaintInfo
- [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) — Neuro knowledge tiers that CIS protects
- [25-attention-as-currency](./25-attention-as-currency.md) — Tainted Engrams cost more AT (discourage use)
- [28-emergent-goal-structures](./28-emergent-goal-structures.md) — CIS prevents corrupted knowledge from generating false goals
- [Topic 04: Verification](../04-verification/INDEX.md) — Gate pipeline that feeds CIS detections
- [Topic 06: Neuro](../06-neuro/INDEX.md) — Knowledge store that CIS protects
- [Topic 09: Daimon](../09-daimon/INDEX.md) — Affect system that CIS modulates via alarms
- [Topic 10: Dreams](../10-dreams/INDEX.md) — Delta cycles that run red team probes
- [Topic 11: Safety](../11-safety/INDEX.md) — Safety layer for capability-level protection
