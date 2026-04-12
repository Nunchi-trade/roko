# Forensic AI: Causal Replay and Regulatory Pre-Compliance

> **Layer**: L3 Harness (audit and replay), L4 Orchestration (cross-agent accountability)
>
> **Crate**: Cross-cutting: `roko-core` (Engram lineage), `roko-fs` (persistent audit log), `roko-gate` (Gate verdict history), target: `roko-forensic` (dedicated replay engine)
>
> **Synapse traits**: All six traits participate — the replay reconstructs the full Synapse Loop for any past action
>
> **Prerequisites**: [02-audit-chain.md](02-audit-chain.md), [12-witness-dag.md](12-witness-dag.md)

---

## Overview

When an agent takes an action that causes harm — a bad trade, a broken deployment, a data breach — Roko can answer the question that no other agent framework can answer:

> *"Why did the agent do this, what information led to this decision, and who is accountable?"*

This capability is called **Forensic AI**: content-addressed causal replay that reconstructs the complete decision context for any past agent action. It is not a debugging feature or a post-mortem tool — it is a **regulatory pre-compliance** capability that transforms agent governance from reactive (investigate after harm) to proactive (prove compliance continuously).

The capability is novel because it exploits a structural property of the Roko architecture: every piece of information is an Engram with a content-addressed hash and lineage chain. The entire Synapse Loop is auditable by construction. No other agent framework has this property because no other framework makes content-addressing and lineage tracking mandatory at the kernel level.

---

## Causal Replay: How It Works

### The Replay Process

Take any agent action — a tool call, a trade execution, a file modification, a knowledge posting — and replay the exact decision context:

**Step 1: Identify the action Engram.** Every action produces an Engram stored in the Substrate (via `roko-fs` FileSubstrate or the Witness DAG). The Engram's `id` (ContentHash) uniquely identifies the action.

**Step 2: Reconstruct the Substrate state at the time.** Query all Engrams with `created_at_ms` before the action's timestamp. This reconstructs what the agent knew at the time of the decision. The FileSubstrate's JSONL format supports temporal queries by scanning entries with timestamp filters.

**Step 3: Reconstruct the Scorer outputs.** Which Scorer implementations were active? What scores did they compute for each Engram? The scores are persisted as metadata on the Engrams (the 7-axis Score: confidence, novelty, utility, reputation, precision, salience, coherence).

**Step 4: Reconstruct the Router selection.** Which Router selected which candidate Engram, with what confidence? Router decisions are logged as Engrams in the audit chain, including the rejected alternatives and their scores.

**Step 5: Reconstruct the Composer output.** Which Composer assembled the context window? Under what budget constraints? Which Engrams were included, which were excluded, and why? The VCG Attention Auction (see `refactoring-prd/09-innovations.md`) records bids and allocations.

**Step 6: Reconstruct the Gate verdict.** Which Gate verified the output? What was the Verdict (Pass, Fail, Skip)? What was the confidence score? Gate verdicts are persisted in `.roko/learn/gate-thresholds.json` and as Engrams in the audit chain.

**Step 7: Reconstruct the Policy decisions.** Which Policy implementations fired? What Engrams did they emit? Policy decisions (permit, deny, modify, log) are recorded by the ToolDispatcher's `emit_audit()` function.

### Cryptographic Verifiability

Every step in the replay is **cryptographically verifiable**:

- Each Engram's `id` is `BLAKE3(kind + body + author + tags)` — if any field has been modified, the hash won't match
- The `lineage: Vec<ContentHash>` field on each Engram records its parent Engrams — the audit DAG is tamper-evident
- The `provenance: Provenance` field records the author, model fingerprint, and taint chain — attribution is non-repudiable
- The optional `attestation: Option<Attestation>` field carries cryptographic proofs of origin

If the Witness DAG (see [12-witness-dag.md](12-witness-dag.md)) is enabled, the replay becomes even richer: five vertex types (Observation, Prediction, Decision, Resolution, NeuroEntry) provide fine-grained cognitive provenance, and BLAKE3 commitment hashes verify the entire reasoning chain.

### Replay as an Engram Stream

The replay itself is an Engram — a `kind: Replay` Engram whose body contains the reconstructed decision context. This replay Engram has lineage pointing to all the Engrams it reconstructed, creating a meta-level audit trail: the replay of the replay is also verifiable.

