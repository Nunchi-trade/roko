# 09 -- Feeds and Recipes

> Continuous data streams as Cell specializations (Connect+Trigger+Store). Raw/derived/composite/meta feeds. Feed registry with discovery API. Recipes as pure data Graphs of Score Cells. Dynamic registration. On-chain advertisement (ERC-8004).

**Kernel mapping**: Feed = Cell + Connect + Trigger + Store protocols. Recipe = Graph of Score Cells. Both compose from existing primitives with no new kernel types.

**Reconciliation with 11-CONNECTIVITY**: Feed = Cell + Connect + Trigger + Store (canonical kernel decomposition from [00-INDEX](00-INDEX.md)). The Feed's output Pulse streams ride on Bus topics (topic pattern: `feed:{id}:data`). [11-CONNECTIVITY](11-CONNECTIVITY.md) describes the transport layer (relay wire protocol, WebSocket lifecycle, backpressure, reconnection). This document defines what Feeds are and how they compose; doc 11 defines how their Pulses are delivered across process and network boundaries.

---

## 1. Feed Primitive

A **Feed** is a Cell specialization that connects to an external data source (Connect protocol), watches for new data (Trigger protocol), and optionally persists snapshots (Store protocol). A Feed is the "always-on" complement to one-shot Connector queries. The architecture is domain-agnostic -- blockchain feeds are the most data-intensive case, but the same pattern applies to CI status feeds, file change feeds, webhook streams, and any other continuous data source.

### 1.1 Kernel Decomposition

```
Feed = Cell + Connect + Trigger + Store

  Cell provides: id, name, version, input_schema, output_schema, capabilities, protocols, execute
  Connect provides: connect, query, execute, disconnect, health_check
  Trigger provides: listen, filter, debounce, fire
  Store provides: put, get, query, query_similar, prune
```

A Feed Cell's `execute()` method reads from its Connect source, applies Trigger filtering, publishes matching data as Pulses on Bus topic `feed:{id}:data`, and optionally graduates Pulses to Signals in Store. The Feed's subscription API is implemented as Bus topic management -- subscribers join the Feed's Bus topic.

### 1.2 Feed Shape

Every Feed conforms to the universal Feed shape:

```
subscribe / unsubscribe / poll / status / configure
```

| Operation | Protocol | What It Does |
|---|---|---|
| subscribe | Bus | Join the Feed's topic. Receive Pulses as they arrive. |
| unsubscribe | Bus | Leave the Feed's topic. Stop receiving Pulses. |
| poll | Connect + Store | Request the latest value on demand (no subscription). |
| status | Observe | Check Feed health: connected, rate, lag, error count. |
| configure | Cell | Update Feed parameters at runtime (filter, rate, schema). |

### 1.3 Core Types

```rust
/// A Feed is a named Cell specialization with Connect + Trigger + Store.
pub struct FeedRegistration {
    pub feed_id: String,
    pub agent_id: AgentId,
    pub kind: FeedKind,
    pub schema: FeedSchema,
    pub description: String,
    pub rate_hz: f64,
    pub access: FeedAccess,
    pub sample: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FeedKind {
    /// Direct data ingestion from an external source.
    Raw,
    /// Computed from one or more raw feeds.
    Derived,
    /// Computed from multiple derived feeds across domains.
    Composite,
    /// Feed about feeds (health monitoring, accuracy tracking).
    Meta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FeedSchema {
    /// Well-known schema (e.g., eth_block, evm_logs, gas_trend_v1).
    Named(String),
    /// Agent-defined schema with custom fields.
    Custom(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FeedAccess {
    /// No payment required. Anyone can subscribe.
    Public,
    /// Payment required. See 18-PAYMENTS for x402 and MPP protocols.
    Paid {
        base_price_usdc_per_hour: u64,
        accepted_protocols: Vec<PaymentProtocol>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PaymentProtocol {
    X402,
    Mpp,
}
```

---

## 2. Feed Types and Composability

Feeds are domain-agnostic. Any agent can produce any feed. The feed system is the same regardless of domain. Feeds compose into value chains where each layer adds computation and charges for it.

### 2.1 Raw Feeds

Direct data ingestion from external sources via the Connect protocol. The Feed Cell connects to the source and publishes data as Pulses on Bus topic `feed:{id}:data`.

| Domain | Examples |
|---|---|
| Blockchain | `eth-mainnet-blocks`, `base-swaps`, `arb-gas` (from RPC WebSocket) |
| Research | `arxiv-new-papers`, `github-trending` (from web polling) |
| Code | `repo-commit-stream`, `ci-build-results` (from webhooks) |
| Market | `binance-funding-rates`, `coingecko-prices` (from exchange APIs) |
| Any | Any external data source an agent consumes can be re-published as a raw feed |

#### Blockchain Raw Feed Example

```rust
// Inside a ChainReaderExt extension
impl ChainReaderExt {
    async fn on_init(&mut self, ctx: &mut AgentContext) -> Result<()> {
        // Subscribe to Ethereum mainnet new blocks
        let eth_blocks = ctx.chain_subscribe(
            "ethereum",
            "newHeads",
            json!({}),
        ).await?;

        // Subscribe to Uniswap V3 swap events on Base
        let base_swaps = ctx.chain_subscribe(
            "base",
            "logs",
            json!({
                "address": "0x...",  // Uniswap V3 factory
                "topics": ["0xc42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67"]
            }),
        ).await?;

        self.feeds.push(eth_blocks);
        self.feeds.push(base_swaps);
        Ok(())
    }
}
```

### 2.2 Derived Feeds

Computed from raw feeds by a Cognition-layer extension. The extension reads raw data, applies transforms, and publishes the result as Pulses on a new Bus topic `feed:{derived-id}:data`.

| Domain | Examples |
|---|---|
| Blockchain | `eth-gas-trend`, `funding-rate-divergence`, `mev-probability` |
| Research | `paper-relevance-scores`, `topic-cluster-updates` |
| Code | `code-quality-trend`, `dependency-risk-index` |
| Market | `volatility-regime`, `cross-venue-spread` |
| Any | Any computation an agent performs on its inputs can be a derived feed |

### 2.3 Composite Feeds

Derived from multiple derived feeds, often across domains. Cost stacks: the producer pays for input feeds and charges for the output feed.

| Feed | Inputs |
|---|---|
| `cross-chain-arb-signal` | gas trends + volume + funding rates |
| `research-portfolio-impact` | paper scores + code quality + market sentiment |

### 2.4 Meta Feeds

Feeds about feeds. Produced by meta-agents that monitor the feed system itself.

