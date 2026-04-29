# Immune System as Graph

> Depth for [26-cognitive-immune-system.md](../../docs/00-architecture/26-cognitive-immune-system.md). Redesigns the five defense layers as a Graph pipeline of Cells. Taint propagation as a monotonic lattice-join React Cell. Anomaly detection as a Lens Cell. Quarantine as a Store partition. Immune memory as a Memory specialization that never forgets attacks.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Kind, taint, provenance), [02-CELL](../../unified/02-CELL.md) (Cell, React protocol, Lens, Verify protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Graph wiring), [04-SPECIALIZATIONS](../../unified/04-SPECIALIZATIONS.md) (Store partitions, Memory), [17-SECURITY-MODEL](../../unified/17-SECURITY-MODEL.md) (capability intersection, fail-closed)

---

## 1. The Immune Pipeline as a Graph

The Cognitive Immune System (CIS) is five defense layers, each implemented as a Cell, wired in a pipeline Graph. The Graph processes every Signal that crosses a trust boundary. Signals flow through the pipeline; findings, verdicts, and quarantine actions flow out.

```toml
# Graph: immune-pipeline
# Five Cells wired in a linear pipeline with feedback from Layer 5.
#
# Signal flow:
#   ingress -> taint -> anomaly -> quarantine -> incident -> immune-memory
#                                                               |
#                                    taint.recognition <---feedback---+

[graph]
id = "immune-pipeline"
description = "Five-layer cognitive immune system"

[[graph.cells]]
id = "taint-propagation"
protocol = "React"
description = "Layer 1: track untrusted lineage through Signals"

[[graph.cells]]
id = "anomaly-detection"
protocol = "Observe"
description = "Layer 2: detect contradiction clusters, fan-out, drift"

[[graph.cells]]
id = "quarantine-gate"
protocol = "Verify"
description = "Layer 3: isolate suspect Signals from default retrieval"

[[graph.cells]]
id = "incident-response"
protocol = "React"
description = "Layer 4: link findings to custody, replay, postmortem"

[[graph.cells]]
id = "immune-memory"
protocol = "Store"
description = "Layer 5: remember attacks and defenses for future recognition"

[[graph.edges]]
from = "taint-propagation.out"
to = "anomaly-detection.in"

[[graph.edges]]
from = "anomaly-detection.findings"
to = "quarantine-gate.in"

[[graph.edges]]
from = "quarantine-gate.verdicts"
to = "incident-response.in"

[[graph.edges]]
from = "incident-response.resolved"
to = "immune-memory.in"

# Feedback: immune memory informs taint recognition
[[graph.edges]]
from = "immune-memory.patterns"
to = "taint-propagation.recognition_library"
```

The pipeline is linear for the common case (most Signals pass through cleanly) but has feedback from Layer 5 to Layer 1: immune memory makes future taint recognition faster.

---

## 2. Core Types

```rust
/// Taint: the provenance-level marker that tracks untrusted origin.
///
/// Taint is durable metadata, not a temporary score penalty.
/// See [01-SIGNAL.md](../../unified/01-SIGNAL.md) SS2 for how taint
/// is stored on Signal provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Taint {
    /// Clean: no untrusted origin in lineage.
    None,
    /// User-provided input (paste, upload, ad hoc instruction).
    UserInput,
    /// External fetch (HTTP, API, scraped page).
    ExternalFetch(Source),
    /// Third-party plugin output.
    ThirdPartyPlugin(PluginId),
    /// Imported archive from another deployment.
    LegacyImport,
}

/// Threat classification for findings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThreatClass {
    PromptInjection,
    MemoryPoisoning,
    TaintCascade,
    AdversarialRetrieval,
    SandboxViolation,
    CrossTenantLeakage,
    LineageMismatch,
}

/// A finding produced by the immune pipeline.
/// This is a Signal with Kind::Finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatFinding {
    pub id: Uuid,
    pub class: ThreatClass,
    pub affected_signals: Vec<ContentHash>,
    pub taint_sources: Vec<ContentHash>,
    pub confidence: f64,
    pub severity: f64,
    pub recommended_action: ContainmentAction,
    pub custody_link: Option<ContentHash>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ContainmentAction {
    /// Watch but do not intervene.
    Monitor,
    /// Move to quarantine partition.
    Quarantine,
    /// Re-run Verify pipeline on the affected Signals.
    Reverify,
    /// Escalate to human review.
    Escalate,
    /// Disable the plugin that produced this taint.
    DisablePlugin,
}
```

