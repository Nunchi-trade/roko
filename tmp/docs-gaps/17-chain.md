# 08-chain -- Gap Checklist

Spec: `docs/08-chain/` (25 files). Code: `crates/roko-chain/`, `apps/mirage-rs/`.

Overall: Tier 6, intentionally deferred. ~5% implemented. ChainClient/ChainWallet traits + mirage-rs simulator built. All 6 contracts and gossip network unbuilt.

## Compliant (no action needed)
- ChainClient trait -- 8 read methods (doc 17)
- ChainWallet trait -- 5 write methods (doc 17)
- Mock implementations (doc 17)
- mirage-rs EVM simulator -- local/fork/scenario modes, 141 tests (doc 18)
- Current status doc accurately reflects deferred state (doc 24)

## Checklist (Tier 6 -- all P2/P3, blocked on Tier 5 completion)

### CHAIN-01: KORAI token contract
- [x] Deploy ERC-20 + demurrage token

**Spec** (doc 02): KORAI is the native token of the Korai chain with 1% annual demurrage (holding cost that decays balances over time, preventing hoarding). 5 earning pathways: (1) task completion, (2) knowledge contribution, (3) validation participation, (4) reputation staking, (5) marketplace fees. 5 spending mechanisms: (1) compute purchase, (2) knowledge access, (3) job posting, (4) escrow deposits, (5) governance participation. DAEJI is the testnet equivalent. The demurrage is applied on-read (lazy calculation) rather than periodic transactions to avoid gas overhead.

**Current code** (`crates/roko-chain/src/phase2.rs:1063`): KORAI referenced as a budget unit in phase2 types. `TokenStandard` enum at phase2.rs includes `ERC20` variant. `AlloyChainWallet` at `crates/roko-chain/src/alloy_impl.rs` implements `ChainWallet` trait with `send_transaction()`. No Solidity contract source exists anywhere in the repo. No deployment scripts.

**What to change**: (1) Create `contracts/korai-token/` directory with a Solidity ERC-20 contract implementing `balanceOf()` with lazy demurrage (return `stored_balance * decay_factor(block.timestamp - last_update)`). (2) Add Foundry deployment script. (3) Add integration test deploying to mirage-rs local fork via `AlloyChainWallet`. (4) Wire the contract address into the chain client config.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:1063` -- KORAI budget references, TokenStandard enum
- `crates/roko-chain/src/alloy_impl.rs` -- AlloyChainWallet for deployment
- `crates/roko-chain/src/types.rs` -- TxRequest for transaction construction
- `crates/roko-chain/src/client.rs` -- ChainClient trait for reading contract state
- `docs/08-chain/02-korai-token-economics.md` -- full KORAI spec, demurrage formula, earning/spending mechanisms
**Depends on**: None (first contract)
**Accept when**:
- [x] ERC-20 Solidity contract with lazy demurrage compiles
- [ ] Deploys to mirage-rs local fork
- [x] `balanceOf()` returns demurrage-adjusted balance
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'KORAI\|demurrage' crates/roko-chain/src/ --include='*.rs'
ls contracts/korai-token/ 2>/dev/null || echo "No contract dir yet"
cargo test -p roko-chain
```
**Priority**: P2 (first contract to deploy -- no dependencies)

### CHAIN-02: Agent Registry contract (ERC-721 soulbound)
- [x] Deploy identity registry with passports

**Spec** (doc 04, 06): ERC-721 soulbound (non-transferable) NFT for agent identity. Each passport carries:
- `passport_id: u256` -- auto-incremented at mint
- `owner: Address` -- EOA or multisig controller
- `capability_list: u64` -- 10 capability bits (inference, data-transform, fine-tune, RAG, multi-agent, trading, security, analytics, knowledge, strategy)
- `domain_stakes: BTreeMap<String, u256>` -- per-domain KORAI staked
- `reputation_tracks: BTreeMap<String, ReputationScore>` -- per-domain EMA scores
- `tee_attestation: Option<(Hash, u64)>` -- latest TEE attestation hash + expiry
- `system_prompt_hash: [u8; 32]` -- SHA-256 of system prompt, committed at registration (ventriloquist defense per doc 05)
- `tier: PassportTier` -- Protocol (0), Sovereign (1), Worker (2), Edge (3)
- `slash_history: Vec<SlashRecord>` -- historical slashing events

Four tiers by KORAI stake: Protocol (100K+), Sovereign (25K+), Worker (5K+), Edge (0+). Soulbound property: `transferFrom()` reverts. The ventriloquist defense commits `SHA-256(system_prompt)` at registration; prompt updates require on-chain tx with 24h timelock; >3 changes in 30 days triggers -0.05 reputation (doc 05).

**Current code**: `AgentPassport` struct at `crates/roko-chain/src/phase2.rs:739` with all fields above. `PassportTier` enum at phase2.rs with 4 variants. `ReputationScore` at line 762 with `score: f64`, `job_count: u64`, `last_update: u64`. `SlashRecord` at line 773 with `violation_type`, `amount`, `block_number`. `DidDocument` at `crates/roko-chain/src/identity_economy_identity.rs:44` with DID resolution scaffolding. No Solidity contract source exists.

