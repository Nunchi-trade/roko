#![allow(
    dead_code,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    clippy::upper_case_acronyms
)]

//! Phase 2+ identity and marketplace stubs for `docs/14-identity-economy`.
//!
//! These types mirror the deferred identity, reputation, marketplace, payment,
//! and tokenomics surfaces described in the docs. They intentionally avoid real
//! runtime logic.

use crate::phase2::{Address, u256};
use std::{collections::HashMap, time::Duration};

/// Placeholder passport or agent identifier used by the identity-economy docs.
pub type AgentId = u256;

/// Placeholder BLAKE3 hash used across the deferred identity-economy surface.
pub type Blake3Hash = [u8; 32];

/// Placeholder detached signature bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signature(pub [u8; 64]);

impl Default for Signature {
    fn default() -> Self {
        Self([0; 64])
    }
}

/// Placeholder HDC vector used for discovery and marketplace indexing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HdcVector(pub [u64; 160]);

impl Default for HdcVector {
    fn default() -> Self {
        Self([0; 160])
    }
}

/// Placeholder DID resolution document for `did:korai` identifiers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DidDocument {
    /// JSON-LD contexts for the document.
    pub context: Vec<String>,
    /// Canonical DID string.
    pub id: String,
    /// DID controller.
    pub controller: String,
    /// Other identifiers linked to the passport.
    pub also_known_as: Vec<String>,
    /// Verification methods published by the document.
    pub verification_method: Vec<VerificationMethod>,
    /// Service endpoints exposed by the agent.
    pub service: Vec<DidServiceEndpoint>,
    /// Authentication method references.
    pub authentication: Vec<String>,
    /// Assertion method references.
    pub assertion_method: Vec<String>,
}

/// Placeholder DID verification method entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerificationMethod {
    /// Fragment identifier for the verification method.
    pub id: String,
    /// Verification suite name.
    pub method_type: String,
    /// DID that controls the method.
    pub controller: String,
    /// Blockchain account reference bound to the method.
    pub blockchain_account_id: String,
}

/// Placeholder DID service endpoint published by a passport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DidServiceEndpoint {
    /// Service identifier.
    pub id: String,
    /// Service type label.
    pub service_type: String,
    /// Concrete endpoint URI.
    pub service_endpoint: String,
}

/// Placeholder W3C VC proof block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataIntegrityProof {
    /// Cryptographic suite used to create the proof.
    pub cryptosuite: String,
    /// Verification method identifier.
    pub verification_method: String,
    /// Proof payload bytes.
    pub proof_value: Vec<u8>,
    /// Timestamp the proof was created.
    pub created: String,
}

/// Placeholder proof used when linking external DIDs to a passport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkProof {
    /// Proof family name.
    pub proof_type: String,
    /// Raw proof bytes.
    pub proof: Vec<u8>,
}

/// A Verifiable Credential issued for a Korai agent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentCredential {
    /// VC JSON-LD contexts.
    pub context: Vec<String>,
    /// VC type list.
    pub credential_type: Vec<String>,
    /// DID of the issuer.
    pub issuer: String,
    /// Valid-from timestamp.
    pub valid_from: String,
    /// Optional expiry timestamp.
    pub valid_until: Option<String>,
    /// Credential subject describing the agent.
    pub credential_subject: AgentCredentialSubject,
    /// Data-integrity proof for the credential.
    pub proof: DataIntegrityProof,
}

/// Subject payload embedded inside an [`AgentCredential`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentCredentialSubject {
    /// DID of the credential holder.
    pub id: String,
    /// Passport tier encoded for VC presentation.
    pub passport_tier: u8,
    /// Human-readable capability names.
    pub capabilities: Vec<String>,
    /// Domain reputation snapshot at issuance time.
    pub domain_reputations: HashMap<String, f64>,
    /// Whether the agent is TEE-attested.
    pub tee_attested: bool,
    /// Certified regulatory templates held by the agent.
    pub compliance_templates: Vec<String>,
}

/// Personalized `PageRank` parameters for trust propagation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PersonalizedPageRank {
    /// Teleport probability.
    pub alpha: f64,
    /// Trusted seed-set passport ids.
    pub seed_set: Vec<u256>,
    /// Maximum iteration count.
    pub max_iterations: u32,
    /// Convergence threshold.
    pub epsilon: f64,
}

