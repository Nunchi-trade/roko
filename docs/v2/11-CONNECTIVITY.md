# 11 — Connectivity

> Relay wire protocol, exoskeleton protocol bindings (MCP, A2A, ERC-8004, x402), in-process vs remote agents, WebSocket subscription lifecycle, backpressure, multi-chain connectivity, and finality oracle. All transport is subscription-based. No polling.

**Subsumes**: Connector trait, Feed system, relay, workspace discovery, agent connectivity, data flow, reconnection protocol, exoskeleton protocol bindings, multi-chain temporal resolution.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse duality, Bus, demurrage, HDC), [02-CELL](02-CELL.md) (9 protocols, typed I/O, capabilities), [05-AGENT-RUNTIME](05-AGENT.md) (vitality, type-state, CorticalState), [15-TELEMETRY](15-TELEMETRY.md) (StateHub projections, Lenses)

---

## 1. Connect Protocol

The **Connect protocol** is one of the 9 Cell protocols ([doc-02](02-CELL.md)). A Cell implementing Connect manages the full lifecycle of an external system connection: establishment, querying, mutation, health monitoring, and teardown. Five methods, no optional.

```rust
/// The Connect protocol — external system lifecycle.
/// Every Connector Cell implements this trait.
#[async_trait]
pub trait Connect: Cell {
    /// Establish connection to an external system.
    /// Called once at Agent startup or on first use.
    /// Fails closed: if connect() fails, the Connector is unavailable.
    async fn connect(&mut self, config: &ConnectConfig) -> Result<()>;

    /// One-shot read query. Idempotent. Does not mutate external state.
    /// Used for reads: balances, events, contract state, API GETs.
    async fn query(&self, request: QueryRequest) -> Result<QueryResponse>;

    /// Mutating operation. NOT idempotent unless the external system guarantees it.
    /// Used for writes: transactions, API POSTs, webhook deliveries.
    async fn execute(&self, request: ExecuteRequest) -> Result<ExecuteResponse>;

    /// Health check. Called at `health_interval` (default 30s).
    /// Returns current health status with latency and error details.
    async fn health(&self) -> Result<HealthStatus>;

    /// Graceful disconnect. Called at Agent shutdown or on explicit teardown.
    /// Must release all external resources (connections, subscriptions, locks).
    async fn disconnect(&mut self) -> Result<()>;
}
```

---

## 2. ConnectorKind

The `ConnectorKind` enum classifies Connectors by the type of external system they interface with. This classification drives default configuration, health check strategies, and capability requirements.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectorKind {
    /// Blockchain RPC endpoints (Ethereum, Solana, Base, Arbitrum).
    /// Capabilities: Chain { read: true, write: configurable }.
    /// Health: eth_blockNumber or equivalent.
    ChainRpc,

    /// Centralized exchange APIs (Hyperliquid, Binance, etc.).
    /// Capabilities: Net, Secrets (API keys).
    /// Health: GET /api/v1/time or equivalent.
    Exchange,

    /// MCP tool servers — auto-discovered from agent.mcp_config.
    /// Capabilities: inherited from the MCP server's tool declarations.
    /// Health: MCP ping.
    McpServer,

    /// A2A agent-card peers — discovered via /.well-known/agent-card.json.
    /// Capabilities: Net.
    /// Health: A2A heartbeat.
    A2aClient,

    /// Relational databases (Postgres, SQLite, MySQL).
    /// Capabilities: Net (remote) or FsRead (SQLite).
    /// Health: SELECT 1.
    Database,

    /// Outbound HTTP webhook endpoints.
    /// Capabilities: Net { domains: configured }.
    /// Health: HEAD request to endpoint.
    Webhook,

    /// Generic REST / gRPC APIs.
    /// Capabilities: Net { domains: configured }, Secrets.
    /// Health: configurable endpoint.
    Api,
}
```

---

## 3. ConnectorManifest

Every Connector carries a manifest declaring its identity, configuration schema, and operational parameters.

```rust
pub struct ConnectorManifest {
    pub name: String,
    pub kind: ConnectorKind,
    pub version: Version,
    pub description: String,
    pub config_schema: TypeSchema,       // what ConnectConfig expects
    pub capabilities: Vec<Capability>,   // what system resources it needs
    pub health_interval: Duration,       // how often to check health (default: 30s)
    pub reconnect_strategy: ReconnectStrategy,
}

