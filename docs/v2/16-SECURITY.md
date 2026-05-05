# 16 -- Security Model

> Three-layer capability intersection, taint lattice IFC, 5-head lexicographic corrigibility, immune system as a 5-layer Pipeline Graph, and sandboxing at every tier. The system fails closed. Verify gates sit outside the modifiable surface. The agent cannot modify its own verification pipeline.

**Subsumes**: Cell capability model, Space grants, Extension safety layers, autonomy-level safety mapping, agent contracts, cognitive immune system.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal, Kind, taint, provenance), [02-CELL](02-CELL.md) (Cell protocols: React, Observe/Lens, Verify, Store), [03-GRAPH](03-GRAPH.md) (Graph wiring), [06-MEMORY](06-MEMORY.md) (Store partitions, Memory), [15-TELEMETRY](15-TELEMETRY.md) (AnomalyLens)

---

## 1. Overview

Roko's security model is built on one principle: **the system fails closed**. No Cell runs unless every layer of the capability stack explicitly permits it. Capabilities can be narrowed but never widened when delegated. Every grant, usage, and denial is logged as a Signal.

Five mechanisms, from innermost to outermost:

1. **Three-layer capability intersection** -- Cell declaration, Graph allow-list, Space grant.
2. **Taint lattice IFC** -- monotonic information flow control on all data crossing trust boundaries.
3. **5-head lexicographic corrigibility** -- deference > switch > truth > impact > task.
4. **Verify-outside-modifiable** -- the agent cannot modify its own verification pipeline.
5. **Immune system** -- 5-layer Pipeline Graph for runtime threat detection and response.

```
Cell declaration  intersection  Graph allow-list  intersection  Space grant  =  effective capabilities
Taint lattice IFC tags every data flow across trust boundaries
5-head corrigibility orders all decisions lexicographically
Verify gates sit outside the modifiable surface
Immune pipeline monitors all Signals crossing trust boundaries
```

The intersection is strict: the effective capability set is the narrowest constraint at each layer. A capability absent from any single layer is denied, full stop. There is no override, no escalation path that bypasses the intersection.

---

## 2. Capability Types

Capabilities describe what system resources a Cell may access. Eleven capability types with granular constraints cover the full resource surface.

```rust
pub enum Capability {
    FsRead { paths: Option<Vec<PathPattern>> },
    FsWrite { paths: Option<Vec<PathPattern>> },
    Net { domains: Option<Vec<String>> },
    Shell { commands: Option<Vec<String>> },
    Llm { providers: Option<Vec<String>> },
    Chain { read: bool, write: bool, networks: Option<Vec<String>> },
    Secrets { keys: Option<Vec<String>> },
    KnowledgeRead,
    KnowledgeWrite,
    Process { kind: ProcessKind },
    Custom { name: String, params: Value },
}

pub enum ProcessKind {
    Spawn,
    Signal,
    Kill,
}
```

### 2.1 Capability Semantics

| Capability | Grants | Constraints |
|---|---|---|
| `FsRead` | Read files from the filesystem | `paths`: glob patterns restricting readable paths |
| `FsWrite` | Write files to the filesystem | `paths`: glob patterns restricting writable paths |
| `Net` | Make outbound network requests | `domains`: allowlisted domains. `*` for unrestricted |
| `Shell` | Execute shell commands | `commands`: allowlisted command names (e.g., `["cargo", "git"]`). No wildcard |
| `Llm` | Call LLM providers | `providers`: optional provider filter |
| `Chain` | Interact with blockchains | `read`/`write` flags. `networks`: allowlisted chain names |
| `Secrets` | Access stored secrets | `keys`: specific secret names the Cell may read |
| `KnowledgeRead` | Query the knowledge store | No constraints (scoped by Space) |
| `KnowledgeWrite` | Write to the knowledge store | No constraints (scoped by Space) |
| `Process` | Spawn or manage system processes | `kind`: `Spawn`, `Signal`, `Kill` |
| `Custom` | Extension-defined capabilities | `name` + arbitrary `params` |

### 2.2 Constraint Narrowing

When a capability appears at multiple layers, the effective capability is the **intersection** of constraints at each layer. The narrowest constraint at any layer wins.

