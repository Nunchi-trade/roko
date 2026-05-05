# Store and Bus Duality

> Depth for [02-CELL.md](../../unified/02-CELL.md). Derives the duality between Store (pull-based, durable Signals) and Bus (push-based, ephemeral Pulses), the graduation/projection bridges, consistency under ring eviction, and backpressure as Cell configuration.

---

## 1. The Duality

Store and Bus are dual in a precise sense. They are two views of the same underlying information flow, related by a formal duality that swaps:

| Store (pull) | Bus (push) |
|---|---|
| Consumer initiates (`query`) | Producer initiates (`publish`) |
| Durable (survives restart) | Ephemeral (bounded ring, then gone) |
| Identity is content hash | Identity is sequence number |
| Time-indexed by creation | Time-indexed by emission |
| Supports similarity (`query_similar`) | Supports topic routing (`TopicFilter`) |
| Retention is decay-based (demurrage) | Retention is capacity-based (ring eviction) |
| Medium: Signal | Medium: Pulse |
| Concurrency: read-many, write-serialized | Concurrency: write-many, read-many (broadcast) |

This duality is not cosmetic. It determines when to use which fabric, how to bridge between them, and what consistency guarantees hold at the boundary.

### 1.1 Formal Statement

Let **Sig** be the category of Signals (objects: Signal values, morphisms: lineage edges) and **Pul** be the category of Pulses (objects: Pulse values, morphisms: topic-filtered delivery edges).

The duality is a pair of adjoint functors:

```text
Graduation: F : Pul -> Sig   (left adjoint)
Projection: G : Sig -> Pul   (right adjoint)

F -| G

Unit:   eta_P : P -> G(F(P))     "graduate then project back"
Counit: eps_S : F(G(S)) -> S     "project then graduate back"
```

The unit says: if you graduate a Pulse to a Signal and then project the Signal as a notification Pulse, you get a "cleaned" version of the original Pulse -- one that now has a SignalRef, content hash, and Store identity.

The counit says: if you project a Signal as a Pulse and then some downstream Cell graduates it, you recover the original Signal (up to content-addressing idempotence).

---

## 2. Store Protocol: The Pull Fabric

Store is pull-based: consumers decide when to read, what to query, and how to filter. The Store does not push data to anyone. This makes Store inherently demand-driven.

### 2.1 Contract Recap

From [02-CELL.md](../../unified/02-CELL.md) S2.1:

```rust
pub trait StoreProtocol: Cell {
    async fn put(&self, signal: Signal) -> Result<SignalRef>;
    async fn get(&self, id: &SignalId) -> Result<Option<Signal>>;
    async fn query(&self, query: StoreQuery) -> Result<Vec<Signal>>;
    async fn query_similar(
        &self,
        fingerprint: &HdcVector,
        radius: f32,
        limit: usize,
    ) -> Result<Vec<(SignalRef, f32)>>;
    async fn prune(&self, threshold: f64) -> Result<PruneReport>;
}
```

### 2.2 Store Retention: Demurrage

Signals in Store lose effective weight over time via demurrage (see [01-SIGNAL.md](../../unified/01-SIGNAL.md)). The balance at time `t` for a Signal created at time `t_0` with initial balance `b_0` is:

```rust
/// Balance with demurrage. Signals that are not refreshed
/// (re-scored, re-accessed, re-linked) lose weight and eventually
/// fall below the prune threshold.
fn effective_balance(signal: &Signal, now_ms: i64) -> f64 {
    let age_ms = now_ms - signal.created_at_ms;
    let decay_factor = signal.decay.apply(age_ms);
    signal.score.effective() * decay_factor
}

/// Prune removes Signals below threshold.
/// This is garbage collection driven by demurrage.
async fn prune(store: &dyn StoreProtocol, threshold: f64) -> Result<PruneReport> {
    store.prune(threshold).await
}
```

Demurrage is what makes Store fundamentally different from a log or event store. Signals are alive -- they age, lose weight, and die. The Store is a living memory, not an archive.

### 2.3 HDC Similarity as Native Query