---

## 3. Layer 1: Taint Propagation (React Cell)

Taint propagation is a **monotonic lattice-join** operation. The lattice is `None < UserInput < ExternalFetch < ThirdPartyPlugin < LegacyImport`. The join rule: if any input Signal is tainted, the derived output inherits the highest taint.

Monotonicity is the critical property: taint can only increase through derivation, never decrease. A Signal that was tainted at ingestion stays traceably tainted through all its descendants. Human review can approve downstream use, but the approval is recorded through custody -- it does not rewrite ancestor provenance.

```rust
/// Layer 1: Taint Propagation React Cell.
///
/// This Cell is a monotonic lattice-join operator.
/// It subscribes to all Signal creation events and ensures that
/// derived Signals inherit the maximum taint of their inputs.
///
/// Lattice: None < UserInput < ExternalFetch < ThirdPartyPlugin < LegacyImport
/// Join: derived_taint = max(input_taints)
///
/// Monotonicity guarantee: taint(descendant) >= taint(ancestor).
/// This is enforced structurally -- the join function cannot produce
/// a result lower than any input.
pub struct TaintPropagationCell {
    /// Recognition library: HDC fingerprints of known attack patterns.
    /// Fed by Layer 5 (immune memory) via the feedback edge.
    recognition_library: RwLock<Vec<(HdcVector, ThreatClass)>>,
}

impl Cell for TaintPropagationCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn name(&self) -> &str { "taint-propagation" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut outputs = Vec::new();

        for signal in &input {
            // 1. Compute derived taint from lineage
            let parent_taints: Vec<Taint> = signal.parent_hashes.iter()
                .filter_map(|h| ctx.store().get_taint(h))
                .collect();
            let derived_taint = lattice_join(&parent_taints);

            // 2. Check ingress taint (new Signals from trust boundaries)
            let ingress_taint = classify_ingress(&signal);
            let final_taint = lattice_join(&[derived_taint, ingress_taint]);

            // 3. Check against recognition library (known attack patterns)
            if let Some(fp) = &signal.metadata.hdc_fingerprint {
                let library = self.recognition_library.read();
                for (pattern_fp, threat_class) in library.iter() {
                    let similarity = hdc_cosine_similarity(fp, pattern_fp);
                    if similarity > 0.85 {
                        // Known attack pattern detected!
                        let finding = ThreatFinding {
                            id: Uuid::new_v4(),
                            class: threat_class.clone(),
                            affected_signals: vec![signal.hash()],
                            taint_sources: signal.parent_hashes.clone(),
                            confidence: similarity,
                            severity: 0.9,
                            recommended_action: ContainmentAction::Quarantine,
                            custody_link: None,
                        };
                        outputs.push(Signal::new(Kind::Finding, finding));
                    }
                }
            }

            // 4. Annotate the Signal with its computed taint
            if final_taint != Taint::None {
                let annotation = Signal::pulse(
                    Kind::Annotation,
                    topic!("safety.taint.detected"),
                    TaintAnnotation {
                        signal_hash: signal.hash(),
                        taint: final_taint,
                    },
                );
                outputs.push(annotation);
            }
        }

        Ok(outputs)
    }
}

/// Monotonic lattice join: returns the maximum taint.
fn lattice_join(taints: &[Taint]) -> Taint {
    taints.iter()
        .max()
        .cloned()
        .unwrap_or(Taint::None)
}
```

### Why Monotonic Matters

If taint could decrease through derivation, an attacker could launder a poisoned Signal by deriving a clean-looking descendant from it. Monotonicity closes this path: the only way to "clean" a tainted lineage is through human review recorded in custody. The ancestor's taint is never rewritten -- only the descendant's use is approved.

---

## 4. Layer 2: Anomaly Detection (Lens Cell)

Not all corruption starts with taint. Layer 2 watches for patterns that suggest the knowledge graph is behaving unlike itself. It is a **Lens Cell** (see [02-CELL.md](../../unified/02-CELL.md) SS7) -- it observes and reports, it does not act.