**What to change**: (1) Create `contracts/agent-registry/AgentRegistry.sol` -- ERC-721 with `_transfer()` override that always reverts (soulbound). Storage mapping `passport_id => AgentPassport`. `mint(address owner, uint64 capabilities) -> uint256` restricted to registry admin. (2) Add `updateSystemPromptHash(uint256 passportId, bytes32 newHash)` with 24h timelock enforced via `block.timestamp > pendingUpdate.submittedAt + 86400`. (3) Add `setTier(uint256 passportId, uint8 tier)` callable only by staking contract. (4) Add integration test deploying to mirage-rs via `AlloyChainWallet`. (5) Wire `AgentPassport` to read from on-chain state via `ChainClient`.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:739` -- AgentPassport struct with all passport fields
- `crates/roko-chain/src/phase2.rs:762` -- ReputationScore struct
- `crates/roko-chain/src/phase2.rs:773` -- SlashRecord struct
- `crates/roko-chain/src/identity_economy_identity.rs:44` -- DidDocument scaffolding
- `crates/roko-chain/src/alloy_impl.rs` -- AlloyChainWallet for contract deployment
- `docs/08-chain/04-korai-passport-erc-721-soulbound.md` -- full passport struct spec, 4 tiers, capability bitmask, registration flow
- `docs/08-chain/05-ventriloquist-defense.md` -- system prompt hash commitment, 24h timelock, rate limiting
- `docs/08-chain/06-erc-8004-registries.md` -- three-registry pattern (Identity/Reputation/Validation)
**Depends on**: CHAIN-01
**Accept when**:
- [x] ERC-721 soulbound contract compiles (transferFrom reverts)
- [x] Passport metadata includes tier, capability bitmask, system_prompt_hash
- [x] Prompt hash update enforces 24h timelock
- [ ] Deploys to mirage-rs local fork
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'ERC721\|soulbound\|passport\|AgentPassport' crates/roko-chain/src/ --include='*.rs'
ls contracts/agent-registry/ 2>/dev/null || echo "No contract dir yet"
cargo test -p roko-chain
```
**Priority**: P2

### CHAIN-03: Reputation Registry contract
- [x] Deploy reputation with 7-domain EMA scoring

**Spec** (doc 14, 06): On-chain reputation registry storing per-domain EMA scores for each agent passport. 7 base domains: `code_quality`, `reliability`, `speed`, `knowledge`, `collaboration`, `security`, `oracle`. Each domain stores:
- `score: f64` -- EMA-smoothed score in [0.0, 1.0]
- `job_count: u64` -- completed jobs in domain
- `last_update: u64` -- last update block number

EMA update formula: `new_score = alpha * observation + (1 - alpha) * old_score` where alpha adapts: `alpha = base_alpha * (1.0 + volatility)`, `base_alpha = 0.1`, `volatility = stddev(recent_10_observations)`. 30-day half-life decay applied on-read: `effective_score = score * 0.5^((now - last_update) / (30 * 86400))`. Four discipline states per agent: `GoodStanding`, `Probation` (score < 0.3), `Suspended` (score < 0.15 or slash count >= 3), `Banned` (governance vote). Slash rates by violation type: incomplete_job=-0.05, quality_failure=-0.10, timeout=-0.03, collusion=-0.50.

**Current code**: `ReputationScore` at `crates/roko-chain/src/phase2.rs:762` with `score: f64`, `job_count: u64`, `last_update: u64`. `ReputationMessage` at phase2.rs:1016 with `passport_id`, `domain`, `old_score`, `new_score`, `job_count`, `reason: ReputationChangeReason`, `epoch`. `ReputationChangeReason` enum at line 1035 with `JobCompletion`, `Slash`, `DemurrageDecay`, `PeerReview` variants. `AgentPassport.reputation_tracks: BTreeMap<String, ReputationScore>` at line 749. No Solidity contract.

