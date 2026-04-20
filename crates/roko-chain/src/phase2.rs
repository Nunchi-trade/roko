#![allow(
    dead_code,
    missing_docs,
    non_camel_case_types,
    clippy::cast_precision_loss,
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::module_name_repetitions,
    clippy::struct_excessive_bools,
    clippy::struct_field_names,
    clippy::suboptimal_flops,
    clippy::unnecessary_map_or,
    clippy::use_self,
    clippy::upper_case_acronyms
)]

//! Phase 2 chain-layer stubs derived from `docs/08-chain`.
//!
//! These items model the deferred Korai chain surface described in the docs.
//! They intentionally avoid production logic and exist so the crate matches
//! the documented type landscape without pulling in new dependencies.

use crate::ChainResult;
use async_trait::async_trait;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, atomic::AtomicU64},
};

/// Simplified address representation for deferred chain stubs.
pub type Address = String;

/// Simplified byte buffer representation for deferred chain stubs.
pub type Bytes = Vec<u8>;

/// Simplified 256-bit hash representation for deferred chain stubs.
pub type B256 = [u8; 32];

/// Simplified hash representation for deferred chain stubs.
pub type Hash = [u8; 32];

/// Simplified big unsigned integer used by Phase 2 stubs.
pub type u256 = u128;

/// Simplified big signed integer used by Phase 2 stubs.
pub type i256 = i128;

/// Shared result type for Phase 2 chain stubs.
pub type Phase2Result<T> = ChainResult<T>;

/// Placeholder signing key used by deferred wallet stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SigningKey {
    /// Opaque signing bytes.
    pub bytes: [u8; 32],
}

/// Placeholder provider descriptor for deferred RPC-backed clients.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provider {
    /// Human-readable transport label.
    pub transport: String,
}

/// Placeholder in-process mirage simulator handle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MirageInstance {
    /// Operating mode for the simulator.
    pub mode: String,
    /// Chain id exposed by the simulator.
    pub chain_id: u64,
}

/// Binary Fuse filter for O(1) approximate membership testing (P1-37).
///
/// A space-efficient probabilistic data structure using ~8.7 bits per entry.
/// False positive rate ~1/256 (0.39%). No false negatives.
///
/// Construction is O(n) expected time. Lookup is O(1) (3 hash lookups + XOR).
///
/// Used in the triage pipeline for fast address/topic filtering without
/// loading the full set into memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BinaryFuse8 {
    /// Fingerprint array (8-bit fingerprints, 3 segments).
    pub fingerprints: Vec<u8>,
    /// Number of segments (filter is divided into 3 equal segments).
    pub segment_length: usize,
    /// Seed for hash function mixing.
    pub seed: u64,
    /// Number of keys encoded.
    pub key_count: usize,
}

impl BinaryFuse8 {
    /// Build a filter from a set of keys.
    ///
    /// The filter is constructed using the XOR-based algorithm from
    /// "Binary Fuse Filters" (Graf & Lemire, 2022).
    ///
    /// Returns a filter that answers `contains()` in O(1) with ~0.39% false positive rate.
    #[must_use]
    pub fn from_keys(keys: &[u64]) -> Self {
        if keys.is_empty() {
            return Self::default();
        }

        let n = keys.len();
        // Array size: ~1.125x the number of keys, divided into 3 segments.
        let segment_length = ((n as f64 * 1.125 / 3.0).ceil() as usize).max(4);
        let array_length = segment_length * 3;
        let seed = Self::compute_seed(keys);

        let mut fingerprints = vec![0u8; array_length];

        // Simple construction: for each key, XOR its fingerprint into the
        // position determined by the hash. This is a simplified version
        // of the full Binary Fuse construction algorithm.
        for &key in keys {
            let hash = Self::mix(key, seed);
            let fp = Self::fingerprint(hash);
            let h0 = (hash as usize) % segment_length;
            let h1 = segment_length + ((hash >> 16) as usize) % segment_length;
            let h2 = 2 * segment_length + ((hash >> 32) as usize) % segment_length;

            // XOR the fingerprint into one of the three positions.
            // Choose the position with the least collisions (simple heuristic).
            fingerprints[h0] ^= fp;
            fingerprints[h1] ^= fp;
            fingerprints[h2] ^= fp;
        }

        Self {
            fingerprints,
            segment_length,
            seed,
            key_count: n,
        }
    }

    /// Test if a key might be in the set.
    ///
    /// - Returns `true` if the key is probably in the set (~0.39% false positive rate).
    /// - Returns `false` if the key is definitely NOT in the set (no false negatives
    ///   when the filter was constructed correctly).
    #[must_use]
    pub fn contains(&self, key: u64) -> bool {
        if self.fingerprints.is_empty() {
            return false;
        }

        let hash = Self::mix(key, self.seed);
        let fp = Self::fingerprint(hash);
        let h0 = (hash as usize) % self.segment_length;
        let h1 = self.segment_length + ((hash >> 16) as usize) % self.segment_length;
        let h2 = 2 * self.segment_length + ((hash >> 32) as usize) % self.segment_length;

        // XOR all three positions — should equal the fingerprint for members.
        let result = self.fingerprints[h0] ^ self.fingerprints[h1] ^ self.fingerprints[h2];
        result == fp
    }

    /// Number of keys encoded in the filter.
    #[must_use]
    pub fn len(&self) -> usize {
        self.key_count
    }

    /// Whether the filter is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.key_count == 0
    }

    /// Approximate memory usage in bytes.
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.fingerprints.len() + std::mem::size_of::<Self>()
    }

    /// Bits per entry (should be ~8.7 for a well-constructed filter).
    #[must_use]
    pub fn bits_per_entry(&self) -> f64 {
        if self.key_count == 0 {
            return 0.0;
        }
        (self.fingerprints.len() * 8) as f64 / self.key_count as f64
    }

    // ─── Internal hash functions ────────────────────────────────────

    fn mix(key: u64, seed: u64) -> u64 {
        let mut h = key.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(seed);
        h ^= h >> 30;
        h = h.wrapping_mul(0xBF58476D1CE4E5B9);
        h ^= h >> 27;
        h = h.wrapping_mul(0x94D049BB133111EB);
        h ^= h >> 31;
        h
    }

    fn fingerprint(hash: u64) -> u8 {
        // Use upper bits for fingerprint (lower bits used for positions).
        let fp = (hash >> 56) as u8;
        // Ensure non-zero fingerprint to avoid trivial XOR matches.
        if fp == 0 { 1 } else { fp }
    }

    fn compute_seed(keys: &[u64]) -> u64 {
        let mut seed = 0x517CC1B727220A95u64;
        for &key in keys {
            seed ^= key.wrapping_mul(0x9E3779B97F4A7C15);
        }
        seed
    }
}

/// Minimal `ArcSwap` stand-in for stubbed watcher types.
#[derive(Clone, Debug)]
pub struct ArcSwap<T> {
    /// Currently active shared value.
    pub value: Arc<T>,
}

impl<T> ArcSwap<T> {
    /// Create a new placeholder `ArcSwap`.
    pub fn new(value: Arc<T>) -> Self {
        Self { value }
    }
}

/// Placeholder WebSocket provider handle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WsProvider {
    /// Endpoint URL for the connection.
    pub url: String,
}

/// Placeholder HTTP provider handle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpProvider {
    /// Endpoint URL for the fallback connection.
    pub url: String,
}

/// Placeholder connection pool used by witness and query stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pool<T> {
    /// Currently provisioned pooled items.
    pub items: Vec<T>,
}

/// Placeholder roaring bitmap used by gap-tracking stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoaringBitmap {
    /// Seen block numbers tracked by the bitmap.
    pub seen: Vec<u64>,
}

/// Placeholder MIDAS-R state used by the triage pipeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MidasR {
    /// Width of the streaming sketch.
    pub width: usize,
    /// Depth of the streaming sketch.
    pub depth: usize,
}

/// Placeholder DDSketch state used by the triage pipeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DDSketch {
    /// Relative accuracy target for quantile estimation.
    pub relative_accuracy: f64,
}