```rust
/// Layer 2: Anomaly Detection Lens Cell.
///
/// Monitors six indicators of knowledge-graph anomaly.
/// Publishes ThreatFinding Signals when indicators exceed thresholds.
pub struct AnomalyDetectionLens {
    /// Z-score threshold for statistical anomaly (default: 3.0).
    z_threshold: f64,
    /// Maximum fan-out before alerting (default: 50).
    fanout_alert_threshold: u64,
    /// Enable lineage gap detection.
    lineage_gap_alert: bool,
}

/// The six anomaly indicators.
/// These are "danger model" style cues (Matzinger 2002):
/// the CIS responds when the system shows signs of damage,
/// not only when the content is foreign.
pub enum AnomalyIndicator {
    /// Many new claims suddenly conflict with established Signals.
    ContradictionBurst {
        new_signals: Vec<ContentHash>,
        contradicted: Vec<ContentHash>,
        contradiction_rate: f64,
    },
    /// Retrieval rank rises but Verify and lineage don't justify it.
    ScoreSpikeWithoutSupport {
        signal_hash: ContentHash,
        score_delta: f64,
        gate_passes: u32,
    },
    /// One import contaminates a large lineage region.
    TaintFanoutBurst {
        source: ContentHash,
        affected_count: u64,
    },
    /// One plugin repeatedly exceeds its permission envelope.
    SandboxViolationCluster {
        plugin_id: PluginId,
        violation_count: u32,
        window_secs: u64,
    },
    /// Query path mixes two tenant prefixes.
    TenantBoundaryMismatch {
        tenant_a: String,
        tenant_b: String,
        mixed_signals: Vec<ContentHash>,
    },
    /// Durable record cites missing or unverifiable ancestors.
    LineageGap {
        signal_hash: ContentHash,
        missing_ancestors: Vec<ContentHash>,
    },
}

impl Cell for AnomalyDetectionLens {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn name(&self) -> &str { "anomaly-detection" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut findings = Vec::new();

        // Check each indicator against its threshold
        for indicator in self.scan_indicators(&input, ctx).await? {
            let (class, severity, confidence) = match &indicator {
                AnomalyIndicator::ContradictionBurst { contradiction_rate, .. } => {
                    let z = self.z_score(*contradiction_rate, ctx).await;
                    if z > self.z_threshold {
                        (ThreatClass::MemoryPoisoning, 0.8, z / 5.0)
                    } else { continue; }
                }
                AnomalyIndicator::TaintFanoutBurst { affected_count, .. } => {
                    if *affected_count > self.fanout_alert_threshold {
                        (ThreatClass::TaintCascade, 0.7, 0.9)
                    } else { continue; }
                }
                AnomalyIndicator::SandboxViolationCluster { violation_count, .. } => {
                    if *violation_count > 3 {
                        (ThreatClass::SandboxViolation, 0.9, 0.95)
                    } else { continue; }
                }
                AnomalyIndicator::TenantBoundaryMismatch { .. } => {
                    // Tenant mismatch never degrades to warning only.
                    (ThreatClass::CrossTenantLeakage, 1.0, 1.0)
                }
                AnomalyIndicator::LineageGap { .. } => {
                    if self.lineage_gap_alert {
                        (ThreatClass::LineageMismatch, 0.5, 0.7)
                    } else { continue; }
                }
                AnomalyIndicator::ScoreSpikeWithoutSupport { score_delta, .. } => {
                    let z = self.z_score(*score_delta, ctx).await;
                    if z > self.z_threshold {
                        (ThreatClass::AdversarialRetrieval, 0.6, z / 5.0)
                    } else { continue; }
                }
            };

            let finding = ThreatFinding {
                id: Uuid::new_v4(),
                class,
                affected_signals: indicator.affected_hashes(),
                taint_sources: indicator.source_hashes(),
                confidence: confidence.min(1.0),
                severity,
                recommended_action: if severity >= 0.8 {
                    ContainmentAction::Quarantine
                } else {
                    ContainmentAction::Monitor
                },
                custody_link: None,
            };
            findings.push(Signal::new(Kind::Finding, finding));
        }

        Ok(findings)
    }
}
```

---

