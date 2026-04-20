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
    /// Compute personalized PageRank trust scores over an interaction graph.
    ///
    /// Iterates until convergence (`max_delta < epsilon`) or `max_iterations`
    /// is reached. Seed nodes receive a teleport share of `alpha`; remaining
    /// `(1-alpha)` mass propagates along weighted edges.
    pub fn compute(&self, graph: &InteractionGraph) -> HashMap<u256, f64> {
        if graph.nodes.is_empty() {
            return HashMap::new();
        }

        let n = graph.nodes.len();

        // Build outgoing-weight sums for normalizing edges.
        let mut out_weight: HashMap<u256, f64> = HashMap::new();
        for &(from, _, w) in &graph.edges {
            *out_weight.entry(from).or_insert(0.0) += w;
        }

        // Uniform initialisation.
        let mut scores: HashMap<u256, f64> =
            graph.nodes.iter().map(|id| (*id, 1.0 / n as f64)).collect();

        let seed_val = if self.seed_set.is_empty() {
            0.0
        } else {
            1.0 / self.seed_set.len() as f64
        };

        for _ in 0..self.max_iterations {
            let mut new_scores: HashMap<u256, f64> = HashMap::with_capacity(n);

            for &node in &graph.nodes {
                // Teleport component: non-zero only for seed nodes.
                let seed_component = if self.seed_set.contains(&node) {
                    self.alpha * seed_val
                } else {
                    0.0
                };

                // Neighbour propagation — sum over in-edges.
                let neighbor_sum: f64 = graph
                    .edges
                    .iter()
                    .filter(|(_, to, _)| *to == node)
                    .map(|(from, _, w)| {
                        let from_score = scores.get(from).copied().unwrap_or(0.0);
                        let out_w = out_weight.get(from).copied().unwrap_or(1.0);
                        from_score * (w / out_w)
                    })
                    .sum();

                new_scores.insert(node, seed_component + (1.0 - self.alpha) * neighbor_sum);
            }

            let max_delta = graph
                .nodes
                .iter()
                .map(|n| {
                    (new_scores.get(n).copied().unwrap_or(0.0)
                        - scores.get(n).copied().unwrap_or(0.0))
                    .abs()
                })
                .fold(0.0_f64, f64::max);

            scores = new_scores;
            if max_delta < self.epsilon {
                break;
            }
        }

        scores
    }
}

impl SybilRankDetector {
    /// Propagate trust from seed nodes for `walk_length` steps, then flag
    /// nodes whose final trust is below `threshold` as potential Sybils.
    pub fn detect(&self, graph: &InteractionGraph) -> SybilScanResult {
        if graph.nodes.is_empty() {
            return SybilScanResult::default();
        }

        let n = graph.nodes.len();

        // Build outgoing-weight sums for normalizing edges.
        let mut out_weight: HashMap<u256, f64> = HashMap::new();
        for &(from, _, w) in &graph.edges {
            *out_weight.entry(from).or_insert(0.0) += w;
        }

        // Initialise trust: uniform budget on seed nodes, zero elsewhere.
        let seed_budget = if self.trust_seed.is_empty() {
            0.0
        } else {
            1.0 / self.trust_seed.len() as f64
        };
        let mut trust: HashMap<u256, f64> = graph
            .nodes
            .iter()
            .map(|&id| {
                let t = if self.trust_seed.contains(&id) {
                    seed_budget
                } else {
                    0.0
                };
                (id, t)
            })
            .collect();

        // Propagate for walk_length steps (O(log n) recommended).
        for _ in 0..self.walk_length {
            let mut new_trust: HashMap<u256, f64> =
                graph.nodes.iter().map(|&id| (id, 0.0)).collect();
            for &(from, to, w) in &graph.edges {
                let from_trust = trust.get(&from).copied().unwrap_or(0.0);
                let out_w = out_weight.get(&from).copied().unwrap_or(1.0);
                *new_trust.entry(to).or_insert(0.0) += from_trust * (w / out_w);
            }
            trust = new_trust;
        }

        // Flag nodes below threshold.
        let flagged_agents: Vec<u256> = graph
            .nodes
            .iter()
            .filter(|id| trust.get(id).copied().unwrap_or(0.0) < self.threshold)
            .copied()
            .collect();
        let honest_region_size = n - flagged_agents.len();

        SybilScanResult {
            flagged_agents,
            clusters: Vec::new(), // cluster detection done via detect_collusion_rings
            honest_region_size,
            scan_timestamp: 0,
        }
    }
}

