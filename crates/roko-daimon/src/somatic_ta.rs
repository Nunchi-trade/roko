//! Somatic TA integration: wiring somatic markers into oracle predictions (TA-11).
//!
//! This module bridges the daimon affect subsystem with the oracle/prediction
//! subsystem in `roko-core`. It provides:
//!
//! - [`SomaticOracleContext`]: pre-computed somatic bias for oracle predictions.
//! - [`somatic_confidence_bias`]: adjusts prediction confidence based on somatic signal.
//! - [`IitPhiMetric`]: Integrated Information Theory (IIT) Phi over TA subsystems.
//! - [`SomaticRetrieval`]: somatic-aware retrieval with 15% contrarian blending.
//!
//! # Somatic Markers and Oracle Predictions
//!
//! Damasio's somatic marker hypothesis (1994) posits that emotional signals
//! from body-mapped memories guide decision-making under uncertainty. Here,
//! when an oracle makes a prediction for a task/strategy region, the somatic
//! landscape's emotional valence biases the confidence:
//!
//! - Positive somatic signal in the strategy region -> slight confidence boost.
//! - Negative somatic signal -> confidence reduction (caution).
//! - 15% of the signal is drawn from contrarian neighbours (Bower 1981).
//!
//! # IIT Phi Metric
//!
//! Tononi's Integrated Information Theory (IIT) measures how much a system
//! is "more than the sum of its parts." Phi over N subsystems estimates the
//! irreducible information integration. For N TA subsystems, we compute Phi
//! from a mutual information matrix.
//!
//! # References
//!
//! - Damasio, A. (1994). *Descartes' Error*.
//! - Bower, G. H. (1981). Mood and memory. *American Psychologist*, 36(2), 129.
//! - Tononi, G. (2004). An information integration theory of consciousness.
//!   *BMC Neuroscience*, 5, 42.
//! - Williams, P. L. & Beer, R. D. (2010). Nonnegative decomposition of
//!   multivariate information.

use serde::{Deserialize, Serialize};

use crate::{SomaticLandscape, SomaticSignal, StrategyCoordinates};

// ---------------------------------------------------------------------------
// SomaticOracleContext
// ---------------------------------------------------------------------------

/// Pre-computed somatic context for biasing oracle predictions.
///
/// Created by querying the `SomaticLandscape` at the strategy coordinates
/// of the current task. The oracle can use this to adjust its confidence
/// based on the emotional history of similar past situations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SomaticOracleContext {
    /// The somatic signal at the queried strategy region.
    pub signal: SomaticSignal,
    /// Whether the signal is strong enough to influence the prediction.
    pub is_actionable: bool,
    /// Confidence adjustment factor in `[0.7, 1.3]`.
    ///
    /// - `> 1.0`: positive somatic memory boosts confidence.
    /// - `< 1.0`: negative somatic memory reduces confidence.
    /// - `= 1.0`: neutral / no somatic data.
    pub confidence_multiplier: f64,
    /// Fraction of the signal that came from contrarian neighbours.
    pub contrarian_fraction: f64,
}

impl Default for SomaticOracleContext {
    fn default() -> Self {
        Self {
            signal: SomaticSignal::default(),
            is_actionable: false,
            confidence_multiplier: 1.0,
            contrarian_fraction: 0.0,
        }
    }
}

impl SomaticOracleContext {
    /// Query the somatic landscape and build an oracle context.
    ///
    /// The `k` parameter controls how many nearby somatic markers to consider
    /// (default 5 in the daimon crate).
    #[must_use]
    pub fn from_landscape(
        landscape: &SomaticLandscape,
        strategy_coords: StrategyCoordinates,
        k: usize,
    ) -> Self {
        let signal = landscape.query(strategy_coords, k);

        let is_actionable = signal.is_actionable();
        let confidence_multiplier = somatic_confidence_bias(signal.valence, signal.intensity);
        let contrarian_fraction = if signal.neighbor_count > 0 {
            signal.contrarian_count as f64 / signal.neighbor_count as f64
        } else {
            0.0
        };

        Self {
            signal,
            is_actionable,
            confidence_multiplier,
            contrarian_fraction,
        }
    }
}

