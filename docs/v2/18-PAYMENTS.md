# 18 -- Payments

> Two payment protocols (x402 per-request, MPP session-based), reputation-based pricing with 5 tiers, feed marketplace economics, and relay payment flow. Payment is a Cell-level concern -- each protocol is a Verify Cell in the feed subscription pipeline.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal), [02-CELL](02-CELL.md) (Verify protocol), [12-CONNECTIVITY](12-CONNECTIVITY.md) (Relay, feeds), [17-AUTH](17-AUTH.md) (agent bearer tokens)

---

## 1. Overview

Roko supports two payment protocols for paid feeds and agent services. Both are implemented as Verify Cells in the feed subscription pipeline -- a subscription request passes through the payment Verify Cell before data flows.

| Protocol | Model | Signing | Settlement | Use case |
|---|---|---|---|---|
| **x402** | Per-request, stateless | ERC-3009 per request | Batch (10min or 100+ auths) | On-demand queries, trying a feed |
| **MPP** | Session-based, streaming | One ERC-3009 per session | On session close/expire | Continuous feeds, multi-agent pipelines |

### 1.1 Payment as Verify Cells

Each payment protocol is implemented as a concrete Cell struct conforming to `Cell + VerifyProtocol`. These Cells sit in the feed subscription pipeline: a subscription request must pass through the appropriate payment Verify Cell before any data flows to the subscriber.

