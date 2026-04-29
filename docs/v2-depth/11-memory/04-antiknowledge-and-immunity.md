# AntiKnowledge and Immunity

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How negative knowledge and knowledge defense emerge from Verify Pipelines and Signal Kind economics rather than bespoke immune machinery.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Kind, demurrage, HDC fingerprints), [02-CELL](../../unified/02-CELL.md) (Verify protocol, Score protocol, React protocol, Observe protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Pipeline pattern, Loop pattern, Functor pattern), [06-MEMORY](../../unified/06-MEMORY.md) (Store, demurrage, tiers, dream consolidation)

**Code**: `crates/roko-neuro/src/lib.rs` (KnowledgeKind::AntiKnowledge, refutation_warning), `crates/roko-neuro/src/admission.rs`, `crates/roko-neuro/src/context.rs` (attention priority for AntiKnowledge)

---

## 1. The Problem: Knowing What Is Wrong

Every knowledge system faces a deceptively hard question: what happens when something you know turns out to be false?

Most agent frameworks treat memory as append-only. Add a text chunk to the vector store. If it stops being useful, maybe it drifts below the cosine similarity threshold and stops appearing in results. There is no explicit mechanism for recording that a specific piece of knowledge is *wrong* -- that "moving to async always improves throughput" was tested, measured, and found to be false under CPU-bound workloads. Without that mechanism, three failure modes compound over time:

1. **Regression to disproven beliefs.** An agent that forgets its mistakes will rediscover them. If session #12 proved that approach X fails and session #47 forgot that proof, session #48 will try approach X again, paying the same cost for the same lesson.

2. **Epistemic parasites.** A knowledge entry with high retrieval frequency but negative decision quality -- it *looks* useful by the metrics but *harms* performance when applied. "Always use the most expensive model for safety-critical code" gets retrieved constantly, never explicitly contradicted (the expensive model always works), but wastes budget when cheaper models would suffice. Without a mechanism for challenging high-fitness beliefs, parasites persist indefinitely.

3. **Confirmation cascades.** A wrong entry happens to be in the context when a task succeeds for unrelated reasons. It gets a confidence boost. It appears more often. More unrelated successes boost it further. Within a few cycles, a false belief reaches Consolidated tier and resists correction. This is the knowledge analog of overfitting: the system memorizes a coincidence and treats it as a pattern.

The old design addressed these with three separate subsystems: AntiKnowledge entries with special decay behavior, a three-level immune system (innate/adaptive/active), and SIR epidemiological tracking. Each was a bespoke mechanism with its own data structures, its own lifecycle, and its own configuration surface.

The insight of this redesign: every one of these mechanisms is already a primitive in the unified vocabulary. AntiKnowledge is a Signal Kind with special demurrage parameters. The immune system is a Verify Pipeline applied to knowledge instead of code. Memetic fitness is a Score Cell. SIR tracking is an Observe Cell. And auto-AntiKnowledge from gate failures is a React Cell subscribed to a Bus topic. No new concepts needed. Just wiring.

---

## 2. AntiKnowledge as Signal Kind

AntiKnowledge is not a separate system. It is a Signal with `Kind::AntiKnowledge` whose demurrage parameters are tuned to express one economic fact: **knowing what is wrong is permanently valuable.**

### 2.1 Demurrage Parameters

Every Signal Kind has a flat tax (`r`), an exponential decay rate (`beta`), and an effective lifetime without reinforcement (see [06-MEMORY](../../unified/06-MEMORY.md) S3). AntiKnowledge uses the same rate law as Insight -- `r = 0.01`, `beta = 0.02` -- but with two modifications:

```rust
/// AntiKnowledge demurrage behavior.
///
/// Same rate law as all Signals: dB/dt = -r - beta*B + reinforcement.
/// Two modifications express the economic value of negative knowledge:
///
/// 1. Balance floor: balance never drops below ANTI_KNOWLEDGE_FLOOR.
///    This is NOT a separate mechanism -- it is a parameter of the
///    standard demurrage_tick function. The floor ensures AntiKnowledge
///    Signals remain retrievable even without active use.
///
/// 2. Demurrage rate multiplier: 0.5x on-chain demurrage.
///    Off-chain, the standard rate applies but the floor catches it.
///    On-chain, the halved rate lets AntiKnowledge persist as a public good.
const ANTI_KNOWLEDGE_FLOOR: f64 = 0.30;
const ANTI_KNOWLEDGE_CHAIN_RATE_MULTIPLIER: f64 = 0.5;

pub fn demurrage_tick(
    signal: &mut Signal,
    dt_days: f64,
    config: &DemurrageConfig,
    novelty: f64,
    reinforcement: Option<ReinforceKind>,
) {
    // Standard rate law -- identical for all Kinds
    let r = config.flat_tax_for(signal.kind);
    let beta = config.exp_decay_for(signal.kind);
    let drain = r * dt_days + beta * signal.balance * dt_days;
    signal.balance -= drain;
    signal.demurrage_paid += drain;

    // Apply reinforcement if present
    if let Some(kind) = reinforcement {
        let bonus = config.reinforcement_bonus(kind);
        let effective = bonus * novelty;
        signal.balance = (signal.balance + effective).min(1.0);
    }

    // Floor enforcement -- Kind-specific
    if signal.kind == Kind::AntiKnowledge {
        signal.balance = signal.balance.max(ANTI_KNOWLEDGE_FLOOR);
    }
}
```

The balance floor is the only special case in the demurrage tick. It is not a conditional branch on a separate code path -- it is a `.max()` call on the standard balance computation. The floor value of 0.30 was chosen because:

- **Above retrieval noise floor (0.10--0.20)**: AntiKnowledge Signals always appear in query results when semantically relevant.
- **Below active knowledge (0.50--1.0)**: AntiKnowledge does not dominate retrieval. It warns; it does not shout.
- **Equal to dream hypothesis confidence (0.20--0.30)**: Creates a natural equilibrium where dream-generated hypotheses and old refutations compete on equal footing.

### 2.2 The Refutation Link

An AntiKnowledge Signal carries a `refuted_insight_id` field that links it to the Signal it contradicts. This is not a special relationship type -- it is the standard `source: Vec<SignalRef>` provenance DAG. The refuted Signal is in the sources. The AntiKnowledge Signal's payload contains the refutation evidence.

```rust
/// Create an AntiKnowledge Signal from a refuted Signal.
///
/// The refuted Signal becomes a source in the provenance DAG.
/// The refutation evidence goes in the payload.
pub fn create_anti_knowledge(
    refuted: &Signal,
    evidence: &str,
    author: Author,
) -> Signal {
    Signal {
        id: SignalId::new(),
        content_hash: ContentHash::compute(&format!(
            "anti:{}:{}", refuted.content_hash, evidence
        )),
        kind: Kind::AntiKnowledge,
        payload: serde_json::json!({
            "refuted_id": refuted.id.to_string(),
            "refuted_content": refuted.payload.get("content")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "evidence": evidence,
        }),
        score: Score::default(),
        confidence: 0.6,
        balance: 1.0,
        demurrage_paid: 0.0,
        last_touched_at: Utc::now(),
        tier: Tier::Working, // starts at Working, not Transient
        created_at: Utc::now(),
        source: vec![SignalRef::from(&refuted.id)],
        provenance: Provenance::system("anti-knowledge-creation"),
        hdc_fingerprint: refuted.hdc_fingerprint.clone(),
        author,
        tags: refuted.tags.clone(),
    }
}
```