impl PersonalizedPageRank {
    /// Compute trust propagation over the interaction graph.
    pub fn compute(&self, _graph: &InteractionGraph) -> HashMap<u256, f64> {
        todo!("Phase 2+: compute personalized PageRank scores for passport trust")
    }
}

/// Placeholder interaction graph for trust propagation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InteractionGraph {
    /// Nodes included in the trust graph.
    pub nodes: Vec<u256>,
    /// Weighted directed edges `(from, to, weight)`.
    pub edges: Vec<(u256, u256, f64)>,
}

/// Deferred `SybilRank` detector parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SybilRankDetector {
    /// Random-walk length.
    pub walk_length: u32,
    /// Trusted seed passports.
    pub trust_seed: Vec<u256>,
    /// Flagging threshold.
    pub threshold: f64,
}

/// Result of a `SybilRank` scan.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SybilScanResult {
    /// Passports flagged as likely Sybil agents.
    pub flagged_agents: Vec<u256>,
    /// Clusters found during the scan.
    pub clusters: Vec<SybilCluster>,
    /// Estimated size of the honest region.
    pub honest_region_size: usize,
    /// Scan timestamp.
    pub scan_timestamp: u64,
}

/// Cluster-level Sybil analysis output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SybilCluster {
    /// Cluster members.
    pub members: Vec<u256>,
    /// Internal graph density.
    pub internal_edge_density: f64,
    /// Edges from the cluster into the honest region.
    pub external_edge_count: u32,
    /// Estimated Sybil probability.
    pub estimated_sybil_probability: f64,
}

/// Optional proof-of-unique-agent attestation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UniquenessAttestation {
    /// Uniqueness mechanism used for the proof.
    pub attestation_type: UniquenessType,
    /// Hash of the attestation payload.
    pub proof_hash: [u8; 32],
    /// Block or timestamp of verification.
    pub verified_at: u64,
    /// Expiry timestamp for the attestation.
    pub expiry: u64,
    /// Passport id of the verifier.
    pub verifier: u256,
}

/// Supported uniqueness-proof families.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UniquenessType {
    /// World ID style proof.
    #[default]
    WorldId,
    /// `BrightID` social-graph proof.
    BrightId,
    /// Gitcoin Passport score threshold.
    GitcoinPassport,
    /// TEE-backed uniqueness proof.
    TeeAttestation,
    /// Governance-based social vouching.
    GovernanceVouch,
}

/// Exact Korai passport stub described in the identity docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KoraiPassport {
    /// ERC-721 token identifier for the passport.
    pub passport_id: u256,
    /// Wallet that controls the passport.
    pub owner: Address,
    /// Capability bitmask granted to the agent.
    pub capability_list: u64,
    /// Domain-specific stakes in KORAI.
    pub domain_stakes: HashMap<u8, u256>,
    /// Seven domain reputation tracks.
    pub reputation_tracks: [ReputationTrack; 7],
    /// TEE attestation hash.
    pub tee_attestation: [u8; 32],
    /// Hash of the committed system prompt.
    pub system_prompt_hash: [u8; 32],
    /// Passport tier.
    pub tier: u8,
    /// Permanent slash history.
    pub slash_history: Vec<SlashRecord>,
    /// Registration block.
    pub registered_block: u64,
    /// URI of the Agent Card metadata.
    pub agent_card_uri: String,
}

/// Per-domain reputation track carried by a [`KoraiPassport`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReputationTrack {
    /// Domain identifier.
    pub domain: u8,
    /// EMA score scaled to `0..=1000`.
    pub score: u16,
    /// Count of feedback events in this domain.
    pub feedback_count: u32,
    /// Last block that updated the track.
    pub last_feedback_block: u64,
    /// Encoded discipline state for the domain.
    pub discipline_state: u8,
}

/// Slash record attached to a Korai passport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlashRecord {
    /// Block that recorded the slash.
    pub block: u64,
    /// Domain in which the slash occurred.
    pub domain: u8,
    /// Reputation amount slashed.
    pub amount: u16,
    /// Hash of the slash evidence or explanation.
    pub reason_hash: [u8; 32],
    /// Slash category applied to the event.
    pub category: SlashCategory,
}

/// Slash severity categories from the passport docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SlashCategory {
    /// Missed a committed deadline.
    MissedDeadline,
    /// Abandoned the job or obligation.
    Abandoned,
    /// Failed quality review.
    QualityRejection,
    /// Repeated quality failures.
    RepeatedQuality,
    /// Plagiarism or unattributed copying.
    Plagiarism,
    /// Manipulated results dishonestly.
    ResultManipulation,
    /// Violated TEE or attestation guarantees.
    #[default]
    TeeViolation,
}

