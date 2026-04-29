# Chain Witness and Triage

> Depth for [22-REGISTRIES.md](../../unified/22-REGISTRIES.md). How blockchain event monitoring emerges as a Feed specialization with Trigger-based ingestion and Pipeline-based triage.

This doc specifies the ChainWitness subsystem -- the runtime that connects to EVM nodes, ingests block data, filters irrelevant events at wire speed, and routes the survivors through a four-stage triage Pipeline before they reach any downstream consumer. The entire design decomposes into Feed, Score, Observe, and Trigger Cells. No new kernel primitives are introduced.

---

## 1. ChainWitness as a Feed

A Feed is `Cell + Connect + Trigger + Store` ([09-FEEDS](../../unified/09-FEEDS.md) S1.1). ChainWitness is the most data-intensive Feed in the system -- it connects to one or more EVM WebSocket endpoints, triggers on each new block header, and stores triaged events as graduated Signals. Every other blockchain Feed (gas trends, swap events, funding rates) is a **Derived Feed** built on top of ChainWitness output.

### 1.1 Kernel Decomposition

```
ChainWitnessFeed = Cell + Connect + Trigger + Store

  Cell provides:    id, name, version, input_schema, output_schema, protocols, execute
  Connect provides: connect (WebSocket to EVM node), disconnect, health_check, query (eth_call)
  Trigger provides: arm (eth_subscribe "newHeads"), fire (on each block header)
  Store provides:   put (graduated chain events), query (by block range, address, topic)
```

The Feed publishes raw chain Pulses on Bus topic `feed:chain:{chain_id}:blocks`. Downstream consumers (triage Pipeline, derived feeds, chain event Triggers) subscribe to this topic. The Feed itself is domain-agnostic at the protocol level -- the chain-specific logic lives in the Connect and Trigger implementations.

### 1.2 ChainWitnessFeed Cell

```rust
/// The ChainWitness Feed Cell. Connects to an EVM node via WebSocket,
/// triggers on new block headers, and publishes chain events as Pulses
/// on Bus topic `feed:chain:{chain_id}:blocks`.
///
/// Kernel decomposition: Cell + Connect + Trigger + Store.
pub struct ChainWitnessFeed {
    /// Chain identifier (e.g., 1 for Ethereum mainnet, 8453 for Base).
    chain_id: u64,

    /// Connection pool for JSON-RPC queries (eth_call, eth_getLogs).
    /// Deadpool-managed: query_pool_size connections (default: 4).
    query_pool: Pool<RpcConnection>,

    /// Binary Fuse filter for logsBloom pre-screening (T0 probe).
    /// Updated on each watched-address change. See S2.
    bloom_probe: Arc<ArcSwap<BinaryFuse16>>,

    /// Set of addresses and topics currently being watched.
    /// Mutations rebuild the bloom_probe via ArcSwap::store().
    watch_set: Arc<RwLock<WatchSet>>,

    /// Roaring Bitmap tracking seen block numbers for gap detection.
    seen_blocks: Arc<RwLock<RoaringBitmap>>,

    /// Latest known block number and gas price (updated per header).
    chain_head: Arc<ArcSwap<ChainHead>>,

    /// Configuration.
    config: ChainWitnessConfig,
}

pub struct ChainWitnessConfig {
    /// Number of connections in the query pool.
    pub query_pool_size: usize,        // default: 4

    /// Maximum blocks to backfill on gap detection.
    pub gap_backfill_limit: u64,       // default: 1_000

    /// Maximum addresses + topics in the watch set.
    pub max_watch_size: usize,         // default: 10_000

    /// WebSocket endpoint URL.
    pub ws_endpoint: String,

    /// Reconnection backoff parameters.
    pub reconnect_base_ms: u64,        // default: 500
    pub reconnect_max_ms: u64,         // default: 30_000

    /// Bus topic for output Pulses.
    pub output_topic: String,          // default: "feed:chain:{chain_id}:blocks"
}
```

### 1.3 Connect Protocol Implementation

The Connect protocol manages the WebSocket lifecycle to the EVM node. Connection health is monitored via heartbeat pings; disconnection triggers automatic reconnection with exponential backoff.

```rust
#[async_trait]
impl ConnectProtocol for ChainWitnessFeed {
    async fn connect(&self, ctx: &CellContext) -> Result<ConnectionHandle> {
        let ws = connect_ws(&self.config.ws_endpoint).await?;

        // Subscribe to new block headers (the Trigger arm).
        let sub_id = ws.send(json!({
            "jsonrpc": "2.0",
            "method": "eth_subscribe",
            "params": ["newHeads"],
        })).await?;

        // Update chain head with current latest block.
        let head = ws.send(json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
        })).await?;
        self.chain_head.store(Arc::new(ChainHead::from(head)));

        Ok(ConnectionHandle {
            ws,
            subscription_id: sub_id,
            connected_at: Utc::now(),
        })
    }

    async fn disconnect(&self, handle: ConnectionHandle) -> Result<()> {
        handle.ws.send(json!({
            "jsonrpc": "2.0",
            "method": "eth_unsubscribe",
            "params": [handle.subscription_id],
        })).await?;
        handle.ws.close().await
    }

    async fn health_check(&self, handle: &ConnectionHandle) -> HealthStatus {
        match handle.ws.ping().await {
            Ok(latency) if latency < Duration::from_secs(5) => HealthStatus::Healthy,
            Ok(latency) => HealthStatus::Degraded {
                reason: format!("high latency: {}ms", latency.as_millis()),
            },
            Err(e) => HealthStatus::Unhealthy { error: e.to_string() },
        }
    }
}
```

