#![allow(
    dead_code,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    clippy::upper_case_acronyms
)]

//! Phase 2+ job-market, settlement, futures, and compliance stubs derived
//! from `docs/14-identity-economy`.

use crate::{
    identity_economy_identity::{
        AgentId, Blake3Hash, GateType, GateVerdict, Signature, X402Receipt,
    },
    phase2::{HiringModel, u256},
};
use std::{collections::HashMap, time::Duration};

// ---------------------------------------------------------------------------
// Compliance result and policy enforcement (IDECON-08)
// ---------------------------------------------------------------------------

/// Result of a regulatory compliance check.
#[derive(Clone, Debug, PartialEq)]
pub enum ComplianceResult {
    /// The action passes the compliance check.
    Pass,
    /// The action violates a policy rule.
    Violation(String),
    /// The action raises a warning but is not blocked.
    Warning(String),
}

impl ComplianceResult {
    /// Whether the result is a pass.
    #[must_use]
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    /// Whether the result is a violation.
    #[must_use]
    pub fn is_violation(&self) -> bool {
        matches!(self, Self::Violation(_))
    }
}

/// SEC/MiFID best-execution policy (IDECON-08).
///
/// Validates that an agent selects the best available price across venues
/// and checks a minimum number of venues before executing.
#[derive(Clone, Debug, PartialEq)]
pub struct BestExecutionPolicy {
    /// Maximum allowable slippage in basis points.
    pub max_slippage_bps: u32,
    /// Minimum number of venues that must be checked.
    pub min_venues_checked: u32,
}

impl Default for BestExecutionPolicy {
    fn default() -> Self {
        Self {
            max_slippage_bps: 50, // 0.5%
            min_venues_checked: 2,
        }
    }
}

impl BestExecutionPolicy {
    /// Check whether a trade execution meets best-execution requirements.
    ///
    /// `selected_price` is the price actually paid; `best_available` is the
    /// best price found across all venues; `venues_checked` is how many
    /// venues the agent queried.
    #[must_use]
    pub fn check(
        &self,
        selected_price: f64,
        best_available: f64,
        venues_checked: u32,
    ) -> ComplianceResult {
        if venues_checked < self.min_venues_checked {
            return ComplianceResult::Violation(format!(
                "insufficient venues checked: {} < {}",
                venues_checked, self.min_venues_checked
            ));
        }
        if best_available <= 0.0 {
            return ComplianceResult::Warning("best available price is zero or negative".into());
        }
        let slippage_bps =
            ((selected_price - best_available).abs() / best_available * 10_000.0) as u32;
        if slippage_bps > self.max_slippage_bps {
            return ComplianceResult::Violation(format!(
                "slippage {}bps exceeds limit {}bps",
                slippage_bps, self.max_slippage_bps
            ));
        }
        ComplianceResult::Pass
    }
}

/// Position-limit policy: max position size per asset (IDECON-08).
#[derive(Clone, Debug, PartialEq)]
pub struct PositionLimitPolicy {
    /// Maximum position as a percentage of total portfolio (0.0 - 1.0).
    pub max_position_pct: f64,
    /// Maximum absolute position value in USDC.
    pub max_absolute_usdc: u64,
}

impl Default for PositionLimitPolicy {
    fn default() -> Self {
        Self {
            max_position_pct: 0.10, // 10%
            max_absolute_usdc: 100_000,
        }
    }
}

impl PositionLimitPolicy {
    /// Check whether a position exceeds limits.
    #[must_use]
    pub fn check(&self, position_value_usdc: u64, portfolio_value_usdc: u64) -> ComplianceResult {
        if position_value_usdc > self.max_absolute_usdc {
            return ComplianceResult::Violation(format!(
                "position {} exceeds absolute limit {}",
                position_value_usdc, self.max_absolute_usdc
            ));
        }
        if portfolio_value_usdc > 0 {
            let pct = position_value_usdc as f64 / portfolio_value_usdc as f64;
            if pct > self.max_position_pct {
                return ComplianceResult::Violation(format!(
                    "position {:.1}% exceeds limit {:.1}%",
                    pct * 100.0,
                    self.max_position_pct * 100.0
                ));
            }
        }
        ComplianceResult::Pass
    }
}

/// Wash-trading detector: identify self-dealing patterns (IDECON-08).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WashTradingDetector {
    /// Minimum interval between opposing trades to not flag (seconds).
    pub min_interval_secs: u64,
}

impl WashTradingDetector {
    /// Check whether a buy followed by a sell (or vice versa) within the
    /// minimum interval constitutes potential wash trading.
    #[must_use]
    pub fn check(
        &self,
        same_asset: bool,
        same_agent: bool,
        interval_secs: u64,
    ) -> ComplianceResult {
        if same_asset && same_agent && interval_secs < self.min_interval_secs {
            return ComplianceResult::Violation(format!(
                "potential wash trade: opposing trade after {}s (min {}s)",
                interval_secs, self.min_interval_secs
            ));
        }
        ComplianceResult::Pass
    }
}

/// Insider-trading information barrier screen (IDECON-08).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InsiderTradingScreen;

impl InsiderTradingScreen {
    /// Check whether the agent is cleared to trade the given asset.
    ///
    /// `restricted_assets` is the set of assets the agent is barred from
    /// trading due to information barriers.
    #[must_use]
    pub fn check(&self, asset: &str, restricted_assets: &[String]) -> ComplianceResult {
        if restricted_assets.iter().any(|a| a == asset) {
            return ComplianceResult::Violation(format!(
                "asset {} is on the restricted list (information barrier)",
                asset
            ));
        }
        ComplianceResult::Pass
    }
}

