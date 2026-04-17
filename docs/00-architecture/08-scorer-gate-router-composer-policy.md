# Scorer, Gate, Router, Composer, Policy — The Five Operational Traits

> **Abstract:** This document specifies the five non-fabric operators after REF04. Roko's
> kernel is two mediums (`Engram`, `Pulse`) moving through two fabrics (`Substrate`, `Bus`),
> with six operators acting on them. The five operators here generalize over `Datum` or Pulse
> streams where appropriate; the fabric traits remain explicit. See
> `tmp/refinements/04-operators-generalized.md` for the canonical proposal,
> `tmp/refinements/08-code-sketches.md` for illustrative Rust, and
> [01-naming-and-glossary.md](./01-naming-and-glossary.md) for the authoritative terminology.

> **Reading order:** Start with [06-synapse-traits.md](./06-synapse-traits.md) for the
> two-medium / two-fabric kernel framing, then [07-substrate-trait.md](./07-substrate-trait.md)
> and [07b-bus-transport-fabric.md](./07b-bus-transport-fabric.md) for the fabric contracts.

> **Implementation status**: Target-state operator design. Current operators still accept
> `&[Engram]` directly in today's codebase. The `Pulse`/`Bus` model and `Datum`-based
> generalization documented here are planned migration targets rather than uniformly shipped
> APIs.

---

## Kernel Framing

REF04 keeps the existing operator vocabulary but generalizes its signatures as a target-state
operator model for the runtime:

- `Engram` is the durable medium.
- `Pulse` is the ephemeral medium.
- `Substrate` is the storage fabric.
- `Bus` is the transport fabric.
- `Scorer`, `Gate`, `Router`, `Composer`, and `Policy` are the five non-fabric operators.

The kernel story is therefore not "one noun, six verbs." It is two mediums, two fabrics, six
operators. This document covers the five non-fabric operators; `Substrate` and `Bus` are
specified in their own deep-dive docs.

### Revised Trait Table

The recommended operator surface is:

| Trait | Role | Input | Output | Medium |
|---|---|---|---|---|
| `Scorer` | Rate an item along multi-axis criteria | `Datum<'_>` or monomorphic `&Engram` / `&Pulse` | `Score` | Either |
| `Gate` | Verify against external reality | `&Engram` or `&[Pulse]` | `Verdict` persisted as an Engram | Either -> Engram |
| `Router` | Choose among candidates | `&[Engram]` or `&[Pulse]` | `Option<Selection>` | Either |
| `Composer` | Combine many inputs under a budget | `&[Datum<'_>]`, `&Budget`, `&dyn Scorer` | `Engram` | Either -> Engram |
| `Policy` | React to streams and outcomes | `&[Pulse]` | `PolicyOutputs` | Pulse -> Either |

The fabric siblings around those operators are:

- `Substrate`: persists and queries durable `Engram` records.
- `Bus`: publishes, subscribes, and replays live `Pulse` traffic through `Topic` and
  `TopicFilter`.

That is the complete kernel grammar for this layer of the architecture: six operations plus two
fabric traits.

### Datum - Shared Surface For Either Medium

> **Implementation status**: `Datum` is a target-state abstraction. Current operators accept
> `&[Engram]` directly. The medium-polymorphic `Datum` wrapper is planned but not yet
> implemented.

Operators that can work polymorphically use `Datum`:

```rust
pub enum Datum<'a> {
    Engram(&'a Engram),
    Pulse(&'a Pulse),
}

impl Datum<'_> {
    pub fn kind(&self) -> &Kind { /* ... */ }
    pub fn body(&self) -> &Body { /* ... */ }
    pub fn tags(&self) -> Option<&BTreeMap<String, String>> { /* ... */ }
    pub fn created_at_ms(&self) -> i64 { /* ... */ }
}
```

`Datum` is intentionally small. It does not hide whether a caller is dealing with durable or
ephemeral material; it just gives operators a common dispatch surface when the same logical
operation applies to either medium.