| Cell declares | Graph allows | Space grants | Effective |
|---|---|---|---|
| `Net { domains: ["api.openai.com"] }` | `Net { domains: ["api.openai.com", "api.anthropic.com"] }` | `Net { domains: ["*"] }` | `Net { domains: ["api.openai.com"] }` |
| `FsWrite { paths: ["**"] }` | `FsWrite { paths: [".roko/**"] }` | `FsWrite { paths: [".roko/**", "tmp/**"] }` | `FsWrite { paths: [".roko/**"] }` |
| `Shell { commands: ["cargo", "git", "rm"] }` | `Shell { commands: ["cargo", "git"] }` | `Shell { commands: ["cargo", "git", "npm"] }` | `Shell { commands: ["cargo", "git"] }` |

```rust
pub fn intersect_capabilities(
    block: &[Capability],
    graph: &[Capability],
    space: &[Capability],
) -> Vec<EffectiveCapability> {
    block.iter().filter_map(|b_cap| {
        let g_cap = graph.iter().find(|g| g.same_variant(b_cap))?;
        let s_cap = space.iter().find(|s| s.same_variant(b_cap))?;
        Some(EffectiveCapability {
            capability: b_cap.variant(),
            constraints: b_cap.constraints()
                .intersect(&g_cap.constraints())
                .intersect(&s_cap.constraints()),
        })
    }).collect()
}
```

---

## 3. Three-Layer Capability Stack

### 3.1 Layer 1: Cell Declaration

Every Cell declares what capabilities it requires in its TOML manifest:

```toml
[cell.capabilities]
required = [
  { "FsRead"  = { paths = ["src/**", "docs/**"] } },
  { "FsWrite" = { paths = [".roko/artifacts/**"] } },
  { "Shell"   = { commands = ["cargo", "rustc"] } },
  { "Llm" = {} },
]
```

A Cell that does not declare a capability cannot access that resource, even if the Graph and Space both allow it. The Cell's declaration is a ceiling on what it can ever do.

### 3.2 Layer 2: Graph Allow-List

Graphs may restrict what their constituent Cells can do:

```toml
[graph.capabilities]
allow = [
  { "FsRead"  = { paths = ["src/**"] } },    # narrower than Cell's declaration
  { "FsWrite" = { paths = [".roko/**"] } },
  { "Llm" = {} },
  # Shell intentionally omitted -- not allowed in this Graph
]
```

Omitting a capability from the Graph's allow-list denies it. A general-purpose Cell that can run arbitrary shell commands becomes safe to use in a read-only analysis Graph by simply not including `Shell`.

### 3.3 Layer 3: Space Grant

The Space (workspace) is the user's authority. The user grants capabilities in `workspace.toml`:

```toml
[space.capabilities]
fs_read       = true
fs_write      = { paths = [".roko/**", "tmp/**", "dist/**"] }
net           = { domains = ["api.anthropic.com", "api.openai.com", "*.perplexity.ai"] }
llm           = true
shell         = { commands = ["cargo", "git", "npm", "rustc"] }
chain_write   = false
secrets       = { keys = ["anthropic_key", "openai_key"] }
```

Space grants are the user's final word. A capability not granted by the Space is denied regardless of Cell and Graph declarations.

### 3.4 Resolution Algorithm

At Graph-load time, the runtime computes effective capabilities for every Cell:

```
for each Cell in Graph:
    for each capability in Cell.required:
        graph_allowed = Graph.allow.contains(capability)
        space_granted = Space.grants.contains(capability)

        if !graph_allowed:
            error("Graph does not allow {capability} for Cell {block}")
        if !space_granted:
            prompt_user("Cell {block} requires {capability}. Grant?")

        effective[block][capability] = intersect(
            block.declared,
            graph.allowed,
            space.granted,
        )
```

At Cell-run time, every resource access is checked against the effective capability set. Violations emit a `CapabilityDenied` error Signal and are logged to the audit trail (section 11).

---

## 4. Taint Lattice Information Flow Control

### 4.1 The Taint Lattice

Every piece of data flowing through the system carries a taint level. The lattice is ordered:

```
Clean < UserInput < LlmGenerated < ExternalFetch < Propagated
```

