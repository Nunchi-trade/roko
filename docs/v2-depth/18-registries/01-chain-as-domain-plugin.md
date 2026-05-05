# Chain as Domain Plugin

> Depth for [22-REGISTRIES.md](../../unified/22-REGISTRIES.md). How the Nunchi chain emerges as a domain-specific Cell specialization implementing standard protocols, not a privileged infrastructure layer.

---

## 1. The Design Error This Corrects

Early chain architecture treated the blockchain as a special layer -- a subsystem with its own traits (`ChainClient`, `ChainWallet`), its own error type (`ChainError`), its own event model (`ObservedEvent`), and its own verification gates (`TxSimGate`, `WalletGate`, `MevGate`). These are useful abstractions, but they violated the central invariant: **everything is a Cell speaking standard protocols**. The chain had its own `ChainClient` trait when it should have been a Connector. It had its own `BlockObserver` event model when it should have been a Bus subscriber. It had its own `ValidationRegistry` when it should have been a Store Cell with a domain-specific schema.

The result was predictable: chain code could not compose with non-chain code. You could not wire a `TxSimGate` into the same 7-rung Pipeline Graph that runs `CompileGate` and `TestGate`. You could not treat an on-chain identity lookup as a `query()` call on a Connector. You could not feed block-arrival events into the same Bus that carries `verify.completed` Pulses.

This depth doc redesigns the chain layer so that every chain component is a standard Cell specialization. The chain becomes one domain plugin -- alongside code intelligence, research, DeFi oracles, and any future domain -- using the same nine protocols that everything else uses.

---

## 2. The Chain IS a Connector Specialization

A Connector is a Cell that implements the Connect protocol: `connect()`, `query()`, `execute()`, `health()`, `disconnect()`. The existing `ChainClient` trait (read-only chain access) and `ChainWallet` trait (signing and submission) map directly onto this:

```rust
/// ChainConnector: a Connector specialization for EVM-compatible chains.
///
/// Wraps ChainClient (reads) and ChainWallet (writes) behind the
/// standard Connect protocol. ConnectorKind::ChainRpc.
///
/// The Connect protocol's five methods map onto chain operations:
///   connect()    -> establish RPC connection, verify chain_id
///   query()      -> eth_call, get_balance, get_logs, get_storage_at
///   execute()    -> sign_and_submit, wait_for_receipt
///   health()     -> eth_blockNumber latency check
///   disconnect() -> drop RPC connection, flush pending nonces
pub struct ChainConnector {
    /// Cell identity, derived from (name, version, chain_id).
    id: CellId,

    /// Chain configuration: RPC endpoints, chain ID, confirmation depth.
    config: ChainConnectorConfig,

    /// The underlying read client (trait object for backend flexibility).
    client: Arc<dyn ChainClient>,

    /// The underlying write wallet (None if read-only mode).
    wallet: Option<Arc<dyn ChainWallet>>,

    /// Connection state: Disconnected | Connecting | Connected | Degraded.
    state: ConnectorState,

    /// Block tracker for gap detection (from observer.rs).
    tracker: BlockTracker,
}

impl Cell for ChainConnector {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "chain.connector" }
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Connect, ProtocolId::Store, ProtocolId::Observe]
    }
    fn capabilities(&self) -> &Capabilities {
        // Net (RPC calls) + Chain { read: true, write: self.wallet.is_some() }
        &self.config.capabilities
    }
}

#[async_trait]
impl Connect for ChainConnector {
    async fn connect(&mut self, config: &ConnectConfig) -> Result<()> {
        // 1. Connect to RPC endpoint(s) with failover
        // 2. Verify chain_id matches config (reject mismatched networks)
        // 3. Fetch current block number to establish baseline
        // 4. Start block subscription if observer mode is enabled
        let chain_id = self.client.chain_id().await
            .map_err(|e| ConnectError::establishment(e.to_string()))?;
        if chain_id != self.config.expected_chain_id {
            return Err(ConnectError::establishment(format!(
                "chain_id mismatch: expected {}, got {}",
                self.config.expected_chain_id, chain_id
            )));
        }
        self.state = ConnectorState::Connected;
        Ok(())
    }

    async fn query(&self, request: QueryRequest) -> Result<QueryResponse> {
        // Dispatch to the appropriate ChainClient method based on query kind.
        // Every chain read is a query(): balances, logs, storage, receipts.
        match request.kind.as_str() {
            "eth_call" => {
                let tx = decode_tx_request(&request.payload)?;
                let result = self.client.eth_call(&tx, request.block).await?;
                Ok(QueryResponse::from_bytes(result.output))
            }
            "get_balance" => {
                let balance = self.client.get_balance(
                    &request.address, request.block
                ).await?;
                Ok(QueryResponse::from_u128(balance))
            }
            "get_logs" => {
                let logs = self.client.get_logs(
                    request.from_block, request.to_block,
                    &request.addresses, &request.topics,
                ).await?;
                Ok(QueryResponse::from_logs(logs))
            }
            _ => Err(ConnectError::unsupported(request.kind)),
        }
    }

    async fn execute(&self, request: ExecuteRequest) -> Result<ExecuteResponse> {
        // Sign and submit a transaction. This is the ONLY write path.
        let wallet = self.wallet.as_ref()
            .ok_or_else(|| ConnectError::unsupported("read-only connector"))?;
        let tx = decode_tx_request(&request.payload)?;
        let tx_hash = wallet.sign_and_submit(tx).await?;
        let receipt = wallet.wait_for_receipt(&tx_hash, request.timeout_ms).await?;
        Ok(ExecuteResponse::from_receipt(receipt))
    }

    async fn health(&self) -> Result<HealthStatus> {
        let start = Instant::now();
        match self.client.block_number().await {
            Ok(block) => Ok(HealthStatus {
                status: Status::Healthy,
                latency_ms: start.elapsed().as_millis() as u64,
                detail: format!("block {block}"),
            }),
            Err(ChainError::Timeout(msg)) => Ok(HealthStatus {
                status: Status::Degraded,
                latency_ms: start.elapsed().as_millis() as u64,
                detail: msg,
            }),
            Err(e) => Ok(HealthStatus {
                status: Status::Unhealthy,
                latency_ms: start.elapsed().as_millis() as u64,
                detail: e.to_string(),
            }),
        }
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.state = ConnectorState::Disconnected;
        Ok(())
    }
}
```