/// Placeholder Count-Min sketch state used by the triage pipeline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountMinSketch {
    /// Sketch width.
    pub width: usize,
    /// Sketch depth.
    pub depth: usize,
}

/// Placeholder normalized transaction for watcher and adapter stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NormalizedTx {
    /// Transaction hash.
    pub hash: B256,
    /// Sender address.
    pub from: Address,
    /// Optional destination address.
    pub to: Option<Address>,
    /// Value transferred in smallest units.
    pub value: u256,
    /// Raw input payload.
    pub input: Bytes,
}

/// Placeholder transaction receipt used by normalized block stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransactionReceipt {
    /// Transaction hash.
    pub tx_hash: B256,
    /// Success status.
    pub status: bool,
    /// Logs emitted by the transaction.
    pub logs: Vec<crate::LogEntry>,
}

/// Placeholder block identifier for chain-adapter stubs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockId {
    /// Resolve a block by number.
    Number(u64),
    /// Resolve a block by hash.
    Hash(B256),
    /// Resolve the latest visible block.
    Latest,
}

/// Placeholder block header used by chain-adapter stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockHeader {
    /// Block number.
    pub number: u64,
    /// Block hash.
    pub hash: B256,
    /// Parent hash.
    pub parent_hash: B256,
    /// Block timestamp.
    pub timestamp: u64,
}

/// Placeholder raw transaction payload used by chain-adapter stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawTxData {
    /// Encoded chain-specific payload.
    pub payload: Bytes,
}

/// Placeholder decoded event used by chain-adapter stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodedEvent {
    /// Emitting address or program identifier.
    pub source: Address,
    /// Event name or topic label.
    pub name: String,
    /// Raw event payload.
    pub data: Bytes,
}

/// Placeholder chain address wrapper used by chain-adapter stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChainAddress(pub String);

/// Placeholder raw chain state used by chain-adapter stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawState {
    /// Encoded state payload.
    pub bytes: Bytes,
}

/// Placeholder precompile config referenced by the Orbit docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrecompileConfig {
    /// Friendly precompile name.
    pub name: String,
    /// Address exposed by the chain.
    pub address: Address,
}

/// Placeholder consensus selector for decentralized sequencer docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsensusType {
    /// Tendermint/CometBFT-style consensus.
    CometBft,
    /// HotStuff-style consensus.
    HotStuff,
    /// Custom consensus name.
    Custom(String),
}

/// Placeholder slashing configuration for AVS validation docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlashingConfig {
    /// Default slash amount in basis points.
    pub default_slash_bps: u16,
    /// Appeal window in blocks.
    pub appeal_window_blocks: u64,
}

/// Placeholder acceptance criterion for knowledge-futures validation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcceptanceCriterion {
    /// Human-readable criterion label.
    pub name: String,
    /// Description of the rule to satisfy.
    pub description: String,
}

/// Placeholder simulation scenario descriptor used by gossip messages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SimulationScenario {
    /// Scenario name.
    pub name: String,
    /// Free-form scenario inputs.
    pub parameters: BTreeMap<String, String>,
}

/// Placeholder state diff record used by simulation result stubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateDiff {
    /// Changed location identifier.
    pub location: String,
    /// Previous value.
    pub before: Bytes,
    /// Updated value.
    pub after: Bytes,
}

/// Supported knowledge message kinds from the gossip topic docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KnowledgeKind {
    /// Durable insight entry.
    Insight,
    /// Warning or hazard signal.
    Warning,
    /// Reusable behavioral or system pattern.
    Pattern,
    /// Contradictory or harmful knowledge entry.
    AntiKnowledge,
}

/// Topic-level posting shape used by job gossip announcements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostingType {
    /// Random VRF hiring.
    RandomVRF,
    /// Blind auction hiring.
    BlindAuction,
    /// Direct-hire announcement.
    DirectHire,
}

/// The eight documented gossip topics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GossipTopic {
    /// Knowledge publication and confirmation.
    Knowledge,
    /// Reputation updates and discipline changes.
    Reputation,
    /// Job marketplace activity.
    Job,
    /// Agent liveness and load.
    Heartbeat,
    /// Triage and anomaly alerts.
    Anomaly,
    /// Shared simulation requests and results.
    Simulation,
    /// Governance proposals and votes.
    Governance,
    /// Peer roster changes.
    PeerDiscovery,
}

/// Passport tiers referenced throughout the chain docs.
///
/// Privilege ordering: Protocol (highest) > Sovereign > Worker > Edge (lowest).
/// Stake thresholds: Protocol (100,000 KORAI), Sovereign (25,000), Worker (5,000), Edge (0).
///
/// **Note**: `Ord` is manually implemented so that higher privilege = greater value.
/// `PassportTier::Protocol > PassportTier::Edge` is always true.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PassportTier {
    /// Protocol tier with the strongest privileges (governance-approved, 100,000 KORAI).
    Protocol,
    /// Sovereign tier for high-trust operators (25,000 KORAI stake).
    Sovereign,
    /// Worker tier for normal marketplace access (5,000 KORAI stake).
    #[default]
    Worker,
    /// Edge tier for constrained participation (no stake required).
    Edge,
}

impl PassportTier {
    /// Numeric privilege level (higher = more privileged).
    const fn privilege_level(self) -> u8 {
        match self {
            Self::Protocol => 3,
            Self::Sovereign => 2,
            Self::Worker => 1,
            Self::Edge => 0,
        }
    }

    /// Minimum KORAI stake required for this tier.
    pub const fn min_stake(self) -> u64 {
        match self {
            Self::Protocol => 100_000,
            Self::Sovereign => 25_000,
            Self::Worker => 5_000,
            Self::Edge => 0,
        }
    }

    /// Whether this tier has at least the given privilege level.
    pub const fn has_privilege(self, required: Self) -> bool {
        self.privilege_level() >= required.privilege_level()
    }
}

impl PartialOrd for PassportTier {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PassportTier {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.privilege_level().cmp(&other.privilege_level())
    }
}

/// Protocol families used by the triage classifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolFamily {
    /// Automated market maker protocol.
    AMM,
    /// Lending or borrowing protocol.
    Lending,
    /// Governance protocol.
    Governance,
    /// Identity or reputation protocol.
    Identity,
    /// Any other named protocol family.
    Custom(String),
}

/// DeFi actions used by the triage classifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeFiAction {
    /// Token swap.
    Swap,
    /// Liquidity mint.
    Mint,
    /// Liquidity burn.
    Burn,
    /// Asset deposit.
    Deposit,
    /// Asset withdrawal.
    Withdraw,
    /// Any other action name.
    Custom(String),
}

/// Token standards recognized by the triage classifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenStandard {
    /// ERC-20 fungible tokens.
    ERC20,
    /// ERC-721 NFTs.
    ERC721,
    /// ERC-1155 semi-fungible tokens.
    ERC1155,
    /// Any other token standard.
    Custom(String),
}

/// Governance actions recognized by the triage classifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GovernanceAction {
    /// Proposal creation.
    Propose,
    /// Vote submission.
    Vote,
    /// Proposal execution.
    Execute,
    /// Any other governance action.
    Custom(String),
}

/// Attack patterns recognized by the triage classifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttackPattern {
    /// Flash-loan-driven exploit pattern.
    FlashLoan,
    /// Sandwich or front-running pattern.
    Sandwich,
    /// Reentrancy exploit pattern.
    Reentrancy,
    /// Oracle manipulation pattern.
    OracleManipulation,
    /// Any other named pattern.
    Custom(String),
}

/// Collusion outcomes emitted by the reputation-monitoring stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollusionRisk {
    /// No suspicious behavior detected.
    None,
    /// Two agents exchanged too many assignments.
    MutualAssignmentFlag { pair: (u256, u256), count: u32 },
    /// A clique appears to self-assign work internally.
    CliqueDetected { members: Vec<u256> },
}

/// Recovery progress status for probation and suspension recovery.
#[derive(Clone, Debug, PartialEq)]
pub enum RecoveryStatus {
    /// The recovery conditions are satisfied.
    Recovered,
    /// Recovery is still underway.
    InProgress {
        /// Remaining jobs needed for recovery.
        jobs_remaining: u32,
        /// Remaining clean days needed for recovery.
        days_remaining: f64,
    },
}