### 1.4 Trigger Protocol Implementation

The Trigger protocol arms a subscription to `newHeads`. Each block header arriving over the WebSocket fires the Trigger, which publishes a Pulse on Bus. This is the push-based design from [13-TRIGGERS](../../unified/13-TRIGGERS.md) -- no polling.

```rust
#[async_trait]
impl TriggerProtocol for ChainWitnessFeed {
    async fn arm(&self, binding: &TriggerBinding, bus: Arc<dyn Bus>) -> Result<TriggerHandle> {
        // The WebSocket subscription is established in connect().
        // arm() registers the callback that fires on each header.
        Ok(TriggerHandle {
            id: TriggerId::new(format!("chain:{}:newHeads", self.chain_id)),
            binding: binding.clone(),
            armed_at: Utc::now(),
            state: TriggerState::Armed,
        })
    }

    async fn disarm(&self, handle: TriggerHandle) -> Result<()> {
        // Unsubscribe handled by disconnect().
        Ok(())
    }
}
```

### 1.5 Block Ingestion Pipeline

When a new block header arrives, the Feed executes a fast-path pipeline before publishing anything to Bus. This pipeline is the core of ChainWitness's efficiency: >90% of blocks are discarded at the Binary Fuse probe (S2) without fetching receipts.

```
newHeads event
    |
    +-> Update chain_head (latest block number, gas price)
    |
    +-> Gap detection (seen_blocks Roaring Bitmap)
    |       |
    |       +-> gap <= 1,000 blocks: spawn backfill task (eth_getLogs)
    |       +-> gap > 1,000 blocks: emit ChainGapDetected Pulse, resume from head
    |
    +-> Binary Fuse probe: check logsBloom against watch_set
    |       |
    |       +-> MISS (>90% of blocks): skip entirely. No RPC calls.
    |       +-> HIT: fetch full block + receipts via query_pool
    |
    +-> Normalize to ChainEventPulse
    |
    +-> Publish on Bus topic feed:chain:{chain_id}:blocks
```

```rust
/// The main ingestion loop. Called for each newHeads notification.
async fn on_block_header(
    &self,
    header: BlockHeader,
    bus: &dyn Bus,
    ctx: &CellContext,
) -> Result<()> {
    // 1. Update chain head (atomic swap).
    self.chain_head.store(Arc::new(ChainHead {
        number: header.number,
        gas_price: header.base_fee_per_gas,
        timestamp: header.timestamp,
    }));

    // 2. Gap detection via Roaring Bitmap.
    let mut seen = self.seen_blocks.write().await;
    let expected = seen.maximum().map(|m| m + 1).unwrap_or(header.number as u32);
    let actual = header.number as u32;

    if actual > expected {
        let gap = (actual - expected) as u64;
        if gap <= self.config.gap_backfill_limit {
            // Backfill: fetch missed blocks via eth_getLogs.
            tokio::spawn(self.backfill(expected as u64, actual as u64, bus.clone()));
        } else {
            // Gap too large: emit warning, resume from head.
            bus.publish(Pulse::new(
                format!("feed:chain:{}:gap", self.chain_id),
                json!({
                    "expected": expected,
                    "actual": actual,
                    "gap_blocks": gap,
                    "action": "resume_from_head",
                }),
            )).await?;
        }
    }
    seen.insert(actual);
    drop(seen);

    // 3. Binary Fuse probe: check logsBloom (T0 probe, see S2).
    let bloom = &header.logs_bloom;
    let filter = self.bloom_probe.load();
    if !bloom_matches_watchset(bloom, &filter) {
        // MISS: nothing in this block matches any watched address/topic.
        // Skip entirely. No RPC calls. This is >90% of blocks.
        return Ok(());
    }

    // 4. HIT: fetch block + receipts.
    let block = self.query_pool.get().await?
        .eth_get_block_with_receipts(header.number).await?;

    // 5. Normalize to ChainEventPulse and publish.
    for event in normalize_block_events(&block, &self.watch_set.read().await) {
        bus.publish(Pulse::new(
            &self.config.output_topic,
            serde_json::to_value(&event)?,
        )).await?;
    }

    Ok(())
}
```

### 1.6 Reconnection and Gap Handling

The Feed's Connect protocol handles two failure modes:

| Failure | Detection | Recovery |
|---|---|---|
| **WebSocket disconnect** | `health_check` returns `Unhealthy` or read error | Exponential backoff reconnect: 500ms, 1s, 2s, ... 30s max |
| **Block gap** | `seen_blocks` Roaring Bitmap shows `actual > expected + 1` | Gap <= 1,000: backfill via `eth_getLogs`. Gap > 1,000: emit `ChainGapDetected`, resume from head |

Backfill uses the query pool (not the WebSocket) to avoid blocking the live subscription. The backfill task fetches logs in 100-block chunks and publishes them as Pulses on Bus in order. Downstream consumers see a contiguous stream.

```rust
async fn backfill(
    &self,
    from_block: u64,
    to_block: u64,
    bus: Arc<dyn Bus>,
) -> Result<()> {
    // Chunk into 100-block ranges to avoid RPC limits.
    for chunk_start in (from_block..to_block).step_by(100) {
        let chunk_end = (chunk_start + 100).min(to_block);
        let conn = self.query_pool.get().await?;
        let logs = conn.eth_get_logs(FilterBuilder::new()
            .from_block(chunk_start)
            .to_block(chunk_end)
            .topics(self.watch_set.read().await.topic_filters())
            .build()
        ).await?;

        for event in normalize_log_events(&logs) {
            bus.publish(Pulse::new(
                &self.config.output_topic,
                serde_json::to_value(&event)?,
            )).await?;
        }
    }
    Ok(())
}
```

---

## 2. The Binary Fuse Probe as a T0 Probe

The system uses 16 T0 cognitive probes (see [05-EXECUTION](../../unified/04-EXECUTION.md)) -- cheap pre-filters that discard obviously irrelevant work before expensive processing begins. The Binary Fuse filter in ChainWitness is exactly this pattern, applied to blockchain data.

### 2.1 The Pattern

```
T0 probe: O(1) check, zero false negatives, tunable false positive rate.
           Pass → expensive processing.
           Fail → discard immediately.
```

Every EVM block header contains a `logsBloom` -- a 2048-bit Bloom filter encoding all addresses and log topics that emitted events in that block. The ChainWitness T0 probe checks whether the block's `logsBloom` could contain any address or topic in the `watch_set`. If not, the block is skipped entirely -- no RPC call to fetch receipts, no parsing, no triage. This single probe eliminates >90% of blocks at effectively zero cost.

### 2.2 Binary Fuse Filter (BinaryFuse16)

The watch set is compiled into a Binary Fuse filter (Lemire et al., 2019), not a standard Bloom filter. Binary Fuse filters have three advantages for this use case:

| Property | Bloom Filter | Binary Fuse16 |
|---|---|---|
| **Space** | ~10 bits/element | ~9.08 bits/element |
| **Lookup** | k hash probes (k=7 typical) | 3 XOR probes (always) |
| **False positive rate** | Tunable, typically 1% | Fixed ~1.5 / 2^16 = 0.0023% |
| **Construction** | Incremental | Batch (rebuild on change) |
| **Cache behavior** | Poor (random access) | Good (3 accesses, predictable) |

The tradeoff is that Binary Fuse filters cannot be incrementally updated -- adding or removing a watched address requires a full rebuild. This is acceptable because:
1. The watch set changes infrequently (new Trigger bindings, not per-block).
2. Rebuild is fast: 10,000 elements in <1ms.
3. The `ArcSwap` enables lock-free replacement: readers see the old filter until `store()` completes.

```rust
/// The T0 probe: Binary Fuse filter over the watch set.
/// Returns true if the block's logsBloom MAY contain events
/// matching any watched address or topic. False positives are
/// possible (~0.002%); false negatives are impossible.
fn bloom_matches_watchset(
    logs_bloom: &[u8; 256],
    filter: &BinaryFuse16,
) -> bool {
    // Extract all 20-byte addresses and 32-byte topics encoded
    // in the logsBloom. EVM Bloom encoding uses 3 pairs of bytes
    // per element, selecting bit positions via keccak256.
    //
    // For each element in the Bloom, check membership in the
    // Binary Fuse filter. If ANY element matches, the block
    // may contain relevant events.
    for element_hash in extract_bloom_elements(logs_bloom) {
        if filter.contains(&element_hash) {
            return true;
        }
    }
    false
}

/// Rebuild the Binary Fuse filter when the watch set changes.
/// Called from watch_set mutation methods. Uses ArcSwap for
/// lock-free replacement -- readers see the old filter until
/// store() completes; no lock contention on the hot path.
fn rebuild_bloom_probe(
    watch_set: &WatchSet,
    probe: &ArcSwap<BinaryFuse16>,
) {
    let hashes: Vec<u64> = watch_set.addresses.iter()
        .map(|addr| fxhash::hash64(addr.as_bytes()))
        .chain(watch_set.topics.iter()
            .map(|topic| fxhash::hash64(topic.as_bytes())))
        .collect();

    let new_filter = BinaryFuse16::try_from(&hashes)
        .expect("Binary Fuse construction should not fail for non-empty sets");

    probe.store(Arc::new(new_filter));
}
```

