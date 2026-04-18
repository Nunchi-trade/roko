//! Coordination primitives for pheromones, subnets, morphogenesis, and
//! cohort intelligence.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// Stable identifier for an agent.
pub type AgentId = String;

/// Stable identifier for a collective.
pub type CollectiveId = String;

/// Stable identifier for a substrate.
pub type SubstrateId = String;

/// A normalized c-factor / specialization score.
pub type SpecializationIndex = f64;

const STRATEGY_DIMS_F64: f64 = 8.0;

const BUILTIN_KIND_NAMES: [&str; 7] = [
    "threat",
    "opportunity",
    "wisdom",
    "alpha",
    "pattern",
    "anomaly",
    "consensus",
];

const fn clamp_unit(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn safe_half_life_fraction(age: Duration, half_life: Duration) -> f64 {
    if half_life.is_zero() {
        return f64::INFINITY;
    }
    age.as_secs_f64() / half_life.as_secs_f64()
}

/// Errors returned when validating a custom pheromone kind identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomKindError {
    /// The identifier was empty or longer than 64 characters.
    InvalidLength {
        /// The offending string length.
        len: usize,
    },
    /// The identifier started with `_`, which is reserved for internal kinds.
    ReservedPrefix,
    /// The identifier contained a character outside `[A-Za-z0-9_]`.
    InvalidCharacter(char),
    /// The identifier collides with a built-in kind name.
    BuiltInCollision {
        /// The colliding identifier.
        name: String,
    },
}

impl fmt::Display for CustomKindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { len } => {
                write!(f, "custom kind identifier must be 1..=64 chars, got {len}")
            }
            Self::ReservedPrefix => write!(f, "custom kind identifiers may not start with '_'"),
            Self::InvalidCharacter(ch) => write!(
                f,
                "custom kind identifiers must use ASCII alphanumeric characters or '_' (found {ch:?})"
            ),
            Self::BuiltInCollision { name } => {
                write!(
                    f,
                    "custom kind identifier collides with built-in kind '{name}'"
                )
            }
        }
    }
}

impl std::error::Error for CustomKindError {}

/// Validate a custom pheromone kind identifier.
///
/// Rules:
/// - ASCII alphanumeric and underscores only
/// - 1..=64 characters
/// - Must not start with `_`
/// - Must not collide with a built-in kind name
pub fn validate_custom_kind(id: &str) -> Result<(), CustomKindError> {
    if id.is_empty() || id.len() > 64 {
        return Err(CustomKindError::InvalidLength { len: id.len() });
    }
    if id.starts_with('_') {
        return Err(CustomKindError::ReservedPrefix);
    }
    for ch in id.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_') {
            return Err(CustomKindError::InvalidCharacter(ch));
        }
    }
    let lower = id.to_ascii_lowercase();
    if BUILTIN_KIND_NAMES.iter().any(|name| *name == lower) {
        return Err(CustomKindError::BuiltInCollision { name: id.into() });
    }
    Ok(())
}

/// Errors returned when validating a subnet identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubnetIdError {
    /// The subnet name was empty or longer than 64 characters.
    InvalidLength {
        /// The offending string length.
        len: usize,
    },
    /// The subnet name contained an invalid character.
    InvalidCharacter(char),
}

impl fmt::Display for SubnetIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { len } => {
                write!(f, "subnet name must be 1..=64 chars, got {len}")
            }
            Self::InvalidCharacter(ch) => {
                write!(
                    f,
                    "subnet name must use ASCII alphanumeric characters or '-' (found {ch:?})"
                )
            }
        }
    }
}

impl std::error::Error for SubnetIdError {}

/// A permissioned subnet within a collective.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubnetId {
    /// The parent collective.
    pub collective: CollectiveId,
    /// The subnet name, unique within the collective.
    pub name: String,
}

impl SubnetId {
    /// Construct a subnet identifier after validating the name.
    pub fn new(collective: CollectiveId, name: impl Into<String>) -> Result<Self, SubnetIdError> {
        let name = name.into();
        validate_subnet_name(&name)?;
        Ok(Self { collective, name })
    }
}

impl fmt::Display for SubnetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.collective, self.name)
    }
}