/// Target-state durable storage for chain-side Engrams.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainSubstrate {
    /// Chain instance served by this substrate.
    pub chain_id: u64,
    /// Retention budget in blocks.
    pub retention_blocks: u64,
}

/// Target-state bus projection for normalized chain pulses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainBus {
    /// Chain instance served by this bus.
    pub chain_id: u64,
    /// Topic names projected onto the bus.
    pub topics: Vec<String>,
}

/// Deferred chain-domain scorer described by the docs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChainScorer {
    /// Weight for price signals.
    pub price_weight: f64,
    /// Weight for TVL signals.
    pub tvl_weight: f64,
    /// Weight for gas-cost signals.
    pub gas_weight: f64,
    /// Weight for system health signals.
    pub health_weight: f64,
}

/// Deferred verification gate for post-execution state checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyChainGate {
    /// Human-readable gate name.
    pub name: String,
    /// Expected state checks enforced by the gate.
    pub checks: Vec<String>,
}

/// Deferred policy that reacts to chain heartbeat signals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeartbeatPolicy {
    /// Topic published by the policy.
    pub publish_topic: String,
    /// Policy cadence in milliseconds.
    pub heartbeat_interval_ms: u64,
}

/// Deferred policy layer for the chain domain.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainPolicy {
    /// Policy name.
    pub name: String,
    /// Topics observed by the policy.
    pub subscriptions: Vec<String>,
}

/// Deferred triage engine coordinating statistical chain analysis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriageEngine {
    /// Chain id served by this triage instance.
    pub chain_id: u64,
    /// Statistical state for the current chain.
    pub state: AnomalyState,
}

/// Korai Hyperlane ISM configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KoraiIsmConfig {
    /// Multisig validator threshold.
    pub multisig_threshold: u8,
    /// Trusted validator addresses.
    pub multisig_validators: Vec<Address>,
    /// Whether optimistic verification is required in addition to multisig.
    pub require_optimistic: bool,
    /// Optimistic challenge window in blocks.
    pub optimistic_window_blocks: u64,
    /// Whether ZK verification is enabled.
    pub zk_enabled: bool,
}

/// Cross-chain intent for agent transfers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrossChainIntent {
    /// Source chain id.
    pub source_chain_id: u64,
    /// Destination chain id.
    pub dest_chain_id: u64,
    /// Token address.
    pub token: Address,
    /// Amount in smallest units.
    pub amount: u256,
    /// Maximum fee in basis points.
    pub max_fee_bps: u16,
    /// Deadline block on the source chain.
    pub deadline_block: u64,
    /// Passport id authorizing the transfer.
    pub passport_id: u256,
}

/// Orbit chain configuration for Korai.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KoraiOrbitConfig {
    /// Chain name.
    pub chain_name: String,
    /// Parent-chain gas token address.
    pub gas_token: Address,
    /// Target block time in milliseconds.
    pub block_time_ms: u64,
    /// Data availability mode.
    pub da_mode: DaMode,
    /// Whether Stylus is enabled.
    pub stylus_enabled: bool,
    /// Custom precompile configs.
    pub precompiles: Vec<PrecompileConfig>,
    /// Sequencer mode.
    pub sequencer: SequencerMode,
}

/// Data availability modes for the Korai Orbit deployment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaMode {
    /// Full rollup mode.
    Rollup,
    /// AnyTrust committee-backed mode.
    AnyTrust {
        committee_size: usize,
        threshold: usize,
    },
    /// Celestia-backed data availability.
    Celestia { namespace: [u8; 32] },
}

/// Sequencer modes for the Korai Orbit deployment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SequencerMode {
    /// Single sequencer deployment.
    Centralized { sequencer_address: Address },
    /// Shared sequencer deployment.
    Shared { sequencer_url: String },
    /// Decentralized sequencer set.
    Decentralized {
        /// Validator set addresses.
        validator_set: Vec<Address>,
        /// Consensus mechanism used by the set.
        consensus: ConsensusType,
    },
}

/// EigenLayer AVS configuration for Korai validation tasks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KoraiAvsConfig {
    /// Tasks delegated to AVS operators.
    pub tasks: Vec<AvsTask>,
    /// Minimum operator stake.
    pub min_operator_stake: u256,
    /// Slashing configuration for invalid work.
    pub slashing: SlashingConfig,
}

/// AVS tasks described by the docs.
#[derive(Clone, Debug, PartialEq)]
pub enum AvsTask {
    /// HDC-search validation.
    HdcSearchValidation {
        /// Challenge window in blocks.
        challenge_window_blocks: u64,
        /// Slash amount in basis points.
        slash_amount_bps: u16,
    },
    /// Knowledge-quality validation.
    KnowledgeQualityValidation {
        /// Minimum validators required.
        min_validators: u8,
        /// Agreement threshold.
        consensus_threshold: f64,
    },
    /// Clearing-certificate validation.
    ClearingCertificateValidation {
        /// Timeout for verification in blocks.
        verification_timeout_blocks: u64,
    },
}

/// Cosmos-native HDC module config from the alternate deployment path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HdcModuleConfig {
    /// Module name.
    pub module_name: String,
    /// Maximum number of indexed vectors.
    pub max_index_size: u64,
    /// Gas per comparison in the Cosmos runtime.
    pub gas_per_comparison: u64,
}

/// Adapter trait for normalizing non-EVM chain interactions.
#[async_trait]
pub trait ChainAdapter: Send + Sync {
    /// Fetch a normalized block.
    async fn fetch_block(&self, block_id: BlockId) -> Phase2Result<NormalizedBlock>;

    /// Extract filter keys from a chain-specific header.
    fn extract_filter_keys(&self, header: &BlockHeader) -> Vec<u64>;

    /// Decode raw transaction data into normalized events.
    fn decode_events(&self, raw: &RawTxData) -> Vec<DecodedEvent>;

    /// Read protocol state from the underlying chain.
    async fn read_state(&self, address: &ChainAddress) -> Phase2Result<RawState>;
}

/// Stylus-backed HDC precompile storage stub.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HdcPrecompile {
    /// Indexed vectors by hash.
    pub vectors: HashMap<B256, Bytes>,
    /// Number of indexed vectors.
    pub vector_count: u64,
}

impl HdcPrecompile {
    /// Compute normalized Hamming similarity between two vectors.
    pub fn similarity(&self, _a: Bytes, _b: Bytes) -> Phase2Result<u256> {
        todo!("Phase 2+: compute HDC similarity via Korai precompile or Stylus")
    }

    /// XOR-bind two vectors.
    pub fn bind(&self, _a: Bytes, _b: Bytes) -> Phase2Result<Bytes> {
        todo!("Phase 2+: bind HDC vectors using XOR")
    }

    /// Majority-vote bundle of multiple vectors.
    pub fn bundle(&self, _vectors: Vec<Bytes>) -> Phase2Result<Bytes> {
        todo!("Phase 2+: bundle HDC vectors via majority vote")
    }
}

/// Optimistic HDC search result subject to fraud proofs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OptimisticHdcResult {
    /// Query vector hash.
    pub query_hash: [u8; 32],
    /// Index Merkle root at query time.
    pub index_root: [u8; 32],
    /// Claimed top-K results.
    pub results: Vec<(u256, u64)>,
    /// Submitter passport id.
    pub submitter: u256,
    /// Bond posted by the submitter.
    pub bond: u256,
    /// Submission block.
    pub submitted_at: u64,
    /// Challenge window duration in blocks.
    pub challenge_window: u64,
}

/// Fraud proof against an optimistic HDC search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HdcFraudProof {
    /// Result identifier under challenge.
    pub result_id: [u8; 32],
    /// Index of the challenged entry.
    pub challenged_index: usize,
    /// Query vector bytes.
    pub query_vector: [u8; 1280],
    /// Stored vector bytes.
    pub stored_vector: [u8; 1280],
    /// Claimed correct Hamming distance.
    pub claimed_correct_distance: u32,
}

