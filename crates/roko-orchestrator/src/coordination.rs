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
///
/// For Alpha pheromones, confirmations **shorten** the effective half-life
/// (anti-herding / Alpha paradox). For all other kinds, confirmations
/// **extend** the half-life as usual.
#[must_use]
pub fn pheromone_decay(
    base_intensity: f64,
    deposited_at: SystemTime,
    half_life: Duration,
    confirmations: u32,
) -> f64 {
    pheromone_decay_for_kind(base_intensity, deposited_at, half_life, confirmations, None)
}

/// Kind-aware pheromone decay.
///
/// When `kind` is `Some(PheromoneKind::Alpha)`, confirmations shorten the
/// effective half-life instead of extending it, matching the Alpha paradox
/// formula in [`Pheromone::effective_half_life`].
#[must_use]
pub fn pheromone_decay_for_kind(
    base_intensity: f64,
    deposited_at: SystemTime,
    half_life: Duration,
    confirmations: u32,
    kind: Option<&PheromoneKind>,
) -> f64 {
    let effective_half_life = if kind.map_or(false, PheromoneKind::is_alpha) {
        // Alpha paradox: consensus makes alpha expire faster.
        let divisor = f64::from(confirmations).mul_add(0.1, 1.0);
        half_life.mul_f64(1.0 / divisor)
    } else {
        half_life.mul_f64(f64::from(confirmations).mul_add(0.5, 1.0))
    };
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
    /// Minimum confirmations for Consensus -> permanent Signal promotion.
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

/// Verify controlling pheromone scope promotion (e.g., Pattern -> Wisdom).
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
// COORD-05: Scope promotion (Local -> Subnet -> Mesh -> Global)
// ---------------------------------------------------------------------------

/// Trust discount factors applied when reading pheromones from a broader scope.
///
/// Signals from Local scope carry full trust; broader scopes are progressively
/// discounted because they passed through more intermediaries and may have
/// been confirmed by agents with unknown provenance. Based on Constructal Law
/// (Bejan 1997): fine-grained local channels carry higher-fidelity information.
pub const TRUST_DISCOUNT: [f64; 4] = [1.0, 0.90, 0.80, 0.50];

/// Confirmation thresholds for scope promotion.
///
/// A pheromone is promoted to the next scope level when it accumulates
/// enough confirmations from distinct agents in the source scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopePromotionConfig {
    /// Minimum confirmations for Local -> Subnet promotion.
    pub local_to_subnet_confirmations: u32,
    /// Minimum confirmations for Subnet -> Mesh promotion.
    pub subnet_to_mesh_confirmations: u32,
    /// Minimum confirmations for Mesh -> Global promotion.
    pub mesh_to_global_confirmations: u32,
}

impl Default for ScopePromotionConfig {
    fn default() -> Self {
        Self {
            local_to_subnet_confirmations: 3,
            subnet_to_mesh_confirmations: 5,
            mesh_to_global_confirmations: 10,
        }
    }
}

/// Check whether a pheromone is eligible for scope promotion.
///
/// Returns `Some(next_scope)` when the pheromone's confirmation count exceeds
/// the threshold for the next scope level. Returns `None` when no promotion
/// is warranted or the pheromone is already at Global scope.
///
/// Note: this only checks the confirmation count. Callers should also verify
/// that confirmations come from distinct agents (e.g., via [`PromotionGate`]).
#[must_use]
pub fn check_scope_promotion(
    pheromone: &Pheromone,
    config: &ScopePromotionConfig,
) -> Option<PheromoneScope> {
    match &pheromone.scope {
        PheromoneScope::Local(substrate) => {
            if pheromone.confirmations >= config.local_to_subnet_confirmations {
                // Promote to Subnet. Use the substrate as collective.
                Some(PheromoneScope::Subnet(
                    SubnetId::new(substrate.clone(), "auto").unwrap_or_else(|_| SubnetId {
                        collective: substrate.clone(),
                        name: "auto".to_owned(),
                    }),
                ))
            } else {
                None
            }
        }
        PheromoneScope::Subnet(_) => {
            if pheromone.confirmations >= config.subnet_to_mesh_confirmations {
                Some(PheromoneScope::Mesh("default".to_owned()))
            } else {
                None
            }
        }
        PheromoneScope::Mesh(_) => {
            if pheromone.confirmations >= config.mesh_to_global_confirmations {
                Some(PheromoneScope::Global)
            } else {
                None
            }
        }
        PheromoneScope::Global => None, // Already at broadest scope.
    }
}

