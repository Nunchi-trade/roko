# Topic Namespace and Grammar

> Depth for [11-CONNECTIVITY.md](../../v2/11-CONNECTIVITY.md). Dot-separated hierarchical topic grammar, canonical topic namespace, wildcard subscription patterns, and topic lifecycle management.

**Depends on**: [01-SIGNAL](../../v2/01-SIGNAL.md) (Pulse, Bus topics), [09-FEEDS](../../v2/09-FEEDS.md) (Feed topic conventions), [10-GROUPS](../../v2/10-GROUPS.md) (Group Bus partitions)

---

## 1. Grammar

Topics are the addressing primitive for all Pulse traffic on the Bus. Every `Pulse` carries a `Topic` that determines which subscribers receive it. Topics follow a dot-separated hierarchical grammar that enables prefix-based filtering, segment-aware wildcards, and URL-safe embedding in REST paths.

### Formal grammar

```
topic        = segment *("." segment)
segment      = 1*(ALPHA / DIGIT / "-" / "_")
ALPHA        = %x61-7A                         ; lowercase a-z
DIGIT        = %x30-39                         ; 0-9
```

### Rules

1. **Dot-separated segments.** Each dot delimits a segment boundary. No dots within segments.
2. **Segment characters.** Alphanumeric (`a-z`, `0-9`), hyphens (`-`), and underscores (`_`). No uppercase, no spaces, no special characters.
3. **Case-sensitive, lowercase by convention.** `chain.31337` and `Chain.31337` are different topics. All canonical topics use lowercase.
4. **Maximum 8 segments.** `a.b.c.d.e.f.g.h` is valid. `a.b.c.d.e.f.g.h.i` is rejected.
5. **Maximum 256 bytes total.** The entire topic string (including dots) must fit in 256 bytes UTF-8.
6. **No empty segments.** `chain..31337` and `.chain` and `chain.` are invalid.
7. **Dynamic creation.** Subscribing to or publishing on a topic creates it implicitly. No registration step.

### Validation

```rust
/// Validate a topic string against the grammar rules.
pub fn validate_topic(topic: &str) -> Result<(), TopicError> {
    if topic.is_empty() { return Err(TopicError::Empty); }
    if topic.len() > 256 { return Err(TopicError::TooLong { len: topic.len(), max: 256 }); }

    let segments: Vec<&str> = topic.split('.').collect();
    if segments.len() > 8 {
        return Err(TopicError::TooManySegments { count: segments.len(), max: 8 });
    }
    for (i, seg) in segments.iter().enumerate() {
        if seg.is_empty() { return Err(TopicError::EmptySegment { position: i }); }
        if !seg.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
            return Err(TopicError::InvalidCharacter { segment: seg.to_string(), position: i });
        }
    }
    Ok(())
}
```

### Examples

| Topic | Valid | Reason |
|---|---|---|
| `chain.31337` | Yes | Two segments, alphanumeric |
| `group.abc-123.coordination` | Yes | Three segments, hyphens allowed |
| `feed.defi_metrics.gas-trend` | Yes | Underscores and hyphens |
| `Chain.31337` | No | Uppercase not allowed |
| `chain..31337` | No | Empty segment |
| `a.b.c.d.e.f.g.h.i` | No | 9 segments exceeds max of 8 |

---

## 2. Why Dots

The dot-separated topic grammar is a deliberate choice. Four reasons.

### 2.1 Industry convention

NATS, RabbitMQ, and Kafka all use dot-separated (or slash-separated with equivalent semantics) topic hierarchies. Adopting the same grammar means operators recognize the pattern, bridging to external brokers requires no topic transformation, and debugging tools behave predictably.

### 2.2 Segment-aware wildcard matching

Dots create explicit segment boundaries that wildcards can operate on (section 4). A prefix match on `chain.` matches `chain.31337` but not `chain_rpc.eth`. Without segment boundaries, prefix matching requires careful escaping to avoid false positives.

### 2.3 URL-safe embedding

Topics appear directly in REST paths for relay and control plane endpoints:

```
GET /relay/topics/chain.31337/messages
GET /relay/topics/group.abc-123.coordination/subscribers
```

Dots are valid in URL path segments without encoding. Slashes would require escaping. Colons conflict with port numbers and some proxy configurations.

### 2.4 Consistency with internal Bus addressing