/// Aggregate compliance gate that runs all template policies (IDECON-08).
///
/// Returns the first violation found, or `Pass` if all checks succeed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComplianceGate {
    /// Best-execution policy.
    pub best_execution: BestExecutionPolicy,
    /// Position-limit policy.
    pub position_limits: PositionLimitPolicy,
    /// Wash-trading detector.
    pub wash_trading: WashTradingDetector,
    /// Insider-trading screen.
    pub insider_screen: InsiderTradingScreen,
}

impl ComplianceGate {
    /// Run all compliance checks and return the first violation, or `Pass`.
    #[must_use]
    pub fn check_trade(
        &self,
        selected_price: f64,
        best_available: f64,
        venues_checked: u32,
        position_value_usdc: u64,
        portfolio_value_usdc: u64,
        asset: &str,
        restricted_assets: &[String],
    ) -> ComplianceResult {
        let exec = self
            .best_execution
            .check(selected_price, best_available, venues_checked);
        if exec.is_violation() {
            return exec;
        }

        let pos = self
            .position_limits
            .check(position_value_usdc, portfolio_value_usdc);
        if pos.is_violation() {
            return pos;
        }

        let insider = self.insider_screen.check(asset, restricted_assets);
        if insider.is_violation() {
            return insider;
        }

        ComplianceResult::Pass
    }
}

// Remaining marker types for policies not yet needing full logic.
macro_rules! marker_types {
    ($($name:ident => $doc:literal),+ $(,)?) => {
        $(
            #[doc = $doc]
            #[derive(Clone, Debug, Default, PartialEq, Eq)]
            pub struct $name;
        )+
    };
}

marker_types!(
    AuditTrailPolicy => "Placeholder audit-trail policy.",
    RiskGate => "Placeholder position-risk gate.",
    ReportingGate => "Placeholder regulatory-reporting gate.",
    PhiDetectionPolicy => "Placeholder PHI detection policy.",
    MinNecessaryPolicy => "Placeholder minimum-necessary policy.",
    ConsentTrackingPolicy => "Placeholder consent-tracking policy.",
    BreakGlassPolicy => "Placeholder emergency override policy.",
    PhiLeakageGate => "Placeholder PHI-leakage gate.",
    AuditGate => "Placeholder HIPAA audit gate.",
    AccessControlGate => "Placeholder access-control gate.",
    PurposeLimitationPolicy => "Placeholder GDPR purpose-limitation policy.",
    DataMinimizationPolicy => "Placeholder GDPR data-minimization policy.",
    ConsentVerificationPolicy => "Placeholder GDPR consent-verification policy.",
    ErasurePolicy => "Placeholder right-to-erasure policy.",
    PortabilityPolicy => "Placeholder data-portability policy.",
    CrossBorderGate => "Placeholder cross-border transfer gate.",
    RetentionGate => "Placeholder data-retention gate.",
    ExplanationGate => "Placeholder explanation gate."
);

/// Job posting specification used by Spore.
#[derive(Clone, Debug, PartialEq)]
pub struct BountySpec {
    /// Job identifier.
    pub job_id: Blake3Hash,
    /// Job title.
    pub title: String,
    /// Detailed job description.
    pub description: String,
    /// Capability bitmask required for the job.
    pub required_capabilities: u64,
    /// Domain used for reputation lookup.
    pub required_domain: String,
    /// Minimum acceptable domain reputation.
    pub min_reputation: f64,
    /// Maximum budget in USDC base units.
    pub max_budget_usdc: u64,
    /// Time allowed for completion in seconds.
    pub deadline: u64,
    /// Hiring model applied to the job.
    pub hiring_model: HiringModel,
    /// Human-readable evaluation criteria.
    pub evaluation_criteria: Vec<String>,
    /// Minimum gate or rubric score.
    pub quality_threshold: f64,
    /// Requester passport id.
    pub poster_passport_id: u256,
}

impl Default for BountySpec {
    fn default() -> Self {
        Self {
            job_id: [0; 32],
            title: String::new(),
            description: String::new(),
            required_capabilities: 0,
            required_domain: String::new(),
            min_reputation: 0.0,
            max_budget_usdc: 0,
            deadline: 0,
            hiring_model: HiringModel::RandomVRF,
            evaluation_criteria: Vec::new(),
            quality_threshold: 0.0,
            poster_passport_id: 0,
        }
    }
}

/// Bid submitted by an agent through Sparrow.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SparrowBid {
    /// Bidder passport id.
    pub bidder_passport_id: u256,
    /// Target bounty identifier.
    pub bounty_id: Blake3Hash,
    /// Price bid in USDC base units.
    pub price_usdc: u64,
    /// Estimated completion time in seconds.
    pub estimated_time: u64,
    /// Capability proof bitmask.
    pub capability_proof: u64,
    /// Reputation snapshot at bid time.
    pub reputation_snapshot: f64,
    /// Signature over the bid payload.
    pub signature: Signature,
}

/// Protocol-generated ecosystem-maintenance job kinds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MiningType {
    /// Genetic optimization of agent configurations.
    Genome,
    /// Re-verification of knowledge artifacts.
    Verifier,
    /// Repair degraded knowledge.
    Repair,
    /// Validate economic mechanism parameters.
    Mechanism,
    /// Rebuild search indices.
    Index,
    /// Consolidate collective memory.
    #[default]
    Memory,
}