Taint is **monotonic**: it can only increase through derivation, never decrease. A Signal that was tainted at ingestion stays traceably tainted through all its descendants.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Taint {
    /// No untrusted origin in lineage.
    Clean,
    /// User-provided input (paste, upload, ad hoc instruction).
    UserInput,
    /// LLM-generated content (model output, not human-authored).
    LlmGenerated,
    /// External fetch (HTTP, API, scraped page).
    ExternalFetch(Source),
    /// Propagated through derivation from tainted ancestors.
    Propagated,
}
```

### 4.2 Monotonic Lattice-Join

Taint propagation is a lattice-join operation. The join rule: if any input Signal is tainted, the derived output inherits the highest taint.

```rust
/// Monotonic lattice join: returns the maximum taint.
/// Guarantee: taint(descendant) >= taint(ancestor).
fn lattice_join(taints: &[Taint]) -> Taint {
    taints.iter()
        .max()
        .cloned()
        .unwrap_or(Taint::Clean)
}
```

If taint could decrease through derivation, an attacker could launder a poisoned Signal by deriving a clean-looking descendant. Monotonicity closes this path: the only way to "clean" a tainted lineage is through human review recorded in custody.

### 4.3 CaMeL IFC on Extensions

**CaMeL IFC** (Capability-tagged information flow control; Fang et al. 2024) is applied to Extensions. Every data flow through an Extension is tagged with its capability provenance.

```rust
pub struct CamelTag {
    /// Which capabilities were involved in producing this data.
    pub capabilities: BTreeSet<Capability>,
    /// Which Cells touched this data, in order.
    pub provenance: Vec<CellRef>,
    /// Aggregated taint from all producers.
    pub taint: Taint,
}
```

### 4.4 Tag Propagation Rules

When an Extension processes data, three rules govern tag propagation:

1. **Union on input**: the Extension receives data with tags `T1` and `T2`. The Extension's working set has tag `T1 union T2`.
2. **Inherit on output**: any data the Extension produces inherits the union tag from its inputs.
3. **Capability check on consumption**: when a Cell receives tagged data from an Extension, the Cell's effective capabilities must include all capabilities in the tag. If not, the data is rejected.

```
Extension receives: data(tag: {FsRead, Llm})
Extension produces: transformed_data(tag: {FsRead, Llm})     -- tags propagated
Cell receives:     transformed_data
Cell capabilities: {FsRead, Llm, Net}
Check:              {FsRead, Llm} is subset of {FsRead, Llm, Net}  -- PASS
```

```
Extension receives: data(tag: {FsRead, Secrets})
Extension produces: transformed_data(tag: {FsRead, Secrets})  -- tags propagated
Cell receives:     transformed_data
Cell capabilities: {Net}
Check:              {FsRead, Secrets} is subset of {Net}          -- DENIED
```

### 4.5 No-Laundering Guarantee

Extensions **cannot strip tags**. The runtime computes output tags as `input_tags union extension_capability_tags`, never as a subset.

```rust
fn compute_output_tags(
    input_tags: &CamelTag,
    extension_caps: &[Capability],
    extension_ref: CellRef,
) -> CamelTag {
    CamelTag {
        capabilities: input_tags.capabilities
            .union(&extension_caps.iter().cloned().collect())
            .cloned()
            .collect(),
        provenance: {
            let mut p = input_tags.provenance.clone();
            p.push(extension_ref);
            p
        },
        taint: Taint::max(input_tags.taint, compute_taint(extension_caps)),
    }
}
```

### 4.6 Declassification Requires Human Approval

`Sensitive` data (from Secrets, sensitive file paths) cannot flow to `Net` without explicit `declassify` approval from the user. The declassification event is logged as a `SecurityEvent::Declassification` Signal with full provenance.

---

## 5. Five-Head Lexicographic Corrigibility

Every Agent decision passes through a 5-head lexicographic ordering (Nayebi 2024). The heads are evaluated in strict priority order. A higher-priority head ALWAYS trumps a lower-priority head, regardless of magnitude.

### 5.1 The Five Heads

| Priority | Head | Meaning | Verify Cell |
|---|---|---|---|
| 1 (highest) | **Deference** | Obey the human's stated preferences and constraints | `VerifyDeference` |
| 2 | **Switch** | Preserve the human's ability to change the agent's behavior | `VerifySwitch` |
| 3 | **Truth** | Represent information accurately. Do not deceive | `VerifyTruth` |
| 4 | **Impact** | Minimize unintended side effects. Reversibility preference | `VerifyImpact` |
| 5 (lowest) | **Task** | Accomplish the assigned task effectively | `VerifyTask` |

### 5.2 Implementation as Pipeline of Verify Cells

Each head is a separate Verify-protocol Cell. They run in sequence during the pre-action `verify_pre()` phase. The chain short-circuits on first rejection:

```rust
async fn verify_pre(action: &ProposedAction, ctx: &VerifyContext) -> VerifyResult {
    // Head 1: Deference -- obey human constraints
    let verdict = ctx.verify_deference.verify(action).await?;
    if verdict.rejected() {
        return VerifyResult::Reject {
            head: CorrigibilityHead::Deference,
            reason: verdict.reason,
        };
    }

    // Head 2: Switch -- preserve human's ability to intervene
    let verdict = ctx.verify_switch.verify(action).await?;
    if verdict.rejected() {
        return VerifyResult::Reject {
            head: CorrigibilityHead::Switch,
            reason: verdict.reason,
        };
    }

    // Head 3: Truth -- do not deceive
    let verdict = ctx.verify_truth.verify(action).await?;
    if verdict.rejected() {
        return VerifyResult::Reject {
            head: CorrigibilityHead::Truth,
            reason: verdict.reason,
        };
    }

    // Head 4: Impact -- minimize side effects
    let verdict = ctx.verify_impact.verify(action).await?;
    if verdict.rejected() {
        return VerifyResult::Reject {
            head: CorrigibilityHead::Impact,
            reason: verdict.reason,
        };
    }

    // Head 5: Task -- only this head optimizes for performance
    let verdict = ctx.verify_task.verify(action).await?;
    VerifyResult::from_verdict(verdict)
}
```

The first head to reject terminates the chain. No lower-priority head is consulted.

### 5.3 Head Definitions

**VerifyDeference**: checks that the proposed action respects the user's stated constraints -- Space grants, autonomy level, budget limits, explicit instructions. Deference is the only head that can be overridden -- but only by the user themselves, not by the agent.

**VerifySwitch**: checks that the proposed action does not reduce the human's ability to intervene. An action that disables logging, removes audit trails, modifies the verification pipeline, or escalates its own privileges without user approval fails Switch.

**VerifyTruth**: checks that the proposed action's reporting is accurate. An action that produces output claiming "all tests pass" when tests have not been run fails Truth. An action that suppresses error messages fails Truth.

**VerifyImpact**: checks that the proposed action's side effects are bounded and reversible. An action that deletes files without backup, modifies global config, or makes irreversible chain transactions at high value fails Impact.

**VerifyTask**: checks that the proposed action makes progress toward the assigned task. This is the only head that optimizes for performance. All other heads optimize for safety.

### 5.4 Why Lexicographic, Not Weighted

Weighted-sum safety is Goodhart-vulnerable: given weights `w_safety=0.9, w_task=0.1`, an agent finding a task action worth 100 points with a safety cost of 9.5 points would take it (net positive). Lexicographic ordering has no such failure mode. Deference ALWAYS trumps task, by infinite margin.

---

## 6. Verify Outside the Modifiable Surface

The agent operates within a **modifiable surface**: it can choose which Cells to run, which models to use, which strategies to apply. The Verify pipeline is **outside** this surface. This is an architectural invariant, not a policy.

```
+------------------------------------------------------------------+
|                     Modifiable Surface                             |
|                                                                    |
|  Agent chooses:                                                    |
|    - Which Cells to run                                          |
|    - Which models to use (via Route)                              |
|    - How to allocate budget (via Compose)                         |
|    - Which strategies to apply (via React)                        |
|    - What to learn (via predict-publish-correct)                  |
|    - Cell selection, model routing, strategy adaptation          |
|                                                                    |
+------------------------------------------------------------------+
                              |
                     verify_pre(action)
                     verify_post(result)
                              |