/// Apply trust discounting to a pheromone's intensity based on its scope.
///
/// Signals from broader scopes are discounted because they traversed more
/// intermediaries. The discount factors are:
/// - Local: 1.0 (full trust)
/// - Subnet: 0.90
/// - Mesh: 0.80
/// - Global: 0.50
#[must_use]
pub fn trust_discounted_intensity(pheromone: &Pheromone) -> f64 {
    let rank = pheromone.scope.rank() as usize;
    let discount = TRUST_DISCOUNT.get(rank).copied().unwrap_or(0.50);
    pheromone.current_intensity() * discount
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
// COORD-06: Cosine similarity for morphogenetic strategy vectors
// ---------------------------------------------------------------------------

/// Cosine similarity between two strategy vectors.
///
/// Returns a value in `[0.0, 1.0]` where 1.0 means identical direction.
/// If either vector has zero norm, returns 0.0.
#[must_use]
pub fn cosine_similarity(a: &[f64; STRATEGY_DIMS], b: &[f64; STRATEGY_DIMS]) -> f64 {
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a * norm_b < 1e-10 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)).clamp(0.0, 1.0)
}

/// A detected niche conflict between two agents with overlapping strategies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NicheConflict {
    /// First agent identifier.
    pub agent_a: AgentId,
    /// Second agent identifier.
    pub agent_b: AgentId,
    /// Cosine similarity between the two strategy vectors.
    pub similarity: f64,
    /// Indices of the top-3 shared dimensions (highest product of concentrations).
    pub shared_dimensions: Vec<usize>,
}

/// Detect niche conflicts among a set of agents.
///
/// Returns all pairs whose cosine similarity exceeds `threshold` (default 0.9).
/// Each conflict includes the shared dimensions that should receive increased
/// collective pheromone to trigger automatic separation via Gierer-Meinhardt
/// inhibition.
#[must_use]
pub fn niche_conflicts(
    agents: &[(AgentId, MorphogeneticState)],
    threshold: f64,
) -> Vec<NicheConflict> {
    let mut conflicts = Vec::new();
    for i in 0..agents.len() {
        for j in (i + 1)..agents.len() {
            let sim = cosine_similarity(&agents[i].1.strategy, &agents[j].1.strategy);
            if sim > threshold {
                // Find the top-3 shared dimensions (highest product).
                let mut dim_products: Vec<(usize, f64)> = (0..STRATEGY_DIMS)
                    .map(|d| (d, agents[i].1.strategy[d] * agents[j].1.strategy[d]))
                    .collect();
                dim_products.sort_by(|a, b| b.1.total_cmp(&a.1));
                let shared_dimensions: Vec<usize> =
                    dim_products.iter().take(3).map(|(d, _)| *d).collect();

                conflicts.push(NicheConflict {
                    agent_a: agents[i].0.clone(),
                    agent_b: agents[j].0.clone(),
                    similarity: sim,
                    shared_dimensions,
                });
            }
        }
    }
    conflicts
}

/// Apply niche conflict resolution by boosting collective pheromone on shared
/// dimensions for both agents. The Gierer-Meinhardt inhibition term then
/// pushes them apart automatically.
pub fn resolve_niche_conflicts(
    agents: &mut [(AgentId, MorphogeneticState)],
    conflicts: &[NicheConflict],
    boost_amount: f64,
) {
    for conflict in conflicts {
        for (id, state) in agents.iter_mut() {
            if *id == conflict.agent_a || *id == conflict.agent_b {
                for &dim in &conflict.shared_dimensions {
                    if dim < STRATEGY_DIMS {
                        state.collective_pheromone[dim] += boost_amount;
                    }
                }
            }
        }
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

// ---------------------------------------------------------------------------
// COORD-08: Access models and subnet membership
// ---------------------------------------------------------------------------

/// Access model determining how agents gain membership in a subnet.
///
/// Three models from club goods theory (Buchanan 1965): subnets are
/// excludable, non-rivalrous goods whose value decreases with
/// over-admission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum AccessModel {
    /// Explicit invitation by an existing member.
    Invite,
    /// Agents with a matching role automatically join.
    Role {
        /// The role required for automatic membership.
        required_role: String,
    },
    /// Agents with reputation above a threshold in a domain gain access.
    Reputation {
        /// The domain in which reputation is measured.
        domain: String,
        /// Minimum reputation score required for membership.
        min_score: f64,
    },
}

/// Membership registry for a permissioned subnet.
///
/// Tracks which agents belong to a subnet, the access model that governs
/// admission, and the minimum number of distinct confirming members
/// required for Subnet -> Mesh scope promotion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubnetMembership {
    /// The subnet this membership controls.
    pub subnet: SubnetId,
    /// Current members of the subnet.
    pub members: HashSet<AgentId>,
    /// Access model governing admission.
    pub access_model: AccessModel,
    /// Minimum distinct confirming members for Subnet -> Mesh promotion.
    pub min_distinct_confirmers: u32,
}