### 2.3 The T0 Probe Family

The Binary Fuse probe is one instance of a general pattern used across the system:

| Subsystem | T0 Probe | What It Pre-Filters | Reject Rate |
|---|---|---|---|
| **ChainWitness** | Binary Fuse on logsBloom | Blocks without relevant events | >90% |
| **Knowledge retrieval** | HDC cosine threshold | Signals below similarity cutoff | ~80% |
| **Verify Pipeline** | DiffGate (vacuous check) | Empty or todo-only diffs | ~15% |
| **Trigger filter** | Kind check (O(1)) | Pulses of wrong Kind | Varies |
| **Compose budget** | Token count estimate | Sections exceeding remaining budget | ~30% |

All share the property: O(1) or near-O(1), zero false negatives, discards the obvious misses before any expensive work.

---

## 3. Triage Pipeline as a Pipeline of Score Cells

Once a block passes the T0 probe and its events are normalized, they enter the triage Pipeline. Triage is a **Pipeline Graph** ([03-GRAPH](../../unified/03-GRAPH.md)) -- a linear chain of Score Cells where each stage scores and filters events. Events that score below a stage's threshold are discarded. No LLM is involved in any stage -- the entire pipeline is rule-based, streaming, and constant-memory.

```
[RuleClassifier] --> [AnomalyDetector] --> [ContextEnricher] --> [CuriosityScorer]
   Score Cell           Observe Cell          Compose Cell          Score Cell
   (O(1) HashMap)       (MIDAS-R lens)        (enrichment)          (4-axis score)
        |                    |                     |                     |
        v                    v                     v                     v
    "kind: DeFi"         "anomaly: 0.72"     "+ context fields"     "curiosity: 0.65"
```

### 3.1 Stage 1: Rule-Based Classification (Score Cell)

The first stage classifies each event by type using O(1) HashMap lookup on the event's method signature or log topic. This is a Score Cell: it takes a Signal (chain event) and produces a Score along the "kind" dimension.

```rust
/// Stage 1: Rule-based event classification.
/// O(1) HashMap lookup on method signature / log topic.
/// Produces a Score with a `kind` field.
pub struct RuleClassifierCell {
    /// Maps 4-byte method selectors to event kinds.
    selector_map: HashMap<[u8; 4], ChainEventKind>,

    /// Maps 32-byte log topics to event kinds.
    topic_map: HashMap<[u8; 32], ChainEventKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChainEventKind {
    DeFiSwap,
    DeFiLiquidity,
    DeFiLend,
    DeFiFlashLoan,
    TokenTransfer,
    TokenApproval,
    NftMint,
    NftTransfer,
    ContractDeployment,
    GovernanceVote,
    GovernanceProposal,
    BridgeDeposit,
    BridgeWithdrawal,
    OracleUpdate,
    Unknown,
}

impl RuleClassifierCell {
    /// Classify an event. O(1): HashMap lookup, no iteration.
    fn classify(&self, event: &ChainEventPulse) -> ChainEventKind {
        // Try method selector first (for transaction inputs).
        if let Some(selector) = event.method_selector() {
            if let Some(kind) = self.selector_map.get(&selector) {
                return *kind;
            }
        }

        // Try log topic (for emitted events).
        if let Some(topic0) = event.primary_topic() {
            if let Some(kind) = self.topic_map.get(&topic0) {
                return *kind;
            }
        }

        ChainEventKind::Unknown
    }
}

impl Cell for RuleClassifierCell {
    fn name(&self) -> &str { "chain-triage-classifier" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
}

#[async_trait]
impl ScoreProtocol for RuleClassifierCell {
    async fn score(&self, signal: &Signal, _ctx: &CellContext) -> Result<Score> {
        let event: ChainEventPulse = serde_json::from_value(signal.payload.clone())?;
        let kind = self.classify(&event);

        // Annotate the Signal with the classification.
        Ok(Score {
            relevance: if kind == ChainEventKind::Unknown { 0.1 } else { 0.5 },
            novelty: 0.5,  // unknown at this stage; set by CuriosityScorer
            confidence: 1.0,  // rule-based: deterministic
            dimensions: vec![
                ("kind".into(), kind_to_score(kind)),
            ],
        })
    }
}
```

The `selector_map` and `topic_map` are populated at startup from a registry of known contract ABIs. Adding support for a new protocol means adding entries to these maps -- no code changes.

### 3.2 Stage 2: MIDAS-R Anomaly Detection (Observe Cell / Lens)

The second stage is a **Lens** (Observe Cell, see [02-CELL](../../unified/02-CELL.md) S2.7) that maintains streaming statistics and detects anomalies without storing raw events. MIDAS-R (Bhatia et al., 2020) uses streaming chi-squared tests over temporal graph edges to detect bursts of activity between address pairs.

