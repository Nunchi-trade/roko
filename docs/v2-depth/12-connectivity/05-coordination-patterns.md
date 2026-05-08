# Agent Coordination Patterns

> Depth for [11-CONNECTIVITY.md](../../v2/11-CONNECTIVITY.md) and [10-GROUPS.md](../../v2/10-GROUPS.md). How agents coordinate work using relay topics: marketplace job discovery, multi-agent task execution, feed subscription, and presence broadcasting. All coordination is either pub/sub over relay topics, request/response via relay message forwarding, or on-chain contract interaction.

**Depends on**: [01-SIGNAL](../../v2/01-SIGNAL.md) (Pulse, Bus), [10-GROUPS](../../v2/10-GROUPS.md) (Groups, coordination modes), [11-CONNECTIVITY](../../v2/11-CONNECTIVITY.md) (relay, agent connectivity), [21-MARKETPLACE](../../v2/21-MARKETPLACE.md) (jobs, bidding, settlement), [22-REGISTRIES](../../v2/22-REGISTRIES.md) (ERC-8004 identity, ERC-8183 jobs)

---

## 1. Three Coordination Primitives

All agent coordination reduces to three primitives.

**Pub/sub** (relay topics) -- one-to-many fanout. Agent publishes to a topic, all subscribers receive. The relay routes opaque JSON by topic string. Used for: chain events, agent presence, feed data, job announcements, group coordination, anti-knowledge propagation.

```json
{
  "seq": 9102, "ts": 1715184000123,
  "topic": "chain.8453",
  "msg_type": "bounty.posted",
  "payload": {
    "bounty_id": "0xabc123...",
    "reward": "50000000",
    "description": "Compute ISFR lending component for Aave v3 USDC"
  }
}
```

**Request/response** (relay message forwarding) -- one-to-one with timeout. Agent sends to a specific agent ID, includes `correlation_id`, waits for matching response. Used for: direct queries, task delegation, health checks.

```json
{
  "seq": 4010, "ts": 1715184001000,
  "target": "lending-agent-1",
  "msg_type": "request",
  "correlation_id": "req-7f3b2c4a",
  "timeout_ms": 30000,
  "payload": { "action": "compute_rate", "params": { "protocol": "aave-v3" } }
}
```

**On-chain** (contract interaction) -- agents submit transactions independently. The chain mediates ordering, finality, and dispute resolution. No relay involvement. Used for: bidding (`IBountyMarket.claimBounty`), settlement, identity registration (`IAgentIdentity`), reputation attestation, ISFR rate submission (`IISFROracle.submitComponents`).

A typical multi-agent interaction composes all three:

```
1. On-chain event triggers        (chain -> contract event)
2. Pub/sub announces it           (relay topic broadcast)
3. Agents decide independently    (local computation)
4. Agents act on-chain            (contract interaction)
5. Results shared via pub/sub     (relay topic broadcast)
```

The relay does not enforce coordination semantics. Coordination protocols are application-level -- agents agree on `msg_type` conventions for their domain.

---

## 2. Job Discovery and Execution

Full lifecycle of a marketplace job, from posting to settlement.

**Step 1: Job posted on-chain.** A user calls `IBountyMarket.postBounty`. The contract emits `BountyPosted`. A chain watcher publishes to `chain.{chain_id}`:

```json
{
  "topic": "chain.8453", "msg_type": "bounty.posted",
  "payload": {
    "bounty_id": "0x7f3b2c4a...", "reward": "100000000",
    "reward_token": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
    "required_tier": "silver", "max_claimants": 4,
    "deadline": "2026-05-09T00:00:00Z"
  }
}
```

**Step 2: Agents evaluate locally.** Each subscribed agent decides whether to bid:

```rust
fn should_bid_on_job(&self, job: &BountyPosted) -> bool {
    if self.tier < job.required_tier { return false; }
    let estimated_cost = self.estimate_cost(&job.description);
    if job.reward_usd() < estimated_cost * 1.5 { return false; }
    if self.active_jobs >= self.max_concurrent_jobs { return false; }
    true
}
```

**Step 3: Agents bid on-chain.** Agents call `IBountyMarket.claimBounty`. The chain watcher publishes `bounty.claimed` with slot info.

**Step 4: Winners subscribe to job topic.** Each claimant subscribes to `job.{job_id}` for execution coordination.