```rust
/// x402 per-request payment verification.
/// Sits in the feed subscription pipeline. Rejects requests
/// that lack a valid ERC-3009 authorization header.
pub struct VerifyX402Cell {
    /// Pending authorizations waiting for batch settlement.
    settlement: X402Settlement,
}

impl Cell for VerifyX402Cell {
    fn name(&self) -> &str { "verify-x402" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
}

#[async_trait]
impl VerifyProtocol for VerifyX402Cell {
    async fn verify_pre(&self, signal: &Signal, ctx: &CellContext) -> Result<Verdict> {
        // Extract X-Payment header from the request Signal
        let auth = match signal.metadata.get("x_payment") {
            Some(header) => parse_erc3009_authorization(header)?,
            None => return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::PaymentMissing,
                reason: "No X-Payment header. Send GET first to receive 402 with payment terms.".into(),
            }),
        };

        // Verify ERC-3009 signature locally via ecrecover (no RPC needed)
        if !verify_erc3009_signature(&auth)? {
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::PaymentInvalid,
                reason: "ERC-3009 signature verification failed.".into(),
            });
        }

        // Check amount >= required price (with reputation tier discount)
        let pricing = ctx.store().get::<ReputationPricing>(&auth.sender).await?;
        if auth.amount < pricing.effective_price() {
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::PaymentInsufficient,
                reason: format!("Amount {} < required {}", auth.amount, pricing.effective_price()),
            });
        }

        // Check nonce and expiry
        if auth.expiry < now_unix() {
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::PaymentExpired,
                reason: "Authorization expired.".into(),
            });
        }

        Ok(Verdict {
            passed: true,
            reward: 1.0,
            evidence: Evidence::PaymentVerified { protocol: "x402", amount: auth.amount },
            reason: "x402 payment verified.".into(),
        })
    }

    async fn verify_post(&self, _signal: &Signal, _output: &Signal, _ctx: &CellContext) -> Result<Verdict> {
        // Post-verification: collect the authorization for batch settlement
        // (handled in execute() after verify_pre passes)
        Ok(Verdict::pass())
    }
}

impl VerifyX402Cell {
    async fn execute(&mut self, input: Vec<Signal>, ctx: &CellContext) -> Result<Vec<Signal>> {
        let auth = parse_erc3009_authorization(input[0].metadata.get("x_payment").unwrap())?;

        // Collect for batch settlement
        self.settlement.pending.push(auth);

        // Check if batch settlement should fire
        if self.settlement.should_settle() {
            if let Some(chain) = ctx.chain_client() {
                self.settlement.settle(chain).await?;
            }
        }

        // Pass through: return the feed data Signal
        Ok(input)
    }
}

/// MPP session-based payment verification.
/// Sits in the feed subscription pipeline. Verifies that the subscriber
/// has an active MPP session with sufficient balance for the draw.
pub struct VerifyMppCell {
    /// Active sessions indexed by session_id.
    sessions: HashMap<SessionId, MppSession>,
}

impl Cell for VerifyMppCell {
    fn name(&self) -> &str { "verify-mpp" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
}

#[async_trait]
impl VerifyProtocol for VerifyMppCell {
    async fn verify_pre(&self, signal: &Signal, ctx: &CellContext) -> Result<Verdict> {
        // Extract session_id from the subscription request
        let session_id = match signal.metadata.get("mpp_session_id") {
            Some(id) => SessionId::from(id),
            None => return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::PaymentMissing,
                reason: "No MPP session_id. Create a session via POST /mpp/sessions first.".into(),
            }),
        };

        // Look up the session
        let session = match self.sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::SessionNotFound,
                reason: format!("MPP session {} not found.", session_id),
            }),
        };

        // Check session status
        match &session.status {
            SessionStatus::Active => {}
            SessionStatus::Exhausted => return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::SessionExhausted,
                reason: "Session balance exhausted. Top-up to resume.".into(),
            }),
            SessionStatus::Expired => return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::SessionExpired,
                reason: "Session expired.".into(),
            }),
            SessionStatus::Settled { .. } => return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::SessionSettled,
                reason: "Session already settled.".into(),
            }),
        }

        // Check sufficient balance for one draw
        let draw_cost = per_message_cost(
            ctx.store().get::<FeedConfig>(&signal.feed_id()).await?.base_price_per_hour,
            ctx.store().get::<FeedConfig>(&signal.feed_id()).await?.rate_hz,
        );

        if session.balance_remaining < draw_cost {
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::PaymentInsufficient,
                reason: format!("Balance {} < draw cost {}", session.balance_remaining, draw_cost),
            });
        }

        Ok(Verdict {
            passed: true,
            reward: 1.0,
            evidence: Evidence::PaymentVerified { protocol: "mpp", amount: draw_cost },
            reason: "MPP session payment verified.".into(),
        })
    }

    async fn verify_post(&self, _signal: &Signal, _output: &Signal, _ctx: &CellContext) -> Result<Verdict> {
        Ok(Verdict::pass())
    }
}

impl VerifyMppCell {
    async fn execute(&mut self, input: Vec<Signal>, ctx: &CellContext) -> Result<Vec<Signal>> {
        let session_id = SessionId::from(input[0].metadata.get("mpp_session_id").unwrap());
        let session = self.sessions.get_mut(&session_id).unwrap();

        let draw_cost = per_message_cost(
            ctx.store().get::<FeedConfig>(&input[0].feed_id()).await?.base_price_per_hour,
            ctx.store().get::<FeedConfig>(&input[0].feed_id()).await?.rate_hz,
        );

        // Deduct from session balance
        session.balance_remaining = session.balance_remaining.saturating_sub(draw_cost);
        session.draws.push(MppDraw {
            amount: draw_cost,
            feed_id: input[0].feed_id().to_string(),
            message_id: input[0].hash().to_string(),
            timestamp: Utc::now(),
        });

        // Transition to Exhausted if balance is zero
        if session.balance_remaining == 0 {
            session.status = SessionStatus::Exhausted;
            // Publish exhaustion notice on Bus
            ctx.bus().publish(Pulse {
                topic: format!("mpp:session:{}:exhausted", session_id),
                payload: json!({ "session_id": session_id, "total_draws": session.draws.len() }),
                seq: ctx.next_seq(),
            }).await?;
        }

        Ok(input)
    }
}
```

---

## 2. x402: Per-Request Stateless Payment

The simplest payment flow. No session, no state. Each request carries its own authorization.

### 2.1 Protocol Flow

```
Client                                  Server (relay / agent)
  |                                         |
  |  GET /relay/feeds/eth-gas-trend/data    |
  | ---------------------------------------> |
  |                                         |
  |  HTTP 402                               |
  |  X-Payment-Required:                    |
  |    amount=50, recipient=0xABC...,       |
  |    nonce=1, expiry=1714000000           |
  | <--------------------------------------- |
  |                                         |
  |  Client signs ERC-3009 authorization    |
  |  (gasless USDC approval, no on-chain tx)|
  |                                         |
  |  GET /relay/feeds/eth-gas-trend/data    |
  |  X-Payment: <signed authorization>      |
  | ---------------------------------------> |
  |                                         |
  |  Server verifies signature (ecrecover,  |
  |  no RPC needed), serves content         |
  |                                         |
  |  200 OK + feed data                     |
  | <--------------------------------------- |
```

### 2.2 ERC-3009 Signatures

x402 uses ERC-3009 `transferWithAuthorization` signatures. The client signs an authorization for USDC transfer without submitting an on-chain transaction. The server verifies the signature locally via `ecrecover` -- no RPC call needed for verification. The authorization is collected and settled later.

