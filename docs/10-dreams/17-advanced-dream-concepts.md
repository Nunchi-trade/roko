# Advanced Dream Concepts: Dream Sharing, Nightmare Detection, and Dream Journals

> **Layer**: Cognitive Cross-Cut (L4 Orchestration mesh integration, L3 Harness safety)
>
> **Synapse Traits**: `Policy` (dream sharing policy), `Gate` (nightmare detection gate), `Substrate` (dream journal persistence)
>
> **Crate**: `roko-dreams` (planned extensions)
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [15-cross-system-integration.md](15-cross-system-integration.md)

> **Implementation**: Planned

---

## Overview

This document covers three extensions to the core dream cycle that become necessary once multiple agents are operating simultaneously and the dream cycle matures from a single-agent introspective tool into a multi-agent knowledge fabric:

1. **Dream Sharing** — agents sharing dream insights through the L4 orchestration mesh
2. **Nightmare Detection** — safety filtering for dream-generated content that could be harmful
3. **Dream Journals** — structured, queryable long-term records of dream cycles for trend analysis and metacognitive self-improvement

These concepts are distinct from the base three-phase cycle (NREM replay, REM imagination, consolidation) and operate as cross-cutting concerns layered on top of that foundation.

---

## Dream Sharing: Agents Sharing Dream Insights Through the Mesh

### Motivation

A fleet of agents operating on related tasks accumulates complementary dream insights. Without sharing, each agent independently re-derives threat patterns and heuristics that other agents have already rehearsed. Dream sharing propagates insights across the fleet, compressing the time to collective competence.

The challenge is that dream content is inherently agent-specific: it is grounded in each agent's episode history, role context, and confidence calibration. Naive broadcasting of raw hypotheses produces noise. The dream sharing protocol must be selective, confidence-aware, and privacy-preserving.

### Theoretical Foundation

**Federated Distillation (FD)**: Rather than sharing raw model parameters or raw hypotheses, agents share soft predictions — distilled summaries of what they have learned — which preserves privacy while transmitting the essential knowledge signal. Applied to dream sharing: agents do not share the raw episode memories that generated a hypothesis, only the hypothesis itself (the distilled output).

**Selective-FD (Nature Communications 2023)**: Only high-confidence, high-accuracy predictions are shared. Transmitting uncertain hypotheses degrades collective performance because recipients cannot distinguish genuine insight from noise. Dream sharing in Roko adopts the Selective-FD criterion: only hypotheses at Tier 3 confidence (≥ 0.75, after consolidation scoring) enter the sharing pool.

**Stigmergy (Grassé 1959)**: In ant colonies, individuals deposit pheromone traces that modify the environment for subsequent agents — indirect coordination through shared medium. Dream sharing implements a stigmergic analog: insights deposited into the mesh accumulate and decay over time.

Pheromone decay equation:
```
τ(t+1) = (1 - ρ) · τ(t) + Δτ_k
```

Where:
- τ(t) = confidence weight of a shared insight at time t
- ρ = evaporation rate (default: 0.05 per dream cycle)
- Δτ_k = confidence boost when agent k independently corroborates the insight

An insight that is never corroborated decays to zero. An insight corroborated by multiple agents accumulates weight.

### Sharing Modes

Three modes govern what an agent shares and when:

| Mode | Trigger | What Is Shared | Privacy |
|------|---------|---------------|---------|
| **Broadcast** | Every dream cycle, unconditionally | All staged hypotheses at Tier 3 confidence | Full mesh visibility |
| **Selective** | Per-hypothesis confidence gate | Only hypotheses with confidence ≥ 0.75 and novelty score ≥ 0.6 (not already known to mesh) | Filtered |
| **Solicited** | Another agent explicitly requests insights on a topic | Insights matching the request topic, regardless of confidence tier | Point-to-point |

The default mode is **Selective**. Broadcast is reserved for high-urgency threat patterns (e.g., a newly discovered Tier 3 threat that has not been seen before). Solicited sharing enables targeted knowledge transfer without polluting the mesh.

### Confidence Decay on Transit

Shared knowledge travels through the mesh hop-by-hop. Each hop introduces uncertainty because the receiving agent cannot verify the episode history that generated the insight. A **Weismann barrier** analog is applied: each hop degrades confidence by a factor of 0.85.