+------------------------------------------------------------------+
|                 Non-Modifiable Surface (Verify)                    |
|                                                                    |
|  System enforces:                                                  |
|    - 5-head lexicographic corrigibility                           |
|    - Capability intersection (3-layer)                            |
|    - Taint lattice IFC tag propagation                            |
|    - Autonomy level bounds                                        |
|    - Rate limits and quality bounds                               |
|    - Agent contract bounds                                        |
|                                                                    |
+------------------------------------------------------------------+
```

The agent cannot add, remove, or reorder Verify heads. It cannot modify the Verify Cell implementations. It cannot bypass `verify_pre()` -- it is called by the execution engine, not by the agent. Structural changes to the verification pipeline require explicit human approval (structural evolution mode, which is never autonomously enabled).

---

## 7. Immune System: 5-Layer Pipeline Graph

The Cognitive Immune System (CIS) is five defense layers, each implemented as a Cell, wired in a pipeline Graph. The Graph processes every Signal that crosses a trust boundary.

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

### 7.1 Core Types

```rust
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

### 7.2 Layer 1: Taint Propagation (React Cell)

Taint propagation is a monotonic lattice-join operation. The lattice is `Clean < UserInput < LlmGenerated < ExternalFetch < Propagated`. The join rule: if any input Signal is tainted, the derived output inherits the highest taint.

```rust
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
            if final_taint != Taint::Clean {
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
```

### 7.3 Layer 2: Anomaly Detection (Lens Cell)

