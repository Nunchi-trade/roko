# Missing Kernel Types -- Specified but Not Built

These are core types from the kernel specification (docs 02b, 06, 07b, 08) that have zero code in the codebase. They form a dependency chain -- Pulse must exist before Bus, Datum, TopicFilter, and PolicyOutputs can be built.

## Dependency chain

```
Topic (K-06)
  |
  v
Pulse (K-01)
  |
  +---> Bus trait (K-02)
  |       |
  |       +---> TopicFilter (K-04)
  |
  +---> Datum enum (K-03)
  |
  +---> PolicyOutputs (K-05)

Probe (K-07) -- independent, reconciliation of two existing traits
```

K-01 through K-06 must exist before the trait migrations in `03-trait-migrations.md` can proceed. K-07 is independent.

## Checklist

### K-01: Pulse struct
- [x] Implement `Pulse` as specified in doc 02b-pulse-ephemeral-event.md

**Spec** (doc 02b, doc 06):
```rust
pub struct Pulse {
    pub seq: u64,
    pub topic: Topic,
    pub kind: Kind,
    pub body: Body,
}
```
Pulse is the ephemeral counterpart to Engram. It represents live transport traffic that has not been persisted. It may "graduate" to an Engram through deliberate promotion.

**Current code**: No `struct Pulse` exists anywhere in `crates/`.
**Where to put it**: `crates/roko-core/src/pulse.rs` (alongside `engram.rs`)
**Accept when**:
- [x] `pub struct Pulse` exists in roko-core
- [x] Has at minimum: `seq: u64`, `topic`, `kind: Kind`, `body: Body`
- [x] Implements `Serialize`, `Deserialize`, `Debug`, `Clone`
- [x] `Engram::from_pulse_synthetic(p: &Pulse) -> Engram` conversion exists (needed by trait defaults)
- [x] `Engram::from_pulses(pulses: &[Pulse]) -> Engram` conversion exists (needed by Gate default)
- [x] Re-exported from `roko-core/src/lib.rs`

**Verify**:
```bash
grep -rn 'pub struct Pulse' crates/roko-core/src/ --include='*.rs'  # should match
cargo build -p roko-core
```

---

### K-02: Bus trait
- [x] Implement `Bus` trait as specified in doc 07b-bus-transport-fabric.md

**Spec** (doc 06, doc 07b):
```rust
#[async_trait]
pub trait Bus: Send + Sync {
    async fn publish(&self, pulse: Pulse) -> Result<u64>;
    async fn subscribe(&self, filter: TopicFilter) -> Result<BusReceiver>;
}
```
Bus is the transport fabric for Pulses, parallel to Substrate (the storage fabric for Engrams). Together they form the "two fabrics" of the kernel.

**Current code**: No `trait Bus` exists. Only `EventBus<E>` in roko-runtime -- a concrete generic, not the abstract kernel trait.
**Where to put it**: `crates/roko-core/src/traits.rs` (alongside Substrate, Scorer, Gate, Router, Composer, Policy)
**Accept when**:
- [x] `pub trait Bus: Send + Sync` exists in roko-core/src/traits.rs
- [x] Has `publish` and `subscribe` methods per spec
- [x] `BusReceiver` type is defined (async channel receiver for Pulses)
- [x] Re-exported from `roko-core/src/lib.rs`

**Verify**:
```bash
grep -rn 'trait Bus' crates/roko-core/src/traits.rs  # should match
cargo build -p roko-core
```

---

### K-03: Datum<'a> enum
- [x] Implement `Datum` enum as specified in doc 06/08

**Spec** (doc 06, doc 08):
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
Datum is the polymorphic input surface that lets operators work over either medium without introducing a new trait family.

**Current code**: No `enum Datum` exists anywhere.
**Where to put it**: `crates/roko-core/src/datum.rs`
**Depends on**: K-01 (Pulse)
**Accept when**:
- [x] `pub enum Datum<'a>` exists with `Engram` and `Pulse` variants
- [x] Has accessor methods: `kind()`, `body()`, `tags()`, `created_at_ms()`
- [x] Re-exported from `roko-core/src/lib.rs`

**Verify**:
```bash
grep -rn 'enum Datum' crates/roko-core/src/ --include='*.rs'  # should match
cargo build -p roko-core
```

---

### K-04: TopicFilter enum
- [x] Implement `TopicFilter` as specified in doc 07b

**Spec** (doc 07b): Filter type used by `Bus::subscribe` to select which Pulses to receive. Exact variants should be specified in doc 07b (likely includes topic prefix matching, kind filtering, etc.).