pub enum ReconnectStrategy {
    /// Exponential backoff with jitter.
    ExponentialBackoff { base_ms: u64, max_ms: u64, jitter: bool },
    /// Fixed interval.
    FixedInterval { interval_ms: u64 },
    /// No automatic reconnection.
    Manual,
}
```

### Connector = Cell + Connect + lifecycle

A **Connector** is a Cell specialization ([02-CELL](02-CELL.md)). It implements the Connect protocol and manages a connection lifecycle: connect at Agent startup, health-check periodically, reconnect on failure, disconnect at shutdown. The lifecycle is managed by the Agent runtime ([doc-05](05-AGENT.md)), not by the Connector itself.

---

## 4. Exoskeleton Protocols

Roko does not invent transport, identity, discovery, or payment. Four external protocols form a fixed **exoskeleton** — load-bearing standards the spec builds on, not competes with. The spec defines **what flows through them**.

| Protocol | Role | Status | Governance |
|---|---|---|---|
| **MCP** | Tool and resource discovery | 97M monthly SDK downloads | Linux Foundation |
| **A2A** | Agent-card discovery (`/.well-known/agent-card.json`) | 150+ org support | Linux Foundation |
| **ERC-8004** | On-chain identity, reputation, validation registries | Ethereum mainnet since Jan 29, 2026 | Ethereum EIP process |
| **x402** | Stablecoin agent-to-agent payments | 75M+ transactions | Open standard |

### 4.1 What flows through MCP: Signal/Pulse over tool calls

MCP provides tool discovery and invocation. Roko defines the payload format that rides inside MCP envelopes:

- **Signal/Pulse format over MCP tool calls.** Every MCP tool invocation carries a `CellInput` (Signals + Macros + context) and returns a `CellOutput` (Signals + persist set + metrics). The MCP `tool.call` envelope wraps the Cell's typed I/O. This means any MCP-connected system can invoke Roko Cells natively, and Roko Agents can invoke any MCP-conforming tool.

```rust
/// What flows through MCP tool.call envelopes.
pub struct McpCellPayload {
    /// Input Signals to the Cell.
    pub input: CellInput,
    /// Context: active Macros, Slot fillings, budget remaining.
    pub context: CellContext,
}

pub struct McpCellResponse {
    /// Output Signals from the Cell.
    pub output: CellOutput,
    /// Signals to persist to Store.
    pub persist: Vec<Signal>,
    /// Execution metrics (cost, duration, tokens).
    pub metrics: CellMetrics,
}
```

MCP auto-registration creates an `McpConnector` per configured server. The MCP tool list becomes the Connector's query/execute surface:

```toml
# roko.toml -- MCP servers register as Connectors automatically
[agent.mcp_config]
servers = [
  { name = "github", command = "roko-mcp-github" },
  { name = "code-intel", command = "roko-mcp-code" },
]
```

### 4.2 What flows through A2A: Agent Cards + HDC fingerprints

A2A provides agent discovery via `/.well-known/agent-card.json`. Roko extends the agent card with HDC capability fingerprints ([doc-01](01-SIGNAL.md)):

- **Agent Card with HDC capability fingerprints.** Each Agent publishes its capabilities as an HDC vector (10,240-bit, Kanerva 2009) in the A2A card. This enables similarity-based capability search: "find an agent that can do what Agent X does" reduces to a Hamming distance query over the agent-card registry.

```json
{
  "name": "coder-1",
  "description": "Coding agent specialized in Rust",
  "url": "https://my-roko.up.railway.app",
  "capabilities": ["code-review", "refactor", "test-gen"],
  "hdc_fingerprint": "base64:...",
  "protocols": ["mcp", "a2a"],
  "version": "0.1.0",
  "vitality": 0.85,
  "profile": "coding"
}
```

Agent discovery merges three sources (see section 10): relay presence (liveness), A2A agent cards (capabilities), and ERC-8004 on-chain registry (identity and reputation).

### 4.3 What flows through ERC-8004: ZK-attested HDC into agent identities

ERC-8004 provides on-chain identity and reputation registries ([doc-22](22-REGISTRIES.md)):

- **ZK-attested HDC fingerprints into ERC-8004 agent identities.** An Agent's HDC capability fingerprint is attested via a ZK proof (PP-HDC: proving the fingerprint was computed from declared capabilities without revealing model weights or internal state) and anchored into the Agent's ERC-8004 identity record. Capability claims become cryptographically verifiable without trusting the claimant.

ERC-8004 identity fields consumed by Roko:

| Field | Source | Used by |
|---|---|---|
| `wallet` | Agent registration | Identity, payment routing, x402 settlement |
| `reputation` | TraceRank (accumulated verified work) | CascadeRouter model selection, marketplace trust |
| `hdc_fingerprint` | ZK attestation (PP-HDC) | Capability discovery, coalition formation |
| `stake` | Bonded tokens | Sybil resistance, economic alignment |
| `tier` | Computed from reputation + stake | gray / copper / silver / gold / amber |

### 4.4 What flows through x402: budget-bounded payment intents

x402 provides stablecoin payment between agents:

- **Budget-bounded payment intents over x402.** When an Agent subscribes to a paid Feed, invokes a paid tool, or delegates work to another Agent, the payment is a structured intent bounded by the Agent's vitality budget ([doc-05](05-AGENT.md)). Overspend attempts fail closed.

```rust
pub struct PaymentIntent {
    pub payer: Address,
    pub payee: Address,
    pub max_amount: U256,
    pub denomination: TokenAddress,   // USDC, DAI, etc.
    pub purpose: String,              // "feed:eth-gas-trend", "tool:code-review"
    pub expiry: DateTime<Utc>,
    pub budget_ref: BudgetRef,        // links to Agent's vitality budget
    pub trace_id: Option<TraceId>,    // distributed tracing correlation
}
```

### 4.5 Exoskeleton composition

The four protocols compose. A single cross-agent interaction may touch all four:

1. **A2A** discovers the target Agent (agent-card lookup, HDC similarity search).
2. **ERC-8004** verifies the target's identity and reputation (ZK-attested identity record).
3. **MCP** invokes the target's tool (Signal/Pulse payload via CellInput/CellOutput).
4. **x402** settles payment for the invocation (budget-bounded intent).

The spec does not modify these protocols. It defines the payloads, schemas, and behavioral contracts that flow through them.

---

## 5. Connector Discovery

Connectors are discovered from three sources. No implicit discovery — every Connector is explicitly declared.

| Source | Mechanism | Example |
|---|---|---|
| **Config** | `connectors = [...]` in agent config | `connectors = ["postgres", "hyperliquid"]` |
| **MCP auto-register** | MCP servers in `agent.mcp_config` | MCP server auto-registers as `McpConnector` |
| **Extension-provided** | Extension registers Connectors in `on_init()` | Chain reader Extension registers `ChainRpcConnector` |

### Config-based discovery

```toml
# roko.toml
[[agents]]
name = "chain-watcher"
connectors = [
  { name = "ethereum-rpc", kind = "chain_rpc", url = "wss://eth.llamarpc.com", chain_id = 1 },
  { name = "base-rpc", kind = "chain_rpc", url = "wss://base.llamarpc.com", chain_id = 8453 },
  { name = "postgres", kind = "database", url = "postgres://localhost/roko" },
]