| Feed | What It Monitors |
|---|---|
| `feed-health` | All feeds: staleness, drift, anomalies |
| `feed-accuracy` | Prediction accuracy of derived feeds over time |

### 2.5 Composition Diagram

```
eth-mainnet-blocks (free, raw)
  +-> gas-oracle agent
       +-> eth-gas-trend ($0.05/hr, derived)
            +-> arb-bot agent
                 +-> cross-chain-gas-arb ($0.50/hr, composite)
                      +-> dashboard subscriber

arxiv-new-papers (free, raw)
  +-> research-scout agent
       +-> defi-paper-relevance ($0.02/hr, derived)
            +-> strategy-agent subscribes for research context
```

Each agent in the chain pays for its inputs and charges for its output. Economics for a mid-chain agent: revenue per subscriber minus input cost. With N subscribers: (price * N) - input_cost = margin.

### 2.6 Practical Composition Example: Funding Rate Divergence

A concrete example showing feed composition. This agent consumes two paid feeds and produces a third.

**Agent manifest**

```toml
[agent]
name = "funding-arb"
profile = "chain"
mode = "persistent"

# This agent CONSUMES two feeds...
[[agent.feed_subscriptions]]
feed_id = "binance-funding-rates"
agent_id = "cex-connector"
budget_usdc = 1000  # $0.001 USDC session deposit

[[agent.feed_subscriptions]]
feed_id = "hyperliquid-funding-rates"
agent_id = "cex-connector"
budget_usdc = 1000

# ...and PRODUCES one feed
[agent.feeds.funding-divergence]
kind = "derived"
description = "Cross-venue funding rate divergence with z-score normalization"
schema = "funding_divergence_v1"
rate_hz = 0.1  # Every 10 seconds
access = "paid"
base_price_usdc_per_hour = 200000  # $0.20/hr
```

**The extension**

```rust
pub struct FundingDivergenceExt {
    binance_sub: FeedSubscription,
    hyperliquid_sub: FeedSubscription,
    history: VecDeque<f64>,
}

#[async_trait]
impl Extension for FundingDivergenceExt {
    fn name(&self) -> &str { "funding-divergence" }
    fn layer(&self) -> ExtensionLayer { ExtensionLayer::Cognition }

    async fn on_observe(&mut self, ctx: &mut AgentContext) -> Result<()> {
        let binance = self.binance_sub.latest_or_default();
        let hyper = self.hyperliquid_sub.latest_or_default();

        let divergence = binance["rate"].as_f64().unwrap_or(0.0)
            - hyper["rate"].as_f64().unwrap_or(0.0);

        self.history.push_back(divergence);
        if self.history.len() > 1000 { self.history.pop_front(); }

        let mean = self.history.iter().sum::<f64>() / self.history.len() as f64;
        let variance = self.history.iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f64>() / self.history.len() as f64;
        let zscore = if variance > 0.0 {
            (divergence - mean) / variance.sqrt()
        } else {
            0.0
        };

        // Publish feed data directly to Bus topic.
        // FeedPublisherExt bridges this to the relay for remote subscribers.
        ctx.bus.publish(
            &format!("feed:funding-divergence:data"),
            Pulse::new(json!({
                "divergence_bps": divergence * 10000.0,
                "zscore": zscore,
                "binance_rate": binance["rate"],
                "hyperliquid_rate": hyper["rate"],
                "signal": if zscore.abs() > 2.0 { "strong" }
                          else if zscore.abs() > 1.0 { "moderate" }
                          else { "none" },
                "direction": if divergence > 0.0 { "long_hyper" }
                             else { "long_binance" },
                "ts": now_ms(),
            })),
        );

        // Extreme divergence triggers T2 reasoning via prediction error
        if zscore.abs() > 3.0 {
            ctx.cortical.set_prediction_error(0.9);
        }

        Ok(())
    }
}
```

**The value chain**

```
cex-connector produces binance-funding-rates ($0.05/hr)
cex-connector produces hyperliquid-funding-rates ($0.05/hr)
  +-> funding-arb consumes both, pays $0.10/hr
       +-> funding-arb produces funding-divergence ($0.20/hr)
            +-> trading-bot subscribes, pays $0.20/hr
            +-> dashboard subscribes, pays $0.20/hr
```

Economics for `funding-arb`: $0.20/hr revenue per subscriber minus $0.10/hr input cost. With 5 subscribers that's ($0.20 * 5) - $0.10 = $0.90/hr pure margin.

---

## 3. Feed Registry

The relay maintains a feed registry. Dashboards and agents discover feeds dynamically. The registry is a Store Cell specialization -- it persists feed metadata as Signals and publishes registration/deregistration events as Pulses on Bus.

### 3.1 Registration

Agents register feeds with the relay on boot. When `[agent.feeds.*]` entries exist in the manifest, `FeedPublisherExt` is auto-loaded and handles the full lifecycle.

```json
POST /relay/feeds/register
{
  "feed_id": "eth-gas-trend",
  "agent_id": "gas-oracle",
  "kind": "derived",
  "schema": "gas_trend_v1",
  "description": "12-block EMA gas price with percentile bands and MEV detection",
  "rate_hz": 0.5,
  "access": {
    "paid": {
      "base_price_usdc_per_hour": 50,
      "accepted_protocols": ["x402", "mpp"]
    }
  },
  "sample": {"ema_12": 42.5, "p25": 35.0}
}
```

### 3.2 Discovery API

```
GET /relay/feeds                              # all feeds
GET /relay/feeds?kind=derived&access=paid     # filter by kind and access
GET /relay/feeds?agent_id=gas-oracle          # feeds from a specific agent
GET /relay/feeds/{feed_id}                    # single feed metadata
GET /relay/feeds/{feed_id}/sample             # sample payload (free, no auth)
```

**Response format**:

```json
[
  {
    "feed_id": "eth-mainnet-blocks",
    "agent_id": "chain-watcher-1",
    "kind": "raw",
    "schema": "eth_block",
    "rate_hz": 0.08,
    "access": "public",
    "subscribers": 3
  },
  {
    "feed_id": "eth-gas-trend",
    "agent_id": "chain-watcher-1",
    "kind": "derived",
    "schema": "gas_trend_v1",
    "rate_hz": 0.5,
    "access": { "paid": { "price_per_hour": 100 } },
    "subscribers": 1
  }
]
```

### 3.3 Pagination

`GET /relay/feeds` supports cursor-based pagination and filtering.

**Query parameters**:

```
Parameter    Type      Default   Description
---------    ----      -------   -----------
limit        u32       50        Results per page. Max: 200.
cursor       string    (none)    Opaque cursor from previous response's next_cursor.
kind         string    (none)    Filter by feed kind: "raw", "derived", "composite", "meta".
access       string    (none)    Filter by access type: "public" or "paid".
agent_id     string    (none)    Filter to feeds from a specific agent.
schema       string    (none)    Filter by schema name (exact match).
search       string    (none)    Full-text search across feed_id and description.
```

**Example**: Get the second page of paid derived feeds:

```
GET /relay/feeds?kind=derived&access=paid&limit=20&cursor=eyJsYXN0IjoiZXRoLWdhcy10cmVuZCJ9

-> {
    "feeds": [ ... ],
    "next_cursor": "eyJsYXN0IjoiYnRjLXZvbC1pbmRleCJ9",
    "total": 47
  }
```

**Cursor format**: Opaque base64-encoded JSON. Clients must not parse or construct cursors -- treat them as opaque strings. The relay uses the last `feed_id` as the cursor anchor (keyset pagination, not offset). This stays stable when feeds are added or removed between pages. When `next_cursor` is `null`, there are no more results.

---

## 4. Feed Lifecycle: FeedPublisherExt

The `FeedPublisherExt` extension handles the complete feed lifecycle: register on boot, bridge Bus Pulses to the relay on each tick, deregister on shutdown. It is auto-loaded when `[agent.feeds.*]` entries exist in the agent manifest.

`FeedPublisherExt` is a **Bus-to-relay bridge**. It does not stage or hold data. Cognition-layer extensions publish feed data directly to the Bus topic `feed:{id}:data` as Pulses. `FeedPublisherExt` subscribes to those Bus topics and forwards Pulses to the relay for remote delivery.

### 4.1 Extension Implementation

```rust
pub struct FeedPublisherExt {
    feeds: Vec<FeedConfig>,
    relay: RelayHandle,
    /// Bus subscriptions for each feed's data topic.
    bus_subs: Vec<BusSubscription>,
}

#[async_trait]
impl Extension for FeedPublisherExt {
    fn name(&self) -> &str { "feed-publisher" }
    fn layer(&self) -> ExtensionLayer { ExtensionLayer::Social }

    async fn on_boot(&mut self, ctx: &mut AgentContext) -> Result<()> {
        for feed in &self.feeds {
            // Register feed with relay (remote discovery + subscription routing)
            ctx.relay.register_feed(FeedRegistration {
                feed_id: feed.id.clone(),
                agent_id: ctx.agent_id.clone(),
                kind: feed.kind,
                schema: feed.schema.clone(),
                rate_hz: feed.rate_hz,
                access: feed.access.clone(),
                sample: feed.sample.clone(),
            }).await?;

            // Subscribe to the feed's Bus topic to bridge data to relay
            let sub = ctx.bus.subscribe(&format!("feed:{}:data", feed.id));
            self.bus_subs.push(sub);
        }
        Ok(())
    }

    async fn on_tick_end(&mut self, ctx: &mut AgentContext) -> Result<()> {
        // Bridge: drain Bus Pulses and forward to relay for remote subscribers
        for (i, sub) in self.bus_subs.iter_mut().enumerate() {
            while let Some(pulse) = sub.try_recv() {
                ctx.relay.publish_feed_data(&self.feeds[i].id, pulse.payload()).await?;
            }
        }
        Ok(())
    }

    async fn on_shutdown(&mut self, ctx: &mut AgentContext) -> Result<()> {
        for feed in &self.feeds {
            ctx.relay.deregister_feed(&feed.id).await?;
        }
        Ok(())
    }
}
```

### 4.2 Data Flow: Bus Publishing (No Hidden Channels)

Feed data flows exclusively through Bus and Store. There are no hidden channels.

```
Cognition Extension                    Bus                      FeedPublisherExt        Relay
(e.g., GasTrendExt)                    (ephemeral)              (Social layer)          (remote)
       |                                |                            |                     |
       | ctx.bus.publish(               |                            |                     |
       |   "feed:eth-gas-trend:data",   |                            |                     |
       |   Pulse::new(json!({...}))     |                            |                     |
       | )                              |                            |                     |
       | -----------------------------> |                            |                     |
       |                                | (local subscribers         |                     |
       |                                |  receive immediately)      |                     |
       |                                |                            |                     |
       |                                | sub.try_recv() ----------> |                     |
       |                                |                            |                     |
       |                                |                            | relay.publish_feed  |
       |                                |                            |   _data(id, data)   |
       |                                |                            | ------------------> |
       |                                |                            |                     |
       |                                |                            |       (remote       |
       |                                |                            |        subscribers  |
       |                                |                            |        receive)     |
```

**Key constraint**: Cognition-layer extensions publish to Bus topic `feed:{id}:data` as Pulses. They do NOT write to cortical state, side-band maps, or any other mechanism. `FeedPublisherExt` is purely a Bus-to-relay bridge: it subscribes to the Bus topic and forwards Pulses to the relay for delivery to remote subscribers. Local subscribers (in-process agents, in-process dashboard) receive Pulses directly from the Bus without going through the relay.

### 4.3 Pipeline Order

The extension layer execution order matters:

```
Perception -> Cognition -> Social
```

1. **Cognition layer** extensions (e.g., `GasTrendExt`) run during `on_observe`, compute derived values, and publish results directly to Bus topic `feed:{id}:data` as Pulses.
2. **Social layer** extensions (e.g., `FeedPublisherExt`) run during `on_tick_end`, drain Pulses from Bus subscriptions, and forward them to the relay for remote delivery.

This guarantees that derived data is computed and published to Bus before `FeedPublisherExt` bridges it to the relay.

### 4.4 Feed Data Computation Example

```rust
pub struct GasTrendExt {
    ema: f64,
    window: VecDeque<f64>,
}

#[async_trait]
impl Extension for GasTrendExt {
    fn name(&self) -> &str { "gas-trend" }
    fn layer(&self) -> ExtensionLayer { ExtensionLayer::Cognition }

    async fn on_observe(&mut self, ctx: &mut AgentContext) -> Result<()> {
        let gas = ctx.cortical.gas_gwei();
        self.window.push_back(gas);
        if self.window.len() > 100 { self.window.pop_front(); }

        // 12-block EMA
        let alpha = 2.0 / 13.0;
        self.ema = alpha * gas + (1.0 - alpha) * self.ema;

        // Percentiles from rolling window
        let mut sorted: Vec<f64> = self.window.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p25 = sorted[sorted.len() / 4];
        let p75 = sorted[3 * sorted.len() / 4];
        let p95 = sorted[19 * sorted.len() / 20];

        // MEV spike: gas exceeds 2x the 95th percentile
        let mev_spike = gas > p95 * 2.0;

        // Publish directly to Bus topic -- no hidden channel
        ctx.bus.publish(
            "feed:eth-gas-trend:data",
            Pulse::new(json!({
                "ema_12": self.ema,
                "p25": p25,
                "p75": p75,
                "p95": p95,
                "mev_spike": mev_spike,
                "current": gas,
                "ts": now_ms(),
            })),
        );

        Ok(())
    }
}
```