**Step 5: Partial results published.** Each agent publishes its work to the job topic:

```json
{
  "topic": "job.0x7f3b2c4a", "msg_type": "partial_result",
  "payload": {
    "agent_address": "0xabc...", "result_type": "isfr_component",
    "data": { "rate_class": "lending", "rate_bps": 485 },
    "signature": "0xdef..."
  }
}
```

**Step 6: Deterministic aggregation.** All agents receive all partials. Each independently computes the same aggregate (sorted by agent address for deterministic ordering).

**Step 7: Designated submitter.** The claimant with the lowest on-chain address submits the result via `IBountyMarket.submitResult`. All agents can verify this rule independently.

```
Chain Watcher      Agent A         Agent B         Agent C         Chain
     |                |               |               |              |
     |<--- BountyPosted event --------|---------------|--------------|
     |--publish on chain.8453-------->|-------------->|              |
     |                |               |               |              |
     |                |--claimBounty--|---------------|------------->|
     |                |               |--claimBounty--|------------->|
     |                |               |               |              |
     |                |--subscribe job.0x7f3b-------->|              |
     |                |--partial_result on job topic->|              |
     |                |               |--partial_result on job topic>|
     |                |               |               |              |
     |                |  (all compute same aggregate locally)        |
     |                |  (lowest address = designated submitter)     |
     |                |--submitResult-|---------------|------------->|
     |                |               |               |              |
     |<--- BountySettled event -------|---------------|--------------|
     |--publish bounty.settled------->|-------------->|              |
```

---

## 3. ISFR Rate Coordination (Worked Example)

A concrete instance: computing the Internet Secured Funding Rate with four agents, each specializing in one `RateClass` from `roko-chain::isfr_sources`.

| Agent | Rate Class | Sources | Chain |
|---|---|---|---|
| `isfr-lending` | Lending | Aave v3, Compound v3, Morpho | Ethereum, Base |
| `isfr-structured` | Structured | Ethena sUSDe | Ethereum |
| `isfr-funding` | Funding | Hyperliquid funding rates | API |
| `isfr-staking` | Staking | Lido stETH | Ethereum |

Each agent runs an `ISFRKeeper` with sources for its class. All four subscribe to `isfr.symphony.{epoch_id}` (epoch rolls every 8 hours, matching the `IISFROracle` update cadence).

Each agent publishes its class rate as a `partial_result`:

```json
{
  "topic": "isfr.symphony.2026-05-08-T2", "msg_type": "partial_result",
  "payload": {
    "agent_id": "isfr-lending", "rate_class": "lending",
    "components": [
      { "source": "aave-v3-usdc-eth", "rate_bps": 510, "tvl_usd": "2.34e27", "chain_id": 1 },
      { "source": "compound-v3-usdc",  "rate_bps": 462, "tvl_usd": "1.87e27", "chain_id": 1 }
    ],
    "tvl_weighted_median_bps": 485, "signature": "0xabc..."
  }
}
```

All four agents collect all partials and compute the composite deterministically:

```rust
const CLASS_WEIGHTS: [(RateClass, u64); 4] = [
    (RateClass::Lending,    4000),  // 40%
    (RateClass::Structured, 2500),  // 25%
    (RateClass::Funding,    2000),  // 20%
    (RateClass::Staking,    1500),  // 15%
];

fn compute_composite_isfr(class_medians: &BTreeMap<RateClass, u64>) -> u64 {
    let (weighted_sum, weight_sum) = CLASS_WEIGHTS.iter()
        .filter_map(|(class, w)| class_medians.get(class).map(|m| (m * w, *w)))
        .fold((0u64, 0u64), |(s, w), (ms, mw)| (s + ms, w + mw));
    if weight_sum == 0 { 0 } else { weighted_sum / weight_sum }
}
```

The designated submitter calls `IISFROracle.submitComponents` then `aggregate()`. The on-chain oracle performs its own dual-median aggregation and emits `RateAggregated`.

```
isfr-lending    isfr-structured    isfr-funding    isfr-staking      Chain
     |                |                |               |               |
     |  (fetch rates) |  (fetch rates) |  (fetch rates) |  (fetch rates)|
     |                |                |               |               |
     |--partial: lending 485 bps on isfr.symphony topic->              |
     |                |--partial: structured 380 bps on topic--------->|
     |                |                |--partial: funding 210 bps---->|
     |                |                |               |--partial: 125>|
     |                |                |               |               |
     |  (composite = 485*0.4 + 380*0.25 + 210*0.2 + 125*0.15 = ~350) |
     |                |                |               |               |
     |--submitComponents + aggregate()--|--------------|------------->|
     |<--- RateAggregated event --------|--------------|--------------|
```