/// Compute a confidence multiplier from somatic valence and intensity.
///
/// The bias follows a sigmoid-like curve:
/// - Positive valence + high intensity -> multiplier up to 1.3.
/// - Negative valence + high intensity -> multiplier down to 0.7.
/// - Low intensity or neutral valence -> multiplier near 1.0.
///
/// The range is clamped to `[0.7, 1.3]` to prevent somatic data from
/// dominating rational prediction.
#[must_use]
pub fn somatic_confidence_bias(valence: f64, intensity: f64) -> f64 {
    // Maximum adjustment is +-30% of confidence.
    const MAX_BIAS: f64 = 0.30;

    // Scale: intensity controls strength, valence controls direction.
    let raw = valence * intensity * MAX_BIAS;
    (1.0 + raw).clamp(0.7, 1.3)
}

/// Apply somatic bias to a prediction confidence value.
///
/// Returns the adjusted confidence, clamped to `[0.0, 1.0]`.
///
/// # Arguments
/// - `base_confidence`: the oracle's raw confidence in `[0.0, 1.0]`.
/// - `somatic_ctx`: the pre-computed somatic context.
#[must_use]
pub fn apply_somatic_confidence_bias(
    base_confidence: f64,
    somatic_ctx: &SomaticOracleContext,
) -> f64 {
    if !somatic_ctx.is_actionable {
        return base_confidence;
    }
    (base_confidence * somatic_ctx.confidence_multiplier).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Somatic retrieval with contrarian blending
// ---------------------------------------------------------------------------

/// Configuration for somatic-aware retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SomaticRetrievalConfig {
    /// Fraction of retrievals that should be contrarian (default 0.15).
    pub contrarian_fraction: f64,
    /// Number of somatic neighbours to consider.
    pub neighbor_count: usize,
    /// Minimum intensity for a somatic signal to be considered.
    pub min_intensity: f64,
}

impl Default for SomaticRetrievalConfig {
    fn default() -> Self {
        Self {
            contrarian_fraction: 0.15,
            neighbor_count: 5,
            min_intensity: 0.15,
        }
    }
}

/// Result of a somatic retrieval query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SomaticRetrieval {
    /// The primary (congruent) signal from nearby markers.
    pub primary_signal: SomaticSignal,
    /// Whether contrarian blending was applied.
    pub contrarian_applied: bool,
    /// The final blended valence.
    pub blended_valence: f64,
    /// The final blended intensity.
    pub blended_intensity: f64,
}

impl SomaticRetrieval {
    /// Perform somatic retrieval from the landscape.
    #[must_use]
    pub fn query(
        landscape: &SomaticLandscape,
        strategy_coords: StrategyCoordinates,
        config: &SomaticRetrievalConfig,
    ) -> Self {
        let signal = landscape.query(strategy_coords, config.neighbor_count);

        // The SomaticLandscape.query() already implements the 15% contrarian
        // blending (see CONTRARIAN_FRACTION in lib.rs). We expose the result.
        let contrarian_applied = signal.contrarian_count > 0;

        Self {
            blended_valence: signal.valence,
            blended_intensity: signal.intensity,
            primary_signal: signal,
            contrarian_applied,
        }
    }
}

// ---------------------------------------------------------------------------
// IIT Phi Metric
// ---------------------------------------------------------------------------

/// Mutual information matrix between TA subsystems.
///
/// Entry `mi[i][j]` is the estimated mutual information between subsystem i
/// and subsystem j, in nats. The diagonal `mi[i][i]` is the self-information
/// (entropy) of subsystem i.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutualInfoMatrix {
    /// Number of subsystems.
    pub n: usize,
    /// Flattened n x n matrix (row-major).
    pub data: Vec<f64>,
}

impl MutualInfoMatrix {
    /// Create a mutual information matrix from a flat row-major array.
    pub fn new(n: usize, data: Vec<f64>) -> Self {
        assert_eq!(data.len(), n * n, "data length must be n*n");
        Self { n, data }
    }