Three design decisions in this function:

1. **HDC fingerprint copied from the refuted Signal.** The AntiKnowledge entry occupies the same region of HDC space as the thing it contradicts. This means HDC similarity search naturally pairs them: a query that finds the original claim also finds the refutation. No separate cross-referencing index needed.

2. **Starting tier is Working, not Transient.** A refutation backed by evidence deserves more persistence than a raw observation. It has already passed through a verification step (the gate failure or challenge that created it).

3. **Content hash includes the refuted hash.** This makes the AntiKnowledge entry content-addressed relative to its target. You cannot create two identical refutations of the same Signal -- the hash would collide, and Store deduplicates.

### 2.3 Retrieval Integration

When the Memory Cell retrieves Signals for an agent's context, AntiKnowledge entries surface through standard HDC similarity search. No special retrieval path. But the context assembler ([05-AGENT](../../unified/05-AGENT.md) SS12) gives AntiKnowledge entries Critical attention priority, meaning they are never dropped from the context pack due to budget constraints:

```rust
// From roko-neuro/src/context.rs -- attention priority
fn chunk_attention_priority(chunk: &ContextChunk) -> AttentionPriority {
    match &chunk.source {
        ContextSource::KnowledgeEntry { kind, .. }
            if kind.eq_ignore_ascii_case("AntiKnowledge") =>
        {
            AttentionPriority::Critical // never evicted from context pack
        }
        // ...other kinds at lower priority...
    }
}
```

The refutation warning is generated by the existing `refutation_warning()` method on `KnowledgeEntry` (in `roko-neuro/src/lib.rs`). When an agent sees a retrieved Insight, the co-retrieved AntiKnowledge entry produces text like: "Previous insight ke_original_async_insight was wrong because Benchmark showed 15% throughput regression when converting CPU-bound computation from sync to async."

---

## 3. The Immune System as a Verify Pipeline

The old design described a three-level immune system (innate, adaptive, active) as three separate structs with three separate APIs. But every level is doing the same thing: verifying incoming knowledge against quality criteria. That is the Verify protocol. The immune system is a **Pipeline** of three Verify Cells applied to the knowledge Store, not a bespoke subsystem.

### 3.1 The Pipeline

```toml
# Graph: knowledge-immune-pipeline
# Three Verify Cells in a Pipeline pattern: Innate -> Adaptive -> Active.
# Processes every Signal entering the Memory Cell's Store.
#
# Signal flow:
#   candidate -> innate-verify -> adaptive-verify -> active-verify -> admitted
#                   |                  |                  |
#               quarantine         quarantine          audit-report
#
# Feedback loop:
#   active-verify.audit_findings -> adaptive-verify.update_prototypes
#   (Loop pattern: output of stage 3 feeds back to stage 2)

[graph]
id = "knowledge-immune-pipeline"
description = "Three-stage Verify pipeline for knowledge admission"

[[graph.cells]]
id = "innate-verify"
protocol = "Verify"
description = "Fast non-specific checks: bloom filter, confidence cap, anomaly detection"

[[graph.cells]]
id = "adaptive-verify"
protocol = "Verify"
description = "Learned pattern matching against known corruption prototypes"

[[graph.cells]]
id = "active-verify"
protocol = "Verify"
description = "Periodic audit: parasite detection, Price equation, cascade risk"

[[graph.edges]]
from = "innate-verify.out"
to = "adaptive-verify.in"

[[graph.edges]]
from = "adaptive-verify.out"
to = "active-verify.in"

# Feedback: audit findings update adaptive prototypes
[[graph.edges]]
from = "active-verify.findings"
to = "adaptive-verify.prototype_updates"
```

This is the same Pipeline pattern used for code verification (CompileGate -> ClippyGate -> TestGate -> ...). The only difference is what is being verified: code artifacts versus knowledge Signals.

### 3.2 Innate Verify Cell

The first stage runs on every incoming Signal. It is fast, non-specific, and stateless. It implements `verify_pre()` -- a check that runs *before* the Signal enters Store.

```rust
/// Innate immunity: fast, stateless checks on every incoming Signal.
///
/// This is a Verify Cell. It conforms to VerifyProtocol. It runs verify_pre
/// on every candidate Signal before admission to the knowledge Store.
///
/// Three checks, ordered by cost:
///   1. Bloom filter:        O(1), catches known-bad fingerprints
///   2. Confidence cap:      O(1), limits trust in external sources
///   3. Anomaly detection:   O(n), HDC similarity to existing population
pub struct InnateVerifyCell {
    /// Bloom filter of known-bad HDC fingerprints (from past AntiKnowledge).
    /// Uses locality-sensitive hashing on the 10,240-bit vector.
    bad_fingerprint_bloom: BloomFilter<256>,

    /// Maximum confidence for Signals from external sources.
    max_external_confidence: f64,  // default: 0.7

    /// Minimum mean HDC similarity to existing knowledge base.
    /// Below this threshold, the Signal is anomalous (anti-correlated
    /// with everything we know -- suspicious).
    anomaly_sim_floor: f64,  // default: 0.48

    /// Minimum number of distinct source episodes for tier promotion.
    min_source_diversity: usize,  // default: 2
}

#[async_trait]
impl VerifyProtocol for InnateVerifyCell {
    async fn verify_pre(
        &self,
        input: &[Signal],
        _plan: &ActionPlan,
        ctx: &VerifyContext,
    ) -> Result<Verdict> {
        let candidate = &input[0]; // Pipeline passes one Signal at a time

        // Check 1: Bloom filter of known-bad fingerprints
        if self.bad_fingerprint_bloom.might_contain(&candidate.hdc_fingerprint) {
            return Ok(Verdict {
                reward: -1.0,
                hard_pass: false,
                hard_criteria: vec![CriterionResult {
                    criterion: Criterion::Custom {
                        name: "known_bad_fingerprint".into(),
                        description: "Signal matches known-bad HDC signature".into(),
                    },
                    passed: false,
                    score: 0.0,
                    evidence_refs: vec![],
                }],
                soft_criteria: vec![],
                evidence: vec![],
                duration: Duration::ZERO,
                explanation: Some("Quarantined: matches known-bad bloom filter".into()),
            });
        }

        // Check 2: Cap confidence for external sources
        if candidate.provenance.is_external() &&
           candidate.confidence > self.max_external_confidence
        {
            // Soft criterion: reduce confidence, don't reject
            return Ok(Verdict {
                reward: 0.3,
                hard_pass: true,
                hard_criteria: vec![],
                soft_criteria: vec![CriterionResult {
                    criterion: Criterion::Custom {
                        name: "external_confidence_cap".into(),
                        description: format!(
                            "External source confidence capped: {:.2} -> {:.2}",
                            candidate.confidence, self.max_external_confidence
                        ),
                    },
                    passed: false,
                    score: self.max_external_confidence / candidate.confidence,
                    evidence_refs: vec![],
                }],
                evidence: vec![],
                duration: Duration::ZERO,
                explanation: Some("Admitted with reduced confidence".into()),
            });
        }

        // Check 3: Anomaly detection via HDC mean similarity
        let store = &ctx.store;
        let neighbors = store.query_similar(
            &candidate.hdc_fingerprint,
            1.0, // full radius
            50,  // sample 50 neighbors
        ).await?;

        if !neighbors.is_empty() {
            let mean_sim: f64 = neighbors.iter()
                .map(|(_, dist)| 1.0 - *dist as f64) // distance -> similarity
                .sum::<f64>() / neighbors.len() as f64;

            if mean_sim < self.anomaly_sim_floor {
                return Ok(Verdict {
                    reward: -0.5,
                    hard_pass: false,
                    hard_criteria: vec![CriterionResult {
                        criterion: Criterion::Custom {
                            name: "anomaly_detection".into(),
                            description: format!(
                                "Anomalous HDC vector: mean similarity {:.3} < floor {:.3}",
                                mean_sim, self.anomaly_sim_floor
                            ),
                        },
                        passed: false,
                        score: mean_sim,
                        evidence_refs: vec![],
                    }],
                    soft_criteria: vec![],
                    evidence: vec![],
                    duration: Duration::ZERO,
                    explanation: Some(format!(
                        "Quarantined: anomalous vector (mean sim {mean_sim:.3})"
                    )),
                });
            }
        }

        Ok(Verdict::pass("innate_verify", Duration::ZERO))
    }

    async fn verify_post(
        &self, _input: &[Signal], _output: &[Signal], _ctx: &VerifyContext,
    ) -> Result<Verdict> {
        // Innate only runs pre-admission. Post is a no-op pass.
        Ok(Verdict::pass("innate_verify_post", Duration::ZERO))
    }

    async fn verify_stream(
        &self, _partial: &[Signal], _ctx: &VerifyContext,
    ) -> Result<StreamVerdict> {
        Ok(StreamVerdict { continue_execution: true, partial: Verdict::pass("innate_stream", Duration::ZERO) })
    }
}
```

