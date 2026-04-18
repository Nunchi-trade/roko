//! Heuristic, worldview, and falsifier shells for the learning layer.
//!
//! These types mirror the target-state vocabulary in the learning docs.
//! The module is intentionally small: it captures the documented shapes and
//! a few helper constructors, while leaving runtime scoring and calibration
//! behavior to future work.

use chrono::{DateTime, Utc};
use globset::Glob;
use roko_core::project::Language;
use roko_primitives::hdc::HdcVector;

use crate::episode_logger::Episode;

/// Stable identifier for a heuristic record.
pub type HeuristicId = String;

/// Stable identifier for a paper-backed claim.
pub type ClaimId = String;

/// Stable identifier for a stored paper record.
pub type PaperId = String;

/// Opaque hash for an episode receipt that justified a heuristic.
pub type EpisodeHash = String;

/// Opaque hash for a situation used in dissonance tracking.
pub type SituationHash = String;

/// Opaque tool identifier used by heuristic predicates.
pub type ToolId = String;

/// Opaque gate identifier used by heuristic predicates.
pub type GateId = String;

/// Opaque agent role identifier used by heuristic predicates.
pub type Role = String;

/// Human-readable hypothesis extracted from a paper-backed claim.
pub type Hypothesis = String;

/// Provenance descriptor for a paper-backed heuristic source.
#[derive(Debug, Clone)]
pub enum PaperProvenance {
    /// Imported from a local curated note or paper archive.
    LocalNote(String),
    /// Imported from an external venue or citation string.
    ExternalCitation(String),
}

/// Replication status for a paper-backed claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationStatus {
    /// No local replication trials have run yet.
    Unknown,
    /// Local evidence currently supports the paper's reported effect.
    Replicated,
    /// Local evidence is mixed or inconclusive.
    Mixed,
    /// Local evidence diverges from the paper's reported effect.
    Diverged,
}

/// Calibration metadata for a heuristic.
///
/// The docs call for trials, confirmations, violations, Brier score, a
/// last-trial timestamp, and a confidence interval.
#[derive(Debug, Clone)]
pub struct Calibration {
    /// Number of times the heuristic was actually tested.
    pub trials: u32,
    /// Number of trials that confirmed the prediction.
    pub confirmations: u32,
    /// Number of trials that produced a violation signal.
    pub violations: u32,
    /// Brier score for the heuristic's predictions.
    pub brier_score: f64,
    /// Timestamp of the most recent trial.
    pub last_trial_at: DateTime<Utc>,
    /// Confidence interval for the current estimate.
    pub confidence_interval: (f64, f64),
}

impl Calibration {
    /// Construct a zeroed calibration record.
    #[must_use]
    pub fn new() -> Self {
        Self {
            trials: 0,
            confirmations: 0,
            violations: 0,
            brier_score: 0.0,
            last_trial_at: Utc::now(),
            confidence_interval: (0.0, 1.0),
        }
    }
}

impl Default for Calibration {
    fn default() -> Self {
        Self::new()
    }
}

/// Paper-backed source record for a research-informed heuristic.
#[derive(Debug, Clone)]
pub struct Paper {
    /// Stable paper identifier.
    pub id: PaperId,
    /// Human-readable paper title.
    pub title: String,
    /// Authors credited by the source.
    pub authors: Vec<String>,
    /// Optional venue or publication surface.
    pub venue: Option<String>,
    /// Publication year.
    pub year: u16,
    /// Provenance for how the paper entered the local runtime.
    pub provenance: PaperProvenance,
    /// Claims attributed to the paper.
    pub claims: Vec<ClaimId>,
}

impl Paper {
    /// Construct a paper record with the required identity fields.
    #[must_use]
    pub fn new(
        id: impl Into<PaperId>,
        title: impl Into<String>,
        authors: Vec<String>,
        year: u16,
        provenance: PaperProvenance,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            authors,
            venue: None,
            year,
            provenance,
            claims: Vec::new(),
        }
    }
}