A Lens is the right abstraction: it reads the event stream, maintains internal state (the temporal edge counts), and produces observation Signals (anomaly scores) without mutating the input stream.

```rust
/// Stage 2: MIDAS-R streaming anomaly detector.
/// An Observe Cell (Lens) maintaining constant-memory statistics.
///
/// MIDAS-R models the event stream as a temporal graph:
///   - Nodes are addresses (source, destination).
///   - Edges are events (transfer, swap, etc.).
///   - Temporal dimension is block number.
///
/// Anomaly = chi-squared test on edge count vs. expected count
/// given the node's historical rate.
pub struct MidasAnomalyLens {
    /// CMS (Count-Min Sketch) for edge counts in the current window.
    current_counts: CountMinSketch,

    /// CMS for edge counts in the total history.
    total_counts: CountMinSketch,

    /// Current temporal tick (block number modulo window).
    current_tick: u64,

    /// Window size in blocks.
    window_size: u64,         // default: 100

    /// CMS parameters.
    cms_width: usize,         // default: 1024
    cms_depth: usize,         // default: 4

    /// Anomaly score threshold for flagging.
    anomaly_threshold: f64,   // default: 3.0 (chi-squared critical value)
}

impl MidasAnomalyLens {
    /// Score an event for anomalousness.
    /// Returns a value in [0.0, 1.0] where higher = more anomalous.
    /// Constant-memory: only the two CMS structures are maintained.
    fn anomaly_score(&mut self, event: &ChainEventPulse) -> f64 {
        let edge = (event.from_address(), event.to_address());
        let edge_hash = hash_edge(&edge);
        let tick = event.block_number % self.window_size;

        // Advance window if needed (reset current counts).
        if tick != self.current_tick {
            self.current_tick = tick;
            self.current_counts.clear();
        }

        // Increment counts.
        self.current_counts.increment(edge_hash);
        self.total_counts.increment(edge_hash);

        let current = self.current_counts.query(edge_hash) as f64;
        let total = self.total_counts.query(edge_hash) as f64;
        let expected = total / self.window_size as f64;

        // Chi-squared statistic: (observed - expected)^2 / expected.
        if expected < 1.0 {
            return 0.0; // Too few observations to judge.
        }
        let chi2 = (current - expected).powi(2) / expected;

        // Normalize to [0, 1] using a sigmoid: 1 / (1 + exp(-chi2 + threshold)).
        let normalized = 1.0 / (1.0 + (-chi2 + self.anomaly_threshold).exp());
        normalized.clamp(0.0, 1.0)
    }
}

impl Cell for MidasAnomalyLens {
    fn name(&self) -> &str { "chain-triage-anomaly" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
}

#[async_trait]
impl ObserveProtocol for MidasAnomalyLens {
    async fn observe(&mut self, signal: &Signal, _ctx: &CellContext) -> Result<Signal> {
        let event: ChainEventPulse = serde_json::from_value(signal.payload.clone())?;
        let anomaly = self.anomaly_score(&event);

        // Emit an observation Signal annotating the event with its anomaly score.
        Ok(Signal::new(Kind::Observation, json!({
            "event_hash": signal.content_hash,
            "anomaly_score": anomaly,
            "midas_tick": self.current_tick,
        })))
    }
}
```

**Why constant memory**: MIDAS-R uses Count-Min Sketch (width=1024, depth=4) -- fixed at ~16KB regardless of event volume. The system can process millions of events per hour without memory growth. This is critical for a background Feed that runs continuously.

### 3.3 Stage 3: Contextual Enrichment (Compose Cell)

The third stage enriches classified, anomaly-scored events with additional context from local stores. This is a Compose Cell: it assembles information from multiple sources into a richer Signal. Context includes:

- **Known contract metadata** (name, protocol, verified status) from a local registry.
- **Historical interaction data** (has this agent interacted with this address before?) from Store.
- **Price context** (token price at the time of the event) from price Feeds.

```rust
/// Stage 3: Contextual enrichment.
/// A Compose Cell that assembles context from local stores.
pub struct ContextEnricherCell {
    /// Local contract registry (address -> metadata).
    contract_registry: Arc<ContractRegistry>,

    /// Store handle for querying historical interactions.
    store: Arc<dyn StoreProtocol>,
}

impl Cell for ContextEnricherCell {
    fn name(&self) -> &str { "chain-triage-enricher" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }
}

#[async_trait]
impl ComposeProtocol for ContextEnricherCell {
    async fn compose(
        &self,
        inputs: Vec<Signal>,
        budget: &ComposeBudget,
        _ctx: &CellContext,
    ) -> Result<ComposeResult> {
        let event_signal = &inputs[0];
        let event: ChainEventPulse = serde_json::from_value(event_signal.payload.clone())?;

        // Look up contract metadata.
        let contract_meta = self.contract_registry.get(&event.to_address()).await;

        // Check for prior interactions in Store.
        let prior_interactions = self.store.query(StoreQuery {
            kind: Some(Kind::ChainEvent),
            tags: Some(vec![format!("addr:{}", event.to_address())]),
            limit: 5,
            ..Default::default()
        }).await?;

        // Produce enriched payload.
        let enriched = json!({
            "event": event,
            "contract": contract_meta,
            "prior_interaction_count": prior_interactions.len(),
            "prior_interactions_recent": prior_interactions.iter()
                .take(3)
                .map(|s| s.id.to_string())
                .collect::<Vec<_>>(),
        });

        Ok(ComposeResult {
            composed: Signal::new(Kind::ChainEvent, enriched),
            accepted: inputs.iter().map(|s| s.id).collect(),
            budget_used: 0, // enrichment has no token cost
        })
    }
}
```