### 3.3 Adaptive Verify Cell

The second stage uses learned patterns. Where the innate stage is stateless, the adaptive stage maintains **corruption prototypes** -- HDC vectors that represent categories of known-bad knowledge, bundled from past AntiKnowledge entries.

```rust
/// Adaptive immunity: learned defenses that improve over time.
///
/// Maintains corruption prototypes per infection vector category.
/// Updated during Dream consolidation (a React Cell on the
/// `dream.consolidation.completed` Bus topic pushes new prototypes).
pub struct AdaptiveVerifyCell {
    /// HDC prototypes per corruption category.
    /// Each prototype is a bundle() of past AntiKnowledge vectors
    /// in that category.
    corruption_prototypes: RwLock<BTreeMap<CorruptionCategory, HdcVector>>,

    /// Per-domain vulnerability scores: how susceptible is each domain
    /// to each corruption category? Learned from historical data.
    domain_vulnerability: RwLock<BTreeMap<String, Vec<(CorruptionCategory, f64)>>>,

    /// Similarity threshold for flagging. Above this, the candidate
    /// resembles a known corruption pattern.
    threat_threshold: f64,  // default: 0.526 (above HDC noise floor)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CorruptionCategory {
    DistillationError,     // LLM extracted wrong pattern from valid data
    ConfirmationCascade,   // coincidental confirmation inflated confidence
    ExternalInjection,     // imported from untrusted source
    ConceptDrift,          // was correct, environment changed
    AdversarialPoisoning,  // deliberately crafted to mislead
}

#[async_trait]
impl VerifyProtocol for AdaptiveVerifyCell {
    async fn verify_pre(
        &self,
        input: &[Signal],
        _plan: &ActionPlan,
        _ctx: &VerifyContext,
    ) -> Result<Verdict> {
        let candidate = &input[0];
        let prototypes = self.corruption_prototypes.read().await;

        let mut max_threat: f64 = 0.0;
        let mut max_category: Option<CorruptionCategory> = None;

        for (category, prototype) in prototypes.iter() {
            let sim = candidate.hdc_fingerprint.similarity(prototype) as f64;
            if sim > self.threat_threshold && sim > max_threat {
                max_threat = sim;
                max_category = Some(*category);
            }
        }

        match max_category {
            Some(category) => Ok(Verdict {
                reward: -max_threat,
                hard_pass: max_threat < 0.9, // reject only at very high similarity
                hard_criteria: if max_threat >= 0.9 {
                    vec![CriterionResult {
                        criterion: Criterion::Custom {
                            name: "adaptive_threat_reject".into(),
                            description: format!(
                                "Signal matches {:?} prototype at {:.3} similarity (reject threshold 0.9)",
                                category, max_threat
                            ),
                        },
                        passed: false,
                        score: 1.0 - max_threat,
                        evidence_refs: vec![],
                    }]
                } else {
                    vec![]
                },
                soft_criteria: vec![CriterionResult {
                    criterion: Criterion::Custom {
                        name: "adaptive_threat_score".into(),
                        description: format!(
                            "Resembles {:?} pattern (similarity {:.3})",
                            category, max_threat
                        ),
                    },
                    passed: max_threat < self.threat_threshold,
                    score: 1.0 - max_threat,
                    evidence_refs: vec![],
                }],
                evidence: vec![],
                duration: Duration::ZERO,
                explanation: Some(format!(
                    "Threat detected: {:?} at similarity {:.3}",
                    category, max_threat
                )),
            }),
            None => Ok(Verdict::pass("adaptive_verify", Duration::ZERO)),
        }
    }

    async fn verify_post(
        &self, _input: &[Signal], _output: &[Signal], _ctx: &VerifyContext,
    ) -> Result<Verdict> {
        Ok(Verdict::pass("adaptive_verify_post", Duration::ZERO))
    }

    async fn verify_stream(
        &self, _partial: &[Signal], _ctx: &VerifyContext,
    ) -> Result<StreamVerdict> {
        Ok(StreamVerdict { continue_execution: true, partial: Verdict::pass("adaptive_stream", Duration::ZERO) })
    }
}

impl AdaptiveVerifyCell {
    /// Learn from new AntiKnowledge entries.
    /// Called by a React Cell subscribed to `knowledge.anti.created` topic.
    pub async fn update_prototypes(&self, anti_signals: &[Signal]) {
        let mut prototypes = self.corruption_prototypes.write().await;
        for signal in anti_signals {
            // Classify the corruption category from the signal payload
            let category = classify_corruption(&signal.payload);
            prototypes.entry(category)
                .and_modify(|proto| {
                    // Bundle the new fingerprint into the existing prototype.
                    // This makes the prototype increasingly representative
                    // of the corruption category over time.
                    *proto = HdcVector::bundle(&[proto, &signal.hdc_fingerprint]);
                })
                .or_insert_with(|| signal.hdc_fingerprint.clone());
        }
    }
}
```

The adaptive cell learns. Its corruption prototypes are HDC bundles that grow more representative with each new AntiKnowledge entry. This is the same HDC algebra used everywhere else in the system -- bundle() for aggregation, similarity() for comparison. No special learning algorithm. Just vectors.

### 3.4 Active Verify Cell

The third stage is periodic, not per-Signal. It runs during Dream consolidation and produces an audit report. It is a `verify_post()` call on the entire knowledge population.