```rust
/// A forensic replay of a past agent action.
pub struct ForensicReplay {
    /// The action being replayed.
    pub action: ContentHash,
    /// Timestamp of the original action.
    pub action_timestamp_ms: i64,
    /// Engrams that were in the Substrate at the time.
    pub substrate_state: Vec<ContentHash>,
    /// Scores computed by Scorers for each Engram.
    pub scorer_outputs: Vec<(ContentHash, Score)>,
    /// Router selection: which Engram was chosen, from which candidates.
    pub router_selection: RouterDecision,
    /// Composer output: the assembled context under budget.
    pub composer_output: ComposerContext,
    /// Gate verdict for the action output.
    pub gate_verdict: Verdict,
    /// Policy decisions that applied.
    pub policy_decisions: Vec<PolicyDecision>,
    /// The replay itself is content-addressed.
    pub replay_hash: ContentHash,
}
```

---

## Regulatory Pre-Compliance

Roko's Forensic AI capability maps directly to specific regulatory requirements. The table below shows how each regulation's requirements are natively satisfied by Roko's architecture:

### EU AI Act

| Article | Requirement | Roko's Native Capability |
|---|---|---|
| Article 14 | Human oversight mechanisms | Cognitive Signals (SIGPAUSE, SIGCONTEXT) + Gate architecture (see [14-cognitive-kernel-safety.md](14-cognitive-kernel-safety.md)) |
| Article 11 | Technical documentation and logging | Complete Engram lineage with content-addressed provenance |
| Article 12 | Record keeping | FileSubstrate JSONL audit log + Witness DAG SQLite |
| Article 13 | Transparency and information to users | Forensic replay: reconstructable decision context |
| FRIA | Fundamental rights impact assessment | Pre-deployment simulation through synthetic scenarios using the Dreams engine |

**Key mapping.** Article 14 requires that high-risk AI systems "be designed and developed in such a way as to be effectively overseen by natural persons." Roko's Cognitive Signals (Pause, Escalate, Cooldown, InjectContext) provide exactly this: a human operator can intervene in an agent's reasoning at any point without destroying state. The Gate architecture provides automated oversight — every action is verified before execution.

### SEC/CFTC (Financial Regulation)

| Requirement | Roko's Native Capability |
|---|---|
| Trading decision reconstruction (MiFID II) | Complete Engram lineage from market data → analysis → trade decision |
| Order audit trail (Rule 17a-4) | Content-addressed provenance chain with timestamps |
| Best execution documentation | Router selection logs showing why this execution venue was chosen |
| Risk management documentation | Adaptive risk system verdicts (see [09-adaptive-risk.md](09-adaptive-risk.md)) persisted as Engrams |
| Market manipulation detection | Temporal logic monitoring (see [11-temporal-logic.md](11-temporal-logic.md)) + MEV detection (see [10-mev-protection.md](10-mev-protection.md)) |

**Key mapping.** SEC Rule 17a-4 requires broker-dealers to preserve records for 3-6 years with tamper-evident storage. Roko's content-addressed Engram chain satisfies this: each Engram's hash commits to its content, and the lineage chain makes tampering detectable. On-chain anchoring of DAG root hashes (see [12-witness-dag.md](12-witness-dag.md)) provides non-repudiable timestamps that survive local storage manipulation.

### HIPAA (Healthcare)

| Requirement | Roko's Native Capability |
|---|---|
| Audit trail for clinical decisions | Content-addressed provenance chain |
| Access controls (who saw what PHI) | Cognitive Namespaces with ACL (see [14-cognitive-kernel-safety.md](14-cognitive-kernel-safety.md)) |
| Integrity controls (data not altered) | BLAKE3 content hashes + commitment hashes |
| Transmission security | TLS for network communications + NetworkPolicy (see [06-sandboxing.md](06-sandboxing.md)) |
| Breach notification evidence | Forensic replay reconstructs exactly what data was accessed |

**Key mapping.** HIPAA's Security Rule requires that covered entities "implement hardware, software, and/or procedural mechanisms that record and examine activity in information systems that contain or use electronic protected health information." Roko's audit chain and Forensic AI replay satisfy this requirement at the architectural level — it is not an add-on feature but a structural property of the system.

### SOX (Sarbanes-Oxley)

| Requirement | Roko's Native Capability |
|---|---|
| Internal controls over financial reporting | Gate verdict history (tamper-proof) |
| Audit trail of control activities | ToolDispatcher `emit_audit()` at every pipeline stage |
| Segregation of duties | Role-based ToolPermission system (see [04-permits-allowlists.md](04-permits-allowlists.md)) |
| Management assessment of controls | Dashboard with Gate pass rates, trend analysis |