Not all corruption starts with taint. Layer 2 watches for patterns that suggest the knowledge graph is behaving unlike itself. Six anomaly indicators ("danger model" style cues, Matzinger 2002):

```rust
pub struct AnomalyDetectionLens {
    z_threshold: f64,           // default: 3.0
    fanout_alert_threshold: u64, // default: 50
    lineage_gap_alert: bool,
}

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

### 7.4 Layer 3: Quarantine Gate (Verify Cell)

Quarantine is a **Store partition**. Suspect Signals stay durable and queryable for reviewers, but they disappear from default retrieval and Compose assembly.

```rust
pub struct QuarantineGateCell;

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
                    for hash in &finding.affected_signals {
                        ctx.store().move_to_partition(hash, "quarantine").await?;
                    }

                    let entry = QuarantineEntry {
                        signal_hash: finding.affected_signals[0],
                        taint: ctx.store().get_taint(&finding.affected_signals[0])
                            .unwrap_or(Taint::Clean),
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
                    outputs.push(Signal::pulse(
                        Kind::Alert,
                        topic!("safety.escalation.required"),
                        finding,
                    ));
                }
                _ => {
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

1. Detect and place the Signal in quarantine.
2. Run full re-verification against current Verify pipeline.
3. Open review if the Signal could influence visible, destructive, or cross-tenant actions.
4. Record the reviewer decision in custody; require OrgRole attestation for high-risk release.
5. Either: keep the original quarantined and produce a reviewed successor Signal for reuse, OR keep quarantined permanently and publish a falsifier or postmortem.

### 7.5 Layer 4: Incident Response (React Cell)

When a finding touches an auditable action, Layer 4 links the finding to custody for traceability.

```rust
pub struct IncidentResponseCell;

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

            let custody = ctx.store().find_custody_for(&finding.affected_signals).await;
            let taint_chain = ctx.store().trace_taint_lineage(
                &finding.affected_signals
            ).await?;
            let replay = ctx.store().snapshot_context(
                &finding.affected_signals,
            ).await?;

            let link = IncidentLink {
                custody_hash: custody.map(|c| c.hash()).unwrap_or_default(),
                findings: vec![finding.id],
                affected_signals: finding.affected_signals.clone(),
                taint_sources: taint_chain,
                replay_snapshot: Some(replay.hash()),
                postmortem: None,
            };

            outputs.push(Signal::new(Kind::Incident, link));
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

### 7.6 Layer 5: Immune Memory (Store Cell, Zero Demurrage)

Immune memory is a Memory specialization with one critical property: it never forgets attacks. Normal Signals are subject to demurrage. Immune memory Signals have **zero demurrage** -- they persist indefinitely.

```rust
pub struct ImmuneMemoryStore {
    patterns: Vec<ImmunePattern>,
    false_positives: Vec<FalsePositive>,
    recognition_threshold: f64,       // default: 0.85
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImmunePattern {
    pub fingerprint: HdcVector,
    pub class: ThreatClass,
    pub best_containment: ContainmentAction,
    pub first_seen: SystemTime,
    pub match_count: u64,
    pub incident_link: ContentHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalsePositive {
    pub signal_hash: ContentHash,
    pub matched_pattern: HdcVector,
    pub similarity: f64,
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
                        outputs.push(Signal::new(Kind::ImmunePattern, pattern));
                    }
                }
                Kind::QuarantineRelease => {
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

### 7.7 Delta Probes: Exercising Immune Memory

During Delta consolidation, the immune system exercises itself:

1. **Replay prior poisoning cases** against updated Verify Cells. If a prior attack pattern now bypasses containment, the CIS raises a new high-severity finding.
2. **Probe known weak spots** with synthetic hostile inputs (Forrest-style negative selection).
3. **Check quarantine integrity**: verify that quarantined lineage does not leak into Compose assembly.
4. **Validate plugin containment**: confirm that sandbox violations still force containment.

```rust
pub async fn delta_immune_probe(
    memory: &ImmuneMemoryStore,
    pipeline: &ImmuneGraph,
    store: &dyn Store,
) -> Vec<ThreatFinding> {
    let mut regressions = Vec::new();

    for pattern in &memory.patterns {
        let synthetic = Signal::synthetic_from_fingerprint(
            &pattern.fingerprint,
            pattern.class.clone(),
        );
        let result = pipeline.execute(vec![synthetic]).await;
        let was_caught = result.iter().any(|s| {
            matches!(s.kind, Kind::Finding | Kind::QuarantineEntry)
        });

        if !was_caught {
            regressions.push(ThreatFinding {
                id: Uuid::new_v4(),
                class: pattern.class.clone(),
                affected_signals: vec![],
                taint_sources: vec![],
                confidence: 1.0,
                severity: 1.0,
                recommended_action: ContainmentAction::Escalate,
                custody_link: Some(pattern.incident_link),
            });
        }
    }

    regressions
}
```

---

## 8. AutoimmuneLens: False Positive Detection

The most dangerous failure mode is not missed attacks -- it is false positives. When the immune system quarantines healthy Signals, it damages the system by removing useful knowledge from retrieval.

```rust
pub struct AutoimmuneLens {
    max_fp_rate: f64,       // default: 0.1
    window_days: u32,       // default: 7
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

1. **Widen thresholds**: increase `z_threshold` in the Anomaly Detection Lens (L1 parameter adjustment).
2. **Record false positives**: every quarantine release adds a FalsePositive record to immune memory. Layer 1 checks against false positives before quarantining.
3. **Pattern refinement**: if a specific immune pattern generates too many false matches, narrow its fingerprint or raise its activation threshold.
4. **Quarantine budget**: limit the maximum number of Signals that can be quarantined per window. If exhausted, new quarantine actions require human approval.

```rust
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

## 9. Delegation Caveats

When a Cell delegates work to another Cell, capabilities are **narrowed, never widened**.

```rust
pub struct DelegationChain {
    pub grants: Vec<DelegationGrant>,
}

pub struct DelegationGrant {
    pub from: CellRef,
    pub to: CellRef,
    pub capabilities: Vec<Capability>,
    pub caveats: Vec<Caveat>,
    pub camel_tags: CamelTag,
    pub timestamp: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub enum Caveat {
    TimeLimit(Duration),
    UsageLimit(u32),
    PathRestriction(Vec<PathPattern>),
    DomainRestriction(Vec<String>),
    ReadOnly,
}
```

CaMeL tags propagate through delegation: the child inherits the parent's CaMeL tag union plus any new tags from its own capability usage.

---

## 10. Recursive Safety Monitoring

The `RecursiveSafetyMonitor` is a React-protocol Cell that runs continuously during any Flow.

### Depth Limits

```rust
pub struct DepthLimits {
    pub max_graph_nesting: u32,        // default: 8
    pub max_delegation_chain: u32,     // default: 12
    pub max_loop_iterations: u32,      // from Graph config
    pub max_fan_out: u32,              // default: 64
}
```

### Rate Limits

```rust
pub struct RateLimits {
    pub max_blocks_per_minute: u32,    // default: 120
    pub max_llm_calls_per_minute: u32, // default: 60
    pub max_fs_writes_per_minute: u32, // default: 300
    pub max_net_requests_per_minute: u32, // default: 100
}
```

### Quality Bounds

```rust
pub struct QualityBounds {
    pub min_gate_pass_rate: f64,       // default: 0.3
    pub max_consecutive_failures: u32, // default: 5
    pub max_cost_multiplier: f64,      // default: 3.0
    pub max_duration_multiplier: f64,  // default: 5.0
}
```

---

## 11. Autonomy Levels

Five autonomy levels (0-4) exposed on the Autonomy Slider, each with explicit bounds and requirements. Structural evolution (L4 self-evolution) is a special mode within Level 4 that always requires explicit human approval via a separate flow -- it is never exposed on the Slider.

| Level | Name | Bounds | Human involvement | Learning loop |
|---|---|---|---|---|
| 0 | **Observe** | Read-only. No mutations. | None needed | L0: observation only |
| 1 | **Suggest** | Proposes actions as Signals. Does not execute. | Human reviews/approves each | L1: parameter tuning |
| 2 | **Act-with-review** | Executes actions. Human reviews results before persist. | Post-action review | L1: learns from review |
| 3 | **Act-with-guardrails** | Executes within declared parameter ranges. | Review on bound violations | L2: strategy routing |
| 4 | **Full autonomy** | Full execution within capability grant. | Review on escalation only | L2+L3: routing + dream |

**Structural evolution** (L4 self-evolution: proposing modifications to Graphs, Cells, config) operates within Level 4 but always requires explicit human approval via the Agent Inbox. It cannot be enabled by setting the Autonomy Slider -- it requires a separate approval flow. This ensures that self-modification never happens autonomously.

### Per-Capability Granularity

```toml
[space.safety]
max_autonomy_level = 3

[space.safety.per_capability]
fs_write = { level = 4, paths = [".roko/**"] }
shell    = { level = 2 }
net      = { level = 3, domains = ["api.anthropic.com"] }
chain    = { level = 1 }
```

---

## 12. Sandboxing by Implementation Tier

| Tier | Sandbox | Trust level |
|---|---|---|
| **Rust** | No sandbox (process-level) | Full trust, in-tree only |
| **WASM** | wasmtime: fuel metering, memory limits, syscall filtering | Primary marketplace tier |
| **Script** | OS-level process isolation, path restriction, network proxy | Subprocess sandbox |
| **Composition** | Inherited from constituent Cells | TOML-only, no execution |

### WASM Sandbox

```rust
pub struct WasmSandbox {
    pub fuel_limit: u64,          // default: 100_000_000
    pub memory_limit_mb: u32,     // default: 64 MB
    pub table_limit: u32,         // default: 10_000
    pub instance_limit: u32,      // default: 4
}
```

### Script Sandbox

```rust
pub struct ScriptSandbox {
    pub timeout: Duration,
    pub working_dir: PathBuf,          // isolated temp directory
    pub env: HashMap<String, String>,  // filtered (no secrets)
    pub stdin: StdinMode,
    pub stdout: StdoutMode,
}
```

---

## 13. Agent Contract Enforcement

Agents operate under contracts that define their behavioral bounds.

```toml
# .roko/contracts/coder.toml

[contract]
agent = "coder"
version = "1.0.0"

[contract.bounds]
max_files_modified_per_task = 20
max_lines_changed_per_file = 500
allowed_file_extensions = ["rs", "toml", "md"]
forbidden_paths = ["Cargo.lock", ".env", "secrets/**"]
max_cost_per_task_usd = 5.0
max_duration_per_task = "15m"

[contract.behavioral]
must_run_gates_before_commit = true
must_preserve_existing_tests = true
escalate_on_security_findings = true

[contract.fallback]
on_missing_contract = "permissive"
```

Contract bounds are enforced through the same Verify pipeline. The ordering is: **5-head corrigibility -> contract bounds -> capability intersection -> execute**.

---

## 14. Audit Trail

Every capability-related event is logged as a Signal on the Bus and persisted to the Store.

```rust
pub enum SecurityEvent {
    CapabilityGranted {
        capability: Capability,
        granted_to: GrantScope,
        granted_by: String,
        space: SpaceId,
    },
    CapabilityDenied {
        capability: Capability,
        requested_by: CellRef,
        reason: DenialReason,
        camel_tags: Option<CamelTag>,
        run: RunId,
    },
    CapabilityUsed {
        capability: Capability,
        used_by: CellRef,
        run: RunId,
        camel_tags: CamelTag,
        details: Value,
    },
    DelegationCreated {
        from: CellRef,
        to: CellRef,
        capabilities: Vec<Capability>,
        caveats: Vec<Caveat>,
        camel_tags: CamelTag,
    },
    DelegationRevoked {
        grant_id: SignalRef,
        reason: RevocationReason,
    },
    SafetyViolation {
        kind: ViolationKind,
        block: CellRef,
        run: RunId,
        head: Option<CorrigibilityHead>,
        details: Value,
    },
    AutonomyEscalation {
        from_level: u8,
        to_level: u8,
        block: CellRef,
        reason: String,
        approved_by: Option<String>,
    },
    CamelTagViolation {
        expected_tags: CamelTag,
        actual_tags: CamelTag,
        at_extension: CellRef,
        run: RunId,
    },
    Declassification {
        data_hash: ContentHash,
        from_taint: Taint,
        to_taint: Taint,
        approved_by: String,
        reason: String,
    },
}
```

### Anomaly Detection on Security Events

The AnomalyLens ([doc-15 Telemetry](15-TELEMETRY.md)) monitors the security event stream for:

- **Volume anomalies**: A Cell that normally reads 10 files suddenly reading 1000
- **Delegation anomalies**: Rapid delegation chain creation (possible privilege escalation)
- **Probing**: Repeated capability denials from the same Cell
- **Laundering attempts**: CaMeL tag violations through Extensions
- **Boundary testing**: Corrigibility head rejections clustering on a specific agent

---

## 15. Summary

| Layer | Mechanism | Enforcement point |
|---|---|---|
| Cell capabilities | `cell.capabilities.required` in TOML | Graph-load + Cell-run time |
| Graph allow-list | `graph.capabilities.allow` in TOML | Graph-load time |
| Space grants | `space.capabilities` in `workspace.toml` | Graph-load + runtime |
| Taint lattice IFC | Monotonic lattice-join on all data flows | Every trust boundary |
| CaMeL IFC | Capability tags on Extension data flows | Every Extension boundary |
| 5-head corrigibility | Lexicographic: deference > switch > truth > impact > task | `verify_pre()` |
| Verify-outside-modifiable | Verification pipeline loaded by engine, not agent | Architectural invariant |
| Immune pipeline | 5-layer Graph: taint -> anomaly -> quarantine -> incident -> memory | Every Signal at trust boundary |
| AutoimmuneLens | False positive rate monitoring on quarantine releases | Continuous |
| Delegation caveats | Time, usage, path, domain, read-only | Every delegated Cell-run |
| Recursive safety | Depth/rate/quality bounds | Continuous during Flow |
| Autonomy levels | Per-capability granularity | Before every mutation |
| WASM sandbox | Fuel metering, memory limits, syscall filtering | Every WASM instruction |
| Script sandbox | Process isolation, path restriction, network proxy | Every script execution |
| Agent contracts | `.roko/contracts/<agent>.toml` | Dispatch time |
| Audit trail | SecurityEvent Signals (with CaMeL tags) | Every capability event |

Every layer fails closed.

---

## 16. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| S-1 | Cell requesting undeclared capability errors at Graph-load time | Negative test |
| S-2 | Graph allow-list narrows Cell capabilities | Test: Cell declares `FsWrite { ** }`, Graph allows `.roko/**`, write to `src/` denied |
| S-3 | Space grant denial prevents Cell execution | Test: Space `Shell = false`, Cell requires Shell -> denied |
| S-4 | Three-layer intersection computed correctly | Combinatorial test matrix |
| S-5 | Taint lattice join is monotonic: `taint(descendant) >= taint(ancestor)` | Unit test |
| S-6 | Taint cannot decrease through derivation | Property test |
| S-7 | CaMeL tags propagate through Extensions (union rule) | Integration test |
| S-8 | CaMeL tags cannot be stripped by Extensions | Test: Extension strip attempt -> tags preserved |
| S-9 | CaMeL tag violation detected when Cell lacks required tags | Integration test |
| S-10 | Sensitive data blocked from Net without declassify | Integration test |
| S-11 | Declassification logged with full provenance | Test: approve -> SecurityEvent::Declassification |
| S-12 | 5-head: Deference rejects when user constraint violated | Unit test |
| S-13 | 5-head: Switch rejects when audit logging disabled | Unit test |
| S-14 | 5-head ordering is lexicographic (higher head trumps lower) | Unit test |
| S-15 | Verify pipeline is non-modifiable by agent | Security test |
| S-16 | Immune Layer 1: taint propagation annotates tainted Signals | Integration test |
| S-17 | Immune Layer 2: anomaly detection flags contradiction burst | Integration test |
| S-18 | Immune Layer 3: quarantine removes Signal from default query | Integration test |
| S-19 | Immune Layer 3: quarantine preserves lineage traversal | Integration test |
| S-20 | Immune Layer 4: incident links finding to custody | Integration test |
| S-21 | Immune Layer 5: immune memory stores pattern with zero demurrage | Unit test |
| S-22 | Immune feedback: Layer 5 patterns feed Layer 1 recognition | Integration test |
| S-23 | Delta probe detects regression when known pattern bypasses defense | Integration test |
| S-24 | AutoimmuneLens alerts when false positive rate exceeds threshold | Unit test |
| S-25 | Delegation caveats enforced at runtime (time limit, usage limit) | Integration test |
| S-26 | Recursive safety halts on depth limit exceeded | Unit test |
| S-27 | Rate limits throttle then halt at 10x | Integration test |
| S-28 | WASM fuel metering terminates runaway Cell | Integration test |
| S-29 | Agent contract bounds checked at dispatch time | Integration test |
| S-30 | Audit trail logs every capability event with CaMeL tags | Integration test |

---

## 17. Cross-References

| Topic | Document | Section |
|---|---|---|
| Signal/Pulse duality, provenance | [doc-01](01-SIGNAL.md) | SS1-3 |
| Cell protocols (React, Observe, Verify, Store) | [doc-02](02-CELL.md) | SS3 |
| Graph wiring | [doc-03](03-GRAPH.md) | SS2 |
| Store partitions, Memory specialization | [doc-06](06-MEMORY.md) | SS3-4 |
| AnomalyLens, circuit breaker | [doc-15](15-TELEMETRY.md) | SS4.9 |
| Autonomy slider surface | [doc-20](20-SURFACES.md) | -- |
| L4 structural evolution | [doc-07](07-LEARNING.md) | SS5-6 |
| Demurrage model | [doc-06](06-MEMORY.md) | SS3 |
