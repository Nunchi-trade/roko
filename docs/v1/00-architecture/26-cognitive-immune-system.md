# Cognitive Immune System

> **Abstract:** Roko operates in adversarial environments: user input can contain prompt injection,
> external fetches can be wrong or hostile, plugin output can exceed its declared trust envelope,
> and imported history can carry stale or forged lineage. The Cognitive Immune System (CIS) is
> Roko's knowledge-integrity subsystem inside the broader safety spine. The safety spine handles
> authorization, sandboxing, attestation, custody, human checkpoints, and multi-tenant isolation
> across every layer. The CIS specializes in taint propagation, quarantine, anomaly detection,
> replay, and immune memory so corrupted knowledge does not silently become durable policy.
> This revision aligns the architecture story with
> [tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md)
> and uses the current vocabulary defined in
> [01-naming-and-glossary](./01-naming-and-glossary.md).

> **Implementation**: Specified

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [05-provenance-and-attestation](./05-provenance-and-attestation.md), [09-universal-cognitive-loop](./09-universal-cognitive-loop.md), [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md)
**Key sources**:
- Chen et al. 2024, NeurIPS (arXiv:2407.12784) - AgentPoison: Red-teaming LLM Agents via Memory Poisoning
- arXiv:2411.18948 (2024) - RevPRAG: Revealing Poisoning Attacks via Activation Analysis
- arXiv:2601.05504 (2025) - Memory Poisoning Attack and Defense on Memory-Based LLM Agents
- NIST AI 100-2e2025 - Adversarial Machine Learning: Taxonomy of Attacks and Mitigations
- Matzinger 2002, Science 296 - The Danger Model
- Forrest et al. 1994 - Self-Nonself Discrimination in Computer Security

---

## 1. Threat Model

### 1.1 Attack Surfaces

The CIS exists because knowledge corruption is usually indirect: hostile input enters through one
surface, mutates a durable Engram somewhere else, and only becomes visible when a later action is
about to cross a real-world boundary.

| Vector | Example | Why CIS Cares | Primary Response |
|---|---|---|---|
| Prompt injection | Tool output includes instructions that alter later tool use | Can taint composed prompts and derived outputs | Mark taint at ingestion, require stronger gates at ACT |
| Memory poisoning | False claim survives into durable Neuro state | Contaminates later retrieval and planning | Quarantine, replay, reviewer release only |
| Adversarial retrieval | Crafted Engrams score high despite low integrity | Pulls the Composer toward bad context | Contradiction and score-distribution checks |
| Plugin abuse | Third-party plugin output exceeds declared capability intent | Untrusted output can steer later actions | `ThirdPartyPlugin` taint, sandbox violation handling |
| Legacy import corruption | Imported archives contain stale or forged lineage | Pollutes trusted local state | `LegacyImport` taint plus quarantine-on-ingest |
| Cross-tenant contamination | Shared deployment leaks a tenant's data into another tenant's query | Breaks isolation and audit boundaries | Immediate quarantine and escalation |

The CIS assumes the same trust boundary as the REF32 safety spine: role authorization and
sandboxing are enforced elsewhere, but their outcomes feed CIS decisions. A plugin that violates
its sandbox is both an isolation failure and a knowledge-integrity incident.

### 1.2 Scope Boundary: Safety Spine vs. CIS

The defensive story is clearer when the responsibilities are split explicitly:

| Concern | Primary owner | CIS role |
|---|---|---|
| Who may act | Safety spine authorization | Consume role and approval outcomes as evidence |
| Where untrusted code may run | Safety spine sandbox tiers | Treat violations as high-severity findings |
| What entered the system | CIS taint model | Label, propagate, and expose risk to gates |
| What must be isolated | CIS quarantine plus Substrate filtering | Remove suspect Engrams from default query paths |
| What happened and why | Custody and attestation | Attach findings, taint sources, and replay evidence |
| How the system learns after failure | CIS immune memory plus policy updates | Turn incidents into future defenses |

The CIS is therefore not a parallel safety subsystem. It is the knowledge-integrity branch of the
same safety spine described in
[tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md).

---

## 2. Architecture