### GDPR (General Data Protection Regulation)

| Requirement | Roko's Native Capability |
|---|---|
| Right to explanation (Article 22) | Forensic replay: reconstructable decision context for any automated decision |
| Purpose limitation (Article 5) | Cognitive Namespace channels with kind filtering |
| Data minimization (Article 5) | Composer budget constraints limit context to relevant data |
| Right to erasure (Article 17) | Engram decay (HalfLife, TTL, Ebbinghaus) enables controlled data expiry |
| Processing records (Article 30) | FileSubstrate JSONL + Witness DAG = complete processing log |

**Key mapping.** GDPR Article 22 gives individuals the right "not to be subject to a decision based solely on automated processing" unless appropriate safeguards exist, including "the right to obtain an explanation of the decision reached." Roko's Forensic AI replay provides exactly this explanation — not a natural-language summary, but a cryptographically verifiable reconstruction of the complete decision context.

---

## Pre-Certified Agent Templates

Build agent configurations for specific regulatory regimes with compliance encoded in Policy traits:

### SEC-Compliant Trading Agent

```toml
# roko.toml section for SEC compliance
[safety.policies]
# MiFID II Policy automatically captures decision factors
mifid2_decision_logging = true
# Order audit trail retention (Rule 17a-4: 6 years)
audit_retention_days = 2190
# Best execution documentation
best_execution_logging = true
# Position limit enforcement
position_limit_policy = "strict"

[safety.gates]
# Pre-trade risk checks
pre_trade_risk_gate = true
# Post-trade compliance verification
post_trade_compliance_gate = true
```

### HIPAA-Compliant Clinical Agent

```toml
# roko.toml section for HIPAA compliance
[safety.policies]
# PHI-aware Gate prevents data leakage
phi_scrubbing = true
# Minimum necessary standard
minimum_necessary_policy = true
# Access logging for all PHI interactions
phi_access_logging = true

[safety.namespaces]
# Isolate PHI in a separate namespace
phi_namespace = { readers = ["clinician"], writers = ["clinician"], audit = true }
# Research namespace with de-identification gate
research_namespace = { readers = ["researcher"], writers = ["researcher"], deidentification_gate = true }
```

### GDPR-Compliant Data Agent

```toml
# roko.toml section for GDPR compliance
[safety.policies]
# Purpose-limitation Policy enforces consent boundaries
purpose_limitation = true
# Right to erasure: Engram TTL enforcement
erasure_policy = { default_ttl_days = 730, consent_categories = ["marketing", "analytics", "operational"] }
# Processing records (Article 30)
processing_records = true

[safety.gates]
# Consent verification gate
consent_gate = true
# Data minimization gate
minimization_gate = true
```

**The certification moat.** Once a regulator blesses a specific agent configuration, switching costs become astronomical. Any competing framework would need to replicate not just the compliance features but the entire content-addressed audit infrastructure that makes them trustworthy. This is Roko's structural moat for enterprise adoption.

---

## Enterprise Value Proposition

**Cost of non-compliance:**
- EU AI Act fines: up to 7% of global annual turnover
- SEC/CFTC fines: $10M-$1B per violation
- HIPAA fines: $100-$50,000 per violation, up to $1.5M per year
- GDPR fines: up to 4% of global annual turnover or 20M EUR

**Cost of Roko compliance:**
- Marginal: Forensic AI is a **structural property** of the architecture, not an add-on module
- The audit chain, content-addressing, and lineage tracking exist for operational reasons (debugging, learning, verification)
- Regulatory compliance is a free byproduct of good engineering

**Enterprise pricing target:** $100-500K/month per regulated enterprise. A single compliance failure costs more than years of Roko licensing. Roko is insurance that also improves agent performance.

---

## Causal Replay Engine Architecture

The Forensic AI capability requires a dedicated replay engine that can efficiently reconstruct past states:

### Temporal Query on FileSubstrate

```rust
/// Query the FileSubstrate for all Engrams that existed at a given timestamp.
/// This reconstructs the Substrate state at the time of a past action.
pub fn temporal_query(
    substrate: &FileSubstrate,
    timestamp_ms: i64,
) -> Vec<Signal> {
    // Note: Signal will be renamed to Engram in Tier 0D
    substrate
        .query_all()
        .filter(|engram| engram.created_at_ms <= timestamp_ms)
        .filter(|engram| !engram.is_expired_at(timestamp_ms))
        .collect()
}
```

### Replay Pipeline