---

## 5. Exposing and Subscribing to Feeds

### 5.1 Agent Publishes a Feed

Any agent can register feeds with the relay. The relay handles discovery, subscription routing, and payment gating. The agent publishes data to the Bus topic; `FeedPublisherExt` bridges it to the relay for remote subscribers.

```rust
// Agent publishes a raw feed
ctx.relay.register_feed(FeedRegistration {
    feed_id: "eth-mainnet-blocks",
    agent_id: ctx.agent_id.clone(),
    kind: FeedKind::Raw,
    schema: FeedSchema::Named("eth_block".into()),
    rate_hz: 0.08,  // ~1 block per 12s
    access: FeedAccess::Public,
})?;

// Agent publishes a derived feed (computed from raw data)
ctx.relay.register_feed(FeedRegistration {
    feed_id: "eth-gas-trend",
    agent_id: ctx.agent_id.clone(),
    kind: FeedKind::Derived,
    schema: FeedSchema::Custom("gas_trend_v1".into()),
    rate_hz: 0.5,
    access: FeedAccess::Paid {
        base_price_usdc_per_hour: 100,
        accepted_protocols: vec![PaymentProtocol::X402, PaymentProtocol::Mpp],
    },
})?;
```

### 5.2 Dashboard Subscribes via WebSocket

The relay manages WebSocket subscription routing. Subscribers join a feed's Bus topic via the relay WebSocket.

```typescript
function useChainFeed(agentId: string, feedId: string) {
  const [data, setData] = useState<FeedData[]>([]);

  useEffect(() => {
    const ws = new WebSocket(`${relayUrl}/relay/ws`);
    ws.onopen = () => {
      ws.send(JSON.stringify({
        type: "subscribe",
        rooms: [`agent:${agentId}:feed:${feedId}`]
      }));
    };
    ws.onmessage = (e) => {
      const event = JSON.parse(e.data);
      if (event.type === "feed_data") {
        setData(prev => [...prev.slice(-999), event.payload]);
      }
    };
    return () => ws.close();
  }, [agentId, feedId]);

  return data;
}
```

### 5.3 Agent-to-Agent Feed Marketplace

Agents subscribe to each other's feeds, creating a data marketplace. For paid feeds, payment is handled automatically via the payment protocols defined in [18-PAYMENTS](18-PAYMENTS.md). The subscribing agent's budget is debited per hour.

```rust
// Agent B subscribes to Agent A's derived feed
let subscription = ctx.relay.subscribe_feed(SubscribeFeedRequest {
    feed_id: "eth-gas-trend",
    source_agent_id: "chain-watcher-1",
})?;

// For paid feeds, payment is handled automatically via the inference gateway's
// cost tracking. The subscribing agent's budget is debited per hour.
```

### 5.4 Agent Feed Consumer Extension

Agents consume feeds through extensions. The subscription is managed by the extension and the session is opened programmatically.

```rust
pub struct GasConsumerExt {
    gas_subscription: Option<FeedSubscription>,
}

#[async_trait]
impl Extension for GasConsumerExt {
    async fn on_boot(&mut self, ctx: &mut AgentContext) -> Result<()> {
        let session = ctx.mpp.open_session(
            "gas-oracle",  // agent producing the feed
            500,           // $0.0005 USDC
        ).await?;

        self.gas_subscription = Some(
            ctx.relay.subscribe_feed("eth-gas-trend", session.session_id).await?
        );
        Ok(())
    }

    async fn on_observe(&mut self, ctx: &mut AgentContext) -> Result<()> {
        if let Some(sub) = &self.gas_subscription {
            if let Some(data) = sub.latest() {
                let mev_spike = data["mev_spike"].as_bool().unwrap_or(false);
                if mev_spike {
                    ctx.cortical.set_prediction_error(0.8);
                }
            }
        }
        Ok(())
    }
}
```

### 5.5 Dashboard Subscribes to Paid Feed

Full flow from discovery through payment to live data:

```typescript
// 1. Discover available feeds
const feeds = await fetch(`${relayUrl}/relay/feeds`).then(r => r.json());
const gasFeed = feeds.find(f => f.feed_id === "eth-gas-trend");
// -> { feed_id, agent_id, kind: "derived", rate_hz: 0.5,
//      access: { paid: { price_per_hour: 50 } } }

// 2. Open an MPP session (one-time ERC-3009 signature)
const session = await openMppSession(relayUrl, {
  amount: 500,  // $0.0005 USDC -- enough for ~10 hours at $0.05/hr
  recipient: gasFeed.agent_wallet,
});
// -> { session_id: "abc-123", funded_amount: 500, status: "active" }

// 3. Subscribe to the feed via WebSocket with session auth
const ws = new WebSocket(`${relayUrl}/relay/ws`);
ws.onopen = () => {
  ws.send(JSON.stringify({
    type: "subscribe",
    rooms: [`feed:${gasFeed.feed_id}`],
    payment: {
      intent: "session",
      session_id: session.session_id,
    }
  }));
};

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.type === "feed_data") {
    // { ema_12: 42.5, p25: 35.0, p75: 55.0, p95: 120.0, mev_spike: false }
    updateGasChart(msg.payload);
  }
  if (msg.type === "payment_draw") {
    // { amount: 1, balance_remaining: 499, session_id: "abc-123" }
    updateBalance(msg.payload);
  }
};
```

---

## 6. Dynamic Endpoint Registration

Agents can register new feeds at runtime. When an agent discovers a new data source or creates a derived feed, it announces it to the relay. The relay publishes a `feed_registered` event on the `system` Bus topic so all connected subscribers discover the new feed.

```rust
// Agent discovers a new DEX and creates a feed for it
ctx.relay.register_feed(FeedRegistration {
    feed_id: format!("dex-{}-swaps", dex_address),
    agent_id: ctx.agent_id.clone(),
    kind: FeedKind::Derived,
    schema: FeedSchema::Custom("dex_swap_v1".into()),
    rate_hz: 2.0,
    access: FeedAccess::Public,
})?;

// The dashboard discovers this feed dynamically
// because it subscribes to the "system" room and receives
// feed_registered events
```