## 5. Layer 3: Quarantine (Store Partition with Restricted Access)

Quarantine is a **Store partition** (see [04-SPECIALIZATIONS.md](../../unified/04-SPECIALIZATIONS.md)). Suspect Signals stay durable and queryable for reviewers, but they disappear from default retrieval and Compose assembly.

```rust
/// Layer 3: Quarantine Gate (Verify Cell).
///
/// When a ThreatFinding recommends quarantine, this Cell:
/// 1. Moves the affected Signals to the quarantine partition.
/// 2. Removes them from default query paths.
/// 3. Retains them in lineage (history is never erased).
/// 4. Emits a quarantine entry Signal for audit.
pub struct QuarantineGateCell;

/// A quarantine entry: metadata about why a Signal was quarantined.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineEntry {
    pub signal_hash: ContentHash,
    pub taint: Taint,
    pub reason: ThreatClass,
    pub placed_at: SystemTime,
    pub custody_link: Option<ContentHash>,
    pub review_required: bool,
    pub reviewer_release: Option<PrincipalId>,
}

impl Cell for QuarantineGateCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "quarantine-gate" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut outputs = Vec::new();

        for signal in &input {
            let finding: ThreatFinding = extract_finding(signal)?;

            match finding.recommended_action {
                ContainmentAction::Quarantine => {
                    // Move affected Signals to quarantine partition
                    for hash in &finding.affected_signals {
                        ctx.store().move_to_partition(hash, "quarantine").await?;
                    }

                    let entry = QuarantineEntry {
                        signal_hash: finding.affected_signals[0],
                        taint: ctx.store().get_taint(&finding.affected_signals[0])
                            .unwrap_or(Taint::None),
                        reason: finding.class.clone(),
                        placed_at: SystemTime::now(),
                        custody_link: finding.custody_link,
                        review_required: finding.severity >= 0.7,
                        reviewer_release: None,
                    };

                    outputs.push(Signal::new(Kind::QuarantineEntry, entry));
                    outputs.push(Signal::pulse(
                        Kind::Event,
                        topic!("safety.quarantine.entered"),
                        QuarantineEvent {
                            signal_hash: finding.affected_signals[0],
                            class: finding.class,
                        },
                    ));
                }
                ContainmentAction::Escalate => {
                    // Emit escalation Pulse for human review
                    outputs.push(Signal::pulse(
                        Kind::Alert,
                        topic!("safety.escalation.required"),
                        finding,
                    ));
                }
                _ => {
                    // Monitor or Reverify: pass through with annotation
                    outputs.push(signal.clone());
                }
            }
        }

        Ok(outputs)
    }
}
```

### Quarantine Store Semantics

| Operation | Quarantine behavior |
|---|---|
| `store.query()` | **Excludes** quarantine partition by default |
| `store.query_with_quarantine()` | Includes quarantine (requires review scope capability) |
| Compose assembly | **Excludes** quarantine unless caller has explicit review scope |
| Lineage traversal | **Includes** quarantine (history is never hidden) |
| Bus publication | Quarantine events publish on `safety.quarantine.*` topics |

### Resolution Workflow

The CIS does not pretend contamination never existed. The release workflow:

1. Detect and place the Signal in quarantine.
2. Run full re-verification against current Verify pipeline.
3. Open review if the Signal could influence visible, destructive, or cross-tenant actions.
4. Record the reviewer decision in custody; require OrgRole attestation for high-risk release.
5. Either:
   - Keep the original quarantined and produce a reviewed successor Signal for reuse, OR
   - Keep quarantined permanently and publish a falsifier or postmortem.

---

## 6. Layer 4: Incident Response (React Cell)

When a finding touches an auditable action, Layer 4 links the finding to custody for traceability.