/// Validate a subnet name.
///
/// Rules:
/// - ASCII alphanumeric characters or hyphen only
/// - 1..=64 characters
pub fn validate_subnet_name(name: &str) -> Result<(), SubnetIdError> {
    if name.is_empty() || name.len() > 64 {
        return Err(SubnetIdError::InvalidLength { len: name.len() });
    }
    for ch in name.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '-') {
            return Err(SubnetIdError::InvalidCharacter(ch));
        }
    }
    Ok(())
}

/// The type of coordination signal a pheromone carries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PheromoneKind {
    /// Something dangerous or harmful has been detected.
    Threat,
    /// A favorable condition has been detected.
    Opportunity,
    /// Validated knowledge or insight that should persist.
    Wisdom,
    /// First-mover advantage or ephemeral edge.
    Alpha,
    /// Recurring structure or regularity detected.
    Pattern,
    /// Something unusual or unexpected detected.
    Anomaly,
    /// Collective agreement on a fact or decision.
    Consensus,
    /// User-defined pheromone kind for domain-specific signals.
    Custom(String),
}

impl PheromoneKind {
    /// Return the built-in canonical name when the kind is not custom.
    #[must_use]
    pub const fn builtin_name(&self) -> Option<&'static str> {
        match self {
            Self::Threat => Some("Threat"),
            Self::Opportunity => Some("Opportunity"),
            Self::Wisdom => Some("Wisdom"),
            Self::Alpha => Some("Alpha"),
            Self::Pattern => Some("Pattern"),
            Self::Anomaly => Some("Anomaly"),
            Self::Consensus => Some("Consensus"),
            Self::Custom(_) => None,
        }
    }

    /// Return true when the kind is built in.
    #[must_use]
    pub const fn is_builtin(&self) -> bool {
        !matches!(self, Self::Custom(_))
    }

    /// Construct a validated custom pheromone kind.
    pub fn custom(name: impl Into<String>) -> Result<Self, CustomKindError> {
        let name = name.into();
        validate_custom_kind(&name)?;
        Ok(Self::Custom(name))
    }

    /// Return the documented default half-life for this kind.
    ///
    /// For custom kinds, the default is 6 hours unless a domain-specific
    /// configuration overrides it elsewhere.
    #[must_use]
    pub const fn default_half_life(&self) -> Duration {
        match self {
            Self::Threat => Duration::from_secs(2 * 60 * 60),
            Self::Opportunity => Duration::from_secs(4 * 60 * 60),
            Self::Wisdom => Duration::from_secs(24 * 60 * 60),
            Self::Alpha => Duration::from_secs(60 * 60),
            Self::Pattern => Duration::from_secs(12 * 60 * 60),
            Self::Anomaly | Self::Custom(_) => Duration::from_secs(6 * 60 * 60),
            Self::Consensus => Duration::from_secs(48 * 60 * 60),
        }
    }
}

/// The propagation scope of a digital pheromone.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PheromoneScope {
    /// Pheromone is visible only within the specified substrate.
    Local(SubstrateId),
    /// Pheromone propagates to all agents in the specified collective.
    Mesh(CollectiveId),
    /// Pheromone propagates within a permissioned subnet.
    Subnet(SubnetId),
    /// Pheromone is published globally.
    Global,
}

impl PheromoneScope {
    const fn rank(&self) -> u8 {
        match self {
            Self::Local(_) => 0,
            Self::Subnet(_) => 1,
            Self::Mesh(_) => 2,
            Self::Global => 3,
        }
    }

    /// Return true when `self` is broader than `other`.
    #[must_use]
    pub const fn is_broader_than(&self, other: &Self) -> bool {
        self.rank() > other.rank()
    }
}

/// A digital pheromone view over a durable coordination record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pheromone {
    /// The semantic kind of the pheromone.
    pub kind: PheromoneKind,
    /// The current intensity in the `[0.0, 1.0]` range.
    pub intensity: f64,
    /// The base half-life before confirmation extension.
    pub half_life: Duration,
    /// The agent that deposited the pheromone.
    pub source: AgentId,
    /// The propagation scope of the pheromone.
    pub scope: PheromoneScope,
    /// The wall-clock time when the pheromone was deposited.
    pub deposited_at: SystemTime,
    /// Confirmation count used to extend the effective half-life.
    pub confirmations: u32,
}