/// W3C DID extension data attached to a Korai passport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PassportDidExtension {
    /// Primary `did:korai` identifier.
    pub primary_did: String,
    /// Linked external DIDs.
    pub linked_dids: Vec<LinkedDid>,
    /// Verifiable credentials issued to the passport.
    pub credentials: Vec<CredentialReference>,
    /// DID service endpoints synchronized from the Agent Card.
    pub service_endpoints: Vec<DidServiceEndpoint>,
}

/// Linked DID reference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkedDid {
    /// Linked DID string.
    pub did: String,
    /// Relationship between the linked DID and the passport.
    pub link_type: DidLinkType,
    /// Verification timestamp.
    pub verified_at: u64,
    /// Cryptographic proof of control.
    pub proof: LinkProof,
}

/// Relationship kinds for linked DIDs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DidLinkType {
    /// Same controller key owns both identities.
    #[default]
    SameOwner,
    /// Linked DID delegates authority.
    Delegation,
    /// Linked DID participates in recovery.
    Recovery,
}

/// Lightweight reference to an issued verifiable credential.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CredentialReference {
    /// Credential type name.
    pub credential_type: String,
    /// DID of the issuer.
    pub issuer: String,
    /// Issuance timestamp.
    pub issued_at: u64,
    /// Optional expiry timestamp.
    pub expiry: Option<u64>,
    /// BLAKE3 hash of the full credential.
    pub credential_hash: [u8; 32],
}

/// Soul-recovery parameters for passport migration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SoulRecovery {
    /// Number of guardians in the recovery quorum.
    pub quorum_size: u32,
    /// Minimum number of attestations required.
    pub quorum_threshold: u32,
    /// Cooldown period before execution.
    pub cooldown_period: u64,
    /// Minimum tier required for guardians.
    pub guardian_min_tier: u8,
}

/// Recovery request for migrating a passport to a new wallet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryRequest {
    /// Passport being recovered.
    pub original_passport: u256,
    /// Destination wallet.
    pub new_wallet: Address,
    /// Start timestamp.
    pub initiated_at: u64,
    /// Guardian attestations collected so far.
    pub attestations: Vec<RecoveryAttestation>,
    /// Current recovery state.
    pub status: RecoveryStatus,
}

/// Attestation from a recovery guardian.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryAttestation {
    /// Guardian passport id.
    pub guardian_passport: u256,
    /// Timestamp of attestation.
    pub attested_at: u64,
    /// Evidence hash supporting the recovery.
    pub evidence_hash: [u8; 32],
    /// Guardian signature bytes.
    pub signature: Signature,
}

/// Lifecycle states for a social recovery request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RecoveryStatus {
    /// Waiting for quorum.
    #[default]
    Pending,
    /// Threshold met and cooling down.
    QuorumReached,
    /// Migration completed.
    Executed,
    /// Cancelled by the original owner.
    Cancelled,
    /// Timed out without sufficient attestations.
    Expired,
}

/// Local `EigenTrust` parameters for trust-weighted feedback.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalEigenTrust {
    /// Propagation depth for trust lookups.
    pub max_hops: u32,
    /// Damping factor.
    pub damping: f64,
    /// Pre-trusted passport ids.
    pub pre_trusted: Vec<u256>,
}

/// Aggregate collusion-analysis output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollusionReport {
    /// Suspected collusion rings.
    pub suspected_rings: Vec<CollusionRing>,
    /// Overall confidence score.
    pub confidence: f64,
    /// Supporting evidence entries.
    pub evidence: Vec<CollusionEvidence>,
    /// Suggested enforcement actions.
    pub recommended_actions: Vec<CollusionAction>,
}

/// Dense cluster suspected of coordinated reputation inflation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollusionRing {
    /// Suspected ring members.
    pub members: Vec<u256>,
    /// Ratio of bidirectional edges inside the ring.
    pub reciprocity_ratio: f64,
    /// Timing-correlation signal.
    pub avg_temporal_correlation: f64,
    /// Internal edge density.
    pub internal_density: f64,
    /// Connectivity to the outside graph.
    pub external_connectivity: f64,
    /// Estimated formation time.
    pub formation_date: u64,
}