```rust
/// Layer 4: Incident Response React Cell.
///
/// Links ThreatFindings to Custody records when the incident
/// touches an auditable action. Supports replay and postmortem.
pub struct IncidentResponseCell;

/// Links a finding to its custody chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentLink {
    pub custody_hash: ContentHash,
    pub findings: Vec<Uuid>,
    pub affected_signals: Vec<ContentHash>,
    pub taint_sources: Vec<ContentHash>,
    pub replay_snapshot: Option<ContentHash>,
    pub postmortem: Option<ContentHash>,
}

impl Cell for IncidentResponseCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn name(&self) -> &str { "incident-response" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut outputs = Vec::new();

        for signal in &input {
            let finding: ThreatFinding = extract_finding_or_entry(signal)?;

            // 1. Find the custody record for the affected action (if any)
            let custody = ctx.store().find_custody_for(&finding.affected_signals).await;

            // 2. Walk backward through contributing Signals and taint sources
            let taint_chain = ctx.store().trace_taint_lineage(
                &finding.affected_signals
            ).await?;

            // 3. Create replay snapshot for future reconstruction
            let replay = ctx.store().snapshot_context(
                &finding.affected_signals,
            ).await?;

            // 4. Build incident link
            let link = IncidentLink {
                custody_hash: custody.map(|c| c.hash()).unwrap_or_default(),
                findings: vec![finding.id],
                affected_signals: finding.affected_signals.clone(),
                taint_sources: taint_chain,
                replay_snapshot: Some(replay.hash()),
                postmortem: None, // filled later by human review
            };

            outputs.push(Signal::new(Kind::Incident, link));

            // 5. Publish incident Pulse for dashboards
            outputs.push(Signal::pulse(
                Kind::Event,
                topic!("safety.incident.opened"),
                IncidentOpened {
                    finding_id: finding.id,
                    class: finding.class,
                    severity: finding.severity,
                },
            ));
        }

        Ok(outputs)
    }
}
```

### What the Incident Record Tells the Auditor

| Question | Source of truth |
|---|---|
| Who initiated the action | `Custody.principal` |
| Why the system thought the action was reasonable | Heuristics and claims in the custody record |
| Which Verify Cells passed or failed | `gates_passed` plus replay |
| Whether tainted inputs were present | CIS taint sources and quarantine entries |
| Whether a human approved release | `authorized` plus attestation level |

---

## 7. Layer 5: Immune Memory (Memory Specialization)

Immune memory is a **Memory specialization** (see [11-MEMORY-AND-KNOWLEDGE](../../unified/11-MEMORY-AND-KNOWLEDGE.md)) with one critical property: it never forgets attacks. Normal Signals are subject to demurrage. Immune memory Signals have **zero demurrage** -- they persist indefinitely.

```rust
/// Layer 5: Immune Memory.
///
/// A Memory specialization that stores:
/// - HDC fingerprints of known attack patterns (for Layer 1 recognition)
/// - Taint source and fan-out shapes (for geometry matching)
/// - Best containment actions (reuse what worked)
/// - False-positive records (avoid over-quarantining)
/// - Postmortem and custody links (audit trail)
///
/// Critical property: immune memory Signals have ZERO demurrage.
/// The system never forgets a confirmed attack pattern.
pub struct ImmuneMemoryStore {
    /// Known attack pattern fingerprints with their classifications.
    patterns: Vec<ImmunePattern>,
    /// False positive records (signals that were quarantined but released).
    false_positives: Vec<FalsePositive>,
    /// Recognition threshold for fingerprint matching (default: 0.85).
    recognition_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImmunePattern {
    /// HDC fingerprint of the attack pattern.
    pub fingerprint: HdcVector,
    /// Classification of the attack.
    pub class: ThreatClass,
    /// The containment action that resolved this pattern.
    pub best_containment: ContainmentAction,
    /// When this pattern was first observed.
    pub first_seen: SystemTime,
    /// How many times this pattern has been matched.
    pub match_count: u64,
    /// Link to the original incident.
    pub incident_link: ContentHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalsePositive {
    /// The Signal that was wrongly quarantined.
    pub signal_hash: ContentHash,
    /// The fingerprint that was wrongly matched.
    pub matched_pattern: HdcVector,
    /// The similarity score that triggered the false match.
    pub similarity: f64,
    /// When the false positive was resolved.
    pub resolved_at: SystemTime,
}

impl Cell for ImmuneMemoryStore {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }
    fn name(&self) -> &str { "immune-memory" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut outputs = Vec::new();

        for signal in &input {
            match signal.kind {
                Kind::Incident => {
                    // Store the incident as an immune pattern
                    let incident: IncidentLink = extract_incident(signal)?;
                    if let Some(fp) = signal.metadata.hdc_fingerprint.as_ref() {
                        let pattern = ImmunePattern {
                            fingerprint: fp.clone(),
                            class: extract_class(&incident)?,
                            best_containment: extract_containment(&incident)?,
                            first_seen: SystemTime::now(),
                            match_count: 1,
                            incident_link: signal.hash(),
                        };

                        // Publish pattern to Layer 1 via feedback edge
                        outputs.push(Signal::new(Kind::ImmunePattern, pattern));
                    }
                }
                Kind::QuarantineRelease => {
                    // Check if this was a false positive
                    let release: QuarantineRelease = extract_release(signal)?;
                    if release.was_false_positive {
                        let fp = FalsePositive {
                            signal_hash: release.signal_hash,
                            matched_pattern: release.matched_pattern,
                            similarity: release.match_similarity,
                            resolved_at: SystemTime::now(),
                        };
                        self.false_positives.push(fp);
                    }
                }
                _ => {}
            }
        }

        Ok(outputs)
    }
}
```