### 2.1 What This Unifies

Before: chain code called `client.get_balance()` directly. Non-chain code called `connector.query()`. They could not compose.

After: an agent that needs a balance check calls `connector.query(QueryRequest { kind: "get_balance", .. })` regardless of whether the connector wraps an RPC node, a simulator (mirage-rs), or a mock. The same agent can hold a database Connector and a chain Connector and address them identically. The Route Cell selects among Connectors by kind, latency, and cost -- the same Route Cell that picks between Claude and Gemini also picks between Ethereum mainnet and Base.

### 2.2 ChainError Maps to ConnectError

The domain-specific `ChainError` enum (Rpc, Timeout, Offline, InsufficientFunds, NonceGap, InvalidAddress, Unsupported) does not disappear. It becomes the inner error of `ConnectError`, which the Connect protocol already defines:

```rust
/// Connect protocol errors carry a domain-specific inner.
pub enum ConnectError {
    Establishment(String),
    QueryFailed { source: Box<dyn Error> },
    ExecuteFailed { source: Box<dyn Error> },
    HealthCheckFailed { source: Box<dyn Error> },
    Unsupported(String),
}

// ChainError converts to ConnectError at the protocol boundary:
impl From<ChainError> for ConnectError {
    fn from(e: ChainError) -> Self {
        match e {
            ChainError::Rpc(msg) => ConnectError::QueryFailed {
                source: Box::new(e),
            },
            ChainError::Offline => ConnectError::Establishment(
                "no reachable RPC".into(),
            ),
            // ... remaining variants
        }
    }
}
```

No information is lost. Callers that need chain-specific error detail can downcast; callers that only need protocol-level semantics use `ConnectError` directly.

---

## 3. ChainSubstrate IS Store Protocol

The existing `FileSubstrate` (JSONL-backed signal storage in `.roko/signals.jsonl`) is a Store Cell. On-chain state is also a Store Cell -- it just reads and writes via `ChainConnector` instead of the filesystem.