/// Evidence explaining why a ring was flagged.
#[derive(Clone, Debug, PartialEq)]
pub enum CollusionEvidence {
    /// Reciprocity exceeded the threshold.
    HighReciprocity {
        /// Observed reciprocity ratio for the suspected ring.
        ratio: f64,
    },
    /// Feedback timing was suspiciously synchronized.
    TemporalSynchronization {
        /// Pearson-style timing correlation across members.
        correlation: f64,
    },
    /// Ring formed a dense subgraph.
    DenseSubgraph {
        /// Measured internal density of the cluster.
        density: f64,
        /// Expected density under a baseline random graph.
        expected: f64,
    },
    /// Ring is unusually isolated from the rest of the network.
    IsolatedCluster {
        /// Number of edges leaving the suspected ring.
        external_edges: u32,
    },
    /// Members give unusually inflated scores.
    ScoreInflation {
        /// Average score given by the suspected ring.
        avg_given: f64,
        /// Average score given across the wider network.
        network_avg: f64,
    },
}

/// Enforcement actions recommended by the detector.
#[derive(Clone, Debug, PartialEq)]
pub enum CollusionAction {
    /// Downweight a collective's influence.
    ReduceCollectiveWeight {
        /// Weighting factor applied to the collective.
        factor: f64,
    },
    /// Apply a reputation penalty.
    ReputationPenalty {
        /// Penalty amount to apply.
        amount: f64,
    },
    /// Escalate discipline state.
    DisciplineEscalation,
    /// Void specific feedback events.
    VoidFeedback {
        /// Feedback identifiers to invalidate.
        feedback_ids: Vec<Blake3Hash>,
    },
    /// Escalate for manual review.
    FlagForReview,
}

/// Simulation inputs for reputation-system modeling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReputationSimConfig {
    /// Number of agents to simulate.
    pub agent_count: u32,
    /// Simulation duration in days.
    pub duration_days: u32,
    /// Fraction of honest agents.
    pub honest_fraction: f64,
    /// Fraction of colluding agents.
    pub collusion_fraction: f64,
    /// Fraction of Sybil agents.
    pub sybil_fraction: f64,
    /// Mean tasks per agent per day.
    pub tasks_per_day: f64,
    /// Honest outcome alpha parameter.
    pub honest_outcome_alpha: f64,
    /// Honest outcome beta parameter.
    pub honest_outcome_beta: f64,
    /// Size of collusion rings.
    pub collusion_ring_size: u32,
    /// EMA alpha cap.
    pub alpha_cap: f64,
    /// Reputation decay half-life.
    pub decay_half_life_days: f64,
}

/// Simulation outputs for reputation-system validation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReputationSimOutput {
    /// Average honest-agent reputation.
    pub avg_honest_reputation: f64,
    /// Average colluder reputation.
    pub avg_collusion_reputation: f64,
    /// Average Sybil reputation.
    pub avg_sybil_reputation: f64,
    /// False-positive rate.
    pub false_positive_rate: f64,
    /// False-negative rate.
    pub false_negative_rate: f64,
    /// Reputation inequality.
    pub gini_coefficient: f64,
    /// Time to convergence in days.
    pub time_to_convergence_days: f64,
    /// Fraction of collusion rings detected.
    pub collusion_detection_rate: f64,
}

/// Marketplace listing published to the bazaar index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketplaceListing {
    /// Hash of the listing metadata.
    pub listing_hash: Blake3Hash,
    /// Seller passport id.
    pub seller_passport_id: u256,
    /// Listing title.
    pub title: String,
    /// Human-readable description.
    pub description: String,
    /// Search and discovery tags.
    pub domain_tags: Vec<String>,
    /// Delivery format.
    pub skill_format: SkillFormat,
    /// Seller-set base price in USDC base units.
    pub base_price_usdc: u64,
    /// Alpha-decay pricing parameters.
    pub decay_params: DecayParams,
    /// Verification badges attached to the listing.
    pub verification_badges: Vec<VerificationBadge>,
    /// Content hash for delivery verification.
    pub content_hash: Blake3Hash,
    /// Embedding used for similarity search.
    pub embedding: HdcVector,
    /// Listing timestamp.
    pub listed_at: u64,
    /// Seller reputation snapshot.
    pub reputation_snapshot: f64,
}

/// Listing payload formats supported by the bazaar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SkillFormat {
    /// Markdown skill document.
    #[default]
    SkillMd,
    /// Raw Engram payload.
    RawEngram,
}