Store supports `query_similar`: given an HDC fingerprint, find the nearest Signals within a Hamming distance radius. This is a native capability, not an external vector-store bolt-on. Every Signal carries a 10,240-bit `HdcVector` fingerprint computed at `put()` time.

```rust
/// HDC similarity query. Returns (ref, distance) pairs sorted by distance.
/// Uses Hamming distance over 10,240-bit vectors.
/// For in-memory and file-backed stores, brute-force scan is viable
/// because popcount is a single instruction per 64-bit word.
async fn query_similar(
    store: &dyn StoreProtocol,
    focus: &HdcVector,
    radius: f32,
    limit: usize,
) -> Result<Vec<(SignalRef, f32)>> {
    store.query_similar(focus, radius, limit).await
}
```

The important property: `query_similar` respects demurrage. A Signal that has decayed below the prune threshold will not be returned even if it is geometrically close. This prevents stale associations from haunting the system.

---

## 3. Bus Protocol: The Push Fabric

Bus is push-based: producers emit Pulses to topics, and the Bus fans them out to all matching subscribers. Consumers declare interest via `TopicFilter` and receive Pulses as they arrive. This makes Bus inherently supply-driven.

### 3.1 Contract Recap

From the target architecture (see source `07b-bus-transport-fabric.md`):

```rust
pub trait Bus: Send + Sync {
    async fn publish(&self, pulse: Pulse) -> Result<u64>;
    async fn subscribe(&self, filter: TopicFilter) -> Result<BusReceiver>;
    async fn replay_since(&self, since_seq: u64, filter: &TopicFilter) -> Result<Vec<Pulse>>;
    async fn current_seq(&self) -> Result<u64>;
    async fn total_published(&self) -> Result<u64>;
    async fn ring_len(&self) -> Result<usize>;
    fn ring_capacity(&self) -> usize;
}
```

### 3.2 Bus Retention: Ring Eviction

Pulses are retained in a bounded ring buffer. When capacity is reached, the oldest Pulse is evicted regardless of content. There is no demurrage -- Pulses do not age. They either exist in the ring or they do not.

```rust
/// Ring buffer semantics.
///
/// The ring is a fixed-size circular buffer. When full, the oldest
/// Pulse is evicted to make room for the newest. There is no
/// demurrage, no scoring, no decay. Eviction is purely positional.
struct RingBuffer {
    capacity: usize,
    buffer: VecDeque<Pulse>,
    next_seq: u64,
}

impl RingBuffer {
    fn push(&mut self, mut pulse: Pulse) -> u64 {
        let seq = self.next_seq;
        pulse.seq = seq;
        self.next_seq += 1;

        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front(); // evict oldest
        }
        self.buffer.push_back(pulse);
        seq
    }

    fn replay_since(&self, since_seq: u64, filter: &TopicFilter) -> Vec<Pulse> {
        self.buffer
            .iter()
            .filter(|p| p.seq > since_seq && filter.matches(&p.topic))
            .cloned()
            .collect()
    }
}
```

### 3.3 Topic Routing

Bus routing is topic-based, not content-based. Topics are dot-separated lowercase strings. The `TopicFilter` language provides exact match, glob patterns, boolean combinators, and the wildcard `All`:

```rust
impl TopicFilter {
    fn matches(&self, topic: &Topic) -> bool {
        match self {
            TopicFilter::Exact(t) => t == topic,
            TopicFilter::Glob(pattern) => glob_match(pattern, topic.as_str()),
            TopicFilter::AnyOf(topics) => topics.contains(topic),
            TopicFilter::All => true,
            TopicFilter::And(a, b) => a.matches(topic) && b.matches(topic),
            TopicFilter::Or(a, b) => a.matches(topic) || b.matches(topic),
            TopicFilter::Not(inner) => !inner.matches(topic),
        }
    }
}
```

Topic naming convention (from the source architecture):

