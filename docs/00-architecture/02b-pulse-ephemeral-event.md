# The Pulse Ephemeral Event Medium

> **Abstract:** Pulse is the ephemeral sibling medium to Engram. Pulses are typed,
> sequence-numbered, in-flight events on the Bus: they are delivered now, may be observed
> once, and are not persisted by default. They can carry a lightweight source attribution
> plus an optional lineage hint, but they do not carry Engram durability fields such as
> `id`, `score`, `decay`, or full provenance. When a Pulse must become auditable or
> replayable, it graduates into an Engram using the conversion law in this document.
> For the durable medium, see [02-engram-data-type.md](./02-engram-data-type.md). For the
> naming map and legacy terms, see [01-naming-and-glossary.md](./01-naming-and-glossary.md).
> This doc is the REF02 companion to `tmp/refinements/02-engram-vs-pulse.md`.


> **Implementation status**: Target-state design. No `Pulse` type exists in the codebase yet.
> The current transport mechanism is `EventBus<RokoEvent>` in
> `roko-runtime/src/event_bus.rs`, with 2 live runtime event variants:
> `PlanRevision` and `PrdPublished`.

---

## 1. Why Pulse Exists

> **Implementation status**: Pulse is a target-state medium. This chapter defines the intended
> Bus/Pulse semantics and graduation model; these are not current runtime guarantees.

Engrams are for durable record. Pulse is for live delivery.

The Bus needs a form for token chunks, progress notifications, process lifecycle signals,
dashboard refreshes, and other transient observations that should fan out immediately but
do not deserve to become part of the long-lived audit DAG unless a policy says otherwise.

Pulse gives the architecture three things:

1. **Low-latency fanout**: Subscribers can receive transient updates without waiting for
   storage writes.
2. **Smaller transport cost**: Hot-path notifications stay light and do not drag the full
   durable-record payload into every hop.
3. **Explicit graduation**: The system decides when a live event deserves to become a
   durable Engram instead of forcing every event into the hashed DAG.

Pulse is not a second storage model. It is the transient transport medium that feeds the
durable medium when needed.

---

## 2. The Pulse Struct

The proposed Pulse shape mirrors the refinement note and keeps the live event separate from
the durable record:

```rust
/// An in-flight event traveling on the Bus.
///
/// Pulses are typed, sequence-numbered messages. They are not content-addressed
/// and are not persisted by default. A Pulse may carry a lineage hint pointing
/// at a durable Engram whose ContentHash contextualizes it.
#[derive(Clone, Debug)]
pub struct Pulse {
    /// Topic-local monotonic sequence.
    pub seq: u64,

    /// Topic string, such as "gate.verdict.emitted" or "agent.msg.chunk".
    pub topic: Topic,

    /// Semantic kind, reused from Engram.
    pub kind: Kind,

    /// Payload, reused from Engram.
    pub body: Body,

    /// Unix milliseconds when the Pulse was published.
    pub emitted_at_ms: i64,

    /// Lightweight source attribution for transport-time routing.
    pub source: PulseSource,

    /// Optional Engram reference that contextualizes this Pulse.
    pub lineage_hint: Option<ContentHash>,

    /// Optional trace identifier for distributed tracing.
    pub trace_id: Option<TraceId>,
}
```

### 2.1 What Pulse Does Not Carry

Pulse deliberately omits the durable-record fields that belong to Engram:

- No `id` or content hash by default
- No `score`
- No `decay`
- No full `Provenance`
- No attestation
- No persisted lineage vector

That omission is the point. A Pulse is allowed to be brief and disposable.

### 2.2 Topic Contract

Topics are the naming surface for live subscribers. The refinement note uses examples like:

- `orchestration.plan.started`
- `agent.msg.chunk`
- `agent.process.spawned`
- `agent.tokens.used`
- `gate.verdict.emitted`
- `ui.refresh.requested`
- `heartbeat.tick`

The exact topic registry can evolve, but the Bus contract is stable: Pulses are addressed by
topic and sequence, not by durable identity.

---

## 3. Pulse to Engram Graduation

Graduation is the explicit conversion from live transport into durable record.