The `Topic` struct in `roko-core` already uses dot-separated strings:

```rust
let topic = Topic::new("gate.verdict.emitted");
assert!(topic.starts_with("gate.verdict"));
```

The `TopicFilter::Prefix` variant performs string-prefix matching on these dotted topics. The grammar formalizes what the codebase already practices.

---

## 3. Canonical Topic Namespace

All standard topics are defined here. Agents and extensions must use these topics for interoperability. Custom topics (section 7) use separate prefixes.

### 3.1 Chain events

| Topic | Payload | Publisher |
|---|---|---|
| `chain.{chain_id}` | Block headers, contract events, finality updates | `ChainRpcConnector` |

The `{chain_id}` is the decimal chain ID: `chain.1` for Ethereum mainnet, `chain.8453` for Base, `chain.42161` for Arbitrum.

```rust
/// Publish a new block Pulse on the chain topic.
let topic = Topic::new(format!("chain.{}", self.chain_id));
let pulse = Pulse::builder(seq, topic, Kind::ChainEvent)
    .body(Body::json(&block_header)?)
    .tag("block_number", block.number.to_string())
    .tag("finality", "reversible")
    .build();
bus.publish(pulse).await?;
```

### 3.2 ISFR (Interest-and-Settlement-Free Rate)

| Topic | Payload | Publisher |
|---|---|---|
| `isfr.rates` | Composite rate update with source weights | ISFR keeper |
| `isfr.epochs` | Epoch transition markers (start, end, settlement) | ISFR keeper |

### 3.3 Jobs and marketplace

| Topic | Payload | Publisher |
|---|---|---|
| `job.posted` | New job announcements (from chain events or API) | Job watcher |
| `job.{job_id}.status` | Per-job lifecycle: accepted, started, completed, disputed | Job executor |

The `{job_id}` is a lowercase identifier (ULID or hex hash). Using a per-job topic allows subscribers to follow a single job without receiving traffic for all jobs.

### 3.4 Agent presence

| Topic | Payload | Publisher |
|---|---|---|
| `agent.presence` | Heartbeat, online/offline transitions, vitality snapshots | Agent runtime |

All agents publish to the same `agent.presence` topic. The Pulse body includes the `agent_id` for demultiplexing.

```rust
/// Agent heartbeat Pulse.
let pulse = Pulse::builder(seq, Topic::new("agent.presence"), Kind::Heartbeat)
    .body(Body::json(&AgentPresence {
        agent_id: self.id.clone(),
        status: PresenceStatus::Online,
        vitality: self.vitality(),
        tick: self.current_tick,
    })?)
    .build();
bus.publish(pulse).await?;
```

### 3.5 Feeds

| Topic | Payload | Publisher |
|---|---|---|
| `feed.{domain}.{name}` | Domain-specific feed data (blocks, prices, metrics) | Connector cells |
| `feed.meta.relay` | Relay internal statistics (subscriber counts, throughput) | Relay |

The `{domain}` identifies the data source category. The `{name}` identifies the specific feed.

```rust
/// Register a feed and publish on its canonical topic.
let topic = Topic::new("feed.eth.gas-trend");
ctx.relay.register_feed(FeedRegistration {
    feed_id: "eth-gas-trend",
    topic: topic.clone(),
    kind: FeedKind::Derived,
    schema: FeedSchema::GasTrend,
    rate_hz: 0.2,
    access: FeedAccess::Public,
})?;
```

### 3.6 Groups

Each group gets a namespace under `group.{id}`.

| Topic | Payload | Publisher |
|---|---|---|
| `group.{id}` | Group-wide broadcast (announcements, configuration) | Group leader |
| `group.{id}.knowledge` | Knowledge publish and validation events | Group members |
| `group.{id}.pheromones` | Pheromone deposit and decay events | Group members |
| `group.{id}.coordination` | Task assignment, completion, handoff events | Coordinator cell |

All group members subscribe to `group.{id}` for broadcast. Specialized cells subscribe to the sub-topics they care about.

### 3.7 Workspace

| Topic | Payload | Publisher |
|---|---|---|
| `workspace.{id}` | Workspace lifecycle (started, stopped, config changed) | Workspace runtime |

### 3.8 System

| Topic | Payload | Publisher |
|---|---|---|
| `system.relay` | Relay health metrics (uptime, connections, throughput) | Relay |

---