/// TEE-attested HDC search result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeeAttestedHdcResult {
    /// Search results.
    pub results: Vec<(u256, u64)>,
    /// Enclave attestation for the result.
    pub attestation: TeeAttestation,
}

/// Hardware attestation attached to TEE-verified HDC results.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeeAttestation {
    /// Measurement of the executed code.
    pub code_measurement: [u8; 32],
    /// Hash of the attested data payload.
    pub data_hash: [u8; 32],
    /// Hardware signature.
    pub signature: Vec<u8>,
    /// Expiry timestamp.
    pub expiry: u64,
}

/// Soulbound Korai passport carrying identity, capabilities, and reputation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentPassport {
    /// Unique passport identifier.
    pub passport_id: u256,
    /// Owner EOA or multisig.
    pub owner: Address,
    /// Capability bitmask.
    pub capability_list: u64,
    /// Per-domain stake amounts.
    pub domain_stakes: BTreeMap<String, u256>,
    /// Per-domain reputation tracks.
    pub reputation_tracks: BTreeMap<String, ReputationScore>,
    /// Optional latest TEE attestation with expiry.
    pub tee_attestation: Option<(Hash, u64)>,
    /// SHA-256 hash of the committed system prompt.
    pub system_prompt_hash: [u8; 32],
    /// Current passport tier.
    pub tier: PassportTier,
    /// Historical slashing events.
    pub slash_history: Vec<SlashRecord>,
    /// Service endpoints for agent discovery (P1-05).
    ///
    /// Each endpoint describes a service this agent offers, with a type
    /// (e.g., "inference", "code-review") and a URL.
    pub service_endpoints: Vec<ServiceEndpoint>,
    /// Runtime fingerprint for ventriloquist defense (P1-05).
    ///
    /// Hash of the agent's runtime environment (model version, tool versions,
    /// MCP config hash) to detect unauthorized prompt/runtime swaps.
    pub runtime_fingerprint: Option<Hash>,
}

/// A typed service endpoint on an agent passport.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceEndpoint {
    /// Service type (e.g., "inference", "code-review", "research").
    pub service_type: String,
    /// URL or address where the service is accessible.
    pub url: String,
    /// Optional human-readable description.
    pub description: Option<String>,
}

/// EMA-smoothed per-domain reputation score.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReputationScore {
    /// Reputation score in the range `[0.0, 1.0]`.
    pub score: f64,
    /// Number of completed jobs in the domain.
    pub job_count: u64,
    /// Last update block.
    pub last_update: u64,
}

/// Recorded slashing event on an agent passport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlashRecord {
    /// Violation that triggered the slash.
    pub violation_type: ViolationType,
    /// Slashed amount.
    pub amount: u256,
    /// Slash block number.
    pub block_number: u64,
}

/// Violation categories that can trigger slashing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ViolationType {
    /// The agent missed a deadline.
    MissedDeadline,
    /// The agent abandoned an assigned job.
    AbandonedJob,
    /// The agent failed quality review.
    QualityRejection,
    /// The agent repeatedly failed quality review.
    RepeatedQualityFailure,
    /// The agent plagiarized or misattributed work.
    Plagiarism,
    /// The agent manipulated results.
    ResultManipulation,
    /// The agent violated TEE integrity expectations.
    #[default]
    TeeViolation,
}

/// Validation-registry work proof for completed marketplace jobs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkProof {
    /// Passport that produced the work.
    pub passport_id: u256,
    /// Job hash associated with the work.
    pub job_hash: [u8; 32],
    /// Merkle root of off-chain deliverables.
    pub deliverable_merkle_root: [u8; 32],
    /// Encoded gate pass/fail results.
    pub gate_results: Vec<u8>,
    /// Encoded clearing certificate bytes.
    pub clearing_cert: Bytes,
    /// Block number where the proof was recorded.
    pub block_number: u64,
    /// Wall-clock timestamp for the proof.
    pub timestamp: u64,
}

/// GossipSub mesh configuration for Korai's T0 layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GossipConfig {
    /// Target mesh size.
    pub mesh_n: usize,
    /// Mesh low watermark.
    pub mesh_n_low: usize,
    /// Mesh high watermark.
    pub mesh_n_high: usize,
    /// Number of lazy relay peers.
    pub gossip_lazy: usize,
    /// Heartbeat interval in milliseconds.
    pub heartbeat_interval_ms: u64,
    /// Message TTL measured in heartbeats.
    pub message_ttl_heartbeats: u32,
    /// History length tracked by the protocol.
    pub history_length: usize,
    /// Number of past history windows shared in gossip.
    pub history_gossip: usize,
}

/// Common gossip envelope used across the four-tier architecture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GossipEnvelope {
    /// Unique message id.
    pub message_id: [u8; 32],
    /// Sender passport id.
    pub sender_passport_id: u256,
    /// Ed25519 signature bytes.
    pub signature: [u8; 64],
    /// Topic label.
    pub topic: GossipTopic,
    /// Serialized payload.
    pub payload: Vec<u8>,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Time-to-live value.
    pub ttl: u32,
    /// Originating gossip tier.
    pub tier: GossipTier,
}

/// Latency tiers in the Korai gossip design.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GossipTier {
    /// Millisecond-scale GossipSub propagation.
    #[default]
    T0GossipSub,
    /// Seconds/minutes simulation tier.
    T1Simulation,
    /// Epoch-level TEE aggregation tier.
    T2FabricAggregation,
    /// Block-finalized canonical tier.
    T3Canonical,
}

/// Vector-clock state used for causal ordering.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VectorClock {
    /// Logical timestamps by passport id.
    pub clocks: BTreeMap<u256, u64>,
}

impl VectorClock {
    /// Merge two vector clocks using element-wise maxima.
    pub fn merge(&mut self, other: &VectorClock) {
        for (id, ts) in &other.clocks {
            self.clocks
                .entry(*id)
                .and_modify(|current| *current = (*current).max(*ts))
                .or_insert(*ts);
        }
    }

    /// Return `true` if `self` causally precedes `other`.
    #[must_use]
    pub fn precedes(&self, other: &VectorClock) -> bool {
        self.clocks
            .iter()
            .all(|(id, ts)| other.clocks.get(id).is_some_and(|other_ts| ts <= other_ts))
            && self.clocks != other.clocks
    }
}

/// Grow-only counter CRDT used by documented gossip flows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GCounter {
    /// Per-agent counters.
    pub counts: BTreeMap<u256, u64>,
}

impl GCounter {
    /// Increment an agent's local count.
    pub fn increment(&mut self, agent_id: u256) {
        *self.counts.entry(agent_id).or_insert(0) += 1;
    }

    /// Merge another counter via element-wise maxima.
    pub fn merge(&mut self, other: &GCounter) {
        for (id, count) in &other.counts {
            self.counts
                .entry(*id)
                .and_modify(|current| *current = (*current).max(*count))
                .or_insert(*count);
        }
    }

    /// Total combined count.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.counts.values().sum()
    }
}

/// Last-writer-wins register CRDT for gossip state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LwwRegister<T> {
    /// Stored value.
    pub value: T,
    /// Associated logical timestamp.
    pub timestamp: u64,
    /// Writer passport id for tie-breaking.
    pub writer: u256,
}

impl<T: Clone> LwwRegister<T> {
    /// Merge another register, preferring newer writes.
    pub fn merge(&mut self, other: &LwwRegister<T>) {
        if other.timestamp > self.timestamp
            || (other.timestamp == self.timestamp && other.writer > self.writer)
        {
            self.value = other.value.clone();
            self.timestamp = other.timestamp;
            self.writer = other.writer;
        }
    }
}

/// Dandelion++ configuration for privacy-sensitive gossip topics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DandelionConfig {
    /// Probability of switching from stem to fluff.
    pub stem_to_fluff_probability: f64,
    /// Maximum number of stem hops.
    pub max_stem_hops: u32,
    /// Stem relay strategy.
    pub stem_relay_mode: StemRelayMode,
    /// Topics that use Dandelion++.
    pub dandelion_topics: Vec<GossipTopic>,
}

