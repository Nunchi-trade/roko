# Relay Deployment Models

> Depth for [11-CONNECTIVITY.md](../../v2/11-CONNECTIVITY.md). Three deployment models for the relay service: sidecar (per-instance), shared (multi-tenant), and validator-embedded. Library-plus-binary architecture enables all three from a single codebase.

**Depends on**: [11-CONNECTIVITY](../../v2/11-CONNECTIVITY.md) (relay wire protocol, agent connectivity), [25-DEPLOYMENT](../../v2/25-DEPLOYMENT.md) (Railway, Docker)

---

## 1. Design Principle

The relay is a library crate (`agent-relay`) with a thin standalone binary. The same code runs
as a sidecar, a shared service, or a supervised task inside a validator. It holds no durable
state -- everything in memory is a projection of on-chain data or ephemeral agent presence.

### Library-Binary Split

```
agent-relay (library crate)
  ├── bus.rs          TopicBus: topic-keyed pub/sub with bounded ring buffers
  ├── chain_watcher.rs  Polls eth_blockNumber, publishes to chain:{chain_id}
  ├── protocol.rs     Wire types: AgentInboundFrame, RelayOutboundFrame, TopicEnvelope
  ├── state.rs        RelayState: agent registry, workspace registry, pending messages
  └── lib.rs          app() -> Router  (axum)

agent-relay (binary: ~40 lines)
  └── main.rs         CLI args, bind listener, spawn chain watcher, serve
```

The library exports `app(state) -> Router` and `run_relay(config, chain_source)`. Any binary
that can construct a `RelayState` and bind a TCP listener can host the relay.

### Invariants

1. **Relay state is transient.** Agent registrations, topic subscriptions, and ring buffer
   contents live in memory. If the relay restarts, agents reconnect and resubscribe. No data
   is lost because the chain is the source of truth.
2. **Any relay watching the same contracts produces the same event projection.** Two relays
   pointed at the same RPC endpoint emit identical `new_block` envelopes (modulo timing jitter).
3. **Agents connect to 1-3 relays simultaneously for redundancy.** The multi-relay client
   deduplicates by `(topic, seq)` pairs.
4. **No relay is privileged.** There is no leader election, no consensus, no shared state
   between relays. Each relay is an independent projection.

```
                    ┌────────────┐
                    │   Chain    │
                    │ (Ethereum) │
                    └─────┬──────┘
                          │ eth_blockNumber
              ┌───────────┼───────────┐
              ▼           ▼           ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │ Relay A  │ │ Relay B  │ │ Relay C  │
        │ (sidecar)│ │ (shared) │ │(embedded)│
        └────┬─────┘ └────┬─────┘ └────┬─────┘
             │            │            │
             │      ┌─────┴─────┐      │
             ▼      ▼           ▼      ▼
          ┌─────┐ ┌─────┐   ┌─────┐ ┌─────┐
          │Ag. 1│ │Ag. 2│   │Ag. 3│ │Ag. 4│
          └─────┘ └─────┘   └─────┘ └─────┘

All three relays poll the same chain. Each produces
identical event projections independently.
```

---

## 2. Sidecar Deployment (Default)

Runs on `127.0.0.1:9011` inside the user's Railway or Docker service. Private to the user's
instance -- no cross-user visibility, no authentication overhead. The `roko-serve` process in
the same container proxies external relay access through `/relay/*` routes when needed.

```
┌──────────────────────────────────────────────────┐
│            Railway Service Container              │
│                                                  │
│  ┌────────────────┐    ┌─────────────────────┐   │
│  │  agent-relay   │    │    roko-serve        │   │
│  │ 127.0.0.1:9011 │◄──►│  0.0.0.0:6677       │   │
│  │ (internal)     │    │  /relay/* proxy      │   │
│  └───────┬────────┘    └──────────┬──────────┘   │
│          │ ws://                  │ https://      │
│    Local agents           External clients       │
└──────────────────────────────────────────────────┘
```

Start command: `agent-relay --bind 127.0.0.1:9011`

### Characteristics

