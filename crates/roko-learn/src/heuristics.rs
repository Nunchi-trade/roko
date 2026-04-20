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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PaperProvenance {
    /// Imported from a local curated note or paper archive.
    LocalNote(String),
    /// Imported from an external venue or citation string.
    ExternalCitation(String),
}

/// Replication status for a paper-backed claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Predicate {
    /// Match on a detected programming language.
    LanguageIs(Language),
    /// Match on a path-like glob pattern (stored as the pattern string).
    FileMatches(String),
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
    ///
    /// Validates the pattern at construction time and stores the string form
    /// so that the predicate is cheaply serializable.
    pub fn file_matches(pattern: impl AsRef<str>) -> Result<Self, globset::Error> {
        // Validate that the pattern is well-formed.
        Glob::new(pattern.as_ref())?;
        Ok(Self::FileMatches(pattern.as_ref().to_owned()))
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Heuristic {
    /// Stable heuristic identifier.
    pub id: HeuristicId,
    /// Human-readable claim captured by the heuristic.
    pub claim: String,
    /// Conditions under which the heuristic applies.
    pub preconditions: Vec<Predicate>,
    /// Predicted outcome if the preconditions hold.
    pub prediction: Predicate,
    /// Optional falsifier: a condition that, if observed, invalidates this
    /// heuristic. When the falsifier is triggered, the heuristic should be
    /// automatically contradicted.
    #[serde(default)]
    pub falsifier: Option<Predicate>,
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
            falsifier: None,
            fingerprint: HdcVector::zeros(),
            calibration: Calibration::default(),
            lineage: Vec::new(),
            receipts: Vec::new(),
        }
    }

    /// Set the falsifier predicate for this heuristic.
    #[must_use]
    pub fn with_falsifier(mut self, falsifier: Predicate) -> Self {
        self.falsifier = Some(falsifier);
        self
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

// ─── Worldview clustering and dissonance detection ──────────────────────────

/// Cluster heuristics into worldviews based on HDC fingerprint similarity.
///
/// Uses a greedy k-medoids approach: pick initial medoids by maximizing
/// pairwise distance, then assign each heuristic to the closest medoid.
/// Heuristics within the same cluster tend to co-occur in successful
/// episodes.
///
/// Returns `k` clusters (or fewer if there are fewer heuristics than `k`).
#[must_use]
pub fn cluster_worldviews(heuristics: &[Heuristic], k: usize) -> Vec<Worldview> {
    if heuristics.is_empty() || k == 0 {
        return Vec::new();
    }

    let k = k.min(heuristics.len());

    // Pick initial medoids greedily (furthest-point initialization).
    let mut medoid_indices: Vec<usize> = Vec::with_capacity(k);
    medoid_indices.push(0);

    while medoid_indices.len() < k {
        let mut best_idx = 0;
        let mut best_min_dist = f64::NEG_INFINITY;

        for (i, h) in heuristics.iter().enumerate() {
            if medoid_indices.contains(&i) {
                continue;
            }
            // Distance to nearest existing medoid.
            let min_dist = medoid_indices
                .iter()
                .map(|&m| 1.0 - hdc_similarity(&h.fingerprint, &heuristics[m].fingerprint))
                .fold(f64::INFINITY, f64::min);
            if min_dist > best_min_dist {
                best_min_dist = min_dist;
                best_idx = i;
            }
        }
        medoid_indices.push(best_idx);
    }

    // Assign each heuristic to the nearest medoid.
    let mut assignments: Vec<usize> = vec![0; heuristics.len()];
    for (i, h) in heuristics.iter().enumerate() {
        let mut best_cluster = 0;
        let mut best_sim = f64::NEG_INFINITY;
        for (c, &medoid) in medoid_indices.iter().enumerate() {
            let sim = hdc_similarity(&h.fingerprint, &heuristics[medoid].fingerprint);
            if sim > best_sim {
                best_sim = sim;
                best_cluster = c;
            }
        }
        assignments[i] = best_cluster;
    }

    // Build worldviews from assignments.
    let mut worldviews: Vec<Worldview> = Vec::with_capacity(k);
    for c in 0..k {
        let members: Vec<HeuristicId> = heuristics
            .iter()
            .enumerate()
            .filter(|(i, _)| assignments[*i] == c)
            .map(|(_, h)| h.id.clone())
            .collect();

        if members.is_empty() {
            continue;
        }

        let mut wv = Worldview::new(format!("worldview-{c}"), members);

        // Domain fingerprint is the mean of member fingerprints.
        let member_fps: Vec<&HdcVector> = heuristics
            .iter()
            .enumerate()
            .filter(|(i, _)| assignments[*i] == c)
            .map(|(_, h)| &h.fingerprint)
            .collect();
        wv.domain_fingerprint = hdc_mean_fingerprint(&member_fps);

        // Coherence = average pairwise similarity within the cluster.
        wv.coherence_score = cluster_coherence(&member_fps);

        worldviews.push(wv);
    }

    worldviews
}

/// Report describing a contradiction between two heuristics.
#[derive(Debug, Clone)]
pub struct DissonanceReport {
    /// The dissonance record.
    pub dissonance: Dissonance,
    /// Overlap strength: cosine similarity of the two fingerprints.
    pub overlap_similarity: f64,
}

/// Detect dissonance between two heuristics.
///
/// Two heuristics are considered dissonant when:
/// 1. Their precondition fingerprints are similar (overlap > threshold),
///    meaning they apply in overlapping situations.
/// 2. Their predictions differ structurally (different variant tags).
///
/// Returns `Some(DissonanceReport)` if dissonance is detected.
#[must_use]
pub fn detect_dissonance(a: &Heuristic, b: &Heuristic) -> Option<DissonanceReport> {
    // Check fingerprint overlap (precondition similarity).
    let similarity = hdc_similarity(&a.fingerprint, &b.fingerprint);

    // Overlap threshold for Hamming similarity.
    // In HDC: 1.0 = identical, 0.5 = uncorrelated random, 0.0 = opposite.
    // A threshold of 0.75 means meaningfully similar contexts.
    let overlap_threshold = 0.75;
    if similarity < overlap_threshold {
        return None;
    }

    // Check if predictions differ (different enum variants).
    if predictions_agree(&a.prediction, &b.prediction) {
        return None;
    }

    let situation_hash = format!(
        "sim={:.3}:{}+{}",
        similarity, a.id, b.id
    );

    Some(DissonanceReport {
        dissonance: Dissonance::new(
            [a.id.clone(), b.id.clone()],
            [a.prediction.clone(), b.prediction.clone()],
            situation_hash,
        ),
        overlap_similarity: similarity,
    })
}

/// Scan all pairs of heuristics for dissonance.
///
/// Returns all detected dissonance reports. O(n^2) so only suitable
/// for moderate collections (< 1000 heuristics).
#[must_use]
pub fn detect_all_dissonances(heuristics: &[Heuristic]) -> Vec<DissonanceReport> {
    let mut reports = Vec::new();
    for i in 0..heuristics.len() {
        for j in (i + 1)..heuristics.len() {
            if let Some(report) = detect_dissonance(&heuristics[i], &heuristics[j]) {
                reports.push(report);
            }
        }
    }
    reports
}

// ─── HDC helpers ────────────────────────────────────────────────────────────

/// Hamming similarity between two HDC vectors, returned as f64.
///
/// Delegates to `HdcVector::similarity()` which computes normalized
/// Hamming similarity in `[0.0, 1.0]`. A value of 1.0 means identical
/// bit patterns; 0.5 means uncorrelated (random); 0.0 means all bits
/// flipped.
fn hdc_similarity(a: &HdcVector, b: &HdcVector) -> f64 {
    a.similarity(b) as f64
}

/// Compute the majority-vote bundle of HDC fingerprints.
///
/// Uses `HdcVector::bundle()` which takes the bitwise majority of
/// all input vectors — the standard HDC superposition operation.
fn hdc_mean_fingerprint(fps: &[&HdcVector]) -> HdcVector {
    if fps.is_empty() {
        return HdcVector::zeros();
    }
    HdcVector::bundle(fps)
}

/// Average pairwise Hamming similarity within a cluster.
fn cluster_coherence(fps: &[&HdcVector]) -> f64 {
    if fps.len() < 2 {
        return 1.0;
    }

    let mut total_sim = 0.0;
    let mut count = 0_u64;
    for i in 0..fps.len() {
        for j in (i + 1)..fps.len() {
            total_sim += hdc_similarity(fps[i], fps[j]);
            count += 1;
        }
    }

    if count == 0 {
        1.0
    } else {
        total_sim / count as f64
    }
}

/// Check if two predictions agree (same variant structure).
fn predictions_agree(a: &Predicate, b: &Predicate) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Make a heuristic with a deterministic fingerprint seeded from the id.
    fn make_heuristic_with_fp(id: &str, fp: HdcVector, pred: Predicate) -> Heuristic {
        let mut h = Heuristic::new(id, format!("claim for {id}"), vec![], pred);
        h.fingerprint = fp;
        h
    }

    #[test]
    fn heuristic_falsifier_defaults_to_none() {
        let h = Heuristic::new(
            "h1",
            "test claim",
            vec![],
            Predicate::LanguageIs(Language::Rust),
        );
        assert!(h.falsifier.is_none());
    }

    #[test]
    fn heuristic_with_falsifier() {
        let h = Heuristic::new(
            "h1",
            "test claim",
            vec![],
            Predicate::LanguageIs(Language::Rust),
        )
        .with_falsifier(Predicate::GateRecentlyFailed("compile".into()));
        assert!(h.falsifier.is_some());
    }

    #[test]
    fn cluster_worldviews_empty_input() {
        let result = cluster_worldviews(&[], 3);
        assert!(result.is_empty());
    }

    #[test]
    fn cluster_worldviews_single_heuristic() {
        let fp = HdcVector::from_seed(b"h1-seed");
        let h = make_heuristic_with_fp("h1", fp, Predicate::LanguageIs(Language::Rust));
        let result = cluster_worldviews(&[h], 1);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].core_heuristics.len(), 1);
    }

    #[test]
    fn cluster_worldviews_assigns_all_heuristics() {
        // Use random fingerprints -- with k=2 all heuristics must land somewhere.
        let heuristics: Vec<Heuristic> = (0..6)
            .map(|i| {
                let fp = HdcVector::from_seed(format!("seed-{i}").as_bytes());
                make_heuristic_with_fp(
                    &format!("h{i}"),
                    fp,
                    Predicate::LanguageIs(Language::Rust),
                )
            })
            .collect();

        let worldviews = cluster_worldviews(&heuristics, 2);
        // All heuristics should be assigned.
        let total: usize = worldviews.iter().map(|w| w.core_heuristics.len()).sum();
        assert_eq!(total, 6);
        // Each cluster should have members.
        for wv in &worldviews {
            assert!(!wv.core_heuristics.is_empty());
        }
    }

    #[test]
    fn detect_dissonance_identical_fingerprint_different_prediction() {
        // Same fingerprint = identical bits = similarity 1.0 => high overlap.
        let fp = HdcVector::from_seed(b"shared-fp");
        let a = make_heuristic_with_fp("h1", fp, Predicate::LanguageIs(Language::Rust));
        let b = make_heuristic_with_fp(
            "h2",
            fp,
            Predicate::GateRecentlyFailed("compile".into()),
        );

        let report = detect_dissonance(&a, &b);
        assert!(report.is_some());
        let report = report.unwrap();
        assert!((report.overlap_similarity - 1.0).abs() < 1e-3);
    }

    #[test]
    fn detect_dissonance_random_fingerprints_no_conflict() {
        // Two independent random vectors have ~0.5 Hamming similarity,
        // below the 0.75 threshold.
        let a = make_heuristic_with_fp(
            "h1",
            HdcVector::from_seed(b"seed-a"),
            Predicate::LanguageIs(Language::Rust),
        );
        let b = make_heuristic_with_fp(
            "h2",
            HdcVector::from_seed(b"seed-b"),
            Predicate::GateRecentlyFailed("test".into()),
        );

        let report = detect_dissonance(&a, &b);
        assert!(report.is_none());
    }

    #[test]
    fn detect_dissonance_same_prediction_no_conflict() {
        // Same fingerprint AND same prediction variant = no dissonance.
        let fp = HdcVector::from_seed(b"shared");
        let a = make_heuristic_with_fp("h1", fp, Predicate::LanguageIs(Language::Rust));
        let b = make_heuristic_with_fp("h2", fp, Predicate::LanguageIs(Language::Go));

        let report = detect_dissonance(&a, &b);
        assert!(report.is_none());
    }

    #[test]
    fn detect_all_dissonances_finds_pairs() {
        let shared_fp = HdcVector::from_seed(b"shared");
        let other_fp = HdcVector::from_seed(b"other-seed");
        let heuristics = vec![
            make_heuristic_with_fp("h1", shared_fp, Predicate::LanguageIs(Language::Rust)),
            make_heuristic_with_fp(
                "h2",
                shared_fp,
                Predicate::GateRecentlyFailed("compile".into()),
            ),
            make_heuristic_with_fp("h3", other_fp, Predicate::LanguageIs(Language::Go)),
        ];

        let reports = detect_all_dissonances(&heuristics);
        // h1 and h2 have same fingerprint but different prediction variants.
        // h3 has a different fingerprint, so no dissonance with h1/h2.
        assert_eq!(reports.len(), 1);
    }

    #[test]
    fn hdc_similarity_identical_vectors() {
        let v = HdcVector::from_seed(b"test-vec");
        assert!((hdc_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hdc_similarity_zero_vector() {
        let a = HdcVector::zeros();
        let b = HdcVector::from_seed(b"nonzero");
        // Zero vs random: about 50% of bits match.
        let sim = hdc_similarity(&a, &b);
        assert!(sim > 0.3 && sim < 0.7, "expected ~0.5, got {sim}");
    }

    #[test]
    fn worldview_coherence_computed() {
        let fp = HdcVector::from_seed(b"coherent");
        let heuristics = vec![
            make_heuristic_with_fp("h1", fp, Predicate::LanguageIs(Language::Rust)),
            make_heuristic_with_fp("h2", fp, Predicate::LanguageIs(Language::Rust)),
        ];

        let worldviews = cluster_worldviews(&heuristics, 1);
        assert_eq!(worldviews.len(), 1);
        // Identical fingerprints => coherence should be 1.0.
        assert!((worldviews[0].coherence_score - 1.0).abs() < 1e-3);
    }
}
