//! Adversarial signal detection using HDC prototype matching.
//!
//! Each incoming signal is compared against a library of known attack
//! prototypes using [`HdcVector::similarity()`] (~10ns per check). If
//! similarity exceeds a configurable threshold (default 0.7), the signal
//! is flagged as adversarial.
//!
//! Attack prototype families:
//! - **Chain**: sandwich attack, oracle manipulation, flash loan, governance attack
//! - **Coding**: prompt injection, path traversal, dependency confusion
//! - **Universal**: replay attack, data poisoning, model extraction

use roko_primitives::HdcVector;

// ---------------------------------------------------------------------------
// Attack taxonomy
// ---------------------------------------------------------------------------

/// Broad category of a detected attack.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AttackDomain {
    /// On-chain / DeFi attack patterns.
    Chain,
    /// Software / agent coding attack patterns.
    Coding,
    /// Domain-agnostic attack patterns.
    Universal,
}

/// Specific attack type within a domain.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AttackType {
    // -- Chain --
    /// Sandwich / front-running attack.
    SandwichAttack,
    /// Oracle price manipulation.
    OracleManipulation,
    /// Flash-loan exploit.
    FlashLoan,
    /// Governance capture.
    GovernanceAttack,

    // -- Coding --
    /// Prompt injection attempt.
    PromptInjection,
    /// Path traversal / directory escape.
    PathTraversal,
    /// Dependency confusion / typo-squatting.
    DependencyConfusion,

    // -- Universal --
    /// Replay of a previously observed signal.
    ReplayAttack,
    /// Data poisoning of training signals.
    DataPoisoning,
    /// Extraction of internal model parameters.
    ModelExtraction,
}

impl AttackType {
    /// Return the domain category for this attack type.
    pub fn domain(&self) -> AttackDomain {
        match self {
            Self::SandwichAttack
            | Self::OracleManipulation
            | Self::FlashLoan
            | Self::GovernanceAttack => AttackDomain::Chain,

            Self::PromptInjection
            | Self::PathTraversal
            | Self::DependencyConfusion => AttackDomain::Coding,

            Self::ReplayAttack
            | Self::DataPoisoning
            | Self::ModelExtraction => AttackDomain::Universal,
        }
    }
}

// ---------------------------------------------------------------------------
// Adversarial detector
// ---------------------------------------------------------------------------

/// An attack prototype: a known-bad pattern encoded as an HDC vector.
#[derive(Clone, Debug)]
pub struct AttackPrototype {
    /// HDC fingerprint of the attack pattern.
    pub vector: HdcVector,
    /// What kind of attack this prototype represents.
    pub attack_type: AttackType,
    /// Human-readable label for logging.
    pub label: String,
}

/// Result of checking a signal against the prototype library.
#[derive(Clone, Debug)]
pub struct DetectionResult {
    /// The matched attack type.
    pub attack_type: AttackType,
    /// Similarity score between the signal and the prototype.
    pub similarity: f32,
    /// Label of the matched prototype.
    pub label: String,
}

/// HDC-based adversarial signal detector.
///
/// Maintains a library of attack prototypes and checks incoming signals
/// for similarity matches.
#[derive(Clone, Debug, Default)]
pub struct AdversarialDetector {
    /// Known-bad prototypes.
    prototypes: Vec<AttackPrototype>,
    /// Similarity threshold above which a signal is flagged.
    threshold: f32,
}

impl AdversarialDetector {
    /// Create a new detector with the given similarity threshold.
    ///
    /// A threshold of 0.7 is recommended (spec default).
    pub fn new(threshold: f32) -> Self {
        Self {
            prototypes: Vec::new(),
            threshold,
        }
    }

    /// Create a detector pre-loaded with deterministic seed-based prototypes
    /// for each attack type.
    pub fn with_default_prototypes(threshold: f32) -> Self {
        let mut det = Self::new(threshold);
        let attacks: [(AttackType, &str, u64); 10] = [
            (AttackType::SandwichAttack, "chain:sandwich", 0xDEAD_0001),
            (AttackType::OracleManipulation, "chain:oracle_manip", 0xDEAD_0002),
            (AttackType::FlashLoan, "chain:flash_loan", 0xDEAD_0003),
            (AttackType::GovernanceAttack, "chain:governance", 0xDEAD_0004),
            (AttackType::PromptInjection, "coding:prompt_inject", 0xDEAD_0005),
            (AttackType::PathTraversal, "coding:path_traversal", 0xDEAD_0006),
            (AttackType::DependencyConfusion, "coding:dep_confusion", 0xDEAD_0007),
            (AttackType::ReplayAttack, "universal:replay", 0xDEAD_0008),
            (AttackType::DataPoisoning, "universal:data_poison", 0xDEAD_0009),
            (AttackType::ModelExtraction, "universal:model_extract", 0xDEAD_000A),
        ];
        for (attack_type, label, seed) in attacks {
            let seed_bytes = seed.to_le_bytes();
            det.add_prototype(AttackPrototype {
                vector: HdcVector::from_seed(&seed_bytes),
                attack_type,
                label: label.to_string(),
            });
        }
        det
    }