```text
{subsystem}.{noun}.{verb}

Examples:
  verify.verdict.passed       -- a Verify Cell passed a Signal
  verify.verdict.failed       -- a Verify Cell failed a Signal
  store.signal.written        -- a Signal was persisted to Store
  agent.msg.chunk             -- an LLM produced a streaming chunk
  cost.charged                -- budget was consumed
  prediction.{block_id}       -- a Cell published a prediction
  outcome.{block_id}          -- reality published an outcome
  calibration.{block_id}.updated -- calibration was recomputed
  safety.capability.denied    -- a capability check failed
```

---

## 4. Graduation: Pulse to Signal

Graduation is the bridge from ephemeral to durable. A Pulse becomes a Signal when:

1. A React Cell determines the Pulse carries information worth preserving.
2. The React Cell constructs a Signal from the Pulse's content.
3. The Signal is written to Store via `StoreProtocol::put`.

### 4.1 Graduation Policy as a React Cell

The graduation decision is not automatic -- it is a React Cell that subscribes to Bus topics and selectively persists:

```rust
/// Graduation policy: a React Cell that watches Bus topics and
/// selectively persists Pulses as durable Signals.
///
/// This is the canonical bridge from ephemeral to durable.
/// Different policies graduate different traffic based on
/// topic, content, rate, or downstream demand.
struct GraduationPolicy {
    /// Topics this policy watches.
    subscription: TopicFilter,

    /// Graduation criteria.
    criteria: GraduationCriteria,
}

struct GraduationCriteria {
    /// Graduate every N-th Pulse (sampling).
    sample_rate: Option<usize>,

    /// Graduate if Pulse body exceeds this size (significance).
    min_body_size: Option<usize>,

    /// Graduate if topic matches these patterns (category).
    always_graduate: Vec<TopicFilter>,

    /// Never graduate these topics (noise suppression).
    never_graduate: Vec<TopicFilter>,

    /// Graduate on verdict failure (forensic preservation).
    graduate_on_failure: bool,

    /// Graduate after this many Pulses accumulate on the same
    /// lineage_hint (batched aggregation).
    batch_threshold: Option<usize>,
}

#[async_trait]
impl ReactProtocol for GraduationPolicy {
    async fn react(
        &self,
        pulses: &[Pulse],
        ctx: &ReactContext,
    ) -> Result<ReactOutput> {
        let mut graduated_signals = Vec::new();

        for pulse in pulses {
            if self.should_graduate(pulse) {
                let signal = Signal::from_pulse(pulse);
                graduated_signals.push(signal);
            }
        }

        // Persist the graduated Signals.
        for signal in &graduated_signals {
            ctx.store.put(signal.clone()).await?;
        }

        Ok(ReactOutput {
            pulses: vec![], // no downstream Pulses needed
            signals: graduated_signals,
        })
    }

    fn subscription(&self) -> TopicFilter {
        self.subscription.clone()
    }
}

impl GraduationPolicy {
    fn should_graduate(&self, pulse: &Pulse) -> bool {
        // Never-graduate takes precedence
        if self.criteria.never_graduate.iter().any(|f| f.matches(&pulse.topic)) {
            return false;
        }

        // Always-graduate topics
        if self.criteria.always_graduate.iter().any(|f| f.matches(&pulse.topic)) {
            return true;
        }

        // Sampling
        if let Some(rate) = self.criteria.sample_rate {
            if pulse.seq % rate as u64 != 0 {
                return false;
            }
        }

        // Body size threshold
        if let Some(min) = self.criteria.min_body_size {
            if pulse.body.len() < min {
                return false;
            }
        }

        // Default: do not graduate (ephemeral by default)
        false
    }
}
```

### 4.2 Signal Construction from Pulse

```rust
impl Signal {
    /// Construct a durable Signal from an ephemeral Pulse.
    /// The Signal inherits the Pulse's content but gains:
    /// - a ContentHash identity
    /// - an HDC fingerprint
    /// - a Score (initially estimated, refined by Score Cells)
    /// - a decay curve (begins demurrage from graduation time)
    fn from_pulse(pulse: &Pulse) -> Self {
        let body = pulse.body.clone();
        let kind = pulse.kind.clone();

        // Compute HDC fingerprint from content
        let fingerprint = HdcVector::encode_body(&body);

        // Initial score is a rough estimate. A Score Cell will
        // refine this later. The graduation policy's job is just
        // to decide "worth keeping", not "how good."
        let score = Score::estimated_from_kind(&kind);

        Signal {
            id: SignalId::content_hash(&kind, &body),
            kind,
            body,
            score,
            decay: Decay::default(), // standard demurrage curve
            fingerprint: Some(fingerprint),
            created_at_ms: pulse.emitted_at_ms,
            lineage: pulse.lineage_hint.map(|h| vec![h]).unwrap_or_default(),
            tags: BTreeMap::new(),
        }
    }
}
```