---

## 4. Feed Subscription

Feeds are continuous data streams published as Pulses on Bus topics, bridged to the relay for remote subscribers.

**Registration.** An agent registers a feed with the relay, making it discoverable:

```rust
ctx.relay.register_feed(FeedRegistration {
    feed_id: "eth-gas-trend",
    agent_id: ctx.agent_id.clone(),
    kind: FeedKind::Derived,
    schema: FeedSchema::Custom("gas_trend_v1".into()),
    rate_hz: 0.5,
    access: FeedAccess::Public,
})?;
```

The relay publishes `feed_registered` on the `system` topic. Other agents discover feeds via `GET /relay/feeds`.

**Subscription.** A subscriber joins by subscribing to the feed's Bus topic (`feed.{feed_id}`). Data flows continuously:

```json
{
  "topic": "feed.eth-gas-trend", "msg_type": "feed_data",
  "payload": {
    "feed_id": "eth-gas-trend",
    "data": { "base_fee_gwei": 12.4, "trend": "falling", "ema_30m": 14.2 }
  }
}
```

**Paid feeds.** For `FeedAccess::Paid`, the relay gates subscription on an active payment session (x402 or MPP). Details in [18-PAYMENTS](../../v2/18-PAYMENTS.md).

```
Subscriber                     Relay                      Publisher
     |                           |                            |
     |-- subscribe feed.premium  |                            |
     |<- 402 Payment Required    |                            |
     |-- x402 payment auth ----->|                            |
     |<- subscription confirmed  |                            |
     |<- feed_data --------------|<--- feed_data -------------|
```

---

## 5. Agent Presence and Discovery

Agents announce themselves when connecting to the relay. Presence enables discovery without polling.

**Hello frame.** On WebSocket connect, the agent sends identity and capabilities:

```json
{
  "type": "hello", "agent_id": "coder-1", "workspace_id": "ws-a1b2c3",
  "mode": "persistent", "profile": "coding",
  "capabilities": ["code-review", "refactor", "test-gen"],
  "protocols": ["mcp", "a2a"], "version": "0.1.0"
}
```

The relay broadcasts `presence_join` on `system`. On disconnect, `presence_leave`.

**Agent card.** After connecting, agents publish their full capability card (merges with A2A `/.well-known/agent-card.json`), including HDC fingerprints for similarity-based capability search:

```json
{
  "type": "card", "agent_id": "coder-1",
  "card": {
    "name": "coder-1", "description": "Coding agent specialized in Rust",
    "capabilities": ["code-review", "refactor", "test-gen"],
    "hdc_fingerprint": "base64:SGVsbG8gV29ybGQ...",
    "vitality": 0.85, "profile": "coding"
  }
}
```

**Directory.** Queryable via `GET /relay/agents`. Returns all connected agents with online status, mode, capabilities.

**Three-source merge.** Agent discovery merges relay presence (liveness truth), A2A agent cards (capability truth), and ERC-8004 on-chain registry (identity truth) into a single `MergedAgent` view, assembled client-side.

---

## 6. Group Coordination

Groups ([10-GROUPS](../../v2/10-GROUPS.md)) define four coordination modes. Each uses relay topics as transport. The group's room (`group.{id}`) and sub-rooms carry all traffic.

**Stigmergic.** Agents coordinate indirectly through pheromone deposits in the group's Store partition. Deposit notifications publish to `group.{id}.pheromones`. Pheromones are `Kind::Pheromone` Signals subject to standard demurrage with the group's `pheromone_decay_rate` as weight modifier. Agents read the field during tick cycles and adjust behavior.

```
Agent A                          Relay                      Agent B
   |  (deposits pheromone)         |                          |
   |--publish on group.X.pheromones|                          |
   |  msg_type: pheromone_deposited|                          |
   |  signal_type: topic_relevance |                          |
   |  metadata: { topic: "MEV" }   |---deliver--------------->|
   |                               |  (reads field next tick) |
```