### 2.3 Batch Settlement

Settlement happens in batches: every 10 minutes or after 100+ accumulated authorizations, whichever comes first. The server submits a single on-chain transaction that settles all pending authorizations. This amortizes gas costs across many payments.

```rust
pub struct X402Settlement {
    /// Pending authorizations waiting for settlement.
    pending: Vec<Erc3009Authorization>,
    /// Settlement triggers.
    max_pending: usize,         // default: 100
    settle_interval: Duration,  // default: 10 minutes
    /// Last settlement timestamp.
    last_settled_at: Instant,
}

impl X402Settlement {
    /// Check if settlement should fire.
    pub fn should_settle(&self) -> bool {
        self.pending.len() >= self.max_pending
            || self.last_settled_at.elapsed() >= self.settle_interval
    }

    /// Submit batch settlement on-chain.
    pub async fn settle(&mut self, chain: &ChainClient) -> Result<TxHash> {
        let batch = std::mem::take(&mut self.pending);
        let tx = chain.batch_transfer_with_authorization(&batch).await?;
        self.last_settled_at = Instant::now();
        Ok(tx)
    }
}
```

---

## 3. MPP: Session-Based Streaming Payment

For continuous feeds. One signature funds an entire session. No re-signing per message.

### 3.1 Protocol Flow

```
Client                                  Server (relay / agent)
  |                                         |
  |  POST /mpp/sessions                     |
  |  { amount: 500, authorization: <sig> }  |
  | ---------------------------------------> |
  |                                         |
  |  201 Created                            |
  |  { session_id: "abc-123",               |
  |    funded: 500, status: "active" }      |
  | <--------------------------------------- |
  |                                         |
  |  WS subscribe with session_id           |
  |  { rooms: ["feed:eth-gas-trend"],       |
  |    payment: { session_id: "abc-123" } } |
  | ---------------------------------------> |
  |                                         |
  |  Per-message draw from session          |
  |  (no client interaction needed)         |
  |                                         |
  |  feed_data: { ema_12: 42.5, ... }       |
  |  payment_draw: { amount: 1,             |
  |    balance_remaining: 499 }             |
  | <--------------------------------------- |
```

### 3.2 Session Lifecycle

```
Active --> Exhausted --> Expired --> Settled
  |            |                       |
  |  (top-up)  |                       |
  +------------+                       +-- Refund unspent balance
```

- **Active**: draws succeed, messages flow.
- **Exhausted**: balance hits zero. Server sends exhaustion notice and pauses delivery. Client can top-up to resume.
- **Expired**: TTL reached (default 24h). No more draws. Transitions to Settled.
- **Settled**: unspent balance refunded. Session closed. Settlement submitted on-chain.

```rust
pub struct MppSession {
    pub id: SessionId,
    pub subscriber: AgentId,
    pub producer: AgentId,
    pub funded_amount: u64,         // USDC base units
    pub balance_remaining: u64,
    pub status: SessionStatus,
    pub authorization: Erc3009Authorization,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,  // default: created_at + 24h
    pub draws: Vec<MppDraw>,
}

pub enum SessionStatus {
    Active,
    Exhausted,
    Expired,
    Settled { tx_hash: TxHash, refund_amount: u64 },
}

pub struct MppDraw {
    pub amount: u64,
    pub feed_id: String,
    pub message_id: String,
    pub timestamp: DateTime<Utc>,
}
```

### 3.3 Top-Up

When a session is exhausted, the client can top-up without creating a new session:

```
POST /mpp/sessions/{session_id}/topup
{ amount: 500, authorization: <sig> }
-> { balance_remaining: 500, status: "active" }
```

Top-up resumes delivery immediately. The session's draw history is preserved.

### 3.4 One-Signature Streaming

The key property of MPP: the client signs once (at session creation or top-up). All subsequent draws happen server-side without client interaction. This enables continuous feed consumption without re-signing for every message -- critical for agents that consume feeds autonomously.

---

## 4. When to Use Which

| Scenario | Protocol | Why |
|---|---|---|
| Try a feed for 5 minutes | x402 | No session overhead, pay per message |
| Subscribe to a price feed for 24h | MPP | One signature, draws per tick |
| Query an agent's analysis on-demand | x402 | Stateless, pay per query |
| Multi-agent pipeline consuming feeds | MPP | Pre-funded sessions per pipeline stage |
| Webhook-triggered one-shot request | x402 | No state to manage |
| Dashboard monitoring live feed | MPP | Continuous stream, balance visible in UI |
| Agent consuming another agent's feed | MPP | Autonomous operation without re-signing |
| Trying feed before committing | x402 | Pay-per-request, low commitment |