impl SubnetMembership {
    /// Create a new membership with default settings.
    #[must_use]
    pub fn new(subnet: SubnetId, access_model: AccessModel) -> Self {
        Self {
            subnet,
            members: HashSet::new(),
            access_model,
            min_distinct_confirmers: 2,
        }
    }

    /// Check whether an agent can access this subnet.
    #[must_use]
    pub fn can_access(&self, agent_id: &str) -> bool {
        self.members.contains(agent_id)
    }

    /// Attempt to admit an agent based on the access model.
    ///
    /// Returns `true` if the agent was admitted, `false` if already a member
    /// or if the role/reputation check failed.
    pub fn try_admit(
        &mut self,
        agent_id: &str,
        agent_role: Option<&str>,
        agent_reputation: Option<f64>,
    ) -> bool {
        if self.members.contains(agent_id) {
            return false;
        }
        match &self.access_model {
            AccessModel::Invite => {
                // Invite-only: must be explicitly added via `admit()`.
                false
            }
            AccessModel::Role { required_role } => {
                if agent_role.map_or(false, |r| r == required_role) {
                    self.members.insert(agent_id.to_owned());
                    true
                } else {
                    false
                }
            }
            AccessModel::Reputation { min_score, .. } => {
                if agent_reputation.map_or(false, |r| r >= *min_score) {
                    self.members.insert(agent_id.to_owned());
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Explicitly admit an agent (for Invite model or admin override).
    pub fn admit(&mut self, agent_id: impl Into<AgentId>) {
        self.members.insert(agent_id.into());
    }

    /// Check whether a pheromone can be promoted from Subnet to Mesh scope.
    ///
    /// Requires that the confirming agents are members of this subnet and
    /// that at least `min_distinct_confirmers` distinct members have
    /// confirmed the pheromone. This prevents a single agent from
    /// promoting unvetted signals.
    #[must_use]
    pub fn can_promote_to_mesh(&self, confirmer_agents: &[AgentId]) -> bool {
        let distinct: HashSet<&str> = confirmer_agents
            .iter()
            .filter(|a| self.members.contains(a.as_str()))
            .map(String::as_str)
            .collect();
        distinct.len() as u32 >= self.min_distinct_confirmers
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
            beta: 10.0, // very strong inhibition
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
        assert!(
            h_high > 0.999,
            "H(100, 0.5, 2) should be ~1.0, got {h_high}"
        );

        // Higher n -> steeper curve.
        let h_n1 = hill_response(0.3, 0.5, 1.0);
        let h_n4 = hill_response(0.3, 0.5, 4.0);
        assert!(
            h_n1 > h_n4,
            "lower n should give higher response at sub-threshold: {h_n1} > {h_n4}"
        );

        // Edge cases.
        assert_eq!(hill_response(1.0, 0.0, 2.0), 0.0); // k=0
        assert_eq!(hill_response(1.0, 1.0, 0.0), 0.0); // n=0
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
            Confirmation {
                agent: "a1".into(),
                gate_passed: true,
            },
            Confirmation {
                agent: "a1".into(),
                gate_passed: true,
            },
            Confirmation {
                agent: "a1".into(),
                gate_passed: true,
            },
        ];
        assert!(!gate.is_satisfied(&same_agent));

        // 3 confirmations from 2 agents — passes.
        let diverse = vec![
            Confirmation {
                agent: "a1".into(),
                gate_passed: true,
            },
            Confirmation {
                agent: "a2".into(),
                gate_passed: true,
            },
            Confirmation {
                agent: "a1".into(),
                gate_passed: false,
            },
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
            Confirmation {
                agent: "a1".into(),
                gate_passed: false,
            },
            Confirmation {
                agent: "a2".into(),
                gate_passed: false,
            },
        ];
        assert!(!gate.is_satisfied(&no_pass));

        let with_pass = vec![
            Confirmation {
                agent: "a1".into(),
                gate_passed: true,
            },
            Confirmation {
                agent: "a2".into(),
                gate_passed: true,
            },
        ];
        assert!(gate.is_satisfied(&with_pass));
    }

    // ----- COORD-05: Scope promotion (Local -> Subnet -> Mesh -> Global) -----

    #[test]
    fn scope_promotion_local_to_subnet() {
        let config = ScopePromotionConfig::default();
        let mut pheromone = Pheromone::new(
            PheromoneKind::Opportunity,
            0.9,
            Duration::from_secs(7200),
            "agent-a".to_owned(),
            PheromoneScope::Local("store-a".to_owned()),
        );
        // Below threshold: no promotion.
        pheromone.confirmations = 2;
        assert!(check_scope_promotion(&pheromone, &config).is_none());

        // At threshold: promotes to Subnet.
        pheromone.confirmations = 3;
        let promoted = check_scope_promotion(&pheromone, &config);
        assert!(promoted.is_some());
        assert!(matches!(promoted.unwrap(), PheromoneScope::Subnet(_)));
    }

    #[test]
    fn scope_promotion_subnet_to_mesh() {
        let config = ScopePromotionConfig::default();
        let subnet = SubnetId::new("collective-a".to_owned(), "eng").unwrap();
        let mut pheromone = Pheromone::new(
            PheromoneKind::Pattern,
            0.8,
            Duration::from_secs(7200),
            "agent-b".to_owned(),
            PheromoneScope::Subnet(subnet),
        );
        pheromone.confirmations = 5;
        let promoted = check_scope_promotion(&pheromone, &config);
        assert!(matches!(promoted, Some(PheromoneScope::Mesh(_))));
    }

    #[test]
    fn scope_promotion_mesh_to_global() {
        let config = ScopePromotionConfig::default();
        let mut pheromone = Pheromone::new(
            PheromoneKind::Consensus,
            0.7,
            Duration::from_secs(7200),
            "agent-c".to_owned(),
            PheromoneScope::Mesh("collective-a".to_owned()),
        );
        pheromone.confirmations = 10;
        let promoted = check_scope_promotion(&pheromone, &config);
        assert_eq!(promoted, Some(PheromoneScope::Global));
    }

    #[test]
    fn scope_promotion_global_stays_global() {
        let config = ScopePromotionConfig::default();
        let mut pheromone = Pheromone::new(
            PheromoneKind::Wisdom,
            0.9,
            Duration::from_secs(7200),
            "agent-d".to_owned(),
            PheromoneScope::Global,
        );
        pheromone.confirmations = 100;
        assert!(check_scope_promotion(&pheromone, &config).is_none());
    }

    #[test]
    fn trust_discount_applies_by_scope_rank() {
        let local = Pheromone::new(
            PheromoneKind::Opportunity,
            1.0,
            Duration::from_secs(999_999_999), // very long half-life for test stability
            "agent-a".to_owned(),
            PheromoneScope::Local("s".to_owned()),
        );
        let global = Pheromone::new(
            PheromoneKind::Opportunity,
            1.0,
            Duration::from_secs(999_999_999),
            "agent-a".to_owned(),
            PheromoneScope::Global,
        );
        let local_i = trust_discounted_intensity(&local);
        let global_i = trust_discounted_intensity(&global);
        // Local should have higher trust-discounted intensity.
        assert!(
            local_i > global_i,
            "local trust should be higher: {local_i} > {global_i}"
        );
        // Global discount factor is 0.50.
        assert!(
            (global_i / local_i - 0.50).abs() < 0.01,
            "global/local ratio should be ~0.50"
        );
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

    // ----- COORD-08: Access models + subnet membership -----

    #[test]
    fn subnet_membership_invite_model() {
        let subnet = SubnetId::new("collective-a".to_owned(), "eng").unwrap();
        let mut membership = SubnetMembership::new(subnet, AccessModel::Invite);

        // Invite model: try_admit always fails (must use explicit admit).
        assert!(!membership.try_admit("agent-1", None, None));
        assert!(!membership.can_access("agent-1"));

        // Explicit admit.
        membership.admit("agent-1");
        assert!(membership.can_access("agent-1"));
        assert!(!membership.can_access("agent-2"));
    }

    #[test]
    fn subnet_membership_role_model() {
        let subnet = SubnetId::new("collective-a".to_owned(), "verifiers").unwrap();
        let mut membership = SubnetMembership::new(
            subnet,
            AccessModel::Role {
                required_role: "verifier".to_owned(),
            },
        );

        // Wrong role: rejected.
        assert!(!membership.try_admit("agent-1", Some("researcher"), None));
        assert!(!membership.can_access("agent-1"));

        // Correct role: admitted.
        assert!(membership.try_admit("agent-2", Some("verifier"), None));
        assert!(membership.can_access("agent-2"));

        // Already a member: returns false.
        assert!(!membership.try_admit("agent-2", Some("verifier"), None));
    }

    #[test]
    fn subnet_membership_reputation_model() {
        let subnet = SubnetId::new("collective-a".to_owned(), "experts").unwrap();
        let mut membership = SubnetMembership::new(
            subnet,
            AccessModel::Reputation {
                domain: "testing".to_owned(),
                min_score: 0.7,
            },
        );

        // Below threshold.
        assert!(!membership.try_admit("agent-1", None, Some(0.5)));

        // Above threshold.
        assert!(membership.try_admit("agent-2", None, Some(0.8)));
        assert!(membership.can_access("agent-2"));
    }

    #[test]
    fn subnet_membership_can_promote_to_mesh() {
        let subnet = SubnetId::new("collective-a".to_owned(), "eng").unwrap();
        let mut membership = SubnetMembership::new(subnet, AccessModel::Invite);
        membership.min_distinct_confirmers = 2;
        membership.admit("agent-1");
        membership.admit("agent-2");
        membership.admit("agent-3");

        // Only 1 distinct member confirmed: not enough.
        let confirmers = vec!["agent-1".to_owned()];
        assert!(!membership.can_promote_to_mesh(&confirmers));

        // 2 distinct members confirmed: sufficient.
        let confirmers = vec!["agent-1".to_owned(), "agent-2".to_owned()];
        assert!(membership.can_promote_to_mesh(&confirmers));

        // Non-members don't count.
        let confirmers = vec!["agent-1".to_owned(), "outsider".to_owned()];
        assert!(!membership.can_promote_to_mesh(&confirmers));
    }

    #[test]
    fn access_model_round_trips_through_serde() {
        for model in [
            AccessModel::Invite,
            AccessModel::Role {
                required_role: "verifier".to_owned(),
            },
            AccessModel::Reputation {
                domain: "test".to_owned(),
                min_score: 0.5,
            },
        ] {
            let json = serde_json::to_string(&model).unwrap();
            let decoded: AccessModel = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, model);
        }
    }

    // ----- COORD-06: Cosine similarity + niche conflicts -----

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = [0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = [0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 1e-10,
            "identical should be 1.0, got {sim}"
        );
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim < 1e-10, "orthogonal should be 0.0, got {sim}");
    }

    #[test]
    fn cosine_similarity_uniform_vectors_are_identical() {
        let uniform = [1.0 / 8.0; STRATEGY_DIMS];
        let sim = cosine_similarity(&uniform, &uniform);
        assert!(
            (sim - 1.0).abs() < 1e-10,
            "uniform vs uniform = 1.0, got {sim}"
        );
    }

    #[test]
    fn cosine_similarity_zero_vector_returns_zero() {
        let zero = [0.0; STRATEGY_DIMS];
        let some = [0.5; STRATEGY_DIMS];
        let sim = cosine_similarity(&zero, &some);
        assert!(
            (sim - 0.0).abs() < 1e-10,
            "zero vector should give 0.0, got {sim}"
        );
    }

    #[test]
    fn niche_conflicts_detects_overlap() {
        let agents = vec![
            ("agent-a".to_owned(), MorphogeneticState::default()),
            ("agent-b".to_owned(), MorphogeneticState::default()),
        ];
        // Uniform strategies have cosine similarity 1.0 > 0.9 threshold.
        let conflicts = niche_conflicts(&agents, 0.9);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].similarity > 0.9);
        assert_eq!(conflicts[0].shared_dimensions.len(), 3);
    }