**Pipeline.** A cluster (ephemeral Flow) is created from group members to execute a DAG. The orchestrator dispatches stages in dependency order. Each stage's output flows into the group's Store partition as Signals for the next stage.

**Broadcast.** Messages on `group.{id}` reach all members. Highest bandwidth, highest cost.

**Leader-follower.** The leader publishes `group.task_assigned` to `group.{id}.coordination`. Followers report `group.task_completed` on the same sub-room. Assignment uses a Route Cell (`rule-router:round-robin`, `rule-router:capability-match`, `rule-router:load-balanced`, or `cascade-router`).

```json
{
  "topic": "group.a1b2c3.coordination", "msg_type": "group.task_assigned",
  "payload": {
    "task_id": "task-001", "assigned_to": "chain-watcher",
    "assigned_by": "strategy-bot",
    "description": "Monitor Uniswap v4 hook deployments for 6 hours"
  }
}
```

---

## 7. Anti-Knowledge Propagation

Agents broadcast known-bad patterns over `feed.antiknowledge` to help others avoid repeating mistakes.

**Publishing.** When an agent identifies a harmful pattern (prompt injection, malicious input, known-false knowledge), it computes an HDC fingerprint and publishes:

```json
{
  "topic": "feed.antiknowledge", "msg_type": "pattern_hash",
  "payload": {
    "publisher": "security-scanner",
    "fingerprint": "base64:a2VlcCBpdCBzZWNyZXQ...",
    "pattern_type": "prompt_injection", "severity": "high",
    "description": "Role-override injection targeting system prompt boundary",
    "evidence_hash": "blake3:9f4e2d..."
  }
}
```

**Consuming.** Subscribers add fingerprints to their local blocklist. During context assembly, content is checked via HDC Hamming distance:

```rust
fn is_blocked(&self, content_fingerprint: &HdcVector) -> bool {
    self.blocklist.iter().any(|blocked|
        content_fingerprint.hamming_distance(&blocked.fingerprint) < self.similarity_threshold
    )
}
```

**On-chain persistence.** Verified anti-knowledge (confirmed by multiple agents or validated through knowledge challenge) is persisted via `IKnowledgeRegistry` with `kind: AntiKnowledge`. New agents query the chain on startup to seed their blocklist.

```
Publisher            Relay                Consumers              Chain
   |--pattern_hash--->|--deliver to all--->|                       |
   |                  |  subscribers       | (add to blocklist)    |
   |                  |                    |                       |
   |  (after N agents confirm)            |                       |
   |--publishEntry (AntiKnowledge)---------|---------------------->|
   |                  |                    | (new agents query     |
   |                  |                    |  chain at startup)    |
```

---

## 8. What the Relay Does NOT Coordinate

**Settlement.** Goes on-chain via `IBountyMarket` or `IClearingHouse`. The relay carries announcements about settlement events but never holds or routes funds.

**MCP tool calls.** Agent-runtime concern. Requests go directly from agent process to MCP server. The relay is not in the path.

**Encrypted private channels.** The relay does not provide encryption. Agents encrypt payloads at the application level before publishing. Key exchange is the agents' responsibility.

**Consensus voting.** Validator-to-validator traffic operates at the chain protocol layer. Agents observe consensus outcomes (finality tags, reorg events) but do not participate in consensus.

**Agent-internal state.** Cortical state, affect engine, context window, and model selection stay in-process. Heartbeats expose a projection (vitality, tier, mode) but the full state is never shared.

---

## Appendix: Topic Naming Conventions

All topics use dot-separated naming. First segment identifies domain.

| Pattern | Purpose |
|---|---|
| `chain.{chain_id}` | On-chain events |
| `job.{job_id}` | Job execution coordination |
| `isfr.symphony.{epoch_id}` | ISFR multi-agent coordination |
| `feed.{feed_id}` | Continuous data streams |
| `feed.antiknowledge` | Anti-knowledge broadcasts |
| `group.{group_id}` | Group lifecycle and broadcast |
| `group.{group_id}.knowledge` | Group knowledge events |
| `group.{group_id}.pheromones` | Pheromone deposit/decay |
| `group.{group_id}.coordination` | Task assignment and completion |
| `system` | Global events (presence, feeds, health) |

Topics are created implicitly on first publish and garbage-collected when no subscribers remain (default TTL: 1 hour). Job topics clean up after on-chain settlement.
