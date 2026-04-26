//! Phase 2 threat-simulation stubs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::phase2::shared::{BasicEvent, ModelTier, ThreatTier};

/// Configuration for systematic threat generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreatGenerator {
    /// Minimum severity before a failure mode is retained.
    pub severity_threshold: u8,
    /// RPN threshold for immediate attention.
    pub rpn_immediate_threshold: u16,
    /// RPN threshold for scheduled attention.
    pub rpn_scheduled_threshold: u16,
    /// Maximum failure modes generated per component.
    pub max_modes_per_component: usize,
    /// Maximum depth of generated fault trees.
    pub max_fta_depth: usize,
    /// Whether adversarial generation is enabled.
    pub use_adversarial_generation: bool,
    /// Model tier used for adversarial generation.
    pub adversarial_model_tier: ModelTier,
}

/// One failure mode enumerated during threat generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureMode {
    /// Stable failure-mode identifier.
    pub id: String,
    /// Component in which the failure occurs.
    pub component: String,
    /// Human-readable description of the failure mode.
    pub description: String,
    /// Severity score on the FMEA scale.
    pub severity: u8,
    /// Occurrence score on the FMEA scale.
    pub occurrence: u8,
    /// Detection score on the FMEA scale.
    pub detection: u8,
    /// Risk-priority number.
    pub rpn: u16,
    /// Threat tier assigned to the failure mode.
    pub tier: ThreatTier,
    /// Optional mitigation guidance.
    pub mitigation: Option<String>,
    /// Early warning signals for the mode.
    pub early_warning_signs: Vec<String>,
}

/// Fault tree rooted at one top-level threat event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaultTree {
    /// Top event explained by the tree.
    pub top_event: String,
    /// Logical gates in the tree.
    pub gates: Vec<FaultGate>,
    /// Basic events feeding the tree.
    pub basic_events: Vec<BasicEvent>,
    /// Minimal cut sets for the top event.
    pub minimal_cut_sets: Vec<Vec<String>>,
}

/// Logical gate type used inside a fault tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultGateType {
    /// All inputs must fire.
    And,
    /// Any input may fire.
    Or,
}

/// One logical gate inside a fault tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultGate {
    /// Stable gate identifier.
    pub id: String,
    /// Verify operator.
    pub gate_type: FaultGateType,
    /// Input event identifiers.
    pub inputs: Vec<String>,
    /// Output event identifier.
    pub output: String,
}

/// Configuration for risk-zone assignment and prioritization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreatSeverityAssessor {
    /// Number of likelihood buckets in the risk matrix.
    pub likelihood_levels: usize,
    /// Number of impact buckets in the risk matrix.
    pub impact_levels: usize,
    /// Threshold for the critical risk zone.
    pub critical_threshold: f64,
    /// Threshold for the high risk zone.
    pub high_threshold: f64,
    /// Threshold for the medium risk zone.
    pub medium_threshold: f64,
    /// Impact level that triggers automatic critical classification.
    pub impact_override_level: u8,
    /// Prior probability used for Bayesian updates.
    pub default_prior: f64,
}

/// Result of severity assessment for one threat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreatAssessment {
    /// Threat being assessed.
    pub threat_id: String,
    /// Estimated likelihood.
    pub likelihood: f64,
    /// Estimated impact.
    pub impact: f64,
    /// Composite risk score.
    pub risk_score: f64,
    /// Assigned risk zone.
    pub risk_zone: RiskZone,
    /// Expected loss under the current estimate.
    pub expected_loss: f64,
    /// Economic value of mitigation.
    pub mitigation_evm: f64,
    /// Priority rank among assessed threats.
    pub priority_rank: usize,
}

/// Coarse risk zones used during prioritization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskZone {
    /// Low risk.
    Low,
    /// Medium risk.
    Medium,
    /// High risk.
    High,
    /// Critical risk.
    Critical,
}

/// Configuration for advanced red-teaming during dreaming.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvancedRedTeamConfig {
    /// Whether tree-structured attack exploration is enabled.
    pub tree_structured_attacks: bool,
    /// Maximum attack-tree depth.
    pub max_attack_tree_depth: usize,
    /// Maximum branches per attack-tree node.
    pub max_branches_per_node: usize,
    /// Probability threshold below which branches are pruned.
    pub prune_threshold: f64,
    /// Whether waking failure episodes are mined for attack patterns.
    pub mine_failure_episodes: bool,
    /// Maximum primitives composed into one compound attack.
    pub max_primitives_per_compound: usize,
    /// Whether attack knowledge is persisted across cycles.
    pub persistent_attack_knowledge: bool,
    /// Path to the persisted attack-knowledge store.
    pub attack_knowledge_path: PathBuf,
}

/// One modular attack primitive used for compound threats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttackPrimitive {
    /// Stable primitive identifier.
    pub id: String,
    /// Human-readable primitive name.
    pub name: String,
    /// Attack category.
    pub category: AttackCategory,
    /// Human-readable description.
    pub description: String,
    /// Historical success rate for the primitive.
    pub historical_success_rate: f64,
    /// Other primitives that compose well with this one.
    pub composable_with: Vec<String>,
    /// How the primitive was discovered.
    pub source: AttackPrimitiveSource,
}

/// Coarse categories for attack primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackCategory {
    /// Input-manipulation attacks.
    InputManipulation,
    /// Timing exploitation.
    TimingExploitation,
    /// Resource-exhaustion attacks.
    ResourceExhaustion,
    /// Context-poisoning attacks.
    ContextPoisoning,
    /// Privilege-escalation attacks.
    PrivilegeEscalation,
    /// Logic-bypass attacks.
    LogicBypass,
    /// State-corruption attacks.
    StateCorruption,
}

/// Provenance for an attack primitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttackPrimitiveSource {
    /// Primitive mined from waking failure data.
    WakingFailure {
        /// Episode that exposed the primitive.
        episode_id: String,
    },
    /// Primitive generated during adversarial dreaming.
    AdversarialGeneration {
        /// Dream cycle that generated the primitive.
        dream_cycle_id: String,
    },
    /// Primitive inherited from another agent.
    MeshInherited {
        /// Source agent that shared the primitive.
        source_agent: String,
    },
    /// Primitive synthesized from other primitives.
    Synthesized {
        /// Parent primitives used in the synthesis.
        parent_primitives: Vec<String>,
    },
}

/// Configuration for constitutional classification of dream outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstitutionalClassifierConfig {
    /// Whether constitutional classification is enabled.
    pub enabled: bool,
    /// Path to the constitutional rule set.
    pub constitution_path: PathBuf,
    /// Model tier used by the classifier.
    pub classifier_tier: ModelTier,
    /// Whether dream prompts are classified in addition to outputs.
    pub classify_inputs: bool,
    /// Whether token-level detection is enabled.
    pub token_level_detection: bool,
    /// Maximum tolerated false-positive rate.
    pub max_false_positive_rate: f64,
}