---

## 5. Reputation-Based Pricing

Higher ERC-8004 reputation tier = lower markup. Applied on top of the feed producer's base price. The spread goes to the relay as an infrastructure fee.

### 5.1 Five Tiers

| Tier | Markup | Description |
|---|---|---|
| **None** | +20% | No on-chain reputation record |
| **Basic** | +18% | Agent registered on-chain |
| **Verified** | +15% | Agent has verified identity (Privy + wallet link) |
| **Trusted** | +12% | Agent has > 100 completed episodes with > 0.7 pass rate |
| **Sovereign** | +8% | Agent has > 1000 episodes, > 0.85 pass rate, > 90d uptime |

### 5.2 Pricing Example

A feed priced at $0.10/hr:

| Subscriber tier | Effective price | Relay fee |
|---|---|---|
| None | $0.120/hr | $0.020/hr |
| Basic | $0.118/hr | $0.018/hr |
| Verified | $0.115/hr | $0.015/hr |
| Trusted | $0.112/hr | $0.012/hr |
| Sovereign | $0.108/hr | $0.008/hr |

The producer always receives the base price ($0.10/hr). The markup is the relay's cut, and it decreases as the subscriber builds reputation.

```rust
pub struct ReputationPricing {
    pub base_price: u64,        // producer's price in USDC base units
    pub tier: ReputationTier,
    pub markup_bps: u16,        // basis points added to base price
}

pub enum ReputationTier {
    None,       // +2000 bps (20%)
    Basic,      // +1800 bps (18%)
    Verified,   // +1500 bps (15%)
    Trusted,    // +1200 bps (12%)
    Sovereign,  //  +800 bps (8%)
}

impl ReputationTier {
    pub fn markup_bps(&self) -> u16 {
        match self {
            Self::None => 2000,
            Self::Basic => 1800,
            Self::Verified => 1500,
            Self::Trusted => 1200,
            Self::Sovereign => 800,
        }
    }
}

impl ReputationPricing {
    pub fn effective_price(&self) -> u64 {
        let markup = (self.base_price as u128 * self.tier.markup_bps() as u128) / 10000;
        self.base_price + markup as u64
    }

    pub fn relay_fee(&self) -> u64 {
        self.effective_price() - self.base_price
    }
}
```

---

## 6. Relay Payment Flow

The relay manages the feed registry, payment gating, and message forwarding. All feed operations go through the relay -- producers publish to it, subscribers connect through it.

### 6.1 Payment Flow Diagram

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

### 6.2 Per-Message Draw Calculation

Each forwarded message triggers a draw from the session balance:

```rust
pub fn per_message_cost(base_price_per_hour: u64, rate_hz: f64) -> u64 {
    // base_price_per_hour is in USDC base units (6 decimals)
    // rate_hz is messages per second
    // cost per message = price_per_hour / (rate_hz * 3600)
    let messages_per_hour = (rate_hz * 3600.0) as u64;
    if messages_per_hour == 0 { return base_price_per_hour; }
    base_price_per_hour / messages_per_hour
}
```

---

## 7. Payment Disputes

Payment disputes arise when a subscriber believes they were charged for incorrect, stale, or missing data. The dispute system provides a structured resolution flow without requiring external arbitration for most cases.

### 7.1 Dispute Triggers

| Trigger | Detection | Example |
|---|---|---|
| **Wrong data** | `feed-accuracy` meta feed shows accuracy drop below threshold | A price feed reports $0.01 ETH instead of $3,000 |
| **Stale data** | `feed-health` meta feed detects staleness (no update for 3x expected interval) | Feed advertised at 0.5 Hz hasn't updated in 10 seconds |
| **Service interruption** | Subscriber received no data during an active MPP session | Relay outage, producer crash, network partition |
| **Overcharge** | Draw amount exceeds the per-message cost at the subscriber's reputation tier | Bug in pricing logic |

### 7.2 Dispute Resolution Flow

```
Subscriber                    Relay                     Feed Producer
    |                           |                            |
    |  POST /disputes           |                            |
    |  { session_id, reason,    |                            |
    |    evidence: [...] }      |                            |
    | ------------------------> |                            |
    |                           |  Create dispute record     |
    |                           |  Freeze draws on session   |
    |                           |                            |
    |                           |  Notify producer           |
    |                           | -------------------------> |
    |                           |                            |
    |                           |  72-hour response window   |
    |                           |                            |
    |                           |  Producer responds:        |
    |                           |  accept / contest          |
    |                           | <------------------------- |
    |                           |                            |
    |  Resolution applied       |                            |
    | <------------------------ |                            |
```