/// Placeholder decay parameters used by bazaar listings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecayParams {
    /// Exponential decay constant.
    pub decay_lambda: f64,
    /// Market-regime multiplier.
    pub regime_multiplier: f64,
}

/// Verification badge attached to a marketplace listing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VerificationBadge {
    /// Badge label.
    pub label: String,
    /// Verifier passport id.
    pub verifier_passport_id: u256,
    /// Confidence score for the badge.
    pub confidence: f64,
}

/// Effectiveness metrics attributed to a purchased skill.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillEffectiveness {
    /// Skill identifier.
    pub skill_id: Blake3Hash,
    /// Buyer receiving the skill.
    pub buyer_agent: AgentId,
    /// Number of predictions made while the skill was active.
    pub predictions_made: u32,
    /// Number of correct predictions.
    pub predictions_correct: u32,
    /// Measured change in predictive accuracy.
    pub accuracy_delta: f64,
}

/// Multi-factor pricing engine for knowledge listings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DynamicPricingEngine {
    /// Base price in USDC base units.
    pub base_price: u64,
    /// Alpha-decay lambda.
    pub decay_lambda: f64,
    /// Regime multiplier.
    pub regime_multiplier: f64,
    /// Demand sensitivity coefficient.
    pub demand_sensitivity: f64,
    /// Competition sensitivity coefficient.
    pub competition_sensitivity: f64,
    /// Seller-defined price floor.
    pub price_floor: u64,
    /// Seller-defined price ceiling.
    pub price_ceiling: u64,
}

/// Dutch-auction sale mode for premium listings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnowledgeDutchAuction {
    /// Listing under auction.
    pub listing_hash: Blake3Hash,
    /// Starting price.
    pub start_price: u64,
    /// Reserve price.
    pub reserve_price: u64,
    /// Auction duration.
    pub auction_duration: Duration,
    /// Auction start timestamp.
    pub started_at: u64,
}

/// Subscription offer for continuing access to a seller's output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubscriptionPlan {
    /// Seller passport id.
    pub seller_passport: u256,
    /// Covered domain.
    pub domain: String,
    /// Monthly price in USDC base units.
    pub monthly_price: u64,
    /// Commitment duration in months.
    pub commitment_months: u32,
    /// Discount schedule keyed by commitment length.
    pub discount_schedule: Vec<(u32, f64)>,
    /// Included skills per month, or `0` for unlimited.
    pub included_skills_per_month: u32,
}

/// Futarchy-style quality-assessment market for listings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QualityPredictionMarket {
    /// Listing being evaluated.
    pub listing_hash: Blake3Hash,
    /// LMSR market maker for the listing.
    pub market: crate::identity_economy_markets::LmsrMarketMaker,
    /// Resolution question.
    pub question: String,
    /// Observation period before resolution.
    pub resolution_period: Duration,
    /// Minimum number of buyers for settlement.
    pub min_sample_size: u32,
}

/// Shared Payment Token used for delegated machine spending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SharedPaymentToken {
    /// Token identifier.
    pub spt_id: String,
    /// Parent MPP session identifier.
    pub parent_session_id: String,
    /// Maximum spend allowed.
    pub max_amount: u64,
    /// Expiry timestamp.
    pub expires_at: u64,
    /// Allowed service endpoints.
    pub scoped_to: Vec<ServiceEndpoint>,
    /// Amount drawn so far.
    pub drawn: u64,
    /// Holder of the delegated token.
    pub holder: AgentId,
}

/// Placeholder service endpoint descriptor for delegated payments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceEndpoint {
    /// Logical endpoint name.
    pub name: String,
    /// Resolved endpoint URL.
    pub url: String,
}

/// Minimal x402 payment client stub.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct X402Client {
    /// Signing-key label or handle.
    pub signer: String,
    /// Locally tracked balance in USDC base units.
    pub balance: u64,
    /// HTTP client handle used for transport.
    pub http: String,
}

/// Per-agent cost accounting entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentCost {
    /// Model name used for the work.
    pub model: String,
    /// Input token count.
    pub input_tokens: u64,
    /// Output token count.
    pub output_tokens: u64,
    /// Cache-read token count.
    pub cache_read_tokens: u64,
    /// Cache-write token count.
    pub cache_write_tokens: u64,
    /// Total estimated cost in USD.
    pub total_cost_usd: f64,
    /// Optional gateway-observed cost.
    pub gateway_cost_usd: Option<f64>,
    /// Optional savings versus direct spend.
    pub savings_usd: Option<f64>,
}