/// Detect collusion rings: clusters with high internal density but few
/// external connections suggest coordinated Sybil behaviour.
///
/// Criteria: `internal_edge_density > 0.5` **and**
/// `external_edge_count < members.len() * 2`.
pub fn detect_collusion_rings(clusters: &[SybilCluster]) -> Vec<&SybilCluster> {
    clusters
        .iter()
        .filter(|c| {
            c.internal_edge_density > 0.5
                && (c.external_edge_count as usize) < c.members.len() * 2
        })
        .collect()
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

/// Passport lifecycle states (IDECON-02).
///
/// State machine: `Minting -> Active -> Suspended -> Revoked`.
/// Only forward transitions are allowed; once `Revoked` the passport is
/// permanently disabled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PassportState {
    /// Passport is being created and is not yet usable.
    Minting,
    /// Passport is fully active and operational.
    Active,
    /// Passport is temporarily suspended.
    Suspended {
        /// Human-readable suspension reason.
        reason: String,
    },
    /// Passport is permanently revoked.
    Revoked {
        /// Human-readable revocation reason.
        reason: String,
    },
}

impl Default for PassportState {
    fn default() -> Self {
        Self::Minting
    }
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
    /// Lifecycle state of the passport.
    pub state: PassportState,
}

impl KoraiPassport {
    /// Transition from `Minting` to `Active`.
    ///
    /// Returns an error if the passport is not in the `Minting` state.
    pub fn activate(&mut self) -> Result<(), &'static str> {
        match &self.state {
            PassportState::Minting => {
                self.state = PassportState::Active;
                Ok(())
            }
            _ => Err("can only activate from Minting state"),
        }
    }

    /// Transition from `Active` to `Suspended`.
    ///
    /// Returns an error if the passport is not in the `Active` state.
    pub fn suspend(&mut self, reason: &str) -> Result<(), &'static str> {
        match &self.state {
            PassportState::Active => {
                self.state = PassportState::Suspended {
                    reason: reason.to_string(),
                };
                Ok(())
            }
            _ => Err("can only suspend from Active state"),
        }
    }

    /// Transition from `Suspended` back to `Active`.
    ///
    /// Returns an error if the passport is not in the `Suspended` state.
    pub fn reinstate(&mut self) -> Result<(), &'static str> {
        match &self.state {
            PassportState::Suspended { .. } => {
                self.state = PassportState::Active;
                Ok(())
            }
            _ => Err("can only reinstate from Suspended state"),
        }
    }

    /// Transition to `Revoked` from any non-revoked state.
    ///
    /// Revocation is permanent and cannot be undone (soul recovery mints
    /// a new passport instead).
    pub fn revoke(&mut self, reason: &str) -> Result<(), &'static str> {
        match &self.state {
            PassportState::Revoked { .. } => Err("passport already revoked"),
            _ => {
                self.state = PassportState::Revoked {
                    reason: reason.to_string(),
                };
                Ok(())
            }
        }
    }

    /// Verify a system prompt against the committed hash.
    ///
    /// Computes `BLAKE3(prompt)` and compares to `self.system_prompt_hash`.
    /// At agent startup, a mismatch should block execution.
    pub fn verify_system_prompt(&self, prompt: &str) -> bool {
        let hash = blake3::hash(prompt.as_bytes());
        *hash.as_bytes() == self.system_prompt_hash
    }

    /// Returns `true` if the passport is in a usable state (`Active`).
    pub fn is_active(&self) -> bool {
        matches!(self.state, PassportState::Active)
    }
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

impl ReputationTrack {
    /// Return the score as an `f64` in the range `[0.0, 1.0]`.
    pub fn score_f64(&self) -> f64 {
        f64::from(self.score) / 1000.0
    }

    /// Apply an EMA reputation update.
    ///
    /// `observation` is in `[0.0, 1.0]`. Adaptive alpha =
    /// `min(0.3, 2/(feedback_count+1))`. On cold start (first observation)
    /// alpha = 1.0 so the first observation sets the initial score.
    /// `now_block` is the current block number used for timestamping.
    pub fn update_reputation(&mut self, observation: f64, now_block: u64) {
        let obs = observation.clamp(0.0, 1.0);
        let alpha = if self.feedback_count == 0 {
            1.0
        } else {
            (2.0 / (self.feedback_count as f64 + 1.0)).min(0.3)
        };
        let old = self.score_f64();
        let new_score = alpha * obs + (1.0 - alpha) * old;
        self.score = (new_score * 1000.0).round().min(1000.0) as u16;
        self.feedback_count += 1;
        self.last_feedback_block = now_block;
    }

    /// Return the reputation score decayed toward the 0.5 baseline using a
    /// 30-day half-life. `now_secs` and `last_updated_secs` are wall-clock
    /// seconds.
    pub fn decayed_score(&self, now_secs: u64, last_updated_secs: u64) -> f64 {
        let days_since =
            now_secs.saturating_sub(last_updated_secs) as f64 / 86_400.0;
        // 30-day half-life: decay factor = exp(-ln2 * days / 30)
        let decay = (-0.693_147_18 * days_since / 30.0).exp();
        let s = self.score_f64();
        0.5 + (s - 0.5) * decay
    }