**What to change**: (1) Create `contracts/reputation-registry/ReputationRegistry.sol` with `mapping(uint256 => mapping(bytes32 => ReputationScore))` (passport_id => domain => score). (2) Add `submitFeedback(uint256 passportId, bytes32 domain, uint64 quality) external onlyAuthorized` that computes EMA update. (3) Add `getScore(uint256 passportId, bytes32 domain) view` that applies 30-day half-life decay on read. (4) Add `slash(uint256 passportId, bytes32 domain, uint8 violationType) external onlyRegistryAdmin`. (5) Add `getDisciplineState(uint256 passportId) view` returning current discipline state. (6) Wire `ReputationMessage` gossip to contract event emission.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:762` -- ReputationScore struct
- `crates/roko-chain/src/phase2.rs:1016` -- ReputationMessage gossip payload
- `crates/roko-chain/src/phase2.rs:1035` -- ReputationChangeReason enum (4 variants)
- `crates/roko-chain/src/phase2.rs:739` -- AgentPassport.reputation_tracks field
- `crates/roko-chain/src/identity_economy_identity.rs` -- identity linkage types
- `docs/08-chain/14-reputation-system-7-domain.md` -- 7 domains, EMA formula, adaptive alpha, decay, discipline states, slash rates, gaming resistance
- `docs/08-chain/06-erc-8004-registries.md` -- Reputation Registry in three-registry pattern
**Depends on**: CHAIN-02
**Accept when**:
- [x] Reputation contract stores 7-domain EMA scores per passport
- [x] EMA update with adaptive alpha on feedback submission
- [x] 30-day half-life decay applied on score reads
- [x] 4 discipline states tracked (GoodStanding/Probation/Suspended/Banned)
- [x] Slash rates match spec per violation type
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'ReputationScore\|ReputationMessage\|ReputationChangeReason\|discipline' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P2

### CHAIN-04: Marketplace + Escrow contracts
- [x] Deploy Spore job market with 3 hiring models

**Spec** (doc 10, 12, 13): Spore job marketplace with full lifecycle (POSTED -> ASSIGNED -> IN_PROGRESS -> SUBMITTED -> SETTLED/DISPUTED/EXPIRED). Three hiring models per doc 12:
1. `RandomVRF` -- VRF selects 2 random eligible agents, lowest-load wins (Sparrow power-of-two-choices per doc 11, O(log log N) max load). Fast, ~1 block.
2. `BlindAuction` -- 3 variants (sealed-bid, Vickrey second-price, reputation-adjusted per doc 13). Commit-reveal scheme over 2 epochs. Adjusted score: `s_i = p_i * (1 + (1 - R_i))`.
3. `DirectHire` -- 1.5x premium, restricted to Protocol/Sovereign tier agents only.

Escrow per doc 10: budget deposited at posting, released on settlement, refunded on expiry. Dispute resolution per doc 20: 4-level escalation (optimistic 72h -> bond escalation -> peer jury -> governance).

**Current code**: `JobMessage` at `crates/roko-chain/src/phase2.rs:1054` with `job_id`, `posting_type: PostingType`, `domain`, `required_capabilities: u64`, `budget: u256`, `deadline_block`, `poster_passport_id`, `description_cid`. `PostingType` enum at line 301 with `Solo`, `Pair`, `Consortium`, `Collective` variants. `SparrowBid` at `crates/roko-chain/src/identity_economy_markets.rs:105`. `MarketplaceListing` at `identity_economy_identity.rs:579`. `DisputeResolution` at phase2.rs:1805 with 4 dispute levels. No Solidity contracts.

**What to change**: (1) Create `contracts/marketplace/Marketplace.sol` with `createJob()`, `assignJob()`, `submitResult()`, `settleJob()`. Storage: `mapping(bytes32 => Job)`. (2) Implement 3 hiring models as separate internal functions: `_assignRandomVRF()` using Chainlink VRF, `_assignAuction()` with commit-reveal, `_assignDirect()` with tier check. (3) Create `contracts/escrow/Escrow.sol` with `deposit(bytes32 jobId)`, `release(bytes32 jobId)`, `dispute(bytes32 jobId)`, `refund(bytes32 jobId)`. (4) Wire `DisputeResolution` struct to contract events. (5) Integration tests via mirage-rs.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:1054` -- JobMessage with job lifecycle fields
- `crates/roko-chain/src/phase2.rs:301` -- PostingType enum (Solo/Pair/Consortium/Collective)
- `crates/roko-chain/src/phase2.rs:1345` -- VRF selection types for Sparrow
- `crates/roko-chain/src/phase2.rs:1805` -- DisputeResolution with 4 escalation levels
- `crates/roko-chain/src/identity_economy_markets.rs:105` -- SparrowBid struct
- `crates/roko-chain/src/identity_economy_identity.rs:579` -- MarketplaceListing
- `docs/08-chain/10-spore-job-market.md` -- job lifecycle, fee structure, capability matching
- `docs/08-chain/11-sparrow-power-of-two-choices.md` -- VRF-based power-of-two-choices dispatch
- `docs/08-chain/12-three-hiring-models.md` -- RandomVRF, BlindAuction, DirectHire specs
- `docs/08-chain/13-vickrey-reputation-auction.md` -- adjusted score formula, commit-reveal, truthful bidding
**Depends on**: CHAIN-01, CHAIN-02, CHAIN-03
**Accept when**:
- [x] Marketplace contract supports 3 hiring models (RandomVRF, BlindAuction, DirectHire)
- [x] Job lifecycle state machine (POSTED->ASSIGNED->IN_PROGRESS->SUBMITTED->SETTLED)
- [x] Escrow contract handles deposit/release/dispute/refund
- [x] Dispute resolution supports 4 escalation levels
- [ ] Deploys to mirage-rs local fork
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'Marketplace\|Escrow\|JobMessage\|PostingType\|DisputeResolution' crates/roko-chain/src/ --include='*.rs'
ls contracts/marketplace/ contracts/escrow/ 2>/dev/null || echo "No contract dirs yet"
cargo test -p roko-chain
```
**Priority**: P2

### CHAIN-05: GossipSub integration
- [ ] Integrate GossipSub v1.1 with 8 topics and peer scoring

**Spec** (doc 07, 08, 09): 4-tier gossip architecture:
- Tier 1: GossipSub v1.1 (ms latency) -- real-time P2P mesh
- Tier 2: MiroFish simulation (sec) -- batch simulation gossip
- Tier 3: FABRIC TEE aggregation (epoch) -- privacy-preserving aggregation
- Tier 4: Canonical Event Bus (block) -- on-chain event finality

8 topics per doc 08, each with its own schema and TTL:
1. `korai/knowledge/v1` -- knowledge entry propagation (TTL: 1h)
2. `korai/reputation/v1` -- reputation score updates (TTL: 30m)
3. `korai/job/v1` -- job postings and assignments (TTL: 24h)
4. `korai/heartbeat/v1` -- agent liveness (TTL: 5m)
5. `korai/anomaly/v1` -- anomaly alerts (TTL: 1h)
6. `korai/simulation/v1` -- simulation results (TTL: 4h)
7. `korai/governance/v1` -- governance proposals (TTL: 7d)
8. `korai/peer-discovery/v1` -- peer announcements (TTL: 30m)

3-layer peer scoring per doc 09:
- Protocol layer: GossipSub mesh behavior (message delivery, duplicates, invalid messages)
- Application layer: domain-specific behavior (knowledge quality, job completion)
- Economic layer: stake-weighted scoring (KORAI staked, reputation tier)
Combined score determines mesh membership; score < -100 triggers disconnect.

Message ordering: vector clocks for causal ordering, CRDTs for state convergence (doc 07). Dandelion++ for anonymity (doc 07): stem phase (3 hops) -> fluff phase (broadcast).

**Current code**: `GossipTopic` enum at `crates/roko-chain/src/phase2.rs:312` with 8 variants. `GossipEnvelope` at line 844 with `topic`, `payload`, `sender`, `timestamp`, `signature` fields. `KnowledgeMessage` at line 997, `ReputationMessage` at line 1016, `JobMessage` at line 1054. `GossipSubScoring` at line 1250 (scaffolded). All are type definitions with no libp2p runtime integration.

**What to change**: (1) Add `libp2p` + `libp2p-gossipsub` dependency to roko-chain. (2) Create `crates/roko-chain/src/gossip.rs` with `GossipNetwork` struct wrapping `libp2p::gossipsub::Behaviour`. (3) Implement `GossipNetwork::new(config: GossipSubConfig)` initializing mesh with 8 topics. (4) Add `publish(topic: GossipTopic, payload: &[u8]) -> Result<MessageId>` and `subscribe(topic: GossipTopic) -> impl Stream<Item = GossipEnvelope>`. (5) Implement 3-layer peer scoring by composing protocol/application/economic scores into `libp2p::gossipsub::PeerScoreParams`. (6) Wire `GossipEnvelope` serialization into gossipsub message format.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:312` -- GossipTopic enum (8 topics)
- `crates/roko-chain/src/phase2.rs:844` -- GossipEnvelope with topic/payload/sender/timestamp/signature
- `crates/roko-chain/src/phase2.rs:997` -- KnowledgeMessage payload
- `crates/roko-chain/src/phase2.rs:1016` -- ReputationMessage payload
- `crates/roko-chain/src/phase2.rs:1054` -- JobMessage payload
- `crates/roko-chain/src/phase2.rs:1250` -- GossipSubScoring scaffolding
- `docs/08-chain/07-4-tier-gossip-architecture.md` -- 4-tier architecture, vector clocks, CRDTs, Dandelion++
- `docs/08-chain/08-eight-gossip-topics.md` -- 8 topics with schemas, TTL policies, subscription rules
- `docs/08-chain/09-peer-scoring-3-layer.md` -- 3-layer scoring (protocol/application/economic), Sybil resistance
**Depends on**: CHAIN-02 (agent identity for peer auth)
**Accept when**:
- [ ] GossipSub v1.1 mesh connects peers via libp2p
- [ ] 8 topics registered with correct TTL policies
- [ ] Message publish/subscribe works with GossipEnvelope serialization
- [ ] 3-layer peer scoring active (protocol + application + economic)
- [ ] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'GossipNetwork\|GossipTopic\|GossipEnvelope\|PeerScore' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P2