---

## 5. Projection: Signal to Pulse

Projection is the bridge from durable to ephemeral. When a Signal is written to Store, the Store (or a bridge Cell) publishes a notification Pulse on the Bus:

```rust
/// Projection bridge: after a successful Store write, publish a
/// notification Pulse on Bus so that React Cells can observe
/// Store mutations without polling.
///
/// This is the dual of graduation. Where graduation converts
/// Pulse content into Signal identity, projection converts
/// Signal identity into Pulse notification.
async fn project_store_write(
    signal: &Signal,
    signal_ref: &SignalRef,
    bus: &dyn Bus,
) -> Result<()> {
    let pulse = Pulse {
        seq: 0, // assigned by Bus on publish
        topic: Topic::from("store.signal.written"),
        kind: signal.kind.clone(),
        body: Body::json(&StoreWriteNotification {
            signal_ref: signal_ref.clone(),
            kind: signal.kind.clone(),
            tags: signal.tags.clone(),
            score_effective: signal.score.effective(),
        }),
        emitted_at_ms: chrono::Utc::now().timestamp_millis(),
        source: PulseSource {
            component: "store:projection".into(),
            agent_id: None,
        },
        lineage_hint: Some(signal.id.clone()),
        trace_id: None,
    };

    bus.publish(pulse).await?;
    Ok(())
}

/// The notification payload. Carries enough metadata for a React
/// Cell to decide whether it cares, without carrying the full Signal
/// body (which lives in Store and can be fetched on demand).
#[derive(Serialize, Deserialize)]
struct StoreWriteNotification {
    signal_ref: SignalRef,
    kind: Kind,
    tags: BTreeMap<String, String>,
    score_effective: f64,
}
```

### 5.1 Projection Is Notification, Not Replication

A critical distinction: projection publishes a notification Pulse, not a copy of the Signal. The Pulse carries the `SignalRef` and metadata, not the full `Signal` body. A React Cell that needs the full Signal fetches it from Store using the `SignalRef`.

This means:

- **Bus bandwidth is bounded**: notifications are small regardless of Signal size.
- **Store remains the source of truth**: the Pulse is a pointer, not a copy.
- **Late subscribers query Store directly**: they do not need the notification Pulse to access the Signal.

---

## 6. The Decision Tree: Store or Bus?

Given a piece of information, which fabric should carry it?

```text
Does the information need to survive process restart?
  |
  +-- YES --> Is it content-addressed (same input = same identity)?
  |             |
  |             +-- YES --> STORE (Signal)
  |             |
  |             +-- NO  --> STORE, but reconsider the identity model.
  |                         Maybe it should be content-addressed.
  |
  +-- NO  --> Is it a notification about something durable?
                |
                +-- YES --> BUS (Pulse) with lineage_hint pointing to the
                |           Signal. This is projection.
                |
                +-- NO  --> Is it ephemeral coordination traffic?
                              |
                              +-- YES --> BUS (Pulse) on the appropriate topic.
                              |           Do NOT graduate unless forensic value
                              |           emerges.
                              |
                              +-- NO  --> Probably not worth persisting or
                                          transporting. Drop it.
```

### 6.1 Examples