### 3.4 Stage 4: Curiosity Scoring (Score Cell)

The final stage computes a composite **curiosity score** across four dimensions. This is a Score Cell that produces the definitive signal quality assessment. The curiosity score determines what happens next: ignore, log silently, alert, or escalate.

```rust
/// Stage 4: Curiosity scoring.
/// A Score Cell producing a 4-axis curiosity score.
///
/// Curiosity = relevance * 0.30
///           + anomaly  * 0.25
///           + novelty  * 0.25
///           + surprise * 0.20
pub struct CuriosityScorerCell {
    /// Weights for the four curiosity axes.
    weights: CuriosityWeights,

    /// Novelty tracker: exponential decay of seen event types.
    novelty_tracker: NoveltyTracker,

    /// Surprise tracker: deviation from predicted event distribution.
    surprise_tracker: SurpriseTracker,
}

pub struct CuriosityWeights {
    pub relevance: f64,   // default: 0.30
    pub anomaly: f64,     // default: 0.25
    pub novelty: f64,     // default: 0.25
    pub surprise: f64,    // default: 0.20
}

impl CuriosityScorerCell {
    fn compute_curiosity(
        &mut self,
        event: &ChainEventPulse,
        classification_score: f64,
        anomaly_score: f64,
    ) -> CuriosityResult {
        // Relevance: from the classifier stage. How well does this event
        // match what the system is currently interested in?
        let relevance = classification_score;

        // Anomaly: from MIDAS-R. How statistically unusual is this event
        // relative to the temporal edge distribution?
        let anomaly = anomaly_score;

        // Novelty: has the system seen this type of event recently?
        // Exponential decay: recently-seen types score low, rare types score high.
        let novelty = self.novelty_tracker.score(event);

        // Surprise: how much does this event deviate from the predicted
        // distribution of events? Uses KL-divergence between predicted
        // and observed event-kind counts over a rolling window.
        let surprise = self.surprise_tracker.score(event);

        let composite = relevance * self.weights.relevance
            + anomaly * self.weights.anomaly
            + novelty * self.weights.novelty
            + surprise * self.weights.surprise;

        CuriosityResult {
            composite,
            relevance,
            anomaly,
            novelty,
            surprise,
            action: CuriosityAction::from_score(composite),
        }
    }
}

/// Curiosity thresholds determine the action taken for each event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CuriosityAction {
    /// 0.0 - 0.2: Below noise floor. Discard entirely.
    Ignore,

    /// 0.2 - 0.5: Potentially interesting. Log to Store but do not alert.
    /// Pulse does NOT graduate to Signal.
    Silent,

    /// 0.5 - 0.8: Interesting. Graduate Pulse to Signal. Emit alert Pulse
    /// on Bus topic `chain:alert:{chain_id}`. Queue for LLM analysis
    /// if an analysis agent is available.
    Alert,

    /// 0.8 - 1.0: Highly anomalous or novel. Immediate escalation.
    /// Graduate to Signal with high priority. Emit on `chain:escalate:{chain_id}`.
    /// Interrupts the current agent focus (Theta interrupt via Daimon).
    Escalate,
}

impl CuriosityAction {
    pub fn from_score(score: f64) -> Self {
        match score {
            s if s >= 0.8 => Self::Escalate,
            s if s >= 0.5 => Self::Alert,
            s if s >= 0.2 => Self::Silent,
            _ => Self::Ignore,
        }
    }
}

impl Cell for CuriosityScorerCell {
    fn name(&self) -> &str { "chain-triage-curiosity" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
}

#[async_trait]
impl ScoreProtocol for CuriosityScorerCell {
    async fn score(&self, signal: &Signal, ctx: &CellContext) -> Result<Score> {
        let event: ChainEventPulse = serde_json::from_value(
            signal.payload.get("event").cloned().unwrap_or_default()
        )?;
        let classification_score = signal.score.relevance;
        let anomaly_score = signal.payload.get("anomaly_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let result = self.compute_curiosity(
            &event, classification_score, anomaly_score,
        );

        Ok(Score {
            relevance: result.relevance,
            novelty: result.novelty,
            confidence: 1.0,
            dimensions: vec![
                ("curiosity".into(), result.composite),
                ("anomaly".into(), result.anomaly),
                ("surprise".into(), result.surprise),
                ("action".into(), result.action as u8 as f64),
            ],
        })
    }
}
```