```rust
/// Active immunity: periodic audit of the entire knowledge Store.
///
/// Runs during Dream consolidation. Detects epistemic parasites,
/// monitors Price equation health, and flags confirmation cascade risk.
///
/// Unlike innate and adaptive (which run verify_pre on each Signal),
/// active runs verify_post on the whole population at once.
pub struct ActiveVerifyCell {
    /// Maximum parasite ratio before triggering remediation.
    max_parasite_ratio: f64,  // default: 0.05 (5%)

    /// Minimum Price equation selection component for healthy knowledge base.
    min_selection_component: f64,  // default: 0.0

    /// Maximum ratio of single-source high-confidence entries.
    max_single_source_ratio: f64,  // default: 0.10 (10%)
}

#[async_trait]
impl VerifyProtocol for ActiveVerifyCell {
    async fn verify_pre(
        &self, _input: &[Signal], _plan: &ActionPlan, _ctx: &VerifyContext,
    ) -> Result<Verdict> {
        // Active does not screen individual entries.
        Ok(Verdict::pass("active_verify_pre", Duration::ZERO))
    }

    async fn verify_post(
        &self,
        _input: &[Signal],   // empty for population audit
        output: &[Signal],    // the entire knowledge population
        ctx: &VerifyContext,
    ) -> Result<Verdict> {
        let start = Instant::now();

        // 1. Detect epistemic parasites
        let parasites = detect_parasites(output, ctx).await;
        let parasite_ratio = parasites.len() as f64
            / output.len().max(1) as f64;

        // 2. Price equation diagnostics
        let (selection, transmission) = price_equation(output, ctx).await;

        // 3. Confirmation cascade risk
        let cascade_risks: Vec<&Signal> = output.iter()
            .filter(|s| {
                s.confidence > 0.7
                    && s.source.len() <= 1
                    && s.kind != Kind::AntiKnowledge
            })
            .collect();
        let cascade_ratio = cascade_risks.len() as f64
            / output.len().max(1) as f64;

        // 4. Build verdict
        let health_score = 1.0_f64
            .min(1.0 - parasite_ratio * 5.0)
            .min(1.0 - cascade_ratio * 2.0)
            .min(if selection < 0.0 { 0.3 } else { 1.0 })
            .max(0.0);

        let hard_pass = parasite_ratio <= self.max_parasite_ratio
            && selection >= self.min_selection_component;

        let mut evidence = Vec::new();
        evidence.push(Evidence {
            kind: EvidenceKind::Custom {
                name: "price_equation".into(),
            },
            content: serde_json::json!({
                "selection": selection,
                "transmission": transmission,
            }),
            collected_at: Utc::now(),
            collector: CellRef::from("active-verify"),
        });

        Ok(Verdict {
            reward: health_score,
            hard_pass,
            hard_criteria: vec![CriterionResult {
                criterion: Criterion::Custom {
                    name: "knowledge_health".into(),
                    description: format!(
                        "Parasite ratio {:.1}% (max {}%), \
                         Price selection {:.3} (min {:.3})",
                        parasite_ratio * 100.0,
                        self.max_parasite_ratio * 100.0,
                        selection,
                        self.min_selection_component,
                    ),
                },
                passed: hard_pass,
                score: health_score,
                evidence_refs: vec![],
            }],
            soft_criteria: vec![CriterionResult {
                criterion: Criterion::Custom {
                    name: "cascade_risk".into(),
                    description: format!(
                        "{} entries ({:.0}%) at cascade risk",
                        cascade_risks.len(),
                        cascade_ratio * 100.0,
                    ),
                },
                passed: cascade_ratio <= self.max_single_source_ratio,
                score: 1.0 - cascade_ratio,
                evidence_refs: vec![],
            }],
            evidence,
            duration: start.elapsed(),
            explanation: Some(format!(
                "Knowledge health: {:.0}%. Parasites: {}. \
                 Selection: {:.3}. Cascade risk: {:.0}%.",
                health_score * 100.0,
                parasites.len(),
                selection,
                cascade_ratio * 100.0,
            )),
        })
    }

    async fn verify_stream(
        &self, _partial: &[Signal], _ctx: &VerifyContext,
    ) -> Result<StreamVerdict> {
        Ok(StreamVerdict { continue_execution: true, partial: Verdict::pass("active_stream", Duration::ZERO) })
    }
}
```

### 3.5 The Pipeline Is the Same Pipeline

This is the central claim: the knowledge immune pipeline and the code verification pipeline are the **same Pipeline pattern** with different Cells plugged in.

| | Code Verification Pipeline | Knowledge Immune Pipeline |
|---|---|---|
| **Cell slot 0** | DiffGate (pre-filter) | InnateVerifyCell (bloom + cap + anomaly) |
| **Cell slot 1** | CompileGate | AdaptiveVerifyCell (learned prototypes) |
| **Cell slot 2** | ClippyGate | ActiveVerifyCell (population audit) |
| **Cell slot 3** | TestGate | *(not needed)* |
| **Input type** | Code artifact (Diff, File) | Knowledge Signal (Insight, Heuristic, ...) |
| **Output type** | Verdict | Verdict |
| **Feedback target** | Adaptive gate thresholds | Adaptive corruption prototypes |
| **Bus topic** | `gate.verdict.emitted` | `knowledge.immune.verdict` |

Both pipelines:
- Consist of Verify Cells wired in a linear sequence.
- Produce Verdicts with hard/soft criteria and evidence.
- Feed verdicts back into learning systems.
- Publish verdicts as Pulses on Bus.
- Support adaptive thresholds that evolve from experience.

The reuse is not superficial. The same `Verdict` struct, the same `CriterionResult` type, the same evidence collection mechanism, and the same Bus topics flow through both pipelines. A dashboard that displays gate verdicts for code verification works unchanged for knowledge immune verdicts.

---

## 4. Memetic Fitness as a Score Cell

The old design computed fitness as `W(E) = fidelity * fecundity * longevity` using ad hoc functions. In the unified vocabulary, fitness is a **Score Cell** -- it implements the Score protocol and rates Signals along the standard 5-axis score.

```rust
/// Memetic fitness scorer: rates knowledge Signals by their
/// evolutionary fitness in the knowledge population.
///
/// This is a Score Cell. It conforms to ScoreProtocol.
/// Fitness maps onto the standard 5-axis Score as follows:
///
///   relevance  -> fecundity (how often retrieved)
///   quality    -> fidelity (how accurately preserved through retrieval)
///   confidence -> calibration score (for Heuristics) or confirmation count
///   novelty    -> inverse of redundancy with existing knowledge
///   utility    -> decision quality (outcome delta when used vs not used)
pub struct MemeticFitnessScorer;

#[async_trait]
impl ScoreProtocol for MemeticFitnessScorer {
    async fn score(
        &self,
        signal: &Signal,
        context: &ScoreContext,
    ) -> Result<Score> {
        // Fecundity: retrieval frequency, normalized
        let fecundity = signal.demurrage_paid.min(100.0) / 100.0;

        // Fidelity: how stable is the content across retrievals?
        // Measured by content hash stability (has the signal been
        // distilled/rewritten? lower fidelity if so)
        let fidelity = if signal.source.len() <= 1 { 1.0 } else { 0.8 };

        // Longevity: age normalized by tier expectation
        let age_days = (Utc::now() - signal.created_at)
            .num_days().max(1) as f64;
        let expected_lifetime = match signal.tier {
            Tier::Transient => 7.0,
            Tier::Working => 30.0,
            Tier::Consolidated => 90.0,
            Tier::Persistent => 365.0,
        };
        let longevity = (age_days / expected_lifetime).min(1.0);

        // Classic fitness
        let fitness = fidelity * fecundity * longevity;

        Ok(Score {
            relevance: fecundity,
            quality: fidelity,
            confidence: signal.confidence,
            novelty: 1.0 - context.neighbors.iter()
                .map(|n| signal.hdc_fingerprint.similarity(
                    &n.hdc_fingerprint
                ) as f64)
                .fold(0.0_f64, f64::max),
            utility: fitness,
        })
    }
}
```

### 4.1 Parasite Detection via Score