### CHAIN-06: HDC on-chain precompile
- [ ] Implement native EVM precompile for HDC operations

**Spec** (doc 03): Four HDC operations as EVM precompiles at address `0x00000000000000000000000000000000000A01`:
- `hdc_similarity(vec_a: bytes, vec_b: bytes) -> uint256` -- Hamming similarity of two 10,240-bit BSC vectors, returns normalized score in [0, 2^64]. Gas: ~130 (calibrated to ECRECOVER anchor: 1 gas = ~10ns, similarity takes ~1.3us per doc 03).
- `hdc_bind(vec_a: bytes, vec_b: bytes) -> bytes` -- XOR binding of two vectors. Gas: ~50.
- `hdc_bundle(vecs: bytes[]) -> bytes` -- Majority-vote bundling of N vectors. Gas: ~50 * N.
- `hdc_topk(query: bytes, index_id: uint256, k: uint8) -> (uint256[], uint256[])` -- Top-K similarity search over an on-chain index. Returns (entry_ids, scores). Gas: ~400 for K=20.

Three-tier search optimization: Bloom filter pre-screen -> approximate nearest neighbors -> exact Hamming comparison. Cross-domain resonance threshold: 0.526 (same as roko-neuro).

**Current code**: `HdcPrecompile` at `crates/roko-chain/src/phase2.rs:657` with `vector_bits: usize = 10240`, `address: String`. `PrecompileConfig` at line 230 with `address`, `gas_cost`, `name`. `HdcVector` at `crates/roko-primitives/src/hdc.rs` with all 4 operations implemented in pure Rust (bind at line 113, bundle at line 130, permute at line 150, similarity at line 223). mirage-rs uses revm but does not register custom precompiles.