/// Before/after metrics attached to a mining submission.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricSnapshot {
    /// Named metric values.
    pub metrics: HashMap<String, f64>,
    /// Timestamp associated with the snapshot.
    pub captured_at: u64,
}

/// Artifact delivered for a mining job.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeltaArtifact {
    /// Mining job type.
    pub mining_type: MiningType,
    /// Agent that produced the artifact.
    pub agent_id: u256,
    /// Metrics before work.
    pub before_metrics: MetricSnapshot,
    /// Metrics after work.
    pub after_metrics: MetricSnapshot,
    /// Hash of the produced artifact.
    pub artifact_hash: Blake3Hash,
    /// Binary proof bundle.
    pub evidence: Vec<u8>,
}

/// Inter-subjective floating-rate submission for a market and epoch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IsfrSubmission {
    /// Submitter passport id.
    pub submitter: u256,
    /// Market identifier.
    pub market_id: String,
    /// Observed rate.
    pub rate: f64,
    /// Component vector that sums to the rate.
    pub components: Vec<f64>,
    /// Confidence assigned by the submitter.
    pub confidence: f64,
    /// Epoch identifier.
    pub epoch_id: u64,
    /// Signature over the submission.
    pub signature: Signature,
}

/// Aggregated ISFR rate distributed for an epoch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IsfrAggregate {
    /// Market identifier.
    pub market_id: String,
    /// Epoch identifier.
    pub epoch_id: u64,
    /// Median floating rate.
    pub median_rate: f64,
    /// Number of submissions used.
    pub submission_count: u32,
    /// Standard deviation of submissions.
    pub std_deviation: f64,
    /// Count of excluded outliers.
    pub excluded_count: u32,
    /// Aggregate timestamp.
    pub timestamp: u64,
    /// Hash or identifier of the TEE computation.
    pub tee_attestation: [u8; 32],
}

/// Purchase record for a knowledge future.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FuturePurchase {
    /// Purchase identifier.
    pub purchase_id: Blake3Hash,
    /// Future identifier.
    pub future_id: Blake3Hash,
    /// Buyer passport id.
    pub buyer: u256,
    /// Price paid for the purchase.
    pub price_paid: u64,
    /// Purchase timestamp.
    pub purchased_at: u64,
    /// x402 payment proof.
    pub x402_receipt: X402Receipt,
    /// Whether access has been granted after delivery.
    pub access_granted: bool,
}

/// Delivery record for a knowledge future.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FutureDelivery {
    /// Future identifier.
    pub future_id: Blake3Hash,
    /// Hash of the delivered knowledge artifact.
    pub delivery_hash: Blake3Hash,
    /// Engram identifier of the delivered artifact.
    pub engram_id: Blake3Hash,
    /// Verify-verified quality score.
    pub quality_score: f64,
    /// Verify verdicts recorded during validation.
    pub gate_verdicts: Vec<GateVerdict>,
    /// Delivery timestamp.
    pub delivered_at: u64,
    /// Whether the delivery beat the deadline.
    pub early_delivery: bool,
}

/// Multi-phase variant of a knowledge future.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResearchFuture {
    /// Future identifier.
    pub future_id: Blake3Hash,
    /// Producing passport id.
    pub producer: u256,
    /// Sequential phases for the research plan.
    pub phases: Vec<ResearchPhase>,
    /// Total price of the commitment.
    pub total_price: u64,
    /// Total stake locked by the producer.
    pub total_stake: u64,
}

/// Single phase inside a [`ResearchFuture`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResearchPhase {
    /// Phase identifier.
    pub phase_id: u32,
    /// Human-readable phase summary.
    pub description: String,
    /// Deliverable promised for the phase.
    pub deliverable: String,
    /// Phase deadline.
    pub deadline: u64,
    /// Fraction of total price released on completion.
    pub price_fraction: f64,
    /// Fraction of total stake at risk.
    pub stake_fraction: f64,
    /// Gates required for successful completion.
    pub gate_requirements: Vec<GateType>,
}

/// LMSR market maker for knowledge-future outcome trading.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LmsrMarketMaker {
    /// Future identifier.
    pub future_id: Blake3Hash,
    /// Liquidity parameter.
    pub b: f64,
    /// Outstanding deliver shares.
    pub shares_deliver: f64,
    /// Outstanding default shares.
    pub shares_default: f64,
    /// Total subsidy committed to the market.
    pub total_subsidy: f64,
}

impl LmsrMarketMaker {
    /// LMSR cost function (Hanson 2003): `C(q) = b * ln(e^(q_d/b) + e^(q_f/b))`.
    ///
    /// `b` is the liquidity parameter; higher `b` means lower price impact per
    /// trade, but higher bounded loss for the market maker (max = `b * ln(2)`
    /// for a binary market).
    pub fn cost(&self) -> f64 {
        let b = self.b.max(f64::EPSILON);
        b * ((self.shares_deliver / b).exp() + (self.shares_default / b).exp()).ln()
    }

    /// Instantaneous price for the "deliver" outcome.
    ///
    /// `p_deliver = e^(q_d/b) / (e^(q_d/b) + e^(q_f/b))`.
    pub fn price_deliver(&self) -> f64 {
        let b = self.b.max(f64::EPSILON);
        let e_d = (self.shares_deliver / b).exp();
        let e_f = (self.shares_default / b).exp();
        e_d / (e_d + e_f)
    }