    /// Create from pairwise correlation coefficients.
    ///
    /// Converts Pearson correlation `r` to mutual information via the
    /// Gaussian approximation: `I(X;Y) = -0.5 * ln(1 - r^2)`.
    pub fn from_correlations(n: usize, correlations: &[f64]) -> Self {
        assert_eq!(
            correlations.len(),
            n * n,
            "correlation matrix size mismatch"
        );
        let data: Vec<f64> = correlations
            .iter()
            .map(|&r| {
                let r_clamped = r.clamp(-0.9999, 0.9999);
                -0.5 * (1.0 - r_clamped * r_clamped).ln()
            })
            .collect();
        Self { n, data }
    }

    /// Get MI(i, j).
    #[must_use]
    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.data[i * self.n + j]
    }
}

/// IIT Phi metric over TA subsystems.
///
/// Phi measures the irreducible integrated information of the system.
/// It quantifies how much the whole system is more than the sum of its
/// parts — a system with high Phi cannot be decomposed into independent
/// subsystems without losing information.
///
/// We use the "minimum information bipartition" (MIB) approximation:
/// for each bipartition of the N subsystems into two non-empty groups,
/// compute the mutual information across the cut, and take the minimum.
///
/// `Phi = min_{bipartitions} MI(A; B) / min(H(A), H(B))`
///
/// where H(A) is the entropy of partition A and MI(A;B) is the mutual
/// information between partitions A and B.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IitPhiMetric {
    /// The computed Phi value (non-negative).
    pub phi: f64,
    /// Number of subsystems evaluated.
    pub num_subsystems: usize,
    /// Number of bipartitions evaluated.
    pub num_bipartitions: u64,
    /// The minimum-information bipartition (bit mask of partition A).
    pub mib_mask: u64,
}

impl IitPhiMetric {
    /// Compute Phi from a mutual information matrix.
    ///
    /// Enumerates all bipartitions of N subsystems (2^(N-1) - 1 non-trivial
    /// bipartitions). For N <= 20 this is tractable; for larger N an
    /// approximation would be needed.
    ///
    /// Returns the IIT Phi metric. A higher value means the subsystems are
    /// more integrated and cannot be decomposed without information loss.
    #[must_use]
    pub fn compute(mi: &MutualInfoMatrix) -> Self {
        let n = mi.n;
        if n <= 1 {
            return Self {
                phi: 0.0,
                num_subsystems: n,
                num_bipartitions: 0,
                mib_mask: 0,
            };
        }

        let mut min_phi = f64::INFINITY;
        let mut mib_mask = 0_u64;
        let mut num_bipartitions = 0_u64;

        // Enumerate all non-trivial bipartitions.
        // mask = set of indices in partition A; complement = partition B.
        // We only need to check masks from 1 to 2^(n-1) - 1 (avoid double-counting).
        let half = 1_u64 << (n - 1);
        for mask in 1..half {
            // Partition A: indices where bit is set.
            // Partition B: indices where bit is clear.
            let mut h_a = 0.0; // Entropy of partition A.
            let mut h_b = 0.0; // Entropy of partition B.
            let mut mi_cross = 0.0; // MI across the cut.

            // H(A) = sum of entropies of subsystems in A
            //       + internal MI within A (already captured in diagonal).
            for i in 0..n {
                if mask & (1 << i) != 0 {
                    h_a += mi.get(i, i);
                } else {
                    h_b += mi.get(i, i);
                }
            }

            // MI(A;B) = sum of MI(i,j) for i in A, j in B.
            for i in 0..n {
                for j in 0..n {
                    if i == j {
                        continue;
                    }
                    let i_in_a = mask & (1 << i) != 0;
                    let j_in_a = mask & (1 << j) != 0;
                    if i_in_a != j_in_a {
                        mi_cross += mi.get(i, j);
                    }
                }
            }
            // Each pair counted twice (i->j and j->i), so halve.
            mi_cross /= 2.0;

            // Normalized: Phi_cut = MI(A;B) / min(H(A), H(B)).
            let min_h = h_a.min(h_b);
            let phi_cut = if min_h > 1e-15 { mi_cross / min_h } else { 0.0 };

            num_bipartitions += 1;

            if phi_cut < min_phi {
                min_phi = phi_cut;
                mib_mask = mask;
            }
        }

        Self {
            phi: if min_phi.is_finite() {
                min_phi.max(0.0)
            } else {
                0.0
            },
            num_subsystems: n,
            num_bipartitions,
            mib_mask,
        }
    }
}