**What to change**: (1) Create `crates/roko-chain/src/hdc_precompile.rs` implementing the `revm::Precompile` trait with `call(input: &Bytes, gas_limit: u64) -> PrecompileResult`. (2) Parse input bytes: first byte = opcode (0=similarity, 1=bind, 2=bundle, 3=topk), remaining bytes = arguments. (3) Delegate to `HdcVector` operations from roko-primitives. (4) Register precompile at `0xA01` in mirage-rs `MirageInstance` setup. (5) Add Solidity interface contract `IHdcPrecompile` with the 4 method signatures for type-safe calls.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:657` -- HdcPrecompile struct (vector_bits, address)
- `crates/roko-chain/src/phase2.rs:230` -- PrecompileConfig (address, gas_cost, name)
- `crates/roko-primitives/src/hdc.rs:113` -- bind() (XOR, the underlying operation)
- `crates/roko-primitives/src/hdc.rs:130` -- bundle() (majority vote)
- `crates/roko-primitives/src/hdc.rs:223` -- similarity() (Hamming)
- `apps/mirage-rs/` -- mirage-rs EVM simulator (register precompile here)
- `docs/08-chain/03-hdc-on-chain-precompile.md` -- gas cost model, Stylus WASM path, ZK proofs, Binius binary-field STARKs
**Depends on**: None
**Accept when**:
- [ ] Precompile registered at `0xA01` in mirage-rs
- [ ] `hdc_similarity`, `hdc_bind`, `hdc_bundle` callable from EVM
- [ ] Gas costs calibrated (similarity ~130, bind ~50, bundle ~50*N)
- [ ] Results match roko-primitives HdcVector operations exactly
- [ ] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'HdcPrecompile\|PrecompileConfig\|hdc_precompile\|0xA01' crates/roko-chain/src/ apps/mirage-rs/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P2

### CHAIN-07: ChainWitness full observer loop
- [x] Wire block watcher -> triage -> router -> action pipeline

**Spec** (doc 15, 16, 19): The full ChainWitness observer loop is a 4-stage pipeline that processes new blocks into actionable signals:

Stage 1 -- **Block Ingestion** (doc 15): WebSocket subscription to new block headers via `ChainClient`. Binary Fuse filter (8.7 bits/entry, <1% FPR, Lemire et al. 2022) pre-screens transactions against a set of watched addresses/topics. Gap detection via Roaring Bitmaps to catch missed blocks. Connection pool with HTTP fallback for reliability.

Stage 2 -- **Triage** (doc 16): 4-step rule-based pipeline (no LLM):
1. Rule-based filter -- match events against known contract addresses, topic hashes, and sender passports
2. MIDAS-R anomaly detection -- streaming anomaly scorer for unusual patterns (Bhatia et al. 2020)
3. Contextual enrichment -- attach passport metadata, reputation scores, and domain context to matched events
4. HDC/Bayesian curiosity scoring -- score events by information gain relative to agent's current knowledge

Stage 3 -- **Router**: Route scored events to handlers based on event type and score. High-curiosity events -> knowledge ingestion. Anomalies -> conductor alerts. Job events -> marketplace handler.

Stage 4 -- **Action**: Execute handler actions (update knowledge store, trigger conductor signals, update reputation).

**Current code**: `ChainWitnessEngine` at `crates/roko-chain/src/witness.rs:26` is a narrow attestation anchor helper only. It implements `witness_on_chain()` at line 53 (submit attestation tx) and `verify_on_chain()` at line 88 (check receipt). The module comment at line 8 explicitly states it "is an attestation anchor helper, not the broader block-observer / triage runtime." `BinaryFuse8` at phase2.rs:77 and `MidasR` at phase2.rs:126 are scaffolded types. `ChainClient` trait at `client.rs` has `get_block()` and `subscribe_blocks()` methods.

**What to change**: (1) Create `crates/roko-chain/src/observer.rs` with `BlockObserver` struct wrapping `ChainClient` for block subscription. (2) Implement Binary Fuse filter using `xorf` crate for address pre-screening. (3) Create `crates/roko-chain/src/triage.rs` with `TriagePipeline` implementing the 4-stage pipeline. (4) Add `MidasR` anomaly scoring from scaffolded type. (5) Wire observer -> triage -> handler pipeline. Keep existing `ChainWitnessEngine` attestation functionality unchanged.

**Reference files**:
- `crates/roko-chain/src/witness.rs:26` -- existing ChainWitnessEngine (attestation only, do not modify)
- `crates/roko-chain/src/phase2.rs:77` -- BinaryFuse8 scaffolded type
- `crates/roko-chain/src/phase2.rs:126` -- MidasR anomaly scorer scaffolded
- `crates/roko-chain/src/client.rs` -- ChainClient trait with get_block(), subscribe_blocks()
- `docs/08-chain/15-chainwitness-event-watching.md` -- Binary Fuse filter, WebSocket ingestion, gap detection, connection pool
- `docs/08-chain/16-triage-curiosity-midas.md` -- 4-stage triage pipeline (rule -> MIDAS-R -> enrichment -> HDC/Bayesian curiosity)
- `docs/08-chain/19-chain-agent-heartbeat.md` -- heartbeat policy within canonical 7-step loop
**Depends on**: None
**Accept when**:
- [x] `BlockObserver` subscribes to new blocks via ChainClient
- [x] Binary Fuse filter pre-screens transactions (address/topic matching)
- [x] Triage pipeline scores events by relevance and curiosity
- [x] Matched events routed to appropriate handlers
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'BlockObserver\|TriagePipeline\|BinaryFuse\|MidasR' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P2

### CHAIN-08: x402 micropayments
- [x] Implement HTTP 402 payment protocol

**Spec** (doc 20): The x402 protocol enables pay-per-request API access without accounts or API keys. Flow:
1. Client sends request to agent's HTTP endpoint
2. Agent responds with `402 Payment Required` + `X-Payment-Request` header containing: `recipient: Address`, `amount: u256`, `token: Address` (KORAI), `nonce: u256`, `deadline: u64`
3. Client signs an ERC-3009 `transferWithAuthorization(from, to, value, validAfter, validBefore, nonce, v, r, s)` -- gasless transfer where the recipient submits the tx
4. Client includes signed authorization in `X-Payment-Authorization` header on retry
5. Agent verifies authorization, submits to chain, serves response

State channels for streaming payments (per doc 20): `openChannel(counterparty, deposit)` -> micro-updates via signed balance proofs -> `closeChannel()` with final state. Reduces gas to 2 tx per session regardless of request count. 4-level dispute resolution: optimistic (72h auto-settle) -> bond escalation (2x bonds) -> peer jury (7 jurors, 66% threshold) -> governance.

**Current code**: `KnowledgeAttestation` at `crates/roko-chain/src/phase2.rs:1771` with `entry_hash`, `attester_passport_id`, `attestation_type: AttestationType`, `confidence`, `signature`, `timestamp`, `bond`. `AttestationType` enum at line 1792 with `Confirmation`, `IndependentVerification`, `UsageValidation`, `Challenge` variants. `DisputeResolution` at line 1805 with `challenger`, `defender`, `current_level: DisputeLevel`, bonds, jury. `ChainWallet` trait at `crates/roko-chain/src/wallet.rs` has `sign_and_submit()`. No HTTP middleware or ERC-3009 implementation.

**What to change**: (1) Create `crates/roko-chain/src/x402.rs` with `PaymentRequest` struct (`recipient`, `amount`, `token`, `nonce`, `deadline`) and `PaymentAuthorization` struct (ERC-3009 fields). (2) Add `X402Middleware` that intercepts HTTP responses, checks payment requirement, returns 402 with payment request header. (3) Implement `verify_authorization(auth: &PaymentAuthorization) -> bool` using ERC-3009 signature verification. (4) Add `StateChannel` struct with `open()`, `update(balance_proof)`, `close()` methods. (5) Wire `DisputeResolution` into channel close disputes.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:1771` -- KnowledgeAttestation struct
- `crates/roko-chain/src/phase2.rs:1792` -- AttestationType enum (4 variants)
- `crates/roko-chain/src/phase2.rs:1805` -- DisputeResolution with 4 escalation levels
- `crates/roko-chain/src/wallet.rs` -- ChainWallet trait for signing
- `docs/08-chain/20-x402-micropayments.md` -- HTTP 402 flow, ERC-3009 gasless transfers, state channels, streaming payments, 4-level dispute resolution
**Depends on**: CHAIN-01
**Accept when**:
- [x] HTTP 402 response includes X-Payment-Request header with correct fields
- [x] ERC-3009 authorization signing and verification works
- [x] State channel open/update/close lifecycle
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'x402\|PaymentRequest\|PaymentAuthorization\|StateChannel\|ERC3009' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P3