impl Pheromone {
    /// Construct a fresh pheromone with zero confirmations.
    #[must_use]
    pub fn new(
        kind: PheromoneKind,
        intensity: f64,
        half_life: Duration,
        source: AgentId,
        scope: PheromoneScope,
    ) -> Self {
        Self {
            kind,
            intensity,
            half_life,
            source,
            scope,
            deposited_at: SystemTime::now(),
            confirmations: 0,
        }
    }

    /// Return the effective half-life after confirmation extension.
    #[must_use]
    pub fn effective_half_life(&self) -> Duration {
        self.half_life
            .mul_f64(f64::from(self.confirmations).mul_add(0.5, 1.0))
    }

    /// Compute the current intensity using the current wall clock.
    #[must_use]
    pub fn current_intensity(&self) -> f64 {
        self.current_intensity_at(SystemTime::now())
    }

    /// Compute the current intensity at an explicit wall-clock time.
    #[must_use]
    pub fn current_intensity_at(&self, now: SystemTime) -> f64 {
        let elapsed = now.duration_since(self.deposited_at).unwrap_or_default();
        current_intensity(self.intensity, self.effective_half_life(), elapsed)
    }

    /// Return true when the pheromone has fallen below `threshold`.
    #[must_use]
    pub fn is_evaporated(&self, threshold: f64) -> bool {
        self.current_intensity() < threshold
    }
}

/// Compute the current intensity of a pheromone given its age.
#[must_use]
pub fn current_intensity(initial_intensity: f64, half_life: Duration, elapsed: Duration) -> f64 {
    if half_life.is_zero() {
        return 0.0;
    }
    let exponent = -(elapsed.as_secs_f64() / half_life.as_secs_f64());
    initial_intensity * exponent.exp2()
}

/// Compute pheromone decay from the deposition time and confirmation count.
#[must_use]
pub fn pheromone_decay(
    base_intensity: f64,
    deposited_at: SystemTime,
    half_life: Duration,
    confirmations: u32,
) -> f64 {
    let effective_half_life = half_life.mul_f64(f64::from(confirmations).mul_add(0.5, 1.0));
    let elapsed = SystemTime::now()
        .duration_since(deposited_at)
        .unwrap_or_default();
    current_intensity(base_intensity, effective_half_life, elapsed)
}

/// Compute the effective confirmation count, weighted by confirmer reputation.
#[must_use]
pub fn effective_confirmations(confirmations: &[(AgentId, f64)]) -> f64 {
    confirmations.iter().map(|(_, rep)| clamp_unit(*rep)).sum()
}

/// Promotion thresholds for the Pattern -> Wisdom -> Consensus cascade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionConfig {
    /// Minimum confirmations for Pattern -> Wisdom promotion.
    pub pattern_to_wisdom_confirmations: u32,
    /// Minimum age as a fraction of half-life for Pattern -> Wisdom promotion.
    pub pattern_to_wisdom_min_age_fraction: f64,
    /// Minimum confirmations for Wisdom -> Consensus promotion.
    pub wisdom_to_consensus_confirmations: u32,
    /// Minimum confirmations for Consensus -> permanent Engram promotion.
    pub consensus_to_engram_confirmations: u32,
    /// Whether the caller should auto-promote or defer to explicit action.
    pub auto_promote: bool,
}

impl Default for PromotionConfig {
    fn default() -> Self {
        Self {
            pattern_to_wisdom_confirmations: 3,
            pattern_to_wisdom_min_age_fraction: 0.5,
            wisdom_to_consensus_confirmations: 4,
            consensus_to_engram_confirmations: 5,
            auto_promote: true,
        }
    }
}

/// Evaluate whether a pheromone is eligible for promotion.
#[must_use]
pub fn check_promotion(
    kind: &PheromoneKind,
    confirmations: u32,
    age: Duration,
    half_life: Duration,
    config: &PromotionConfig,
) -> Option<PheromoneKind> {
    if half_life.is_zero() {
        return None;
    }

    let age_fraction = safe_half_life_fraction(age, half_life);
    match kind {
        PheromoneKind::Pattern
            if confirmations >= config.pattern_to_wisdom_confirmations
                && age_fraction >= config.pattern_to_wisdom_min_age_fraction =>
        {
            Some(PheromoneKind::Wisdom)
        }
        PheromoneKind::Wisdom if confirmations >= config.wisdom_to_consensus_confirmations => {
            Some(PheromoneKind::Consensus)
        }
        _ => None,
    }
}