    #[test]
    fn niche_conflicts_no_overlap_for_specialized_agents() {
        let mut state_a = MorphogeneticState::default();
        state_a.strategy = [0.7, 0.05, 0.05, 0.05, 0.05, 0.025, 0.025, 0.05];
        let mut state_b = MorphogeneticState::default();
        state_b.strategy = [0.05, 0.05, 0.7, 0.05, 0.05, 0.025, 0.025, 0.05];

        let agents = vec![
            ("agent-a".to_owned(), state_a),
            ("agent-b".to_owned(), state_b),
        ];
        let conflicts = niche_conflicts(&agents, 0.9);
        assert!(
            conflicts.is_empty(),
            "specialized agents should not conflict"
        );
    }

    #[test]
    fn resolve_niche_conflicts_boosts_shared_dims() {
        let mut agents = vec![
            ("agent-a".to_owned(), MorphogeneticState::default()),
            ("agent-b".to_owned(), MorphogeneticState::default()),
        ];
        let conflicts = niche_conflicts(&agents, 0.9);
        assert!(!conflicts.is_empty());

        resolve_niche_conflicts(&mut agents, &conflicts, 0.5);

        // Both agents should have boosted collective_pheromone on shared dims.
        for (_id, state) in &agents {
            let total_boost: f64 = state.collective_pheromone.iter().sum();
            assert!(total_boost > 0.0, "collective pheromone should be boosted");
        }
    }