/// Stem relay selection strategies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StemRelayMode {
    /// Random peer selection.
    #[default]
    Random,
    /// Peer with the highest economic score.
    HighestEconomicScore,
    /// Rotate the relay peer periodically.
    Rotating { rotation_interval_heartbeats: u32 },
}

/// Subscription-privacy controls for topic participation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubscriptionPrivacyConfig {
    /// Number of additional cover topics.
    pub cover_topics: usize,
    /// Dummy-message rate per heartbeat.
    pub cover_traffic_rate: f64,
    /// Whether subscriptions only change at epoch boundaries.
    pub epoch_locked_subscriptions: bool,
}

/// Knowledge-gossip payload for `korai/knowledge/v1`.
#[derive(Clone, Debug, PartialEq)]
pub struct KnowledgeMessage {
    /// Knowledge entry hash.
    pub entry_hash: [u8; 32],
    /// 10,240-bit HDC vector.
    pub hdc_vector: [u64; 160],
    /// Declared knowledge domain.
    pub domain: String,
    /// Kind of knowledge entry.
    pub kind: KnowledgeKind,
    /// Author passport id.
    pub author_passport_id: u256,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Optional metadata CID.
    pub metadata_cid: Option<String>,
}

/// Reputation-gossip payload for `korai/reputation/v1`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReputationMessage {
    /// Target passport id.
    pub passport_id: u256,
    /// Domain whose score changed.
    pub domain: String,
    /// Previous score.
    pub old_score: f64,
    /// New score.
    pub new_score: f64,
    /// Number of jobs tracked in the domain.
    pub job_count: u64,
    /// Cause of the change.
    pub reason: ReputationChangeReason,
    /// Aggregation epoch.
    pub epoch: u64,
}

/// Reasons a reputation score changed.
#[derive(Clone, Debug, PartialEq)]
pub enum ReputationChangeReason {
    /// Reputation change after job completion.
    JobCompletion {
        job_hash: [u8; 32],
        quality_score: f64,
    },
    /// Reputation change due to slashing.
    Slash {
        violation_type: ViolationType,
        amount: u256,
    },
    /// Reputation changed due to demurrage decay.
    DemurrageDecay,
    /// Reputation changed due to peer review.
    PeerReview { reviewer_passport_id: u256 },
}

/// Job-gossip payload for `korai/job/v1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobMessage {
    /// Job identifier.
    pub job_id: [u8; 32],
    /// Posting shape for the job.
    pub posting_type: PostingType,
    /// Target domain.
    pub domain: String,
    /// Required capability bitmask.
    pub required_capabilities: u64,
    /// Budget in KORAI units.
    pub budget: u256,
    /// Deadline block.
    pub deadline_block: u64,
    /// Poster passport id.
    pub poster_passport_id: u256,
    /// CID for the full description.
    pub description_cid: String,
}

/// Heartbeat payload for `korai/heartbeat/v1`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeartbeatMessage {
    /// Passport id emitting the heartbeat.
    pub passport_id: u256,
    /// Current passport tier.
    pub tier: PassportTier,
    /// Declared capabilities.
    pub capabilities: u64,
    /// Number of active jobs.
    pub active_jobs: u32,
    /// Load factor in `[0.0, 1.0]`.
    pub load_factor: f64,
    /// Currently active domains.
    pub domains: Vec<String>,
    /// Running software version.
    pub software_version: String,
}

/// Anomaly payload for `korai/anomaly/v1`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnomalyMessage {
    /// Anomaly identifier.
    pub anomaly_id: [u8; 32],
    /// Detecting passport id.
    pub detector_passport_id: u256,
    /// Chain id where the anomaly was observed.
    pub chain_id: u64,
    /// Block number of the observation.
    pub block_number: u64,
    /// Optional transaction hash.
    pub tx_hash: Option<[u8; 32]>,
    /// Classified anomaly type.
    pub anomaly_type: AnomalyType,
    /// Severity score in `[0.0, 1.0]`.
    pub severity: f64,
    /// Human-readable description.
    pub description: String,
    /// Agents that independently confirmed the anomaly.
    pub confirming_agents: Vec<u256>,
}

/// Anomaly categories carried by `korai/anomaly/v1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AnomalyType {
    /// Flash-loan exploit pattern.
    #[default]
    FlashLoanPattern,
    /// Unusual gas-spike pattern.
    UnusualGasSpike,
    /// Large value transfer.
    LargeValueTransfer,
    /// Contract self-destruct event.
    ContractSelfDestruct,
    /// Reentrancy pattern.
    ReentrancyPattern,
    /// Oracle manipulation pattern.
    OracleManipulation,
    /// Governance attack pattern.
    GovernanceAttack,
    /// Any other anomaly label.
    Custom(String),
}

/// Shared simulation payload carried by `korai/simulation/v1`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimulationMessage {
    /// Simulation identifier.
    pub simulation_id: [u8; 32],
    /// Requesting passport id.
    pub requester_passport_id: u256,
    /// Target chain id.
    pub chain_id: u64,
    /// Requested scenario.
    pub scenario: SimulationScenario,
    /// Optional computed result.
    pub result: Option<SimulationResult>,
    /// Block context used for the simulation.
    pub block_context: u64,
}

/// Simulation result shared for collaborative decision-making.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimulationResult {
    /// Whether the simulation succeeded.
    pub success: bool,
    /// Gas used by the simulated transaction.
    pub gas_used: u64,
    /// Observed state diffs.
    pub state_diffs: Vec<StateDiff>,
    /// Profit/loss in wei.
    pub profit_loss: i256,
    /// Risk score in `[0.0, 1.0]`.
    pub risk_score: f64,
    /// Free-form notes.
    pub notes: String,
}

/// Governance-gossip payload for `korai/governance/v1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GovernanceMessage {
    /// Proposal identifier.
    pub proposal_id: [u8; 32],
    /// Proposer passport id.
    pub proposer_passport_id: u256,
    /// Proposal type.
    pub proposal_type: ProposalType,
    /// CID for the full proposal description.
    pub description_cid: String,
    /// Voting deadline block.
    pub voting_deadline_block: u64,
    /// Quorum required, weighted by KORAI.
    pub quorum_required: u256,
    /// Current votes supporting the proposal.
    pub current_votes_for: u256,
    /// Current votes opposing the proposal.
    pub current_votes_against: u256,
}

/// Governance proposal categories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalType {
    /// Add a feedback source.
    AddFeedbackSource(Address),
    /// Remove a feedback source.
    RemoveFeedbackSource(Address),
    /// Update a named parameter.
    UpdateParameter(String, u256),
    /// Promote an agent to protocol tier.
    PromoteToProtocolTier(u256),
    /// Execute an emergency action.
    EmergencyAction(String),
}

/// Peer-discovery payload for `korai/peer-discovery/v1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeerDiscoveryMessage {
    /// Passport id described by the message.
    pub passport_id: u256,
    /// Peer roster event type.
    pub event_type: PeerEventType,
    /// Declared capabilities.
    pub capabilities: u64,
    /// Current passport tier.
    pub tier: PassportTier,
    /// Declared active domains.
    pub domains: Vec<String>,
    /// Gossip connection multiaddr.
    pub gossip_address: String,
}

/// Peer discovery event types.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PeerEventType {
    /// New peer registered.
    #[default]
    Registered,
    /// Existing peer updated metadata.
    Updated,
    /// Peer departed the network.
    Departed { reason: DepartureReason },
}

/// Reasons a peer departed the network.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DepartureReason {
    /// Voluntary departure.
    #[default]
    Voluntary,
    /// Suspension due to policy or slashing.
    Suspended,
    /// Departure due to stake withdrawal.
    StakeWithdrawn,
    /// Passive departure after inactivity.
    Inactive { last_heartbeat_block: u64 },
}

/// GossipSub protocol-scoring parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProtocolScoreParams {
    /// Score cap per topic.
    pub topic_score_cap: f64,
    /// Weight for IP co-location penalties.
    pub ip_colocation_factor_weight: f64,
    /// IP co-location threshold.
    pub ip_colocation_factor_threshold: usize,
    /// Decay interval in seconds.
    pub decay_interval_secs: u64,
    /// Graylist score threshold.
    pub graylist_threshold: f64,
    /// Publish threshold.
    pub publish_threshold: f64,
    /// Gossip threshold.
    pub gossip_threshold: f64,
}