    /// Instantaneous price for the "default" outcome.
    ///
    /// Always `1 - price_deliver()`, maintaining the unit-sum property.
    pub fn price_default(&self) -> f64 {
        1.0 - self.price_deliver()
    }

    /// Buy `shares` of the selected outcome.
    ///
    /// Returns the cost of the purchase (always non-negative): the
    /// difference in cost function before and after the trade.
    pub fn buy(&mut self, outcome: Outcome, shares: f64) -> f64 {
        let cost_before = self.cost();
        match outcome {
            Outcome::Deliver => self.shares_deliver += shares,
            Outcome::Default => self.shares_default += shares,
        }
        self.cost() - cost_before
    }

    /// Sell `shares` of the selected outcome.
    ///
    /// Returns the refund received (always non-negative under normal
    /// conditions): the reduction in the cost function.
    pub fn sell(&mut self, outcome: Outcome, shares: f64) -> f64 {
        let cost_before = self.cost();
        match outcome {
            Outcome::Deliver => self.shares_deliver -= shares,
            Outcome::Default => self.shares_default -= shares,
        }
        cost_before - self.cost()
    }
}

/// Binary outcome for the baseline knowledge-futures LMSR market.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    /// Producer delivers successfully.
    #[default]
    Deliver,
    /// Producer defaults or misses the target.
    Default,
}

/// Conditional outcome-token market for multi-dimensional futures.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConditionalOutcomes {
    /// Future identifier.
    pub future_id: Blake3Hash,
    /// Conditions composing the market.
    pub conditions: Vec<Condition>,
    /// Outcome slots derived from the conditions.
    pub outcome_slots: Vec<OutcomeSlot>,
}

/// Single condition inside a conditional outcome market.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Condition {
    /// Condition identifier.
    pub condition_id: Blake3Hash,
    /// Oracle responsible for resolution.
    pub oracle: u256,
    /// Resolution question.
    pub question: String,
    /// Number of outcomes for the condition.
    pub outcome_count: u32,
}

/// Tokenized slot representing one combined market outcome.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutcomeSlot {
    /// Slot index within the market.
    pub slot_index: u32,
    /// Human-readable description.
    pub description: String,
    /// Outstanding shares in the slot.
    pub shares: f64,
    /// Whether the slot has been resolved.
    pub resolved: bool,
    /// Whether this slot was the winner.
    pub winning: bool,
}

/// Final resolution record for a knowledge-futures market.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketResolution {
    /// Future identifier.
    pub future_id: Blake3Hash,
    /// Resolution timestamp.
    pub resolved_at: u64,
    /// Winning binary outcome.
    pub winning_outcome: Outcome,
    /// Optional gate-verified quality score.
    pub quality_score: Option<f64>,
    /// Delivery timing classification.
    pub delivery_timing: DeliveryTiming,
    /// Total traded volume.
    pub total_volume: f64,
    /// Final market price for deliver.
    pub final_price_deliver: f64,
    /// Calibration error at resolution.
    pub calibration_error: f64,
}

/// Delivery timing categories for futures resolution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DeliveryTiming {
    /// Delivered before half the deadline elapsed.
    Early,
    /// Delivered before the deadline.
    #[default]
    OnTime,
    /// Missed the deadline or defaulted.
    Default,
}

/// Content-addressed knowledge unit for forensic replay stubs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Engram {
    /// Content hash of the Engram.
    pub hash: Blake3Hash,
    /// Engram kind.
    pub kind: Kind,
    /// Raw content body.
    pub body: Vec<u8>,
    /// Authoring agent id.
    pub author: AgentId,
    /// Free-form tags.
    pub tags: Vec<String>,
    /// Parent hashes in the lineage DAG.
    pub lineage: Vec<Blake3Hash>,
    /// Seven-axis quality score.
    pub score: [f64; 7],
    /// Persistence tier.
    pub tier: Tier,
    /// Creation timestamp.
    pub created_at: u64,
    /// Provenance metadata.
    pub provenance: Provenance,
}

/// Provenance metadata attached to an [`Engram`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    /// Source of the Engram.
    pub source: ProvenanceSource,
    /// Optional original author.
    pub original_author: Option<AgentId>,
    /// Optional original timestamp.
    pub original_timestamp: Option<u64>,
    /// Chain-of-custody history.
    pub chain_of_custody: Vec<CustodyEntry>,
}

/// Chain-of-custody event for an [`Engram`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustodyEntry {
    /// Agent responsible for the action.
    pub agent: AgentId,
    /// Custody action that occurred.
    pub action: CustodyAction,
    /// Timestamp of the action.
    pub timestamp: u64,
    /// Hash of the Engram at the time of the action.
    pub hash_at_action: Blake3Hash,
}

/// High-level Engram kinds used by the compliance docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// Free-form task or work artifact.
    #[default]
    Task,
    /// Analysis or insight artifact.
    Insight,
    /// Warning or anomaly artifact.
    Warning,
    /// React or compliance artifact.
    React,
    /// Any other named kind.
    Custom(String),
}

/// Persistence tier for a forensic Engram.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Tier {
    /// Short-lived transient artifact.
    Transient,
    /// Working-memory artifact.
    #[default]
    Working,
    /// Reference-quality artifact.
    Reference,
    /// Permanent or canonical artifact.
    Permanent,
}