| Property | Value |
|---|---|
| Visibility | Private (loopback only) |
| Auth | None needed (local processes only) |
| Expected connections | 10-50 (one user's agent pool) |
| Chain events | From local `mirage-rs` or configured RPC |
| Ring buffer size | 128 messages per topic (default) |
| Best for | Development, single-user setups, self-hosted agents |

---

## 3. Shared Deployment

Standalone relay accessible at a public URL. Multiple agents from multiple users connect to it.

```
           ┌──────────────────────────────┐
           │  Shared Relay                 │
           │  relay.example.com:9011       │
           │  TopicBus + ChainWatcher      │
           │  + AgentRegistry              │
           └──────────┬───────────────────┘
                      │
         ┌────────────┼────────────┐
         ▼            ▼            ▼
    User A agents  User B agents  User C agents
```

Start command: `agent-relay --bind 0.0.0.0:9011 --rpc-ws-url wss://rpc.chain.network --chain-id 1`

Binding to `0.0.0.0` makes the relay network-accessible. The `--rpc-ws-url` flag enables the
chain watcher, publishing `new_block` envelopes to `chain:{chain_id}`.

### Cross-User Features

- **Agent discovery.** Connected agent cards visible via `GET /relay/agents` and
  `GET /relay/cards/{id}`, enabling cross-user agent-to-agent messaging.
- **Marketplace feeds.** Agents subscribe to `marketplace:jobs` and receive job postings
  from all users. No central job server required.
- **Shared chain events.** One relay polls `eth_blockNumber` once, N agents receive. Eliminates
  redundant RPC calls.
- **Workspace registry.** Each `roko-serve` instance registers via `WorkspaceHello`, enabling
  cross-instance coordination.

### Anyone Can Run One

The relay binary is the same as sidecar mode. Point it at a public address and an RPC endpoint.
No special permissions needed -- analogous to running an Ethereum RPC node.

### Characteristics

| Property | Value |
|---|---|
| Visibility | Public (network-accessible) |
| Auth | Read: none. Publish: agent passport (ERC-8004) |
| Expected connections | 100-10,000 |
| Chain events | From production RPC (mainnet, Base, Arbitrum) |
| Ring buffer size | 512+ messages per topic (configured higher) |
| Best for | Marketplace job discovery, shared ISFR feeds, community relays |

---

## 4. Multi-Relay Connection

Agents connect to multiple relays simultaneously. The recommended configuration is one sidecar
relay for low-latency private communication plus one or two shared relays for cross-user
features.

### Configuration

```rust
/// Relay endpoint configuration.
/// Agents connect to all configured endpoints on startup.
pub struct RelayConfig {
    /// Ordered list of relay endpoints to connect to.
    pub relays: Vec<RelayEndpoint>,
}

/// A single relay endpoint with its connection role.
pub struct RelayEndpoint {
    /// WebSocket URL for the relay (ws:// or wss://).
    pub url: String,
    /// How this relay is used by the agent.
    pub role: RelayRole,
}

/// Connection role determines which operations an agent
/// performs on each relay.
pub enum RelayRole {
    /// Full read/write. Agent subscribes, publishes, and
    /// registers its card. Typically the sidecar relay.
    Primary,
    /// Read/write with lower priority. Used for cross-user
    /// discovery and marketplace feeds. Typically a shared relay.
    Secondary,
    /// Agent publishes to this relay but does not subscribe.
    /// Used for one-way feed distribution.
    WriteOnly,
    /// Agent subscribes but does not publish.
    /// Used for consuming shared feeds without exposing presence.
    ReadOnly,
}
```

```toml
# roko.toml
[relay]
endpoints = [
  { url = "ws://127.0.0.1:9011", role = "primary" },
  { url = "wss://relay.nunchi.dev", role = "secondary" },
]
```

### Relay Discovery

Agents discover relays through three mechanisms, checked in order:

1. **Static configuration.** The `[relay].endpoints` array in `roko.toml`. Always checked
   first. The sidecar relay is always configured here.
2. **On-chain agent registry.** The ERC-8004 agent registry contract stores relay URLs
   alongside agent cards. Agents query the registry at startup to discover shared relays
   operated by known parties.
3. **Bootstrap list.** A hardcoded list of well-known relay URLs compiled into the binary.
   Used as a fallback when no configuration exists and the on-chain registry is unreachable.

### Deduplication

When connected to multiple relays, the same event may arrive from more than one source. The
agent tracks the highest sequence number seen per `(relay_url, topic)` pair and discards
anything at or below the watermark. Sequence numbers are per-relay (each relay has its own
monotonic `AtomicU64` counter). Cross-relay deduplication for the same logical event (e.g.,
two relays watching the same chain) uses payload content (`block_number`, `chain_id`).

### Failover

If one relay disconnects, the agent continues on the remaining relays. The multi-relay client
reconnects with exponential backoff (2s, 4s, 8s...). On reconnect, the agent resubscribes
and receives ring buffer replay for messages published during the gap (up to `ring_capacity`
per topic).

---

## 5. Validator-Embedded Deployment

The relay library crate can be embedded directly inside a blockchain validator binary. This
eliminates the JSON-RPC hop for chain events -- the embedded relay reads from the validator's
internal finalized block channel instead.

### Architecture

```
┌────────────────────────────────────────────────────────────┐
│                    Validator Process                        │
│                                                            │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                Consensus Engine                       │  │
│  │                                                      │  │
│  │  Propose ──► Vote ──► Finalize ──► Apply             │  │
│  │                                      │               │  │
│  │                                      │ finalized     │  │
│  │                                      │ block channel │  │
│  │                                      ▼               │  │
│  └──────────────────────────────────────┬───────────────┘  │
│                                         │                  │
│  ┌──────────────────────────────────────▼───────────────┐  │
│  │              Embedded Relay (tokio task)              │  │
│  │                                                      │  │
│  │  LedgerChainWatcher ◄── finalized block channel      │  │
│  │  TopicBus (ring_capacity=128)                        │  │
│  │  AgentRegistry                                       │  │
│  │                                                      │  │
│  │  Separate axum listener: 0.0.0.0:9011                │  │
│  │  No shared state with consensus                      │  │
│  │  Panic tracker with automatic restart                │  │
│  └──────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────┘
                          │
                          │ ws://validator:9011
                          ▼
                    ┌──────────┐
                    │  Agents  │
                    └──────────┘
```

### Library/Binary Split

The library crate exposes `run_relay` and a `ChainEventSource` trait. The standalone binary
and validator embedding call the same function with different chain source implementations:

```rust
/// Run the relay as a long-lived async task.
/// When chain_source is None, the relay operates without chain
/// watching (agents still get pub/sub, presence, forwarding).
pub async fn run_relay(
    config: RelayConfig,
    chain_source: Option<impl ChainEventSource>,
) -> Result<()> {
    let state = Arc::new(RelayState::new());
    let listener = TcpListener::bind(&config.bind).await?;
    if let Some(source) = chain_source {
        let s = Arc::clone(&state);
        tokio::spawn(async move { source.watch(s).await; });
    }
    axum::serve(listener, app(state)).await?;
    Ok(())
}

/// Trait for chain event sources. AlloyChainWatcher polls
/// eth_blockNumber via JSON-RPC. LedgerChainWatcher reads
/// from the validator's internal finalized block channel.
#[async_trait]
pub trait ChainEventSource: Send + 'static {
    async fn watch(&self, state: Arc<RelayState>);
}
```

The standalone binary is ~30 lines: parse CLI, construct `AlloyChainWatcher`, call `run_relay`.
Validator embedding constructs a `LedgerChainWatcher` from the ledger's `subscribe_finalized()`
channel and spawns `run_relay` inside a `supervised()` wrapper for panic isolation.

### Isolation Guarantees

The embedded relay must not interfere with consensus:

1. **Separate network listener.** The relay binds its own port (`--relay-port 9011`). It does
   not share the validator's P2P or RPC listeners.
2. **No shared mutable state.** The relay reads from the finalized block channel (a
   `broadcast::Receiver`) but never writes to consensus state. The channel is one-directional.
3. **Panic isolation.** The relay runs inside a `supervised()` wrapper that catches panics,
   logs them, and restarts the relay task after a backoff delay. A relay panic does not
   propagate to the consensus task.
4. **Resource budget.** The relay's memory usage is bounded by `ring_capacity * num_topics`.
   With 128 messages per topic and 10 topics, this is approximately 1-5 MB -- negligible
   relative to validator memory requirements.

### CLI

```bash
# Start validator with embedded relay on port 9011
validator --relay-port 9011
```

When `--relay-port` is omitted, the validator runs without an embedded relay.

### Characteristics

| Property | Value |
|---|---|
| Chain event latency | ~0ms (internal channel, no RPC) |
| Relay lifecycle | Coupled to validator process |
| Isolation | Separate listener, no shared consensus state |
| Expected connections | 10-50 (sidecar-level, not public) |
| Best for | Small validator sets where operators run everything |

---

## 6. Auth Model

Authentication requirements depend on the deployment model. The principle is: tighter
isolation means less auth overhead.

### Sidecar (Loopback)

No authentication. Only processes on the same host can reach `127.0.0.1:9011`. The operating
system's network stack provides isolation.

```
Agent ──(ws://127.0.0.1:9011)──► Relay
         loopback only
         no auth needed
```

### Shared (Public)

Two tiers of access:

| Operation | Auth Required | Mechanism |
|---|---|---|
| Read agent directory | None | `GET /relay/agents` is public |
| Subscribe to topics | None | WebSocket subscribe frame |
| Read topic messages | None | `GET /relay/topics/{topic}/messages` |
| Publish to topics | Agent passport | ERC-8004 identity in `AgentHello` |
| Register agent card | Agent passport | ERC-8004 identity in `AgentHello` |
| Forward messages | Agent passport | `POST /relay/messages` checks sender |

Agent passport verification uses the ERC-8004 identity standard. The agent's `AgentHello`
frame includes a signed challenge (`keccak256(relay_url || timestamp)`) that the relay verifies
against the on-chain agent registry. Read-only access requires no passport -- anyone can
browse the agent directory or subscribe to public topics.

### Validator-Embedded

Same auth model as shared, plus optional validator-level allowlists restricting which agent
addresses may connect.

---

## 7. Docker Deployment

The standalone relay runs as a single Docker container. This Dockerfile produces a minimal
image suitable for shared relay operation.

```dockerfile
# ── Build stage ───────────────────────────────────────────
FROM rust:1.91-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p agent-relay \
    && strip target/release/agent-relay

# ── Runtime stage ─────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/agent-relay /usr/local/bin/

EXPOSE 9011

# Default: shared relay on all interfaces, no chain watching.
# Override CMD to add --rpc-ws-url for chain event support.
ENTRYPOINT ["agent-relay"]
CMD ["--bind", "0.0.0.0:9011"]
```

Build and run:

```bash
docker build -t agent-relay -f Dockerfile.relay .
docker run -d --name relay -p 9011:9011 agent-relay \
  --bind 0.0.0.0:9011 --rpc-ws-url wss://rpc.chain.network --chain-id 1

# Health check
curl http://localhost:9011/relay/health  # => "ok"
```

---

## 8. Railway Deployment

In Railway, the relay starts alongside `roko-serve` and `mirage-rs` in one container. The
`start-railway.sh` script manages the startup sequence: relay first (fastest to boot, health
checked at `/relay/health`), then `mirage-rs` (local chain), then `roko serve` (HTTP API).

```
start-railway.sh (PID 1)
  ├── agent-relay --bind 127.0.0.1:9011          (core process)
  ├── mirage-rs --host 127.0.0.1 --port 8545 ... (core process)
  └── roko serve --bind 0.0.0.0 --port 6677 ...  (core process)
```

All three are core processes. If any exits, the script shuts down the container.

| Variable | Default | Purpose |
|---|---|---|
| `ROKO_AGENT_RELAY_BIND` | `127.0.0.1:9011` | Relay bind address |
| `ROKO_AGENT_RELAY_URL` | `http://127.0.0.1:9011` | URL agents use to connect |
| `PORT` / `ROKO_PORT` | `6677` | Public HTTP port (Railway sets `PORT`) |

---

## 9. Scaling Considerations

### Connection Capacity by Deployment Model

| Model | Expected Load | Bottleneck |
|---|---|---|
| Sidecar | 10-50 connections | Not a concern (single user) |
| Shared | 100-10,000 connections | WebSocket fan-out, memory for ring buffers |
| Validator | 10-50 connections | Must not contend with consensus |

### Horizontal Scaling (Shared Relays)

Deploy more relay instances, each watching the same chain. No inter-relay coordination needed.

- **Stateless.** No synchronization between instances. Each independently projects chain state.
- **Memory-bounded.** Ring buffers have fixed capacity (`ring_capacity` per topic). Memory
  does not grow with throughput, only with the number of active topics.
- **Fan-out cost.** N subscribers = N WebSocket writes per publish. This is the primary CPU
  cost at high subscriber counts.

### Validator-Embedded Scaling

The embedded relay should not exceed sidecar-level load. Validators needing to serve many
external agents should run a separate shared relay instead.

### Long-Term: Dedicated Relay Operators

As the network grows, dedicated relay operators emerge -- analogous to RPC providers
(Alchemy, Infura). They run high-availability clusters, register relay URLs in the on-chain
agent registry, and charge fees via x402 payment protocol. The stateless architecture makes
this viable: operators do not coordinate with each other.
