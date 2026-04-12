# Somatic Technical Analysis and Emergent Multiscale Intelligence

> Somatic TA uses Damasio's somatic marker hypothesis to create "gut feelings" about TA patterns. Emergent multiscale intelligence measures integrated information (IIT Phi) across the TA subsystems, detecting when the whole is greater than the sum of its parts.

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

## Cross-references

- See [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) for HDC encoding of somatic markers
- See [08-adaptive-signal-metabolism.md](./08-adaptive-signal-metabolism.md) for the signal ecosystem that somatic markers modulate
- See [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) for resonant patterns interacting with somatic markers
- See [11-adversarial-signal-robustness.md](./11-adversarial-signal-robustness.md) for adversarial robustness feeding somatic markers