```rust
/// ChainStore: the Store protocol applied to on-chain state.
///
/// Signals stored here are anchored on-chain. The content hash is the
/// on-chain key; the Signal metadata (score, provenance, HDC fingerprint)
/// is stored in a companion off-chain index for fast retrieval.
///
/// Three tiers of storage:
///   Hot:  in-memory cache (recent blocks, ~1000 signals)
///   Warm: local index (SQLite or JSONL, full history)
///   Cold: on-chain (the source of truth, read via ChainConnector.query())
pub struct ChainStore {
    id: CellId,

    /// The chain connector used for reads and writes.
    connector: Arc<ChainConnector>,

    /// Local cache of recently-accessed signals (LRU, capped).
    cache: LruCache<ContentHash, Signal>,

    /// Off-chain index mapping content hashes to on-chain locations
    /// (contract address, storage slot, block number).
    index: ChainStoreIndex,
}

impl Cell for ChainStore {
    fn name(&self) -> &str { "chain.store" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }
}

#[async_trait]
impl Store for ChainStore {
    async fn read(&self, hash: &ContentHash) -> Result<Option<Signal>> {
        // 1. Check hot cache
        if let Some(signal) = self.cache.get(hash) {
            return Ok(Some(signal.clone()));
        }

        // 2. Check local index for on-chain location
        let Some(location) = self.index.locate(hash) else {
            return Ok(None);
        };

        // 3. Read from chain via connector query
        let response = self.connector.query(QueryRequest {
            kind: "get_storage_at".into(),
            address: location.contract.clone(),
            slot: location.slot.clone(),
            block: Some(location.block_number),
            ..Default::default()
        }).await?;

        let signal = decode_signal_from_chain_bytes(&response.data)?;
        Ok(Some(signal))
    }

    async fn write(&self, signal: &Signal) -> Result<ContentHash> {
        // 1. Compute content hash
        let hash = signal.content_hash();

        // 2. Encode signal for on-chain storage
        let calldata = encode_signal_for_chain(signal);

        // 3. Submit via connector execute
        self.connector.execute(ExecuteRequest {
            payload: calldata,
            timeout_ms: 30_000,
            ..Default::default()
        }).await?;

        // 4. Update local index and cache
        self.cache.put(hash, signal.clone());
        Ok(hash)
    }

    async fn query(&self, predicate: &Predicate) -> Result<Vec<Signal>> {
        // For simple predicates (by kind, by age), scan local index.
        // For HDC similarity queries, use the on-chain precompile
        // (see 02-hdc-on-chain-and-verification.md).
        self.index.query_with_cache(predicate, &self.cache).await
    }
}
```

### 3.1 The Three-Level Knowledge Hierarchy

The three-level knowledge architecture falls out naturally once ChainStore is a Store Cell:

| Level | Store Cell | Scope | Latency | Cost |
|---|---|---|---|---|
| **Local** | `FileSubstrate` (NeuroStore) | Single agent | < 1 ms | Free |
| **Mesh** | `MeshStore` (gossip) | Agent swarm | 10-100 ms | Free |
| **Chain** | `ChainStore` | Global (all agents) | 50-500 ms | Gas |

All three implement Store. A Route Cell selects which Store to query first based on the expected hit rate and cost. Writes flow upward: local first, promoted to mesh if Score exceeds mesh threshold, promoted to chain if Score exceeds chain threshold. This is the same tiered-storage pattern used for Signal demurrage tiers (Warm/Hot/Archive), applied to storage scope.

### 3.2 What Goes On-Chain vs Off-Chain

The boundary is a Store-level decision, not an architectural decision:

| On-chain (ChainStore) | Off-chain (FileSubstrate / NeuroStore) |
|---|---|
| Knowledge entries (content hash + HDC fingerprint) | Full Signal payload (body text, embedments) |
| Identity records (ERC-8004 fields) | Episode logs (agent turn recordings) |
| Reputation scores (per-domain EMA) | System prompts, templates |
| Validation proofs (Verify scores, job hashes) | Daimon state (affect engine) |
| Pheromone signals (coordination) | Strategy parameters |
| Job market listings (escrow, deadlines) | Local learning state (cascade router) |
| KORAI token balances (with demurrage) | Efficiency events |

The distinction is not "important vs unimportant." It is "globally verifiable vs locally useful." On-chain Signals must be small (gas cost) and meaningful to third parties (verification, reputation, coordination). Off-chain Signals can be large, private, and fast.

---

## 4. ChainBus IS Bus with Block-Arrival Pulses

The existing `BlockObserver` watches for new blocks, filters log entries against watched addresses, and produces `ObservedEvent` values. In the unified vocabulary, this is a Bus -- a typed channel where block arrivals are Pulses and log entries are Pulses graduated to Signals when they match.