## 4. Wildcard Subscriptions (Future)

The current Bus implementation requires exact topic matching via `TopicFilter::Exact` or string-prefix matching via `TopicFilter::Prefix`. A future iteration adds segment-aware wildcard matching using NATS-style tokens.

### 4.1 Single-segment wildcard: `*`

Matches exactly one segment at the position where it appears.

| Pattern | Matches | Does not match |
|---|---|---|
| `chain.*` | `chain.31337`, `chain.8453` | `chain.31337.block`, `chain` |
| `group.*.coordination` | `group.abc123.coordination` | `group.coordination`, `group.a.b.coordination` |
| `feed.*.gas-trend` | `feed.eth.gas-trend` | `feed.gas-trend`, `feed.eth.arb.gas-trend` |

### 4.2 Multi-segment wildcard: `>`

Matches one or more segments. Must appear as the last token (terminal position only).

| Pattern | Matches | Does not match |
|---|---|---|
| `chain.>` | `chain.31337`, `chain.31337.block` | `chain` |
| `group.abc123.>` | `group.abc123.knowledge`, `group.abc123.coordination` | `group.abc123`, `group.xyz.knowledge` |
| `feed.>` | `feed.eth.gas-trend`, `feed.meta.relay` | `feed` |

### 4.3 Prefix filter vs wildcards

The existing `TopicFilter::Prefix` performs byte-level prefix matching. Wildcards operate on segment boundaries. The two differ:

```rust
// Prefix: matches anything starting with "chain." (byte-level)
let prefix = TopicFilter::Prefix("chain.".into());

// Wildcard *: matches exactly one segment after "chain." (segment-level)
let wildcard = TopicFilter::Wildcard("chain.*".into());  // future

// Wildcard >: matches one or more segments after "chain." (segment-level)
let multi = TopicFilter::WildcardMulti("chain.>".into()); // future
```

When wildcards are implemented, they will be added as new `TopicFilter` variants alongside the existing `Exact`, `Prefix`, `All`, `And`, `Or`, and `Not` variants. Existing variants remain unchanged.

### 4.4 Matching algorithm

Wildcard matching decomposes the topic and pattern into segment arrays, then matches segment-by-segment:

```rust
/// Segment-aware wildcard matching (future implementation).
fn wildcard_matches(pattern: &str, topic: &str) -> bool {
    let pat_segs: Vec<&str> = pattern.split('.').collect();
    let top_segs: Vec<&str> = topic.split('.').collect();

    for (i, pat) in pat_segs.iter().enumerate() {
        match *pat {
            ">" => return i < top_segs.len(),           // terminal: match rest
            "*" if i >= top_segs.len() => return false,  // no segment to match
            "*" => {}                                    // one segment consumed
            lit if i >= top_segs.len() || top_segs[i] != lit => return false,
            _ => {}                                      // literal match
        }
    }
    pat_segs.len() == top_segs.len()
}
```

---

## 5. Topic Lifecycle

Topics are ephemeral by default. No explicit creation or deletion API exists. The lifecycle is driven entirely by usage.

### 5.1 Creation

A topic is created implicitly the first time any of these occurs:

- A Pulse is published to the topic via `Bus::publish`.
- A subscriber registers a `TopicFilter::Exact` for the topic.
- A feed registers with the topic via `relay.register_feed`.

There is no `create_topic` call. The Bus maintains an internal topic set that grows as new topics are referenced.

### 5.2 Garbage collection

A topic becomes eligible for garbage collection when all three conditions hold:

1. Subscriber count drops to zero.
2. Ring buffer is empty (all Pulses evicted by newer entries).
3. A configurable TTL has elapsed since the last publish (default: 5 minutes).

The TTL prevents thrashing when a publisher temporarily pauses.

```rust
/// Topic metadata tracked by the Bus for lifecycle management.
struct TopicState {
    subscriber_count: usize,
    last_publish_ms: i64,
    permanent: bool,
}

impl TopicState {
    fn is_gc_eligible(&self, now_ms: i64, ttl_ms: i64) -> bool {
        !self.permanent
            && self.subscriber_count == 0
            && (now_ms - self.last_publish_ms) > ttl_ms
    }
}
```

### 5.3 Permanent topics

System and chain topics are never garbage collected. They are identified by prefix and marked `permanent: true` at creation time:

- `system.*` -- relay and infrastructure health.
- `chain.*` -- chain watcher events.
- `isfr.*` -- rate oracle events.

---

## 6. Reserved Prefixes

Certain topic prefixes are reserved for system components. Agents must not publish to reserved topics. The Bus enforces this at publish time.

| Prefix | Owner | Purpose |
|---|---|---|
| `system.*` | Relay | Infrastructure health, relay metrics, provider status |
| `chain.*` | Chain watcher | Block events, finality updates, reorg notifications |
| `isfr.*` | ISFR keeper | Rate updates, epoch transitions |
| `feed.meta.*` | Relay | Feed registry metadata, relay-internal statistics |

```rust
/// Reserved topic prefixes that agents cannot publish to.
const RESERVED_PREFIXES: &[&str] = &["system.", "chain.", "isfr.", "feed.meta."];

/// Check whether a topic is in a reserved namespace.
pub fn reserved_prefix(topic: &Topic) -> Option<&'static str> {
    RESERVED_PREFIXES.iter().find(|p| topic.starts_with(p)).copied()
}
```

Attempts to publish to a reserved topic from agent code return `Err(TopicError::Reserved)`. System components bypass this check through a privileged `BusHandle` that carries a `SystemPublisher` capability.

---

## 7. Custom Topics

Agents and extensions can create arbitrary topics for application-specific use, provided the topic does not fall under a reserved prefix.

### Convention

Use the agent's domain or application name as the first segment to avoid collisions:

| Pattern | Example | Use case |
|---|---|---|
| `{domain}.events` | `defi.events` | Domain-specific event stream |
| `{domain}.alerts` | `monitoring.alerts` | Alert notifications |
| `{domain}.{subsystem}.{event}` | `trading.orders.filled` | Fine-grained event topics |
| `{app}.internal.{channel}` | `mybot.internal.debug` | Application-internal channels |

### Guidelines

1. **Namespace your topics.** Always use at least two segments. Single-segment topics risk collision with future system topics.
2. **Document your topics.** Declare custom topics in the extension manifest for operator discoverability.
3. **Respect the grammar.** Custom topics follow the same rules (section 1): lowercase, max 8 segments, max 256 bytes.
4. **Prefer specific over broad.** `trading.orders.filled` is better than `trading.events` because subscribers can filter precisely without parsing the Pulse body.

### Extension topic declaration

Extensions declare their custom topics in their manifest:

```rust
/// A declared custom topic with documentation.
pub struct TopicDeclaration {
    /// The topic string (e.g., "defi.alerts").
    pub topic: String,
    /// Human-readable description of what this topic carries.
    pub description: String,
    /// Expected Pulse Kind on this topic.
    pub kind: Kind,
    /// Approximate publish rate.
    pub rate_hz: f64,
}
```

---

## 8. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| TN-1 | `validate_topic` accepts all canonical topics from section 3 | Unit test: iterate canonical topics, all pass validation |
| TN-2 | `validate_topic` rejects empty, oversized, too-many-segments, uppercase | Unit test: each invalid case returns the correct `TopicError` variant |
| TN-3 | Topics are created implicitly on first publish or subscribe | Integration test: publish to new topic, verify it exists in Bus state |
| TN-4 | Topics with zero subscribers and empty buffer are GC'd after TTL | Integration test: publish, unsubscribe, wait TTL, verify topic removed |
| TN-5 | Permanent topics (`system.*`, `chain.*`, `isfr.*`) are never GC'd | Integration test: unsubscribe from `system.relay`, verify it persists |
| TN-6 | `reserved_prefix` identifies all reserved namespaces | Unit test: check each reserved prefix, verify non-reserved topics return `None` |
| TN-7 | Agent publish to reserved topic returns `TopicError::Reserved` | Integration test: agent publishes to `chain.1`, verify error |
| TN-8 | System publisher can publish to reserved topics | Integration test: system handle publishes to `system.relay`, verify success |
| TN-9 | Custom topics with valid grammar are accepted | Unit test: multi-segment custom topics pass validation |
| TN-10 | Wildcard `*` matches exactly one segment (when implemented) | Unit test: `chain.*` matches `chain.31337`, not `chain.31337.block` |
| TN-11 | Wildcard `>` matches one or more trailing segments (when implemented) | Unit test: `chain.>` matches `chain.31337` and `chain.31337.block` |