// ---------------------------------------------------------------------------
// Subsystem activity vector for PID synergy detection
// ---------------------------------------------------------------------------

/// Per-subsystem activity summary for PID (Partial Information Decomposition)
/// synergy detection.
///
/// Each entry represents a TA subsystem's recent activity level.
/// Synergy is detected when the joint activity of multiple subsystems
/// carries more information than the sum of their individual activities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsystemActivity {
    /// Subsystem name.
    pub name: String,
    /// Recent activity level in `[0.0, 1.0]`.
    pub activity: f64,
    /// Number of predictions made by this subsystem in the window.
    pub prediction_count: u64,
    /// Average prediction accuracy in `[0.0, 1.0]`.
    pub avg_accuracy: f64,
}

/// Detect synergy between subsystems using a simple co-activation heuristic.
///
/// True PID synergy detection (Williams & Beer 2010) requires full probability
/// distributions. This simplified version detects co-activation patterns
/// where joint activity exceeds the product of marginal activities (positive
/// synergy) or falls below it (redundancy).
///
/// Returns `(synergy_score, redundancy_score)`:
/// - `synergy > 0`: subsystems are more useful together than apart.
/// - `redundancy > 0`: subsystems duplicate information.
#[must_use]
pub fn detect_synergy(activities: &[SubsystemActivity]) -> (f64, f64) {
    if activities.len() < 2 {
        return (0.0, 0.0);
    }

    let n = activities.len() as f64;
    let mean_activity: f64 = activities.iter().map(|a| a.activity).sum::<f64>() / n;
    let mean_accuracy: f64 = activities.iter().map(|a| a.avg_accuracy).sum::<f64>() / n;

    // Joint activity: product of all activities (higher if all are active).
    let joint_activity: f64 = activities.iter().map(|a| a.activity.max(0.01)).product();

    // Expected under independence: product of marginals.
    let independent_expected: f64 = mean_activity.powf(n);

    // Synergy: joint exceeds independent expectation.
    let synergy = if independent_expected > 1e-15 {
        ((joint_activity / independent_expected).ln()).max(0.0)
    } else {
        0.0
    };

    // Redundancy: high correlation in accuracy suggests overlapping info.
    let accuracy_variance: f64 = activities
        .iter()
        .map(|a| (a.avg_accuracy - mean_accuracy).powi(2))
        .sum::<f64>()
        / n;

    // Low variance in accuracy = high redundancy (all doing similar work).
    let redundancy = if accuracy_variance < 0.01 && mean_accuracy > 0.5 {
        (1.0 - accuracy_variance * 100.0).max(0.0) * mean_accuracy
    } else {
        0.0
    };

    (synergy, redundancy)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SomaticLandscape;
    use roko_core::ContentHash;

    // --- SomaticOracleContext ---

    #[test]
    fn empty_landscape_gives_neutral_context() {
        let landscape = SomaticLandscape::new();
        let coords = StrategyCoordinates::default();
        let ctx = SomaticOracleContext::from_landscape(&landscape, coords, 5);

        assert!(!ctx.is_actionable);
        assert!((ctx.confidence_multiplier - 1.0).abs() < 1e-10);
    }

    #[test]
    fn positive_somatic_boosts_confidence() {
        let mut landscape = SomaticLandscape::new();
        let coords = StrategyCoordinates::default();
        let now = chrono::Utc::now();

        // Record a strong positive outcome.
        landscape.record_outcome(coords.clone(), 0.8, 0.9, ContentHash::of(b"ep1"), now);
        landscape.record_outcome(coords.clone(), 0.7, 0.8, ContentHash::of(b"ep2"), now);

        let ctx = SomaticOracleContext::from_landscape(&landscape, coords, 5);
        assert!(
            ctx.confidence_multiplier > 1.0,
            "positive somatic should boost confidence: {}",
            ctx.confidence_multiplier
        );
    }

    #[test]
    fn negative_somatic_reduces_confidence() {
        let mut landscape = SomaticLandscape::new();
        let coords = StrategyCoordinates::default();
        let now = chrono::Utc::now();

        // Record a strong negative outcome.
        landscape.record_outcome(coords.clone(), -0.8, 0.9, ContentHash::of(b"ep1"), now);
        landscape.record_outcome(coords.clone(), -0.7, 0.8, ContentHash::of(b"ep2"), now);

        let ctx = SomaticOracleContext::from_landscape(&landscape, coords, 5);
        assert!(
            ctx.confidence_multiplier < 1.0,
            "negative somatic should reduce confidence: {}",
            ctx.confidence_multiplier
        );
    }

    // --- somatic_confidence_bias ---

    #[test]
    fn neutral_valence_gives_unity_bias() {
        assert!((somatic_confidence_bias(0.0, 0.5) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn bias_clamped_to_range() {
        // Extreme positive.
        let high = somatic_confidence_bias(1.0, 1.0);
        assert!(high <= 1.3 + 1e-10);
        assert!(high >= 1.2); // Should be close to max.

        // Extreme negative.
        let low = somatic_confidence_bias(-1.0, 1.0);
        assert!(low >= 0.7 - 1e-10);
        assert!(low <= 0.8); // Should be close to min.
    }

    #[test]
    fn low_intensity_gives_weak_bias() {
        let bias = somatic_confidence_bias(1.0, 0.01);
        assert!(
            (bias - 1.0).abs() < 0.01,
            "low intensity should give near-neutral bias: {bias}"
        );
    }

    // --- apply_somatic_bias ---

    #[test]
    fn non_actionable_context_preserves_confidence() {
        let ctx = SomaticOracleContext::default();
        assert_eq!(apply_somatic_confidence_bias(0.8, &ctx), 0.8);
    }

    #[test]
    fn actionable_context_modifies_confidence() {
        let ctx = SomaticOracleContext {
            signal: SomaticSignal {
                valence: 0.5,
                intensity: 0.8,
                neighbor_count: 3,
                contrarian_count: 1,
                source_episodes: vec![],
            },
            is_actionable: true,
            confidence_multiplier: 1.12,
            contrarian_fraction: 0.15,
        };
        let adjusted = apply_somatic_confidence_bias(0.8, &ctx);
        assert!(
            (adjusted - 0.896).abs() < 0.01,
            "expected ~0.896, got {adjusted}"
        );
    }

    // --- SomaticRetrieval ---

    #[test]
    fn somatic_retrieval_from_empty_landscape() {
        let landscape = SomaticLandscape::new();
        let config = SomaticRetrievalConfig::default();
        let coords = StrategyCoordinates::default();
        let retrieval = SomaticRetrieval::query(&landscape, coords, &config);

        assert!(!retrieval.contrarian_applied);
        assert_eq!(retrieval.blended_valence, 0.0);
    }

    // --- IIT Phi Metric ---

    #[test]
    fn phi_single_subsystem_is_zero() {
        let mi = MutualInfoMatrix::new(1, vec![1.0]);
        let result = IitPhiMetric::compute(&mi);
        assert_eq!(result.phi, 0.0);
        assert_eq!(result.num_subsystems, 1);
    }

    #[test]
    fn phi_independent_subsystems_is_zero() {
        // Two independent subsystems: no MI between them.
        let mi = MutualInfoMatrix::new(
            2,
            vec![
                1.0, 0.0, // H(0)=1, MI(0,1)=0
                0.0, 1.0, // MI(1,0)=0, H(1)=1
            ],
        );
        let result = IitPhiMetric::compute(&mi);
        assert!(
            result.phi < 1e-10,
            "independent subsystems should have Phi ≈ 0: {}",
            result.phi
        );
    }

    #[test]
    fn phi_integrated_subsystems_is_positive() {
        // Two highly integrated subsystems.
        let mi = MutualInfoMatrix::new(
            2,
            vec![
                1.0, 0.8, // H(0)=1, MI(0,1)=0.8
                0.8, 1.0, // MI(1,0)=0.8, H(1)=1
            ],
        );
        let result = IitPhiMetric::compute(&mi);
        assert!(
            result.phi > 0.1,
            "integrated subsystems should have positive Phi: {}",
            result.phi
        );
    }

    #[test]
    fn phi_three_subsystems() {
        // Three subsystems with varying integration.
        let mi = MutualInfoMatrix::new(
            3,
            vec![
                1.0, 0.5, 0.3, // H(0)=1
                0.5, 1.0, 0.4, // H(1)=1
                0.3, 0.4, 1.0, // H(2)=1
            ],
        );
        let result = IitPhiMetric::compute(&mi);
        assert!(result.phi > 0.0);
        assert_eq!(result.num_subsystems, 3);
        // 3 subsystems -> 3 bipartitions: {0}|{1,2}, {1}|{0,2}, {0,1}|{2}
        assert_eq!(result.num_bipartitions, 3);
    }

    #[test]
    fn phi_from_correlations() {
        // Two correlated subsystems.
        let mi = MutualInfoMatrix::from_correlations(
            2,
            &[
                1.0, 0.9, // near-perfect correlation
                0.9, 1.0,
            ],
        );
        let result = IitPhiMetric::compute(&mi);
        // MI from Gaussian approx: I(X;Y) = -0.5 * ln(1 - r^2)
        // The Phi value depends on the normalization. For two subsystems:
        // Phi = MI(0,1) / min(H(0), H(1)).
        // Since from_correlations converts *all* entries (including diagonal),
        // the diagonal becomes -0.5*ln(1-1) which is handled by clamping.
        assert!(
            result.phi > 0.0,
            "correlated subsystems should have positive Phi: {}",
            result.phi
        );
    }

    // --- Synergy detection ---

    #[test]
    fn detect_synergy_single_subsystem() {
        let activities = vec![SubsystemActivity {
            name: "chain".into(),
            activity: 0.8,
            prediction_count: 10,
            avg_accuracy: 0.75,
        }];
        let (synergy, redundancy) = detect_synergy(&activities);
        assert_eq!(synergy, 0.0);
        assert_eq!(redundancy, 0.0);
    }

    #[test]
    fn detect_synergy_coactive_subsystems() {
        let activities = vec![
            SubsystemActivity {
                name: "chain".into(),
                activity: 0.9,
                prediction_count: 20,
                avg_accuracy: 0.8,
            },
            SubsystemActivity {
                name: "coding".into(),
                activity: 0.9,
                prediction_count: 15,
                avg_accuracy: 0.75,
            },
        ];
        let (synergy, _) = detect_synergy(&activities);
        // Both active: joint activity should approximate independence.
        // Since both are high, synergy should be small but non-negative.
        assert!(synergy >= 0.0, "synergy should be non-negative: {synergy}");
    }

    #[test]
    fn detect_redundancy_uniform_accuracy() {
        let activities = vec![
            SubsystemActivity {
                name: "a".into(),
                activity: 0.5,
                prediction_count: 10,
                avg_accuracy: 0.8,
            },
            SubsystemActivity {
                name: "b".into(),
                activity: 0.5,
                prediction_count: 10,
                avg_accuracy: 0.8,
            },
            SubsystemActivity {
                name: "c".into(),
                activity: 0.5,
                prediction_count: 10,
                avg_accuracy: 0.8,
            },
        ];
        let (_, redundancy) = detect_synergy(&activities);
        assert!(
            redundancy > 0.0,
            "uniform accuracy should indicate redundancy: {redundancy}"
        );
    }
}