/// Per-agent response thresholds for pheromone-driven task allocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseThresholds {
    /// Per-kind response thresholds. Lower means more responsive.
    pub thresholds: HashMap<PheromoneKind, f64>,
    /// Hill coefficient controlling response curve steepness.
    pub hill_coefficient: u32,
    /// Learning rate for threshold adaptation.
    pub learning_rate: f64,
    /// Minimum threshold allowed after reinforcement.
    pub min_threshold: f64,
    /// Maximum threshold allowed after habituation.
    pub max_threshold: f64,
}

impl Default for ResponseThresholds {
    fn default() -> Self {
        Self {
            thresholds: HashMap::new(),
            hill_coefficient: 2,
            learning_rate: 0.05,
            min_threshold: 0.05,
            max_threshold: 0.95,
        }
    }
}

impl ResponseThresholds {
    /// Compute the probability of responding to a pheromone of a given kind and intensity.
    #[must_use]
    pub fn response_probability(&self, kind: &PheromoneKind, intensity: f64) -> f64 {
        let theta = clamp_unit(self.thresholds.get(kind).copied().unwrap_or(0.5));
        let n = f64::from(self.hill_coefficient.max(1));
        let i_n = clamp_unit(intensity).powf(n);
        let theta_n = theta.powf(n);
        let denom = i_n + theta_n;
        if denom == 0.0 { 0.0 } else { i_n / denom }
    }

    /// Lower the threshold after a successful response.
    pub fn reinforce(&mut self, kind: &PheromoneKind) {
        if let Some(theta) = self.thresholds.get_mut(kind) {
            *theta = (*theta - self.learning_rate).max(self.min_threshold);
        }
    }

    /// Raise the threshold after ignoring a signal.
    pub fn habituate(&mut self, kind: &PheromoneKind) {
        if let Some(theta) = self.thresholds.get_mut(kind) {
            *theta = self
                .learning_rate
                .mul_add(0.5, *theta)
                .min(self.max_threshold);
        }
    }
}

/// Number of strategy dimensions used by the morphogenetic model.
pub const STRATEGY_DIMS: usize = 8;

/// Morphogenetic state for an agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MorphogeneticState {
    /// Strategy concentration vector in `[0, 1]`, normalized to sum to 1.
    pub strategy: [f64; STRATEGY_DIMS],
    /// Per-dimension returns attributed since the last update.
    pub attributed_returns: [f64; STRATEGY_DIMS],
    /// Aggregated strategy vectors received from the collective.
    pub collective_pheromone: [f64; STRATEGY_DIMS],
    /// Number of agents in the collective.
    pub collective_size: usize,
}

impl Default for MorphogeneticState {
    fn default() -> Self {
        let baseline = 1.0 / STRATEGY_DIMS_F64;
        Self {
            strategy: [baseline; STRATEGY_DIMS],
            attributed_returns: [0.0; STRATEGY_DIMS],
            collective_pheromone: [0.0; STRATEGY_DIMS],
            collective_size: 1,
        }
    }
}

impl MorphogeneticState {
    /// Return the specialization index for the current strategy vector.
    #[must_use]
    pub fn specialization_index(&self) -> SpecializationIndex {
        specialization_index(&self.strategy)
    }
}

/// Morphogenetic parameters controlling reaction-diffusion dynamics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MorphogeneticParams {
    /// Activation rate for profitable strategies.
    pub alpha: f64,
    /// Inhibition rate for collective overlap.
    pub beta: f64,
    /// Decay rate toward baseline.
    pub mu: f64,
    /// Baseline concentration per dimension.
    pub baseline: f64,
    /// Noise standard deviation for symmetry breaking.
    pub sigma_noise: f64,
    /// Resource pressure scalar modulating activation.
    pub resource_pressure_scalar: f64,
}

impl Default for MorphogeneticParams {
    fn default() -> Self {
        Self {
            alpha: 0.05,
            beta: 0.15,
            mu: 0.01,
            baseline: 1.0 / STRATEGY_DIMS_F64,
            sigma_noise: 0.005,
            resource_pressure_scalar: 1.0,
        }
    }
}