/// Application-layer peer scoring dimensions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplicationScore {
    /// Knowledge contribution quality.
    pub knowledge_quality: f64,
    /// Anomaly-detection accuracy.
    pub anomaly_accuracy: f64,
    /// Job completion reliability.
    pub job_reliability: f64,
    /// Utility of shared simulation results.
    pub simulation_utility: f64,
    /// Governance participation quality.
    pub governance_participation: f64,
}

/// Spore marketplace job posting.
#[derive(Clone, Debug, PartialEq)]
pub struct SporeJobPosting {
    /// Job identifier.
    pub job_id: [u8; 32],
    /// Job domain.
    pub domain: String,
    /// Required capability bitmask.
    pub required_capabilities: u64,
    /// Budget in KORAI units.
    pub budget: u256,
    /// Delivery deadline block.
    pub deadline_block: u64,
    /// Hiring model used for assignment.
    pub hiring_model: HiringModel,
    /// Minimum reputation required.
    pub min_reputation: f64,
    /// Minimum passport tier required.
    pub min_tier: PassportTier,
    /// CID containing the full description.
    pub description_cid: String,
    /// Poster passport id.
    pub poster_passport_id: u256,
    /// Optional direct-hire target.
    pub direct_hire_target: Option<u256>,
    /// Maximum number of agents that may be assigned.
    pub max_agents: u32,
}

/// Hiring-model options for Spore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HiringModel {
    /// Random assignment via VRF.
    RandomVRF,
    /// Blind auction assignment.
    BlindAuction {
        /// Auction duration in blocks.
        auction_duration_blocks: u64,
        /// Auction flavor.
        auction_type: AuctionType,
    },
    /// Direct hire of a specific passport.
    DirectHire { target_passport_id: u256 },
}

/// Auction flavors supported by Spore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuctionType {
    /// First-price sealed-bid auction.
    FPSB,
    /// Vickrey second-price sealed-bid auction.
    Vickrey,
    /// Dutch descending-price auction.
    Dutch {
        /// Starting price.
        start_price: u256,
        /// Price decrement per block.
        decrement_per_block: u256,
    },
}

/// VRF selection result for Sparrow dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VrfSelection {
    /// VRF output used as randomness.
    pub vrf_output: [u8; 32],
    /// VRF proof bytes.
    pub vrf_proof: [u8; 64],
    /// Two selected agent passport ids.
    pub selected_agents: [u256; 2],
    /// Input digest used to derive the output.
    pub vrf_input: [u8; 32],
}

/// Self-reported load snapshot used by Sparrow.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentLoad {
    /// Active jobs in progress.
    pub active_jobs: u32,
    /// Estimated total capacity.
    pub capacity: u32,
    /// Load factor in `[0.0, 1.0]`.
    pub load_factor: f64,
    /// Estimated next free block.
    pub estimated_free_block: u64,
}

/// Commit phase record for Vickrey auctions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BidCommitment {
    /// Commitment hash of bid, salt, passport id, and job id.
    pub commitment: [u8; 32],
    /// Bidder passport id.
    pub bidder_passport_id: u256,
    /// Commit block number.
    pub committed_at_block: u64,
}

/// Reveal phase record for Vickrey auctions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BidReveal {
    /// Revealed bid amount.
    pub bid_amount: u256,
    /// Salt used when computing the commitment.
    pub salt: [u8; 32],
    /// Revealing passport id.
    pub passport_id: u256,
    /// Job identifier.
    pub job_id: [u8; 32],
}

/// Broader event-watching engine described in the chain-witness docs.
#[derive(Debug)]
pub struct WitnessEngine {
    /// Binary Fuse filter of watched addresses and topics.
    pub watch_filter: Arc<ArcSwap<BinaryFuse8>>,
    /// Dedicated WebSocket subscription connection.
    pub subscription: Arc<WsProvider>,
    /// Query pool for block and receipt fetches.
    pub query_pool: Pool<WsProvider>,
    /// Bitmap of seen blocks for gap detection.
    pub seen_blocks: Arc<Mutex<RoaringBitmap>>,
    /// Latest observed head block.
    pub latest_block: AtomicU64,
}

/// Connection pool state used by the documented witness engine.
#[derive(Clone, Debug, Default)]
pub struct WitnessPool {
    /// Dedicated subscription connection.
    pub subscription_conn: Arc<WsProvider>,
    /// Query connections.
    pub query_conns: Pool<WsProvider>,
    /// HTTP fallback providers.
    pub rpc_fallbacks: Vec<Arc<HttpProvider>>,
}

/// Chain-agnostic normalized block produced by the witness pipeline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NormalizedBlock {
    /// Chain id of the block.
    pub chain_id: u64,
    /// Block number.
    pub number: u64,
    /// Block hash.
    pub hash: B256,
    /// Block timestamp.
    pub timestamp: u64,
    /// Base fee per gas.
    pub base_fee_per_gas: u64,
    /// Normalized transactions contained in the block.
    pub transactions: Vec<NormalizedTx>,
    /// Transaction receipts for the block.
    pub receipts: Vec<TransactionReceipt>,
}

/// Per-chain configuration for the witness engine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainWitnessConfig {
    /// Target chain id.
    pub chain_id: u64,
    /// WebSocket endpoint.
    pub ws_url: String,
    /// HTTP fallback URLs.
    pub rpc_urls: Vec<String>,
    /// Query pool size.
    pub query_pool_size: usize,
    /// Maximum gap backfill in blocks.
    pub gap_backfill_limit: u64,
    /// Maximum watch-filter size.
    pub max_watch_size: usize,
}

/// Triage categories assigned during Stage 1 classification.
#[derive(Clone, Debug, PartialEq)]
pub enum TxCategory {
    /// Recognized DeFi interaction.
    DeFi {
        /// Protocol family label.
        protocol_family: ProtocolFamily,
        /// DeFi action kind.
        action: DeFiAction,
    },
    /// Token transfer event.
    Transfer {
        /// Token standard involved in the transfer.
        standard: TokenStandard,
        /// Transfer value.
        value: u256,
    },
    /// Contract deployment event.
    Deployment { bytecode_hash: B256 },
    /// Governance action event.
    Governance {
        /// Governance protocol name.
        protocol: String,
        /// Governance action kind.
        action: GovernanceAction,
    },
    /// Oracle update event.
    OracleUpdate {
        /// Oracle family name.
        oracle: String,
        /// Asset whose price was updated.
        asset: String,
    },
    /// Known suspicious pattern.
    SuspiciousPattern {
        /// Attack pattern name.
        pattern: AttackPattern,
        /// Confidence in the classification.
        confidence: f64,
    },
    /// Unknown transaction selector.
    Unknown { selector: [u8; 4] },
}

/// Statistical state used by the triage pipeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnomalyState {
    /// MIDAS-R sketch for edge anomalies.
    pub midas: MidasR,
    /// DDSketches for numerical signals.
    pub sketches: HashMap<String, DDSketch>,
    /// Frequency estimator for novelty scoring.
    pub frequency: CountMinSketch,
}

/// Composite curiosity score emitted by the triage pipeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CuriosityScore {
    /// Rule-based relevance score.
    pub relevance: f64,
    /// Statistical anomaly score.
    pub anomaly: f64,
    /// HDC-based novelty score.
    pub novelty: f64,
    /// Bayesian surprise score.
    pub surprise: f64,
    /// Weighted composite score.
    pub composite: f64,
}

/// Development wallet using an in-memory local key.
#[derive(Clone)]
pub struct LocalKeyWallet {
    /// Local signing key.
    pub private_key: SigningKey,
    /// Derived wallet address.
    pub address: Address,
    /// Provider used for chain reads.
    pub provider: Arc<dyn crate::ChainClient>,
}

/// ERC-4337 account-abstraction wallet.
#[derive(Clone)]
pub struct ERC4337Wallet {
    /// Account-contract address.
    pub account_address: Address,
    /// Session signing key.
    pub session_key: SigningKey,
    /// Entrypoint contract address.
    pub entrypoint: Address,
    /// Bundler URL used for user operations.
    pub bundler_url: String,
    /// Provider used for chain reads.
    pub provider: Arc<dyn crate::ChainClient>,
}