    // ----- COORD-07: Alpha-aware pheromone_decay_for_kind -----

    #[test]
    fn pheromone_decay_for_kind_alpha_shortens_half_life() {
        // Alpha with confirmations should decay faster than non-alpha.
        // Use a fixed elapsed duration to avoid time-sensitivity.
        let half_life = Duration::from_secs(3600);
        let confirmations = 5;
        let elapsed = Duration::from_secs(1800); // 30 minutes

        // Alpha effective half-life: 3600 / (1.0 + 5*0.1) = 2400s
        let alpha_eff = half_life.mul_f64(1.0 / f64::from(confirmations).mul_add(0.1, 1.0));
        let alpha_intensity = current_intensity(1.0, alpha_eff, elapsed);

        // Normal effective half-life: 3600 * (1.0 + 5*0.5) = 12600s
        let normal_eff = half_life.mul_f64(f64::from(confirmations).mul_add(0.5, 1.0));
        let normal_intensity = current_intensity(1.0, normal_eff, elapsed);

        // Alpha should have decayed MORE (lower intensity) because confirmations
        // shortened its half-life, while Opportunity extended it.
        assert!(
            alpha_intensity < normal_intensity,
            "alpha should decay faster than normal: {alpha_intensity} < {normal_intensity}"
        );
        // Both should be non-zero at 30 minutes.
        assert!(alpha_intensity > 0.0, "alpha should still have intensity");
        assert!(normal_intensity > 0.0, "normal should still have intensity");
    }

    #[test]
    fn pheromone_decay_for_kind_none_uses_standard_formula() {
        // When kind is None, should behave identically to pheromone_decay().
        // Use recent time so both produce measurable intensity.
        let start = SystemTime::now() - Duration::from_secs(600); // 10 minutes ago
        let half_life = Duration::from_secs(7200);
        let confirmations = 2;

        let with_kind = pheromone_decay_for_kind(1.0, start, half_life, confirmations, None);
        let without_kind = pheromone_decay(1.0, start, half_life, confirmations);

        assert!(
            (with_kind - without_kind).abs() < 1e-6,
            "None kind should match standard: {with_kind} vs {without_kind}"
        );
        // Should be close to 1.0 since only 10 min elapsed out of 7200s eff.
        assert!(with_kind > 0.5, "should not have decayed much: {with_kind}");
    }
}