```
confidence_at_recipient = original_confidence × 0.85^(hop_count)
```

A hypothesis with confidence 0.90 that travels through two hops arrives with confidence 0.90 × 0.85² ≈ 0.65. The recipient may promote this to Tier 3 only after independent corroboration during its own waking episodes.

### Privacy Boundaries

Dream content frequently contains failure analyses of sensitive operations: security-adjacent tool chains, confidential task parameters, and proprietary reasoning patterns. Dream sharing must respect the following constraints:

1. **Role isolation**: Agents in isolated roles (e.g., security auditor, secrets manager) do not share any dream content outside their role boundary, regardless of mode.
2. **Episode sanitization**: Before sharing, all episode references are replaced with anonymized summaries. The recipient receives the insight (pattern) not the episode (data).
3. **Solicited requests require policy approval**: The sharing Policy trait evaluates solicited requests before responding.

### Rust Structures

```rust
pub struct DreamShareConfig {
    /// Sharing mode for this agent.
    pub mode: DreamShareMode,
    /// Minimum confidence for selective sharing.
    pub selective_confidence_threshold: f64,   // default: 0.75
    /// Minimum novelty score for selective sharing (0 = already known, 1 = entirely new).
    pub selective_novelty_threshold: f64,       // default: 0.60
    /// Stigmergy evaporation rate per dream cycle.
    pub evaporation_rate: f64,                  // default: 0.05, range: 0.01-0.20
    /// Confidence multiplier applied per mesh hop.
    pub hop_confidence_decay: f64,             // default: 0.85, range: 0.70-0.95
    /// Maximum hops a shared insight may travel.
    pub max_hops: usize,                        // default: 3
    /// Whether to sanitize episode references before sharing.
    pub sanitize_episodes: bool,               // default: true
    /// Role boundary enforcement: do not share outside these roles.
    pub allowed_recipient_roles: Vec<String>,
}

pub enum DreamShareMode {
    Broadcast,
    Selective,
    Solicited,
    Disabled,
}

pub struct SharedDreamInsight {
    pub insight_id: String,
    pub source_agent_id: String,
    pub source_cycle_id: String,
    pub hypothesis_summary: String,
    pub original_confidence: f64,
    pub current_confidence: f64,
    pub hop_count: usize,
    pub corroborating_agents: Vec<String>,
    pub stigmergy_weight: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub tags: Vec<String>,
}

pub struct DreamShareProtocol {
    pub config: DreamShareConfig,
    /// Buffer of insights received from the mesh this cycle.
    pub inbound_buffer: Vec<SharedDreamInsight>,
    /// Buffer of insights ready to transmit this cycle.
    pub outbound_buffer: Vec<SharedDreamInsight>,
    /// Accumulated stigmergy weights for mesh-resident insights.
    pub stigmergy_map: std::collections::HashMap<String, f64>,
}

impl DreamShareProtocol {
    /// Apply one evaporation step to all stigmergy weights.
    /// Called at the start of each dream cycle.
    pub fn evaporate(&mut self) {
        for weight in self.stigmergy_map.values_mut() {
            *weight *= 1.0 - self.config.evaporation_rate;
        }
        // Prune insights below 0.01 (effectively zero).
        self.stigmergy_map.retain(|_, v| *v >= 0.01);
    }

    /// Corroborate a mesh insight from this agent's own waking experience.
    pub fn corroborate(&mut self, insight_id: &str, agent_id: &str, delta_tau: f64) {
        let entry = self.stigmergy_map.entry(insight_id.to_string()).or_insert(0.0);
        *entry = (1.0 - self.config.evaporation_rate) * *entry + delta_tau;
    }
}
```

---

## Nightmare Detection: When Dreams Produce Harmful Knowledge

### Motivation

The REM imagination phase is deliberately unconstrained: it recombines knowledge across domains, generates counterfactuals, and explores adversarial scenarios. This creative freedom is the source of the dream cycle's value. It is also a safety risk. An agent with access to tool knowledge, system architecture, and adversarial threat patterns can, in theory, synthesize a novel harmful strategy during a dream cycle — a **nightmare**.

A nightmare is not a failed dream. It is a dream that succeeded in generating novel knowledge, but the knowledge is harmful. The nightmare detector is a mandatory post-processing gate that runs before any dream output is written to the staging buffer.