---

## 7. On-Chain Feed Advertisement (ERC-8004)

Agents with wallets advertise their feeds in their ERC-8004 passport. This makes feeds discoverable on-chain even when the agent or relay is offline.

### 7.1 Solidity Interface

```solidity
// AgentRegistry.sol -- feed advertisement extension
struct FeedAdvert {
    bytes32 feedId;        // keccak256 of feed name
    bytes32 schemaHash;    // keccak256 of schema definition
    uint16  rateMilliHz;   // rate in milli-Hz (500 = 0.5 Hz)
    uint96  pricePerHour;  // USDC base units per hour (0 = free)
    uint32  updatedAt;     // last update timestamp
}

function updateFeeds(FeedAdvert[] calldata adverts) external;
function getFeeds(address agent) external view returns (FeedAdvert[] memory);
```

### 7.2 Dual Registration

When an agent boots with feeds configured, it performs dual registration:

1. **Relay registration** (for live presence and subscription routing)
2. **On-chain advertisement** (for persistent discovery via ERC-8004 passport)

On feed config changes (add/remove/reprice), both relay and chain are updated.

```rust
// In FeedPublisherExt::on_boot()
async fn on_boot(&mut self, ctx: &mut AgentContext) -> Result<()> {
    for feed in &self.feeds {
        // Register with relay (live routing)
        ctx.relay.register_feed(/* ... */).await?;

        // Advertise on-chain (persistent discovery)
        if let Some(chain) = &ctx.chain_client {
            chain.update_feed_advert(&ctx.agent_wallet, FeedAdvert {
                feed_id: keccak256(feed.id.as_bytes()),
                schema_hash: keccak256(feed.schema.as_bytes()),
                rate_milli_hz: (feed.rate_hz * 1000.0) as u16,
                price_per_hour: feed.price_usdc_per_hour,
            }).await?;
        }
    }
    Ok(())
}
```

### 7.3 Merged Discovery

Feed discovery uses both relay (live) and chain (persistent) sources:

```typescript
// Dashboard merges relay (live) + chain (persistent) feed data
async function discoverFeeds(): Promise<Feed[]> {
  const [relayFeeds, chainFeeds] = await Promise.all([
    fetch(`${relayUrl}/relay/feeds`).then(r => r.json()),
    chainClient.getRegisteredFeeds(),  // reads all ERC-8004 feed adverts
  ]);

  // Merge: relay has live status, chain has persistent metadata
  return mergeFeeds(relayFeeds, chainFeeds);
  // Result: each feed has { ...chainAdvert, live: boolean, subscribers: number }
}
```

An agent's feeds appear in its passport even when the agent is offline. Users browsing the on-chain registry can see what feeds exist, their pricing, and their schemas -- then subscribe when the agent comes online.

---

## 8. TOML Manifest Declaration

Feeds are declared in the agent manifest (`roko.toml`). When the agent boots, `FeedPublisherExt` reads these declarations and registers them with the relay. No manual registration needed.

### 8.1 Producer Manifest

```toml
# roko.toml -- agent manifest
[agent]
name = "gas-oracle"
profile = "chain"
mode = "persistent"

[agent.feeds]
[agent.feeds.eth-gas-trend]
kind = "derived"
description = "12-block EMA gas price with percentile bands and MEV spike detection"
schema = "gas_trend_v1"
rate_hz = 0.5
access = "paid"
base_price_usdc_per_hour = 50  # $0.05/hr in USDC base units (6 decimals = 50 = $0.000050)
# For pricier feeds:
# base_price_usdc_per_hour = 500000  # $0.50/hr

[agent.feeds.eth-gas-trend.sample]
# Sample payload shown to prospective subscribers before they pay
data = '{"ema_12": 42.5, "p25": 35.0, "p75": 55.0, "p95": 120.0, "mev_spike": false}'
```

### 8.2 Consumer Manifest

```toml
[[agent.feed_subscriptions]]
feed_id = "binance-funding-rates"
agent_id = "cex-connector"
budget_usdc = 1000  # Session deposit

[[agent.feed_subscriptions]]
feed_id = "hyperliquid-funding-rates"
agent_id = "cex-connector"
budget_usdc = 1000
```

---

## 9. Extensibility Pattern

Any extension can produce a feed. The universal pattern:

1. Declare the feed in the agent manifest (`[agent.feeds.*]`).
2. Compute data in an extension's `on_observe()` or `on_tick_end()` hook.
3. Publish directly to Bus topic `feed:{id}:data` as a Pulse via `ctx.bus.publish(...)`.
4. `FeedPublisherExt` (auto-loaded when feeds are declared) bridges Bus Pulses to the relay for remote subscribers.

Users create custom feed schemas, set arbitrary pricing, compose feeds from other feeds. The relay handles discovery, payment gating, and delivery. The dashboard surfaces it all without any feed-specific code -- it reads the schema from the registry and renders accordingly.

---

## 10. Relay Feed Infrastructure

The relay manages the feed registry, payment gating, and message forwarding. All feed operations go through the relay -- producers publish to it, subscribers connect through it.

### 10.1 Payment Flow

```
Subscriber                    Relay                     Feed Producer
    |                           |                            |
    |  Open MPP session         |                            |
    |  (ERC-3009 auth)          |                            |
    | ------------------------> |  Store session ref         |
    |                           | -------------------------> |
    |  Subscribe to feed room   |                            |
    |  with session_id          |                            |
    | ------------------------> |                            |
    |                           |                            |
    |                           |  <-- feed_data ----------  |
    |                           |                            |
    |                           |  Draw from session:        |
    |                           |  cost = base_price         |
    |                           |        / rate_hz / 3600    |
    |                           |                            |
    |                           |  Draw succeeds?            |
    |  <-- feed_data ---------- |  Yes: forward              |
    |  <-- payment_draw ------- |                            |
    |                           |                            |
    |                           |  Draw fails (exhausted)?   |
    |  <-- exhaustion_notice -- |  Unsubscribe, notify       |
    |                           |                            |
    |  Top-up session           |                            |
    | ------------------------> |  Resume draws              |
    |                           |                            |
    |  Disconnect / unsubscribe |                            |
    | ------------------------> |  Session stays open        |
    |                           |  (reusable on reconnect)   |
```

### 10.2 Feed Subscription with Payment (WebSocket)

```json
{
  "type": "subscribe",
  "rooms": ["feed:eth-gas-trend"],
  "payment": {
    "intent": "session",
    "session_id": "abc-123"
  }
}
```