### Delta Probes: Exercising Immune Memory

During Delta consolidation, the immune system exercises itself:

1. **Replay prior poisoning cases** against updated Verify Cells. If a prior attack pattern now bypasses containment, the CIS raises a new high-severity finding.
2. **Probe known weak spots** with synthetic hostile inputs (Forrest-style negative selection).
3. **Check quarantine integrity**: verify that quarantined lineage does not leak into Compose assembly.
4. **Validate plugin containment**: confirm that sandbox violations still force containment.

```rust
/// Delta probe: exercise immune memory against current defenses.
pub async fn delta_immune_probe(
    memory: &ImmuneMemoryStore,
    pipeline: &ImmuneGraph,
    store: &dyn Store,
) -> Vec<ThreatFinding> {
    let mut regressions = Vec::new();

    for pattern in &memory.patterns {
        // Synthesize a Signal matching the attack pattern
        let synthetic = Signal::synthetic_from_fingerprint(
            &pattern.fingerprint,
            pattern.class.clone(),
        );

        // Run through the immune pipeline
        let result = pipeline.execute(vec![synthetic]).await;

        // Check if the pipeline caught it
        let was_caught = result.iter().any(|s| {
            matches!(s.kind, Kind::Finding | Kind::QuarantineEntry)
        });

        if !was_caught {
            // Regression! A known attack pattern now bypasses defenses.
            regressions.push(ThreatFinding {
                id: Uuid::new_v4(),
                class: pattern.class.clone(),
                affected_signals: vec![],
                taint_sources: vec![],
                confidence: 1.0,
                severity: 1.0, // maximum severity for a regression
                recommended_action: ContainmentAction::Escalate,
                custody_link: Some(pattern.incident_link),
            });
        }
    }

    regressions
}
```

---

## 8. Autoimmune: When the Immune System Attacks Healthy Signals

The most dangerous failure mode is not missed attacks -- it is false positives. When the immune system quarantines healthy Signals, it damages the system by removing useful knowledge from retrieval.

### Detection

False positives are detected by monitoring the quarantine release rate. If operators are releasing more than a threshold percentage of quarantined Signals, the immune system is over-aggressive.

```rust
/// Lens Cell: false positive rate monitoring.
///
/// If quarantine_release_rate > threshold, the immune system
/// is too aggressive and is damaging healthy knowledge.
pub struct AutoimmuneLens {
    /// Maximum acceptable false positive rate (default: 0.1).
    max_fp_rate: f64,
    /// Window for rate calculation (default: 7 days).
    window_days: u32,
}

impl AutoimmuneLens {
    pub fn check(
        &self,
        quarantined: u64,
        released: u64,
    ) -> Option<Signal> {
        if quarantined == 0 { return None; }
        let fp_rate = released as f64 / quarantined as f64;

        if fp_rate > self.max_fp_rate {
            Some(Signal::pulse(
                Kind::Alert,
                topic!("safety.autoimmune.warning"),
                AutoimmuneWarning {
                    fp_rate,
                    threshold: self.max_fp_rate,
                    recommendation: "Widen anomaly detection thresholds \
                        or review recognition library for over-broad patterns".into(),
                },
            ))
        } else {
            None
        }
    }
}
```