```rust
/// ChainBus: the Bus with block-arrival-triggered Pulses.
///
/// Each new block produces a BlockArrival Pulse. Log entries matching
/// the observer's address filter produce LogMatch Pulses. Matched
/// events can be graduated to Signals and persisted in ChainStore.
pub struct ChainBus {
    id: CellId,

    /// The underlying Bus (ring buffer, topic subscriptions).
    inner: Bus,

    /// Block observer for address/topic filtering.
    observer: BlockObserver,

    /// The chain connector providing block data.
    connector: Arc<ChainConnector>,
}

impl ChainBus {
    /// Standard topic names for chain events.
    pub const TOPIC_BLOCK_ARRIVAL: &'static str = "chain.block.arrival";
    pub const TOPIC_LOG_MATCH: &'static str = "chain.log.match";
    pub const TOPIC_TX_CONFIRMED: &'static str = "chain.tx.confirmed";
    pub const TOPIC_IDENTITY_CHANGED: &'static str = "chain.identity.changed";
    pub const TOPIC_REPUTATION_UPDATED: &'static str = "chain.reputation.updated";

    /// Process a new block header and its logs.
    ///
    /// Emits a BlockArrival Pulse for every block, plus a LogMatch
    /// Pulse for each log entry passing the observer's address filter.
    pub async fn on_new_block(
        &mut self,
        header: &ChainHeader,
        logs: &[LogEntry],
    ) -> Vec<Pulse> {
        let mut pulses = Vec::new();

        // 1. Block arrival Pulse (always emitted)
        pulses.push(Pulse::new(
            Self::TOPIC_BLOCK_ARRIVAL,
            PulsePayload::BlockArrival {
                number: header.number,
                hash: header.hash.clone(),
                timestamp: header.timestamp,
            },
        ));

        // 2. Process through observer filter
        let events = self.observer.process_block(header, logs);
        for event in &events {
            pulses.push(Pulse::new(
                Self::TOPIC_LOG_MATCH,
                PulsePayload::LogMatch {
                    block_number: event.block_number,
                    address: event.log.address.clone(),
                    topics: event.log.topics.clone(),
                    data: event.log.data.clone(),
                },
            ));
        }

        // 3. Publish all Pulses to the inner Bus
        for pulse in &pulses {
            self.inner.publish(pulse.clone());
        }

        pulses
    }
}
```

### 4.1 Block-Driven Trigger Pattern

The Trigger protocol (doc-13) defines how events cause Graphs to fire. Block arrivals are a natural Trigger source. Instead of a custom block-watching loop, the chain domain uses a standard Trigger Cell:

```rust
/// Trigger Cell that fires a Graph on every Nth block.
///
/// Configuration:
///   every_n_blocks: u64     -- fire every N blocks (default 1)
///   min_confirmations: u64  -- wait for N confirmations before firing
///   filter: AddressFilter   -- only fire if matching logs present
pub struct BlockTrigger {
    id: CellId,
    config: BlockTriggerConfig,
}

impl Cell for BlockTrigger {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Trigger] }
}

#[async_trait]
impl Trigger for BlockTrigger {
    async fn should_fire(&self, pulse: &Pulse) -> bool {
        let PulsePayload::BlockArrival { number, .. } = &pulse.payload else {
            return false;
        };
        number % self.config.every_n_blocks == 0
    }

    fn source_topics(&self) -> &[&str] {
        &[ChainBus::TOPIC_BLOCK_ARRIVAL]
    }
}
```

This lets any Graph subscribe to block events without knowing anything about chain internals. A DeFi price-update Graph, a reputation-decay Graph, and a heartbeat Graph all wire the same way: `BlockTrigger -> [their logic]`.

---

## 5. The Three Registries ARE Store Cells

The codebase has three registries: `AgentRegistry` (identity), `ReputationRegistry` (per-domain scores), and `ValidationRegistry` (work proofs). Each is currently a standalone `HashMap`-backed in-memory store. In the unified vocabulary, each is a Store Cell with a domain-specific schema.

### 5.1 IdentityStore