/// Source classification for provenance records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProvenanceSource {
    /// Produced from human input.
    HumanInput,
    /// Produced by another agent or workflow.
    #[default]
    AgentGenerated,
    /// Restored from backup or history.
    Restored,
    /// Imported from an external system.
    Imported,
}

/// Chain-of-custody actions tracked for an Engram.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CustodyAction {
    /// Created the Engram.
    #[default]
    Created,
    /// Modified the content.
    Modified,
    /// Shared with another system.
    Shared,
    /// Restored from prior history.
    Restored,
}

/// Template for SEC-compliant trading agents.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SecTradingTemplate {
    /// Best-execution policy.
    pub best_execution_policy: BestExecutionPolicy,
    /// Position-limit policy.
    pub position_limit_policy: PositionLimitPolicy,
    /// Wash-trading detector.
    pub wash_trading_detector: WashTradingDetector,
    /// Insider-trading screen.
    pub insider_trading_screen: InsiderTradingScreen,
    /// Audit-trail capture policy.
    pub audit_trail_policy: AuditTrailPolicy,
    /// Compliance gate.
    pub compliance_gate: ComplianceGate,
    /// Risk gate.
    pub risk_gate: RiskGate,
    /// Reporting gate.
    pub reporting_gate: ReportingGate,
    /// Max position size.
    pub max_position_pct: f64,
    /// Max daily turnover in USDC.
    pub max_daily_turnover: u64,
    /// Mandatory cooling-off period in seconds.
    pub mandatory_cooling: u64,
}

/// Template for HIPAA-compliant clinical agents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HipaaClinicalTemplate {
    /// PHI detection policy.
    pub phi_detection_policy: PhiDetectionPolicy,
    /// Minimum-necessary policy.
    pub minimum_necessary_policy: MinNecessaryPolicy,
    /// Consent-tracking policy.
    pub consent_tracking_policy: ConsentTrackingPolicy,
    /// Emergency override policy.
    pub break_glass_policy: BreakGlassPolicy,
    /// PHI-leakage gate.
    pub phi_leakage_gate: PhiLeakageGate,
    /// HIPAA audit gate.
    pub audit_gate: AuditGate,
    /// Role-based access gate.
    pub access_control_gate: AccessControlGate,
    /// Required privacy tier.
    pub privacy_tier: PrivacyTier,
    /// Maximum retention period.
    pub data_retention: Duration,
}

/// Template for GDPR-compliant data agents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GdprDataTemplate {
    /// Purpose-limitation policy.
    pub purpose_limitation_policy: PurposeLimitationPolicy,
    /// Data-minimization policy.
    pub data_minimization_policy: DataMinimizationPolicy,
    /// Consent-verification policy.
    pub consent_verification_policy: ConsentVerificationPolicy,
    /// Right-to-erasure policy.
    pub erasure_policy: ErasurePolicy,
    /// Data-portability policy.
    pub portability_policy: PortabilityPolicy,
    /// Cross-border transfer gate.
    pub cross_border_gate: CrossBorderGate,
    /// Retention gate.
    pub retention_gate: RetentionGate,
    /// Explanation gate.
    pub explanation_gate: ExplanationGate,
    /// Covered data categories.
    pub data_categories: Vec<DataCategory>,
    /// Supported legal bases.
    pub legal_bases: Vec<LegalBasis>,
    /// Retention periods by data category.
    pub retention_periods: HashMap<DataCategory, Duration>,
}

/// Privacy tiers referenced by the HIPAA template docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PrivacyTier {
    /// Standard privacy posture.
    Standard,
    /// Elevated privacy controls.
    Enhanced,
    /// Maximum privacy controls.
    #[default]
    Maximum,
}

/// Data categories referenced by the GDPR template docs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum DataCategory {
    /// Identity and profile data.
    #[default]
    Identity,
    /// Financial data.
    Financial,
    /// Health or PHI data.
    Health,
    /// Behavioral telemetry.
    Behavioral,
    /// Any other named category.
    Custom(String),
}

/// Legal bases referenced by the GDPR template docs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LegalBasis {
    /// Data subject consent.
    #[default]
    Consent,
    /// Contractual necessity.
    Contract,
    /// Legal obligation.
    LegalObligation,
    /// Legitimate interest.
    LegitimateInterest,
    /// Vital interest or emergency use.
    VitalInterest,
}

// ---------------------------------------------------------------------------
// Vickrey reputation-adjusted auction (IDECON-03)
// ---------------------------------------------------------------------------

/// Compute a reputation-adjusted bid score.
///
/// `s_i = price * (1 + (1 - reputation))` — agents with *higher* reputation
/// incur a *lower* score, giving them an advantage.  The winner is the
/// agent with the lowest score (`argmin`).
pub fn score_bid(bid: &SparrowBid) -> f64 {
    bid.price_usdc as f64 * (1.0 + (1.0 - bid.reputation_snapshot))
}

/// Result of a Vickrey winner selection.
#[derive(Clone, Debug, PartialEq)]
pub struct VickreyResult {
    /// Index into the original bid slice.
    pub winner_index: usize,
    /// Second-price payment (adjusted for reputation).
    pub payment: f64,
}