    /// Add an attack prototype to the library.
    pub fn add_prototype(&mut self, proto: AttackPrototype) {
        self.prototypes.push(proto);
    }

    /// Check a signal against all prototypes.
    ///
    /// Returns `Some(DetectionResult)` if similarity exceeds the threshold,
    /// `None` if the signal appears clean. Runtime is O(n) in prototype
    /// count, with each comparison being ~10ns (HDC Hamming distance).
    pub fn check_signal(&self, signal: &HdcVector) -> Option<DetectionResult> {
        let mut best: Option<(f32, usize)> = None;
        for (i, proto) in self.prototypes.iter().enumerate() {
            let sim = signal.similarity(&proto.vector);
            if sim >= self.threshold {
                if best.map_or(true, |(s, _)| sim > s) {
                    best = Some((sim, i));
                }
            }
        }
        best.map(|(sim, idx)| {
            let proto = &self.prototypes[idx];
            DetectionResult {
                attack_type: proto.attack_type.clone(),
                similarity: sim,
                label: proto.label.clone(),
            }
        })
    }

    /// Check a signal and return all matching prototypes above threshold,
    /// sorted by similarity (descending).
    pub fn check_signal_all(&self, signal: &HdcVector) -> Vec<DetectionResult> {
        let mut results: Vec<DetectionResult> = self
            .prototypes
            .iter()
            .filter_map(|proto| {
                let sim = signal.similarity(&proto.vector);
                if sim >= self.threshold {
                    Some(DetectionResult {
                        attack_type: proto.attack_type.clone(),
                        similarity: sim,
                        label: proto.label.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();
        results.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    /// Return the number of prototypes in the library.
    pub fn prototype_count(&self) -> usize {
        self.prototypes.len()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detector_flags_matching_signal() {
        let proto_vec = HdcVector::from_seed(b"test_seed_42");
        let mut det = AdversarialDetector::new(0.7);
        det.add_prototype(AttackPrototype {
            vector: proto_vec,
            attack_type: AttackType::PromptInjection,
            label: "test".to_string(),
        });
        // Same vector -> similarity 1.0
        let result = det.check_signal(&proto_vec);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.attack_type, AttackType::PromptInjection);
        assert!((r.similarity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn detector_passes_random_signal() {
        let det = AdversarialDetector::with_default_prototypes(0.7);
        // A random vector should not match any prototype.
        let random_signal = HdcVector::from_seed(b"random_cafe_babe");
        let result = det.check_signal(&random_signal);
        assert!(result.is_none(), "random signal should not match prototypes");
    }

    #[test]
    fn default_prototypes_loaded() {
        let det = AdversarialDetector::with_default_prototypes(0.7);
        assert_eq!(det.prototype_count(), 10);
    }

    #[test]
    fn check_all_returns_sorted() {
        let proto1 = HdcVector::from_seed(b"proto_seed_1");
        let proto2 = HdcVector::from_seed(b"proto_seed_2");
        let mut det = AdversarialDetector::new(0.0); // threshold 0 to get all matches
        det.add_prototype(AttackPrototype {
            vector: proto1,
            attack_type: AttackType::SandwichAttack,
            label: "a".to_string(),
        });
        det.add_prototype(AttackPrototype {
            vector: proto2,
            attack_type: AttackType::FlashLoan,
            label: "b".to_string(),
        });
        let results = det.check_signal_all(&proto1);
        assert!(!results.is_empty());
        // First result should be the exact match (similarity ~1.0).
        assert_eq!(results[0].attack_type, AttackType::SandwichAttack);
        // Results sorted descending.
        for w in results.windows(2) {
            assert!(w[0].similarity >= w[1].similarity);
        }
    }

    #[test]
    fn attack_type_domain_mapping() {
        assert_eq!(AttackType::SandwichAttack.domain(), AttackDomain::Chain);
        assert_eq!(AttackType::PromptInjection.domain(), AttackDomain::Coding);
        assert_eq!(AttackType::ReplayAttack.domain(), AttackDomain::Universal);
    }
}
