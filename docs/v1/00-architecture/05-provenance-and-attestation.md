# Provenance and Attestation

> **Abstract:** In the revised architecture, provenance is the durable audit context attached to every `Engram`, not just a lightweight author tag. It tells operators who produced the record, how much to trust it, whether it is tainted, and which higher-assurance artifacts such as `Custody` and `Attestation` are linked to it. This document aligns the architecture chapter with the REF32 safety spine: typed taint, chain-of-custody records for auditable actions, explicit attestation levels, and a clear split between durable proof in `Substrate` and live safety signaling on the `Bus`. See [tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md) and [Naming Map and Glossary](01-naming-and-glossary.md).

> **Implementation status:** Specified architecture with a shipping subset

---

## 1. Why Provenance Matters

Roko runs in adversarial and regulated environments. It ingests user input, tool output,
external fetches, plugin results, and its own prior knowledge. That means the system must
preserve more than content. It must preserve audit context.

At the architecture level, provenance answers four questions for every durable record:

1. **Who produced this Engram?** A user, agent, gate, plugin, chain source, or system role.
2. **How trusted was that producer at emission time?** Trust is a snapshot, not a live lookup.
3. **Is the record tainted, and why?** Taint is a typed safety signal, not a generic warning bit.
4. **What higher-assurance proof exists around this record?** A `Custody` chain, an
   `Attestation`, or both.

This is load-bearing for three reasons:

- **Safety:** tainted inputs must not silently flow into high-risk actions.
- **Auditability:** operators must be able to answer who did what, why, with what approval,
  and with what consequence.
- **Composability:** `Scorer`, `Gate`, `Router`, `Composer`, and `Policy` need a common,
  architecture-level vocabulary for trust and safety.

`Pulse` remains the ephemeral wire medium on the `Bus`. It can carry lightweight source
metadata for routing and live review, but provenance becomes durable only when the runtime
graduates relevant state into an `Engram` and persists it in `Substrate`.

---

## 2. The Provenance Contract

Every `Engram` carries provenance. The exact shipping struct can evolve, but the architecture
contract is stable: provenance is the minimum durable audit context required to interpret the
record safely after the fact.

```rust
pub struct Provenance {
    pub author: String,
    pub trust: f32,
    pub session: Option<String>,
    pub taint: Taint,
}
```

The fields mean:

| Field | Meaning |
|---|---|
| `author` | Durable producer identity for the record: user, agent, gate, plugin, system role, or external source label |
| `trust` | Snapshot trust score at time of emission; later reputation changes do not rewrite history |
| `session` | Optional run/session grouping for replay, audits, and scoped queries |
| `taint` | Typed safety classification for whether the record originated from or depends on untrusted input |

Two architecture rules follow from that contract:

1. Provenance is part of the Engram's durable meaning. Two identical bodies from different
   authors or taint states are not the same audit record.
2. Provenance is not the whole safety story. It is the base layer that `Custody`,
   `Attestation`, and safety-focused `Pulse` streams build on top of.

Where a record needs stronger guarantees than `author + trust + taint + session`, those
guarantees are modeled as linked durable artifacts rather than by overloading the base
provenance struct.

---

## 3. Provenance as Durable Audit Context on Engrams

The durable medium in Roko is the `Engram`, so the durable audit trail also lives there.
Architecture-level provenance therefore means:

- A retrieved Engram from `Substrate` is self-describing enough to evaluate safety without
  consulting the original runtime process.
- A reviewer can inspect lineage and see whether a record came from trusted gates, user input,
  plugin output, or external fetches.
- The runtime can attach stronger evidence, such as a `Custody` record or cryptographic
  `Attestation`, without mutating the historical meaning of the original record.

In practice, that yields three layers of audit evidence:

| Layer | Stored on | Purpose |
|---|---|---|
| `Provenance` | every `Engram` | minimum durable audit context |
| `Custody` | auditable-action `Engram` | why a privileged or externally visible action happened |
| `Attestation` | opt-in on selected `Engram` kinds | cryptographic proof of signer and integrity |

This is the key architectural split:

- `Bus` is for live delivery, approvals, and safety telemetry.
- `Substrate` is for durable audit truth.

The system may publish `safety.*`, `network.egress.*`, `plugin.violation`, or
`gate.verdict.emitted` Pulses as events happen, but the evidence that survives replay is the
Engram set persisted in `Substrate`.

---

## 4. Taint Analysis

The older binary taint flag is too coarse for the safety spine described in REF32. The
architecture-level taint model is typed:

```rust
#[non_exhaustive]
enum Taint {
    Clean,
    LlmHallucination { detail: String },
    ToolFailure { detail: String },
    UserFlagged { detail: String },
    StaleData { threshold_ms: i64 },
    UnverifiedSource { detail: String },
    Propagated { detail: String, inherited_from: Option<ContentHash> },
    UserInput { detail: String },
    Custom(String),
}
```

`Taint` captures where risk entered the system:

| Variant | Meaning |
|---|---|
| `Clean` | No taint — data is from a trusted, verified source |
| `LlmHallucination` | LLM-generated content that may contain hallucinated facts |
| `ToolFailure` | A tool call failed or returned suspect data |
| `UserFlagged` | A human operator explicitly flagged this data |
| `StaleData` | Data has exceeded its freshness window |
| `UnverifiedSource` | Data came from an unverified external source (API, webhook, chain) |
| `Propagated` | Taint was inherited from an upstream tainted signal (tracks `inherited_from` hash) |
| `UserInput` | User-provided prompt, paste, file upload, or inline instruction |
| `Custom` | Application-specific taint reason not covered by other variants |

### 4.1 Propagation Rules

Taint is one-way and conservative:

1. If a `Composer` reads any tainted Engram, the composed prompt Engram is tainted.
2. If an LLM turn or tool action consumes tainted input, its derived output stays tainted until
   explicitly reviewed and signed off.
3. If multiple taint sources contribute to one result, the output records the strongest relevant
   taint classification rather than silently collapsing to `None`.
4. Gate verdicts can validate claims about tainted inputs, but they do not erase the historical
   fact that tainted material participated in the decision path.
5. Clearing taint requires explicit human action recorded in the audit trail; it is not an
   automatic side effect of normal execution.

This matters because taint is consulted at action time. A tainted summary might be acceptable
for drafting, but the same tainted lineage can require confirmation, denial, or escalation for a
destructive write, a network egress, or a signed chain action.

### 4.2 Taint and the Cognitive Immune System

The cognitive immune system consumes taint as an architectural input rather than inventing its
own parallel concept. `Taint` marks the first-order source of concern; the immune system layers
quarantine, anomaly detection, re-verification, and attack-pattern memory on top of that base.
See [26-cognitive-immune-system.md](26-cognitive-immune-system.md) for the defense-in-depth
path that builds on the taint model defined here.

---

## 5. Custody Records for Auditable Actions

Not every Engram needs a full chain-of-custody record. But any action that changes external
state, performs a privileged operation, or needs compliance-grade review must emit a durable
`Custody` Engram.

REF32 defines the record shape:

```rust
Custody {
    action: ActionHash,
    principal: PrincipalId,
    when: Timestamp,
    authorized: AuthzEvidence,
    why_heuristics: Vec<HeuristicId>,
    why_claims: Vec<ClaimId>,
    simulation: Option<SimHash>,
    gates_passed: Vec<GateVerdict>,
    result: Option<ResultHash>,
    witness: Option<ChainWitness>,
}
```

The fields answer the auditable questions directly:

| Field | Why it exists |
|---|---|
| `action` | Canonical identity for what was attempted or executed |
| `principal` | Who initiated the action: user, agent, plugin, or delegated role |
| `when` | Timestamp for replay and audit sequencing |
| `authorized` | Which role grant, confirmation, escalation, or session approval allowed it |
| `why_heuristics` | Which heuristics shaped the choice |
| `why_claims` | Which research-backed claims or prior findings justified it |
| `simulation` | Optional dry-run or preflight evidence, especially for ops and chain domains |
| `gates_passed` | Which verification steps approved the action |
| `result` | Durable pointer to the outcome |
| `witness` | Optional external witness, including chain witness in Phase 2+ |

Architecture rules for `Custody`:

- `Custody` is itself durable. It lives in `Substrate`, not only in logs.
- Domain profiles decide which actions require custody, but destructive and externally visible
  actions are the default high-priority cases.
- `Custody` does not replace ordinary provenance; it augments it for actions that need deeper
  accountability.
- When present, `Custody` should be queryable independently of the original runtime that issued
  the action.

---

## 6. Attestation

Attestation is the cryptographic layer on top of provenance. It proves that a specific signer
committed to a specific durable record.

```rust
Attestation {
    signer: PublicKey,
    signature: Ed25519Signature,
    signed_hash: ContentHash,
    timestamp: i64,
    level: AttestationLevel,
}

enum AttestationLevel {
    LocalAgent,
    OrgRole,
    ChainWitness,
}
```

### 6.1 Attestation Levels

| Level | Meaning | Typical use |
|---|---|---|
| `LocalAgent` | Signed by the current agent session key | low-friction auditability for gate verdicts and local runtime outputs |
| `OrgRole` | Signed by a human-owned organizational or approval key | destructive or externally visible actions that require human sign-off |
| `ChainWitness` | Independently witnessed on-chain | cross-deployment trust, later verification, and Phase 2+ external audit |