### CHAIN-09: ISFR clearing settlement
- [x] Implement intersubjective fact registry with QP solver

**Spec** (doc 21): The Intersubjective Fact Registry (ISFR) resolves disputed facts through reputation-weighted aggregation and quadratic programming optimization. Flow:
1. Agent submits `FactClaim` with topic, value, and stake
2. Claims aggregated per clearing epoch (default: 1h)
3. Off-chain QP solver computes optimal allocation using bisection O(80n) convergence
4. Solver submits `ClearingCertificate` with allocations, dual variables, KKT residual
5. On-chain verifier checks KKT optimality conditions (complementary slackness, stationarity)
6. If KKT residual < epsilon, clearing is finalized; otherwise, challenge period opens

Four fact topic types: `ServicePrice`, `QualityAssessment`, `OracleResolution`, `Custom`. Three value types: `Numeric(f64)`, `Boolean(bool)`, `Score(f64)`, `Price(u256)`. Reputation weighting: each claim's weight = `claim_value * reputation_score * stake^0.5` (square root to prevent plutocracy).

**Current code**: `FactTopic` enum at `crates/roko-chain/src/phase2.rs:1870` with `ServicePrice`, `QualityAssessment`, `OracleResolution`, `Custom` variants. `FactValue` enum at line 1882 with `Numeric`, `Boolean`, `Score`, `Price`. `ClearingCertificate` at line 1896 with `allocations: Vec<Allocation>`, `dual_variables: Vec<f64>`, `kkt_residual: f64`, `total_welfare: f64`, `clearing_block`, `merkle_root`. `Allocation` at line 1912 with `agent_passport_id`, `job_id`, `price`, `quality_score`. Types fully scaffolded, no solver or verification logic.