/// Compute the specialization index of a strategy vector.
#[must_use]
pub fn specialization_index(strategy: &[f64; STRATEGY_DIMS]) -> SpecializationIndex {
    let h: f64 = strategy
        .iter()
        .copied()
        .filter(|&s| s > 1e-10)
        .map(|s| -s * s.ln())
        .sum();
    let h_max = STRATEGY_DIMS_F64.ln();
    if h_max == 0.0 { 0.0 } else { 1.0 - h / h_max }
}

/// The five axes used to measure collective intelligence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CohortMetrics {
    /// Normalized Shannon entropy of turn-taking.
    pub turn_taking_entropy: f64,
    /// Fraction of peer predictions that landed within tolerance.
    pub peer_prediction_accuracy: f64,
    /// How often citations are reciprocal and validated.
    pub citation_reciprocity: f64,
    /// Proportion of intended Bus traffic that was delivered.
    pub delivery_rate: f64,
    /// HDC diversity across cohort artifacts.
    pub hdc_diversity: f64,
}

/// Linear weights for the c-factor model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CohortWeights {
    /// Weight on turn-taking entropy.
    pub turn_taking_entropy: f64,
    /// Weight on peer prediction accuracy.
    pub peer_prediction_accuracy: f64,
    /// Weight on citation reciprocity.
    pub citation_reciprocity: f64,
    /// Weight on delivery rate.
    pub delivery_rate: f64,
    /// Weight on HDC diversity.
    pub hdc_diversity: f64,
    /// Learned bias term.
    pub bias: f64,
}

/// Compute the c-factor for a cohort.
#[must_use]
pub fn c_factor(metrics: &CohortMetrics, weights: &CohortWeights) -> f64 {
    let weighted_sum = weights.turn_taking_entropy.mul_add(
        metrics.turn_taking_entropy,
        weights.peer_prediction_accuracy.mul_add(
            metrics.peer_prediction_accuracy,
            weights.citation_reciprocity.mul_add(
                metrics.citation_reciprocity,
                weights.delivery_rate.mul_add(
                    metrics.delivery_rate,
                    weights.hdc_diversity * metrics.hdc_diversity,
                ),
            ),
        ),
    );
    weighted_sum + weights.bias
}

/// `WisdomGate` inputs for consensus aggregation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WisdomGate {
    /// Minimum turn-taking entropy.
    pub min_turn_taking_entropy: f64,
    /// Minimum peer prediction accuracy.
    pub min_peer_prediction_accuracy: f64,
    /// Minimum citation reciprocity.
    pub min_citation_reciprocity: f64,
    /// Minimum HDC diversity.
    pub min_hdc_diversity: f64,
    /// Maximum lineage overlap.
    pub max_lineage_overlap: f64,
    /// Maximum sender share.
    pub max_sender_share: f64,
}