```rust
/// IdentityStore: Store Cell for ERC-8004 agent identities.
///
/// Schema: Signal<Kind::Identity> with body fields mapped to
/// AgentIdentity struct. Content hash is derived from token_id.
///
/// On-chain: IAgentIdentity contract at 0xA100.
/// Off-chain: local cache in .roko/state/identity.json.
pub struct IdentityStore {
    id: CellId,

    /// In-memory cache (from agent_registry.rs AgentRegistry).
    registry: AgentRegistry,

    /// Chain connector for on-chain reads/writes.
    connector: Option<Arc<ChainConnector>>,
}

impl Cell for IdentityStore {
    fn name(&self) -> &str { "chain.store.identity" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }
}

#[async_trait]
impl Store for IdentityStore {
    async fn read(&self, hash: &ContentHash) -> Result<Option<Signal>> {
        // Decode token_id from hash, look up in registry cache.
        // If cache miss and connector available, read from chain.
        let token_id = decode_token_id(hash)?;
        if let Some(identity) = self.registry.get(token_id) {
            return Ok(Some(identity_to_signal(identity)));
        }
        if let Some(conn) = &self.connector {
            let response = conn.query(QueryRequest {
                kind: "identity.get".into(),
                payload: encode_token_id(token_id),
                ..Default::default()
            }).await?;
            let identity = decode_identity(&response.data)?;
            return Ok(Some(identity_to_signal(&identity)));
        }
        Ok(None)
    }

    async fn write(&self, signal: &Signal) -> Result<ContentHash> {
        // Extract identity fields from Signal, call register() or update().
        let identity = signal_to_identity(signal)?;
        if let Some(conn) = &self.connector {
            conn.execute(ExecuteRequest {
                payload: encode_register_call(&identity),
                ..Default::default()
            }).await?;
        }
        self.registry.register(identity.clone());
        Ok(signal.content_hash())
    }

    async fn query(&self, predicate: &Predicate) -> Result<Vec<Signal>> {
        // Support queries: by_owner, by_capability, by_tier.
        match predicate {
            Predicate::ByOwner(addr) => {
                let ids = self.registry.by_owner(addr);
                Ok(ids.into_iter().map(|id| identity_to_signal(&id)).collect())
            }
            Predicate::ByCapability(cap) => {
                let ids = self.registry.by_capability(cap);
                Ok(ids.into_iter().map(|id| identity_to_signal(&id)).collect())
            }
            _ => Ok(Vec::new()),
        }
    }
}
```

### 5.2 ReputationStore

```rust
/// ReputationStore: Store Cell for per-domain reputation scores.
///
/// Schema: Signal<Kind::Reputation> with body fields mapped to
/// AgentReputation struct. Content hash is derived from (passport_id, domain).
///
/// On-chain: ReputationRegistry contract at 0xA200.
/// Off-chain: in-memory EMA state (from reputation_registry.rs).
pub struct ReputationStore {
    id: CellId,

    /// In-memory state (the existing ReputationRegistry).
    registry: ReputationRegistry,

    /// Chain connector for on-chain sync.
    connector: Option<Arc<ChainConnector>>,
}

impl Cell for ReputationStore {
    fn name(&self) -> &str { "chain.store.reputation" }
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Store, ProtocolId::Score]
    }
}

/// ReputationStore also implements Score protocol: given a Signal
/// representing an agent's work, it returns the agent's reputation
/// in the relevant domain. This makes reputation a first-class
/// scoring dimension alongside novelty, recency, and relevance.
#[async_trait]
impl Score for ReputationStore {
    async fn score(&self, signal: &Signal, ctx: &CellContext) -> Result<ScoreVector> {
        let passport_id = extract_passport_id(signal)?;
        let domain = extract_domain(signal)?;
        let now = ctx.now_secs();

        let reputation = self.registry.get_score(passport_id, &domain, now);
        let discipline = self.registry.discipline_state(passport_id, now);

        Ok(ScoreVector {
            reputation,
            discipline_penalty: match discipline {
                DisciplineState::GoodStanding => 0.0,
                DisciplineState::Probation => 0.2,
                DisciplineState::Suspended => 0.8,
                DisciplineState::Banned => 1.0,
            },
            feedback_weight: self.registry.feedback_weight(passport_id, now),
        })
    }
}
```

### 5.3 ValidationStore

```rust
/// ValidationStore: Store Cell for work proof attestation.
///
/// Schema: Signal<Kind::Validation> with body fields mapped to
/// ValidationRecord. Content hash is derived from (job_hash, passport_id).
///
/// On-chain: ValidationRegistry contract at 0xA300.
/// Off-chain: in-memory records (from validation_registry.rs).
pub struct ValidationStore {
    id: CellId,

    /// In-memory state (the existing ValidationRegistry).
    registry: ValidationRegistry,

    /// Chain connector for on-chain sync.
    connector: Option<Arc<ChainConnector>>,
}

impl Cell for ValidationStore {
    fn name(&self) -> &str { "chain.store.validation" }
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Store, ProtocolId::Verify]
    }
}

/// ValidationStore also implements Verify protocol: given a Signal
/// claiming work was completed, it checks the on-chain validation
/// record and returns a Verdict.
#[async_trait]
impl Verify for ValidationStore {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        let start = Instant::now();
        let job_hash = extract_job_hash(signal);
        let passport_id = extract_passport_id(signal).unwrap_or(0);

        match self.registry.verify_proof(&job_hash, passport_id) {
            VerificationResult::Verified { pass_rate, block_number, attested } => {
                Verdict::pass("chain.validation")
                    .with_detail(format!(
                        "verified: pass_rate={pass_rate:.2}, block={block_number}, attested={attested}"
                    ))
                    .with_duration(start.elapsed().as_millis() as u64)
            }
            VerificationResult::Rejected { pass_rate, threshold } => {
                Verdict::fail("chain.validation", format!(
                    "rejected: pass_rate={pass_rate:.2} < threshold={threshold:.2}"
                ))
                .with_duration(start.elapsed().as_millis() as u64)
            }
            VerificationResult::NotFound => {
                Verdict::fail("chain.validation", "no validation record found")
                    .with_duration(start.elapsed().as_millis() as u64)
            }
        }
    }

    fn name(&self) -> &str { "chain.validation" }
}
```