```rust
/// Reconstruct the complete decision context for a past action.
pub async fn replay(
    action_hash: &ContentHash,
    substrate: &FileSubstrate,
    witness_dag: Option<&WitnessDAG>,
) -> Result<ForensicReplay> {
    // 1. Find the action Engram
    let action = substrate.get(action_hash)?;
    let timestamp = action.created_at_ms;

    // 2. Reconstruct Substrate state at the time
    let state = temporal_query(substrate, timestamp);

    // 3. Reconstruct scores (stored as metadata)
    let scores: Vec<(ContentHash, Score)> = state
        .iter()
        .map(|e| (e.id.clone(), e.score.clone()))
        .collect();

    // 4. Find Router decision from audit chain
    let router_decision = find_router_decision(substrate, action_hash)?;

    // 5. Find Composer output from audit chain
    let composer_output = find_composer_output(substrate, action_hash)?;

    // 6. Find Gate verdict from audit chain
    let gate_verdict = find_gate_verdict(substrate, action_hash)?;

    // 7. Find Policy decisions from audit chain
    let policy_decisions = find_policy_decisions(substrate, action_hash)?;

    // 8. If Witness DAG available, enrich with cognitive provenance
    let dag_provenance = witness_dag.map(|dag| {
        dag.provenance(&action_hash.into())
    });

    // 9. Construct replay Engram
    let replay = ForensicReplay {
        action: action_hash.clone(),
        action_timestamp_ms: timestamp,
        substrate_state: state.iter().map(|e| e.id.clone()).collect(),
        scorer_outputs: scores,
        router_selection: router_decision,
        composer_output,
        gate_verdict,
        policy_decisions,
        replay_hash: ContentHash::compute(&replay_body),
    };

    Ok(replay)
}
```

---

## Comparison with Existing Agent Observability

| Feature | Typical Agent Framework | Roko Forensic AI |
|---|---|---|
| Logging | Text logs, unstructured | Content-addressed Engrams with lineage |
| Audit trail | Optional, bolt-on | Mandatory, structural |
| Decision reconstruction | Not possible | Full replay of Synapse Loop |
| Tamper evidence | None | BLAKE3 hashes + Merkle chain |
| Regulatory mapping | Custom, expensive | Pre-built for EU AI Act, SEC, HIPAA, SOX, GDPR |
| ZK proofs | Not available | Strategy auditing without revelation |
| Cross-agent accountability | Not supported | Witness DAG + Collective trust |

---

## Implementation Status

| Component | Status | Location |
|---|---|---|
| Engram content-addressing (BLAKE3) | Built | `roko-core/src/signal.rs` |
| Engram lineage tracking | Built | `roko-core/src/signal.rs` `lineage` field |
| FileSubstrate (JSONL persistence) | Built | `roko-fs/` |
| ToolDispatcher audit emissions | Built | `roko-agent/src/dispatcher/mod.rs` `emit_audit()` |
| Gate verdict persistence | Built | `.roko/learn/gate-thresholds.json` |
| Episode logging | Built | `.roko/episodes.jsonl` via orchestrate.rs |
| Temporal query on FileSubstrate | Design only | Target: Tier 3 |
| Dedicated replay engine | Design only | Target: Tier 3 |
| Witness DAG integration | Design only | Target: Tier 3 |
| ZK proof generation | Design only | Target: Tier 4 |
| On-chain anchoring | Design only | Target: Tier 4 |
| Pre-certified agent templates | Design only | Target: Tier 5 |

---

## Academic References

| Paper | Contribution |
|---|---|
| Sumers et al. (2023, arXiv:2309.02427) | CoALA cognitive architecture — the 9-step loop that Forensic AI replays |
| Lee et al. (2026, arXiv:2603.28052) | Meta-Harness — harness optimization that Forensic AI makes auditable |
| Merkle (1987) | Merkle tree — foundation of tamper-evident audit chains |
| O'Connor & Aumasson (2020) | BLAKE3 specification — hash function for content-addressing |
| Saltzer & Schroeder (1975) | "Protection of Information in Computer Systems" — complete mediation principle |

---

## Related Topics

- [02-audit-chain.md](02-audit-chain.md) — The linear audit chain that Forensic AI replays
- [12-witness-dag.md](12-witness-dag.md) — The DAG that provides rich cognitive provenance for replays
- [14-cognitive-kernel-safety.md](14-cognitive-kernel-safety.md) — Cognitive Signals provide human oversight mechanisms required by EU AI Act
- [09-adaptive-risk.md](09-adaptive-risk.md) — Risk system verdicts are part of the replayed decision context