[agent.mcp_config]
servers = [
  { name = "github", command = "roko-mcp-github" },
  { name = "code-intel", command = "roko-mcp-code" },
]
```

### MCP auto-registration

Any MCP server configured in `agent.mcp_config` automatically registers as a `McpConnector`. The MCP tools become available through the Connector's `query()` and `execute()` methods.

### Extension-provided discovery

Extensions can register Connectors during their `on_init()` hook ([doc-12](12-EXTENSIONS.md)):

```rust
impl Extension for ChainReaderExt {
    async fn on_init(&mut self, ctx: &mut AgentContext) -> Result<()> {
        ctx.register_connector(Box::new(EthereumConnector::new(
            &self.config.rpc_url,
        )))?;
        Ok(())
    }
}
```

---

## 6. Distinction: Connector vs Extension

| Aspect | Connector | Extension |
|---|---|---|
| Protocol | Connect (5 methods: connect/query/execute/health/disconnect) | Hook-based interception (22 hooks across 8 layers, [doc-12](12-EXTENSIONS.md)) |
| Purpose | Provide bidirectional I/O with external systems | Modify Agent behavior through pipeline interception |
| Agent relationship | Agent *uses* Connectors | Agent *loads* Extensions |
| Data direction | Bidirectional (Agent <-> external system) | Interceptor (sits in the pipeline) |
| Lifecycle | Independent connection lifecycle (connect/disconnect) | Tied to Agent lifecycle (on_init/on_shutdown) |
| Composition | An Extension can *wrap* a Connector | A Connector cannot intercept an Extension |

An Agent *loads* Extensions but *uses* Connectors. An Extension can *wrap* a Connector to add retry logic, rate limiting, or caching — but the reverse is not possible.

---

## 7. Feed as Pulse Stream on Bus

In the unified vocabulary, a **Feed** is not a separate primitive. A Feed is **Pulses published to a Bus topic by a Connector** ([doc-01](01-SIGNAL.md)). The Feed concept is a naming convention for a common pattern: a Connector continuously producing Pulses on a well-known Bus topic.

### How it works

1. A Connector establishes a connection to an external system (chain RPC, exchange API, webhook).
2. The Connector subscribes to events from the external system.
3. Each event becomes a Pulse published to a Bus topic.
4. Other Cells subscribe to that topic to consume the stream.

```rust
// A ChainRpcConnector publishes block Pulses to the Bus
impl ChainRpcConnector {
    async fn on_new_block(&self, block: EthBlock, bus: &BusHandle) -> Result<()> {
        let pulse = Pulse {
            seq: bus.next_seq(),
            topic: Topic::from(format!("feed:{}-blocks", self.chain_name)),
            kind: Kind::Json,
            body: serde_json::to_value(&block)?,
            emitted_at_ms: Utc::now().timestamp_millis(),
            source: PulseSource::Cell(self.block_ref()),
            lineage_hint: None,
            trace_id: None,
        };
        bus.publish(pulse).await
    }
}
```

### Feed registration

Connectors register their Feed topics for discovery:

```rust
ctx.relay.register_feed(FeedRegistration {
    feed_id: "eth-mainnet-blocks",
    agent_id: ctx.agent_id.clone(),
    kind: FeedKind::Raw,
    schema: FeedSchema::EthBlock,
    rate_hz: 0.08,  // ~1 block per 12s
    access: FeedAccess::Public,
})?;
```

### Feed kinds

| Kind | Description | Example |
|---|---|---|
| **Raw** | Unprocessed data from external system | Ethereum blocks, exchange trades |
| **Derived** | Processed or computed from raw Feeds | Gas trend indicators, MACD signals |

### Feed access

| Access | Description | Payment |
|---|---|---|
| **Public** | Any subscriber can consume for free | None |
| **Paid** | Requires payment per time unit | x402 intent per subscription period |

---

## 8. Workspace Discovery via Relay

Roko instances register with the relay on startup. Dashboards discover available workspaces automatically.

### Registration

When `roko serve` starts, it connects to the relay and announces itself:

```json
{
  "type": "workspace_hello",
  "workspace_id": "ws-a1b2c3",
  "name": "will-dev",
  "url": "https://my-roko.up.railway.app",
  "version": "0.1.0",
  "capabilities": ["agents", "plans", "prds", "learning", "gateway"],
  "owner_wallet": "0x7f3b...2c4a",
  "agents_count": 3,
  "uptime_secs": 3600,
  "exoskeleton": {
    "mcp": true,
    "a2a": true,
    "erc8004_chain_id": 1,
    "x402": true
  }
}
```

The relay also pushes workspace events on the events WebSocket:

```json
{"type": "workspace_connected", "workspace_id": "ws-a1b2c3", "url": "https://..."}
{"type": "workspace_disconnected", "workspace_id": "ws-a1b2c3"}
```

### Discovery flow

The relay maintains a workspace directory. Dashboards query it:

```
GET /relay/workspaces
-> [
    {
      "workspace_id": "ws-a1b2c3",
      "name": "will-dev",
      "url": "https://my-roko.up.railway.app",
      "owner_wallet": "0x7f3b...2c4a",
      "agents_count": 3,
      "online": true,
      "last_seen_ms": 1713960000000
    }
  ]