### 7.3 Resolution Outcomes

**Automatic resolution** (no human needed):

| Condition | Action |
|---|---|
| `feed-accuracy` score < 0.5 for the disputed period | Credit subscriber from next MPP session. Producer's TraceRank -0.02. |
| `feed-health` confirms staleness during disputed period | Pro-rata refund for stale duration. No reputation hit if < 5 minutes. |
| Producer does not respond within 72 hours | Dispute auto-resolved in subscriber's favor. Full session credit. |

**Contested resolution** (relay arbitrates):

When the producer contests and evidence is ambiguous, the relay acts as arbiter:

1. **Evidence review**: The relay examines Bus event logs for the disputed period. All feed data, draws, and health events are logged on Bus and recoverable from the ring buffer or graduated Store Signals.
2. **Ruling**: The relay issues a `DisputeVerdict`:
   - `Credit` -- subscriber receives credit applied to their next MPP session (or refund if no future session).
   - `Dismissed` -- dispute rejected; no action taken.
   - `Split` -- partial credit (e.g., stale for 50% of disputed period).
3. **Appeal**: Either party can appeal to on-chain arbitration via ERC-8004 dispute mechanism (Phase 2+). The on-chain record is immutable.

```rust
pub struct Dispute {
    pub id: DisputeId,
    pub session_id: SessionId,
    pub subscriber: AgentId,
    pub producer: AgentId,
    pub reason: DisputeReason,
    pub evidence: Vec<SignalRef>,     // references to Bus events or Store Signals
    pub status: DisputeStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

pub enum DisputeReason {
    WrongData { expected: String, actual: String },
    StaleData { last_update: DateTime<Utc>, expected_interval: Duration },
    ServiceInterruption { start: DateTime<Utc>, end: DateTime<Utc> },
    Overcharge { charged: u64, expected: u64 },
}

pub enum DisputeStatus {
    Open,
    ProducerNotified { notified_at: DateTime<Utc> },
    Contested { producer_evidence: Vec<SignalRef> },
    Resolved { verdict: DisputeVerdict },
}

pub enum DisputeVerdict {
    Credit { amount: u64, applied_to: Option<SessionId> },
    Dismissed { reason: String },
    Split { credit_amount: u64, credit_ratio: f64 },
}
```

### 7.4 Credit Application

Credits from resolved disputes are applied to the subscriber's next MPP session with the same producer. If the subscriber opens a new session, the credit is automatically deducted from the required funding amount. Credits expire after 30 days if unused.

```rust
pub struct DisputeCredit {
    pub subscriber: AgentId,
    pub producer: AgentId,
    pub amount: u64,
    pub dispute_id: DisputeId,
    pub expires_at: DateTime<Utc>,  // created_at + 30 days
    pub applied: bool,
}
```

### 7.5 Reputation Impact

Dispute outcomes affect TraceRank reputation scores:

| Outcome | Producer Impact | Subscriber Impact |
|---|---|---|
| Credit (subscriber wins) | TraceRank -0.02 | No change |
| Dismissed (producer wins) | No change | TraceRank -0.01 (discourage frivolous disputes) |
| Split | TraceRank -0.01 | No change |
| 3+ disputes in 7 days | Relay warning; feed flagged in discovery | -- |
| 10+ upheld disputes in 30 days | Feed suspended from relay | -- |

---

## 8. Feed Registration and Discovery

### 7.1 Feed Registration (Agent -> Relay on Boot)

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

### 7.2 Feed Discovery (Dashboard or Agent -> Relay)

```
GET /relay/feeds                              # all feeds
GET /relay/feeds?kind=derived&access=paid     # filter by kind and access
GET /relay/feeds?agent_id=gas-oracle          # feeds from a specific agent
GET /relay/feeds/{feed_id}                    # single feed metadata
GET /relay/feeds/{feed_id}/sample             # sample payload (free, no auth)
```

### 7.3 Feed Subscription with Payment

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

The relay verifies the MPP session with the feed producer's agent, then forwards feed data to the subscriber.

---

## 9. Feed Marketplace Economics

### 8.1 Feed Types and Composability

Feeds compose into value chains. Each layer adds computation and charges for it.