### 2.1 Five Defense Layers

```
┌───────────────────────────────────────────────────────────┐
│ Layer 5: IMMUNE MEMORY + DELTA PROBES                    │
│ Remember prior attacks; replay and probe weak points     │
├───────────────────────────────────────────────────────────┤
│ Layer 4: CUSTODY-LINKED INCIDENT RESPONSE                │
│ Tie findings to Custody, replay, and postmortems         │
├───────────────────────────────────────────────────────────┤
│ Layer 3: QUARANTINE                                      │
│ Remove suspect Engrams from default retrieval paths      │
├───────────────────────────────────────────────────────────┤
│ Layer 2: ANOMALY DETECTION                               │
│ Detect contradiction clusters, fan-out, and drift        │
├───────────────────────────────────────────────────────────┤
│ Layer 1: TAINT PROPAGATION                               │
│ Track untrusted lineage through Engrams and Pulses       │
└───────────────────────────────────────────────────────────┘
```

Layer 1 is the fast path. Layers 2 and 3 contain suspect knowledge before it reaches action.
Layers 4 and 5 make the system auditable and self-improving.

### 2.2 Core Types

The CIS extends the safety spine with a small set of knowledge-integrity records:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Taint {
    None,
    UserInput,
    ExternalFetch(Source),
    ThirdPartyPlugin(PluginId),
    LegacyImport,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatFinding {
    pub id: Uuid,
    pub class: ThreatClass,
    pub affected_engrams: Vec<ContentHash>,
    pub taint_sources: Vec<ContentHash>,
    pub confidence: f64,
    pub severity: f64,
    pub recommended_action: ContainmentAction,
    pub custody: Option<ContentHash>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ContainmentAction {
    Monitor,
    Quarantine,
    Reverify,
    Escalate,
    DisablePlugin,
}
```

`ThreatFinding` is not a substitute for `Custody`. It points to the relevant custody record when
the incident touched an auditable action, and otherwise serves as the local detection record that
later incident handling can cite.

---

## 3. Layer 1: Taint Propagation Tracking

### 3.1 Canonical Taint Model

REF32 makes the taint rule explicit: untrusted origin is durable metadata, not a temporary score
penalty. Every Engram that enters through a trust boundary carries a first-class `Taint`:

```rust
pub struct Provenance {
    pub author: AuthorId,
    pub trust: f32,
    pub taint: Taint,
    pub session: Option<SessionId>,
}
```

Typical mappings are:

| Ingress path | Initial taint |
|---|---|
| User paste, upload, ad hoc instruction | `UserInput` |
| HTTP fetch, remote API, scraped page | `ExternalFetch(source)` |
| Tier-3, Tier-4, or Tier-5 plugin output | `ThirdPartyPlugin(plugin_id)` |
| Imported archive from another deployment | `LegacyImport` |
| Locally reviewed durable record with no untrusted ancestor | `None` |

This keeps the CIS aligned with the safety chapter and with
[05-provenance-and-attestation](./05-provenance-and-attestation.md).

### 3.2 Propagation Law

The propagation rule is intentionally simple:

1. If any Engram or Pulse consumed during composition is tainted, the derived output is tainted.
2. Pulses carry taint metadata while in motion on the Bus; when a Pulse graduates to an Engram,
   that taint becomes durable provenance.
3. Taint is monotonic. It propagates forward through lineage and does not silently disappear.
4. Human review can approve later use, but approval is recorded through `Custody` or attestation;
   it does not rewrite ancestor provenance.

The core operation is therefore set-preserving lineage tracking rather than confidence decay:

```rust
pub fn derive_taint(inputs: &[Taint]) -> Taint {
    inputs.iter().find(|t| !matches!(t, Taint::None)).cloned().unwrap_or(Taint::None)
}
```

Implementations may retain richer internal annotations, but the public architectural contract is
"once tainted, always traceably tainted unless a human explicitly signs off on the downstream use."

### 3.3 Gate and Action Consequences

Taint is inert until a decision boundary reads it. The CIS provides evidence; the safety spine
decides consequences.

| Situation | Default CIS output | Typical safety response |
|---|---|---|
| Tainted context used for drafting only | Warning marker on composed Engram | Allow with visible taint annotation |
| Tainted tool output proposes file mutation | `ThreatFinding(TaintCascade)` | Confirmation or denial depending on role |
| Tainted address or credential reaches a high-risk sink | High-severity finding | Escalate or refuse |
| Repeated plugin-tainted outputs fail gates | Sandbox-related finding | Disable plugin and quarantine descendants |

The CIS also publishes Pulses such as `safety.taint.detected`, `safety.quarantine.entered`, and
`plugin.violation` so StateHub and auditor surfaces see the same truth the gates saw.

---

## 4. Layer 2: Anomaly Detection

### 4.1 Monitored Indicators

Not all corruption starts with taint. The CIS also watches for patterns that suggest the
knowledge graph is behaving unlike itself:

| Indicator | Example | Likely class |
|---|---|---|
| Contradiction burst | Many new claims suddenly conflict with established Engrams | `MemoryPoisoning` |
| Score spike without support | Retrieval rank rises but gates and lineage do not justify it | `AdversarialRetrieval` |
| Taint fan-out burst | One import suddenly contaminates a large lineage region | `TaintCascade` |
| Sandbox violation cluster | One plugin repeatedly exceeds permission envelope | `SandboxViolation` |
| Tenant-boundary mismatch | Query path mixes two tenant prefixes | `CrossTenantLeakage` |
| Lineage gap | Durable record cites missing or unverifiable ancestors | `LineageMismatch` |

These indicators are "danger model" style cues: the CIS responds when the system shows signs of
damage, not only when the content is foreign.

### 4.2 Detection Outcomes

An anomaly does not immediately make new truth. It creates evidence that must be handled:

1. Emit a `ThreatFinding` with severity and recommended containment.
2. Publish a matching safety Pulse for dashboards and replay.
3. Raise the quarantine candidate set if the finding touches durable Engrams.
4. If an auditable action is involved, attach the finding to the action's `Custody` record.

This keeps the CIS from becoming an uncontrolled policy engine. It finds, classifies, and routes.
Human approval, role policy, and attestation remain in the safety spine.

---

## 5. Layer 3: Quarantine

### 5.1 Quarantine Partition

Quarantine is a first-class containment boundary inside the Substrate. Suspect Engrams stay
durable and queryable for reviewers, but they disappear from default retrieval and composition.

```rust
pub struct QuarantineEntry {
    pub engram_hash: ContentHash,
    pub taint: Taint,
    pub reason: ThreatClass,
    pub placed_at: SystemTime,
    pub custody: Option<ContentHash>,
    pub review_required: bool,
    pub reviewer_release: Option<PrincipalId>,
}
```

Operational rules:

1. `Substrate.query()` excludes quarantine by default.
2. Composer assembly excludes quarantine unless the caller has an explicit review scope.
3. Quarantine status is visible on StateHub and on `safety.quarantine.*` Topics.
4. Quarantined records remain part of lineage and replay; they are hidden from normal use, not
   erased from history.

### 5.2 Resolution Workflow

REF32's one-way taint rule changes quarantine semantics. The system may release use of a record,
but it never pretends the contamination never existed.

1. Detect and place the Engram in quarantine.
2. Run full reverification against current gates and any domain-specific checks.
3. Open review if the Engram could influence visible, destructive, or cross-tenant actions.
4. Record the reviewer decision in `Custody`; require `OrgRole` attestation for high-risk release.
5. Either:
   - keep the original Engram quarantined and produce a reviewed successor Engram for reuse, or
   - keep the Engram quarantined permanently and publish a falsifier or postmortem.

Quarantine is therefore both a storage partition and a workflow checkpoint.

---

## 6. Layer 4: Custody-Linked Incident Response

### 6.1 Joining Threat Findings to Custody

REF32 requires an operator to answer "who did what, with what authorization, and with what
consequence?" The CIS contributes the knowledge-integrity side of that answer by linking findings
to custody whenever the chain crosses an auditable action.

```rust
pub struct IncidentLink {
    pub custody: ContentHash,
    pub findings: Vec<Uuid>,
    pub affected_engrams: Vec<ContentHash>,
    pub taint_sources: Vec<ContentHash>,
    pub replay_snapshot: Option<ContentHash>,
    pub postmortem: Option<ContentHash>,
}
```

The linked custody record tells the auditor:

| Question | Source of truth |
|---|---|
| Who initiated the action | `Custody.principal` |
| Why the system thought the action was reasonable | `why_heuristics` and `why_claims` |
| Which gates passed or failed | `gates_passed` plus replay |
| Whether tainted inputs were present | CIS taint sources and quarantine entries |
| Whether a human approved release | `authorized` plus attestation level |

### 6.2 Incident Workflow

The CIS follows the same incident sequence as the REF32 safety spine:

1. Identify the action's custody record when one exists.
2. Walk backward through contributing Engrams and taint sources.
3. Reconstruct the exact gate, heuristic, and context state through replay.
4. Publish an incident Engram or postmortem linked to the custody chain.
5. Update the relevant defense:
   - tighten a gate,
   - lower a plugin's permissions,
   - expand quarantine rules,
   - demote or falsify the offending knowledge.

This is the architectural reason custody belongs in the CIS story: detection without replayable
accountability is only monitoring.

---

## 7. Layer 5: Immune Memory

### 7.1 Remembering Successful and Failed Defenses

Immune memory stores reusable patterns from prior findings:

| Stored artifact | Why it matters |
|---|---|
| HDC fingerprint of known poisoning pattern | Fast similarity lookup during future intake |
| Taint source and fan-out shape | Recognize repeated propagation geometry |
| Best containment action | Reuse the defense that worked last time |
| False-positive record | Avoid over-quarantining benign material |
| Postmortem or custody link | Keep learning grounded in auditable history |

This makes the CIS cumulative: each resolved incident should make the next similar incident
cheaper to detect and contain.

### 7.2 Delta Probes and Replay

Delta-speed work in Dreams exercises the immune memory:

1. Replay prior poisoning cases against updated gates.
2. Probe known weak spots with synthetic hostile inputs.
3. Check whether quarantined lineage still leaks into Composer assembly.
4. Verify that plugin sandbox violations still force containment.

If a prior attack pattern now bypasses containment, the CIS raises a new high-severity finding and
reopens the relevant policy surface.

---

## 8. Daimon Integration: Caution Without Policy Override

The CIS may bias the Daimon toward a more cautious operating posture, but it does not override the
safety spine's authorization decisions. High recent finding severity should lower willingness to
take autonomous action, increase confirmation pressure, and bias routing toward stricter gates.

That separation matters:

1. The Daimon can alter posture.
2. The safety spine still approves or denies.
3. Custody still records who approved what.

The cognitive immune response is therefore advisory for behavior and authoritative for
knowledge-integrity metadata, not a replacement for role policy.

---

## 9. Configuration

```toml
[immune]
enabled = true

[immune.taint]
propagate_through_pulses = true
require_human_signoff_for_release = true

[immune.anomaly]
z_threshold = 3.0
fanout_alert_threshold = 50
lineage_gap_alert = true

[immune.quarantine]
default_partition = "quarantine"
hide_from_query = true
hide_from_compose = true
require_org_attestation_for_high_risk_release = true

[immune.incident]
link_to_custody = true
publish_topics = ["safety.taint.detected", "safety.quarantine.entered", "safety.incident.opened"]

[immune.memory]
recognition_threshold = 0.85
store_false_positives = true
replay_on_delta = true
```

The important configuration boundary is not threshold tuning. It is whether release, replay, and
attestation requirements stay consistent with the safety spine.

---

## 10. Integration Wiring

### 10.1 Into the Seven-Step Loop

| Loop step | CIS integration |
|---|---|
| 1. SENSE | Filter quarantined Engrams from default queries; attach taint metadata to incoming Pulses |
| 2. ASSESS | Run contradiction, fan-out, and lineage checks; classify findings |
| 3. COMPOSE | Exclude quarantine by default; surface taint annotations in composed context |
| 4. ACT | Gates read taint before tool use, egress, signing, or other high-risk actions |
| 5. VERIFY | Gate verdicts and plugin violations feed the anomaly detector and incident links |
| 6. PERSIST / BROADCAST | Persist findings, quarantine entries, custody links, and attestation state; publish `safety.*` Pulses |
| 7. REACT | Policy tightens thresholds, disables plugins, or opens reviewer work based on recent findings |

The CIS is therefore not an extra loop step. It injects into the existing seven-step loop in the
same way REF32 places taint, custody, and attestation into the broader safety story.

### 10.2 Target-State Components

| Component | CIS responsibility |
|---|---|
| `roko-core` | Durable `Taint` and provenance shape on Engrams |
| `roko-agent` | Gate-time checks before risky actions and plugin calls |
| `roko-neuro` | Quarantine-aware query and lineage traversal |
| `roko-daimon` | Caution bias from recent finding severity |
| `roko-dreams` | Delta replay and adversarial probes |
| `roko-chain` | Optional witness path for high-assurance custody evidence |

---

## 11. Test Criteria

| Test | What it validates | Type |
|---|---|---|
| `test_taint_propagates_from_pulse_to_engram` | Graduated durable records keep the Pulse taint | Unit |
| `test_taint_never_clears_without_review` | Normal derivation cannot erase taint | Unit |
| `test_quarantine_hidden_from_default_query` | Suspect Engrams stay out of normal retrieval | Integration |
| `test_compose_refuses_quarantine_without_scope` | Composer cannot silently use quarantined lineage | Integration |
| `test_plugin_violation_opens_containment` | Sandbox violation produces containment and review work | Integration |
| `test_incident_links_back_to_custody` | High-risk incident can be reconstructed from custody plus replay | Integration |
| `test_high_risk_release_requires_attestation` | Reviewer release for risky material requires the configured attestation level | Integration |
| `test_cross_tenant_mix_forces_escalation` | Tenant-boundary mismatch never degrades to a warning only | Integration |
| `test_delta_replay_reopens_regression` | A broken defense in replay raises a fresh finding | Integration |

---

## 12. Theoretical Foundations

### 12.1 Biological Analogy

| Biological idea | CIS analogue |
|---|---|
| Innate immunity | Immediate tainting and quarantine |
| Adaptive immunity | HDC-backed immune memory and replay |
| Tissue damage response | Contradiction and lineage-integrity indicators |
| Memory cells | Stored postmortems, false positives, and proven defenses |

### 12.2 Negative Selection

Forrest-style negative selection appears in Delta replay: the system keeps generating hostile
cases that should not survive composition, verification, or release. If one does, the CIS found a
gap before production drift turned it into an operator-visible failure.

### 12.3 NIST Alignment

| NIST category | CIS defense |
|---|---|
| Data poisoning | Taint propagation, anomaly detection, quarantine |
| Evasion of safeguards | Replay and adversarial probes against gates |
| Supply-chain corruption | Plugin sandbox findings plus attestation and custody links |
| Cross-domain contamination | Tenant-aware quarantine and escalation |

The CIS does not solve host compromise or physical access. Those remain outside the architecture
scope, consistent with the threat model in REF32.

---

## Cross-References

- [tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md) - canonical safety spine refinement propagated here
- [01-naming-and-glossary](./01-naming-and-glossary.md) - current terminology for Engram, Pulse, Bus, Topic, TypedContext, and Custody
- [05-provenance-and-attestation](./05-provenance-and-attestation.md) - durable provenance, attestation levels, and custody-linked auditability
- [09-universal-cognitive-loop](./09-universal-cognitive-loop.md) - seven-step loop that CIS injects into rather than extending
- [Topic 04: Verification](../04-verification/INDEX.md) - gate pipeline that consumes taint and emits verdicts
- [Topic 06: Neuro](../06-neuro/INDEX.md) - durable store and lineage graph protected by quarantine
- [Topic 09: Daimon](../09-daimon/INDEX.md) - cautious behavior modulation based on recent finding severity
- [Topic 10: Dreams](../10-dreams/INDEX.md) - Delta replay and adversarial probes
- [Topic 11: Safety](../11-safety/INDEX.md) - broader safety spine: authorization, sandbox, secrets, and multi-tenant controls
