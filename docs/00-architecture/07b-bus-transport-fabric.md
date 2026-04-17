# The Bus Transport Fabric

> **Abstract:** Bus is the transport fabric and kernel primitive of the runtime. It publishes,
> subscribes to, and replays Pulses through topics and bounded ring buffers. It is the sibling
> of `Substrate`, and together they form the two-fabric kernel story described in
> `tmp/refinements/03-bus-as-first-class.md`.
> For naming conventions and current terminology, see [01-naming-and-glossary.md](01-naming-and-glossary.md).

> **Implementation**: Documented kernel target over shipping runtime behavior

---

## 1. Role in the Architecture

Bus is the kernel's ephemeral transport fabric at L0. It exists for communication, not
durable storage. Where Substrate preserves Engrams, Bus delivers Pulses to subscribers
that care about a topic family. The two fabrics are the complete kernel surface at L0:
storage lives in Substrate, transport lives in Bus.

The design separates transport from persistence so that:

- high-frequency Pulses can move without forcing storage writes,
- late subscribers can catch up from the bounded replay ring,
- cross-layer couplings can be expressed as topics instead of direct crate dependencies.

This is the architectural replacement for historical `EventBus<E>`-style plumbing. The
legacy generic broadcast channel remains as the default in-process implementation, but the
kernel name is now `Bus`.

### 1.1 Two-Fabric Summary

| Fabric | Medium | Core operations | Retention model | Typical backends |
|---|---|---|---|---|
| Substrate | Engram | `put`, `get`, `query`, `prune` | Long-lived storage with content identity and decay-aware pruning | Memory, File, HDC, Chain |
| Bus | Pulse | `publish`, `subscribe`, `replay_since`, `current_seq` | Bounded transport ring with topic routing and replay retention | BroadcastBus, MemoryBus, MultiBus, NATS/Kafka/Redpanda, ChainBus |

---

## 2. Trait Surface

REF03 promotes the shipping runtime behavior into the following kernel trait surface:

```rust
#[async_trait]
pub trait Bus: Send + Sync {
    /// Publish a Pulse. Returns its global sequence number.
    async fn publish(&self, pulse: Pulse) -> Result<u64>;

    /// Subscribe to a topic filter. Returns a BusReceiver of matching Pulses.
    async fn subscribe(&self, filter: TopicFilter) -> Result<BusReceiver>;

    /// Replay Pulses newer than `since_seq` that still match the filter and remain in ring.
    async fn replay_since(&self, since_seq: u64, filter: &TopicFilter) -> Result<Vec<Pulse>>;

    /// Current global sequence number, useful for checkpoints and resume.
    async fn current_seq(&self) -> Result<u64>;

    /// Total Pulses published since bus start, for metrics and capacity planning.
    async fn total_published(&self) -> Result<u64>;

    /// Ring buffer current occupancy.
    async fn ring_len(&self) -> Result<usize>;

    /// Ring buffer capacity.
    fn ring_capacity(&self) -> usize;

    /// Human-readable name for logging/debugging.
    fn name(&self) -> &'static str {
        "unnamed_bus"
    }
}
```

### 2.1 Payload and routing terms

- `Pulse` is the ephemeral message type carried by the Bus.
- `Topic` is the routing handle on a Pulse. It should be treated as a lowercase,
  dot-separated routing key.
- `TopicFilter` is the declarative subscription language used to match topics and replay
  catch-up windows.
- `BusReceiver` is the subscriber handle returned by `subscribe()`. It yields Pulses in
  publish order and tracks the subscriber's last seen sequence for bounded resume logic.

The Bus does not content-address messages and does not persist them by default. If the
message must survive beyond the replay ring, graduate it to an Engram and store it on a
Substrate.

### 2.2 TopicFilter

```rust
pub enum TopicFilter {
    Exact(Topic),
    Glob(String),
    AnyOf(Vec<Topic>),
    All,
    And(Box<TopicFilter>, Box<TopicFilter>),
    Or(Box<TopicFilter>, Box<TopicFilter>),
    Not(Box<TopicFilter>),
}
```

