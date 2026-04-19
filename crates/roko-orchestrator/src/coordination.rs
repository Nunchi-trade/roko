//! Coordination primitives for pheromones, subnets, morphogenesis, and
//! cohort intelligence.

use std::collections::{HashMap, HashSet};
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

    /// Return true when the kind is Alpha (first-mover advantage).
    ///
    /// Alpha pheromones have inverted confirmation dynamics: consensus
    /// shortens their half-life to prevent lock-in.
    #[must_use]
    pub const fn is_alpha(&self) -> bool {
        matches!(self, Self::Alpha)
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
    ///
    /// For most pheromone kinds, confirmations extend the half-life (more
    /// agreement = longer persistence). Alpha pheromones invert this:
    /// confirmations shorten the half-life to prevent consensus lock-in.
    #[must_use]
    pub fn effective_half_life(&self) -> Duration {
        if self.kind.is_alpha() {
            // Alpha paradox: consensus makes alpha expire faster.
            let divisor = f64::from(self.confirmations).mul_add(0.1, 1.0);
            self.half_life.mul_f64(1.0 / divisor)
        } else {
            self.half_life
                .mul_f64(f64::from(self.confirmations).mul_add(0.5, 1.0))
        }
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

    /// Update morphogenetic field using Gierer-Meinhardt reaction-diffusion dynamics.
    ///
    /// Per dimension `i`:
    ///   `ds_i = alpha * returns_i * pressure - beta * collective_i / size - mu * (s_i - baseline) + N(0, sigma)`
    ///
    /// After computing deltas, the strategy vector is re-normalized to sum to 1.0
    /// and accumulators are reset.
    pub fn update(&mut self, params: &MorphogeneticParams) {
        let pressure = params.resource_pressure_scalar;
        let size = (self.collective_size as f64).max(1.0);

        for i in 0..STRATEGY_DIMS {
            let activation = params.alpha * self.attributed_returns[i] * pressure;
            let inhibition = params.beta * self.collective_pheromone[i] / size;
            let decay = params.mu * (self.strategy[i] - params.baseline);
            let noise = box_muller_normal() * params.sigma_noise;
            self.strategy[i] += activation - inhibition - decay + noise;
            // Prevent negative concentrations.
            if self.strategy[i] < 0.001 {
                self.strategy[i] = 0.001;
            }
        }

        // Re-normalize to sum to 1.0.
        let sum: f64 = self.strategy.iter().sum();
        if sum > 0.0 {
            for s in &mut self.strategy {
                *s /= sum;
            }
        }

        // Reset accumulators for next cycle.
        self.attributed_returns = [0.0; STRATEGY_DIMS];
        self.collective_pheromone = [0.0; STRATEGY_DIMS];
    }

    /// Record a return observation for a specific strategy dimension.
    pub fn attribute_return(&mut self, dimension: usize, value: f64) {
        if dimension < STRATEGY_DIMS {
            self.attributed_returns[dimension] += value;
        }
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

/// Hill function for sigmoid response thresholds.
///
/// `H(x) = x^n / (k^n + x^n)`
///
/// - `concentration`: the input signal level
/// - `k`: half-maximal concentration (threshold)
/// - `n`: Hill coefficient (steepness, typically 1-4)
#[must_use]
pub fn hill_response(concentration: f64, k: f64, n: f64) -> f64 {
    if k <= 0.0 || n <= 0.0 {
        return 0.0;
    }
    let x_n = concentration.abs().powf(n);
    let k_n = k.powf(n);
    let denom = k_n + x_n;
    if denom == 0.0 { 0.0 } else { x_n / denom }
}

/// Generate a single standard-normal sample using the Box-Muller transform.
fn box_muller_normal() -> f64 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let u1: f64 = rng.r#gen::<f64>().max(1e-15); // avoid ln(0)
    let u2: f64 = rng.r#gen::<f64>();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
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

// ---------------------------------------------------------------------------
// COORD-05: Scope promotion gates
// ---------------------------------------------------------------------------

/// Gate controlling pheromone scope promotion (e.g., Pattern -> Wisdom).
///
/// Prevents gaming by requiring confirmations from a minimum number of
/// distinct agents and optionally requiring that a gate pass accompanies
/// each confirmation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionGate {
    /// Minimum total confirmations required for promotion.
    pub min_confirmations: u32,
    /// Minimum number of distinct agents among the confirmers.
    pub min_agents: u32,
    /// Whether each confirmation must be accompanied by a passing gate result.
    pub require_gate_pass: bool,
}

impl Default for PromotionGate {
    fn default() -> Self {
        Self {
            min_confirmations: 3,
            min_agents: 2,
            require_gate_pass: false,
        }
    }
}

/// A single confirmation record for scope promotion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirmation {
    /// The agent that confirmed.
    pub agent: AgentId,
    /// Whether a gate pass accompanied the confirmation.
    pub gate_passed: bool,
}

impl PromotionGate {
    /// Check whether the given confirmations satisfy the promotion gate.
    ///
    /// Returns `true` when there are enough confirmations from enough
    /// distinct agents (and all gate pass requirements are met).
    #[must_use]
    pub fn is_satisfied(&self, confirmations: &[Confirmation]) -> bool {
        let valid: Vec<_> = if self.require_gate_pass {
            confirmations.iter().filter(|c| c.gate_passed).collect()
        } else {
            confirmations.iter().collect()
        };

        if (valid.len() as u32) < self.min_confirmations {
            return false;
        }

        let distinct_agents: HashSet<&AgentId> = valid.iter().map(|c| &c.agent).collect();
        distinct_agents.len() as u32 >= self.min_agents
    }
}

// ---------------------------------------------------------------------------
// COORD-06: Niche competition
// ---------------------------------------------------------------------------

/// Outcome of competition between two pheromones in the same scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompetitionOutcome {
    /// Pheromone `a` wins and suppresses `b`.
    Winner,
    /// Both pheromones coexist without interference.
    Coexist,
    /// The two pheromones should be merged into one.
    Merge,
}