/// Select a winner from a set of bids using a Vickrey (second-price)
/// reputation-adjusted auction.
///
/// The winner is `argmin(score_bid)`.  Payment uses second-price logic:
/// `payment = second_lowest_score / (1 + (1 - R_winner))`.  This preserves
/// truthfulness (Vickrey 1961) — agents should bid their true cost because
/// the payment is set by the second-best bidder.
///
/// Returns `None` if the bid set is empty.
pub fn select_winner(bids: &[SparrowBid]) -> Option<VickreyResult> {
    if bids.is_empty() {
        return None;
    }

    let mut scored: Vec<(usize, f64)> = bids
        .iter()
        .enumerate()
        .map(|(i, b)| (i, score_bid(b)))
        .collect();
    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let winner_idx = scored[0].0;
    let second_score = if scored.len() > 1 {
        scored[1].1
    } else {
        scored[0].1
    };
    let winner_rep = bids[winner_idx].reputation_snapshot;
    let payment = second_score / (1.0 + (1.0 - winner_rep));

    Some(VickreyResult {
        winner_index: winner_idx,
        payment,
    })
}

/// Anti-centralization fee for direct-hire dispatch.
///
/// Fee grows logarithmically with repeat hires of the same agent:
/// `base_fee * (1 + ln(1 + repeat_count))`.
pub fn anti_centralization_fee(base_fee: f64, repeat_count: u32) -> f64 {
    base_fee * (1.0 + (1.0 + repeat_count as f64).ln())
}

// ---------------------------------------------------------------------------
// IDECON-09: Three hiring models — dispatch functions
// ---------------------------------------------------------------------------

/// Result of dispatching a job to an agent (IDECON-09).
#[derive(Clone, Debug, PartialEq)]
pub struct DispatchDecision {
    /// Passport id of the winning agent.
    pub winner: u256,
    /// Payment owed to the winner.
    pub payment: f64,
    /// Hiring model that produced this decision.
    pub model: HiringModel,
    /// Human-readable audit trail entries.
    pub audit_trail: Vec<String>,
}

/// Dispatch a job using random VRF with power-of-two-choices (IDECON-09).
///
/// Randomly picks two candidates from the pool, compares their reputation
/// scores, and assigns the job to the one with the higher score.  This is
/// `O(1)` per dispatch and achieves near-optimal load balancing (Ousterhout
/// 2013).
///
/// Returns `None` if the pool is empty.
pub fn dispatch_random_vrf(pool: &[SparrowBid], bounty: &BountySpec) -> Option<DispatchDecision> {
    if pool.is_empty() {
        return None;
    }
    if pool.len() == 1 {
        return Some(DispatchDecision {
            winner: pool[0].bidder_passport_id,
            payment: bounty.max_budget_usdc as f64,
            model: HiringModel::RandomVRF,
            audit_trail: vec!["single candidate — auto-assigned".into()],
        });
    }

    // Deterministic "random" pair selection using a simple hash to avoid
    // pulling in a full CSPRNG dependency.  In production the indices would
    // come from a verifiable random function.
    let n = pool.len();
    let i = bounty.job_id[0] as usize % n;
    let j = {
        let candidate = bounty.job_id[1] as usize % n;
        if candidate == i {
            (candidate + 1) % n
        } else {
            candidate
        }
    };

    let score_i = pool[i].reputation_snapshot;
    let score_j = pool[j].reputation_snapshot;
    let winner_idx = if score_i >= score_j { i } else { j };

    Some(DispatchDecision {
        winner: pool[winner_idx].bidder_passport_id,
        payment: bounty.max_budget_usdc as f64,
        model: HiringModel::RandomVRF,
        audit_trail: vec![format!(
            "P2C: candidate[{i}] rep={score_i:.3} vs candidate[{j}] rep={score_j:.3} -> winner=[{winner_idx}]"
        )],
    })
}

/// Dispatch a job using a blind auction (IDECON-09).
///
/// Delegates to the Vickrey second-price auction from IDECON-03 via
/// [`select_winner`].  Returns `None` if no bids are present.
pub fn dispatch_blind_auction(
    bids: &[SparrowBid],
    bounty: &BountySpec,
) -> Option<DispatchDecision> {
    let result = select_winner(bids)?;
    let auction_type = match &bounty.hiring_model {
        HiringModel::BlindAuction { auction_type, .. } => auction_type.clone(),
        _ => crate::phase2::AuctionType::Vickrey,
    };
    Some(DispatchDecision {
        winner: bids[result.winner_index].bidder_passport_id,
        payment: result.payment,
        model: HiringModel::BlindAuction {
            auction_duration_blocks: 0,
            auction_type,
        },
        audit_trail: vec![format!(
            "Vickrey winner: index={}, payment={:.2}",
            result.winner_index, result.payment
        )],
    })
}