**What to change**: (1) Create `crates/roko-chain/src/isfr.rs` with `IsfrRegistry` struct. (2) Add `submit_claim(topic: FactTopic, value: FactValue, stake: u256) -> ClaimId`. (3) Implement QP solver using iterative bisection: minimize `sum(w_i * (x_i - v_i)^2)` subject to budget and stake constraints, where `w_i` is reputation-weighted. (4) Add `generate_certificate(claims: &[FactClaim]) -> ClearingCertificate` computing KKT conditions. (5) Add `verify_certificate(cert: &ClearingCertificate) -> bool` checking complementary slackness and stationarity. (6) Wire into clearing epoch scheduler.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:1870` -- FactTopic enum (4 topic types)
- `crates/roko-chain/src/phase2.rs:1882` -- FactValue enum (4 value types)
- `crates/roko-chain/src/phase2.rs:1896` -- ClearingCertificate with KKT residual
- `crates/roko-chain/src/phase2.rs:1912` -- Allocation struct
- `docs/08-chain/21-isfr-clearing-settlement.md` -- QP solver spec, bisection O(80n), KKT certificates, reputation-weighted aggregation
**Depends on**: CHAIN-03 (reputation scores for weighting)
**Accept when**:
- [x] FactClaim submission and epoch-based collection works
- [x] QP solver produces ClearingCertificate with valid KKT residual
- [x] On-chain verification of KKT optimality conditions
- [x] Reputation weighting applied with square-root stake scaling
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'IsfrRegistry\|FactTopic\|FactValue\|ClearingCertificate\|kkt_residual' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P3

### CHAIN-10: Valhalla privacy layer
- [ ] Implement P1 (encryption) and P2 (TEE) tiers

**Spec** (doc 22): 4 privacy tiers controlling data visibility on-chain:
- `P0 Public` -- data fully visible on-chain (default)
- `P1 AccessGated` -- AES-256-GCM envelope encryption. Decryption key shared via ECDH with authorized passport holders. Key rotation on membership change.
- `P2 Confidential` -- TEE (Trusted Execution Environment) processing. Data encrypted at rest and in transit, decrypted only inside TEE enclave. Attestation proves computation integrity.
- `P3 FullSealed` -- ZK proofs. Facts verified without revealing underlying data. Uses SNARK/STARK proofs for computation verification.

PSI (Private Set Intersection) for capability matching: agents can prove they have required capabilities without revealing their full capability set.

**Current code**: `AgentPassport` at `crates/roko-chain/src/phase2.rs:739` has `tee_attestation: Option<(Hash, u64)>` for TEE integration. No `PrivacyTier` enum, no encryption helpers, no TEE attestation verification logic. The `system_prompt_hash` field demonstrates the pattern of committing hashes on-chain while keeping data off-chain.

**What to change**: (1) Add `PrivacyTier` enum to `crates/roko-chain/src/phase2.rs` with `Public`, `AccessGated`, `Confidential`, `FullSealed` variants. (2) Create `crates/roko-chain/src/privacy.rs` with `PrivacyLayer` struct. (3) Implement P1: `encrypt(data: &[u8], recipients: &[Address]) -> EncryptedEnvelope` using AES-256-GCM with ECDH-derived key per recipient. `EncryptedEnvelope` contains `nonce: [u8; 12]`, `ciphertext: Vec<u8>`, `recipient_keys: Vec<(Address, EncryptedKey)>`. (4) Implement P2: `TeeAttestation` struct with `enclave_hash: [u8; 32]`, `report: Vec<u8>`, `timestamp: u64` and `verify_attestation(attestation: &TeeAttestation) -> bool` stub that checks report format. (5) Add `PrivacyTier` field to data storage structs.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:739` -- AgentPassport.tee_attestation (existing TEE field)
- `crates/roko-chain/src/types.rs` -- transaction primitives
- `crates/roko-chain/src/wallet.rs` -- ChainWallet for key operations
- `docs/08-chain/22-valhalla-privacy-layer.md` -- 4 tiers spec, PSI for capability matching, TEE attestation, ZK verification
**Depends on**: CHAIN-01
**Accept when**:
- [ ] `PrivacyTier` enum with 4 variants exists
- [ ] P1 AES-256-GCM encryption/decryption with ECDH key exchange works
- [ ] P2 TEE attestation struct and verification stub exists
- [ ] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'PrivacyTier\|PrivacyLayer\|EncryptedEnvelope\|TeeAttestation' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P3

### CHAIN-11: Knowledge futures market
- [x] Implement demand signaling and staked commitment

**Spec** (doc 23): Knowledge futures allow agents to commit to producing specific knowledge by a deadline, with staked collateral. Lifecycle state machine:
- `OPEN` -- future created with specification, reward pool, and deadline. Interested producers stake collateral.
- `COMMITTED` -- producer has staked and accepted the future. Work begins.
- `SUBMITTED` -- producer submits result with knowledge entry hash and validation evidence.
- `FULFILLED` -- result passes quality check (HDC similarity to target_hdc, min_quality threshold), stake returned + reward released.
- `EXPIRED` -- deadline passed without submission. Stake slashed, reward returned to demand pool.