The practical rule is:

- use `&Engram` when only the durable path makes sense
- use `&Pulse` or `&[Pulse]` when the operator is reacting to live traffic
- use `Datum` when a single operator needs to accept either medium without introducing a new
  trait family

## 1. Scorer — Rate Engrams

Most scorers are still naturally Engram-first. REF04's change is that live traffic can be scored
without pretending it is already stored.

```rust
pub trait Scorer: Send + Sync {
    fn score_engram(&self, e: &Engram, ctx: &Context) -> Score;

    fn score_pulse(&self, p: &Pulse, ctx: &Context) -> Score {
        let synthetic = Engram::from_pulse_synthetic(p);
        self.score_engram(&synthetic, ctx)
    }

    fn score(&self, datum: Datum<'_>, ctx: &Context) -> Score {
        match datum {
            Datum::Engram(e) => self.score_engram(e, ctx),
            Datum::Pulse(p) => self.score_pulse(p, ctx),
        }
    }

    fn name(&self) -> &'static str;
}
```

This shape preserves the fast path:

- Engram-oriented scorers implement `score_engram` only.
- Pulse-aware scorers override `score_pulse` when transport-native behavior matters.
- Callers that want one entry point use `score(Datum)`.

Typical Pulse-aware uses include scoring stream chunks for drift, conductor health Pulses for
distress prioritization, and webhook Pulses for triage before graduation.

## 2. Gate — Verify Against Ground Truth

Gate remains the verification operator. The durable Engram path stays primary; the new capability
is stream verification over a Pulse window.

```rust
#[async_trait]
pub trait Gate: Send + Sync {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Verdict;

    async fn verify_stream(&self, pulses: &[Pulse], ctx: &Context) -> Verdict {
        let synthetic = Engram::from_pulses(pulses);
        self.verify(&synthetic, ctx).await
    }

    fn name(&self) -> &str;
}
```

The key invariant does not change: a `Verdict` is still a durable audit artifact. Even when a
Gate verifies a live window, the result persists as an Engram so the audit DAG remains durable.

Stream-gates exist for cases where the truth criterion is temporal rather than already stored:

- `BudgetGate` watches `agent.tokens.used` over a window.
- `SafetyGate` watches `safety.approval.requested` for concurrency or sequencing violations.
- `LivenessGate` watches `agent.msg.chunk` timing and trips on silence.

Most existing gates remain unchanged because the default `verify_stream` path materializes a
synthetic Engram and reuses the durable verification logic.

## 3. Router — Select Among Alternatives

Router still chooses among alternatives and learns from outcomes. REF04 adds a native Pulse path
for transport-side routing decisions.

```rust
pub trait Router: Send + Sync {
    fn select_engram(&self, candidates: &[Engram], ctx: &Context) -> Option<Selection>;

    fn select_pulse(&self, candidates: &[Pulse], ctx: &Context) -> Option<Selection> {
        None
    }

    fn feedback(&self, outcome: &Outcome);
    fn name(&self) -> &'static str;
}
```

That split is deliberate:

- durable selection still covers "which episode or exemplar applies"
- Pulse routing covers "which reviewer should receive this approval request" or "which agent
  should receive this ready task"

The default `None` on `select_pulse` keeps durable-only routers unchanged until a subsystem has a
real live-routing need.

## 4. Composer — Combine Under Budget

Composer remains the bounded assembly operator. The output is still durable; the input set can
now contain either medium.

```rust
pub trait Composer: Send + Sync {
    fn compose(
        &self,
        inputs: &[Datum<'_>],
        budget: &Budget,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<Engram>;

    fn name(&self) -> &'static str;
}
```

This matches the architectural boundary exactly:

- composition may need stored episodes plus the last N stream chunks
- scoring and budget logic still apply across the whole candidate set
- the result remains an `Engram` because composed artifacts are durable records