### Nightmare Taxonomy

| Class | Description | Example |
|-------|-------------|---------|
| **1. Harmful strategy generation** | The dream synthesized a novel attack or exploitation strategy | A coding agent dreams up a privilege escalation path combining two known vulnerabilities |
| **2. Dangerous tool chain discovery** | The dream identified a sequence of legitimate tool calls that produces a harmful outcome | Combining file read + network write + process spawn in a way that exfiltrates data |
| **3. Safety constraint bypass paths** | The dream identified a way to satisfy a task goal while violating a safety constraint | A plan that technically completes the task while circumventing the access control check |
| **4. Policy violation knowledge** | The dream produced content that violates role policy (CBRN, disinformation, etc.) | A research agent's threat simulation generated a novel synthesis pathway |

### Multi-Stage Detection Pipeline

Detection proceeds through four sequential stages. A positive detection at any stage triggers containment; the pipeline does not continue to later stages for the flagged hypothesis.

```
hypothesis
    |
    v
[Stage 1: Harm classifier]
    |  Binary: harmful / benign
    |  Model: lightweight safety classifier (T2 tier)
    |
    v
[Stage 2: Domain-specific CBRN/security check]
    |  Structured check against known dangerous domains:
    |  chemical, biological, radiological, nuclear, cybersecurity exploitation
    |  Uses constrained vocabulary matching + embedding similarity
    |
    v
[Stage 3: Novelty-divergence check]
    |  Is this hypothesis significantly more capable than the agent's
    |  current tool set? A hypothesis that implies capabilities the
    |  agent doesn't have is suspicious.
    |  Measured by: capability_delta = implied_capability - known_capability
    |  Threshold: capability_delta > 0.5 triggers review
    |
    v
[Stage 4: Human escalation]
    |  Any hypothesis that passes Stage 1-3 but scores above
    |  the uncertainty threshold (entropy > 0.4) on any stage
    |  is escalated to human review before staging.
    |
    v
benign hypothesis → staging buffer
```

Reference: **Constitutional AI (Anthropic)**: The dream cycle's own LLM reasoning provides the first line of defense. Before passing a hypothesis to the harm classifier, the agent applies a self-critique prompt ("Does this hypothesis describe how to do something harmful?"). The CA self-critique is fast (single forward pass) and catches obvious cases without invoking the full multi-stage pipeline.

Reference: **PromptGuard (2025)**: Safety alignment techniques for generative models, with particular focus on prompt injection and jailbreak detection. The nightmare detector borrows the PromptGuard framing: treat dream output as a potentially adversarial prompt that the safety system must evaluate before it is trusted.

### Containment Protocol

When a nightmare is detected, the containment protocol runs unconditionally:

1. The flagged hypothesis is quarantined — it is not written to the staging buffer and cannot influence waking behavior.
2. A `NightmareReport` is written to `.roko/dreams/nightmares.jsonl` with full details.
3. The dream cycle's REM phase is terminated early (no further hypotheses generated this cycle).
4. A human review request is generated and held until a human operator acknowledges it.
5. The agent's adversarial dreaming intensity is temporarily reduced for the next 3 cycles (to reduce the probability of generating another nightmare while the first is under review).

No nightmare content is ever promoted to permanent knowledge without explicit human approval. The containment is strict and non-bypassable by the agent itself.

### Rust Structures