Demand signaling: agents can signal demand for knowledge in a domain by depositing KORAI into a demand pool. Higher demand pools attract more producers. Early withdrawal penalty: 10% of deposited amount.

**Current code**: `KnowledgeFuture` at `crates/roko-chain/src/phase2.rs:1926` with `future_id: [u8; 32]`, `specification: KnowledgeSpec`, `producer_passport_id: u256`, `stake: u256`, `deadline_block: u64`, `reward: u256`, `state: FutureState`. `KnowledgeSpec` at line 1944 with `domain: String`, `topic: String`, `min_quality: f64`, `target_hdc: Option<[u64; 160]>`, `acceptance_criteria: Vec<AcceptanceCriterion>`. `FutureState` enum at line 1958 (variants not fully visible but scaffolded). All type definitions, no lifecycle logic.

**What to change**: (1) Create `crates/roko-chain/src/futures_market.rs` with `FuturesMarket` struct. (2) Add `create_future(spec: KnowledgeSpec, reward: u256, deadline: u64) -> FutureId`. (3) Add `commit(future_id: FutureId, stake: u256)` that locks producer's stake. (4) Add `submit(future_id: FutureId, entry_hash: [u8; 32], evidence: Vec<u8>)` that transitions to SUBMITTED. (5) Add `fulfill(future_id: FutureId)` that validates submission against `KnowledgeSpec` (HDC similarity check if `target_hdc` is set, quality >= `min_quality`), releases stake + reward. (6) Add `expire(future_id: FutureId)` callable after deadline, slashes stake, returns reward to pool.

**Reference files**:
- `crates/roko-chain/src/phase2.rs:1926` -- KnowledgeFuture struct with all fields
- `crates/roko-chain/src/phase2.rs:1944` -- KnowledgeSpec with domain, topic, min_quality, target_hdc
- `crates/roko-chain/src/phase2.rs:1958` -- FutureState enum
- `crates/roko-primitives/src/hdc.rs:223` -- similarity() for quality validation against target_hdc
- `docs/08-chain/23-knowledge-futures-market.md` -- lifecycle spec, demand signaling, staking, early withdrawal, market-making
**Depends on**: CHAIN-01 (KORAI for staking)
**Accept when**:
- [x] KnowledgeFuture lifecycle state machine works (OPEN->COMMITTED->SUBMITTED->FULFILLED/EXPIRED)
- [x] Stake deposited on commit, returned on fulfillment, slashed on expiry
- [x] Quality validation checks HDC similarity and min_quality threshold
- [x] Demand pool with early withdrawal penalty
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'KnowledgeFuture\|FuturesMarket\|KnowledgeSpec\|FutureState' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P3 (Phase 3+)

### CHAIN-12: Validation Registry contract (ERC-8004)
- [x] Deploy validation registry for work proof attestation

**Spec** (doc 06): Three on-chain registries follow ERC-8004 pattern. The Validation Registry stores work proof attestations: when an agent completes a job and the result passes gate verification, the result hash and gate scores are recorded on-chain. This provides a tamper-evident record of completed work that feeds the reputation system. Separation of concerns: Identity Registry (who), Reputation Registry (how well), Validation Registry (what was done and verified).

**Current code**: `crates/roko-chain/src/phase2.rs` contains identity and reputation types but no `ValidationRegistry` or work proof attestation types. `crates/roko-chain/src/identity_economy_identity.rs` has identity scaffolding. The 6-contract build order in doc 24 lists Validation Registry as contract #4 (after Agent Registry).

**What to change**: (1) Add `ValidationProof` struct to `crates/roko-chain/src/phase2.rs` with fields: `agent_id`, `job_id`, `result_hash`, `gate_scores: Vec<(GateKind, f64)>`, `timestamp`, `attester_id`. (2) Write Solidity Validation Registry contract with `submitProof()` and `verifyProof()`. (3) Wire into the marketplace escrow flow: proof submission triggers escrow release.

**Reference files**:
- `crates/roko-chain/src/phase2.rs` -- add ValidationProof types
- `crates/roko-chain/src/identity_economy_identity.rs` -- identity types for agent_id/attester_id
- `docs/08-chain/06-erc-8004-registries.md` -- three-registry spec, cross-registry flows, separation of concerns
- `docs/08-chain/24-current-status-and-6-contracts.md` -- build order (Validation is contract #4)
**Depends on**: CHAIN-02 (Agent Registry for agent identity references)
**Accept when**:
- [x] `ValidationProof` struct exists with required fields
- [x] Validation Registry Solidity contract compiles
- [x] Proof submission and verification work
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'ValidationProof\|ValidationRegistry\|submitProof' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P2

## Build order (per doc 24)
1. KORAI Token (no deps)
2. Agent Registry (depends on KORAI)
3. Reputation Registry (depends on Agent Registry)
4. Validation Registry (depends on Agent Registry)
5. Escrow (depends on KORAI)
6. Marketplace (depends on all above)

## Verify
```bash
cargo test -p roko-chain
cargo test -p mirage-rs  # if applicable
```