**Raw feeds** -- direct data ingestion:
- Blockchain: `eth-mainnet-blocks`, `base-swaps`, `arb-gas` (from RPC WebSocket)
- Research: `arxiv-new-papers`, `github-trending` (from web polling)
- Code: `repo-commit-stream`, `ci-build-results` (from webhooks)
- Market: `binance-funding-rates`, `coingecko-prices` (from exchange APIs)

**Derived feeds** -- computed from raw:
- Blockchain: `eth-gas-trend`, `funding-rate-divergence`, `mev-probability`
- Research: `paper-relevance-scores`, `topic-cluster-updates`
- Code: `code-quality-trend`, `dependency-risk-index`
- Market: `volatility-regime`, `cross-venue-spread`

**Composite feeds** -- derived from multiple derived feeds:
- `cross-chain-arb-signal` (consumes gas trends + volume + funding rates)
- `research-portfolio-impact` (consumes paper scores + code quality + market sentiment)
- Cost stacks: producer pays for input feeds, charges for output feed

**Meta feeds** -- feeds about feeds:
- `feed-health` (monitors all feeds for staleness, drift, anomalies)
- `feed-accuracy` (tracks prediction accuracy of derived feeds over time)

### 8.2 Composition Example: Value Chain

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

Each agent in the chain pays for its inputs and charges for its output.

### 8.3 Practical Example: Funding Rate Divergence

An agent that consumes two paid feeds and produces a third:

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

The extension that computes the feed data. Feed data is published via Bus (not `ctx.cortical`) -- this eliminates hidden channels and aligns with the universal transport rule (see [00-INDEX](00-INDEX.md): "Everything through Bus or Store").

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

        // Publish feed data via Bus (not cortical -- no hidden channels)
        ctx.bus().publish(Pulse {
            topic: "feed:funding-divergence:data".into(),
            payload: json!({
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
            }),
            seq: ctx.next_seq(),
        }).await?;

        // Extreme divergence triggers T2 reasoning via prediction error Bus event
        if zscore.abs() > 3.0 {
            ctx.bus().publish(Pulse {
                topic: format!("agent:{}:prediction_error", ctx.agent_id()),
                payload: json!({ "prediction_error": 0.9, "source": "funding-divergence" }),
                seq: ctx.next_seq(),
            }).await?;
        }

        Ok(())
    }
}
```

> **Design note**: Previous versions used `ctx.cortical.set_feed_data()` which is a hidden side-channel. Feed data MUST flow through Bus (ephemeral) or Store (durable). The `FeedPublisherExt` (Social layer) subscribes to `feed:{id}:data` Bus topics and forwards to the relay. This maintains the invariant: two fabrics, no exceptions.

**Economics for `funding-arb`**: $0.20/hr revenue per subscriber minus $0.10/hr input cost. With 5 subscribers: ($0.20 * 5) - $0.10 = $0.90/hr pure margin.

---

## 10. Setting Up a Paid Feed

### 9.1 Declare the Feed in Agent Manifest

```toml
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
base_price_usdc_per_hour = 50
```

When the agent boots, `FeedPublisherExt` reads these declarations and registers them with the relay.

### 9.2 The FeedPublisherExt Extension

Auto-loaded when `[agent.feeds.*]` entries exist. Handles the full lifecycle: register on boot, publish on each tick, deregister on shutdown.

```rust
pub struct FeedPublisherExt {
    feeds: Vec<FeedConfig>,
    relay: RelayHandle,
}

#[async_trait]
impl Extension for FeedPublisherExt {
    fn name(&self) -> &str { "feed-publisher" }
    fn layer(&self) -> ExtensionLayer { ExtensionLayer::Social }

    async fn on_boot(&mut self, ctx: &mut AgentContext) -> Result<()> {
        for feed in &self.feeds {
            ctx.relay.register_feed(FeedRegistration {
                feed_id: feed.id.clone(),
                agent_id: ctx.agent_id.clone(),
                kind: feed.kind,
                schema: feed.schema.clone(),
                rate_hz: feed.rate_hz,
                access: feed.access.clone(),
                sample: feed.sample.clone(),
            }).await?;
        }
        Ok(())
    }