| Information | Fabric | Why |
|---|---|---|
| Compiled code artifact | Store | Durable, content-addressed, has lineage |
| "Agent X started task Y" | Bus | Ephemeral coordination, no durable value |
| Verify verdict | Both | Published on Bus for reactive Cells, graduated to Store for audit |
| LLM streaming chunk | Bus | Ephemeral, high-frequency, compose the final output later |
| Final composed prompt | Store | Durable, used for replay and audit |
| Heartbeat tick | Bus | Pure ephemeral coordination, never graduate |
| Gate failure at rung 3 | Both | Bus for immediate React, Store for learning and forensics |
| Cost accounting event | Store | Must survive restart for budget enforcement |
| Calibration update | Bus | Ephemeral notification; the CalibrationTable is the durable artifact |

---

## 7. Consistency at the Boundary

The boundary between Store and Bus is where consistency questions arise. The core question: **what happens when a graduated Signal is written to Store but the originating Pulse has already been evicted from the Bus ring?**

### 7.1 The Eviction Window Problem

```text
Timeline:
  t1: Pulse P is published on Bus                    [P in ring]
  t2: React Cell R subscribes and starts processing  [P in ring]
  t3: Ring fills up, P is evicted                    [P NOT in ring]
  t4: React Cell R graduates P as Signal S           [S in Store]
  t5: Another subscriber S2 calls replay_since(t0)  [P NOT in ring, S2 misses it]
  t6: S2 queries Store and finds S                   [S2 finds the graduated Signal]
```

The scenario is not a bug -- it is the design working correctly:

- **P is ephemeral**: its eviction from the ring is expected.
- **S is durable**: once graduated, it lives in Store regardless of P's fate.
- **S2 can find S via Store query**: the Store query does not depend on the Bus ring.
- **The projection Pulse for S**: when S was put into Store at t4, a projection Pulse was published on `store.signal.written`. If S2's subscription includes that topic, S2 sees the projection Pulse even though P is gone.

### 7.2 Consistency Guarantees

The system provides the following consistency model:

```rust
/// Consistency guarantees at the Store/Bus boundary.
///
/// 1. STORE-FIRST: graduation writes to Store before publishing
///    any downstream Pulses. If the Store write fails, no Pulse
///    is emitted. This prevents phantom notifications.
///
/// 2. PROJECTION-BEST-EFFORT: the projection Pulse after a Store
///    write is best-effort. If Bus publishing fails, the Signal is
///    still in Store. Late subscribers can always query Store directly.
///    No Signal is lost because a Pulse failed.
///
/// 3. IDEMPOTENT-GRADUATION: graduating the same Pulse twice produces
///    the same SignalRef (content-addressed). The Store treats it as
///    a no-op. This means retries are safe.
///
/// 4. RING-EVICTION-IS-NOT-DATA-LOSS: eviction from the Bus ring
///    only removes the Pulse from the replay window. If the Pulse
///    was graduated, the Signal is in Store. If it was not graduated,
///    it was deemed ephemeral by the graduation policy. Either way,
///    no durable information is lost.
async fn graduate_with_guarantees(
    pulse: &Pulse,
    store: &dyn StoreProtocol,
    bus: &dyn Bus,
) -> Result<SignalRef> {
    // 1. Store-first: persist before notifying
    let signal = Signal::from_pulse(pulse);
    let signal_ref = store.put(signal.clone()).await?;

    // 2. Projection-best-effort: notify but don't fail if Bus is down
    let _ = project_store_write(&signal, &signal_ref, bus).await;

    // 3. Idempotent: calling again with same pulse returns same ref
    Ok(signal_ref)
}
```

### 7.3 The Replay-Query Equivalence

For any subscriber that needs to see "everything that happened," there are two equivalent strategies:

1. **Bus replay + Store query**: call `replay_since(checkpoint_seq)` on Bus for recent Pulses, then `query` Store for anything that might have been evicted.

2. **Store-only query**: just query Store with a time range. This works because all durable information was graduated by some graduation policy.

The strategies are equivalent for durable information. They differ for ephemeral-only Pulses (heartbeats, streaming chunks, etc.) that were never graduated. Bus replay catches these; Store query does not. If ephemeral Pulses matter for catch-up, increase the ring capacity.