/// Matchable condition used to select heuristics and falsifiers.
#[derive(Debug, Clone)]
pub enum Predicate {
    /// Match on a detected programming language.
    LanguageIs(Language),
    /// Match on a path-like glob pattern.
    FileMatches(Glob),
    /// Match when a tool is available.
    ToolAvailable(ToolId),
    /// Match when a gate has recently failed.
    GateRecentlyFailed(GateId),
    /// Match when the active role matches.
    AgentRoleIs(Role),
    /// All nested predicates must hold.
    And(Vec<Predicate>),
    /// Any nested predicate may hold.
    Or(Vec<Predicate>),
    /// Negate a predicate.
    Not(Box<Predicate>),
    /// Match by fingerprint similarity above a threshold.
    SimilarTo {
        /// HDC fingerprint to compare against.
        fingerprint: Box<HdcVector>,
        /// Minimum similarity threshold.
        threshold: f64,
    },
}

impl Predicate {
    /// Create a language predicate.
    #[must_use]
    pub const fn language_is(language: Language) -> Self {
        Self::LanguageIs(language)
    }

    /// Create a file-matching predicate from a glob pattern.
    pub fn file_matches(pattern: impl AsRef<str>) -> Result<Self, globset::Error> {
        Ok(Self::FileMatches(Glob::new(pattern.as_ref())?))
    }

    /// Create a tool-availability predicate.
    #[must_use]
    pub fn tool_available(tool_id: impl Into<ToolId>) -> Self {
        Self::ToolAvailable(tool_id.into())
    }

    /// Create a recent-gate-failure predicate.
    #[must_use]
    pub fn gate_recently_failed(gate_id: impl Into<GateId>) -> Self {
        Self::GateRecentlyFailed(gate_id.into())
    }

    /// Create an agent-role predicate.
    #[must_use]
    pub fn agent_role_is(role: impl Into<Role>) -> Self {
        Self::AgentRoleIs(role.into())
    }

    /// Create a similarity predicate.
    #[must_use]
    pub fn similar_to(fingerprint: HdcVector, threshold: f64) -> Self {
        Self::SimilarTo {
            fingerprint: Box::new(fingerprint),
            threshold,
        }
    }

    /// Conjoin a set of predicates.
    #[must_use]
    pub fn and(predicates: Vec<Self>) -> Self {
        Self::And(predicates)
    }

    /// Disjoin a set of predicates.
    #[must_use]
    pub fn or(predicates: Vec<Self>) -> Self {
        Self::Or(predicates)
    }

    /// Negate a predicate.
    #[must_use]
    pub fn logical_not(predicate: Self) -> Self {
        Self::Not(Box::new(predicate))
    }
}

/// Reusable claim with conditions, prediction, fingerprint, and receipts.
#[derive(Debug, Clone)]
pub struct Heuristic {
    /// Stable heuristic identifier.
    pub id: HeuristicId,
    /// Human-readable claim captured by the heuristic.
    pub claim: String,
    /// Conditions under which the heuristic applies.
    pub preconditions: Vec<Predicate>,
    /// Predicted outcome if the preconditions hold.
    pub prediction: Predicate,
    /// HDC fingerprint associated with the heuristic.
    pub fingerprint: HdcVector,
    /// Calibration record for the heuristic.
    pub calibration: Calibration,
    /// Prior heuristics that this heuristic descends from.
    pub lineage: Vec<HeuristicId>,
    /// Episode hashes that justify the heuristic.
    pub receipts: Vec<EpisodeHash>,
}

impl Heuristic {
    /// Construct a heuristic shell with the required fields.
    #[must_use]
    pub fn new(
        id: impl Into<HeuristicId>,
        claim: impl Into<String>,
        preconditions: Vec<Predicate>,
        prediction: Predicate,
    ) -> Self {
        Self {
            id: id.into(),
            claim: claim.into(),
            preconditions,
            prediction,
            fingerprint: HdcVector::zeros(),
            calibration: Calibration::default(),
            lineage: Vec::new(),
            receipts: Vec::new(),
        }
    }

    /// Add a receipt hash that supports the heuristic.
    pub fn add_receipt(&mut self, receipt: impl Into<EpisodeHash>) {
        self.receipts.push(receipt.into());
    }

    /// Add a heuristic ancestor to the lineage.
    pub fn add_ancestor(&mut self, ancestor: impl Into<HeuristicId>) {
        self.lineage.push(ancestor.into());
    }
}