/// Per-plan aggregate cost accounting entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanCost {
    /// Plan identifier.
    pub plan_id: String,
    /// Per-agent costs within the plan.
    pub agents: Vec<AgentCost>,
    /// Total observed plan cost.
    pub total_cost_usd: f64,
    /// Gateway savings realized by the plan.
    pub gateway_savings_usd: f64,
    /// Estimated cost at planning time.
    pub estimated_cost_usd: f64,
    /// Difference between estimated and realized cost.
    pub delta_pct: f64,
}

/// Augmented bonding curve for curation staking.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurationBondingCurve {
    /// Slope parameter.
    pub slope: f64,
    /// Exponent parameter.
    pub exponent: f64,
    /// Base price parameter.
    pub base: f64,
    /// Reserve ratio.
    pub reserve_ratio: f64,
}

/// Token-simulation parameters for cadCAD/radCAD modeling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TokenSimConfig {
    /// Initial token supply.
    pub initial_supply: f64,
    /// Monthly agent growth rate.
    pub agent_growth_rate: f64,
    /// Carrying capacity of the agent population.
    pub max_agents: u32,
    /// Daily posting activity per agent.
    pub posts_per_agent_day: f64,
    /// Daily query activity per agent.
    pub queries_per_agent_day: f64,
    /// Daily jobs completed per agent.
    pub jobs_per_agent_day: f64,
    /// Annual demurrage rate.
    pub demurrage_rate: f64,
    /// Percentage of fees burned.
    pub burn_rate_pct: f64,
    /// Percentage routed to the knowledge vault.
    pub vault_rate_pct: f64,
    /// Percentage routed to treasury.
    pub treasury_rate_pct: f64,
    /// Average balance fraction staked.
    pub avg_stake_fraction: f64,
    /// Average balance fraction used for curation.
    pub avg_curation_fraction: f64,
    /// Simulation duration.
    pub duration_days: u32,
    /// Number of Monte Carlo runs.
    pub monte_carlo_runs: u32,
}

/// Token-simulation output metrics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TokenSimOutput {
    /// Final simulated supply.
    pub final_supply: f64,
    /// Simulated token velocity.
    pub token_velocity: f64,
    /// Average worker balance.
    pub avg_agent_balance: f64,
    /// Wealth inequality metric.
    pub gini_coefficient: f64,
    /// Knowledge-vault APY.
    pub knowledge_vault_apy: f64,
    /// Net annual supply change.
    pub net_annual_supply_change: f64,
    /// Fraction of supply that remains staked.
    pub staked_fraction: f64,
    /// Daily burn volume.
    pub daily_burn_volume: f64,
    /// Daily mint volume.
    pub daily_mint_volume: f64,
}

/// Harberger-style tax parameters for premium listing slots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HarbergerListingTax {
    /// Annual tax rate.
    pub tax_rate: f64,
    /// Minimum holding period before a buyout.
    pub min_holding: u64,
}

/// Premium listing slot priced under Harberger taxation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PremiumSlot {
    /// Slot identifier.
    pub slot_id: u32,
    /// Current holder passport id.
    pub holder: u256,
    /// Holder's self-assessed value.
    pub self_assessed_value: u64,
    /// Acquisition timestamp.
    pub acquired_at: u64,
    /// Taxes paid to date.
    pub taxes_paid: u64,
    /// Marketplace domain covered by the slot.
    pub domain: String,
}

/// Deferred gate types referenced by marketplace and futures docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GateType {
    /// Compile or syntax verification.
    Compile,
    /// Semantic or grounding verification.
    Semantic,
    /// Quality or rubric scoring.
    Quality,
    /// Safety or policy check.
    Safety,
    /// Arbitrary named gate.
    #[default]
    Custom,
}

/// Deferred gate verdict referenced by futures delivery records.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GateVerdict {
    /// Gate that produced the verdict.
    pub gate: GateType,
    /// Whether the gate passed.
    pub passed: bool,
    /// Optional score produced by the gate.
    pub score: Option<f64>,
    /// Human-readable explanation.
    pub detail: String,
}

/// Placeholder x402 payment proof used in futures purchases.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct X402Receipt {
    /// Receipt identifier.
    pub receipt_id: Blake3Hash,
    /// Amount charged through x402.
    pub amount: u64,
    /// Payer identifier.
    pub payer: String,
    /// Payee identifier.
    pub payee: String,
    /// Settlement timestamp.
    pub settled_at: u64,
}