impl WisdomGate {
    /// Return true when the cohort is broad enough for consensus aggregation.
    #[must_use]
    pub fn allows(&self, metrics: &CohortMetrics, lineage_overlap: f64, sender_share: f64) -> bool {
        metrics.turn_taking_entropy >= self.min_turn_taking_entropy
            && metrics.peer_prediction_accuracy >= self.min_peer_prediction_accuracy
            && metrics.citation_reciprocity >= self.min_citation_reciprocity
            && metrics.hdc_diversity >= self.min_hdc_diversity
            && lineage_overlap <= self.max_lineage_overlap
            && sender_share <= self.max_sender_share
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_custom_kind_names() {
        assert!(validate_custom_kind("code_coverage_gap").is_ok());
        assert!(validate_custom_kind("Threat").is_err());
        assert!(validate_custom_kind("_internal").is_err());
        assert!(validate_custom_kind("bad-kind").is_err());
    }

    #[test]
    fn validates_subnet_names_and_renders_ids() {
        let subnet = SubnetId::new("collective-42".to_owned(), "engineering").unwrap();
        assert_eq!(subnet.to_string(), "collective-42:engineering");
        assert!(validate_subnet_name("research-team").is_ok());
        assert!(validate_subnet_name("bad team").is_err());
    }

    #[test]
    fn decays_by_half_life_and_confirmations() {
        let start = SystemTime::UNIX_EPOCH;
        let half_life = Duration::from_secs(2 * 60 * 60);
        let intensity = pheromone_decay(1.0, start, half_life, 0);
        let two_hours = Duration::from_secs(2 * 60 * 60);
        let exact = current_intensity(1.0, half_life, two_hours);
        assert!((exact - 0.5).abs() < 1e-12);
        assert!(intensity < 1.0);

        let boosted = current_intensity(1.0, half_life.mul_f64(2.0), two_hours);
        assert!(boosted > exact);
    }

    #[test]
    fn promotes_pattern_then_wisdom() {
        let config = PromotionConfig::default();
        let age = Duration::from_secs(3 * 60 * 60);
        let half_life = Duration::from_secs(4 * 60 * 60);
        let next = check_promotion(&PheromoneKind::Pattern, 3, age, half_life, &config);
        assert_eq!(next, Some(PheromoneKind::Wisdom));

        let wisdom = check_promotion(
            &PheromoneKind::Wisdom,
            4,
            Duration::from_secs(1),
            half_life,
            &config,
        );
        assert_eq!(wisdom, Some(PheromoneKind::Consensus));
    }

    #[test]
    fn response_thresholds_follow_the_hill_curve() {
        let mut thresholds = ResponseThresholds {
            thresholds: HashMap::from([(PheromoneKind::Threat, 0.5)]),
            ..Default::default()
        };

        let strong = thresholds.response_probability(&PheromoneKind::Threat, 0.9);
        let weak = thresholds.response_probability(&PheromoneKind::Threat, 0.1);
        assert!(strong > weak);

        thresholds.reinforce(&PheromoneKind::Threat);
        assert!(thresholds.thresholds[&PheromoneKind::Threat] < 0.5);
        thresholds.habituate(&PheromoneKind::Threat);
        assert!(thresholds.thresholds[&PheromoneKind::Threat] <= 0.5);
    }

    #[test]
    fn specialization_and_c_factor_are_linear() {
        let mut strategy = [0.0; STRATEGY_DIMS];
        strategy[0] = 1.0;
        assert!((specialization_index(&strategy) - 1.0).abs() < 1e-12);

        let metrics = CohortMetrics {
            turn_taking_entropy: 0.5,
            peer_prediction_accuracy: 0.25,
            citation_reciprocity: 0.25,
            delivery_rate: 0.5,
            hdc_diversity: 0.75,
        };
        let weights = CohortWeights {
            turn_taking_entropy: 1.0,
            peer_prediction_accuracy: 2.0,
            citation_reciprocity: 3.0,
            delivery_rate: 4.0,
            hdc_diversity: 5.0,
            bias: 0.5,
        };
        let score = c_factor(&metrics, &weights);
        assert!((score - 8.0).abs() < 1e-12);
    }

    #[test]
    fn wisdom_gate_enforces_breadth_thresholds() {
        let gate = WisdomGate {
            min_turn_taking_entropy: 0.4,
            min_peer_prediction_accuracy: 0.4,
            min_citation_reciprocity: 0.3,
            min_hdc_diversity: 0.5,
            max_lineage_overlap: 0.2,
            max_sender_share: 0.6,
        };
        let metrics = CohortMetrics {
            turn_taking_entropy: 0.5,
            peer_prediction_accuracy: 0.6,
            citation_reciprocity: 0.4,
            delivery_rate: 0.9,
            hdc_diversity: 0.8,
        };
        assert!(gate.allows(&metrics, 0.1, 0.3));
        assert!(!gate.allows(&metrics, 0.3, 0.3));
    }

    #[test]
    fn scope_hierarchy_treats_subnet_as_mid_tier() {
        let local = PheromoneScope::Local("store-a".to_owned());
        let subnet =
            PheromoneScope::Subnet(SubnetId::new("collective-a".to_owned(), "eng").unwrap());
        let mesh = PheromoneScope::Mesh("collective-a".to_owned());
        let global = PheromoneScope::Global;

        assert!(subnet.is_broader_than(&local));
        assert!(mesh.is_broader_than(&subnet));
        assert!(global.is_broader_than(&mesh));
        assert!(!local.is_broader_than(&subnet));
    }
}