### 5.4 Multi-Protocol Cells

Notice that IdentityStore implements Store, ReputationStore implements Store + Score, and ValidationStore implements Store + Verify. This is the power of the Cell model: a single Cell can speak multiple protocols. The ReputationStore is *simultaneously* a place to persist reputation data (Store) and a Score Cell that rates agents by reputation (Score). The ValidationStore is *simultaneously* a place to record proofs (Store) and a verifier that checks whether a proof exists (Verify). These dual roles were implicit in the original code; making them explicit protocols lets the Graph composer wire them into any pipeline.

---

## 6. ERC-8004 Identity IS a Signal Kind

Agent identity is not a special-purpose struct. It is a Signal with `Kind::Identity` and domain-specific body fields. The `AgentIdentity` struct from `roko-chain/src/identity_economy_identity.rs` maps directly onto Signal fields:

```rust
/// How an ERC-8004 identity maps to Signal fields.
///
/// Every identity IS a Signal. The content hash (identity) is derived
/// from the token_id. The body carries the mutable fields.
/// Non-transferable semantics are enforced by the on-chain contract,
/// not by the Signal type system.
fn identity_to_signal(id: &AgentIdentity) -> Signal {
    Signal::builder(Kind::Identity)
        // Content hash from token_id (stable across updates)
        .content_hash(ContentHash::from_seed(
            &id.token_id.to_le_bytes()
        ))
        // Provenance: on-chain at contract address 0xA100
        .provenance(Provenance::OnChain {
            chain_id: id.chain_id,
            contract: "0xA100".into(),
            block: id.created_at_block,
        })
        // Body: the identity-specific fields
        .body(Body::Structured(IdentityBody {
            token_id: id.token_id,
            wallet: id.wallet,
            name: id.name.clone(),
            capabilities: id.capabilities.clone(),
            tier: id.tier,
            reputation_score: id.reputation_score,
            feeds: id.feeds.clone(),
            service_endpoints: id.service_endpoints.clone(),
            delegation_caveats: id.delegation_caveats.clone(),
            parent_identity: id.parent_identity,
            metadata_uri: id.metadata_uri.clone(),
        }))
        // HDC fingerprint: encode identity fields for similarity search
        .hdc_fingerprint(encode_identity_fingerprint(id))
        // Score: initial balance from reputation
        .score(Score {
            relevance: id.reputation_score as f32,
            novelty: 0.0,  // identities are not novel
            confidence: 1.0, // on-chain = verified
            recency: 1.0,
            salience: match id.tier {
                ReputationTier::Gray => 0.2,
                ReputationTier::Copper => 0.4,
                ReputationTier::Silver => 0.6,
                ReputationTier::Gold => 0.8,
                ReputationTier::Amber => 1.0,
            },
        })
        // Demurrage: identities do not decay (balance is refreshed by activity)
        .balance(f64::MAX)
        .build()
}
```

### 6.1 Non-Transferable Semantics

The unified spec (22-REGISTRIES.md S2) describes identities as transferable NFTs. The on-chain contract enforces transfer rules (owner-only, caveat narrowing for children). In the Signal model, transfer is a provenance-chain operation: when an identity is transferred, a new Signal is emitted with the old Signal's content hash as a parent reference and the new wallet as provenance. The old Signal's balance drains to zero (standard demurrage archival). The new Signal inherits the HDC fingerprint but has fresh provenance.

This means identity transfer is visible in the provenance DAG -- the same DAG that tracks knowledge lineage. You can audit "who owned this identity" the same way you audit "where did this knowledge come from."

---

## 7. NUNCHI Demurrage IS Signal Demurrage