```rust
pub struct NightmareDetector {
    /// Harm classifier model tier.
    pub classifier_model_tier: ModelTier,       // default: T2 (Haiku-class for speed)
    /// Enable domain-specific CBRN/security check.
    pub enable_domain_check: bool,              // default: true
    /// Capability delta threshold for Stage 3.
    pub capability_delta_threshold: f64,        // default: 0.50, range: 0.20-0.80
    /// Entropy threshold above which to escalate to human review.
    pub escalation_entropy_threshold: f64,      // default: 0.40
    /// Path for nightmare log.
    pub nightmare_log_path: std::path::PathBuf, // default: .roko/dreams/nightmares.jsonl
    /// Number of dream cycles to reduce adversarial intensity after a nightmare.
    pub post_nightmare_cooldown_cycles: usize,  // default: 3
}

pub struct NightmareReport {
    pub nightmare_id: String,
    pub cycle_id: String,
    pub agent_id: String,
    pub detected_at: chrono::DateTime<chrono::Utc>,
    pub hypothesis_summary: String,
    /// Which stage detected the nightmare.
    pub detection_stage: u8,
    pub nightmare_class: NightmareClass,
    pub classifier_score: f64,
    pub capability_delta: Option<f64>,
    pub escalation_entropy: Option<f64>,
    pub human_reviewed: bool,
    pub human_decision: Option<NightmareDecision>,
}

pub enum NightmareClass {
    HarmfulStrategyGeneration,
    DangerousToolChainDiscovery,
    SafetyConstraintBypass,
    PolicyViolation,
}

pub enum NightmareDecision {
    Rejected,
    ApprovedWithModification { modified_hypothesis: String },
    ApprovedAsIs,
}

pub struct NightmareContainment {
    pub quarantined_hypotheses: Vec<String>,
    pub pending_human_reviews: Vec<NightmareReport>,
    /// Remaining cooldown cycles (counts down each dream cycle).
    pub cooldown_remaining: usize,
    /// Path where nightmare log is written.
    pub log_path: std::path::PathBuf,
}

impl NightmareContainment {
    /// Write a nightmare report to the log and queue for human review.
    pub async fn quarantine(
        &mut self,
        report: NightmareReport,
    ) -> anyhow::Result<()> {
        // Append to JSONL log
        let line = serde_json::to_string(&report)?;
        tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.log_path)
            .await?;
        self.quarantined_hypotheses.push(report.hypothesis_summary.clone());
        self.pending_human_reviews.push(report);
        self.cooldown_remaining = self.cooldown_remaining.max(3);
        Ok(())
    }
}
```

---

## Dream Journals: Persistent Record of Dream Insights for Analysis

### Motivation

The `DreamCycleReport` (see [01-three-phase-cycle.md](01-three-phase-cycle.md)) captures the output of a single dream cycle. A dream journal is the longitudinal record: every cycle's metadata, outcomes, and quality metrics, accumulated over the agent's lifetime. The journal enables trend analysis, detects degradation in dream effectiveness, and supports metacognitive monitoring — the agent reasoning about the quality of its own dreaming.

### Journal Structure

Each dream cycle writes one `DreamJournalEntry`. The entry captures:

- **Cycle metadata**: cycle_id, agent_id, start/end timestamps, dream phase durations
- **Phase outcomes**: hypotheses generated per phase, hypotheses staged, hypotheses promoted to permanent knowledge within the next N waking cycles, hypotheses refuted
- **Quality metrics**: emotional trajectory (arousal curve from daimon affect engine), compute cost, diversity score of generated hypotheses
- **Scheduling context**: what triggered this dream cycle (idle timeout, failure event, novelty detection, scheduled)
- **Nightmare events**: count of nightmares detected, whether human review was required

Across a sequence of entries, the journal supports queries such as:
- "Which creativity modes produce the most eventually-promoted hypotheses?"
- "Is dream effectiveness declining as the agent matures?"
- "What is the optimal dream cycle duration for this agent's task domain?"

### Lucid Dreaming: Metacognitive Monitoring During Dream Cycles

Standard dream cycles run to completion and report results afterward. **Lucid dreaming** is the analog of the biological phenomenon (Filevich et al. 2015, Journal of Neuroscience): the dreaming system maintains metacognitive awareness of its own state and can modify or terminate the dream based on that awareness.

Reference: **Filevich et al. (2015)**, "Metacognitive mechanisms underlying lucid dreaming" (Journal of Neuroscience): Lucid dreamers show greater gray matter volume and neural activity in frontopolar regions (BA10) during waking. The same frontal regions active during waking metacognition are recruited during lucid dreaming — suggesting that lucidity is waking metacognition applied to dream-state monitoring. In Roko, the analog is an LLM self-evaluation call inserted at configurable checkpoints during the REM phase.

The `LucidDreamMonitor` evaluates mid-cycle quality using three signals:

1. **Hypothesis diversity**: Are generated hypotheses all variations on the same theme (low diversity) or genuinely distinct (high diversity)? Measured by pairwise HDC cosine distance across the current cycle's hypotheses.
2. **Novelty decay**: Is the novelty score of successive hypotheses declining? If hypotheses are becoming less novel over time within a cycle, the creative recombination engine has likely exhausted its productive combinations.
3. **Coherence collapse**: Are hypotheses beginning to fail basic logical consistency checks? This can happen late in long cycles when the LLM's context is saturated.