### 3.5 Pipeline Assembly

The four stages compose into a Pipeline Graph. Short-circuiting is by score threshold: events scoring below 0.2 curiosity are dropped after the CuriosityScorerCell. Events scoring below the classifier's minimum are dropped even earlier.

```rust
/// Assemble the triage pipeline from individual Cells.
fn build_triage_pipeline(config: &TriageConfig) -> PipelineGraph {
    PipelineGraph::new("chain-triage")
        .stage(RuleClassifierCell::new(&config.classifier))
        .stage(MidasAnomalyLens::new(&config.anomaly))
        .stage(ContextEnricherCell::new(&config.enrichment))
        .stage(CuriosityScorerCell::new(&config.curiosity))
        .with_early_exit(|score| score.relevance < 0.05) // drop unclassifiable
}
```

### 3.6 Pipeline Cost Profile

| Stage | Cost | Memory | Latency | What It Saves |
|---|---|---|---|---|
| Binary Fuse probe (T0) | O(3) XOR lookups | 9 bits/element | <1us | >90% of blocks: no RPC calls |
| Rule classifier | O(1) HashMap lookup | ~100KB for 5,000 selectors | <1us | Nothing (classifies, does not filter) |
| MIDAS-R anomaly | O(4) CMS increments | ~16KB fixed | <1us | Nothing (scores, does not filter) |
| Context enrichment | 1 Store query | Negligible | ~1ms | Nothing (enriches, does not filter) |
| CuriosityScorerCell | O(1) arithmetic | ~1KB trackers | <1us | 60-80% of events (Ignore + Silent) |

Total cost per block: <1ms for misses (T0 probe only), ~2ms for hits. At 12-second block times (Ethereum mainnet), this is negligible.

---

## 4. Graduation and Downstream Routing

Events that survive the triage Pipeline follow different paths based on their curiosity action:

```
CuriosityAction::Ignore    -> Discard. Pulse dies in Bus ring buffer.
CuriosityAction::Silent    -> Log to Bus topic chain:silent:{chain_id}.
                              No graduation. Available for batch analysis later.
CuriosityAction::Alert     -> Graduate Pulse to Signal in Store.
                              Publish on chain:alert:{chain_id}.
                              Queue for optional LLM analysis.
CuriosityAction::Escalate  -> Graduate with high priority.
                              Publish on chain:escalate:{chain_id}.
                              Emit Theta interrupt to Daimon
                              (shifts agent attention immediately).
```

### 4.1 Graduation to Signal

Alert and Escalate events graduate from Pulse to Signal via the standard graduation path ([01-SIGNAL](../../unified/01-SIGNAL.md) S2):

```rust
async fn graduate_chain_event(
    event: &ChainEventPulse,
    curiosity: &CuriosityResult,
    store: &dyn StoreProtocol,
) -> Result<Signal> {
    let signal = Pulse::graduate(
        // Provenance: the chain event's block hash and transaction hash.
        Provenance::chain(event.chain_id, event.block_number, event.tx_hash),
        // Initial balance: proportional to curiosity score.
        // Higher curiosity = longer demurrage lifetime.
        curiosity.composite,
        // Score: the 4-axis curiosity score.
        Score {
            relevance: curiosity.relevance,
            novelty: curiosity.novelty,
            confidence: 1.0,
            dimensions: vec![
                ("anomaly".into(), curiosity.anomaly),
                ("surprise".into(), curiosity.surprise),
            ],
        },
        // Tags for Store query.
        vec![
            format!("chain:{}", event.chain_id),
            format!("kind:{:?}", event.kind),
            format!("addr:{}", event.to_address()),
        ],
    );

    store.put(&signal).await?;
    Ok(signal)
}
```

### 4.2 Theta Interrupt Path

Escalate-level events trigger a Theta interrupt via the Daimon affect engine. This shifts the agent's attention immediately, interrupting lower-priority work:

```rust
/// Emit a Theta interrupt for Escalate-level chain events.
/// The Daimon subscribes to this topic and modulates attention.
async fn emit_theta_interrupt(
    event: &ChainEventPulse,
    curiosity: &CuriosityResult,
    bus: &dyn Bus,
) -> Result<()> {
    bus.publish(Pulse::new(
        "daimon.theta.interrupt",
        json!({
            "source": "chain-witness",
            "chain_id": event.chain_id,
            "curiosity": curiosity.composite,
            "event_kind": format!("{:?}", event.kind),
            "urgency": "immediate",
        }),
    )).await
}
```

---

## 5. Watch Set Management

The watch set determines what the ChainWitness cares about. It is not static -- it evolves as Trigger bindings are created, agents subscribe to chain event topics, and the system learns which addresses are relevant.