The relay verifies the MPP session with the feed producer's agent, then forwards feed data to the subscriber. Each forwarded message triggers a draw. See [18-PAYMENTS](18-PAYMENTS.md) for full payment protocol details.

---

## 11. Dashboard Integration

### 11.1 Feed Data-Source Mapping

| Section | WS rooms | Event types | REST fallback |
|---|---|---|---|
| Fleet / Feeds | `system` | `feed_registered`, `feed_deregistered`, `feed_status` | `GET /relay/feeds` |
| Fleet / Feed detail | `feed:{id}` | `feed_data`, `feed_status`, `payment_draw` | `GET /relay/feeds/{id}` |
| Treasury / Subscriptions | `system` | `session_opened`, `session_exhausted`, `session_settled` | `GET /mpp/sessions` |
| Treasury / Feed Revenue | `system` | `feed_revenue_update`, `settlement_batch` | `GET /relay/feeds/revenue` |

### 11.2 Chain Feed Auto-Subscribe

When the dashboard connects to a blockchain agent, it automatically subscribes to that agent's chain feeds for UI rendering.

```typescript
function useAgentChainFeeds(agent: MergedAgent) {
  const feeds = agent.feeds?.filter(f =>
    f.schema.startsWith("eth_") ||
    f.schema.startsWith("evm_") ||
    f.schema === "block" ||
    f.schema === "transaction"
  );

  // Auto-subscribe to chain feeds for live rendering
  for (const feed of feeds ?? []) {
    useRealtimeFeed(feed.feedId, {
      onData: (data) => {
        updateBlockHeight(data.blockNumber);
        updateGasGauge(data.gasUsed);
        updateTransactionList(data.transactions);
      }
    });
  }
}
```

### 11.3 Schema-Based Rendering

The dashboard renders different UI elements based on feed schema:

| Feed schema | Dashboard renders |
|---|---|
| `eth_block` | Block height counter, gas gauge, tx list |
| `evm_logs` | Live event log with contract decode |
| `gas_trend_*` | Gas price sparkline with percentile bands |
| `funding_*` | Funding rate chart with divergence alerts |
| `position_*` | Position cards with P&L |
| `tick_activity_*` | Liquidity heatmap |
| Any custom | Raw JSON viewer with auto-detected chart type |

For non-blockchain feeds, the dashboard renders based on the data shape -- numeric values get sparklines, boolean values get status indicators, arrays get tables.

### 11.4 Dashboard Surfaces

**Feeds page** (in Fleet or System section):

```
+--------------------------------------------------------------+
| Available Feeds                               [+ Publish Feed]|
|                                                               |
| Filter: [All v] [Paid v] [Chain: All v] [Search...]          |
|                                                               |
| +----------------------------------------------------------+ |
| | eth-gas-trend                                 * LIVE      | |
| | by gas-oracle (Trusted)                      $0.05/hr    | |
| | 12-block EMA gas with percentile bands + MEV detect      | |
| | Schema: gas_trend_v1   Rate: 0.5 Hz   Subs: 7           | |
| | [Preview]  [Subscribe]                                   | |
| +----------------------------------------------------------+ |
|                                                               |
| +----------------------------------------------------------+ |
| | uniswap-v3-tick-activity                     * LIVE      | |
| | by pool-watcher (Verified)                  $0.20/hr    | |
| | Real-time tick-level activity for top 50 pools           | |
| | Schema: tick_activity_v2   Rate: 2 Hz   Subs: 3         | |
| | [Preview]  [Subscribe]                                   | |
| +----------------------------------------------------------+ |
|                                                               |
| +----------------------------------------------------------+ |
| | eth-mainnet-blocks                           * LIVE      | |
| | by chain-watcher-1 (Basic)                     FREE     | |
| | Raw Ethereum mainnet block headers                       | |
| | Schema: eth_block   Rate: 0.08 Hz   Subs: 12            | |
| | [Preview]  [Subscribe]                                   | |
| +----------------------------------------------------------+ |
+--------------------------------------------------------------+
```

**My subscriptions** (in Treasury or Settings):

```
+--------------------------------------------------------------+
| My Feed Subscriptions                                         |
|                                                               |
| Active spend: $0.25/hr across 3 feeds                        |
|                                                               |
| +----------------------------------------------------------+ |
| | eth-gas-trend          * Active    Session: $4.82 left    | |
| | gas-oracle             $0.05/hr   Since: 2h ago           | |
| | [Pause] [Top-up $5] [Unsubscribe]                        | |
| +----------------------------------------------------------+ |
|                                                               |
| +----------------------------------------------------------+ |
| | cross-chain-gas-arb    * Active    Session: $1.20 left    | |
| | arb-bot                $0.50/hr   Since: 45m ago          | |
| | [Pause] [Top-up $10] [Unsubscribe]                       | |
| +----------------------------------------------------------+ |
|                                                               |
| Total spent this month: $12.40                                |
| Total earned from my feeds: $8.70                             |
+--------------------------------------------------------------+
```

**Feed detail page** (click into a feed):

```
+--------------------------------------------------------------+
| eth-gas-trend                                     * LIVE      |
| by gas-oracle (Trusted, 342 episodes)            $0.05/hr    |
|                                                               |
| +--------------- Live Preview -------------------------+     |
| | EMA: 42.5 gwei   P25: 35.0   P75: 55.0   P95: 120  |     |
| | MEV: none                                            |     |
| |                                                      |     |
| | [sparkline chart of last 100 data points]            |     |
| +------------------------------------------------------+     |
|                                                               |
| Schema: gas_trend_v1                                          |
| Fields: ema_12 (f64), p25 (f64), p75 (f64), p95 (f64),      |
|         mev_spike (bool), current (f64), ts (u64)            |
|                                                               |
| Uptime: 99.7% (30d)   Avg latency: 120ms                     |
| Subscribers: 7   Revenue: $84.20 (30d)                        |
|                                                               |
| Dependencies: eth-mainnet-blocks (free)                       |
|                                                               |
| Payment: x402 or MPP session                                  |
| [Subscribe with MPP ($5 deposit)]  [Try with x402 ($0.01)]   |
+--------------------------------------------------------------+
```

**Feed revenue** (in Treasury / Cost Analytics):

```
+--------------------------------------------------------------+
| Feed Revenue                                                  |
|                                                               |
| Total earned (30d): $84.20    Active subscribers: 7           |
|                                                               |
| Feed               Subs  Revenue/30d  Status                  |
| eth-gas-trend       7     $84.20      * producing             |
|                                                               |
| [chart: revenue over time, subscriber count over time]        |
|                                                               |
| Settlement: 12 batches settled on-chain                       |
| Pending: $2.30 (next batch in ~8 min)                         |
+--------------------------------------------------------------+
```