Representative uses:

- `LiveContextComposer` builds prompt context from both Substrate retrieval and recent Bus
  traffic
- `TelemetryRollupComposer` consolidates a minute of transport Pulses into a summary Engram
- `PromptComposer` keeps the old durable-only path by passing `Datum::Engram` wrappers

## 5. Policy — React to Streams

> **Implementation status**: The `Policy` shape below is the REF04 target contract. Current
> policy implementations are still Engram-first in today's codebase; stream-reactive `Pulse`
> inputs remain planned migration work.

Policy is the most consequential signature change in REF04. Reactive logic naturally consumes
Pulses, not retrospective slices of stored Engrams.

```rust
pub trait Policy: Send + Sync {
    fn decide(&self, stream: &[Pulse], ctx: &Context) -> PolicyOutputs;
    fn name(&self) -> &'static str;
}

pub struct PolicyOutputs {
    pub pulses: Vec<Pulse>,
    pub engrams: Vec<Engram>,
}
```

`PolicyOutputs` makes the reaction step explicit:

- publish new Pulses on the Bus for immediate downstream reactions
- persist Engrams for summaries, graduations, metrics, or durable decisions

This is the architectural fix for the long-standing mismatch where a policy wanted to react to
live changes but the signature implied it was consuming stored artifacts.

Common examples:

- `EpisodePolicy` subscribes to `substrate.engram.stored` filtered to episode kinds and emits
  summary Engrams or follow-on Pulses
- `CircuitBreakerPolicy` watches `gate.verdict.emitted` and publishes failure-rate or pause
  Pulses
- `HeartbeatPolicy` publishes `heartbeat.tick` Pulses at Gamma, Theta, and Delta cadence
- `MetricPolicy` publishes `metric.*` Pulses and graduates summaries on a cadence

Policy is the only breaking trait migration in this batch. The others are additive.

## 6. REF08 Sketches: Operator Signatures In Motion

> **Implementation status**: The examples in this section describe post-migration behavior.
> They are architectural sketches, not uniformly shipped APIs.

REF04 gives the operator surface. REF08 shows how that surface behaves in code once live
transport is explicit. The snippets below are illustrative excerpts rather than the full sketch;
use `tmp/refinements/08-code-sketches.md` when you want the end-to-end Rust.

### 6.1 Policy outputs drive the Bus directly

The key operational point is that a reactive policy now consumes Pulse traffic and can emit both
ephemeral follow-on work and durable records in one pass:

```rust
pub struct PolicyOutputs {
    pub pulses: Vec<Pulse>,
    pub engrams: Vec<Engram>,
}

pub struct PlanRevisionPolicy<B: Bus> {
    bus: std::sync::Arc<B>,
    threshold: usize,
    failures: parking_lot::Mutex<std::collections::HashMap<String, usize>>,
}
```

REF08's `PlanRevisionPolicy` subscribes to `gate.verdict.emitted`, counts consecutive failures,
and publishes `plan.revision.requested` once a task crosses threshold. That is the clearest
worked example of the revised `Policy` contract: watch live Pulses, react on the Bus, and
graduate durable artifacts only when lineage or audit value matters.

### 6.2 Bus mediation dissolves cross-layer imports

The same sketch resolves the old conductor-layer violation by replacing a direct dependency with
topic-mediated transport:

```rust
pub struct CircuitBreaker<B: Bus> {
    bus: std::sync::Arc<B>,
    threshold: f32,
    current_rate: std::sync::atomic::AtomicU32,
}
```

In the REF08 sketch, a learning-side policy publishes `gate.failure.rate`, while conductor
subscribes and emits `conductor.circuit.tripped` when the threshold is crossed. No operator
signature changes for that migration; the fix comes from using `Bus`, `Pulse`, `Topic`, and
`TopicFilter` as the kernel boundary instead of reaching across layers for private types.