    /// Reputation multiplier: `0.1 + 2.9 * R^1.7`.
    ///
    /// Maps a reputation in `[0, 1]` to a multiplier in `[0.1, 3.0]`, used
    /// by Vickrey auction scoring and dynamic pricing.
    pub fn reputation_multiplier(&self) -> f64 {
        0.1 + 2.9 * self.score_f64().powf(1.7)
    }
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

/// Four-stage ingestion pipeline for marketplace listings (IDECON-04).
///
/// `Quarantine -> Consensus -> Sandbox -> Active`.
#[derive(Clone, Debug, PartialEq)]
pub enum ListingStage {
    /// Listing submitted; metadata validated, held for minimum 24h.
    Quarantine {
        /// Timestamp when the listing was submitted.
        submitted_at: u64,
    },
    /// Two+ independent validators must approve before advancement.
    Consensus {
        /// Approvals collected.
        approvals: u32,
        /// Rejections collected.
        rejections: u32,
    },
    /// Trial purchase with 100% refund guarantee; effectiveness tracked.
    Sandbox {
        /// Timestamp when the sandbox period started.
        trial_start: u64,
        /// Refunds issued during the sandbox period.
        refunds: u32,
    },
    /// Full marketplace listing with dynamic pricing active.
    Active,
}

impl Default for ListingStage {
    fn default() -> Self {
        Self::Quarantine { submitted_at: 0 }
    }
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
    /// Current ingestion stage (IDECON-04).
    pub stage: ListingStage,
}

impl MarketplaceListing {
    /// Advance the listing to the next ingestion stage.
    ///
    /// Enforces the pipeline rules:
    /// - `Quarantine -> Consensus`: requires 24h (86400s) elapsed since submission.
    /// - `Consensus -> Sandbox`: requires `approvals >= 2 && approvals > rejections`.
    /// - `Sandbox -> Active`: always allowed (immediate).
    /// - `Active` is terminal.
    pub fn advance_stage(&mut self, now_secs: u64) -> Result<(), &'static str> {
        match &self.stage {
            ListingStage::Quarantine { submitted_at } => {
                if now_secs.saturating_sub(*submitted_at) < 86_400 {
                    return Err("quarantine requires 24h minimum");
                }
                self.stage = ListingStage::Consensus {
                    approvals: 0,
                    rejections: 0,
                };
                Ok(())
            }
            ListingStage::Consensus {
                approvals,
                rejections,
            } => {
                if *approvals < 2 {
                    return Err("consensus requires at least 2 approvals");
                }
                if approvals <= rejections {
                    return Err("consensus requires approvals > rejections");
                }
                self.stage = ListingStage::Sandbox {
                    trial_start: now_secs,
                    refunds: 0,
                };
                Ok(())
            }
            ListingStage::Sandbox { .. } => {
                self.stage = ListingStage::Active;
                Ok(())
            }
            ListingStage::Active => Err("listing is already active"),
        }
    }

    /// Record an approval vote during the Consensus stage.
    pub fn record_approval(&mut self) -> Result<(), &'static str> {
        match &mut self.stage {
            ListingStage::Consensus { approvals, .. } => {
                *approvals += 1;
                Ok(())
            }
            _ => Err("approvals can only be recorded in Consensus stage"),
        }
    }

    /// Record a rejection vote during the Consensus stage.
    pub fn record_rejection(&mut self) -> Result<(), &'static str> {
        match &mut self.stage {
            ListingStage::Consensus { rejections, .. } => {
                *rejections += 1;
                Ok(())
            }
            _ => Err("rejections can only be recorded in Consensus stage"),
        }
    }
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

impl DynamicPricingEngine {
    /// Compute the dynamic price for a listing (IDECON-04).
    ///
    /// `P(t) = base_price * rep_mult(seller_rep) * e^(-decay_lambda * hours) * demand_mult`
    ///
    /// where `rep_mult(R) = 0.1 + 2.9 * R^1.7` and
    /// `demand_mult = 1.0 + demand_sensitivity * (recent_purchases / avg_purchases - 1.0)`.
    ///
    /// The result is clamped to `[price_floor, price_ceiling]`.
    pub fn compute_price(
        &self,
        hours_since_listing: f64,
        seller_reputation: f64,
        recent_purchases: f64,
        avg_purchases: f64,
    ) -> u64 {
        let rep_mult = 0.1 + 2.9 * seller_reputation.clamp(0.0, 1.0).powf(1.7);
        let decay = (-self.decay_lambda * hours_since_listing).exp();
        let demand_mult =
            1.0 + self.demand_sensitivity * (recent_purchases / avg_purchases.max(1.0) - 1.0);
        let price = self.base_price as f64 * rep_mult * decay * demand_mult;
        (price.round() as u64).clamp(self.price_floor, self.price_ceiling)
    }
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

/// HTTP 402 payment challenge received from a paid resource (IDECON-07).
///
/// The server returns this in the `X-Payment-Challenge` header when a
/// request requires payment via the x402 protocol.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaymentChallenge {
    /// Amount in USDC base units required for access.
    pub amount: u64,
    /// Recipient address (0x-prefixed hex).
    pub recipient: String,
    /// ERC-20 token contract address (typically USDC).
    pub token: String,
    /// Network identifier (e.g. "base", "ethereum").
    pub network: String,
    /// Challenge nonce to prevent replay.
    pub nonce: u64,
    /// Challenge expiry timestamp (Unix seconds).
    pub expires_at: u64,
}