The KORAI token has 1% annual demurrage: `effective = stored * (1 - 0.01) ^ (elapsed / year)`. This is exactly the Signal demurrage formula from [01-SIGNAL.md](../../unified/01-SIGNAL.md), applied to token balances instead of knowledge freshness.

The existing `BalanceRecord::effective_balance()` in `korai_token.rs` is isomorphic to the Signal demurrage tick:

| Signal demurrage | Token demurrage |
|---|---|
| `balance -= flat_tax + prop_charge` | `effective = stored * (1 - rate)^(t/year)` |
| Reinforcement: earned by query hits | Reinforcement: earned by task completion |
| Threshold: archive if balance < min | Threshold: none (balance approaches zero asymptotically) |
| Rate: 0.05/day default (configurable) | Rate: 1%/year (fixed) |
| Per-Signal | Per-address |

The difference is quantitative (tokens decay much slower than knowledge freshness), not structural. Both are Gesell demurrage applied to different asset classes. The unified view:

```rust
/// Demurrage is a universal property of valued assets in Roko.
///
/// Signals apply it to knowledge (fast decay, novelty-weighted).
/// KORAI applies it to tokens (slow decay, constant rate).
/// Reputation applies it to scores (30-day half-life toward neutral).
///
/// All three use the same exponential form:
///   effective(t) = anchor + (stored - anchor) * decay^(elapsed / period)
///
/// Where:
///   Signal:     anchor = 0,    period = ~20 days, decay = exp(-rate)
///   KORAI:      anchor = 0,    period = 1 year,   decay = 0.99
///   Reputation: anchor = 0.5,  period = 30 days,  decay = 0.5
pub trait Demurrage {
    fn effective_value(&self, now: u64) -> f64;
    fn anchor(&self) -> f64;
    fn period_secs(&self) -> f64;
    fn decay_factor(&self) -> f64;
}

impl Demurrage for BalanceRecord {
    fn effective_value(&self, now: u64) -> f64 {
        self.effective_balance(now, DEFAULT_DEMURRAGE_RATE) as f64
    }
    fn anchor(&self) -> f64 { 0.0 }
    fn period_secs(&self) -> f64 { SECONDS_PER_YEAR }
    fn decay_factor(&self) -> f64 { 1.0 - DEFAULT_DEMURRAGE_RATE }
}

impl Demurrage for DomainReputation {
    fn effective_value(&self, now: u64) -> f64 {
        self.effective_score(now)
    }
    fn anchor(&self) -> f64 { NEUTRAL }
    fn period_secs(&self) -> f64 { HALF_LIFE_SECS }
    fn decay_factor(&self) -> f64 { 0.5 }
}
```

The unification is not decorative. It means the same learning loop that tunes Signal demurrage rates (based on Store hit rates and archival frequency) could also tune token demurrage rates (based on velocity and hoarding metrics). The same telemetry lens that shows "which knowledge is decaying fastest" also shows "which accounts are losing tokens fastest." Cross-domain insights become possible because the underlying model is the same.

---

## 8. Chain Verify Cells

The existing chain gates (`TxSimGate`, `WalletGate`, `MevGate`) are already Verify Cells in spirit -- they implement `roko_core::traits::Verify` and return `Verdict`. The redesign changes nothing about their internal logic. What changes is their *composition context*: they can now be wired into the standard 7-rung Pipeline Graph alongside `CompileGate`, `TestGate`, and the others.

```rust
/// How chain Verify Cells fit into the Pipeline Graph.
///
/// For agent tasks involving on-chain operations, the rung
/// configuration adds chain-specific Verify Cells:
///
/// Rung 0: CompileGate        (code compiles)
/// Rung 1: ClippyGate         (no lint warnings)
/// Rung 2: TestGate           (unit tests pass)
/// Rung 3: TxSimGate          (transaction simulates successfully)
/// Rung 4: WalletGate         (wallet has sufficient balance, nonce ok)
/// Rung 5: MevGate            (no MEV exposure detected)
/// Rung 6: IntegrationGate    (end-to-end test against testnet)
///
/// The Route Cell selecting rungs uses the same cost/confidence
/// tradeoff as for non-chain tasks. TxSimGate is cheap (~50ms
/// simulation), so it runs early. MevGate is expensive (mempool
/// analysis), so it runs late and only when the transaction value
/// exceeds a configurable threshold.
```

---

## 9. Domain Plugin Pattern

The chain is one domain plugin. The pattern it establishes is reusable:

| Component | Chain domain | Code-intel domain | Research domain |
|---|---|---|---|
| **Connector** | `ChainConnector` (RPC) | `LspConnector` (language server) | `PerplexityConnector` (search API) |
| **Bus** | `ChainBus` (block Pulses) | `FsWatchBus` (file-change Pulses) | (none -- pull-based) |
| **Store** | `ChainStore` (on-chain state) | `IndexStore` (code graph) | `ResearchStore` (citations) |
| **Score** | `ReputationStore` (agent rep) | `CodeComplexityScorer` | `CitationScorer` |
| **Verify** | `TxSimGate`, `WalletGate`, `MevGate` | `CompileGate`, `TestGate` | `FactCheckGate` |
| **Route** | `CascadeRouter` (model by domain) | `CascadeRouter` (same) | `CascadeRouter` (same) |

Every column uses the same nine protocols. Every column composes with every other column in a Graph. The chain is not special -- it just has more Connectors and more domain-specific Verify Cells.

---

## What This Enables

1. **Cross-domain composition.** A Graph can read code from IndexStore, dispatch an agent to modify it, verify with CompileGate, simulate the resulting transaction with TxSimGate, check reputation via ReputationStore, and publish the result to ChainStore -- all as a single Pipeline Graph with no custom wiring.

2. **Testability without a chain.** Replace `ChainConnector` with `MockChainConnector` (which wraps the existing `MockChainClient` + `MockChainWallet`). The Graph does not change. The mock Connector speaks the same Connect protocol, so every Graph that works against a real chain works identically against the mock.

3. **Multi-chain without architecture changes.** Each chain (Ethereum, Base, Arbitrum, Korai) is a separate `ChainConnector` with its own `chain_id`. The Route Cell selects among them. Adding a new chain requires instantiating a new Connector, not modifying framework code.

4. **Reputation-informed routing.** Because ReputationStore implements Score, the CascadeRouter can include reputation as a scoring dimension when selecting agents for tasks. An agent with Gold-tier coding reputation gets routed higher-capability models; a Gray-tier agent gets cheaper models with tighter Verify thresholds.

5. **Unified monitoring.** The Telemetry system (Observe protocol, Lenses) sees chain Connectors the same way it sees database Connectors. Health dashboards, latency histograms, and error budgets are automatic.

---

## Feedback Loops

1. **Chain health -> Route.** `ChainConnector.health()` latency feeds into the Route Cell's cost model. High-latency RPC endpoints get deprioritized. This is the same health-feedback loop used for LLM provider health in `roko-learn`.

2. **Validation -> Reputation.** When ValidationStore accepts a proof, it emits a `chain.validation.accepted` Pulse. A React Cell subscribes and calls `ReputationStore.submit_feedback()` with the Verify pass rate as quality. Reputation improves. This closes the validation-to-reputation feedback loop currently implicit in the codebase.

3. **Demurrage -> Store promotion.** Signals in NeuroStore whose demurrage balance drops below threshold are candidates for archival. Signals whose balance *rises* (heavy use) are candidates for promotion to ChainStore (global durability). The same demurrage tick that governs local freshness also governs global publication.

4. **Block events -> Learning.** Block-arrival Pulses feed into the learning system. Gas prices, block times, and confirmation depths are observable metrics. The CascadeRouter can learn "transaction-heavy tasks should run during low-gas periods" by correlating block Pulses with task outcomes.

---

## Open Questions

1. **State sync granularity.** How frequently does ChainStore sync with on-chain state? Per-block (50ms, expensive) or per-epoch (configurable, cheaper)? The answer affects consistency guarantees for cross-agent coordination.

2. **Partial failure semantics.** If `ChainConnector.execute()` submits a transaction that is included but reverts, the Connector reports success (the tx was mined) but the Verify Cell reports failure (the revert). How does the Graph handle this split verdict? Current approach: the Graph treats Verify failure as authoritative over Connector success.

3. **Privacy boundary enforcement.** The on-chain/off-chain split (S3.2) is currently advisory. Should ChainStore refuse to write Signals with `Kind::Episode` or `Kind::SystemPrompt`? Or should the boundary be a policy decision left to the React protocol?

4. **Connector multiplexing.** An agent interacting with three chains (Ethereum for DeFi, Korai for identity, Base for cheap settlement) needs three ChainConnectors. Should these be separate Cells in the Graph, or a single MultiplexConnector that routes by chain_id?

5. **Mock fidelity.** The existing `MockChainClient` is state-based (in-memory block numbers, receipts, logs). For integration testing of ChainStore, do we need a fork-mode mock that replays real chain state? Or is the current mock sufficient?