---

## 12. Recipes

> Added 2026-04-24. Recipe is a first-class specialization in the vocabulary.

### 12.1 Definition

A **Recipe** is a composable data transformation pipeline. Its shape is `create / execute / chain / status / configure`. Recipes are the composition glue between Feeds, Score Cells, and Signals -- they turn raw data streams into derived values.

**Kernel mapping**: Recipe = Graph of Score Cells. Pure data pipeline: Feed -> transform -> output. No LLM inference, no agent dispatch.

### 12.2 Why Recipe is Distinct from Plan and Compose

| Concept | What It Is | Involves LLM? | Involves Agents? |
|---|---|---|---|
| **Plan** | Agent task DAG (tasks with dependencies, assigned to agents) | Yes | Yes |
| **Compose** | Prompt assembly for LLM calls (VCG auction, section effects) | Yes | Yes |
| **Recipe** | Pure data pipeline: Feed -> transform -> output | No | No |

Recipes make indicator chains, P&L attribution, HDC encoding, and scoring pipelines into first-class composable objects that users can author, share, and backtest.

### 12.3 Core Recipe Types

```rust
/// A Recipe is a Graph of Score Cells that transforms Feed data
/// into derived values without LLM inference.
pub struct RecipeCell {
    /// Feed IDs that provide input data for this recipe.
    pub input_feeds: Vec<String>,
    /// Ordered transformation stages. Each stage is a Score Cell.
    pub stages: Vec<RecipeStage>,
    /// Where the recipe output goes.
    pub output: OutputTarget,
}

/// A single transformation stage in a Recipe pipeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipeStage {
    /// Unique identifier for this stage within the recipe.
    pub id: String,
    /// The Score Cell that performs the transformation.
    pub cell_id: String,
    /// Parameters for the Score Cell (e.g., window size, threshold).
    pub params: serde_json::Value,
    /// Input mapping: which fields from prior stage(s) to consume.
    pub input_map: HashMap<String, String>,
    /// Output fields this stage produces.
    pub output_fields: Vec<FieldSpec>,
}

/// Specification for a single output field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldSpec {
    pub name: String,
    pub field_type: FieldType,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FieldType {
    F64,
    I64,
    Bool,
    String,
    Vec(Box<FieldType>),
    Json,
}

/// Where the recipe output is directed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OutputTarget {
    /// Publish as a new Feed on Bus topic `feed:{id}:data`.
    Feed { feed_id: String, rate_hz: f64, access: FeedAccess },
    /// Graduate to a Signal in Store.
    Signal { kind: String },
    /// Write to a knowledge entry in the neuro store.
    Knowledge { topic: String, tier: KnowledgeTier },
    /// Raw value: return to caller (for interactive/backtest use).
    Raw,
}

impl Cell for RecipeCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn name(&self) -> &str { "recipe" }

    fn input_schema(&self) -> TypeSchema {
        TypeSchema::named("Vec<Pulse>")  // Feed data Pulses
    }
    fn output_schema(&self) -> TypeSchema {
        TypeSchema::named("RecipeOutput")
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut data = extract_feed_data(&input)?;

        // Run each stage in sequence
        for stage in &self.stages {
            let cell = ctx.resolve_cell(&stage.cell_id)?;
            let stage_input = apply_input_map(&data, &stage.input_map)?;
            data = cell.execute(stage_input, ctx).await?;
        }

        // Route output to target
        match &self.output {
            OutputTarget::Feed { feed_id, .. } => {
                ctx.bus.publish(
                    &format!("feed:{}:data", feed_id),
                    Pulse::from_signals(&data),
                );
            }
            OutputTarget::Signal { kind } => {
                for signal in &mut data {
                    signal.kind = kind.clone().into();
                    ctx.store.put(signal.clone()).await?;
                }
            }
            OutputTarget::Knowledge { topic, tier } => {
                ctx.store.put_knowledge(topic, &data, *tier).await?;
            }
            OutputTarget::Raw => {
                // Return directly -- caller handles the output
            }
        }

        Ok(data)
    }
}
```

### 12.4 Recipe TOML Graph Definition

Recipes are defined as TOML Graphs and loaded at startup or runtime:

```toml
[graph]
id = "macd-regime-recipe"
description = "Price feed -> MACD -> RSI -> regime score -> trading signal"

[[graph.cells]]
id = "macd"
protocol = "Score"
cell_type = "MacdCell"
input_schema = "PriceFeed"
output_schema = "MacdOutput"

[graph.cells.params]
fast_period = 12
slow_period = 26
signal_period = 9

[[graph.cells]]
id = "rsi"
protocol = "Score"
cell_type = "RsiCell"
input_schema = "PriceFeed"
output_schema = "RsiOutput"

[graph.cells.params]
period = 14

[[graph.cells]]
id = "regime-score"
protocol = "Score"
cell_type = "RegimeScoreCell"
input_schema = "MacdOutput + RsiOutput"
output_schema = "RegimeScore"

[graph.cells.params]
thresholds = { calm = 0.3, normal = 0.6, volatile = 0.8, crisis = 0.95 }

[[graph.cells]]
id = "signal-emitter"
protocol = "Score"
cell_type = "ThresholdSignalCell"
input_schema = "RegimeScore"
output_schema = "TradingSignal"

[graph.cells.params]
buy_threshold = 0.7
sell_threshold = 0.3

[[graph.edges]]
from = "macd.output"
to = "regime-score.macd_input"

[[graph.edges]]
from = "rsi.output"
to = "regime-score.rsi_input"

[[graph.edges]]
from = "regime-score.output"
to = "signal-emitter.input"

[graph.output]
target = "feed"
feed_id = "macd-regime-signal"
rate_hz = 0.1
access = "paid"
base_price_usdc_per_hour = 100000
```

### 12.5 Cross-Domain Examples

| Domain | Recipe | Pipeline |
|---|---|---|
| DeFi | Indicator pipeline | price feed -> MACD -> RSI -> regime score -> trading signal |
| DeFi | P&L attribution | fill events -> FIFO matching -> realized P&L -> Sharpe ratio |
| DeFi | HDC encoding | market state -> role-filler binding -> hypervector -> similarity search |
| Governance | Voting power | delegation graph -> power aggregation -> threshold signal |
| Ops | Cost tracking | agent token usage -> pricing -> daily/weekly rollup -> budget signal |
| Code | Quality scoring | diff -> lint warnings -> test coverage -> quality score |