```

### Dashboard connection flow

1. Dashboard loads, connects to relay.
2. Fetches `GET /relay/workspaces` — lists all online roko instances.
3. If user has a Privy wallet, auto-matches workspaces by `owner_wallet`.
4. If exactly one match: auto-connect (zero friction).
5. If multiple matches: show picker ("You have 2 workspaces online — which one?").
6. If no match: show global-only view (agents from relay, chain data, no workspace features).
7. User can also manually add a workspace URL in Settings.

### Local development

`roko serve` on localhost registers with the relay if `relay.url` is configured. For pure local dev (no relay), the dashboard falls back to `VITE_ROKO_API_URL` env var or `localhost:6677`.

```toml
# roko.toml
[relay]
url = "wss://relay.nunchi.dev"
workspace_name = "will-dev"
```

If `[relay]` is not configured, roko serves HTTP only — no relay registration, no auto-discovery.

---

## 9. Data Flow: Subscription-Only

Every piece of data flows through WebSocket subscriptions. No polling.

### Event sources

| Source | Transport | What it carries |
|---|---|---|
| **Relay** | WS `/relay/ws` | Agent presence, message lifecycle, relay health |
| **roko-serve** | WS `/ws` | Plan progress, gate results, episodes, learning metrics, job updates |
| **Agent (direct)** | WS (per-agent) | Heartbeats, streaming LLM output, decision traces |
| **Agent (via relay)** | WS `/relay/ws` | Same as direct, tunneled through relay |
| **Chain** | WS (RPC sub) | Blocks, contract events, ERC-8004 registry updates, finality tags |

### WebSocket message envelope

Every message uses the same envelope:

```json
{
  "seq": 4821,
  "ts": 1713974400123,
  "room": "agent:coder-1:heartbeat",
  "type": "heartbeat",
  "payload": { }
}
```

| Field | Type | Purpose |
|---|---|---|
| `seq` | `u64` | Monotonic sequence number per connection. Enables reconnection replay. |
| `ts` | `u64` | Unix milliseconds. Server clock. |
| `room` | `string` | Scoping. Clients subscribe to rooms, receive only matching messages. |
| `type` | `string` | Event discriminant. |
| `payload` | `object` | Type-specific data. |

### Room naming convention

```
agent:{id}                  Agent lifecycle events (spawn, stop, error)
agent:{id}:heartbeat        Heartbeat ticks (T0/T1/T2, cortical state)
agent:{id}:output           Streaming LLM output chunks
agent:{id}:trace            Decision traces per tick
agent:{id}:feed:{feed_id}   Pulse streams exposed by the agent
plan:{id}                   Plan progress, task transitions, gate results
cluster:{id}                Cluster pipeline events
chain:{chain_id}            Chain events (blocks, reorgs, finality updates)
system                      Server health, provider status, cost updates
learning                    Experiment results, router updates, thresholds
```

### Event types

| Type | Room pattern | Payload |
|---|---|---|
| `presence_join` | system | `{ agent_id, mode, profile }` |
| `presence_leave` | system | `{ agent_id, reason }` |
| `heartbeat` | agent:{id}:heartbeat | `{ tick, tier, pe, cortical_state }` |
| `output_chunk` | agent:{id}:output | `{ content, done, usage }` |
| `trace` | agent:{id}:trace | `{ tick, steps[], gate_result }` |
| `task_started` | plan:{id} | `{ task_id, phase }` |
| `task_completed` | plan:{id} | `{ task_id, outcome }` |
| `gate_result` | plan:{id} | `{ task_id, gate, rung, passed }` |
| `phase_transition` | plan:{id} | `{ from, to }` |
| `feed_data` | agent:{id}:feed:{fid} | `{ feed_id, data }` |
| `feed_registered` | system | `{ agent_id, feed_id, schema }` |
| `cost_update` | system | `{ agent_id, delta, total }` |
| `provider_status` | system | `{ provider, healthy, latency_ms }` |
| `experiment_result` | learning | `{ experiment_id, winner, p_value }` |
| `router_update` | learning | `{ model, weight, reason }` |
| `chain_reorg` | chain:{chain_id} | `{ old_head, new_head, orphaned_range, depth }` |
| `finality_update` | chain:{chain_id} | `{ block_number, level, confirmations }` |

### WebSocket subscription lifecycle

Clients subscribe when a view mounts and unsubscribe when it unmounts:

```typescript
function useAgentFeed(agentId: string) {
  const [state, setState] = useState<AgentState | null>(null);

  useEffect(() => {
    const ws = new WebSocket(`${relayUrl}/relay/ws`);

    ws.onopen = () => {
      ws.send(JSON.stringify({
        type: "subscribe",
        rooms: [`agent:${agentId}`, `agent:${agentId}:heartbeat`]
      }));
    };

    ws.onmessage = (e) => {
      const event = JSON.parse(e.data);
      setState(prev => applyEvent(prev, event));
    };

    return () => {
      ws.send(JSON.stringify({
        type: "unsubscribe",
        rooms: [`agent:${agentId}`, `agent:${agentId}:heartbeat`]
      }));
      ws.close();
    };
  }, [agentId]);

  return state;
}
```

---

## 10. Agent Connectivity

Agents communicate across users and across machines. The relay is the rendezvous point — any agent connected to the relay can discover and message any other agent, regardless of who owns them. There is no concept of "my agents" vs "their agents" at the protocol level. Ownership and access control are handled by auth (Privy JWT, wallet signatures, API keys), not by network isolation.

### Cross-user communication patterns

- **Direct messaging**: Agent A sends a message to Agent B via relay. B receives it in its inbox, processes it during the next tick, and can respond.
- **Feed subscription**: Agent A subscribes to Agent B's feed (free or paid). Data flows B -> relay -> A continuously.
- **Pheromone signaling**: Agents deposit pheromones on-chain. Any agent can read them — stigmergic coordination without explicit messaging.
- **Cluster collaboration**: Agents from different users can join the same cluster if authorized. The cluster pipeline orchestrates them together.
- **Knowledge sharing**: Agents publish knowledge to the InsightStore (on-chain). Any agent can query it regardless of owner.

### Auth controls

| Action | Auth required |
|---|---|
| Discover agents on relay | None (public) |
| Read agent card / capabilities | None (public) |
| Send message to agent | Privy JWT or API key |
| Subscribe to free feed | None |
| Subscribe to paid feed | MPP session or x402 payment |
| Join a cluster | Cluster owner's invitation token |
| Read on-chain knowledge | None (public chain data) |
| Publish knowledge on-chain | Agent wallet signature |

---

## 11. Agent Topologies

### In-process agents (default)

Agents run as tokio tasks inside the roko process. Communication through channels.

```
+------------------------------------------------------------+
|                      roko process                           |
|                                                             |
|  +-----------+       mpsc         +------------------+      |
|  | Control   | <----------------> | AgentRuntime     |      |
|  | Plane     |       mpsc         | "coder-1"        |      |
|  |           |                    |                   |      |
|  | Routes    |       mpsc         +------------------+      |
|  | msgs via  | <----------------> | AgentRuntime     |      |
|  | channel   |       mpsc         | "research"       |      |
|  | map       |                    +------------------+      |
|  +-----------+                                              |
|                                                             |
|  +--------------------------------------------------------+ |
|  | Inference Gateway (shared by all in-process agents)     | |
|  +--------------------------------------------------------+ |
+------------------------------------------------------------+
```

Benefits: zero serialization overhead, shared inference gateway, shared memory structures, no network latency.

### Remote agents

For isolation or NAT traversal, Agents connect OUTBOUND to the relay. No inbound server required.

```bash
# On the Fly Machine / Railway container
roko agent run \
  --name "isolated-coder" \
  --relay wss://relay.nunchi.dev \
  --inference-proxy https://my-roko.up.railway.app/api/inference \
  --auth-token $AGENT_TOKEN