/// Compete two pheromones in the same scope with overlapping concerns.
///
/// The stronger pheromone (higher current intensity) suppresses the weaker
/// one when their intensity ratio exceeds `niche_overlap_threshold`.
/// When intensities are very close, the outcome is `Merge`.
#[must_use]
pub fn compete(a: &Pheromone, b: &Pheromone, niche_overlap_threshold: f64) -> CompetitionOutcome {
    let ia = a.current_intensity();
    let ib = b.current_intensity();

    // Avoid division by zero.
    let max_intensity = ia.max(ib);
    if max_intensity < 1e-12 {
        return CompetitionOutcome::Coexist;
    }

    let ratio = (ia - ib).abs() / max_intensity;

    if ratio < 0.1 {
        // Intensities are within 10% — merge.
        CompetitionOutcome::Merge
    } else if ratio >= niche_overlap_threshold {
        // Clear winner by the stronger signal.
        CompetitionOutcome::Winner
    } else {
        CompetitionOutcome::Coexist
    }
}

// ---------------------------------------------------------------------------
// COORD-08: Permissioned subnet enforcement
// ---------------------------------------------------------------------------

/// Access control configuration for subnet-scoped pheromones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubnetPermissions {
    /// The set of agents allowed to publish into this subnet.
    pub allowed_agents: HashSet<String>,
    /// Whether publications must include an attestation payload.
    pub require_attestation: bool,
}

impl SubnetPermissions {
    /// Check whether `agent` is authorized to publish into this subnet.
    #[must_use]
    pub fn is_authorized(&self, agent: &str) -> bool {
        self.allowed_agents.contains(agent)
    }

    /// Check whether a publication from `agent` with the given attestation
    /// status should be accepted.
    #[must_use]
    pub fn accept_publication(&self, agent: &str, has_attestation: bool) -> bool {
        if !self.is_authorized(agent) {
            return false;
        }
        if self.require_attestation && !has_attestation {
            return false;
        }
        true
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

    #[test]
    fn morphogenetic_update_diverges_strategies() {
        let params = MorphogeneticParams {
            sigma_noise: 0.0, // deterministic for testing
            ..Default::default()
        };

        let mut state = MorphogeneticState::default();

        // Attribute strong returns to dimension 0 (depth).
        state.attribute_return(0, 1.0);
        // Collective signal on dimension 2 (execution).
        state.collective_pheromone[2] = 0.5;
        state.collective_size = 3;

        let initial_spec = state.specialization_index();
        state.update(&params);

        // Strategy should have diverged — specialization increases.
        let updated_spec = state.specialization_index();
        assert!(
            updated_spec > initial_spec,
            "specialization should increase: {updated_spec} > {initial_spec}"
        );

        // Strategy still sums to 1.0.
        let sum: f64 = state.strategy.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-10,
            "strategy should be normalized: sum={sum}"
        );

        // Accumulators should be reset.
        assert_eq!(state.attributed_returns, [0.0; STRATEGY_DIMS]);
        assert_eq!(state.collective_pheromone, [0.0; STRATEGY_DIMS]);
    }