**Current code**: No `TopicFilter` exists anywhere.
**Where to put it**: `crates/roko-core/src/pulse.rs` (alongside Pulse)
**Depends on**: K-01 (Pulse), K-02 (Bus)
**Accept when**:
- [x] `pub enum TopicFilter` or `pub struct TopicFilter` exists
- [x] Used as parameter in `Bus::subscribe`
- [x] Re-exported from `roko-core/src/lib.rs`

**Verify**:
```bash
grep -rn 'TopicFilter' crates/roko-core/src/ --include='*.rs'  # should match
cargo build -p roko-core
```

---

### K-05: PolicyOutputs struct
- [x] Implement `PolicyOutputs` as specified in doc 08

**Spec** (doc 08):
```rust
pub struct PolicyOutputs {
    pub pulses: Vec<Pulse>,
    pub engrams: Vec<Engram>,
}
```
PolicyOutputs makes the Policy reaction step explicit -- policies can publish new Pulses on the Bus for immediate downstream reactions AND persist Engrams for summaries and decisions.

**Current code**: No `PolicyOutputs` exists anywhere. Current Policy returns `Vec<Engram>`.
**Where to put it**: `crates/roko-core/src/pulse.rs` or a dedicated file
**Depends on**: K-01 (Pulse)
**Accept when**:
- [x] `pub struct PolicyOutputs` exists with `pulses: Vec<Pulse>` and `engrams: Vec<Engram>`
- [x] Re-exported from `roko-core/src/lib.rs`
- [x] Used as return type of `Policy::decide_with_pulses` in traits.rs:348 (backwards-compat migration path per 03-trait-migrations.md)

**Verify**:
```bash
grep -rn 'struct PolicyOutputs' crates/roko-core/src/ --include='*.rs'  # should match
cargo build -p roko-core
```

---

---

### K-06: Topic type
- [x] Implement `Topic` type as specified in doc 02b-pulse-ephemeral-event.md

**Spec** (doc 02b, doc 06):
```rust
pub struct Topic(pub String);

impl Topic {
    pub fn new(s: impl Into<String>) -> Self { Self(s.into()) }
}
```
Topic is the addressing/routing key for Pulses on the Bus. Used as `topic: Topic` field in Pulse struct. Docs reference topics like `"gate.verdict.emitted"`, `"heartbeat.gamma.tick"`, etc.

**Current code**: No `struct Topic` or `type Topic` exists in roko-core. `TopicRequest` in roko-serve is an unrelated HTTP request type. The Pulse struct (K-01) needs Topic for its `topic` field.
**Where to put it**: `crates/roko-core/src/pulse.rs` (alongside Pulse)
**Accept when**:
- [x] `pub struct Topic` (or `pub type Topic = String`) exists in roko-core
- [x] Implements `Serialize`, `Deserialize`, `Debug`, `Clone`, `PartialEq`, `Eq`, `Hash`
- [x] Constructor `Topic::new(s)` works
- [x] Re-exported from `roko-core/src/lib.rs`

**Verify**:
```bash
grep -rn 'struct Topic\|type Topic' crates/roko-core/src/ --include='*.rs'
cargo build -p roko-core
```

---

### K-07: Probe trait (kernel-level)
- [x] Define canonical `Probe` trait in roko-core

**Spec** (doc 11, doc 09 in 16-heartbeat): T0 Probes are zero-LLM deterministic checks that run at every gamma tick. 16 probes specified. Probe trait is the extension point.

**Current code**: **Only one Probe trait exists:**
- `crates/roko-core/src/obs/health.rs:89` — `pub trait Probe: Send + Sync` (canonical)

The duplicate in roko-runtime has been consolidated. The kernel-level trait lives in roko-core as intended.

**Where to put it**: `crates/roko-core/src/obs/health.rs` (keep existing location but ensure it's canonical)
**Accept when**:
- [x] Single canonical `Probe` trait in roko-core
- [x] Runtime's heartbeat_probes.rs uses/re-exports the core trait (no duplicate)
- [x] `ProbeRegistry` consolidation (one canonical registry)
- [x] Re-exported from `roko-core/src/lib.rs`

**Verify**:
```bash
grep -rn 'trait Probe' crates/ --include='*.rs' | grep -v target/  # should show exactly 1 definition
cargo build -p roko-core
cargo build -p roko-runtime
```

---

## Implementation order

1. K-01 (Pulse) -- no dependencies, do first
2. K-06 (Topic) -- no dependencies, needed by K-01
3. K-03 (Datum) -- depends on K-01
4. K-04 (TopicFilter) -- depends on K-01
5. K-05 (PolicyOutputs) -- depends on K-01
6. K-02 (Bus) -- depends on K-01 and K-04
7. K-07 (Probe reconciliation) -- independent, can be done anytime

After K-01 through K-06 exist, proceed to `03-trait-migrations.md`.