An epistemic parasite is a Signal with high fitness (high utility in the Score) but negative decision quality. The ActiveVerifyCell detects parasites by scoring the population and flagging entries where:

- `score.utility > 0.3` (high fitness -- frequently used, long-lived)
- decision quality delta < -0.1 (outcomes are worse when this Signal is in context)

```rust
/// Detect epistemic parasites in the knowledge population.
///
/// A parasite has high evolutionary fitness (it persists and replicates)
/// but negative actual value (decisions are worse when it is used).
async fn detect_parasites(
    population: &[Signal],
    ctx: &VerifyContext,
) -> Vec<SignalRef> {
    let scorer = MemeticFitnessScorer;
    let score_ctx = ScoreContext {
        neighbors: vec![],
        query: None,
        attention_focus: None,
    };

    let mut parasites = Vec::new();
    for signal in population {
        if signal.kind == Kind::AntiKnowledge {
            continue; // AntiKnowledge is immune to parasite detection
        }

        let score = scorer.score(signal, &score_ctx).await.unwrap_or_default();
        let decision_quality = compute_decision_quality(signal, ctx).await;

        if score.utility > 0.3 && decision_quality < -0.1 {
            parasites.push(SignalRef::from(&signal.id));
        }
    }
    parasites
}

/// Decision quality: mean outcome when this Signal was in context
/// minus mean outcome when it was not.
///
/// Positive = Signal helps. Negative = Signal harms.
async fn compute_decision_quality(
    signal: &Signal,
    ctx: &VerifyContext,
) -> f64 {
    // Query episodes where this Signal was in the context pack
    let episodes_with = ctx.store.query(StoreQuery {
        kind_filter: Some(Kind::Episode),
        tag_filter: None,
        content_contains: Some(signal.id.to_string()),
        ..Default::default()
    }).await.unwrap_or_default();

    if episodes_with.is_empty() { return 0.0; }

    let mean_with: f64 = episodes_with.iter()
        .filter_map(|ep| ep.score.utility.into())
        .sum::<f64>() / episodes_with.len() as f64;

    // Compare to population baseline
    let baseline = ctx.store.query(StoreQuery {
        kind_filter: Some(Kind::Episode),
        ..Default::default()
    }).await.unwrap_or_default();

    if baseline.is_empty() { return 0.0; }

    let mean_baseline: f64 = baseline.iter()
        .filter_map(|ep| ep.score.utility.into())
        .sum::<f64>() / baseline.len() as f64;

    mean_with - mean_baseline
}
```

### 4.2 Price Equation as Population Diagnostic

The Price equation decomposes the change in mean fitness into a selection component and a transmission component:

```
delta(mean_fitness) = Cov(fitness, frequency) + E(delta_fitness)
```

This is computed by the ActiveVerifyCell during audit and published as Evidence in the Verdict. It requires no special machinery -- it is a statistical computation over Score values that already exist on every Signal.

```rust
/// Price equation diagnostics over the knowledge population.
///
/// Returns (selection, transmission):
///   selection > 0   -> healthy: good entries used more
///   selection < 0   -> unhealthy: bad entries used more
///   transmission > 0 -> distillation is improving entries
///   transmission < 0 -> distillation is degrading entries
async fn price_equation(
    population: &[Signal],
    ctx: &VerifyContext,
) -> (f64, f64) {
    if population.is_empty() {
        return (0.0, 0.0);
    }

    let n = population.len() as f64;

    // Fitness: utility score. Frequency: retrieval rate (demurrage_paid as proxy).
    // Delta fitness: change in utility since last audit.
    let entries: Vec<(f64, f64, f64)> = population.iter()
        .map(|s| {
            let fitness = s.score.utility;
            let frequency = s.demurrage_paid.min(100.0) / 100.0;
            let delta = 0.0; // would come from comparison with previous audit
            (fitness, frequency, delta)
        })
        .collect();

    let mean_fitness: f64 = entries.iter().map(|(f, _, _)| f).sum::<f64>() / n;
    let mean_freq: f64 = entries.iter().map(|(_, r, _)| r).sum::<f64>() / n;
    let mean_delta: f64 = entries.iter().map(|(_, _, d)| d).sum::<f64>() / n;

    let e_product: f64 = entries.iter()
        .map(|(f, r, _)| f * r)
        .sum::<f64>() / n;

    let selection = e_product - mean_fitness * mean_freq;
    let transmission = mean_delta;

    (selection, transmission)
}
```

---

## 5. SIR Tracking as an Observe Cell (Lens)

The SIR (Susceptible-Infected-Recovered) model treats the knowledge population as an epidemiological system. This is a read-only observation -- it does not modify knowledge, it observes its health. That makes it an **Observe Cell** (Lens).

```rust
/// SIR epidemiological lens over the knowledge Store.
///
/// Categorizes the knowledge population into:
///   S = Susceptible: could become corrupted
///   I = Infected: currently incorrect (has AntiKnowledge challenger)
///   R = Recovered: protected by AntiKnowledge challenges
///
/// Publishes SIR metrics as Observation Signals on Bus topic
/// `knowledge.sir.snapshot`.
pub struct SirLens;

/// SIR snapshot at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SirSnapshot {
    pub susceptible: usize,
    pub infected: usize,
    pub recovered: usize,
    pub total: usize,

    /// Basic reproduction number: R0 = beta / gamma.
    /// R0 < 1 means the immune system is containing infection.
    /// R0 > 1 means bad knowledge spreads faster than detection.
    pub r_naught: f64,

    /// Confirmation cascade rate (beta): how quickly bad entries
    /// get coincidentally confirmed.
    pub beta: f64,

    /// Detection rate (gamma): how quickly gate failures expose
    /// bad entries.
    pub gamma: f64,

    pub timestamp: DateTime<Utc>,
}

#[async_trait]
impl ObserveProtocol for SirLens {
    async fn observe(&self, ctx: &ObserveContext) -> Result<Vec<Signal>> {
        let all_signals = ctx.store.query(StoreQuery {
            kind_filter: None,
            ..Default::default()
        }).await?;

        let anti_knowledge: Vec<&Signal> = all_signals.iter()
            .filter(|s| s.kind == Kind::AntiKnowledge)
            .collect();

        // Signals that have an AntiKnowledge challenger = Infected
        let infected_ids: HashSet<SignalId> = anti_knowledge.iter()
            .flat_map(|anti| anti.source.iter())
            .map(|r| r.id.clone())
            .collect();

        // Signals protected by AntiKnowledge (the AntiKnowledge
        // was created AND the original was demoted) = Recovered
        let recovered_ids: HashSet<SignalId> = all_signals.iter()
            .filter(|s| {
                infected_ids.contains(&s.id) &&
                s.tier <= Tier::Working // demoted = recovering
            })
            .map(|s| s.id.clone())
            .collect();

        let knowledge_signals: Vec<&Signal> = all_signals.iter()
            .filter(|s| matches!(s.kind,
                Kind::Insight | Kind::Heuristic | Kind::CausalLink |
                Kind::StrategyFragment | Kind::AntiKnowledge
            ))
            .collect();

        let total = knowledge_signals.len();
        let infected = infected_ids.len();
        let recovered = recovered_ids.len();
        let susceptible = total.saturating_sub(infected + recovered);

        // Estimate beta and gamma from recent episodes
        let recent_episodes = ctx.store.query(StoreQuery {
            kind_filter: Some(Kind::Episode),
            time_range: Some(TimeRange::last_days(7)),
            ..Default::default()
        }).await.unwrap_or_default();

        // beta = fraction of episodes where an undetected bad entry
        //        was coincidentally confirmed
        // gamma = fraction of episodes where a gate failure exposed
        //         a bad entry
        let (beta, gamma) = estimate_sir_rates(&recent_episodes, &infected_ids);

        let r_naught = if gamma > 0.0 { beta / gamma } else { f64::INFINITY };

        let snapshot = SirSnapshot {
            susceptible,
            infected,
            recovered,
            total,
            r_naught,
            beta,
            gamma,
            timestamp: Utc::now(),
        };

        // Package as an Observation Signal
        let obs_signal = Signal {
            id: SignalId::new(),
            content_hash: ContentHash::compute(
                &serde_json::to_string(&snapshot)?
            ),
            kind: Kind::Observation,
            payload: serde_json::to_value(&snapshot)?,
            score: Score::default(),
            confidence: 1.0,
            balance: 0.5,
            demurrage_paid: 0.0,
            last_touched_at: Utc::now(),
            tier: Tier::Transient,
            created_at: Utc::now(),
            source: vec![],
            provenance: Provenance::system("sir-lens"),
            hdc_fingerprint: HdcVector::zero(),
            author: Author::System,
            tags: vec!["sir".into(), "knowledge-health".into()],
        };

        // Publish on Bus as a real-time metric
        ctx.bus.publish(Pulse {
            seq: 0,
            topic: Topic::from("knowledge.sir.snapshot"),
            kind: Kind::Observation,
            body: serde_json::to_value(&snapshot)?,
            emitted_at_ms: Utc::now().timestamp_millis(),
            source: PulseSource::Cell(CellRef::from("sir-lens")),
            lineage_hint: Some(obs_signal.content_hash.clone()),
            trace_id: None,
        }).await;

        Ok(vec![obs_signal])
    }
}
```