### 6.3 The bridge for durable-first callers

REF08 also sketches the migration bridge for durable-first subsystems:

```rust
impl Engram {
    pub fn to_pulse(&self, topic: Topic, seq: u64, source: PulseSource) -> Pulse {
        Pulse {
            seq,
            topic,
            kind: self.kind.clone(),
            body: self.body.clone(),
            emitted_at_ms: self.created_at_ms,
            source,
            lineage_hint: Some(self.id.clone()),
            trace_id: None,
        }
    }
}
```

That bridge lets a `Substrate.put()` implementation publish `substrate.engram.stored` after a
successful durable write. Existing durable workflows can therefore cross the cutover through a
Bus topic instead of a temporary trait fork.

## Academic Foundations

REF04 does not introduce a new research basis for these traits; it tightens the operator grammar
so the documented signatures match the target two-medium / two-fabric architecture. For
the broader theoretical background, use [06-synapse-traits.md](./06-synapse-traits.md),
[09-universal-cognitive-loop.md](./09-universal-cognitive-loop.md), and
[23-architectural-analysis-improvements.md](./23-architectural-analysis-improvements.md).

## Current Status and Gaps

REF04 changes signatures, not the operator count.

The audit argument remains intact:

- no seventh non-fabric operation is required
- the awkward cases were medium mismatches, not missing operator categories
- the real architectural additions are the second medium (`Pulse`) and second fabric (`Bus`)

So the right statement is not "invent more verbs." It is "generalize the verbs that already
exist so they accept the correct medium."

The main implementation gap after this documentation pass is migration sequencing:

- `Scorer`, `Gate`, `Router`, and `Composer` can evolve additively.
- `Policy` is the only breaking signature change in the batch.
- Bus/topic bridges such as `substrate.engram.stored` carry existing durable-first workflows
  through the cutover.

## Default Method Strategy And Migration

The recommended implementation strategy is deliberately conservative:

- monomorphic `_engram` and `_pulse` entry points exist where performance matters
- a `Datum` dispatcher sits on top as a thin convenience layer
- default implementations use "convert and recurse" so single-medium implementations get the
  other path for free at a small cost

Per-trait migration cost is small except for `Policy`:

| Trait | Before | After | Migration shape |
|---|---|---|---|
| `Scorer` | `score(&Engram, &Context)` | add `score_pulse`, `score(Datum)` | additive |
| `Gate` | `verify(&Engram, &Context)` | add `verify_stream(&[Pulse], &Context)` | additive |
| `Router` | `select(&[Engram], &Context)` | add `select_pulse(&[Pulse], ...)` | additive |
| `Composer` | `compose(&[Engram], ...)` | `compose(&[Datum], ...)` | additive wrapper at call sites |
| `Policy` | `decide(&[Engram], ctx) -> Vec<Engram>` | `decide(&[Pulse], ctx) -> PolicyOutputs` | breaking; use a shim during migration |

The canonical migration shim is a bridge that subscribes to `substrate.engram.stored`, converts
relevant durable writes into a Pulse stream, and lets Engram-oriented policy implementations
delete themselves only after the last caller has moved.

## Cross-References

- [06-synapse-traits.md](./06-synapse-traits.md) gives the top-level "two mediums, two fabrics,
  six operators" framing.
- [07-substrate-trait.md](./07-substrate-trait.md) defines the durable storage fabric.
- [07b-bus-transport-fabric.md](./07b-bus-transport-fabric.md) defines the Bus, `Topic`, and
  `TopicFilter`.
- [09-universal-cognitive-loop.md](./09-universal-cognitive-loop.md) shows how these operators
  plug into the current loop framing; REF05 retells that loop in seven steps.
- `tmp/refinements/04-operators-generalized.md` is the canonical source for this signature
  generalization.
- `tmp/refinements/08-code-sketches.md` is the canonical source for the illustrative Rust
  migration sketches referenced here.