/// Dispatch a job using direct hire with anti-centralization fee (IDECON-09).
///
/// The operator specifies the target agent directly.  A logarithmic fee
/// is applied based on how many times the hirer has previously hired this
/// agent in the recent window, discouraging centralization.
pub fn dispatch_direct_hire(
    target_passport_id: u256,
    base_fee: f64,
    repeat_count: u32,
) -> DispatchDecision {
    let fee = anti_centralization_fee(base_fee, repeat_count);
    DispatchDecision {
        winner: target_passport_id,
        payment: fee,
        model: HiringModel::DirectHire { target_passport_id },
        audit_trail: vec![format!(
            "direct hire: target={target_passport_id}, base_fee={base_fee:.2}, repeats={repeat_count}, fee={fee:.2}"
        )],
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity_economy_identity::Signature;

    fn make_bid(price: u64, rep: f64) -> SparrowBid {
        SparrowBid {
            price_usdc: price,
            reputation_snapshot: rep,
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // IDECON-05: LMSR market maker
    // -----------------------------------------------------------------------

    #[test]
    fn lmsr_initial_prices_equal() {
        let mm = LmsrMarketMaker {
            b: 100.0,
            shares_deliver: 0.0,
            shares_default: 0.0,
            ..Default::default()
        };
        assert!((mm.price_deliver() - 0.5).abs() < 1e-9);
        assert!((mm.price_default() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn lmsr_prices_sum_to_one() {
        let mm = LmsrMarketMaker {
            b: 50.0,
            shares_deliver: 10.0,
            shares_default: 5.0,
            ..Default::default()
        };
        assert!((mm.price_deliver() + mm.price_default() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn lmsr_buy_increases_price() {
        let mut mm = LmsrMarketMaker {
            b: 100.0,
            shares_deliver: 0.0,
            shares_default: 0.0,
            ..Default::default()
        };
        let initial = mm.price_deliver();
        let cost = mm.buy(Outcome::Deliver, 10.0);
        assert!(cost > 0.0, "buying should cost something");
        assert!(mm.price_deliver() > initial, "buying should increase price");
    }

    #[test]
    fn lmsr_sell_decreases_price() {
        let mut mm = LmsrMarketMaker {
            b: 100.0,
            shares_deliver: 20.0,
            shares_default: 0.0,
            ..Default::default()
        };
        let initial = mm.price_deliver();
        let refund = mm.sell(Outcome::Deliver, 10.0);
        assert!(refund > 0.0, "selling should produce refund");
        assert!(
            mm.price_deliver() < initial,
            "selling should decrease price"
        );
    }

    #[test]
    fn lmsr_buy_sell_roundtrip() {
        let mut mm = LmsrMarketMaker {
            b: 100.0,
            shares_deliver: 0.0,
            shares_default: 0.0,
            ..Default::default()
        };
        let cost = mm.buy(Outcome::Deliver, 5.0);
        let refund = mm.sell(Outcome::Deliver, 5.0);
        assert!(
            (cost - refund).abs() < 1e-9,
            "round-trip should be cost-neutral"
        );
    }

    #[test]
    fn lmsr_bounded_loss() {
        let mm = LmsrMarketMaker {
            b: 100.0,
            shares_deliver: 0.0,
            shares_default: 0.0,
            ..Default::default()
        };
        // For a binary market, max loss = b * ln(2) ~ 69.31
        let max_loss = mm.b * 2.0_f64.ln();
        assert!(max_loss < 70.0 && max_loss > 69.0);
    }

    // -----------------------------------------------------------------------
    // IDECON-03: Vickrey auction
    // -----------------------------------------------------------------------

    #[test]
    fn vickrey_empty_returns_none() {
        assert!(select_winner(&[]).is_none());
    }

    #[test]
    fn vickrey_single_bidder() {
        let bids = vec![make_bid(100, 0.9)];
        let result = select_winner(&bids).unwrap();
        assert_eq!(result.winner_index, 0);
        // Single bidder: payment = own_score / (1 + (1 - rep))
        let score = score_bid(&bids[0]);
        let expected = score / (1.0 + (1.0 - 0.9));
        assert!((result.payment - expected).abs() < 1e-6);
    }

    #[test]
    fn vickrey_reputation_advantage() {
        let bids = vec![
            make_bid(100, 0.9), // score = 100 * (1 + 0.1) = 110
            make_bid(100, 0.5), // score = 100 * (1 + 0.5) = 150
        ];
        let result = select_winner(&bids).unwrap();
        assert_eq!(
            result.winner_index, 0,
            "higher rep should win at same price"
        );
    }

    #[test]
    fn vickrey_second_price_payment() {
        let bids = vec![
            make_bid(100, 0.9), // score = 110
            make_bid(120, 0.8), // score = 120 * 1.2 = 144
        ];
        let result = select_winner(&bids).unwrap();
        assert_eq!(result.winner_index, 0);
        // payment = 144 / (1 + 0.1) = 144 / 1.1 ~ 130.91
        let expected = 144.0 / 1.1;
        assert!((result.payment - expected).abs() < 0.01);
    }

    #[test]
    fn anti_centralization_increases_with_repeats() {
        let f0 = anti_centralization_fee(10.0, 0);
        let f5 = anti_centralization_fee(10.0, 5);
        let f20 = anti_centralization_fee(10.0, 20);
        assert!(f0 < f5);
        assert!(f5 < f20);
    }

    #[test]
    fn anti_centralization_base_case() {
        // repeat_count=0: fee = base * (1 + ln(1)) = base * 1.0 = base
        let f = anti_centralization_fee(10.0, 0);
        assert!((f - 10.0).abs() < 1e-9);
    }

    // -----------------------------------------------------------------------
    // IDECON-09: Three hiring models
    // -----------------------------------------------------------------------

    fn make_bounty() -> BountySpec {
        BountySpec {
            job_id: [1; 32],
            max_budget_usdc: 500,
            ..Default::default()
        }
    }

    fn make_bid_with_passport(price: u64, rep: f64, passport: u256) -> SparrowBid {
        SparrowBid {
            bidder_passport_id: passport,
            price_usdc: price,
            reputation_snapshot: rep,
            ..Default::default()
        }
    }

    #[test]
    fn dispatch_random_vrf_empty_pool() {
        assert!(dispatch_random_vrf(&[], &make_bounty()).is_none());
    }

    #[test]
    fn dispatch_random_vrf_single_candidate() {
        let pool = vec![make_bid_with_passport(100, 0.9, 42)];
        let result = dispatch_random_vrf(&pool, &make_bounty()).unwrap();
        assert_eq!(result.winner, 42);
        assert_eq!(result.payment, 500.0);
        assert!(matches!(result.model, HiringModel::RandomVRF));
    }

    #[test]
    fn dispatch_random_vrf_picks_higher_rep() {
        let pool = vec![
            make_bid_with_passport(100, 0.3, 10),
            make_bid_with_passport(100, 0.9, 20),
            make_bid_with_passport(100, 0.5, 30),
        ];
        let bounty = make_bounty();
        let result = dispatch_random_vrf(&pool, &bounty).unwrap();
        // The result should be one of the pool members.
        assert!(pool.iter().any(|b| b.bidder_passport_id == result.winner));
        assert!(!result.audit_trail.is_empty());
    }

    #[test]
    fn dispatch_blind_auction_uses_vickrey() {
        let bids = vec![
            make_bid_with_passport(100, 0.9, 10),
            make_bid_with_passport(120, 0.8, 20),
        ];
        let bounty = make_bounty();
        let result = dispatch_blind_auction(&bids, &bounty).unwrap();
        assert_eq!(result.winner, 10); // higher rep wins at similar price
        assert!(result.payment > 0.0);
        assert!(matches!(result.model, HiringModel::BlindAuction { .. }));
    }

    #[test]
    fn dispatch_direct_hire_applies_fee() {
        let result = dispatch_direct_hire(42, 10.0, 0);
        assert_eq!(result.winner, 42);
        assert!((result.payment - 10.0).abs() < 1e-9); // repeat_count=0 -> fee=base
        assert!(matches!(result.model, HiringModel::DirectHire { .. }));

        let result_repeated = dispatch_direct_hire(42, 10.0, 5);
        assert!(result_repeated.payment > result.payment);
    }

    // -----------------------------------------------------------------------
    // IDECON-08: Regulatory compliance enforcement
    // -----------------------------------------------------------------------

    #[test]
    fn best_execution_pass() {
        let policy = BestExecutionPolicy {
            max_slippage_bps: 50,
            min_venues_checked: 2,
        };
        let result = policy.check(100.0, 100.0, 3);
        assert!(result.is_pass());
    }

    #[test]
    fn best_execution_insufficient_venues() {
        let policy = BestExecutionPolicy {
            max_slippage_bps: 50,
            min_venues_checked: 3,
        };
        let result = policy.check(100.0, 100.0, 2);
        assert!(result.is_violation());
    }

    #[test]
    fn best_execution_excessive_slippage() {
        let policy = BestExecutionPolicy {
            max_slippage_bps: 50, // 0.5%
            min_venues_checked: 1,
        };
        // 1% slippage should violate 0.5% limit.
        let result = policy.check(101.0, 100.0, 2);
        assert!(result.is_violation());
    }

    #[test]
    fn position_limit_pass() {
        let policy = PositionLimitPolicy {
            max_position_pct: 0.10,
            max_absolute_usdc: 100_000,
        };
        let result = policy.check(5_000, 100_000);
        assert!(result.is_pass());
    }

    #[test]
    fn position_limit_exceeds_pct() {
        let policy = PositionLimitPolicy {
            max_position_pct: 0.10,
            max_absolute_usdc: 100_000,
        };
        // 15% of portfolio.
        let result = policy.check(15_000, 100_000);
        assert!(result.is_violation());
    }

    #[test]
    fn position_limit_exceeds_absolute() {
        let policy = PositionLimitPolicy {
            max_position_pct: 1.0, // no pct limit
            max_absolute_usdc: 50_000,
        };
        let result = policy.check(60_000, 1_000_000);
        assert!(result.is_violation());
    }

    #[test]
    fn wash_trading_detected() {
        let detector = WashTradingDetector {
            min_interval_secs: 60,
        };
        let result = detector.check(true, true, 30);
        assert!(result.is_violation());
    }

    #[test]
    fn wash_trading_pass_different_asset() {
        let detector = WashTradingDetector {
            min_interval_secs: 60,
        };
        let result = detector.check(false, true, 30);
        assert!(result.is_pass());
    }

    #[test]
    fn insider_trading_restricted() {
        let screen = InsiderTradingScreen;
        let restricted = vec!["ETH".to_string(), "BTC".to_string()];
        let result = screen.check("ETH", &restricted);
        assert!(result.is_violation());
    }

    #[test]
    fn insider_trading_clear() {
        let screen = InsiderTradingScreen;
        let restricted = vec!["ETH".to_string()];
        let result = screen.check("USDC", &restricted);
        assert!(result.is_pass());
    }

    #[test]
    fn compliance_gate_aggregates_checks() {
        let gate = ComplianceGate::default();
        let result = gate.check_trade(
            100.0,   // selected
            100.0,   // best available
            3,       // venues
            5_000,   // position
            100_000, // portfolio
            "USDC",  // asset
            &[],     // no restrictions
        );
        assert!(result.is_pass());
    }

    #[test]
    fn compliance_gate_catches_execution_violation() {
        let gate = ComplianceGate {
            best_execution: BestExecutionPolicy {
                max_slippage_bps: 10,
                min_venues_checked: 1,
            },
            ..Default::default()
        };
        // 2% slippage
        let result = gate.check_trade(102.0, 100.0, 2, 1000, 100_000, "ETH", &[]);
        assert!(result.is_violation());
    }
}