    async fn on_tick_end(&mut self, ctx: &mut AgentContext) -> Result<()> {
        // Subscribe to each feed's Bus topic and forward to relay
        for feed in &self.feeds {
            if let Some(pulse) = ctx.bus().latest(&format!("feed:{}:data", feed.id)).await {
                ctx.relay.publish_feed_data(&feed.id, pulse.payload).await?;
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

### 9.3 Compute Feed Data (Cognition Layer)

Feed data is published via Bus, not `ctx.cortical` -- see design note in SS8.3.

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
        // Read gas price from the raw feed Bus topic
        let gas = ctx.bus().latest("feed:eth-mainnet-blocks:data").await
            .and_then(|p| p.payload["gas_gwei"].as_f64())
            .unwrap_or(0.0);
        self.window.push_back(gas);
        if self.window.len() > 100 { self.window.pop_front(); }

        let alpha = 2.0 / 13.0;
        self.ema = alpha * gas + (1.0 - alpha) * self.ema;

        let mut sorted: Vec<f64> = self.window.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p25 = sorted[sorted.len() / 4];
        let p75 = sorted[3 * sorted.len() / 4];
        let p95 = sorted[19 * sorted.len() / 20];
        let mev_spike = gas > p95 * 2.0;

        // Publish derived feed data via Bus
        ctx.bus().publish(Pulse {
            topic: "feed:eth-gas-trend:data".into(),
            payload: json!({
                "ema_12": self.ema,
                "p25": p25,
                "p75": p75,
                "p95": p95,
                "mev_spike": mev_spike,
                "current": gas,
                "ts": now_ms(),
            }),
            seq: ctx.next_seq(),
        }).await?;

        Ok(())
    }
}
```

Pipeline order: `GasTrendExt` (Cognition layer) runs during `on_observe`, publishes data to the `feed:eth-gas-trend:data` Bus topic. Then `FeedPublisherExt` (Social layer) runs during `on_tick_end`, reads the latest Pulse from that Bus topic and forwards to the relay. Extension layers execute in order: Perception -> Cognition -> Social. All data flows through Bus -- no hidden channels.

---

## 11. Dashboard Subscription (TypeScript)

```typescript
// 1. Discover available feeds
const feeds = await fetch(`${relayUrl}/relay/feeds`).then(r => r.json());
const gasFeed = feeds.find(f => f.feed_id === "eth-gas-trend");

// 2. Open an MPP session (one-time ERC-3009 signature)
const session = await openMppSession(relayUrl, {
  amount: 500,  // $0.0005 USDC -- enough for ~10 hours at $0.05/hr
  recipient: gasFeed.agent_wallet,
});

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
    updateGasChart(msg.payload);
  }
  if (msg.type === "payment_draw") {
    updateBalance(msg.payload);
  }
};
```

---

## 12. Agent-to-Agent Feed Subscription (Rust)

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
                    // Publish prediction error via Bus (not cortical)
                    ctx.bus().publish(Pulse {
                        topic: format!("agent:{}:prediction_error", ctx.agent_id()),
                        payload: json!({ "prediction_error": 0.8, "source": "eth-gas-trend" }),
                        seq: ctx.next_seq(),
                    }).await?;
                }
            }
        }
        Ok(())
    }
}
```

---

## 13. On-Chain Feed Advertisement (ERC-8004)

Agents with wallets advertise their feeds in their ERC-8004 passport. This makes feeds discoverable on-chain even when the agent or relay is offline.

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

When an agent boots with feeds configured, it:

1. Registers feeds with the relay (for live presence and subscription routing).
2. Updates its ERC-8004 passport with feed advertisements (for persistent discovery).
3. On feed config changes (add/remove/reprice), updates both relay and chain.

```rust
// In FeedPublisherExt::on_boot()
async fn on_boot(&mut self, ctx: &mut AgentContext) -> Result<()> {
    for feed in &self.feeds {
        ctx.relay.register_feed(/* ... */).await?;

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

Feed discovery uses both sources:

```typescript
async function discoverFeeds(): Promise<Feed[]> {
  const [relayFeeds, chainFeeds] = await Promise.all([
    fetch(`${relayUrl}/relay/feeds`).then(r => r.json()),
    chainClient.getRegisteredFeeds(),
  ]);

  return mergeFeeds(relayFeeds, chainFeeds);
  // Result: each feed has { ...chainAdvert, live: boolean, subscribers: number }
}
```

An agent's feeds appear in its passport even when the agent is offline.

---

## 14. Dashboard Integration

### 13.1 Feeds Page

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
+--------------------------------------------------------------+
```

### 13.2 My Subscriptions

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

### 13.3 Feed Detail Page

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

### 13.4 Feed Revenue

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

### 13.5 Dashboard Data Sources

| Section | WS rooms | Event types | REST fallback |
|---|---|---|---|
| Fleet / Feeds | `system` | `feed_registered`, `feed_deregistered`, `feed_status` | `GET /relay/feeds` |
| Fleet / Feed detail | `feed:{id}` | `feed_data`, `feed_status`, `payment_draw` | `GET /relay/feeds/{id}` |
| Treasury / Subscriptions | `system` | `session_opened`, `session_exhausted`, `session_settled` | `GET /mpp/sessions` |
| Treasury / Feed Revenue | `system` | `feed_revenue_update`, `settlement_batch` | `GET /relay/feeds/revenue` |