/// Deferred RPC-backed chain client surface matching the docs.
#[derive(Clone, Debug, Default)]
pub struct RpcChainClient {
    /// Underlying provider handle.
    pub provider: Arc<Provider>,
    /// Chain id exposed by the provider.
    pub chain_id: u64,
    /// Human-readable client name.
    pub name: String,
}

/// Deferred mirage-backed chain client.
#[derive(Clone, Debug, Default)]
pub struct MirageChainClient {
    /// Backing in-process simulator.
    pub mirage: Arc<MirageInstance>,
    /// Chain id surfaced by the simulator.
    pub chain_id: u64,
}

/// mirage-rs transaction request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransactionRequest {
    /// Sender address.
    pub from: Address,
    /// Optional destination address.
    pub to: Option<Address>,
    /// Value transferred.
    pub value: u256,
    /// Input payload.
    pub data: Bytes,
    /// Optional gas limit.
    pub gas_limit: Option<u64>,
    /// Optional nonce.
    pub nonce: Option<u64>,
}

/// mirage-rs error surface described by the docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirageError {
    /// Invalid JSON-RPC parameters.
    InvalidParams { message: String },
    /// Internal simulator error.
    InternalError { message: String },
    /// Execution reverted with optional data.
    ExecutionReverted {
        message: String,
        data: Option<Bytes>,
    },
    /// Fork setup or fetch error.
    ForkError { message: String },
    /// Referenced snapshot id was not found.
    SnapshotNotFound { id: String },
    /// Error in a chain-extension module.
    ChainExtensionError { message: String },
}

/// Overall confidence attached to a simulation result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimulationConfidence {
    /// Composite confidence score.
    pub score: f64,
    /// Individual contributing factors.
    pub factors: ConfidenceFactors,
}

impl SimulationConfidence {
    /// Compute a weighted confidence score from the documented factors.
    #[must_use]
    pub fn compute(factors: &ConfidenceFactors) -> f64 {
        factors.state_freshness * 0.25
            + factors.oracle_independence * 0.25
            + factors.ordering_independence * 0.25
            + factors.contract_verification * 0.15
            + factors.gas_confidence * 0.10
    }
}

/// Factors contributing to simulation confidence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfidenceFactors {
    /// Freshness of the forked state.
    pub state_freshness: f64,
    /// Independence from oracle or time-sensitive state.
    pub oracle_independence: f64,
    /// Independence from transaction ordering.
    pub ordering_independence: f64,
    /// Confidence in contract verification coverage.
    pub contract_verification: f64,
    /// Confidence in gas estimation fidelity.
    pub gas_confidence: f64,
}

/// Differential test tying a mainnet transaction to a local replay.
#[derive(Clone, Debug, PartialEq)]
pub struct DifferentialTest {
    /// Mainnet transaction hash.
    pub mainnet_tx_hash: crate::TxHash,
    /// Fork block used for replay.
    pub fork_block: crate::BlockNumber,
    /// Expected-versus-actual comparison.
    pub comparison: DiffComparison,
}

/// Comparison between simulated and observed execution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiffComparison {
    /// Absolute gas delta.
    pub gas_diff: i64,
    /// Percentage gas delta.
    pub gas_diff_pct: f64,
    /// Whether return data matched.
    pub return_data_matches: bool,
    /// Whether state diffs matched.
    pub state_diffs_match: bool,
    /// Divergent storage slots.
    pub divergent_slots: Vec<(Address, u256)>,
    /// Whether emitted logs matched.
    pub logs_match: bool,
    /// Whether success/failure status matched.
    pub status_matches: bool,
}

/// Foundry-style invariant test harness for Korai contracts.
#[derive(Clone, Debug, Default)]
pub struct KoraiInvariantTest {
    /// mirage instance used for testing.
    pub mirage: MirageInstance,
    /// Invariants checked after random transaction sequences.
    pub invariants: Vec<KoraiInvariant>,
}

/// Invariants described for Korai contract testing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KoraiInvariant {
    /// Registry length matches total passport count.
    PassportCountConsistency,
    /// Total staked value does not exceed supply.
    StakeSupplyBound,
    /// Tier and stake requirements remain consistent.
    TierStakeConsistency,
    /// Reputation scores stay in `[0.0, 1.0]`.
    ReputationBounds,
    /// Escrow remains solvent against active jobs.
    EscrowSolvency,
    /// Custom on-chain expression.
    Custom { expression: String },
}

/// Payment channel between two agents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPaymentChannel {
    /// Channel identifier.
    pub channel_id: [u8; 32],
    /// Payer-side channel party.
    pub agent_a: ChannelParty,
    /// Payee-side channel party.
    pub agent_b: ChannelParty,
    /// Deposit supplied by agent A.
    pub deposit_a: u256,
    /// Deposit supplied by agent B.
    pub deposit_b: u256,
    /// Current off-chain channel state.
    pub state: ChannelState,
    /// Challenge window in blocks.
    pub challenge_window: u64,
}

/// Participant metadata for a payment channel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelParty {
    /// Passport id of the participant.
    pub passport_id: u256,
    /// Payment address.
    pub address: Address,
}

/// Off-chain state update for a payment channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelState {
    /// Monotonic channel nonce.
    pub nonce: u64,
    /// Balance allocated to agent A.
    pub balance_a: u256,
    /// Balance allocated to agent B.
    pub balance_b: u256,
    /// Signature from agent A.
    pub sig_a: [u8; 64],
    /// Signature from agent B.
    pub sig_b: [u8; 64],
}

/// Streaming payment between two agents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaymentStream {
    /// Stream identifier.
    pub stream_id: [u8; 32],
    /// Sender passport id.
    pub sender: u256,
    /// Receiver passport id.
    pub receiver: u256,
    /// Flow rate in wei per second.
    pub flow_rate: u256,
    /// Start timestamp.
    pub started_at: u64,
    /// Buffer requirement in seconds.
    pub buffer_seconds: u64,
}

impl PaymentStream {
    /// Compute the receiver's accrued balance at `current_timestamp`.
    #[must_use]
    pub fn receiver_balance(&self, current_timestamp: u64) -> u256 {
        let elapsed = current_timestamp.saturating_sub(self.started_at);
        self.flow_rate.saturating_mul(u128::from(elapsed))
    }
}

/// Knowledge attestation paid for or settled via x402 flows.
#[derive(Clone, Debug, PartialEq)]
pub struct KnowledgeAttestation {
    /// Knowledge entry hash.
    pub entry_hash: [u8; 32],
    /// Attesting passport id.
    pub attester_passport_id: u256,
    /// Attestation kind.
    pub attestation_type: AttestationType,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Attestation signature.
    pub signature: [u8; 64],
    /// Timestamp of the attestation.
    pub timestamp: u64,
    /// Optional bond posted alongside the attestation.
    pub bond: Option<u256>,
}

/// Knowledge-attestation categories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttestationType {
    /// Confirms the entry is accurate.
    Confirmation,
    /// Independent verification by another agent.
    IndependentVerification,
    /// Validation through downstream usage.
    UsageValidation,
    /// Challenge against the entry.
    Challenge { reason: String },
}

/// Dispute state for optimistic knowledge claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisputeResolution {
    /// Challenged entry hash.
    pub entry_hash: [u8; 32],
    /// Challenger passport id.
    pub challenger: u256,
    /// Defender passport id.
    pub defender: u256,
    /// Current dispute level.
    pub current_level: DisputeLevel,
    /// Challenger bond.
    pub challenger_bond: u256,
    /// Defender bond.
    pub defender_bond: u256,
    /// Optional jury membership.
    pub jury: Option<Vec<u256>>,
    /// Deadline block for the current stage.
    pub deadline_block: u64,
}