```rust
/// Catch-up strategy for a subscriber that reconnects at a checkpoint.
async fn catch_up(
    checkpoint_seq: u64,
    checkpoint_time: i64,
    bus: &dyn Bus,
    store: &dyn StoreProtocol,
    filter: &TopicFilter,
) -> Result<CatchUpResult> {
    // Strategy 1: Bus replay for recent ephemeral + durable
    let bus_pulses = bus.replay_since(checkpoint_seq, filter).await?;

    // Strategy 2: Store query for anything that might have been evicted
    let store_signals = store.query(StoreQuery {
        since_ms: Some(checkpoint_time),
        ..Default::default()
    }).await?;

    // Merge: Bus Pulses that have lineage_hint matching a Store Signal
    // are deduplicated. Pulses without a Store match are ephemeral-only.
    let merged = merge_bus_and_store(bus_pulses, store_signals);

    Ok(merged)
}
```

---

## 8. Backpressure Strategies as Cell Configuration

When a publisher emits Pulses faster than subscribers can consume them, backpressure arises. The Bus does not have built-in backpressure (it is fire-and-forget broadcast). Instead, backpressure is handled by configuring the Cells around the Bus.

### 8.1 Ring Capacity Sizing

The simplest backpressure control: size the ring buffer to absorb burst traffic.

```toml
# roko.toml -- Bus configuration
[bus]
ring_capacity = 16384     # default: 8192
# Rule of thumb: 2x the expected peak burst size.
# If agents produce ~100 Pulses/sec and subscribers
# process at ~50 Pulses/sec, a 10-second burst needs
# at least 1000 slots of buffer. 16384 gives ~160 seconds.
```

### 8.2 Sampling React Cells

A React Cell that cannot keep up can sample instead of processing every Pulse:

```rust
/// A sampling React Cell that processes every N-th Pulse.
/// Uses reservoir sampling to maintain statistical properties
/// even under high throughput.
struct SamplingReactCell<R: ReactProtocol> {
    inner: R,
    sample_rate: usize,
    counter: AtomicU64,
}

#[async_trait]
impl<R: ReactProtocol> ReactProtocol for SamplingReactCell<R> {
    async fn react(
        &self,
        pulses: &[Pulse],
        ctx: &ReactContext,
    ) -> Result<ReactOutput> {
        let sampled: Vec<&Pulse> = pulses
            .iter()
            .filter(|_| {
                let n = self.counter.fetch_add(1, Ordering::Relaxed);
                n % self.sample_rate as u64 == 0
            })
            .collect();

        if sampled.is_empty() {
            return Ok(ReactOutput::empty());
        }

        // Delegate to the inner Cell with the sampled subset
        let sampled_owned: Vec<Pulse> = sampled.into_iter().cloned().collect();
        self.inner.react(&sampled_owned, ctx).await
    }

    fn subscription(&self) -> TopicFilter {
        self.inner.subscription()
    }
}
```

### 8.3 Windowed Aggregation

Instead of processing individual Pulses, aggregate over a time window:

```rust
/// A windowed aggregation React Cell. Collects Pulses for a
/// window duration, then processes the batch. Reduces per-Pulse
/// overhead and naturally applies backpressure by buffering.
struct WindowedReactCell<R: ReactProtocol> {
    inner: R,
    window_duration: Duration,
    buffer: Mutex<Vec<Pulse>>,
    last_flush: AtomicI64,
}

#[async_trait]
impl<R: ReactProtocol> ReactProtocol for WindowedReactCell<R> {
    async fn react(
        &self,
        pulses: &[Pulse],
        ctx: &ReactContext,
    ) -> Result<ReactOutput> {
        let mut buffer = self.buffer.lock().await;
        buffer.extend_from_slice(pulses);

        let now = chrono::Utc::now().timestamp_millis();
        let last = self.last_flush.load(Ordering::Relaxed);

        if now - last >= self.window_duration.as_millis() as i64 {
            self.last_flush.store(now, Ordering::Relaxed);
            let batch: Vec<Pulse> = buffer.drain(..).collect();
            drop(buffer); // release lock before processing

            self.inner.react(&batch, ctx).await
        } else {
            Ok(ReactOutput::empty())
        }
    }

    fn subscription(&self) -> TopicFilter {
        self.inner.subscription()
    }
}
```