The SIR lens produces real-time Bus metrics. A dashboard subscribed to `knowledge.sir.snapshot` can display R0 as a live gauge. If R0 exceeds 1.0, it means bad knowledge is spreading faster than the immune pipeline can catch it -- an actionable alert that can trigger more aggressive innate screening (lowering thresholds) or an emergency Dream consolidation cycle.

---

## 6. Auto-AntiKnowledge as a React Cell

When a Verify Cell (gate) fails and the failure can be attributed to retrieved knowledge, the system should automatically create an AntiKnowledge Signal. The old design wired this as a function call in the orchestrator. The unified design makes it a **React Cell** subscribed to the `gate.verdict.emitted` Bus topic.

```rust
/// Auto-AntiKnowledge generator: React Cell that creates AntiKnowledge
/// Signals from gate failures.
///
/// Subscribed to `gate.verdict.emitted` Bus topic. When a gate fails
/// and the context pack included knowledge Signals, generates an
/// AntiKnowledge Signal linking the failure to the retrieved knowledge.
pub struct AutoAntiKnowledgeReactor {
    /// Minimum number of failures before creating AntiKnowledge.
    /// A single failure might be flaky; repeated failures are signal.
    failure_count_threshold: u32,  // default: 1

    /// Track failure counts per (knowledge_signal_id, gate_name) pair.
    failure_counts: RwLock<BTreeMap<(SignalId, String), u32>>,
}

#[async_trait]
impl ReactProtocol for AutoAntiKnowledgeReactor {
    fn subscription(&self) -> TopicFilter {
        TopicFilter::exact("gate.verdict.emitted")
    }

    async fn react(
        &self,
        pulses: &[Pulse],
        ctx: &ReactContext,
    ) -> Result<ReactOutput> {
        let mut new_signals = Vec::new();
        let mut new_pulses = Vec::new();

        for pulse in pulses {
            // Only react to failures
            let verdict: Verdict = serde_json::from_value(
                pulse.body.clone()
            )?;
            if verdict.hard_pass {
                continue; // gate passed, nothing to do
            }

            // Extract context pack Signal IDs from the verdict evidence
            let context_signal_ids = extract_context_signals(&verdict);
            if context_signal_ids.is_empty() {
                continue; // no knowledge in context, can't attribute
            }

            // Find the most-relevant knowledge Signal
            let gate_name = verdict.hard_criteria.first()
                .map(|c| match &c.criterion {
                    Criterion::Custom { name, .. } => name.clone(),
                    other => format!("{:?}", other),
                })
                .unwrap_or_default();

            for signal_id in &context_signal_ids {
                let key = (signal_id.clone(), gate_name.clone());
                let mut counts = self.failure_counts.write().await;
                let count = counts.entry(key).or_insert(0);
                *count += 1;

                if *count >= self.failure_count_threshold {
                    // Retrieve the original Signal
                    if let Some(original) = ctx.store.get(signal_id).await? {
                        // Confidence increases with failure count:
                        // 1 failure -> 0.5, 2 -> 0.6, 3+ -> 0.7
                        let confidence = match *count {
                            1 => 0.5,
                            2 => 0.6,
                            _ => 0.7,
                        };

                        let failure_reason = verdict.explanation
                            .clone()
                            .unwrap_or_else(|| "gate failure".into());

                        let anti = Signal {
                            id: SignalId::new(),
                            content_hash: ContentHash::compute(
                                &format!("anti:{}:{}", original.content_hash, failure_reason)
                            ),
                            kind: Kind::AntiKnowledge,
                            payload: serde_json::json!({
                                "refuted_id": original.id.to_string(),
                                "evidence": failure_reason,
                                "failure_count": *count,
                                "gate": gate_name,
                            }),
                            score: Score::default(),
                            confidence,
                            balance: 1.0,
                            tier: Tier::Working,
                            created_at: Utc::now(),
                            last_touched_at: Utc::now(),
                            demurrage_paid: 0.0,
                            source: vec![SignalRef::from(&original.id)],
                            provenance: Provenance::system("auto-anti-knowledge"),
                            hdc_fingerprint: original.hdc_fingerprint.clone(),
                            author: Author::System,
                            tags: original.tags.clone(),
                        };

                        // The new AntiKnowledge Signal goes through the
                        // immune pipeline itself before being admitted
                        new_signals.push(anti);

                        // Notify on Bus
                        new_pulses.push(Pulse {
                            seq: 0,
                            topic: Topic::from("knowledge.anti.created"),
                            kind: Kind::AntiKnowledge,
                            body: serde_json::json!({
                                "refuted_id": original.id.to_string(),
                                "failure_count": *count,
                            }),
                            emitted_at_ms: Utc::now().timestamp_millis(),
                            source: PulseSource::Cell(CellRef::from(
                                "auto-anti-knowledge-reactor"
                            )),
                            lineage_hint: None,
                            trace_id: None,
                        });
                    }
                }
            }
        }

        Ok(ReactOutput {
            pulses: new_pulses,
            signals: new_signals,
        })
    }
}
```

The auto-AntiKnowledge reactor is a React Cell. It subscribes to `gate.verdict.emitted` (the same topic that feeds adaptive gate thresholds, efficiency tracking, and episode logging). It emits new Signals (the AntiKnowledge entries) and new Pulses (notifications on `knowledge.anti.created`, which the AdaptiveVerifyCell subscribes to for prototype updates).

This creates a closed feedback loop: gate failure -> React Cell creates AntiKnowledge -> AntiKnowledge enters immune pipeline -> AdaptiveVerifyCell updates prototypes -> future similar Signals are flagged earlier. The loop closes through standard Bus subscription, not through any ad hoc wiring.

---

## 7. AntiKnowledge as Pre-Admission Functor