---

## 15. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| P-1 | x402: 402 response includes amount, recipient, nonce, expiry | Unit test |
| P-2 | x402: ERC-3009 signature verified via ecrecover (no RPC) | Unit test |
| P-3 | x402: batch settlement fires at 100 accumulated authorizations | Integration test |
| P-4 | x402: batch settlement fires at 10-minute interval | Integration test with mocked clock |
| P-5 | MPP: session created with funded balance | Integration test |
| P-6 | MPP: per-message draw decrements balance | Unit test |
| P-7 | MPP: exhausted session pauses delivery and sends notice | Integration test |
| P-8 | MPP: top-up resumes delivery | Integration test |
| P-9 | MPP: expired session transitions to settled with refund | Integration test |
| P-10 | MPP: session stays open on disconnect (reusable) | Integration test |
| P-11 | Reputation tier correctly applies markup to base price | Unit test for all 5 tiers |
| P-12 | Producer receives base price regardless of subscriber tier | Unit test |
| P-13 | Relay receives markup as infrastructure fee | Unit test |
| P-14 | Feed registration stores metadata and makes feed discoverable | Integration test |
| P-15 | Feed sample accessible without auth or payment | Integration test |
| P-16 | Feed subscription with MPP session receives data | Integration test |
| P-17 | Feed subscription with x402 returns single data point | Integration test |
| P-18 | Composite feed: agent pays for input feeds and charges for output | Integration test |
| P-19 | ERC-8004 feed advert updated on agent boot | Integration test with mock chain |
| P-20 | Feed discovery merges relay (live) and chain (persistent) sources | Integration test |
| P-21 | Dashboard subscription manager shows session balance | E2E test |
| P-22 | Dashboard feed revenue shows settlement status | E2E test |
| P-23 | VerifyX402Cell implements Cell + VerifyProtocol | Unit test |
| P-24 | VerifyX402Cell rejects requests without X-Payment header with 402 terms | Unit test |
| P-25 | VerifyX402Cell verifies ERC-3009 signature via ecrecover | Unit test |
| P-26 | VerifyX402Cell collects authorization for batch settlement | Integration test |
| P-27 | VerifyMppCell implements Cell + VerifyProtocol | Unit test |
| P-28 | VerifyMppCell rejects requests with exhausted session | Unit test |
| P-29 | VerifyMppCell deducts draw from session balance and transitions to Exhausted at zero | Unit test |
| P-30 | VerifyMppCell publishes exhaustion Pulse on Bus | Integration test |
| P-31 | Dispute creation freezes draws on the disputed session | Integration test |
| P-32 | Dispute auto-resolves in subscriber favor if producer does not respond in 72h | Integration test with mocked clock |
| P-33 | Dispute credit applied to subscriber's next MPP session with same producer | Integration test |
| P-34 | Dispute credits expire after 30 days if unused | Unit test |
| P-35 | 10+ upheld disputes in 30 days suspends producer feed from relay | Integration test |
| P-36 | Feed data published via Bus (not cortical) -- no hidden channels | Unit test: verify Bus Pulse on `feed:{id}:data` topic |
| P-37 | FeedPublisherExt reads from Bus topic (not cortical) for relay forwarding | Unit test |

---

## 16. Cross-References

| Topic | Document | Section |
|---|---|---|
| Signal, Pulse | [doc-01](01-SIGNAL.md) | SS1-3 |
| Verify protocol | [doc-02](02-CELL.md) | SS3.3 |
| Bus (ephemeral transport) | [doc-01](01-SIGNAL.md) | SS2 |
| Relay, feed infrastructure | [doc-11](11-CONNECTIVITY.md) | SS3 |
| Agent bearer tokens | [doc-17](17-AUTH.md) | SS4 |
| ERC-8004 passport | [doc-22](22-REGISTRIES.md) | SS2 |
| Reputation tiers | [doc-22](22-REGISTRIES.md) | SS3 |
| Extension layers (Perception/Cognition/Social) | [doc-12](12-EXTENSIONS.md) | SS3 |
| Feed Cell (Connect+Trigger+Store) | [doc-09](09-FEEDS.md) | SS1-4 |
| Anti-pattern: hidden channels | [doc-00](00-INDEX.md) | Anti-Principles |