```

The Agent:
1. Connects to relay WebSocket.
2. Announces presence with Agent ID and capabilities (published as A2A agent card).
3. Enters the standard `run()` loop.
4. Routes inference requests to the parent's gateway via HTTPS proxy.
5. Publishes heartbeats and events through the relay.

### Direct-reachable agents

Remote Agents with public URLs (Railway services, dedicated VMs) can receive messages directly via HTTP in addition to the relay path.

```toml
# roko.toml
[[remote_agents]]
name = "staging-monitor"
url = "https://staging-monitor.fly.dev"
auth_token_ref = "secrets.staging_monitor_token"
```

The control plane prefers direct HTTP for request-response patterns (lower latency) and uses the relay for event streaming and presence.

---

## 12. Message Routing

The control plane routes messages based on Agent location, with a three-tier fallback:

```rust
impl ControlPlane {
    async fn send_to_agent(
        &self,
        agent_id: &AgentId,
        msg: AgentMessage,
    ) -> Result<()> {
        // 1. In-process agents first (fastest path -- channel send)
        if let Some(sender) = self.local_agents.get(agent_id) {
            return sender.send(msg).await.map_err(Into::into);
        }

        // 2. Direct-reachable agents (HTTP -- low latency)
        if let Some(url) = self.deployment_urls.get(agent_id) {
            return self.http_client
                .post(format!("{url}/api/message"))
                .json(&msg)
                .send()
                .await
                .map_err(Into::into);
        }

        // 3. Fall back to relay (works for NAT traversal)
        self.relay.send(agent_id, msg).await
    }
}
```

### Routing priority

| Priority | Path | Latency | Use case |
|---|---|---|---|
| 1 | In-process (mpsc channel) | ~0 | Default local Agents |
| 2 | Direct HTTP | ~10-50ms | Remote Agents with public URLs |
| 3 | Relay-forwarded | ~50-200ms | NAT-traversed Agents, no public URL |

---

## 13. Agent Discovery: Three Sources Merged

Agents are discovered from three sources, merged client-side:

| Source | Provides | Truth claim |
|---|---|---|
| **Relay presence** | Who is online right now | Liveness truth |
| **A2A agent cards** | Capabilities, HDC fingerprint, protocol support | Capability truth |
| **ERC-8004 on-chain registry** | Wallet, reputation, stake, tier, ZK-attested fingerprint | Identity truth |
| **User deployment list** | Railway/Fly URLs, manually added endpoints | Reachability truth |

### Merged agent view

```rust
pub struct MergedAgent {
    pub id: String,
    pub name: String,