The task description asks: "What if AntiKnowledge was a Functor that pre-emptively filtered incoming Signals before they hit Store?" This is the right question, and it has a concrete answer.

In the unified vocabulary, a **Functor** is a cross-cut node wired to every node in a subgraph via FanOut/FanIn. It transforms Signals before or after a target Cell without changing the Graph's topology (see [03-GRAPH](../../unified/03-GRAPH.md) S7). An AntiKnowledge Functor wraps the Memory Cell's `put()` method: every Signal that would enter Store first passes through the functor.

```rust
/// AntiKnowledge admission functor.
///
/// Wraps a Store Cell's put() operation. Before any Signal is
/// persisted, this functor compares its HDC fingerprint against
/// all existing AntiKnowledge Signals and applies repulsion.
///
/// This is the Functor pattern: wired as a FanOut -> [this Cell]
/// -> FanIn around the Store Cell in the Memory Graph.
pub struct AntiKnowledgeFunctor {
    /// Repulsion thresholds (graduated response)
    warn_threshold: f64,     // 0.50 -- log warning
    discount_threshold: f64, // 0.70 -- halve initial balance
    reject_threshold: f64,   // 0.90 -- reject outright
    discount_factor: f64,    // 0.50 -- balance multiplier at discount
}

impl AntiKnowledgeFunctor {
    /// Apply repulsion to a candidate Signal.
    ///
    /// Returns the modified Signal (possibly with reduced balance)
    /// or None if the Signal should be rejected.
    pub async fn filter(
        &self,
        candidate: &mut Signal,
        store: &dyn Store,
    ) -> FilterResult {
        // Query existing AntiKnowledge Signals
        let anti_signals = store.query(StoreQuery {
            kind_filter: Some(Kind::AntiKnowledge),
            ..Default::default()
        }).await.unwrap_or_default();

        let mut max_similarity: f64 = 0.0;
        let mut blocking_anti: Option<SignalId> = None;

        for anti in &anti_signals {
            let sim = candidate.hdc_fingerprint
                .similarity(&anti.hdc_fingerprint) as f64;
            if sim > max_similarity {
                max_similarity = sim;
                blocking_anti = Some(anti.id.clone());
            }
        }

        if max_similarity >= self.reject_threshold {
            FilterResult::Reject {
                reason: format!(
                    "Similarity {:.3} to AntiKnowledge {:?} exceeds reject threshold {:.2}",
                    max_similarity,
                    blocking_anti,
                    self.reject_threshold,
                ),
            }
        } else if max_similarity >= self.discount_threshold {
            candidate.balance *= self.discount_factor;
            FilterResult::DiscountedAdmit {
                original_balance: candidate.balance / self.discount_factor,
                new_balance: candidate.balance,
                reason: format!(
                    "Balance halved: similarity {:.3} to AntiKnowledge {:?}",
                    max_similarity,
                    blocking_anti,
                ),
            }
        } else if max_similarity >= self.warn_threshold {
            FilterResult::WarnAdmit {
                reason: format!(
                    "Warning: resembles AntiKnowledge {:?} (similarity {:.3})",
                    blocking_anti,
                    max_similarity,
                ),
            }
        } else {
            FilterResult::Admit
        }
    }
}

pub enum FilterResult {
    Admit,
    WarnAdmit { reason: String },
    DiscountedAdmit { original_balance: f64, new_balance: f64, reason: String },
    Reject { reason: String },
}
```

The Functor and the Verify Pipeline are complementary, not redundant:

- The **Verify Pipeline** (S3) runs when knowledge is formally ingested -- batch imports, dream consolidation outputs, mesh sync. It is thorough and multi-stage.
- The **Functor** (this section) runs on every `put()` call -- including quick single-Signal writes from gate verdicts, agent observations, and research. It is fast and focused (HDC comparison only).

Both use the same AntiKnowledge Signals. Neither requires the other to function. Together, they provide defense in depth.

---

## 8. Confirmation Cascade Prevention

The confirmation cascade is the most dangerous failure mode in knowledge systems. It is worth examining how the unified primitives prevent it without any bespoke mechanism.

### The Cascade Mechanism

```
1. Bad Signal B enters Store at Transient tier (balance 1.0, confidence 0.4)
2. Agent retrieves B alongside 20 other Signals for task T1
3. T1 succeeds (for reasons unrelated to B)
4. B gets reinforcement: Retrieved (+0.05) + GatePassed (+0.15) = +0.20
5. B's balance is now 1.10 (capped at 1.0). Promoted to Working tier.
6. B appears more often in queries (higher balance = higher retrieval rank)
7. B gets more reinforcement from more unrelated successes
8. B reaches Consolidated tier with high confidence
9. B is now extremely difficult to demote
```

### Prevention Through Existing Primitives

Three unified mechanisms break the cascade without any cascade-specific code:

**1. Novelty-weighted reinforcement** (from [06-MEMORY](../../unified/06-MEMORY.md) S3):

```
novelty = 1 / (1 + ln(retrieval_count))
effective_bonus = base_bonus * novelty
```

The first retrieval gives full reinforcement. The 10th gives ~0.30x. The 100th gives ~0.18x. A Signal cannot game its way to high balance through retrieval frequency alone. Diminishing returns prevent the cascade's positive feedback loop from accelerating.

**2. Source diversity requirement for tier promotion** (from [06-MEMORY](../../unified/06-MEMORY.md) S4):

Working -> Consolidated requires 5+ independent confirmations from different Agents or contexts. A Signal confirmed only by coincidental co-occurrence with unrelated successes will not have diverse sources. The InnateVerifyCell's `min_source_diversity` check (default: 2) adds a hard gate on promotion.

**3. ActiveVerifyCell cascade risk detection** (from S3.4 above):

The periodic audit explicitly flags high-confidence, single-source Signals as cascade risks. These are surfaced in the Verdict evidence for human or automated review.

Together: novelty weighting slows the cascade, source diversity blocks promotion, and active audit catches what slipped through. All three are standard primitives applied to the knowledge domain.

---

## 9. Wiring It All Together

Here is the complete Graph that wires AntiKnowledge and immunity into the Memory system. Every Cell is a standard unified primitive. Every edge is a standard Bus subscription or Pipeline wire.

```
                       Bus: gate.verdict.emitted
                              |
                    +---------v-----------+
                    | AutoAntiKnowledge   |
                    | ReactCell           |
                    +----+-------+--------+
                         |       |
          Signal         |       |  Pulse: knowledge.anti.created
          (new anti)     |       |
                         v       v
                   +-----+------+---------+
                   | AntiKnowledge        |
                   | Functor              |   <-- wraps every put()
                   +----------+-----------+
                              |
            Admitted Signal   |
                              v
              +---------------+----------------+
              |  Knowledge Immune Pipeline     |
              |  (Verify Pipeline pattern)     |
              |                                |
              |  +----------+   +-----------+  |
              |  | Innate   +-->| Adaptive  |  |
              |  | Verify   |   | Verify    |  |
              |  +----------+   +-----+-----+  |
              |                       |         |
              |                 +-----v-----+   |
              |                 | Active    |   |
              |                 | Verify    |   |
              |                 +-----+-----+   |
              +---------------+-------+---------+
                              |       |
                   Admitted   |       |  Quarantined
                              v       v
                        +-----------+----------+
                        | Memory Cell (Store)  |
                        +-----------+----------+
                                    |
                              Bus: knowledge.immune.verdict
                                    |
                    +---------------v--------------+
                    | SirLens (ObserveCell)        |
                    | reads population, publishes  |
                    | SIR metrics on Bus           |
                    +------------------------------+
                                    |
                        Bus: knowledge.sir.snapshot
                                    |
                    +---------------v--------------+
                    | Dashboard / Alerts           |
                    | (React Cell on SIR topic)    |
                    +------------------------------+

Feedback loop (closed):
  Active Verify findings --> Adaptive Verify prototype updates
  (via Bus: knowledge.audit.findings)
```