### 8.4 Priority-Based Eviction

For Buses that serve mixed traffic (critical verdicts + noisy heartbeats), priority-based eviction preserves important Pulses longer:

```rust
/// A priority ring buffer that evicts low-priority Pulses first.
/// Priority is determined by topic: `verify.verdict.*` has higher
/// priority than `agent.heartbeat.*`.
struct PriorityRingBuffer {
    capacity: usize,
    buffer: BTreeMap<(Priority, u64), Pulse>, // (priority, seq) -> Pulse
    next_seq: u64,
    topic_priorities: HashMap<TopicFilter, Priority>,
}

impl PriorityRingBuffer {
    fn push(&mut self, mut pulse: Pulse) -> u64 {
        let seq = self.next_seq;
        pulse.seq = seq;
        self.next_seq += 1;

        let priority = self.priority_for(&pulse.topic);

        if self.buffer.len() >= self.capacity {
            // Evict the lowest-priority, oldest Pulse
            if let Some((&key, _)) = self.buffer.iter().next() {
                self.buffer.remove(&key);
            }
        }

        self.buffer.insert((priority, seq), pulse);
        seq
    }

    fn priority_for(&self, topic: &Topic) -> Priority {
        for (filter, priority) in &self.topic_priorities {
            if filter.matches(topic) {
                return *priority;
            }
        }
        Priority::default()
    }
}
```

---

## 9. The BroadcastBus Implementation

The default in-process Bus implementation wraps `tokio::sync::broadcast` with a replay ring:

```rust
/// BroadcastBus: default single-process Bus implementation.
///
/// Uses tokio broadcast for fan-out and a VecDeque ring for replay.
/// This is the target replacement for today's concrete EventBus<E>.
pub struct BroadcastBus {
    sender: broadcast::Sender<Pulse>,
    ring: RwLock<RingBuffer>,
    total_published: AtomicU64,
}

impl BroadcastBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            ring: RwLock::new(RingBuffer::new(capacity)),
            total_published: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl Bus for BroadcastBus {
    async fn publish(&self, pulse: Pulse) -> Result<u64> {
        let mut ring = self.ring.write().await;
        let seq = ring.push(pulse.clone());
        self.total_published.fetch_add(1, Ordering::Relaxed);

        // Best-effort broadcast. If no subscribers, that is fine.
        let _ = self.sender.send(pulse);
        Ok(seq)
    }

    async fn subscribe(&self, filter: TopicFilter) -> Result<BusReceiver> {
        let rx = self.sender.subscribe();
        Ok(BusReceiver::new(rx, filter))
    }

    async fn replay_since(
        &self,
        since_seq: u64,
        filter: &TopicFilter,
    ) -> Result<Vec<Pulse>> {
        let ring = self.ring.read().await;
        Ok(ring.replay_since(since_seq, filter))
    }

    async fn current_seq(&self) -> Result<u64> {
        let ring = self.ring.read().await;
        Ok(ring.next_seq.saturating_sub(1))
    }

    async fn total_published(&self) -> Result<u64> {
        Ok(self.total_published.load(Ordering::Relaxed))
    }

    async fn ring_len(&self) -> Result<usize> {
        let ring = self.ring.read().await;
        Ok(ring.buffer.len())
    }

    fn ring_capacity(&self) -> usize {
        self.ring.blocking_read().capacity
    }

    fn name(&self) -> &'static str {
        "broadcast_bus"
    }
}
```

---

## 10. Distributed Bus

For multi-process deployments, the Bus trait abstracts over distributed transports:

```text
Single-process:
  BroadcastBus (tokio broadcast + VecDeque ring)

Multi-process, same datacenter:
  NatsBus (NATS JetStream, topic -> subject mapping)
  KafkaBus (Kafka/Redpanda, topic -> partition key)

Multi-process, cross-datacenter:
  MultiBus (fan-in from multiple Bus backends)

On-chain:
  ChainBus (topics -> contract event logs, replay by block scan)
```

All backends implement the same `Bus` trait. A `MultiBus` aggregates multiple backends into a single Bus surface:

```rust
/// MultiBus: aggregates multiple Bus backends.
/// Publish goes to all backends. Subscribe merges all streams.
struct MultiBus {
    backends: Vec<Arc<dyn Bus>>,
}

#[async_trait]
impl Bus for MultiBus {
    async fn publish(&self, pulse: Pulse) -> Result<u64> {
        // Publish to all backends. Use the first seq as the canonical one.
        let mut first_seq = None;
        for backend in &self.backends {
            let seq = backend.publish(pulse.clone()).await?;
            if first_seq.is_none() {
                first_seq = Some(seq);
            }
        }
        Ok(first_seq.unwrap_or(0))
    }

    async fn subscribe(&self, filter: TopicFilter) -> Result<BusReceiver> {
        // Merge all backend streams into one.
        let receivers: Vec<BusReceiver> = futures::future::try_join_all(
            self.backends.iter().map(|b| b.subscribe(filter.clone()))
        ).await?;
        Ok(BusReceiver::merged(receivers))
    }

    // replay_since, current_seq, etc. aggregate across backends
    // with deduplication by Pulse content hash.
}
```

---

## What This Enables

1. **Clean separation of concerns**: Store handles durable memory, Bus handles live coordination. No subsystem is forced to pretend ephemeral traffic is durable or vice versa.

2. **Cross-layer decoupling**: Cells communicate through Bus topics instead of direct crate dependencies. The old `roko-conductor -> roko-learn` coupling is dissolved by publishing `verify.verdict.failed` on Bus rather than importing learning types.

3. **Graduated memory**: Important ephemeral traffic becomes durable through explicit graduation policies. The graduation decision is a first-class Cell, not an implicit side effect.

4. **Replay and catch-up**: Subscribers that disconnect briefly can catch up from the ring buffer. Subscribers that disconnect for longer fall through to Store queries. The two strategies compose seamlessly.

5. **Backpressure without blocking**: Bus is non-blocking broadcast. Backpressure is handled structurally by configuring sampling, windowing, priority eviction, or ring capacity. No Cell ever blocks the publisher.

---

## Feedback Loops

- **Ring capacity tuning**: The system tracks how often `replay_since` returns an empty or truncated result (indicating the subscriber fell behind the ring). When this exceeds a threshold, the ring capacity is increased automatically. This is a Lens (Observe protocol Cell) publishing `bus.ring.lag` Pulses, consumed by a React Cell that adjusts configuration.

- **Graduation policy refinement**: The graduation policy's criteria are updated by the predict-publish-correct Loop. Before graduating, the policy publishes a prediction ("this Pulse will be queried within 1 hour"). After an hour, reality is checked. If graduated Signals are never queried, the criteria are tightened. If un-graduated Pulses are missed, the criteria are loosened.

- **Backpressure adaptation**: Sampling and windowing parameters are adjusted based on measured subscriber lag. A subscriber that consistently keeps up has its sample rate reduced (more Pulses processed). A subscriber that falls behind has its sample rate increased.

---

## Open Questions

1. **Exactly-once graduation**: The current model is at-least-once (idempotent via content hash). Is there a use case for exactly-once semantics where even the idempotent re-write has observable cost (e.g., triggering a projection Pulse twice)?

2. **Cross-datacenter consistency**: When a MultiBus spans datacenters, different backends may have different views of the Pulse stream. What is the convergence model? Is eventual consistency sufficient, or do some topics (e.g., `safety.capability.denied`) need stronger guarantees?

3. **Store-as-Bus**: Could a Store implementation also implement Bus by treating writes as publications? This would collapse the duality into a single unified fabric. The cost would be losing the distinction between ephemeral and durable -- every Pulse would be persisted. Is this ever the right tradeoff (e.g., for audit-heavy deployments)?

4. **Bus ordering guarantees**: The current model guarantees per-publisher ordering (a publisher's Pulses arrive in publish order). Does the system need total ordering across publishers? For most React Cells, per-publisher ordering suffices, but some aggregate Cells (circuit breakers, budget enforcers) might need total order to reason correctly about interleaved traffic.