    // From relay
    pub online: bool,
    pub last_seen: u64,
    pub mode: AgentMode,
    pub profile: String,

    // From A2A agent card
    pub a2a_capabilities: Option<Vec<String>>,
    pub hdc_fingerprint: Option<HdcVector>,
    pub supported_protocols: Option<Vec<String>>,

    // From chain (ERC-8004)
    pub wallet: Option<Address>,
    pub reputation: Option<f64>,
    pub stake: Option<u128>,
    pub tier: Option<AgentTier>,       // gray | copper | silver | gold | amber
    pub zk_attested_fingerprint: Option<HdcVector>,

    // From deployment list
    pub direct_url: Option<String>,
    pub deploy_platform: Option<DeployPlatform>,
}
```

---

## 14. Backpressure and Coalescing

High-frequency events (heartbeats at 100ms, chain blocks at 2s) need throttling for dashboard consumption.

| Strategy | Applies to | Behavior |
|---|---|---|
| **Coalesce** | heartbeat | Buffer per agent, send latest every 500ms |
| **Drop-oldest** | output_chunk | Ring buffer per agent (1024 chunks). Slow consumers miss old. |
| **Lossless** | gate_result, task_completed | Queue with backpressure. TCP-level flow control on overflow. |
| **Sample** | feed_data | Every Nth update where N = ceil(source_rate / 2Hz) |

---

## 15. Reconnection Protocol

Clients track the last received `seq`. On reconnect, the client sends a `resume` message:

```json
{ "type": "resume", "last_seq": 4821 }
```

### Recovery flow

```
Client                                  Relay
  |                                       |
  |---- WS connect ---------------------->|
  |                                       |
  |---- { "type": "resume",              |
  |       "last_seq": 4821 } ----------->|
  |                                       |
  |                           +-----------+
  |                           | Check gap |
  |                           +-----------+
  |                                       |
  |  Case 1: gap <= 64K entries           |
  |<---- replay events 4822..4900 --------|
  |<---- live events continue ------------|
  |                                       |
  |  Case 2: gap > 64K entries            |
  |<---- { "type": "snapshot",            |
  |        "state": { ... } } ------------|
  |<---- live events continue ------------|