/// Testable claim derived from a paper-backed source.
#[derive(Debug, Clone)]
pub struct Claim {
    /// Parent paper identifier.
    pub paper: PaperId,
    /// Runtime-facing claim identifier.
    pub id: ClaimId,
    /// Human-readable hypothesis.
    pub hypothesis: Hypothesis,
    /// Predicate that can falsify the claim.
    pub falsifier: Predicate,
    /// Context predicates that scope the claim.
    pub context: Vec<Predicate>,
    /// Calibration state for local evidence against the claim.
    pub calibration: Calibration,
}

impl Claim {
    /// Construct a claim shell.
    #[must_use]
    pub fn new(
        id: impl Into<ClaimId>,
        paper: impl Into<PaperId>,
        hypothesis: impl Into<Hypothesis>,
        falsifier: Predicate,
    ) -> Self {
        Self {
            paper: paper.into(),
            id: id.into(),
            hypothesis: hypothesis.into(),
            falsifier,
            context: Vec::new(),
            calibration: Calibration::default(),
        }
    }
}

/// Calibrator that scores a heuristic against a completed episode.
///
/// The return value is intentionally rich enough to preserve refinement and
/// generalization as new heuristic records rather than in-place mutation.
pub trait Calibrator {
    /// Score a heuristic using an episode as evidence.
    fn score(&self, heuristic: &Heuristic, episode: &Episode) -> Verdict;
}

/// Outcome of calibrating a heuristic against an episode.
#[derive(Debug, Clone)]
pub enum Verdict {
    /// The heuristic's prediction held.
    Confirmed,
    /// The heuristic's prediction failed.
    Violated,
    /// The episode did not exercise the heuristic.
    Irrelevant,
    /// Replace the heuristic with a narrower variant.
    Refined(Heuristic),
    /// Replace the heuristic with a broader variant.
    Generalized(Heuristic),
    /// Retire the heuristic from the hot path.
    Refuted,
}

/// A cluster of heuristics that tends to co-occur in successful episodes.
#[derive(Debug, Clone)]
pub struct Worldview {
    /// Stable worldview identifier.
    pub id: String,
    /// Heuristics that define the worldview's center of gravity.
    pub core_heuristics: Vec<HeuristicId>,
    /// How internally consistent the worldview appears to be.
    pub coherence_score: f64,
    /// How effective the worldview has been in practice.
    pub effectiveness_score: f64,
    /// Domain fingerprint used for matching incoming tasks.
    pub domain_fingerprint: HdcVector,
}

impl Worldview {
    /// Construct a worldview shell with the required fields.
    #[must_use]
    pub fn new(id: impl Into<String>, core_heuristics: Vec<HeuristicId>) -> Self {
        Self {
            id: id.into(),
            core_heuristics,
            coherence_score: 0.0,
            effectiveness_score: 0.0,
            domain_fingerprint: HdcVector::zeros(),
        }
    }
}

/// A pair of heuristics with incompatible predictions for the same situation.
#[derive(Debug, Clone)]
pub struct Dissonance {
    /// The two competing heuristics.
    pub heuristics: [HeuristicId; 2],
    /// The competing predictions that disagree.
    pub predictions: [Predicate; 2],
    /// Situation hash that keyed the contradiction.
    pub situation: SituationHash,
}

impl Dissonance {
    /// Construct a dissonance record.
    #[must_use]
    pub fn new(
        heuristics: [HeuristicId; 2],
        predictions: [Predicate; 2],
        situation: impl Into<SituationHash>,
    ) -> Self {
        Self {
            heuristics,
            predictions,
            situation: situation.into(),
        }
    }
}

/// Runtime replication evidence comparing paper and local effects.
#[derive(Debug, Clone)]
pub struct ReplicationLedger {
    /// Claim tracked by the ledger.
    pub claim: ClaimId,
    /// Effect size reported by the paper.
    pub paper_effect: f64,
    /// Effect size observed locally.
    pub our_effect: f64,
    /// Local sample count backing `our_effect`.
    pub our_n: u32,
    /// Divergence confidence interval.
    pub divergence_ci: (f64, f64),
    /// Current replication status.
    pub status: ReplicationStatus,
}

impl ReplicationLedger {
    /// Construct a replication ledger shell.
    #[must_use]
    pub fn new(claim: impl Into<ClaimId>, paper_effect: f64, our_effect: f64, our_n: u32) -> Self {
        Self {
            claim: claim.into(),
            paper_effect,
            our_effect,
            our_n,
            divergence_ci: (0.0, 0.0),
            status: ReplicationStatus::Unknown,
        }
    }
}