### 12.6 Rust Mapping

Recipes compose `Score` protocol Cell instances from `roko-core`. Existing implementations that become recipe templates:

| Crate | Struct | Recipe type |
|---|---|---|
| `roko-learn` | `TradingReflect` | P&L attribution pipeline |
| `roko-learn` | `FifoMatcher` | Position matching transform |
| `roko-learn` | `IndicatorTracker` | Indicator accuracy pipeline |
| `roko-primitives` | `MarketHdcEncoder` | HDC encoding pipeline |
| `roko-dreams` | `CounterfactualEngine` | Alternative outcome simulation |

### 12.7 Dashboard Authoring Surface

The Recipe Editor is a 4-stage authoring surface:

1. **Input selection** -- choose feed(s) or connector query as input (drag from feed/connector list)
2. **Pipeline builder** -- chain transform stages: map, filter, window, aggregate, score (visual DAG editor)
3. **Output configuration** -- emit as: Signal, Knowledge Entry, Feed, or raw value (type-checked output)
4. **Backtest and validate** -- run against historical data, compare output distribution (chart overlay)

---

## 13. Crate Mapping

| Component | Crate | Status |
|---|---|---|
| Feed types (`FeedRegistration`, `FeedKind`, `FeedAccess`) | `roko-core` | Kernel types |
| Feed registry + relay routing | `roko-serve` (relay routes) | Wired |
| `FeedPublisherExt` | `roko-agent` (extensions) | Wired |
| Feed subscription management | `roko-runtime` (relay client) | Wired |
| Feed on-chain advertisement | `roko-chain` (Phase 2+) | Deferred |
| Recipe types (`RecipeCell`, `RecipeStage`, `OutputTarget`) | `roko-core` | Kernel types |
| Recipe templates | `roko-learn`, `roko-primitives` | Existing |
| Recipe authoring UI | Dashboard | Depends on [20-SURFACES](20-SURFACES.md) |
| Feed payment protocols (x402, MPP) | See [18-PAYMENTS](18-PAYMENTS.md) | Separate doc |

---

## 14. Acceptance Criteria

### Feed Primitive

- [ ] `FeedRegistration`, `FeedKind`, `FeedSchema`, `FeedAccess` types exist in `roko-core`
- [ ] Feed Cell conforms to Connect + Trigger + Store protocols
- [ ] `FeedPublisherExt` auto-loads when `[agent.feeds.*]` exists in manifest
- [ ] Register on boot, publish on tick, deregister on shutdown lifecycle works end-to-end
- [ ] Extension layer ordering (Cognition before Social) is enforced

### Feed Data Flow (No Hidden Channels)

- [ ] Cognition-layer extensions publish feed data to Bus topic `feed:{id}:data` as Pulses
- [ ] No data flows through `ctx.cortical.set_feed_data()` or any hidden staging map
- [ ] `FeedPublisherExt` is a pure Bus-to-relay bridge: subscribes to Bus topics, forwards to relay
- [ ] Local subscribers receive Pulses directly from Bus without relay roundtrip
- [ ] Remote subscribers receive Pulses via relay forwarding from `FeedPublisherExt`

### Feed Registry

- [ ] `POST /relay/feeds/register` creates feed entries in registry Store
- [ ] `GET /relay/feeds` returns all feeds with cursor-based pagination
- [ ] Query parameters (kind, access, agent_id, schema, search) filter correctly
- [ ] `GET /relay/feeds/{feed_id}/sample` returns sample data without auth
- [ ] `feed_registered` and `feed_deregistered` Pulses publish on `system` Bus topic

### Subscription Routing

- [ ] WebSocket subscribe to `feed:{id}` room delivers Pulses to subscriber
- [ ] Paid feed subscription validates payment session before forwarding
- [ ] Session exhaustion pauses delivery and sends exhaustion notice
- [ ] Session top-up resumes delivery
- [ ] Agent-to-agent subscription works programmatically via `subscribe_feed`

### Dynamic Registration

- [ ] Agents can register new feeds at runtime (not just on boot)
- [ ] Dynamic registration publishes `feed_registered` event on `system` room
- [ ] Dashboard discovers dynamically registered feeds via WebSocket

### On-Chain Advertisement

- [ ] `FeedAdvert` struct exists in Solidity with feedId, schemaHash, rateMilliHz, pricePerHour, updatedAt
- [ ] `updateFeeds` and `getFeeds` functions exist on AgentRegistry contract
- [ ] Dual registration (relay + chain) executes on boot when chain client is configured
- [ ] Merged discovery returns both live (relay) and persistent (chain) feed data

### Recipes

- [ ] `RecipeCell` type exists with `input_feeds: Vec<String>`, `stages: Vec<RecipeStage>`, `output: OutputTarget`
- [ ] `RecipeStage` type exists with `id`, `cell_id`, `params`, `input_map`, `output_fields`
- [ ] `OutputTarget` enum supports Feed, Signal, Knowledge, and Raw variants
- [ ] Recipe conforms to `create / execute / chain / status / configure` shape
- [ ] At least 5 recipe templates exist from existing crate implementations
- [ ] Recipe TOML Graph definition loads and validates (cells, edges, output config)
- [ ] Backtest queries Store for historical data and replays through recipe stages
- [ ] Recipe output graduates to Signal when OutputTarget::Signal is configured
- [ ] Recipe output publishes to Bus topic `feed:{id}:data` when OutputTarget::Feed is configured
- [ ] Recipe output writes to neuro store when OutputTarget::Knowledge is configured
- [ ] Recipe Editor in dashboard supports 4-stage authoring (input, pipeline, output, backtest)
- [ ] Recipe stages compose: output fields of stage N match input_map of stage N+1
- [ ] Recipe with invalid stage composition (type mismatch) fails at load time with clear error
- [ ] Recipe TOML is loadable via `roko config validate` (same as other TOML artifacts)
- [ ] Recipe can be shared between agents via feed registry (output as Feed)

### TOML Config

- [ ] `[agent.feeds.*]` in `roko.toml` is parsed into `FeedConfig` structs
- [ ] `[[agent.feed_subscriptions]]` in `roko.toml` is parsed and subscriptions opened on boot
- [ ] Sample payload config (`[agent.feeds.*.sample]`) is served via registry

### Integration

- [ ] Feed composition chains work: raw -> derived -> composite -> subscriber
- [ ] Payment draws compute correctly: `cost = base_price / rate_hz / 3600` per message
- [ ] Dashboard renders feed data based on schema type (sparklines, gauges, tables)
- [ ] All four feed kinds (raw, derived, composite, meta) can be registered and discovered