```

### Relay ring buffer

The relay maintains a ring buffer (default: 64K entries, ~10 minutes at moderate throughput). If the gap exceeds the buffer, the relay sends a `snapshot` event with current state followed by live events.

### Snapshot format

```json
{
  "type": "snapshot",
  "seq": 71042,
  "state": {
    "agents": [
      { "id": "coder-1", "online": true, "mode": "persistent", "profile": "coding" },
      { "id": "research", "online": true, "mode": "ephemeral", "profile": "research" }
    ],
    "feeds": [
      { "feed_id": "eth-gas-trend", "agent_id": "chain-watcher-1", "schema": "gas_trend_v1" }
    ],
    "rooms": ["agent:coder-1", "agent:coder-1:heartbeat", "plan:current"]
  }
}
```

### Gap detection on the client

Clients track the last received `seq` and check every incoming message for continuity. A gap (missing sequence numbers between the last received and the current message) means events were lost. On gap detection, the client should reconnect and send a `resume` message.

### Multi-instance handling

Each roko instance connects with a unique `instance_id` (generated at startup, format: `inst_{ulid}`). If two instances claim the same `agent_id`, the relay uses **last-write-wins**. The most recent connection becomes authoritative. The old connection receives a supersession notice:

```json
{ "type": "superseded", "agent_id": "coder-1", "by": "inst_01HZ3X9K2M..." }
```

On receiving `superseded`, the old instance must stop publishing events and heartbeats for that agent. It can continue operating other agents that are not in conflict. This prevents ghost presence where two instances both claim an agent is online.

---

## 16. Multi-Chain Temporal Resolution

Agents operating across multiple chains must reason about finality, reorgs, and cross-chain consistency. Three mechanisms.

### 16.1 Actor-per-chain

Each chain connection is an independent Connector with its own event loop, state, and failure domain. An Agent watching Ethereum and Base runs two `ChainRpcConnector` instances. They share the Agent's Bus for coordination but do not share connection state.

```toml
[[agents]]
name = "multi-chain-watcher"
connectors = [
  { name = "ethereum-rpc", kind = "chain_rpc", url = "wss://eth.llamarpc.com", chain_id = 1 },
  { name = "base-rpc", kind = "chain_rpc", url = "wss://base.llamarpc.com", chain_id = 8453 },
  { name = "arbitrum-rpc", kind = "chain_rpc", url = "wss://arb.llamarpc.com", chain_id = 42161 },
]
```

This avoids cross-chain state coupling. A reorg on Base does not corrupt Ethereum state. Each Connector publishes its own Pulse topic (`feed:base-blocks`, `feed:eth-blocks`).

### 16.2 Finality oracle

Every chain event Signal carries a finality tag. The finality oracle assigns one of three confidence levels per transaction:

```rust
pub enum FinalityLevel {
    /// Transaction is in the canonical chain with sufficient confirmations.
    /// Reorg probability < 10^-6. Safe for irreversible actions.
    Final,

    /// Transaction has moderate confirmation depth.
    /// Reorg probability < 10^-3. Safe for most operations.
    QuasiFinalized,

    /// Transaction is in a recent block or mempool.
    /// Reorg probability is non-trivial. Use for monitoring only.
    Reversible,
}

pub struct FinalityTag {
    pub level: FinalityLevel,
    pub chain_id: u64,
    pub block_number: u64,
    pub confirmations: u64,
    pub timestamp: DateTime<Utc>,
}
```

Chain-specific thresholds:

| Chain | Final | QuasiFinalized | Reversible |
|---|---|---|---|
| Ethereum | 64 blocks (~13 min) | 12 blocks (~2.5 min) | < 12 blocks |
| Base / Arbitrum / OP | L1 finality + proof posted | Sequencer confirmed | Sequencer pending |
| Solana | 32 confirmations | 1 confirmation | Processed |

Cells that consume chain events specify their required finality level. A Verify Cell validating a deposit requires `Final`. A monitoring Cell tracking gas prices accepts `Reversible`.

### 16.3 Reorg handling

When a Connector detects a chain reorganization:

1. Publishes a `ChainReorg` Pulse on Bus with old and new chain heads.
2. All Signals derived from the orphaned block range are tagged `reorg_invalidated: true`.
3. Cells subscribed to the affected Feed receive a `FeedInvalidation` Pulse listing the invalidated sequence range.
4. The Agent's React Cells handle the invalidation (re-process from new canonical chain, alert, or escalate).

```rust
pub struct ChainReorgPulse {
    pub chain_id: u64,
    pub old_head: BlockHash,
    pub new_head: BlockHash,
    pub orphaned_range: Range<u64>,    // block numbers dropped
    pub new_range: Range<u64>,         // block numbers added
    pub depth: u64,                    // reorg depth
}
```

Reorgs deeper than a configurable threshold (default: 10 blocks) trigger a `SafetyViolation` Signal ([doc-16](16-SECURITY.md)).

---

## 17. TOML Configuration

### Relay configuration

```toml
[relay]
url = "wss://relay.nunchi.dev"
workspace_name = "will-dev"
reconnect_interval_ms = 5000
ring_buffer_size = 65536        # 64K entries for reconnection replay
```

### Connector configuration

```toml
[[agents]]
name = "chain-watcher"
connectors = [
  { name = "ethereum-rpc", kind = "chain_rpc", url = "wss://eth.llamarpc.com", chain_id = 1 },
  { name = "base-rpc", kind = "chain_rpc", url = "wss://base.llamarpc.com", chain_id = 8453 },
  { name = "postgres", kind = "database", url = "postgres://localhost/roko" },
]

