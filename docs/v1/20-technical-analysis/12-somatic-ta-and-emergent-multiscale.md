# Somatic Technical Analysis and Emergent Multiscale Intelligence

> Somatic TA uses Damasio's somatic marker hypothesis to create "gut feelings" about TA patterns. Emergent multiscale intelligence measures integrated information (IIT Phi) across the TA subsystems, detecting when the whole is greater than the sum of its parts.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [06-hyperdimensional-ta](./06-hyperdimensional-ta.md) for HDC encoding, [08-adaptive-signal-metabolism](./08-adaptive-signal-metabolism.md) for signal ecosystem
**Key sources**: `bardo-backup/prd/23-ta/09-somatic-technical-analysis.md`, `bardo-backup/prd/23-ta/10-emergent-multiscale-intelligence.md`

---

## Part I: Somatic Technical Analysis

### Damasio's somatic marker hypothesis

Antonio Damasio's somatic marker hypothesis (Damasio, 1994, *Descartes' Error*) proposes that emotions are not irrational noise but fast heuristics for decision-making. When a person encounters a situation similar to one that previously had a strong outcome (good or bad), they experience a "gut feeling" — a somatic marker — that biases their decision before conscious analysis completes.

Roko implements somatic markers for TA patterns: when the agent encounters a pattern similar to one that previously led to profit or loss, it retrieves an HDC-encoded "feeling" that biases the prediction. This is System 1 cognition (Kahneman, 2011) for agents — fast, pre-analytical, and often correct.

### Somatic markers as HDC bindings

Each somatic marker binds a TA pattern vector to an affect (PAD) vector:

```rust
/// A somatic marker: an HDC binding between a pattern and an affect.
///
/// marker_hv = BIND(pattern_hv, affect_hv)
///
/// When the agent encounters a new pattern, it retrieves somatic
/// markers with high similarity to BIND(new_pattern, ?) — i.e.,
/// it finds patterns that are similar AND checks what affect they
/// are associated with.
pub struct SomaticMarker {
    /// The combined HDC vector: BIND(pattern, affect).
    pub marker_hv: HdcVector,

    /// The pattern this marker was formed from.
    pub pattern_hv: HdcVector,

    /// The affect vector (PAD encoding).
    pub affect_hv: HdcVector,

    /// PAD values for interpretability.
    pub pleasure: f64,
    pub arousal: f64,
    pub dominance: f64,

    /// Strength of the marker (decays over time, strengthened by re-experience).
    pub strength: f64,

    /// Which episodes formed this marker.
    pub episode_sources: Vec<ContentHash>,

    /// Creation timestamp.
    pub created_at_ms: i64,
}
```

### PAD encoding in HDC

The PAD (Pleasure-Arousal-Dominance) vector is encoded as an HDC vector using the same quantized codebook approach as numeric values (see [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md)):

```rust
/// Encode PAD state as an HDC vector.
///
/// Uses role-filler composition:
///   affect_hv = BUNDLE(
///       BIND(pleasure_role, pleasure_value),
///       BIND(arousal_role, arousal_value),
///       BIND(dominance_role, dominance_value),
///   )
///
/// The encoding preserves the continuous nature of PAD values
/// while enabling HDC similarity operations.
pub fn encode_pad(
    pleasure: f64,
    arousal: f64,
    dominance: f64,
    codebook: &AffectCodebook,
) -> HdcVector {
    let p_binding = codebook.pleasure_role.xor(
        &codebook.value_codebook.encode(pleasure)
    );
    let a_binding = codebook.arousal_role.xor(
        &codebook.value_codebook.encode(arousal)
    );
    let d_binding = codebook.dominance_role.xor(
        &codebook.value_codebook.encode(dominance)
    );

    HdcVector::bundle(&[p_binding, a_binding, d_binding])
}
```

The affect codebook is compatible with the Mehrabian & Russell (1974) PAD model, which provides the dimensional framework for Roko's Daimon subsystem.

### Somatic retrieval — Pre-analytical "gut feeling"

Before making a prediction, the oracle queries the somatic landscape for emotional valence of similar patterns:

```rust
/// Somatic retrieval: query "what does this pattern feel like?"
///
/// Given a new TA pattern, find somatic markers with similar patterns
/// and aggregate their affect vectors.
///
/// Cost: ~63ns per marker comparison (BIND + Hamming similarity).
/// For 1,000 markers: ~63µs.
///
/// This runs BEFORE analytical prediction — it's a fast System 1
/// heuristic that biases the subsequent System 2 analysis.
pub fn somatic_retrieval(
    pattern: &HdcVector,
    somatic_map: &[SomaticMarker],
    threshold: f64,
    contrarian_fraction: f64,  // typically 0.15 per Bower (1981)
) -> SomaticAssessment {
    // Find all markers where the pattern component is similar
    let mut matches: Vec<(f64, &SomaticMarker)> = somatic_map.iter()
        .filter_map(|marker| {
            // Unbind the affect to compare just the pattern component
            let pattern_component = marker.marker_hv.xor(&marker.affect_hv);
            let similarity = pattern.hamming_similarity(&pattern_component);
            if similarity > threshold {
                Some((similarity, marker))
            } else {
                None
            }
        })
        .collect();

    matches.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    // Aggregate affect, weighted by similarity and marker strength
    let total_weight: f64 = matches.iter()
        .map(|(sim, m)| sim * m.strength)
        .sum();

    let avg_pleasure = matches.iter()
        .map(|(sim, m)| m.pleasure * sim * m.strength / total_weight)
        .sum::<f64>();
    let avg_arousal = matches.iter()
        .map(|(sim, m)| m.arousal * sim * m.strength / total_weight)
        .sum::<f64>();
    let avg_dominance = matches.iter()
        .map(|(sim, m)| m.dominance * sim * m.strength / total_weight)
        .sum::<f64>();

    // Mandatory 15% contrarian retrieval (Bower, 1981)
    // Retrieve markers with OPPOSITE valence to prevent echo chambers
    let contrarian_count = (matches.len() as f64 * contrarian_fraction).ceil() as usize;
    let contrarian_markers = find_contrarian_markers(
        pattern, somatic_map, avg_pleasure, contrarian_count
    );

    SomaticAssessment {
        valence: avg_pleasure,
        arousal: avg_arousal,
        dominance: avg_dominance,
        confidence: total_weight / matches.len().max(1) as f64,
        n_matching_markers: matches.len(),
        contrarian_markers,
    }
}
```

The mandatory 15% contrarian retrieval is critical. Without it, the somatic system would create an emotional echo chamber — if the agent has positive associations with a pattern, it would always retrieve positive markers, reinforcing the bias. The contrarian retrieval ensures the agent considers counterarguments, following Bower's (1981) research on mood-congruent memory bias.

### Somatic marker formation

Markers form after prediction resolution — when the oracle knows whether a pattern led to a good or bad outcome:

```rust
/// Create a somatic marker from a resolved prediction.
///
/// When a prediction resolves, the oracle knows:
/// - The pattern that was observed (HDC vector)
/// - The outcome (good/bad, encoded as PAD)
///
/// The somatic marker binds these together for future retrieval.
pub fn form_somatic_marker(
    pattern: &HdcVector,
    outcome: &PredictionAccuracy,
    current_pad: &PadState,
    codebook: &AffectCodebook,
) -> SomaticMarker {
    // Encode the affect: outcome quality modulates PAD
    let pleasure = if outcome.accuracy > 0.7 { 0.8 } else { -0.6 };
    let arousal = outcome.residual.abs();  // larger errors = more arousal
    let dominance = outcome.accuracy;  // higher accuracy = more confidence

    let affect_hv = encode_pad(pleasure, arousal, dominance, codebook);
    let marker_hv = pattern.xor(&affect_hv);  // BIND

    SomaticMarker {
        marker_hv,
        pattern_hv: pattern.clone(),
        affect_hv,
        pleasure,
        arousal,
        dominance,
        strength: 1.0,
        episode_sources: vec![outcome.prediction_id],
        created_at_ms: now_ms(),
    }
}
```

### Somatic marker decay and reinforcement

Markers weaken over time (Ebbinghaus decay) but are strengthened by re-experience:

```rust
/// Update somatic marker strength.
///
/// Decay: strength *= exp(-λt) where λ depends on marker type.
/// Reinforcement: when a similar pattern is re-encountered with
/// similar affect, strength increases.
pub fn update_marker_strength(
    marker: &mut SomaticMarker,
    elapsed_ms: i64,
    reinforcement: Option<f64>,
) {
    // Decay
    let lambda = 0.001;  // half-life ~700 ms (fast for working memory)
    let decay_factor = (-lambda * elapsed_ms as f64).exp();
    marker.strength *= decay_factor;

    // Reinforcement
    if let Some(reinforcement_strength) = reinforcement {
        marker.strength = (marker.strength + reinforcement_strength).min(5.0);
    }
}
```

---

## Part II: Emergent Multiscale Intelligence

### Integrated Information Theory (IIT) for TA

Giulio Tononi's Integrated Information Theory (Tononi, 2004; Tononi et al., 2016) proposes that consciousness arises from systems with high integrated information — measured as Phi (Φ). In Roko, we apply IIT not to measure consciousness but to measure **emergent intelligence** in the TA subsystem: when the 9 TA subsystems working together produce more insight than the sum of their individual contributions.

### The 9 TA subsystems

| # | Subsystem | What it contributes |
|---|---|---|
| 1 | HDC pattern algebra | Structural pattern encoding and cross-domain matching |
| 2 | Spectral liquidity manifolds | Riemannian geometry for execution cost modeling |
| 3 | Adaptive signal metabolism | Evolutionary signal selection and speciation |
| 4 | Causal microstructure discovery | Causal reasoning (Pearl's 3 levels) |
| 5 | Predictive geometry (TDA) | Topological constraints on trajectories |
| 6 | Resonant pattern ecosystem | Multi-signal pattern competition and evolution |
| 7 | Adversarial signal robustness | Defense against manipulation |
| 8 | Somatic technical analysis | Pre-analytical "gut feelings" |
| 9 | Predictive foraging + active inference | Prediction-resolution-calibration loop |

### Phi computation over TA subsystems

```rust
/// Compute Phi (integrated information) across the 9 TA subsystems.
///
/// Phi measures the degree to which the whole system generates more
/// information than the sum of its parts when partitioned.
///
/// For 9 subsystems, there are 2^9 - 2 = 510 possible partitions
/// (excluding the trivial empty and full partitions).
///
/// For each partition, we compute:
///   ΔI = I(whole) - I(part_A) - I(part_B)
///
/// Phi = minimum ΔI across all partitions.
///
/// This is the Minimum Information Bipartition (MIB).
pub struct PhiComputer {
    /// Current state of each TA subsystem.
    subsystem_states: [SubsystemState; 9],

    /// Information flow matrix: how much information flows
    /// from subsystem i to subsystem j.
    flow_matrix: [[f64; 9]; 9],
}

pub struct SubsystemState {
    /// Entropy of the subsystem's output distribution.
    pub entropy: f64,

    /// Mutual information with each other subsystem.
    pub mutual_info: [f64; 9],

    /// The subsystem's current prediction accuracy.
    pub accuracy: f64,
}

impl PhiComputer {
    /// Compute Phi across all 510 bipartitions.
    pub fn compute_phi(&self) -> PhiResult {
        let n = 9;
        let mut min_phi = f64::MAX;
        let mut min_partition = (0u16, 0u16);

        // Enumerate all non-trivial bipartitions
        for mask in 1..(1u16 << n) - 1 {
            let complement = ((1u16 << n) - 1) ^ mask;

            let part_a: Vec<usize> = (0..n).filter(|i| mask & (1 << i) != 0).collect();
            let part_b: Vec<usize> = (0..n).filter(|i| complement & (1 << i) != 0).collect();

            // Information generated by the whole
            let i_whole = self.integrated_information_whole();

            // Information generated by each part independently
            let i_a = self.integrated_information_part(&part_a);
            let i_b = self.integrated_information_part(&part_b);

            // Information lost by partitioning
            let delta_i = i_whole - i_a - i_b;

            if delta_i < min_phi {
                min_phi = delta_i;
                min_partition = (mask, complement);
            }
        }

        PhiResult {
            phi: min_phi,
            mib_partition: min_partition,
            interpretation: self.interpret_phi(min_phi),
        }
    }

    fn interpret_phi(&self, phi: f64) -> PhiInterpretation {
        if phi < 0.1 {
            PhiInterpretation::Modular
            // Subsystems operate independently — no emergent intelligence
        } else if phi < 0.5 {
            PhiInterpretation::WeaklyIntegrated
            // Some cross-subsystem synergy
        } else {
            PhiInterpretation::StronglyIntegrated
            // The TA system is generating insights that no subsystem could alone
        }
    }
}
```

### Minimum Information Bipartition (MIB) as diagnostic

The MIB reveals the system's weakest link — the partition that causes the least information loss:

```rust
/// The MIB diagnostic: which bipartition is the weakest link?
///
/// If the MIB separates {HDC, TDA, Somatic} from {Causal, Manifold, Adversarial, ...},
/// this tells us that the first group operates somewhat independently
/// from the second. Strengthening the connections between these
/// groups would increase Phi.
///
/// Actionable: add more cross-subsystem information flows at the MIB boundary.
pub fn diagnose_mib(phi_result: &PhiResult, subsystem_names: &[&str; 9]) -> MibDiagnosis {
    let (mask_a, mask_b) = phi_result.mib_partition;

    let group_a: Vec<String> = (0..9)
        .filter(|i| mask_a & (1 << i) != 0)
        .map(|i| subsystem_names[i].to_string())
        .collect();

    let group_b: Vec<String> = (0..9)
        .filter(|i| mask_b & (1 << i) != 0)
        .map(|i| subsystem_names[i].to_string())
        .collect();

    MibDiagnosis {
        group_a,
        group_b,
        phi: phi_result.phi,
        recommendation: format!(
            "Increase information flow between groups to raise Phi. \
             Current weakest link: Phi = {:.3}.",
            phi_result.phi
        ),
    }
}
```

### Partial Information Decomposition (PID)

PID (Williams & Beer, 2010) decomposes the information provided by multiple TA subsystems about a target variable into four components:

```rust
/// Partial Information Decomposition for TA subsystem analysis.
///
/// Given two TA subsystems S1 and S2 predicting a target T:
///
/// I(S1, S2 ; T) = Redundancy + Unique_S1 + Unique_S2 + Synergy
///
/// Redundancy: what both S1 and S2 independently know about T
/// Unique_S1: what only S1 knows about T
/// Unique_S2: what only S2 knows about T
/// Synergy: what S1 and S2 together know that neither knows alone
///
/// Synergy is the emergent intelligence: information that only
/// exists in the interaction between subsystems.
pub struct PidAnalysis {
    pub redundancy: f64,
    pub unique_s1: f64,
    pub unique_s2: f64,
    pub synergy: f64,
}

impl PidAnalysis {
    /// Compute PID for two TA subsystems predicting a target.
    pub fn compute(
        s1_predictions: &[f64],
        s2_predictions: &[f64],
        target: &[f64],
    ) -> Self {
        let i_s1_t = mutual_information(s1_predictions, target);
        let i_s2_t = mutual_information(s2_predictions, target);
        let i_s1s2_t = joint_mutual_information(s1_predictions, s2_predictions, target);

        // Williams & Beer (2010) minimum mutual information
        let redundancy = i_s1_t.min(i_s2_t);
        let unique_s1 = i_s1_t - redundancy;
        let unique_s2 = i_s2_t - redundancy;
        let synergy = i_s1s2_t - i_s1_t - i_s2_t + redundancy;

        PidAnalysis { redundancy, unique_s1, unique_s2, synergy }
    }

    /// Is there significant synergy between these subsystems?
    pub fn has_synergy(&self) -> bool {
        self.synergy > 0.05  // threshold for meaningful synergy
    }
}
```

### Synergy detection across all TA subsystem pairs

```rust
/// Detect synergistic pairs across all TA subsystems.
///
/// For each pair of subsystems, compute PID and flag pairs
/// with high synergy — these are producing emergent insights.
pub fn detect_synergies(
    subsystem_outputs: &[[f64; N]; 9],
    target: &[f64; N],
) -> Vec<SynergyPair> {
    let mut pairs = Vec::new();

    for i in 0..9 {
        for j in (i + 1)..9 {
            let pid = PidAnalysis::compute(
                &subsystem_outputs[i],
                &subsystem_outputs[j],
                target,
            );

            if pid.has_synergy() {
                pairs.push(SynergyPair {
                    subsystem_a: i,
                    subsystem_b: j,
                    synergy: pid.synergy,
                    redundancy: pid.redundancy,
                });
            }
        }
    }

    pairs.sort_by(|a, b| b.synergy.partial_cmp(&a.synergy).unwrap());
    pairs
}
```

---

## Integration: Somatic + Multiscale

Somatic markers and multiscale intelligence interact bidirectionally:

1. **Somatic → Multiscale**: Somatic markers provide fast pre-analytical biases that increase the speed of the overall TA system, reducing the information processing burden and potentially increasing Phi (by enabling faster cross-subsystem communication via affect).

2. **Multiscale → Somatic**: When the Phi computation reveals high synergy between two subsystems, somatic markers form at the boundary — encoding the "feeling" of their joint activation. This creates a fast path for future detection of similar synergistic conditions.

3. **Daimon integration**: Both somatic assessment and Phi computation feed into the Daimon PAD vector:
   - Somatic valence → Pleasure dimension
   - Phi value → Dominance dimension (high integration = high confidence)
   - Synergy detection → Arousal dimension (novel synergy = surprise)

---

## Implementation details

### PAD encoding: AffectCodebook generation

The AffectCodebook uses the same deterministic generation as other HDC codebooks (see [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md)), seeded with domain = "affect":

```rust
/// AffectCodebook for PAD encoding in HDC space.
///
/// Generated deterministically from seed "affect".
/// Three role vectors for the three PAD dimensions.
/// One shared QuantizedCodebook for value encoding (range [-1.0, 1.0]).
pub struct AffectCodebook {
    /// Role vector for the Pleasure dimension.
    pub pleasure_role: HdcVector,
    /// Role vector for the Arousal dimension.
    pub arousal_role: HdcVector,
    /// Role vector for the Dominance dimension.
    pub dominance_role: HdcVector,
    /// Shared quantized codebook for PAD values.
    /// Range: [-1.0, 1.0], n_levels: 32.
    pub value_codebook: QuantizedCodebook,
}

impl AffectCodebook {
    pub fn new(dim: usize) -> Self {
        let gen = CodebookGenerator::new("affect", dim);
        Self {
            pleasure_role: gen.generate_role(0),
            arousal_role: gen.generate_role(1),
            dominance_role: gen.generate_role(2),
            value_codebook: gen.generate_quantized(100, 32, -1.0, 1.0),
        }
    }
}
```

**Configuration parameters**:

| Parameter | Default | Range | Notes |
|---|---|---|---|
| `dim` | 10,240 | Must match global HDC dimensionality | Shared with all other codebooks. |
| `n_levels` | 32 | 16 - 64 | 32 gives ~6.25% resolution per PAD dimension. Sufficient for affect encoding. |
| `value_range` | [-1.0, 1.0] | Fixed | Matches Mehrabian-Russell PAD range. |

The quantization levels are generated via thermometer construction: each adjacent level differs by `dim / (2 * 32) = 160` bits. Two PAD states differing by 0.1 on one dimension have similarity ~0.975 on that dimension's component.

### Somatic retrieval: k-d tree in 8D

The somatic map can contain thousands of markers. Linear scan is adequate for < 5,000 markers (~315 microseconds at 63ns/comparison). For larger collections, use a k-d tree on the PAD+pattern summary space:

```rust
/// Somatic marker index for fast retrieval.
///
/// Each marker is projected into an 8-dimensional space:
///   [pleasure, arousal, dominance,         // 3 PAD dims
///    pattern_pca_0, ..., pattern_pca_4]    // 5 PCA dims of pattern vector
///
/// The PCA projection compresses the 10,240-bit pattern vector into
/// 5 f64 dimensions that capture the most variance. This loses some
/// information but enables tree-based spatial indexing.
///
/// Build cost: O(n * log(n)) where n = marker count.
/// Query cost: O(log(n)) average, O(n) worst case (high-dimensional curse).
/// For n = 10,000 markers: ~10x faster than linear scan.
pub struct SomaticIndex {
    /// k-d tree over the 8D projection space.
    tree: KdTree<f64, usize, 8>,

    /// PCA projection matrix (10,240 -> 5 dimensions).
    pca_matrix: [[f64; 5]; 10_240],

    /// The underlying markers (indexed by position in this vec).
    markers: Vec<SomaticMarker>,
}

impl SomaticIndex {
    /// Build the index from a collection of markers.
    pub fn build(markers: Vec<SomaticMarker>) -> Self {
        let pca_matrix = compute_pca_projection(&markers, 5);
        let mut tree = KdTree::new(8);

        for (idx, marker) in markers.iter().enumerate() {
            let point = Self::project(marker, &pca_matrix);
            tree.add(point, idx).unwrap();
        }

        Self { tree, pca_matrix, markers }
    }

    /// Query: find k nearest markers to a pattern + PAD query.
    pub fn query_nearest(
        &self,
        pattern: &HdcVector,
        pad_hint: (f64, f64, f64),
        k: usize,
    ) -> Vec<(f64, &SomaticMarker)> {
        let query_point = self.project_query(pattern, pad_hint);
        self.tree.nearest(&query_point, k, &squared_euclidean)
            .unwrap()
            .into_iter()
            .map(|(dist, &idx)| (dist.sqrt(), &self.markers[idx]))
            .collect()
    }
}
```

**Distance metric**: Squared Euclidean in the 8D projection space. The PAD dimensions and PCA dimensions are on different scales, so normalize each dimension to unit variance before building the tree.

**Unbinding operation**: To compare just the pattern component of a somatic marker (ignoring affect), unbind by XORing the marker vector with its affect vector: `pattern_component = marker_hv XOR affect_hv`. This recovers the approximate pattern vector (XOR is its own inverse in BSC).

### Phi computation: information flow matrix

The 9x9 information flow matrix `flow_matrix[i][j]` measures how much information flows from subsystem i to subsystem j:

```rust
/// Compute the information flow matrix across TA subsystems.
///
/// Method: for each pair (i, j), compute the transfer entropy
/// from subsystem i's output time series to subsystem j's output
/// time series over the last window_size observations.
///
/// Transfer entropy T(i -> j) measures the reduction in uncertainty
/// about j's next state when knowing i's past states, beyond what
/// j's own past provides.
///
///   T(i->j) = H(j_t | j_{t-1..t-k}) - H(j_t | j_{t-1..t-k}, i_{t-1..t-k})
///
/// where H is conditional entropy and k is the lag order.
pub struct FlowMatrixComputer {
    /// Number of lag steps for transfer entropy.
    pub lag_order: usize,          // default: 3
    /// Observation window size.
    pub window_size: usize,        // default: 100
    /// Number of histogram bins for entropy estimation.
    pub n_bins: usize,             // default: 10
}

impl FlowMatrixComputer {
    pub fn compute(
        &self,
        subsystem_outputs: &[[f64]; 9],
    ) -> [[f64; 9]; 9] {
        let mut flow = [[0.0; 9]; 9];
        for i in 0..9 {
            for j in 0..9 {
                if i != j {
                    flow[i][j] = transfer_entropy(
                        &subsystem_outputs[i],
                        &subsystem_outputs[j],
                        self.lag_order,
                        self.n_bins,
                    );
                }
            }
        }
        flow
    }
}
```

**Temporal lag model**: Transfer entropy uses lag_order = 3 by default (looks 3 time steps back). At Theta frequency (~75s), this covers ~225s of history. For subsystems that communicate at different speeds (e.g., HDC is instantaneous, TDA requires batch computation), the lag order should be adjusted per pair.

### Minimum information bipartition: algorithm for n = 9

With 9 subsystems, there are `2^9 - 2 = 510` non-trivial bipartitions. This is small enough for exhaustive enumeration:

```rust
/// Enumerate all 510 bipartitions and find the MIB.
///
/// For each bipartition (A, B):
///   1. Compute I(whole) = sum of all transfer entropies in the flow matrix.
///   2. Compute I(A) = sum of transfer entropies within subsystems in A.
///   3. Compute I(B) = sum of transfer entropies within subsystems in B.
///   4. delta_I = I(whole) - I(A) - I(B).
///   5. Track the bipartition with minimum delta_I.
///
/// Cost: 510 iterations, each O(81) operations on the flow matrix.
/// Total: ~41K arithmetic operations. Negligible (< 1ms).
pub fn find_mib(flow_matrix: &[[f64; 9]; 9]) -> (u16, u16, f64) {
    let n = 9;
    let i_whole: f64 = flow_matrix.iter().flat_map(|row| row.iter()).sum();
    let mut min_delta = f64::MAX;
    let mut min_mask = (0u16, 0u16);

    for mask in 1u16..(1 << n) - 1 {
        let complement = ((1u16 << n) - 1) ^ mask;

        let i_a: f64 = (0..n).flat_map(|i| (0..n).map(move |j| (i, j)))
            .filter(|(i, j)| mask & (1 << i) != 0 && mask & (1 << j) != 0)
            .map(|(i, j)| flow_matrix[i][j])
            .sum();

        let i_b: f64 = (0..n).flat_map(|i| (0..n).map(move |j| (i, j)))
            .filter(|(i, j)| complement & (1 << i) != 0 && complement & (1 << j) != 0)
            .map(|(i, j)| flow_matrix[i][j])
            .sum();

        let delta = i_whole - i_a - i_b;
        if delta < min_delta {
            min_delta = delta;
            min_mask = (mask, complement);
        }
    }

    (min_mask.0, min_mask.1, min_delta)
}
```

**Scalability note**: For n = 9, exhaustive enumeration is trivial. For n > 20, the number of bipartitions exceeds 10^6 and heuristic search (e.g., spectral bisection on the flow matrix) becomes necessary. This is not an issue for the current 9-subsystem architecture.

### Partial information decomposition: algorithm and bias correction

The PID implementation uses the Williams-Beer I_min (minimum specific information) approach:

```rust
/// PID computation using Williams-Beer I_min.
///
/// For two sources S1, S2 and target T:
///   Redundancy = I_min(S1; T) where I_min is the minimum specific info.
///   I_min is computed over all realizations t of T:
///     I_min(S1, S2; T) = sum_t p(t) * min(I_spec(S1; t), I_spec(S2; t))
///   where I_spec(S; t) = sum_s p(s|t) * log(p(s|t) / p(s)).
///
/// Sample size requirement: at least 5 * n_bins^2 observations
/// to avoid severe estimation bias.
pub struct PidConfig {
    /// Number of histogram bins per variable.
    pub n_bins: usize,         // default: 5
    /// Minimum sample size: 5 * n_bins^2 = 125 with default.
    pub min_samples: usize,    // derived: 5 * n_bins * n_bins
    /// Bias correction method.
    pub bias_correction: BiasCorrection,
}

pub enum BiasCorrection {
    /// No correction (raw plugin estimator).
    None,
    /// Miller-Madow correction: subtract (|alphabet| - 1) / (2 * n).
    MillerMadow,
    /// Jackknife resampling: leave-one-out estimate of bias.
    /// More accurate but O(n) times more expensive.
    Jackknife,
}
```

**Recommended settings**: Use `n_bins = 5` and `MillerMadow` bias correction for routine monitoring. Switch to `Jackknife` for publication-quality Phi/PID estimates. The Miller-Madow correction subtracts `(k - 1) / (2n)` from each entropy estimate, where k is the number of non-empty bins and n is the sample size.

### State machine: Phi/somatic markers feeding Daimon updates

The Phi computation and somatic assessment update the Daimon on a schedule:

```
THETA TICK (every ~75s):
  1. Evaluate somatic assessment for the current TA state.
     -> If somatic valence is strong (|pleasure| > 0.5):
        Update Daimon.pleasure += 0.3 * somatic_pleasure.
  2. No Phi computation (too expensive for Theta frequency).

DELTA TICK (every few hours):
  1. Compute the 9x9 information flow matrix from recent Theta outputs.
  2. Find MIB and compute Phi.
  3. Compute PID for all 36 subsystem pairs.
  4. Update Daimon:
     - Phi > 0.5 -> Daimon.dominance += 0.2 (high integration = high confidence)
     - New synergy detected (PID synergy > 0.1 for a pair that was
       previously < 0.05) -> Daimon.arousal += 0.3 (surprise)
  5. Form somatic markers at synergistic boundaries:
     - For each high-synergy pair (i, j), encode the joint activation
       pattern and bind it with the positive affect of discovery.
     - These markers enable fast future detection of similar synergistic
       conditions at Theta frequency (avoiding the expensive Phi computation).
  6. Log Phi value and MIB partition to .roko/learn/phi.jsonl.
```

**Computation frequency**: Phi is computed at Delta frequency only. At Theta, somatic markers serve as fast proxies for the Phi-derived state. This two-speed design keeps Theta ticks cheap while still incorporating multiscale intelligence insights.

### Error handling

- **Empty somatic map**: If no markers exist, somatic retrieval returns a neutral assessment (pleasure = 0.0, arousal = 0.0, dominance = 0.0, confidence = 0.0).
- **All markers expired**: Same as empty map. Log a warning suggesting that either the system is too new or the decay rate is too aggressive.
- **Zero weight in somatic aggregation**: If total weight is zero (all matched markers have zero strength), return neutral assessment.
- **Degenerate flow matrix**: If all transfer entropies are zero (no inter-subsystem communication), Phi = 0 and MIB is arbitrary. This indicates the subsystems are operating independently.
- **Insufficient data for PID**: If sample count < `min_samples`, skip PID computation and report `synergy = NaN` with a warning.
- **PCA failure in somatic index**: If the marker set has fewer than 5 unique patterns, reduce PCA dimensions to match. If fewer than 2, fall back to linear scan.

### Test criteria

- **PAD encoding round-trip**: Encode PAD (0.5, -0.3, 0.8) as HDC vector, then decode by unbinding each role. The decoded values should be within 0.1 of the originals (limited by quantization).
- **Somatic retrieval correctness**: Store a marker for pattern A with positive pleasure. Query with a pattern similar to A. The assessment should have positive valence.
- **Contrarian retrieval**: Somatic retrieval always returns at least `ceil(n_matches * 0.15)` contrarian markers when available.
- **Phi monotonicity**: Adding a strong inter-subsystem connection (increasing one flow_matrix entry by 1.0) does not decrease Phi.
- **MIB exhaustiveness**: For n = 9, exactly 510 bipartitions are evaluated.
- **PID non-negativity**: Redundancy, unique_s1, unique_s2 are all >= 0. Synergy can be negative (indicates suppression).
- **State machine scheduling**: Phi is never computed at Theta frequency. Somatic assessment is computed at every Theta tick.

---

## Academic foundations

- Damasio, A. R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. Putnam. — Somatic marker hypothesis.
- Mehrabian, A., & Russell, J. A. (1974). *An Approach to Environmental Psychology*. MIT Press. — PAD model.
- Bower, G. H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129-148. — Mood-congruent memory (15% contrarian retrieval).
- Kahneman, D. (2011). *Thinking, Fast and Slow*. Farrar, Straus and Giroux. — System 1/System 2 dual-process theory.
- Tononi, G. (2004). "An information integration theory of consciousness." *BMC Neuroscience*, 5(42). — IIT Phi.
- Tononi, G., Boly, M., Massimini, M., & Koch, C. (2016). "Integrated information theory." *Nature Reviews Neuroscience*, 17(7), 450-461. — IIT 3.0.
- Williams, P. L., & Beer, R. D. (2010). "Nonnegative decomposition of multivariate information." arXiv:1004.2515. — PID framework.
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). — HDC for somatic marker encoding.

---

## Cross-References

- See [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) for HDC encoding of somatic markers
- See [08-adaptive-signal-metabolism.md](./08-adaptive-signal-metabolism.md) for the signal ecosystem that somatic markers modulate
- See [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) for resonant patterns interacting with somatic markers
- See [11-adversarial-signal-robustness.md](./11-adversarial-signal-robustness.md) for adversarial robustness feeding somatic markers