Attestation is opt-in by kind. REF32's default posture is:

- `GateVerdict` Engrams default to `LocalAgent`.
- `Custody` for destructive actions defaults to `OrgRole`.
- Heuristic-commons contributions may use `ChainWitness` when cross-deployment trust matters.

Three rules keep the model coherent:

1. An attestation signs the `ContentHash`; it does not replace content addressing.
2. Attestation strengthens integrity and signer identity, but it does not erase taint.
3. Attestation and custody compose: a `Custody` Engram can itself be attested.

---

## 7. Provenance in the Seven-Step Loop

The seven-step loop gives provenance, taint, custody, and attestation their operational home.
See [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) for the full loop.

### Step 1: SENSE

- `Substrate.query()` returns provenance-bearing Engrams.
- `Bus.subscribe()` returns live Pulses that may later graduate into Engrams.
- External I/O enters as potential taint sources and should be normalized before further use.

### Step 2: ASSESS

- `Scorer` and `Router` are allowed to consult provenance and taint, not just semantic content.
- Tainted or weakly trusted records can be down-ranked, routed to a stronger gate path, or sent
  toward confirmation workflows.

### Step 3: COMPOSE

- `Composer` preserves taint lineage when assembling prompt Engrams.
- Prompt assembly is where untrusted context can become dangerous, so provenance-aware
  composition is a core safety obligation rather than a later add-on.

### Step 4: ACT

- Actions consume the taint state of their inputs.
- High-risk actions use that state to require confirmation, deny execution, or escalate.
- If the action is auditable, this step allocates the `Custody` record that explains what
  happened and why.

### Step 5: VERIFY

- `Gate` implementations record which verdicts passed and feed those verdicts into `Custody`.
- Verification can validate an action without deleting the historical taint that fed it.
- Attestation commonly starts here for `GateVerdict` and approval-bearing outputs.

### Step 6: PERSIST and BROADCAST

- `Substrate.put()` persists the action result, verdict Engrams, and any `Custody` record.
- `Bus.publish()` emits live Pulses such as `safety.*`, `network.egress.*`, or
  `gate.verdict.emitted`.
- The architectural invariant is that `Substrate` holds durable audit truth while `Bus` delivers
  real-time visibility and reaction triggers.

### Step 7: REACT

- `Policy` reads the new safety evidence and decides follow-up actions: quarantine, approval
  request, replay, or escalation.
- The reactive path may publish more Pulses immediately while also persisting additional Engrams
  for later review.

---

## 8. Provenance Across the Two Fabrics

The safety spine only works if the two fabrics keep distinct responsibilities.

### 8.1 Substrate Responsibilities

`Substrate` is where durable audit context lives:

- persisted Engram provenance
- `Custody` records
- attested records
- lineage needed for replay and incident response

Auditors query `Substrate` because that is where the replayable truth survives.

### 8.2 Bus Responsibilities

`Bus` is where live safety coordination happens:

- approval prompts and confirmations
- `safety.*` notifications
- `network.egress.*` telemetry
- sandbox violations and gate verdict notifications

The `Bus` is not a substitute for durable audit evidence. It is the transport path that keeps
operators, UIs, and policies informed in time to intervene.

### 8.3 Why the Split Matters

The split prevents a common failure mode in agent systems: safety evidence exists only in live
logs or UI traces and disappears once the process exits. In Roko, live observation and durable
proof are separate but connected:

- `Pulse` enables immediate intervention.
- `Engram` enables later audit.
- `Custody` and `Attestation` lift selected actions to stronger guarantees.

---

## 9. Current Status and Direction

The repository still contains a shipping subset of this story: a simple provenance shape with
author/trust/session and a taint bit. The architecture contract in this document is the wider
target state after REF32:

- taint is typed and propagated conservatively
- auditable actions emit `Custody`
- selected kinds support explicit `AttestationLevel`
- `Substrate` and `Bus` divide durable proof from live safety signaling

That direction keeps the docs aligned with the seven-step loop, the two-fabric model, and the
broader safety spine instead of treating provenance as an isolated helper struct.

---

## Cross-References

- [Naming Map and Glossary](01-naming-and-glossary.md)
- [02-engram-data-type.md](02-engram-data-type.md)
- [07-substrate-trait.md](07-substrate-trait.md)
- [07b-bus-transport-fabric.md](07b-bus-transport-fabric.md)
- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md)
- [26-cognitive-immune-system.md](26-cognitive-immune-system.md)
- [tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md)