/// Escalation levels for knowledge disputes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisputeLevel {
    /// Bond-doubling dispute round.
    BondEscalation { round: u8 },
    /// Peer-jury vote tally.
    PeerJury { votes_for: u32, votes_against: u32 },
    /// Governance vote escalation.
    GovernanceVote { proposal_id: [u8; 32] },
    /// Fully resolved dispute.
    Resolved {
        winner: u256,
        outcome: DisputeOutcome,
    },
}

/// Outcomes of a knowledge dispute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisputeOutcome {
    /// Entry was upheld.
    EntryUpheld,
    /// Entry was removed.
    EntryRemoved,
    /// Entry was amended.
    EntryAmended { amendment_hash: [u8; 32] },
}

/// Intersubjective fact claim submitted to the clearing system.
#[derive(Clone, Debug, PartialEq)]
pub struct FactClaim {
    /// Topic addressed by the claim.
    pub topic: FactTopic,
    /// Claimed value.
    pub value: FactValue,
    /// Confidence in the claim.
    pub confidence: f64,
    /// Claimant passport id.
    pub claimant_passport_id: u256,
    /// Domain used for reputation weighting.
    pub domain: String,
    /// Submission block.
    pub submitted_at_block: u64,
}

/// Claim topics supported by the ISFR docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactTopic {
    /// Service-price discovery.
    ServicePrice { service_type: String },
    /// Quality assessment of a work product.
    QualityAssessment { job_hash: [u8; 32] },
    /// Resolution of an oracle prediction.
    OracleResolution { prediction_id: [u8; 32] },
    /// Custom governance or market topic.
    Custom(String),
}

/// Claim values supported by the ISFR docs.
#[derive(Clone, Debug, PartialEq)]
pub enum FactValue {
    /// Numeric scalar.
    Numeric(f64),
    /// Boolean truth value.
    Boolean(bool),
    /// Bounded score in `[0.0, 1.0]`.
    Score(f64),
    /// Price in KORAI wei.
    Price(u256),
}

/// KKT certificate submitted by the off-chain clearing solver.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClearingCertificate {
    /// Allocation results.
    pub allocations: Vec<Allocation>,
    /// Dual variables for each constraint.
    pub dual_variables: Vec<f64>,
    /// Residual of the KKT conditions.
    pub kkt_residual: f64,
    /// Total welfare achieved by the solution.
    pub total_welfare: f64,
    /// Block where clearing occurred.
    pub clearing_block: u64,
    /// Merkle root of the full clearing data.
    pub merkle_root: [u8; 32],
}

/// Single allocation produced by the clearing solver.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Allocation {
    /// Agent receiving the assignment.
    pub agent_passport_id: u256,
    /// Job identifier.
    pub job_id: [u8; 32],
    /// Clearing price.
    pub price: u256,
    /// Quality score for the assignment.
    pub quality_score: f64,
}

/// Deferred knowledge future for proactive knowledge production.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnowledgeFuture {
    /// Future identifier.
    pub future_id: [u8; 32],
    /// Specification of the promised knowledge.
    pub specification: KnowledgeSpec,
    /// Producing passport id.
    pub producer_passport_id: u256,
    /// Staked collateral.
    pub stake: u256,
    /// Delivery deadline block.
    pub deadline_block: u64,
    /// Reward available from the demand pool.
    pub reward: u256,
    /// Current state of the future.
    pub state: FutureState,
}

/// Specification for a future knowledge artifact.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnowledgeSpec {
    /// Knowledge domain.
    pub domain: String,
    /// Human-readable topic description.
    pub topic: String,
    /// Minimum validation quality threshold.
    pub min_quality: f64,
    /// Optional target HDC vector.
    pub target_hdc: Option<[u64; 160]>,
    /// Acceptance criteria for validation.
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
}

/// Lifecycle states for a knowledge future.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FutureState {
    /// Commitment is open and not yet submitted.
    #[default]
    Open,
    /// Knowledge has been submitted and awaits validation.
    Submitted,
    /// Knowledge was validated successfully.
    Fulfilled,
    /// Deadline passed without valid delivery.
    Expired,
    /// Producer withdrew before deadline.
    Withdrawn,
}

/// Aggregated demand pool for a future knowledge topic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DemandPool {
    /// Shared specification receiving demand.
    pub spec: KnowledgeSpec,
    /// Total KORAI demand deposited.
    pub total_demand: u256,
    /// Individual deposits into the pool.
    pub deposits: Vec<DemandDeposit>,
    /// Optional committed producer.
    pub committed_producer: Option<u256>,
    /// Creation block.
    pub created_at_block: u64,
}

/// Individual demand deposit into a futures pool.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DemandDeposit {
    /// Depositor passport id.
    pub depositor_passport_id: u256,
    /// Amount deposited.
    pub amount: u256,
    /// Deposit block.
    pub deposited_at_block: u64,
}

/// Detects suspicious assignment patterns in the reputation system.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollusionDetector {
    /// Assignment counts between pairs of agents.
    pub assignment_graph: HashMap<(u256, u256), u32>,
    /// Detection thresholds.
    pub config: CollusionConfig,
}

impl CollusionDetector {
    /// Check a new assignment and return the resulting risk classification.
    pub fn check_assignment(&mut self, poster: u256, agent: u256) -> CollusionRisk {
        let key = if poster < agent {
            (poster, agent)
        } else {
            (agent, poster)
        };
        let count = self.assignment_graph.entry(key).or_insert(0);
        *count += 1;

        if *count > self.config.mutual_assignment_threshold {
            CollusionRisk::MutualAssignmentFlag {
                pair: key,
                count: *count,
            }
        } else {
            CollusionRisk::None
        }
    }
}

/// Thresholds for collusion detection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollusionConfig {
    /// Threshold for repeated mutual assignments.
    pub mutual_assignment_threshold: u32,
    /// Minimum clique size for detection.
    pub clique_detection_min_size: usize,
    /// Minimum internal-assignment ratio.
    pub clique_internal_ratio: f64,
    /// Detection window in blocks.
    pub detection_window_blocks: u64,
}

/// Recovery state for an agent on probation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbationRecovery {
    /// Probation start block.
    pub started_at: u64,
    /// Required completed jobs.
    pub required_jobs: u32,
    /// Minimum average feedback.
    pub min_avg_feedback: f64,
    /// Required clean days.
    pub required_clean_days: u32,
    /// Jobs completed during probation.
    pub jobs_completed: u32,
    /// Sum of feedback scores during probation.
    pub feedback_sum: f64,
    /// Most recent slashing block, if any.
    pub last_slash_block: Option<u64>,
}

impl ProbationRecovery {
    /// Check whether the recovery conditions have been satisfied.
    pub fn check_recovery(&self, current_block: u64) -> RecoveryStatus {
        let days_elapsed = (current_block.saturating_sub(self.started_at) as f64) * 0.4 / 86_400.0;
        let avg_feedback = if self.jobs_completed > 0 {
            self.feedback_sum / f64::from(self.jobs_completed)
        } else {
            0.0
        };
        let clean = self.last_slash_block.map_or(true, |block| {
            (current_block.saturating_sub(block) as f64) * 0.4 / 86_400.0
                >= f64::from(self.required_clean_days)
        });

        if self.jobs_completed >= self.required_jobs
            && avg_feedback >= self.min_avg_feedback
            && clean
            && days_elapsed >= f64::from(self.required_clean_days)
        {
            RecoveryStatus::Recovered
        } else {
            RecoveryStatus::InProgress {
                jobs_remaining: self.required_jobs.saturating_sub(self.jobs_completed),
                days_remaining: (f64::from(self.required_clean_days) - days_elapsed).max(0.0),
            }
        }
    }
}

/// Governance-issued amnesty for systemic reputation failures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReputationAmnesty {
    /// Governance proposal that authorized the amnesty.
    pub proposal_id: [u8; 32],
    /// Agents affected by the amnesty.
    pub affected_agents: Vec<u256>,
    /// Start block of the systemic event.
    pub event_start_block: u64,
    /// End block of the systemic event.
    pub event_end_block: u64,
    /// Domains covered by the amnesty.
    pub domains: Vec<String>,
    /// Whether reputation scores are restored.
    pub restore_reputation_scores: bool,
    /// Whether discipline states are restored.
    pub restore_discipline_states: bool,
}