Topics are dot-separated lowercase strings such as `gate.verdict.emitted`,
`agent.msg.chunk`, or `prediction.error`. They name transport intent rather than durable
storage identity.

---

## 3. Semantics

### 3.1 Publish and subscribe

`publish()` fans a Pulse out to every matching subscriber. The delivery model is broadcast,
not queue-based work stealing. Every subscriber sees every matching Pulse, in publish order.

`subscribe()` returns a receiver that is cancel-safe. Callers can keep the receiver open
for a narrow topic family or for broad catch-all monitoring.

### 3.2 Replay and sequence numbers

The Bus keeps a bounded replay ring. `replay_since()` returns Pulses whose sequence number
is strictly greater than `since_seq` and that still remain in the ring. `ring_len()` and
`ring_capacity()` expose that retention window directly.

`current_seq()` returns the latest global sequence number for checkpointing and resume
logic. A subscriber can store the last seen sequence, disconnect briefly, and later resume
with `replay_since()` before rejoining the live stream.

`total_published()` gives the monotonic publish count for metrics and health checks, while
`ring_len()` and `ring_capacity()` expose the current retention window so operators can see
when a subscriber is at risk of falling behind the ring.

### 3.3 Ring semantics

The ring buffer is a bounded retention window, not durable storage:

- newer Pulses evict older Pulses when capacity is reached,
- replay only covers what remains in the ring,
- subscribers that fall behind lose history unless the Pulse is also graduated to an Engram.

The default in-process Bus uses a bounded ring over `tokio::sync::broadcast` plus replay
state. Future backends can use the same semantics with different transport internals.

---

## 4. Backend Families

### 4.1 BroadcastBus

The default shipping implementation. It wraps in-process broadcast primitives and serves
single-process agents, tests, and local tooling.

### 4.2 MemoryBus

Test-only backend with the same trait surface and no background transport task.

### 4.3 MultiBus

An aggregator that merges multiple Bus backends into one stream. Useful when a runtime
needs to see in-process Pulses and remote Pulses together.

### 4.4 NATS / Kafka / Redpanda

Multi-process and distributed backends. These are the natural fit for remote workers and
control-plane deployments.

### 4.5 ChainBus

On-chain Pulse transport for Korai-integrated deployments. It maps topics to chain logs
and replays by block scanning.

---

## 5. Concurrency

All Bus implementations are `Send + Sync`. They must handle concurrent publishers and
subscribers internally.

- In-process backends rely on concurrent broadcast and a bounded replay ring.
- Multi-process backends use broker or log semantics provided by the transport.
- The API is designed so callers do not need external locks to publish or subscribe.

The Bus is the transport fabric counterpart to Substrate's storage semantics. The same
runtime may hold both handles and use them concurrently.

---

## 6. Two-Fabric Kernel Story

The kernel surface is intentionally small:

- `Substrate` persists durable Engrams.
- `Bus` transports ephemeral Pulses.

This split lets Roko express durable knowledge and live communication without forcing every
message through the same mechanism. It also makes cross-layer communication auditable
through topic names rather than direct dependencies.

The Bus-first framing is what dissolves the old `roko-conductor -> roko-learn` coupling:
the conductor can react to topic streams such as `gate.failure.rate` and
`gate.verdict.emitted` instead of importing learning types directly.

---

## 7. Current Status and Gaps

- **Implemented today**: in-process broadcast-and-replay behavior in the runtime layer.
- **Promoted by REF03**: the stable kernel naming (`Bus`, `Topic`, `TopicFilter`) and the
  trait surface documented in this chapter.
- **Planned backends**: `MultiBus`, `NATS`/`Kafka`/`Redpanda`, `ChainBus`, and `MemoryBus`.

---

## Cross-References

- [01-naming-and-glossary.md](01-naming-and-glossary.md) - Canonical names for Bus terminology
- [06-synapse-traits.md](06-synapse-traits.md) - Trait overview that includes Bus
- [07-substrate-trait.md](07-substrate-trait.md) - The storage fabric sibling
- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) - Where Bus fits into the loop
- `see tmp/refinements/03-bus-as-first-class.md`