    #[test]
    fn morphogenetic_update_preserves_positive_concentrations() {
        let params = MorphogeneticParams {
            beta: 10.0,        // very strong inhibition
            sigma_noise: 0.0,
            ..Default::default()
        };

        let mut state = MorphogeneticState::default();
        state.collective_pheromone = [1.0; STRATEGY_DIMS];
        state.collective_size = 1;

        state.update(&params);

        // All concentrations must remain positive.
        for (i, &s) in state.strategy.iter().enumerate() {
            assert!(s > 0.0, "dimension {i} should be positive: {s}");
        }
    }

    #[test]
    fn attribute_return_accumulates_per_dimension() {
        let mut state = MorphogeneticState::default();
        state.attribute_return(3, 0.5);
        state.attribute_return(3, 0.3);
        state.attribute_return(7, 1.0);
        assert!((state.attributed_returns[3] - 0.8).abs() < 1e-10);
        assert!((state.attributed_returns[7] - 1.0).abs() < 1e-10);

        // Out-of-bounds dimension is silently ignored.
        state.attribute_return(99, 1.0);
    }

    #[test]
    fn hill_response_is_correct_sigmoid() {
        // At concentration == k, Hill function should return 0.5.
        let h = hill_response(0.5, 0.5, 2.0);
        assert!((h - 0.5).abs() < 1e-10, "H(k, k, n) = 0.5, got {h}");

        // At zero concentration, response is 0.
        assert_eq!(hill_response(0.0, 0.5, 2.0), 0.0);

        // At very high concentration, response approaches 1.
        let h_high = hill_response(100.0, 0.5, 2.0);
        assert!(h_high > 0.999, "H(100, 0.5, 2) should be ~1.0, got {h_high}");

        // Higher n -> steeper curve.
        let h_n1 = hill_response(0.3, 0.5, 1.0);
        let h_n4 = hill_response(0.3, 0.5, 4.0);
        assert!(h_n1 > h_n4, "lower n should give higher response at sub-threshold: {h_n1} > {h_n4}");

        // Edge cases.
        assert_eq!(hill_response(1.0, 0.0, 2.0), 0.0); // k=0
        assert_eq!(hill_response(1.0, 1.0, 0.0), 0.0);  // n=0
    }

    // ----- COORD-07: Alpha pheromone paradox fix -----

    #[test]
    fn alpha_pheromone_half_life_shortens_with_confirmations() {
        let mut alpha = Pheromone::new(
            PheromoneKind::Alpha,
            1.0,
            Duration::from_secs(3600),
            "agent-a".to_owned(),
            PheromoneScope::Global,
        );

        let base = alpha.effective_half_life();
        assert_eq!(base, Duration::from_secs(3600));

        alpha.confirmations = 5;
        let confirmed = alpha.effective_half_life();
        // 3600 / (1.0 + 5 * 0.1) = 3600 / 1.5 = 2400
        assert!(
            confirmed < base,
            "alpha half-life should shorten: {confirmed:?} < {base:?}"
        );
        let expected = Duration::from_secs(3600).mul_f64(1.0 / 1.5);
        assert!(
            (confirmed.as_secs_f64() - expected.as_secs_f64()).abs() < 1e-6,
            "expected {expected:?}, got {confirmed:?}"
        );
    }

    #[test]
    fn non_alpha_half_life_extends_with_confirmations() {
        let mut wisdom = Pheromone::new(
            PheromoneKind::Wisdom,
            1.0,
            Duration::from_secs(3600),
            "agent-b".to_owned(),
            PheromoneScope::Global,
        );

        let base = wisdom.effective_half_life();
        wisdom.confirmations = 4;
        let confirmed = wisdom.effective_half_life();
        assert!(
            confirmed > base,
            "non-alpha half-life should extend: {confirmed:?} > {base:?}"
        );
    }