### Recovery Path

When autoimmune behavior is detected:

1. **Widen thresholds**: increase `z_threshold` in the Anomaly Detection Lens. This is an L1 parameter adjustment ([10-LEARNING-LOOPS.md](../../unified/10-LEARNING-LOOPS.md) SS3).
2. **Record false positives**: every quarantine release adds a FalsePositive record to immune memory. Layer 1 checks against false positives before quarantining.
3. **Pattern refinement**: if a specific immune pattern generates too many false matches, narrow its fingerprint or raise its activation threshold.
4. **Quarantine budget**: limit the maximum number of Signals that can be quarantined per window. If the budget is exhausted, new quarantine actions require human approval.

```rust
/// False positive recovery: check if a Signal matches a known false positive
/// before quarantining. Returns true if the Signal should NOT be quarantined.
fn is_known_false_positive(
    signal: &Signal,
    false_positives: &[FalsePositive],
    threshold: f64,
) -> bool {
    if let Some(fp) = &signal.metadata.hdc_fingerprint {
        false_positives.iter().any(|known_fp| {
            let similarity = hdc_cosine_similarity(fp, &known_fp.matched_pattern);
            similarity > threshold
        })
    } else {
        false
    }
}
```

---

## 9. Daimon Integration: Caution Without Override

The immune system may bias the Daimon toward a more cautious operating posture, but it does not override the security model's authorization decisions (see [17-SECURITY-MODEL.md](../../unified/17-SECURITY-MODEL.md)).

High recent finding severity should:
- Lower willingness to take autonomous action.
- Increase confirmation pressure.
- Bias routing toward stricter Verify pipelines.

The separation matters: the Daimon alters posture, the security model approves or denies, custody records who approved what. The immune response is advisory for behavior and authoritative for knowledge-integrity metadata.

---

## What This Enables

1. **Pipeline defense**: five layers, each catching what the previous layer missed. A Signal must evade all five to cause harm.
2. **Monotonic taint**: tainted lineage cannot be laundered by derivation. Provenance is permanent.
3. **Self-improving defense**: immune memory feeds Layer 1 recognition, making future detection faster.
4. **Autoimmune protection**: false positive monitoring prevents the immune system from damaging healthy knowledge.
5. **Auditable incidents**: every finding links back to custody, replay snapshot, and postmortem.
6. **Delta self-testing**: the immune system exercises itself offline and detects defense regressions.

## Feedback Loops

- **L1**: anomaly detection thresholds adjust via EMA based on true/false positive rates.
- **L2**: routing avoids models/plugins with recent sandbox violations.
- **L3**: Delta probes exercise immune memory and detect regressions.
- **L4**: structural proposals to tighten or relax immune thresholds based on sustained false positive rates.
- **Memory feedback**: Layer 5 patterns feed Layer 1 recognition via the Graph's feedback edge.

## Open Questions

1. **Immune memory growth**: if immune memory never forgets, how large does it grow? Is there a practical limit on the number of stored patterns? The answer may be that patterns are HDC fingerprints (compact) and the library is searched by similarity (sublinear), so growth is manageable. But this needs empirical confirmation.
2. **Cross-deployment immune sharing**: should immune patterns be shared across deployments? This would spread defense knowledge faster but also spread false positive patterns. The heuristic commons from [autocatalytic-compounding.md](../10-learning-loops/autocatalytic-compounding.md) could be the transport mechanism, with quarantine-on-import for untrusted patterns.
3. **Taint declassification**: can taint ever be formally downgraded (not just approved for use)? The monotonic lattice says no. But some scenarios (e.g., a URL that was hostile but is now controlled by the system operator) suggest that taint reclassification may be needed. The custody system could support this with sufficiently high attestation requirements.
4. **Performance impact**: running every Signal through a five-layer pipeline adds latency. The common case (clean Signal, no anomaly) should be fast -- but how fast? Layer 1 is O(1) lattice join plus O(k) HDC similarity checks against the recognition library. Layer 2 is O(n) indicator scans. This needs benchmarking.
5. **Composability with external immune systems**: if Roko is deployed behind an enterprise security perimeter, should its CIS integrate with external threat feeds? The recognition library could ingest external IoCs as immune patterns, but the confidence calibration would be different.