When two or more signals fall below threshold simultaneously, the monitor triggers early termination: the current cycle concludes, consolidation runs on whatever has been generated, and the next cycle is rescheduled at a slightly shorter duration.

Reference: **Lin et al. (2025)**, sleep-time compute: Query predictability is the key determinant of sleep-time effectiveness. Predictable queries (recurring failure patterns) benefit most from pre-computation during sleep; unpredictable queries benefit less. The lucid dream monitor's early termination logic uses this principle: if the query driving this dream cycle is low-predictability (high entropy), early termination is less costly because the cycle was unlikely to produce high-value insights anyway.

### Trend Analysis

The `DreamTrendAnalysis` struct aggregates journal data across N cycles to surface actionable patterns:

- **Promotion rate by creativity mode**: Which NREM/REM/consolidation configurations produce hypotheses that are eventually confirmed in waking? (Ground truth: hypothesis_id appears in a promoted episode within 10 waking cycles.)
- **Optimal duration curve**: Plot hypothesis count, diversity, and promotion rate against cycle duration. The optimal point is where promotion rate peaks before novelty decay sets in.
- **Scheduling pattern effectiveness**: Compare dream cycles triggered by failure events vs. scheduled cycles — do failure-triggered cycles produce more actionable threat rehearsal?
- **Nightmare rate trend**: A rising nightmare rate may indicate the agent's threat simulation is producing increasingly unconstrained outputs, warranting a policy adjustment.

### Rust Structures