    // ----- COORD-05: Scope promotion gates -----

    #[test]
    fn promotion_gate_requires_diverse_agents() {
        let gate = PromotionGate {
            min_confirmations: 3,
            min_agents: 2,
            require_gate_pass: false,
        };

        // 3 confirmations from 1 agent — fails min_agents.
        let same_agent = vec![
            Confirmation { agent: "a1".into(), gate_passed: true },
            Confirmation { agent: "a1".into(), gate_passed: true },
            Confirmation { agent: "a1".into(), gate_passed: true },
        ];
        assert!(!gate.is_satisfied(&same_agent));

        // 3 confirmations from 2 agents — passes.
        let diverse = vec![
            Confirmation { agent: "a1".into(), gate_passed: true },
            Confirmation { agent: "a2".into(), gate_passed: true },
            Confirmation { agent: "a1".into(), gate_passed: false },
        ];
        assert!(gate.is_satisfied(&diverse));
    }

    #[test]
    fn promotion_gate_enforces_gate_pass() {
        let gate = PromotionGate {
            min_confirmations: 2,
            min_agents: 2,
            require_gate_pass: true,
        };

        let no_pass = vec![
            Confirmation { agent: "a1".into(), gate_passed: false },
            Confirmation { agent: "a2".into(), gate_passed: false },
        ];
        assert!(!gate.is_satisfied(&no_pass));

        let with_pass = vec![
            Confirmation { agent: "a1".into(), gate_passed: true },
            Confirmation { agent: "a2".into(), gate_passed: true },
        ];
        assert!(gate.is_satisfied(&with_pass));
    }

    // ----- COORD-06: Niche competition -----

    #[test]
    fn niche_competition_winner_when_large_gap() {
        let a = Pheromone::new(
            PheromoneKind::Opportunity,
            0.9,
            Duration::from_secs(7200),
            "agent-a".to_owned(),
            PheromoneScope::Global,
        );
        let b = Pheromone::new(
            PheromoneKind::Opportunity,
            0.2,
            Duration::from_secs(7200),
            "agent-b".to_owned(),
            PheromoneScope::Global,
        );

        let outcome = compete(&a, &b, 0.5);
        assert_eq!(outcome, CompetitionOutcome::Winner);
    }

    #[test]
    fn niche_competition_merge_when_close() {
        let a = Pheromone::new(
            PheromoneKind::Pattern,
            0.80,
            Duration::from_secs(7200),
            "agent-a".to_owned(),
            PheromoneScope::Global,
        );
        let b = Pheromone::new(
            PheromoneKind::Pattern,
            0.79,
            Duration::from_secs(7200),
            "agent-b".to_owned(),
            PheromoneScope::Global,
        );

        let outcome = compete(&a, &b, 0.5);
        assert_eq!(outcome, CompetitionOutcome::Merge);
    }

    #[test]
    fn niche_competition_coexist_in_middle_range() {
        let a = Pheromone::new(
            PheromoneKind::Threat,
            0.7,
            Duration::from_secs(7200),
            "agent-a".to_owned(),
            PheromoneScope::Global,
        );
        let b = Pheromone::new(
            PheromoneKind::Threat,
            0.5,
            Duration::from_secs(7200),
            "agent-b".to_owned(),
            PheromoneScope::Global,
        );

        let outcome = compete(&a, &b, 0.5);
        assert_eq!(outcome, CompetitionOutcome::Coexist);
    }

    // ----- COORD-08: Subnet permissions -----

    #[test]
    fn subnet_permissions_allow_authorized_agents() {
        let perms = SubnetPermissions {
            allowed_agents: HashSet::from(["agent-1".to_string(), "agent-2".to_string()]),
            require_attestation: false,
        };
        assert!(perms.accept_publication("agent-1", false));
        assert!(!perms.accept_publication("agent-3", false));
    }

    #[test]
    fn subnet_permissions_require_attestation() {
        let perms = SubnetPermissions {
            allowed_agents: HashSet::from(["agent-1".to_string()]),
            require_attestation: true,
        };
        assert!(!perms.accept_publication("agent-1", false));
        assert!(perms.accept_publication("agent-1", true));
    }
}