[agent.mcp_config]
servers = [
  { name = "github", command = "roko-mcp-github" },
  { name = "code-intel", command = "roko-mcp-code" },
]
```

### Multi-chain configuration

```toml
[chains]
default_finality = "QuasiFinalized"
reorg_depth_alert = 10

[chains.ethereum]
chain_id = 1
finality_confirmations = 64
quasi_finality_confirmations = 12

[chains.base]
chain_id = 8453
finality = "l1_proof_posted"
quasi_finality = "sequencer_confirmed"
```

---

## 18. Crate Mapping

| Crate | Responsibility |
|---|---|
| `roko-core` | Connect protocol trait, ConnectorKind, QueryRequest/Response, HealthStatus, FinalityLevel, FinalityTag |
| `roko-agent` | Connector discovery, lifecycle management, MCP auto-registration, A2A card publishing |
| `roko-runtime` | Bus (Pulse pub/sub), relay WebSocket client, reconnection protocol |
| `roko-serve` | HTTP control plane, WebSocket server, workspace registration |
| `roko-agent-server` | Per-agent sidecar, direct HTTP for remote Agents, MCP tool endpoint |
| `roko-chain` | ChainRpcConnector, on-chain feed sources, finality oracle, reorg detection |
| `roko-cli` | Connector and relay configuration in roko.toml |

---

## 19. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| CN-1 | Connect protocol compiles with all 5 methods | `cargo check` on Connect trait |
| CN-2 | Config-based Connector discovery loads declared Connectors | Integration test: declare Connector in config, verify loaded |
| CN-3 | MCP auto-registration creates McpConnector per server | Integration test: configure MCP server, verify McpConnector registered |
| CN-4 | Extension-provided Connector registered via on_init | Integration test: Extension registers Connector, verify available |
| CN-5 | Connector health check runs at configured interval | Integration test: verify periodic health calls |
| CN-6 | Connector disconnect called on Agent shutdown | Integration test: shutdown Agent, verify disconnect called |
| CN-7 | Signal/Pulse payloads flow over MCP tool calls | Integration test: invoke Cell via MCP, verify CellInput/CellOutput format |
| CN-8 | A2A agent card published with HDC fingerprint | Integration test: start Agent, query agent-card.json, verify fingerprint |
| CN-9 | ZK-attested fingerprint written to ERC-8004 identity record | Integration test: attest fingerprint, read from chain, verify match |
| CN-10 | x402 payment intent respects Agent budget bounds | Test: issue payment > remaining budget -> denied |
| CN-11 | Feed registration publishes to relay | Integration test: register Feed, query relay, verify listed |
| CN-12 | Feed subscription delivers Pulses via Bus topic | Integration test: register Feed, subscribe, verify data flow |
| CN-13 | Workspace hello registers with relay on startup | Integration test: start `roko serve`, verify relay has workspace |
| CN-14 | Dashboard auto-connects by owner_wallet | Integration test: match wallet, verify auto-select |
| CN-15 | WebSocket message envelope has seq, ts, room, type, payload | Unit test: serialize message, verify all fields |
| CN-16 | Coalesce strategy buffers heartbeats to 500ms | Integration test: send 10 heartbeats in 500ms, verify 1 delivered |
| CN-17 | Drop-oldest strategy uses ring buffer for output_chunk | Integration test: overflow buffer, verify old chunks dropped |
| CN-18 | Lossless strategy queues gate_result events | Integration test: slow consumer, verify all events delivered |
| CN-19 | Reconnection with resume replays from last_seq | Integration test: disconnect, reconnect with seq, verify replay |
| CN-20 | Reconnection gap > 64K sends snapshot | Integration test: large gap, verify snapshot received |
| CN-21 | In-process Agent routing via mpsc (priority 1) | Unit test: local Agent, verify channel send |
| CN-22 | Direct HTTP routing for remote Agents (priority 2) | Integration test: remote Agent with URL, verify HTTP post |
| CN-23 | Relay fallback routing (priority 3) | Integration test: NAT Agent, verify relay forwarding |
| CN-24 | Supersession notice on duplicate agent_id | Integration test: two instances same agent_id, verify superseded |
| CN-25 | Finality oracle tags chain events with correct level | Test: Ethereum event at 12 confirmations -> QuasiFinalized, at 64 -> Final |
| CN-26 | Reorg detection publishes ChainReorg Pulse and invalidates Signals | Integration test: simulate reorg, verify Pulse + invalidation |
| CN-27 | Actor-per-chain isolation prevents cross-chain state corruption | Test: reorg on chain A does not affect chain B state |
| CN-28 | Agent discovery merges relay + A2A + chain + deployment list | Integration test: populate all sources, verify merged view |