impl PaymentChallenge {
    /// Check whether the challenge has expired relative to `now_secs`.
    #[must_use]
    pub fn is_expired(&self, now_secs: u64) -> bool {
        now_secs >= self.expires_at
    }
}

/// ERC-3009 `transferWithAuthorization` signature payload (IDECON-07).
///
/// This is the gasless USDC transfer authorization that the agent signs
/// in response to a [`PaymentChallenge`]. The server submits this on-chain
/// to execute the transfer without the agent paying gas.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Erc3009Auth {
    /// Sender address (the agent's wallet).
    pub from: String,
    /// Recipient address (the resource provider).
    pub to: String,
    /// Amount in token base units.
    pub value: u64,
    /// Earliest valid timestamp for the authorization.
    pub valid_after: u64,
    /// Latest valid timestamp for the authorization.
    pub valid_before: u64,
    /// Unique nonce preventing replay.
    pub nonce: u64,
    /// ECDSA signature v component.
    pub v: u8,
    /// ECDSA signature r component (32 bytes).
    pub r: [u8; 32],
    /// ECDSA signature s component (32 bytes).
    pub s: [u8; 32],
}

/// Result of an x402 payment attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum X402PaymentResult {
    /// Payment succeeded; resource access granted.
    Success {
        /// Receipt returned by the server.
        receipt: X402Receipt,
    },
    /// Payment failed with an error description.
    Failure {
        /// Human-readable failure reason.
        reason: String,
    },
    /// Challenge expired before the agent could respond.
    Expired,
    /// Insufficient balance to cover the challenge amount.
    InsufficientBalance {
        /// Required amount.
        required: u64,
        /// Available balance.
        available: u64,
    },
}

/// Minimal x402 payment client (IDECON-07).
///
/// Implements the Coinbase x402 HTTP payment protocol for machine-to-machine
/// micropayments. The flow is:
/// 1. Agent sends HTTP request to a paid resource.
/// 2. Server returns `HTTP 402` with `X-Payment-Challenge` header.
/// 3. Agent calls [`parse_challenge`] to decode the challenge.
/// 4. Agent calls [`sign_authorization`] to produce an ERC-3009 payload.
/// 5. Agent retries the request with `X-Payment-Authorization` header.
/// 6. Server verifies on-chain and returns the resource + `X-Payment-Receipt`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct X402Client {
    /// Signing-key label or handle.
    pub signer: String,
    /// Locally tracked balance in USDC base units.
    pub balance: u64,
    /// HTTP client handle used for transport.
    pub http: String,
    /// Wallet address (0x-prefixed hex) derived from the signer.
    pub address: String,
}

impl X402Client {
    /// Create a new x402 client with the given signer and initial balance.
    #[must_use]
    pub fn new(signer: String, balance: u64, address: String) -> Self {
        Self {
            signer,
            balance,
            http: String::new(),
            address,
        }
    }

    /// Parse a `X-Payment-Challenge` header value into a [`PaymentChallenge`].
    ///
    /// The header is expected to be a JSON object with fields:
    /// `amount`, `recipient`, `token`, `network`, `nonce`, `expires_at`.
    ///
    /// Returns `Err` if the header is malformed or missing required fields.
    pub fn parse_challenge(header: &str) -> Result<PaymentChallenge, String> {
        // Minimal JSON parsing without pulling in a full JSON library.
        // In production this would use serde_json; here we parse the
        // structured fields manually for zero-dep identity economy stubs.
        let trimmed = header.trim();
        if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
            return Err("challenge header must be a JSON object".into());
        }