```rust
pub struct WatchSet {
    /// Watched addresses (contract addresses, wallets).
    pub addresses: HashSet<Address>,

    /// Watched log topics (event signatures).
    pub topics: HashSet<[u8; 32]>,

    /// Source tracking: why is each entry watched?
    pub sources: HashMap<WatchKey, WatchSource>,
}

#[derive(Debug, Clone)]
pub enum WatchSource {
    /// Explicitly configured in roko.toml.
    Config,

    /// From a ChainEvent Trigger binding.
    TriggerBinding { trigger_name: String },

    /// From an agent's Feed subscription.
    FeedSubscription { agent_id: AgentId, feed_id: String },

    /// Learned: address appeared in high-curiosity events.
    Learned { first_seen_block: u64, curiosity_mean: f64 },
}
```

When the watch set changes, the Binary Fuse filter is rebuilt via `ArcSwap::store()`. The rebuild is <1ms for 10,000 elements and does not block the ingestion loop.

---

## 6. Configuration

```toml
[chain.witness]
# Chain to watch.
chain_id = 1
ws_endpoint = "wss://eth-mainnet.ws.alchemyapi.io/v2/<key>"

# Connection pool.
query_pool_size = 4

# Gap handling.
gap_backfill_limit = 1000

# Watch set limits.
max_watch_size = 10000

# Triage thresholds.
[chain.witness.triage]
anomaly_threshold = 3.0        # Chi-squared critical value for MIDAS-R.
curiosity_weights = { relevance = 0.30, anomaly = 0.25, novelty = 0.25, surprise = 0.20 }

# MIDAS-R parameters.
[chain.witness.midas]
window_size = 100              # blocks
cms_width = 1024
cms_depth = 4
```

---

## What This Enables

1. **Domain-agnostic event ingestion**: The Feed+Pipeline decomposition means the same architecture works for any event source. Replace the WebSocket Connect Cell with a webhook Connect Cell and the same triage Pipeline processes CI events, file changes, or market data.

2. **Composable chain intelligence**: Because ChainWitness is a Feed, any Derived Feed can subscribe to its output topic. A gas oracle agent, a MEV detector, and a whale tracker can all consume the same triaged events without duplicating ingestion infrastructure.

3. **Curiosity-driven attention allocation**: The four-axis scoring replaces manual filtering rules with a principled measure of interestingness. The system allocates expensive LLM analysis time only to events that score above the Alert threshold -- a form of compute budgeting at the data layer.

4. **Constant-memory streaming**: MIDAS-R and the Bloom probe operate in constant memory regardless of event volume. The system can watch thousands of addresses across multiple chains without memory growth.

5. **Graceful degradation**: Gap detection and backfill ensure no events are lost on transient disconnects. Gaps beyond the backfill limit are acknowledged (not silently dropped), letting downstream consumers decide how to handle the discontinuity.

---

## Feedback Loops

- **Curiosity -> Watch Set**: Addresses that appear in high-curiosity events are candidates for automatic addition to the watch set (Learned source). This makes the system self-tuning: it watches what turns out to be interesting.

- **Curiosity -> Triage Weights**: If Alert-level events consistently fail to produce useful analysis (downstream agent discards them), the curiosity weights should adapt. The mechanism is the same predict-publish-correct Loop used elsewhere: the CuriosityScorer publishes its score, downstream agents publish whether the event was useful, and a CalibrationPolicy adjusts weights.

- **Verify Verdicts -> Classification**: When a chain event leads to a Verify pass or fail in downstream processing, the classification contributes to the agent's efficiency metrics. Events classified as DeFiSwap that consistently lead to useful actions validate the classifier; misclassifications that waste compute suggest the classifier needs updating.

- **Watch Set -> Bloom Probe**: Watch set mutations trigger Binary Fuse rebuild. This is not a feedback loop but a reactive dependency -- the probe always reflects the current watch set.

---

## Open Questions

1. **Multi-chain coordination**: The current design runs one `ChainWitnessFeed` per chain. With 3+ chains (Ethereum, Base, Arbitrum), should there be a coordinator Cell that merges cross-chain events before triage? Or should each chain run independent triage with a downstream Composite Feed?

2. **Curiosity weight adaptation**: The weights (0.30/0.25/0.25/0.20) are static defaults. Should they be treated as learnable parameters via the predict-publish-correct Loop? The risk is overfitting to recent event distributions.

3. **MIDAS-R window size**: The 100-block window (~20 minutes on Ethereum) is tuned for DeFi activity patterns. Different chains (Base at 2-second blocks) may need different windows. Should window size be auto-tuned based on observed event rates?

4. **Watch set pruning**: Addresses added via the Learned source could accumulate without bound. A demurrage-like mechanism (decay the learned entry's curiosity_mean unless refreshed by new high-curiosity events) would keep the watch set bounded, but the right decay rate is unknown.

5. **Bloom probe false positive rate**: The Binary Fuse filter's ~0.002% false positive rate is negligible at Ethereum's 12-second block time. At Base's 2-second block time with higher event density, false positives mean more wasted RPC calls. Should the filter be tunable per chain?