```rust
impl Pulse {
    /// Graduate this Pulse into a durable Engram.
    ///
    /// The caller supplies the full provenance, decay policy, score,
    /// and tags. The lineage hint becomes the first durable parent.
    pub fn graduate(
        &self,
        provenance: Provenance,
        decay: Decay,
        score: Score,
        tags: BTreeMap<String, String>,
    ) -> Engram {
        let lineage = self.lineage_hint.iter().copied().collect();
        EngramBuilder::new(self.kind.clone(), self.body.clone())
            .created_at_ms(self.emitted_at_ms)
            .provenance(provenance)
            .decay(decay)
            .score(score)
            .lineage(lineage)
            .tags(tags)
            .build()
    }
}
```

### 3.1 Graduation Law

The law is simple:

- A Pulse remains transient while it is only useful for delivery or UI fanout.
- A Pulse graduates when it needs auditability, replay, verification, or durable learning.
- Once graduated, the Engram is the durable source of truth for downstream operators.

That means the Bus carries live events, while the Substrate stores the durable record.

### 3.2 Provenance Upgrade

Pulse carries lightweight `source` attribution; Engram carries full `Provenance`.

When a Pulse graduates:

- `source` becomes `provenance.author`
- the source class determines the initial `trust` and `tainted` state
- `lineage_hint` becomes the first durable lineage parent
- `trace_id` stays transport metadata and does not affect the Engram hash
- optional attestation can be attached only once the durable record exists

The upgrade is a refinement, not a rewrite. The same event becomes more accountable when it
crosses the transport-to-record boundary.

### 3.3 Graduation Example

```rust
let pulse = Pulse {
    seq: 42,
    topic: Topic::new("gate.verdict.emitted"),
    kind: Kind::GateVerdict,
    body: Body::Json(json!({"passed": true})),
    emitted_at_ms: now_ms(),
    source: PulseSource::trusted("gate:compile"),
    lineage_hint: Some(task.id),
    trace_id: Some(trace_id),
};

let engram = pulse.graduate(
    Provenance::trusted("gate:compile"),
    Decay::None,
    Score::NEUTRAL,
    BTreeMap::from([("phase".into(), "verification".into())]),
);
```

The Pulse is for delivery; the Engram is for record.

---

## 4. Graduation Policy

> **Implementation status**: This policy table is target-state guidance, not a shared runtime
> policy that exists today.

Not every Pulse should graduate. The default policy should keep hot-path noise out of the
durable DAG and only promote events whose future value outweighs the storage cost.

| Pulse topic | Graduate? | Default policy | Reason |
|---|---|---|---|
| `orchestration.plan.started` | Yes | Graduate immediately | Plan lifecycle belongs in the audit DAG |
| `orchestration.task.ready` | No | Drop after delivery | Redundant with the durable Task Engram |
| `agent.msg.chunk` | Batch | Graduate on stream close | Chunks are transport noise; the completed turn is durable |
| `agent.process.spawned` | Yes | Graduate immediately | Process lifecycle is forensic evidence |
| `agent.process.exited` | Yes | Graduate immediately | Exit state matters for reconstruction |
| `agent.tokens.used` | Aggregate | Graduate per turn or per batch | Per-token noise is too granular for the DAG |
| `gate.verdict.emitted` | Yes | Graduate immediately | Verdicts are first-class audit records |
| `safety.approval.requested` | Yes | Graduate immediately | Safety decisions must be durable |
| `conductor.circuit.tripped` | Yes | Graduate immediately | Health failures need history |
| `ui.refresh.requested` | No | Drop after fanout | UI-local and not semantically durable |
| `heartbeat.tick` | No | Drop after fanout | Infrastructure chatter, not knowledge |

This table is a policy default, not a hard law. A domain can override graduation behavior
when a specific Pulse topic has stronger durability requirements.

### 4.1 Policy Ownership

The default graduation rules should live close to `roko-core` so that every producer and
consumer can share the same baseline. Higher-level applications may narrow or expand the
policy, but they should not invent a second durability model.

---

## 5. Relationship to Engram

Pulse and Engram are sibling media, not competing truth models.

- Pulse carries the live event.
- Engram carries the durable record.
- Graduation is the bridge between them.

The durable invariants still belong to Engram only: content addressability, lineage DAG,
decay, provenance, and optional attestation. Pulse remains intentionally lighter so the Bus
can stay fast and the durable DAG can stay meaningful.

For the durable side of the architecture, read [02-engram-data-type.md](./02-engram-data-type.md).