---

## 10. What Changed From the Old Design

| Old Design | Unified Redesign | Why |
|---|---|---|
| AntiKnowledge with bespoke confidence floor and GC exemption | Standard Signal Kind with `balance.max(FLOOR)` in demurrage_tick | One mechanism, one code path, one configuration surface |
| Three-level immune system (InnateImmunity, AdaptiveImmunity, KnowledgeHealthAudit) | Three Verify Cells in a Pipeline Graph | Same pattern as code verification. Same Verdict type. Same dashboard. |
| SIR model as standalone diagnostics | Observe Cell (Lens) publishing on Bus | Real-time metrics, standard Bus subscription, standard dashboard integration |
| Auto-AntiKnowledge as orchestrator function call | React Cell on `gate.verdict.emitted` | Decoupled from orchestrator. Closes feedback loop through Bus. |
| Reactive checking as NeuroStore method | AntiKnowledge Functor wrapping Store put() | Functor pattern. Transparent to callers. No special API. |
| Memetic fitness as standalone function | Score Cell with standard 5-axis Score | Reuses Score infrastructure. Parasites detected by comparing Score to outcomes. |

---

## What This Enables

1. **Unified dashboards.** Code verification verdicts and knowledge immune verdicts flow through the same Verdict type on the same Bus. A single dashboard displays both without adaptation.

2. **Adaptive immunity that actually learns.** The AdaptiveVerifyCell's corruption prototypes are updated by a React Cell on `knowledge.anti.created`. Every new AntiKnowledge entry makes the immune system better at detecting similar corruption. The feedback loop is closed through Bus, not through ad hoc function calls.

3. **R0 as an operational metric.** The SIR lens publishes R0 on Bus in real time. When R0 exceeds 1.0, automated remediation can lower the InnateVerifyCell's anomaly threshold, trigger an emergency Dream consolidation, or alert a human operator. This is the same React-on-Bus pattern used for circuit breakers and cost alerts.

4. **Knowledge and code share a verification dialect.** An organization that customizes its code Verify pipeline (adding custom Criterion types, adjusting rung thresholds) can use the same configuration patterns for knowledge admission. The mental model transfers.

5. **Composable defense.** Need a domain-specific immune check? Write a Verify Cell, add it to the Pipeline Graph. Need to skip the adaptive stage for trusted internal sources? Wire around it. The Pipeline is a Graph; Graph edges are configurable.

---

## Feedback Loops

| Loop | Trigger | Effect | Closes Via |
|---|---|---|---|
| **Verify failure -> AntiKnowledge** | Verify verdict fail on Bus | AutoAntiKnowledgeReactor creates AntiKnowledge Signal | React Cell subscription on `gate.verdict.emitted` |
| **AntiKnowledge -> Adaptive prototypes** | AntiKnowledge creation on Bus | AdaptiveVerifyCell bundles new fingerprint into corruption prototype | React Cell subscription on `knowledge.anti.created` |
| **Active audit -> Adaptive thresholds** | Dream consolidation triggers audit | ActiveVerifyCell findings update AdaptiveVerifyCell sensitivity | Bus topic `knowledge.audit.findings` |
| **SIR alert -> Innate tightening** | R0 > 1.0 on Bus | React Cell lowers InnateVerifyCell anomaly threshold | React Cell subscription on `knowledge.sir.snapshot` |
| **Functor rejection -> Balance signal** | Candidate rejected by AntiKnowledge Functor | Rejection published on Bus, reinforces the blocking AntiKnowledge Signal's balance | Bus topic `knowledge.admission.rejected` -> reinforce via ReinforceKind::Cited |
| **Parasite detection -> AntiKnowledge creation** | ActiveVerifyCell flags parasites | Parasites generate AntiKnowledge candidates via AutoAntiKnowledgeReactor | Audit findings published on `gate.verdict.emitted` (synthetic failure) |

Every loop closes through Bus. No loop requires the orchestrator to call a specific function. No loop has an open end. Mori lesson learned.

---

## Open Questions

1. **Is the balance floor the right mechanism?** The floor ensures AntiKnowledge persists, but it creates an immortal class of Signals. An alternative: instead of a floor, give AntiKnowledge a very high reinforcement bonus for `ReinforceKind::Cited` (when it successfully blocks a bad candidate). This would make active AntiKnowledge persist and inactive AntiKnowledge decay naturally. The question is whether inactive AntiKnowledge (refuting something nobody has tried in months) is still valuable. Probably yes -- it prevents regression when an agent encounters the same problem in a new context.

2. **Is three-level immunity over-engineered?** For a single-agent deployment with a knowledge base under 10K Signals, the innate stage alone might suffice. The adaptive stage has value only after enough AntiKnowledge entries exist to build meaningful prototypes (probably 50+). The active stage has value only when the population is large enough for statistical analysis (probably 500+). Consider making stages 2 and 3 opt-in based on Store size thresholds. The Pipeline pattern makes this natural: just wire fewer Cells for smaller deployments.

3. **How should the SIR lens handle concept drift?** The current model treats concept drift (knowledge that was correct but the environment changed) as an infection. But it is not really an infection -- it is normal staleness. Should the SIR lens distinguish between Signals that were *always* wrong (true infection) and Signals that *became* wrong (drift)? This matters because the remediation is different: infection triggers AntiKnowledge creation, while drift triggers demurrage pressure (which already exists). Over-creating AntiKnowledge for drift would clutter the immune system.

4. **What is the right threshold for the AntiKnowledge Functor's reject tier?** The current design uses 0.9 similarity as the hard rejection threshold. At 10,240-bit HDC vectors, 0.9 similarity means the candidate and the AntiKnowledge entry share ~90% of their bits -- they are encoding nearly identical structured content. But HDC similarity is not semantic identity. Two genuinely different claims about the same topic with the same tags could hit 0.9. A more conservative approach: reject at 0.95, discount at 0.80, warn at 0.60. This widens the warning band and narrows the rejection band. The right thresholds depend on empirical false positive rates, which we do not yet have.

5. **Should the Price equation alert fire on Dream cycles or wall-clock time?** The current design fires when selection < -0.1 for 3 consecutive Dream cycles. But Dream cycles are not periodic -- they trigger on idle timeout or episode threshold. If the agent is very active and never enters Dreams, the Price equation never runs. An alternative: also run the Price equation on a wall-clock timer (e.g., daily) independent of Dreams. This trades purity (all knowledge operations happen in Dreams) for reliability (the diagnostic always runs).

6. **How does AntiKnowledge interact with Worldviews?** If a Heuristic within a Worldview is refuted, the Worldview's average calibration drops. But the Worldview itself is not converted to AntiKnowledge -- it loses a member. What happens when enough members are refuted that the Worldview's calibration falls below the challenger's? The worldview swap mechanism ([06-MEMORY](../../unified/06-MEMORY.md) S6) handles this, but the interaction between the swap and the immune system should be explicitly tested: does refuting enough heuristics in a worldview reliably trigger a swap to the challenger?