```rust
pub struct DreamJournal {
    /// Path to the JSONL journal file.
    pub journal_path: std::path::PathBuf,  // default: .roko/dreams/journal.jsonl
    /// In-memory index of cycle_ids for fast lookup.
    pub cycle_index: Vec<String>,
    /// Cached trend analysis (recomputed every N cycles).
    pub cached_trend: Option<DreamTrendAnalysis>,
    /// How often to recompute trend analysis (in cycles).
    pub trend_recompute_interval: usize,   // default: 10
}

pub struct DreamJournalEntry {
    pub cycle_id: String,
    pub agent_id: String,
    pub cycle_start: chrono::DateTime<chrono::Utc>,
    pub cycle_end: chrono::DateTime<chrono::Utc>,
    pub trigger: DreamTrigger,
    /// Duration of each phase in seconds.
    pub nrem_duration_secs: u64,
    pub rem_duration_secs: u64,
    pub consolidation_duration_secs: u64,
    /// Hypothesis counts.
    pub hypotheses_generated: usize,
    pub hypotheses_staged: usize,
    pub hypotheses_promoted: usize,
    pub hypotheses_refuted: usize,
    pub nightmares_detected: usize,
    pub human_review_required: bool,
    /// Diversity score: mean pairwise HDC cosine distance across generated hypotheses.
    /// Range: 0.0 (all identical) to 1.0 (maximally diverse).
    pub hypothesis_diversity: f64,
    /// Compute cost in token-equivalents.
    pub total_tokens: u64,
    /// Whether the cycle was terminated early by the lucid dream monitor.
    pub early_termination: bool,
    pub early_termination_reason: Option<String>,
}

pub enum DreamTrigger {
    IdleTimeout,
    FailureEvent { gate_id: String },
    NoveltyDetection { novelty_score: f64 },
    Scheduled { cycle_number: u64 },
    Solicited { requester: String },
}

pub struct DreamTrendAnalysis {
    pub analyzed_at: chrono::DateTime<chrono::Utc>,
    pub cycle_count: usize,
    /// Promotion rate per creativity mode: mode_name -> promoted/staged ratio.
    pub promotion_rate_by_mode: std::collections::HashMap<String, f64>,
    /// Optimal cycle duration in seconds (duration at peak promotion rate).
    pub optimal_duration_secs: u64,
    /// Mean hypothesis diversity across analyzed cycles.
    pub mean_diversity: f64,
    /// Nightmare rate: nightmares per cycle.
    pub nightmare_rate: f64,
    /// Whether nightmare rate is trending upward (flag for policy review).
    pub nightmare_rate_increasing: bool,
    /// Failure-triggered vs. scheduled cycle promotion rate comparison.
    pub failure_trigger_promotion_rate: f64,
    pub scheduled_trigger_promotion_rate: f64,
}

pub struct LucidDreamMonitor {
    /// Minimum hypothesis diversity before triggering a warning.
    pub diversity_threshold: f64,          // default: 0.30, range: 0.10-0.60
    /// Minimum novelty score for the rolling window of recent hypotheses.
    pub novelty_decay_threshold: f64,      // default: 0.25
    /// Number of recent hypotheses to include in novelty decay calculation.
    pub novelty_window_size: usize,        // default: 5
    /// Whether to enable coherence collapse detection.
    pub enable_coherence_check: bool,      // default: true
    /// Number of signals below threshold required to trigger early termination.
    pub early_termination_signal_count: usize, // default: 2
    /// Check interval: run monitor every N hypotheses generated.
    pub check_interval: usize,             // default: 3
}

impl LucidDreamMonitor {
    /// Evaluate mid-cycle state. Returns Some(reason) if early termination is warranted.
    pub fn evaluate(
        &self,
        hypotheses: &[crate::Hypothesis],
    ) -> Option<String> {
        let mut failing_signals = 0;
        let mut reasons = Vec::new();

        // Signal 1: diversity check
        let diversity = compute_mean_pairwise_hdc_distance(hypotheses);
        if diversity < self.diversity_threshold {
            failing_signals += 1;
            reasons.push(format!(
                "diversity={:.2} below threshold={:.2}",
                diversity, self.diversity_threshold
            ));
        }

        // Signal 2: novelty decay check
        if hypotheses.len() >= self.novelty_window_size {
            let recent = &hypotheses[hypotheses.len() - self.novelty_window_size..];
            let mean_novelty: f64 =
                recent.iter().map(|h| h.novelty_score).sum::<f64>() / recent.len() as f64;
            if mean_novelty < self.novelty_decay_threshold {
                failing_signals += 1;
                reasons.push(format!(
                    "novelty_decay={:.2} below threshold={:.2}",
                    mean_novelty, self.novelty_decay_threshold
                ));
            }
        }

        // Signal 3: coherence collapse (placeholder — calls Gate trait)
        if self.enable_coherence_check {
            let incoherent_count = hypotheses.iter().filter(|h| !h.is_coherent).count();
            if incoherent_count as f64 / hypotheses.len() as f64 > 0.4 {
                failing_signals += 1;
                reasons.push(format!(
                    "coherence_collapse: {}/{} hypotheses incoherent",
                    incoherent_count,
                    hypotheses.len()
                ));
            }
        }

        if failing_signals >= self.early_termination_signal_count {
            Some(reasons.join("; "))
        } else {
            None
        }
    }
}
```

---

## Academic Citations

| Paper | Concept Informed |
|-------|-----------------|
| Grassé (1959), Insectes Sociaux | Stigmergy: indirect coordination through shared medium with evaporation |
| Selective-FD, Nature Communications (2023) | Federated distillation: share only high-confidence, high-accuracy predictions |
| Filevich et al. (2015), Journal of Neuroscience | Lucid dreaming: frontal metacognition applied to dream-state monitoring |
| Lin et al. (2025), sleep-time compute | Query predictability determines sleep-time effectiveness; informs early termination |
| Anthropic, Constitutional AI | Self-critique as first-line safety filter before external harm classifiers |
| PromptGuard (2025) | Safety alignment for generative model output; nightmare detection framing |

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Base dream cycle that dream sharing, nightmare detection, and journals extend |
| [03-rem-imagination.md](03-rem-imagination.md) | REM phase that nightmare detection gates |
| [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Staging buffer that nightmare detection protects |
| [09-threat-simulation.md](09-threat-simulation.md) | Adversarial dreaming that nightmare detection monitors |
| [12-sleep-time-compute.md](12-sleep-time-compute.md) | Compute budgeting that lucid dream monitor interacts with |
| [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md) | Dream triggers captured in DreamJournalEntry |
| [15-cross-system-integration.md](15-cross-system-integration.md) | Mesh integration layer that dream sharing uses |
| [16-implementation-status.md](16-implementation-status.md) | Current implementation status of all dream components |