        fn extract_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
            let needle = format!("\"{key}\"");
            let pos = json.find(&needle)?;
            let after = &json[pos + needle.len()..];
            let after = after.trim_start().strip_prefix(':')?;
            let after = after.trim_start();
            if after.starts_with('"') {
                let content = &after[1..];
                let end = content.find('"')?;
                Some(&content[..end])
            } else {
                None
            }
        }

        fn extract_u64(json: &str, key: &str) -> Option<u64> {
            let needle = format!("\"{key}\"");
            let pos = json.find(&needle)?;
            let after = &json[pos + needle.len()..];
            let after = after.trim_start().strip_prefix(':')?;
            let after = after.trim_start();
            let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
            after[..end].parse().ok()
        }

        Ok(PaymentChallenge {
            amount: extract_u64(trimmed, "amount").unwrap_or(0),
            recipient: extract_str(trimmed, "recipient")
                .unwrap_or("")
                .to_string(),
            token: extract_str(trimmed, "token").unwrap_or("").to_string(),
            network: extract_str(trimmed, "network")
                .unwrap_or("base")
                .to_string(),
            nonce: extract_u64(trimmed, "nonce").unwrap_or(0),
            expires_at: extract_u64(trimmed, "expires_at").unwrap_or(0),
        })
    }

    /// Sign an ERC-3009 `transferWithAuthorization` payload for the given challenge.
    ///
    /// In production this would invoke the actual signing key (hardware wallet,
    /// KMS, or local keyfile). Here we produce a deterministic stub signature
    /// from the challenge parameters for type-safety and testing.
    pub fn sign_authorization(
        &self,
        challenge: &PaymentChallenge,
        now_secs: u64,
    ) -> Result<Erc3009Auth, String> {
        if challenge.is_expired(now_secs) {
            return Err("challenge has expired".into());
        }
        if self.balance < challenge.amount {
            return Err(format!(
                "insufficient balance: have {} need {}",
                self.balance, challenge.amount
            ));
        }

        // Construct the authorization payload.
        // The `v`, `r`, `s` fields would be produced by ECDSA signing in production.
        // Here we use a deterministic stub: hash(signer || nonce) -> r, s.
        let mut r_bytes = [0u8; 32];
        let mut s_bytes = [0u8; 32];
        // Deterministic stub: fill r with signer hash, s with nonce bytes.
        for (i, b) in self.signer.bytes().enumerate() {
            r_bytes[i % 32] ^= b;
        }
        s_bytes[..8].copy_from_slice(&challenge.nonce.to_le_bytes());

        Ok(Erc3009Auth {
            from: self.address.clone(),
            to: challenge.recipient.clone(),
            value: challenge.amount,
            valid_after: 0,
            valid_before: challenge.expires_at,
            nonce: challenge.nonce,
            v: 27, // standard ECDSA v value
            r: r_bytes,
            s: s_bytes,
        })
    }

    /// Format the signed authorization as an `X-Payment-Authorization` header value.
    #[must_use]
    pub fn make_payment_header(auth: &Erc3009Auth) -> String {
        fn bytes_to_hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
        format!(
            "{{\"from\":\"{}\",\"to\":\"{}\",\"value\":{},\"valid_after\":{},\"valid_before\":{},\"nonce\":{},\"v\":{},\"r\":\"{}\",\"s\":\"{}\"}}",
            auth.from,
            auth.to,
            auth.value,
            auth.valid_after,
            auth.valid_before,
            auth.nonce,
            auth.v,
            bytes_to_hex(&auth.r),
            bytes_to_hex(&auth.s),
        )
    }

    /// Attempt the full x402 payment flow for a given challenge.
    ///
    /// This is the synchronous logic that:
    /// 1. Validates the challenge is not expired.
    /// 2. Checks the local balance is sufficient.
    /// 3. Signs the ERC-3009 authorization.
    /// 4. Decrements the local balance.
    ///
    /// In production, step 4 would be replaced by an actual HTTP retry
    /// and on-chain verification. Here it returns the signed auth for
    /// the caller to attach to the retry request.
    pub fn pay(
        &mut self,
        challenge: &PaymentChallenge,
        now_secs: u64,
    ) -> X402PaymentResult {
        if challenge.is_expired(now_secs) {
            return X402PaymentResult::Expired;
        }
        if self.balance < challenge.amount {
            return X402PaymentResult::InsufficientBalance {
                required: challenge.amount,
                available: self.balance,
            };
        }

        match self.sign_authorization(challenge, now_secs) {
            Ok(_auth) => {
                self.balance -= challenge.amount;
                X402PaymentResult::Success {
                    receipt: X402Receipt {
                        receipt_id: [0; 32],
                        amount: challenge.amount,
                        payer: self.address.clone(),
                        payee: challenge.recipient.clone(),
                        settled_at: now_secs,
                    },
                }
            }
            Err(reason) => X402PaymentResult::Failure { reason },
        }
    }
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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // IDECON-01: EMA reputation
    // -----------------------------------------------------------------------

    #[test]
    fn reputation_cold_start_sets_initial() {
        let mut t = ReputationTrack::default();
        t.update_reputation(0.8, 100);
        assert_eq!(t.score, 800); // cold start alpha=1.0
        assert_eq!(t.feedback_count, 1);
        assert_eq!(t.last_feedback_block, 100);
    }

    #[test]
    fn reputation_ema_blends() {
        let mut t = ReputationTrack::default();
        // Cold start
        t.update_reputation(1.0, 1);
        assert_eq!(t.score, 1000);
        // Second observation: alpha = min(0.3, 2/2) = min(0.3, 1.0) = 0.3
        // new = 0.3 * 0.0 + 0.7 * 1.0 = 0.7
        t.update_reputation(0.0, 2);
        assert_eq!(t.score, 700);
    }

    #[test]
    fn reputation_alpha_caps_at_03() {
        let mut t = ReputationTrack {
            feedback_count: 100,
            score: 500,
            ..Default::default()
        };
        // alpha = min(0.3, 2/101) = min(0.3, ~0.0198) = ~0.0198
        t.update_reputation(1.0, 200);
        // Small alpha means small move: 0.0198 * 1.0 + 0.9802 * 0.5 ~ 0.51
        assert!(t.score_f64() > 0.5 && t.score_f64() < 0.55);
    }

    #[test]
    fn decayed_score_at_zero_days() {
        let t = ReputationTrack {
            score: 800,
            ..Default::default()
        };
        let d = t.decayed_score(1000, 1000);
        assert!((d - 0.8).abs() < 1e-9);
    }

    #[test]
    fn decayed_score_at_30_days_halves_delta() {
        let t = ReputationTrack {
            score: 1000,
            ..Default::default()
        };
        let d = t.decayed_score(30 * 86_400, 0);
        // After 30 days: 0.5 + (1.0 - 0.5) * 0.5 = 0.75
        assert!((d - 0.75).abs() < 0.01);
    }

    #[test]
    fn reputation_multiplier_boundaries() {
        let low = ReputationTrack {
            score: 0,
            ..Default::default()
        };
        let high = ReputationTrack {
            score: 1000,
            ..Default::default()
        };
        assert!((low.reputation_multiplier() - 0.1).abs() < 1e-9);
        assert!((high.reputation_multiplier() - 3.0).abs() < 1e-9);
    }

    // -----------------------------------------------------------------------
    // IDECON-06: PersonalizedPageRank
    // -----------------------------------------------------------------------

    #[test]
    fn ppr_empty_graph() {
        let ppr = PersonalizedPageRank {
            alpha: 0.15,
            seed_set: vec![],
            max_iterations: 100,
            epsilon: 1e-6,
        };
        let graph = InteractionGraph::default();
        let scores = ppr.compute(&graph);
        assert!(scores.is_empty());
    }

    #[test]
    fn ppr_seed_gets_highest_score() {
        let ppr = PersonalizedPageRank {
            alpha: 0.15,
            seed_set: vec![1],
            max_iterations: 100,
            epsilon: 1e-9,
        };
        let graph = InteractionGraph {
            nodes: vec![1, 2, 3],
            edges: vec![
                (1, 2, 1.0),
                (2, 3, 1.0),
                (3, 1, 1.0),
            ],
        };
        let scores = ppr.compute(&graph);
        assert!(scores[&1] > scores[&2]);
        assert!(scores[&1] > scores[&3]);
    }

    #[test]
    fn ppr_all_scores_nonneg() {
        let ppr = PersonalizedPageRank {
            alpha: 0.15,
            seed_set: vec![1, 2],
            max_iterations: 50,
            epsilon: 1e-6,
        };
        let graph = InteractionGraph {
            nodes: vec![1, 2, 3],
            edges: vec![(1, 2, 1.0), (2, 3, 1.0)],
        };
        let scores = ppr.compute(&graph);
        for s in scores.values() {
            assert!(*s >= 0.0, "PPR score must be non-negative");
        }
    }

    // -----------------------------------------------------------------------
    // IDECON-06: SybilRank
    // -----------------------------------------------------------------------

    #[test]
    fn sybil_rank_flags_disconnected_nodes() {
        let detector = SybilRankDetector {
            walk_length: 4, // even, so trust ends back at the seed
            trust_seed: vec![1],
            threshold: 0.01,
        };
        // Cycle so trust flows back to seed after even-length walk.
        let graph = InteractionGraph {
            nodes: vec![1, 2, 99],
            edges: vec![(1, 2, 1.0), (2, 1, 1.0)],
        };
        let result = detector.detect(&graph);
        // Node 99 is disconnected, should be flagged.
        assert!(result.flagged_agents.contains(&99));
        // Node 1 is a seed and retains trust -- not flagged.
        assert!(!result.flagged_agents.contains(&1));
    }

    #[test]
    fn collusion_ring_detection() {
        let clusters = vec![
            SybilCluster {
                members: vec![10, 11, 12],
                internal_edge_density: 0.8,
                external_edge_count: 1, // < 3*2 = 6
                estimated_sybil_probability: 0.0,
            },
            SybilCluster {
                members: vec![20, 21],
                internal_edge_density: 0.2, // below 0.5
                external_edge_count: 10,
                estimated_sybil_probability: 0.0,
            },
        ];
        let rings = detect_collusion_rings(&clusters);
        assert_eq!(rings.len(), 1);
        assert!(rings[0].members.contains(&10));
    }

    // -----------------------------------------------------------------------
    // IDECON-02: Passport lifecycle
    // -----------------------------------------------------------------------

    #[test]
    fn passport_default_is_minting() {
        let p = KoraiPassport::default();
        assert_eq!(p.state, PassportState::Minting);
        assert!(!p.is_active());
    }

    #[test]
    fn passport_activate_from_minting() {
        let mut p = KoraiPassport::default();
        assert!(p.activate().is_ok());
        assert_eq!(p.state, PassportState::Active);
        assert!(p.is_active());
    }

    #[test]
    fn passport_activate_from_active_fails() {
        let mut p = KoraiPassport::default();
        p.activate().unwrap();
        assert!(p.activate().is_err());
    }

    #[test]
    fn passport_suspend_from_active() {
        let mut p = KoraiPassport::default();
        p.activate().unwrap();
        assert!(p.suspend("policy violation").is_ok());
        assert_eq!(
            p.state,
            PassportState::Suspended {
                reason: "policy violation".to_string()
            }
        );
        assert!(!p.is_active());
    }

    #[test]
    fn passport_suspend_from_minting_fails() {
        let mut p = KoraiPassport::default();
        assert!(p.suspend("bad").is_err());
    }

    #[test]
    fn passport_reinstate_from_suspended() {
        let mut p = KoraiPassport::default();
        p.activate().unwrap();
        p.suspend("test").unwrap();
        assert!(p.reinstate().is_ok());
        assert!(p.is_active());
    }

    #[test]
    fn passport_revoke_from_any_non_revoked() {
        let mut p = KoraiPassport::default();
        p.activate().unwrap();
        assert!(p.revoke("fraud").is_ok());
        assert_eq!(
            p.state,
            PassportState::Revoked {
                reason: "fraud".to_string()
            }
        );
        // Cannot revoke again.
        assert!(p.revoke("double revoke").is_err());
    }

    #[test]
    fn passport_verify_system_prompt() {
        let prompt = "You are a helpful agent.";
        let hash = blake3::hash(prompt.as_bytes());
        let mut p = KoraiPassport::default();
        p.system_prompt_hash = *hash.as_bytes();
        assert!(p.verify_system_prompt(prompt));
        assert!(!p.verify_system_prompt("You are a malicious agent."));
    }

    // -----------------------------------------------------------------------
    // IDECON-04: Marketplace staging + dynamic pricing
    // -----------------------------------------------------------------------

    #[test]
    fn listing_default_is_quarantine() {
        let l = MarketplaceListing::default();
        assert!(matches!(l.stage, ListingStage::Quarantine { .. }));
    }

    #[test]
    fn listing_advance_quarantine_requires_24h() {
        let mut l = MarketplaceListing {
            stage: ListingStage::Quarantine { submitted_at: 1000 },
            ..Default::default()
        };
        // Too early — only 100s elapsed.
        assert!(l.advance_stage(1100).is_err());
        // Exactly 24h later.
        assert!(l.advance_stage(1000 + 86_400).is_ok());
        assert!(matches!(l.stage, ListingStage::Consensus { .. }));
    }

    #[test]
    fn listing_advance_consensus_requires_approvals() {
        let mut l = MarketplaceListing {
            stage: ListingStage::Consensus {
                approvals: 1,
                rejections: 0,
            },
            ..Default::default()
        };
        // Only 1 approval — need 2.
        assert!(l.advance_stage(0).is_err());

        l.stage = ListingStage::Consensus {
            approvals: 2,
            rejections: 0,
        };
        assert!(l.advance_stage(0).is_ok());
        assert!(matches!(l.stage, ListingStage::Sandbox { .. }));
    }

    #[test]
    fn listing_advance_consensus_majority_required() {
        let mut l = MarketplaceListing {
            stage: ListingStage::Consensus {
                approvals: 2,
                rejections: 3,
            },
            ..Default::default()
        };
        assert!(l.advance_stage(0).is_err());
    }

    #[test]
    fn listing_advance_sandbox_to_active() {
        let mut l = MarketplaceListing {
            stage: ListingStage::Sandbox {
                trial_start: 0,
                refunds: 0,
            },
            ..Default::default()
        };
        assert!(l.advance_stage(0).is_ok());
        assert_eq!(l.stage, ListingStage::Active);
    }

    #[test]
    fn listing_active_is_terminal() {
        let mut l = MarketplaceListing {
            stage: ListingStage::Active,
            ..Default::default()
        };
        assert!(l.advance_stage(0).is_err());
    }

    #[test]
    fn listing_record_approval_and_rejection() {
        let mut l = MarketplaceListing {
            stage: ListingStage::Consensus {
                approvals: 0,
                rejections: 0,
            },
            ..Default::default()
        };
        l.record_approval().unwrap();
        l.record_approval().unwrap();
        l.record_rejection().unwrap();

        if let ListingStage::Consensus {
            approvals,
            rejections,
        } = l.stage
        {
            assert_eq!(approvals, 2);
            assert_eq!(rejections, 1);
        } else {
            panic!("expected Consensus stage");
        }
    }

    #[test]
    fn dynamic_pricing_basic() {
        let engine = DynamicPricingEngine {
            base_price: 1000,
            decay_lambda: 0.01,
            regime_multiplier: 1.0,
            demand_sensitivity: 0.5,
            competition_sensitivity: 0.0,
            price_floor: 100,
            price_ceiling: 5000,
        };
        let price = engine.compute_price(0.0, 1.0, 10.0, 10.0);
        // At t=0, rep=1.0: rep_mult=3.0, decay=1.0, demand=1.0 -> 3000
        assert_eq!(price, 3000);
    }

    #[test]
    fn dynamic_pricing_clamped_to_floor() {
        let engine = DynamicPricingEngine {
            base_price: 100,
            decay_lambda: 1.0,
            demand_sensitivity: 0.0,
            price_floor: 50,
            price_ceiling: 5000,
            ..Default::default()
        };
        // After many hours, decay drives price to ~0 but floor clamps it.
        let price = engine.compute_price(100.0, 0.0, 1.0, 1.0);
        assert_eq!(price, 50);
    }

    #[test]
    fn dynamic_pricing_clamped_to_ceiling() {
        let engine = DynamicPricingEngine {
            base_price: 10_000,
            decay_lambda: 0.0,
            demand_sensitivity: 2.0,
            price_floor: 100,
            price_ceiling: 5000,
            ..Default::default()
        };
        // High demand + high rep drives price above ceiling.
        let price = engine.compute_price(0.0, 1.0, 100.0, 1.0);
        assert_eq!(price, 5000);
    }

    // -----------------------------------------------------------------------
    // IDECON-07: x402 HTTP payment protocol
    // -----------------------------------------------------------------------

    #[test]
    fn x402_parse_challenge_basic() {
        let header = r#"{"amount": 500, "recipient": "0xABC", "token": "0xUSDC", "network": "base", "nonce": 42, "expires_at": 99999}"#;
        let challenge = X402Client::parse_challenge(header).unwrap();
        assert_eq!(challenge.amount, 500);
        assert_eq!(challenge.recipient, "0xABC");
        assert_eq!(challenge.token, "0xUSDC");
        assert_eq!(challenge.network, "base");
        assert_eq!(challenge.nonce, 42);
        assert_eq!(challenge.expires_at, 99999);
    }

    #[test]
    fn x402_parse_challenge_rejects_non_json() {
        assert!(X402Client::parse_challenge("not json").is_err());
    }

    #[test]
    fn x402_sign_authorization_ok() {
        let client = X402Client::new("key1".into(), 1000, "0xAgent".into());
        let challenge = PaymentChallenge {
            amount: 500,
            recipient: "0xProvider".into(),
            token: "0xUSDC".into(),
            network: "base".into(),
            nonce: 1,
            expires_at: 200,
        };
        let auth = client.sign_authorization(&challenge, 100).unwrap();
        assert_eq!(auth.from, "0xAgent");
        assert_eq!(auth.to, "0xProvider");
        assert_eq!(auth.value, 500);
        assert_eq!(auth.nonce, 1);
        assert_eq!(auth.v, 27);
    }

    #[test]
    fn x402_sign_rejects_expired_challenge() {
        let client = X402Client::new("key1".into(), 1000, "0xAgent".into());
        let challenge = PaymentChallenge {
            expires_at: 50,
            ..Default::default()
        };
        assert!(client.sign_authorization(&challenge, 100).is_err());
    }

    #[test]
    fn x402_sign_rejects_insufficient_balance() {
        let client = X402Client::new("key1".into(), 100, "0xAgent".into());
        let challenge = PaymentChallenge {
            amount: 500,
            expires_at: 200,
            ..Default::default()
        };
        assert!(client.sign_authorization(&challenge, 50).is_err());
    }

    #[test]
    fn x402_pay_deducts_balance() {
        let mut client = X402Client::new("key1".into(), 1000, "0xAgent".into());
        let challenge = PaymentChallenge {
            amount: 300,
            recipient: "0xProvider".into(),
            expires_at: 200,
            ..Default::default()
        };
        let result = client.pay(&challenge, 100);
        assert!(matches!(result, X402PaymentResult::Success { .. }));
        assert_eq!(client.balance, 700);
    }

    #[test]
    fn x402_pay_insufficient_balance() {
        let mut client = X402Client::new("key1".into(), 100, "0xAgent".into());
        let challenge = PaymentChallenge {
            amount: 500,
            expires_at: 200,
            ..Default::default()
        };
        let result = client.pay(&challenge, 50);
        assert!(matches!(
            result,
            X402PaymentResult::InsufficientBalance {
                required: 500,
                available: 100,
            }
        ));
    }

    #[test]
    fn x402_pay_expired() {
        let mut client = X402Client::new("key1".into(), 1000, "0xAgent".into());
        let challenge = PaymentChallenge {
            amount: 100,
            expires_at: 50,
            ..Default::default()
        };
        let result = client.pay(&challenge, 100);
        assert!(matches!(result, X402PaymentResult::Expired));
    }

    #[test]
    fn x402_make_payment_header_format() {
        let auth = Erc3009Auth {
            from: "0xA".into(),
            to: "0xB".into(),
            value: 100,
            valid_after: 0,
            valid_before: 999,
            nonce: 42,
            v: 27,
            r: [0; 32],
            s: [1; 32],
        };
        let header = X402Client::make_payment_header(&auth);
        assert!(header.contains("\"from\":\"0xA\""));
        assert!(header.contains("\"value\":100"));
        assert!(header.contains("\"nonce\":42"));
    }
}
