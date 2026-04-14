//! `LinUCB` contextual bandit router for model selection (section 13.3-13.7).
//!
//! This module implements the `LinUCB` algorithm (Li et al., 2010) for
//! context-dependent model routing. Each model is an "arm" with its own
//! A matrix (d x d) and b vector (d x 1). Given a context vector x,
//! the algorithm selects the arm with the highest upper confidence bound:
//!
//! ```text
//! score(a) = theta_a^T * x + alpha * sqrt(x^T * A_a^{-1} * x)
//! ```
//!
//! where `theta_a = A_a^{-1} * b_a`.
//!
//! # Features
//!
//! The context vector encodes:
//! - Task category (one-hot, 8 dimensions for [`TaskCategory`] variants)
//! - Complexity band (scalar 0.0 / 0.5 / 1.0)
//! - Iteration (normalized: iteration / 10, capped at 1.0)
//! - Agent role (hashed to 4-dim float vector)
//! - Crate familiarity for the crate being modified
//!   (`success_count / total_count`, clamped to `[0.0, 1.0]`)
//! - Has prior failure (0.0 or 1.0)
//! - Bias term (always 1.0)
//! - Cache affinity to the previous model (1.0 when the candidate matches)
//!
//! Total dimension: 8 + 1 + 1 + 4 + 1 + 1 + 1 + 1 = 18.
//!
//! # Cold start
//!
//! When observation count is below 50, the router falls back to a static
//! mapping from [`ModelTier`] to a default model slug.
//!
//! # Alpha decay
//!
//! The exploration parameter alpha decays exponentially from 1.0 to 0.05
//! over 200 observations: `alpha = 0.05 + 0.95 * exp(-observations / 60)`.
//!
//! # Thread safety
//!
//! All mutable state is behind a [`parking_lot::RwLock`] so the router can
//! be shared across async tasks via `Arc<LinUCBRouter>` while allowing
//! concurrent read-side routing.

use crate::cost_table::CostTable;
use parking_lot::RwLock;
use rand::Rng;
use roko_core::DaimonPolicy;
use roko_core::agent::{AgentRole, ModelSpec, ModelTier};
pub use roko_core::config::schema::RewardWeights;
use roko_core::task::{TaskCategory, TaskComplexityBand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ─── Constants ──────────────────────────────────────────────────────────────

/// Dimensionality of the context vector.
pub const CONTEXT_DIM: usize = 18;

/// Minimum observations before `LinUCB` is used (below this, static routing).
pub const COLD_START_THRESHOLD: u64 = 50;

/// Minimum alpha value (exploration floor).
const ALPHA_MIN: f64 = 0.05;

/// Maximum alpha value (exploration ceiling at cold start).
const ALPHA_MAX: f64 = 1.0;

/// Decay time constant: chosen so that at 200 observations,
/// `exp(-200/60) ~ 0.036`, giving alpha ~ 0.084,
/// and it effectively converges to `ALPHA_MIN`.
const ALPHA_TAU: f64 = 60.0;
/// Default discount factor for Thompson sampling in non-stationary environments.
const THOMPSON_DEFAULT_DISCOUNT: f64 = 0.99;

// ─── RoutingContext ─────────────────────────────────────────────────────────

/// Input features for model selection.
#[derive(Debug, Clone)]
pub struct RoutingContext {
    /// Broad category of the task.
    pub task_category: TaskCategory,
    /// Complexity band (Fast / Standard / Complex).
    pub complexity: TaskComplexityBand,
    /// Current iteration number (0-based).
    pub iteration: u32,
    /// Agent role requesting the model.
    pub role: AgentRole,
    /// Familiarity with the target crate (0.0 = unknown, 1.0 = very familiar).
    pub crate_familiarity: f64,
    /// Whether a prior attempt at this task has failed.
    pub has_prior_failure: bool,
    /// Normalized conductor pressure in `[0, 1]` derived from active agents,
    /// ready-queue depth, and queue wait time.
    pub conductor_load: f64,
    /// Number of currently active agent processes.
    pub active_agents: u32,
    /// Number of ready tasks currently waiting in the queue.
    pub ready_queue_depth: u32,
    /// Longest observed ready-queue wait across queued tasks, in hours.
    pub max_queue_wait_hours: f64,
    /// First-class affect policy snapshot from the Daimon.
    pub daimon_policy: DaimonPolicy,
    /// Requested thinking / reasoning level for this task, if any.
    pub thinking_level: Option<String>,
    /// Model used for the previous task in the same plan.
    pub previous_model: Option<String>,
    /// Estimated shared prefix size for cached context reuse.
    pub plan_context_tokens: Option<u64>,
}

impl RoutingContext {
    /// Encode into a fixed-length feature vector of dimension [`CONTEXT_DIM`].
    #[must_use]
    pub fn to_features(&self) -> Vec<f64> {
        self.to_features_for_model(self.previous_model.as_deref())
    }

    /// Encode into a fixed-length feature vector for a specific candidate model.
    #[must_use]
    pub fn to_features_for_model(&self, candidate_model: Option<&str>) -> Vec<f64> {
        let mut x = vec![0.0; CONTEXT_DIM];
        let mut idx = 0;

        // One-hot for TaskCategory (8 variants)
        let cat_idx = task_category_index(self.task_category);
        x[cat_idx] = 1.0;
        idx += 8;

        // Complexity scalar
        x[idx] = complexity_to_float(self.complexity);
        idx += 1;

        // Iteration (normalized, capped at 1.0)
        x[idx] = (f64::from(self.iteration) / 10.0).min(1.0);
        idx += 1;

        // Role hash (4 float features from the role label hash)
        let role_hash = hash_role(self.role);
        x[idx..idx + 4].copy_from_slice(&role_hash);
        idx += 4;

        // Per-crate familiarity score for the crate being modified.
        x[idx] = self.crate_familiarity.clamp(0.0, 1.0);
        idx += 1;

        // Has prior failure
        x[idx] = if self.has_prior_failure { 1.0 } else { 0.0 };
        idx += 1;

        // Bias term
        x[idx] = 1.0;
        idx += 1;

        // Cache affinity bonus for reusing the same model as the previous task.
        x[idx] = if candidate_model
            .is_some_and(|candidate| self.previous_model.as_deref() == Some(candidate))
        {
            1.0
        } else {
            0.0
        };

        x
    }
}

/// Map [`TaskCategory`] to a 0-based index for one-hot encoding.
const fn task_category_index(cat: TaskCategory) -> usize {
    match cat {
        TaskCategory::Implementation => 1,
        TaskCategory::Integration => 2,
        TaskCategory::Verification => 3,
        TaskCategory::Research => 4,
        TaskCategory::Refactor => 5,
        TaskCategory::Infra => 6,
        TaskCategory::Docs => 7,
        // Scaffolding and forward-compat unknown categories map to slot 0
        _ => 0,
    }
}

/// Map complexity band to a float scalar.
const fn complexity_to_float(band: TaskComplexityBand) -> f64 {
    match band {
        TaskComplexityBand::Fast => 0.0,
        TaskComplexityBand::Complex => 1.0,
        // Standard and forward-compat
        _ => 0.5,
    }
}

/// Produce a 4-element float vector from a role label.
///
/// Uses a simple string hash distributed across 4 buckets, each scaled to `[0,1]`.
#[allow(clippy::cast_precision_loss)]
fn hash_role(role: AgentRole) -> [f64; 4] {
    let label = role.label();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    for b in label.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3); // FNV prime
    }
    [
        (h & 0xFFFF) as f64 / 65535.0,
        ((h >> 16) & 0xFFFF) as f64 / 65535.0,
        ((h >> 32) & 0xFFFF) as f64 / 65535.0,
        ((h >> 48) & 0xFFFF) as f64 / 65535.0,
    ]
}

// ─── Reward computation ─────────────────────────────────────────────────────

/// Compute the composite reward signal for a model observation.
///
/// Formula: `pass_rate * 0.5 + (1.0 - normalized_cost) * 0.3 + (1.0 - normalized_duration) * 0.2`
///
/// All inputs should be in `[0, 1]`. Values are clamped.
#[must_use]
pub fn compute_routing_reward(
    pass_rate: f64,
    normalized_cost: f64,
    normalized_duration: f64,
) -> f64 {
    compute_routing_reward_with_weights(
        pass_rate,
        normalized_cost,
        normalized_duration,
        &RewardWeights::default(),
    )
}

/// Compute the scalarized routing reward using explicit reward weights.
#[must_use]
pub fn compute_routing_reward_with_weights(
    pass_rate: f64,
    normalized_cost: f64,
    normalized_duration: f64,
    weights: &RewardWeights,
) -> f64 {
    let pr = pass_rate.clamp(0.0, 1.0);
    let nc = normalized_cost.clamp(0.0, 1.0);
    let nd = normalized_duration.clamp(0.0, 1.0);
    (1.0 - nd).mul_add(
        weights.latency,
        pr.mul_add(weights.quality, (1.0 - nc) * weights.cost),
    )
}

/// Normalize a model's blended cost against the routing ceiling.
///
/// Uses the cost table's blended per-million-token estimate so cost
/// comparisons stay consistent across providers with different pricing
/// structures and tokenizers.
#[must_use]
pub fn normalized_cost(model_slug: &str, cost_table: &CostTable) -> f64 {
    let blended = cost_table.blended_cost_per_m(model_slug);
    let max_blended = 75.0;
    (blended / max_blended).min(1.0)
}

/// Compute the composite reward signal using observed latency and an SLA.
///
/// Faster models get a higher reward because the observed latency is normalized
/// against the latency SLA before being fed into the same reward formula.
#[must_use]
pub fn compute_routing_reward_v2(
    pass_rate: f64,
    normalized_cost: f64,
    observed_latency_ms: f64,
    latency_sla_ms: f64,
) -> f64 {
    compute_routing_reward_v2_with_weights(
        pass_rate,
        normalized_cost,
        observed_latency_ms,
        latency_sla_ms,
        &RewardWeights::default(),
    )
}

/// Compute the scalarized reward using observed latency and explicit weights.
#[must_use]
pub fn compute_routing_reward_v2_with_weights(
    pass_rate: f64,
    normalized_cost: f64,
    observed_latency_ms: f64,
    latency_sla_ms: f64,
    weights: &RewardWeights,
) -> f64 {
    let normalized_duration = if latency_sla_ms > 0.0 {
        (observed_latency_ms / latency_sla_ms).min(1.0)
    } else {
        1.0
    };

    compute_routing_reward_with_weights(pass_rate, normalized_cost, normalized_duration, weights)
}

// ─── Per-arm state ──────────────────────────────────────────────────────────

/// Per-arm reward vector statistics for multi-objective routing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MultiObjectiveStats {
    /// Sum of observed quality rewards.
    pub quality_sum: f64,
    /// Sum of squared quality rewards.
    pub quality_sq_sum: f64,
    /// Sum of observed normalized costs.
    pub cost_sum: f64,
    /// Sum of squared normalized costs.
    pub cost_sq_sum: f64,
    /// Sum of observed normalized latencies.
    pub latency_sum: f64,
    /// Sum of squared normalized latencies.
    pub latency_sq_sum: f64,
    /// Number of multi-objective observations recorded.
    pub observations: u64,
}

impl MultiObjectiveStats {
    /// Record one quality / cost / latency observation.
    pub fn observe(&mut self, quality: f64, cost: f64, latency: f64) {
        let quality = quality.clamp(0.0, 1.0);
        let cost = cost.clamp(0.0, 1.0);
        let latency = latency.clamp(0.0, 1.0);

        self.quality_sum += quality;
        self.quality_sq_sum += quality * quality;
        self.cost_sum += cost;
        self.cost_sq_sum += cost * cost;
        self.latency_sum += latency;
        self.latency_sq_sum += latency * latency;
        self.observations += 1;
    }

    /// Convert the accumulated vector into a scalar reward using `weights`.
    #[must_use]
    pub fn scalarize(&self, weights: &RewardWeights) -> f64 {
        let observations = self.observations.max(1) as f64;
        let q = self.quality_sum / observations;
        let c = 1.0 - (self.cost_sum / observations);
        let l = 1.0 - (self.latency_sum / observations);
        q * weights.quality + c * weights.cost + l * weights.latency
    }
}

/// Serializable state for one `LinUCB` arm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmState {
    /// Model slug this arm represents.
    pub slug: String,
    /// A matrix (d x d), stored row-major as `Vec<Vec<f64>>`.
    pub a_matrix: Vec<Vec<f64>>,
    /// b vector (d x 1).
    pub b_vector: Vec<f64>,
    /// Number of observations for this arm.
    pub observations: u64,
    /// Multi-objective reward history for this arm.
    #[serde(default)]
    pub reward_stats: MultiObjectiveStats,
}

/// Debug score for one candidate arm under a specific routing context.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateArmScore {
    /// Model slug this score belongs to.
    pub slug: String,
    /// Full LinUCB score (`exploitation + exploration`).
    pub score: f64,
    /// Learned mean-reward estimate (`theta^T * x`).
    pub exploitation: f64,
    /// Uncertainty bonus added for exploration.
    pub exploration: f64,
}

impl ArmState {
    /// Create a fresh arm with identity A matrix and zero b vector.
    fn new(slug: impl Into<String>, dim: usize) -> Self {
        let mut a = vec![vec![0.0; dim]; dim];
        for (i, row) in a.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        Self {
            slug: slug.into(),
            a_matrix: a,
            b_vector: vec![0.0; dim],
            observations: 0,
            reward_stats: MultiObjectiveStats::default(),
        }
    }
}

/// Serializable state for one Thompson-sampling arm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThompsonArm {
    /// Model slug this arm represents.
    pub slug: String,
    /// Success count plus Beta prior.
    pub alpha: f64,
    /// Failure count plus Beta prior.
    pub beta: f64,
    /// Running reward sum for future continuous Thompson variants.
    pub sum_reward: f64,
    /// Running squared reward sum for future continuous Thompson variants.
    pub sum_reward_sq: f64,
    /// Number of observations recorded for this arm.
    pub observations: u64,
    /// Discount factor applied before each update for non-stationarity.
    pub discount: f64,
}

impl ThompsonArm {
    /// Create a fresh Thompson arm with Beta(1, 1) priors.
    #[must_use]
    pub fn new(slug: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            alpha: 1.0,
            beta: 1.0,
            sum_reward: 0.0,
            sum_reward_sq: 0.0,
            observations: 0,
            discount: THOMPSON_DEFAULT_DISCOUNT,
        }
    }

    /// Sample a Bernoulli success rate from the arm's Beta posterior.
    #[must_use]
    pub fn sample(&self) -> f64 {
        let mut rng = rand::thread_rng();
        self.sample_with_rng(&mut rng)
    }

    /// Update the posterior with a new reward and success outcome.
    pub fn update(&mut self, reward: f64, success: bool) {
        self.alpha = 1.0 + self.discount * (self.alpha - 1.0);
        self.beta = 1.0 + self.discount * (self.beta - 1.0);

        if success {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }

        self.sum_reward += reward;
        self.sum_reward_sq += reward * reward;
        self.observations += 1;
    }

    #[must_use]
    fn sample_with_rng<R: Rng + ?Sized>(&self, rng: &mut R) -> f64 {
        sample_beta(self.alpha, self.beta, rng).clamp(0.0, 1.0)
    }
}

// ─── Simple matrix operations ───────────────────────────────────────────────

/// Multiply a matrix A (n x n) by a vector x (n x 1), returning n x 1.
fn mat_vec_mul(a: &[Vec<f64>], x: &[f64]) -> Vec<f64> {
    a.iter()
        .map(|row| row.iter().zip(x).map(|(ai, xi)| ai * xi).sum())
        .collect()
}

/// Dot product of two vectors.
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(ai, bi)| ai * bi).sum()
}

fn sample_beta<R: Rng + ?Sized>(alpha: f64, beta: f64, rng: &mut R) -> f64 {
    let x = sample_gamma(alpha.max(f64::MIN_POSITIVE), rng);
    let y = sample_gamma(beta.max(f64::MIN_POSITIVE), rng);
    let total = x + y;
    if total <= 0.0 { 0.5 } else { x / total }
}

fn sample_gamma<R: Rng + ?Sized>(shape: f64, rng: &mut R) -> f64 {
    if shape <= 0.0 {
        return 0.0;
    }

    if shape < 1.0 {
        let u = sample_open_unit(rng);
        return sample_gamma(shape + 1.0, rng) * u.powf(1.0 / shape);
    }

    if (shape - 1.0).abs() < f64::EPSILON {
        return -sample_open_unit(rng).ln();
    }

    let d = shape - (1.0 / 3.0);
    let c = (1.0 / (9.0 * d)).sqrt();

    loop {
        let x = sample_standard_normal(rng);
        let v = 1.0 + c * x;
        if v <= 0.0 {
            continue;
        }

        let v_cubed = v * v * v;
        let u = sample_open_unit(rng);

        if u < 1.0 - 0.0331 * x.powi(4) {
            return d * v_cubed;
        }

        if u.ln() < 0.5 * x * x + d * (1.0 - v_cubed + v_cubed.ln()) {
            return d * v_cubed;
        }
    }
}

fn sample_standard_normal<R: Rng + ?Sized>(rng: &mut R) -> f64 {
    let u1 = sample_open_unit(rng);
    let u2 = sample_open_unit(rng);
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

fn sample_open_unit<R: Rng + ?Sized>(rng: &mut R) -> f64 {
    rng.gen_range(f64::MIN_POSITIVE..1.0)
}

/// Compute the inverse of a positive-definite matrix using Cholesky decomposition.
///
/// Returns `None` if the matrix is not positive definite (diagonal element
/// becomes non-positive during decomposition).
#[allow(clippy::needless_range_loop)] // matrix indexing is clearer with explicit indices
fn cholesky_inverse(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = a.len();
    if n == 0 {
        return Some(vec![]);
    }

    // Cholesky decomposition: A = L * L^T
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let mut s: f64 = a[i][j];
            for k in 0..j {
                s -= l[i][k] * l[j][k];
            }
            if i == j {
                if s <= 0.0 {
                    return None;
                }
                l[i][j] = s.sqrt();
            } else {
                l[i][j] = s / l[j][j];
            }
        }
    }

    // Invert L (lower triangular inverse)
    // L * L_inv = I => for i > j:
    //   L_inv[i][j] = -(1/L[i][i]) * sum_{k=j}^{i-1} L[i][k] * L_inv[k][j]
    let mut l_inv = vec![vec![0.0; n]; n];
    for i in 0..n {
        l_inv[i][i] = 1.0 / l[i][i];
        for j in (0..i).rev() {
            let mut s = 0.0;
            for k in j..i {
                s += l[i][k] * l_inv[k][j];
            }
            l_inv[i][j] = -s / l[i][i];
        }
    }

    // A^{-1} = L^{-T} * L^{-1}
    let mut inv = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let mut s = 0.0;
            for k in i..n {
                s += l_inv[k][i] * l_inv[k][j];
            }
            inv[i][j] = s;
            inv[j][i] = s;
        }
    }

    Some(inv)
}

// ─── Serializable snapshot ──────────────────────────────────────────────────

/// Wire format for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RouterSnapshot {
    /// Per-arm state.
    arms: Vec<ArmState>,
    /// Total observations across all arms.
    total_observations: u64,
}

// ─── LinUCBRouter ───────────────────────────────────────────────────────────

/// `LinUCB` contextual bandit router for model selection.
///
/// Thread-safe: wrap in `Arc` for shared access.
pub struct LinUCBRouter {
    state: RwLock<RouterState>,
    /// Filesystem path for persistence (optional).
    persist_path: Option<PathBuf>,
    /// Static fallback table: tier -> model slug.
    static_table: HashMap<ModelTier, String>,
}

/// Interior mutable state protected by the read-write lock.
#[derive(Debug, Clone)]
struct RouterState {
    arms: Vec<ArmState>,
    total_observations: u64,
}

impl LinUCBRouter {
    /// Create a router with the given model slugs as arms.
    ///
    /// # Panics
    ///
    /// Panics if `model_slugs` is empty.
    pub fn new(model_slugs: Vec<String>) -> Self {
        assert!(
            !model_slugs.is_empty(),
            "LinUCBRouter: need at least one model"
        );
        let static_table = default_static_table(&model_slugs);
        let arms: Vec<ArmState> = model_slugs
            .into_iter()
            .map(|slug| ArmState::new(slug, CONTEXT_DIM))
            .collect();
        Self {
            state: RwLock::new(RouterState {
                arms,
                total_observations: 0,
            }),
            persist_path: None,
            static_table,
        }
    }

    /// Attach a persistence path (builder pattern).
    #[must_use]
    pub fn with_persist_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.persist_path = Some(path.into());
        self
    }

    /// Override the static fallback table (builder pattern).
    #[must_use]
    pub fn with_static_table(mut self, table: HashMap<ModelTier, String>) -> Self {
        self.static_table = table;
        self
    }

    /// Current exploration parameter alpha, decaying exponentially.
    ///
    /// `alpha = ALPHA_MIN + (ALPHA_MAX - ALPHA_MIN) * exp(-observations / ALPHA_TAU)`
    #[must_use]
    pub fn current_alpha(&self) -> f64 {
        let obs = self.state.read().total_observations;
        alpha_for_observations(obs)
    }

    /// Total observations recorded.
    #[must_use]
    pub fn total_observations(&self) -> u64 {
        self.state.read().total_observations
    }

    /// Override the total observation count (used when restoring from persisted state).
    pub fn set_total_observations(&self, count: u64) {
        self.state.write().total_observations = count;
    }

    /// Select the best model for the given context.
    ///
    /// If `total_observations < COLD_START_THRESHOLD`, returns the static
    /// fallback model for the context's complexity band tier.
    pub fn select_model(&self, ctx: &RoutingContext) -> ModelSpec {
        let state = self.state.read();

        // Cold start: use static routing.
        if state.total_observations < COLD_START_THRESHOLD {
            let tier = complexity_to_tier(ctx.complexity);
            drop(state);
            let slug = self
                .static_table
                .get(&tier)
                .cloned()
                .unwrap_or_else(|| "claude-sonnet-4-5".to_string());
            return ModelSpec::from_slug(slug);
        }

        let alpha = alpha_for_observations(state.total_observations);

        let mut best_slug = state.arms[0].slug.clone();
        let mut best_score = f64::NEG_INFINITY;

        for arm in &state.arms {
            let x = ctx.to_features_for_model(Some(&arm.slug));
            let score = linucb_score(arm, &x, alpha);
            if score > best_score {
                best_score = score;
                best_slug.clone_from(&arm.slug);
            }
        }

        drop(state);
        ModelSpec::from_slug(best_slug)
    }

    /// Select the best model for a raw 18-dim context vector.
    ///
    /// This is the lower-level entry point used by the cascade router when it
    /// already has the encoded feature vector.
    pub fn select_features(&self, x: &[f64]) -> ModelSpec {
        let state = self.state.read();

        // Cold start: use static routing.
        if state.total_observations < COLD_START_THRESHOLD {
            let tier = context_vec_to_tier(x);
            drop(state);
            let slug = self
                .static_table
                .get(&tier)
                .cloned()
                .unwrap_or_else(|| "claude-sonnet-4-5".to_string());
            return ModelSpec::from_slug(slug);
        }

        let alpha = alpha_for_observations(state.total_observations);

        let mut best_slug = state.arms[0].slug.clone();
        let mut best_score = f64::NEG_INFINITY;

        for arm in &state.arms {
            let score = linucb_score(arm, &x, alpha);
            if score > best_score {
                best_score = score;
                best_slug.clone_from(&arm.slug);
            }
        }

        drop(state);
        ModelSpec::from_slug(best_slug)
    }

    /// Select the best model from a filtered candidate set.
    ///
    /// This preserves the existing LinUCB scoring logic while restricting the
    /// eligible arms to `candidate_slugs`.
    pub fn select_features_from_candidates(
        &self,
        ctx: &RoutingContext,
        candidate_slugs: &[String],
    ) -> ModelSpec {
        let alpha = {
            let state = self.state.read();
            alpha_for_observations(state.total_observations)
        };
        self.select_features_from_candidates_with_alpha_adjuster(ctx, candidate_slugs, |_| alpha)
    }

    /// Select the best model from a filtered candidate set using a custom
    /// exploration parameter per arm.
    ///
    /// This preserves the existing LinUCB scoring logic while allowing the
    /// caller to down-weight exploration for a subset of arms.
    pub fn select_features_from_candidates_with_alpha_adjuster<F>(
        &self,
        ctx: &RoutingContext,
        candidate_slugs: &[String],
        mut alpha_for_slug: F,
    ) -> ModelSpec
    where
        F: FnMut(&str) -> f64,
    {
        if candidate_slugs.is_empty() {
            return self.select_model(ctx);
        }

        let state = self.state.read();

        // Cold start: use the filtered static table.
        if state.total_observations < COLD_START_THRESHOLD {
            let tier = complexity_to_tier(ctx.complexity);
            let slug = pick_static_from_candidates(candidate_slugs, tier);
            return ModelSpec::from_slug(slug);
        }

        let mut best_slug: Option<String> = None;
        let mut best_score = f64::NEG_INFINITY;

        for arm in &state.arms {
            if !candidate_slugs
                .iter()
                .any(|candidate| slugs_match(&arm.slug, candidate))
            {
                continue;
            }

            let x = ctx.to_features_for_model(Some(&arm.slug));
            let score = linucb_score(arm, &x, alpha_for_slug(&arm.slug));
            if score > best_score {
                best_score = score;
                best_slug = Some(arm.slug.clone());
            }
        }

        if let Some(slug) = best_slug {
            drop(state);
            return ModelSpec::from_slug(slug);
        }

        drop(state);
        ModelSpec::from_slug(candidate_slugs[0].clone())
    }

    /// Compute raw LinUCB scores for a filtered candidate set.
    ///
    /// The returned scores use the same candidate filtering and per-arm feature
    /// encoding as [`select_features_from_candidates_with_alpha_adjuster`].
    #[must_use]
    pub fn score_features_from_candidates_with_alpha_adjuster<F>(
        &self,
        ctx: &RoutingContext,
        candidate_slugs: &[String],
        mut alpha_for_slug: F,
    ) -> Vec<(String, f64)>
    where
        F: FnMut(&str) -> f64,
    {
        if candidate_slugs.is_empty() {
            return Vec::new();
        }

        let state = self.state.read();

        if state.total_observations < COLD_START_THRESHOLD {
            let tier = complexity_to_tier(ctx.complexity);
            let selected = pick_static_from_candidates(candidate_slugs, tier);
            return candidate_slugs
                .iter()
                .map(|slug| {
                    (
                        slug.clone(),
                        if slugs_match(slug, &selected) {
                            1.0
                        } else {
                            0.0
                        },
                    )
                })
                .collect();
        }

        candidate_slugs
            .iter()
            .map(|candidate| {
                let score = state
                    .arms
                    .iter()
                    .find(|arm| slugs_match(&arm.slug, candidate))
                    .map(|arm| {
                        let x = ctx.to_features_for_model(Some(&arm.slug));
                        linucb_score(arm, &x, alpha_for_slug(&arm.slug))
                    })
                    .unwrap_or(f64::NEG_INFINITY);
                (candidate.clone(), score)
            })
            .collect()
    }

    /// Update the arm's A matrix and b vector after observing a reward.
    ///
    /// `LinUCB` update rules:
    /// - `A_a = A_a + x * x^T`
    /// - `b_a = b_a + reward * x`
    pub fn update(&self, ctx: &RoutingContext, model_slug: &str, reward: f64) {
        let x = ctx.to_features_for_model(Some(model_slug));
        let Some(model_idx) = self.model_index(model_slug) else {
            return;
        };
        self.update_features(&x, model_idx, reward);
    }

    /// Update the router with explicit multi-objective reward components.
    pub fn update_with_metrics(
        &self,
        ctx: &RoutingContext,
        model_slug: &str,
        quality: f64,
        normalized_cost: f64,
        normalized_latency: f64,
        weights: &RewardWeights,
    ) {
        let x = ctx.to_features_for_model(Some(model_slug));
        let Some(model_idx) = self.model_index(model_slug) else {
            return;
        };
        self.update_features_multi_objective(
            &x,
            model_idx,
            quality,
            normalized_cost,
            normalized_latency,
            weights,
        );
    }

    /// Update the arm identified by `model_idx` with a precomputed feature vector.
    ///
    /// This is the lower-level observation entry point used by the cascade router
    /// when it already has the raw context vector.
    ///
    /// If a `persist_path` is configured, the router state is automatically
    /// saved to disk after each update. Save errors are silently ignored so
    /// that a filesystem hiccup never breaks the update flow.
    pub fn update_features(&self, x: &[f64], model_idx: usize, reward: f64) {
        self.update_features_internal(x, model_idx, reward, None);
    }

    /// Update the router and track the underlying reward vector.
    pub fn update_features_multi_objective(
        &self,
        x: &[f64],
        model_idx: usize,
        quality: f64,
        normalized_cost: f64,
        normalized_latency: f64,
        weights: &RewardWeights,
    ) {
        let reward = compute_routing_reward_with_weights(
            quality,
            normalized_cost,
            normalized_latency,
            weights,
        );
        self.update_features_internal(
            x,
            model_idx,
            reward,
            Some((quality, normalized_cost, normalized_latency)),
        );
    }

    fn update_features_internal(
        &self,
        x: &[f64],
        model_idx: usize,
        reward: f64,
        reward_vector: Option<(f64, f64, f64)>,
    ) {
        if x.len() != CONTEXT_DIM {
            return;
        }

        {
            let mut state = self.state.write();
            let Some(arm) = state.arms.get_mut(model_idx) else {
                return;
            };

            // A = A + x * x^T
            for (i, row) in arm.a_matrix.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    *cell += x[i] * x[j];
                }
            }
            // b = b + reward * x
            for (bi, xi) in arm.b_vector.iter_mut().zip(x) {
                *bi += reward * xi;
            }
            if let Some((quality, cost, latency)) = reward_vector {
                arm.reward_stats.observe(quality, cost, latency);
            }
            arm.observations += 1;
            state.total_observations += 1;
        } // lock released before save() to avoid deadlock

        // Auto-persist when a path is configured.
        if self.persist_path.is_some() {
            let _ = self.save();
        }
    }

    /// Return the index of the arm for `model_slug`.
    #[must_use]
    pub fn model_index(&self, model_slug: &str) -> Option<usize> {
        let state = self.state.read();
        state
            .arms
            .iter()
            .position(|arm| arm.slug == model_slug)
            .or_else(|| {
                state
                    .arms
                    .iter()
                    .position(|arm| slugs_match(&arm.slug, model_slug))
            })
    }

    /// Snapshot of all arm statistics (clone under lock).
    pub fn arm_stats(&self) -> Vec<ArmState> {
        self.state.read().arms.clone()
    }

    /// Score the supplied candidates for the given routing context.
    ///
    /// This mirrors the internal LinUCB selection math without selecting a
    /// winner, so debugging surfaces can show how each arm compared.
    pub fn score_candidates_with_alpha_adjuster<F>(
        &self,
        ctx: &RoutingContext,
        candidate_slugs: &[String],
        mut alpha_for_slug: F,
    ) -> Vec<CandidateArmScore>
    where
        F: FnMut(&str) -> f64,
    {
        if candidate_slugs.is_empty() {
            return Vec::new();
        }

        let state = self.state.read();
        let mut scores = Vec::new();

        for arm in &state.arms {
            if !candidate_slugs
                .iter()
                .any(|candidate| slugs_match(&arm.slug, candidate))
            {
                continue;
            }

            let x = ctx.to_features_for_model(Some(&arm.slug));
            let alpha = alpha_for_slug(&arm.slug);
            let (exploitation, exploration) = linucb_score_components(arm, &x, alpha);
            scores.push(CandidateArmScore {
                slug: arm.slug.clone(),
                score: exploitation + exploration,
                exploitation,
                exploration,
            });
        }

        scores
    }

    /// Persist router state to the configured path.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if no persist path is set or filesystem
    /// operations fail.
    pub fn save(&self) -> std::io::Result<()> {
        let dest = self
            .persist_path
            .as_ref()
            .ok_or_else(|| std::io::Error::other("LinUCBRouter: no persist_path set"))?;

        let snapshot = {
            let state = self.state.read();
            RouterSnapshot {
                arms: state.arms.clone(),
                total_observations: state.total_observations,
            }
        };

        let json = serde_json::to_vec_pretty(&snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let parent = dest.parent().unwrap_or_else(|| Path::new("."));
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let tmp = parent.join(format!(".linucb_tmp_{nanos}.json"));
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, dest)?;
        Ok(())
    }

    /// Load router state from disk. Returns a fresh router if the file is
    /// missing.
    ///
    /// # Errors
    ///
    /// Returns an I/O or deserialization error if the file exists but cannot
    /// be read.
    pub fn load(path: impl AsRef<Path>, model_slugs: Vec<String>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::new(model_slugs).with_persist_path(path));
            }
            Err(e) => return Err(e),
        };

        let snapshot: RouterSnapshot = serde_json::from_slice(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        // Merge: use saved stats for arms whose slugs match, fresh for new arms.
        let arms: Vec<ArmState> = model_slugs
            .iter()
            .map(|slug| {
                snapshot
                    .arms
                    .iter()
                    .find(|a| &a.slug == slug)
                    .cloned()
                    .unwrap_or_else(|| ArmState::new(slug, CONTEXT_DIM))
            })
            .collect();

        let total_observations = arms.iter().map(|a| a.observations).sum();

        Ok(Self {
            state: RwLock::new(RouterState {
                arms,
                total_observations,
            }),
            persist_path: Some(path.to_path_buf()),
            static_table: default_static_table(&model_slugs),
        })
    }
}

/// Compute the `LinUCB` score for a single arm.
///
/// `score = theta^T * x + alpha * sqrt(x^T * A_inv * x)`
///
/// If matrix inversion fails (should not happen for a well-formed A with
/// identity initialization), returns the mean reward estimate only.
fn linucb_score(arm: &ArmState, x: &[f64], alpha: f64) -> f64 {
    let (exploitation, exploration) = linucb_score_components(arm, x, alpha);
    exploitation + exploration
}

/// Compute the LinUCB exploitation and exploration components separately.
fn linucb_score_components(arm: &ArmState, x: &[f64], alpha: f64) -> (f64, f64) {
    let Some(a_inv) = cholesky_inverse(&arm.a_matrix) else {
        // Fallback: use mean reward only (no exploration bonus).
        // theta ~ A_inv * b is undefined if A is singular.
        return (0.0, 0.0);
    };

    // theta = A_inv * b
    let theta = mat_vec_mul(&a_inv, &arm.b_vector);
    let exploitation = dot(&theta, x);

    // exploration = alpha * sqrt(x^T * A_inv * x)
    let a_inv_x = mat_vec_mul(&a_inv, x);
    let exploration = alpha * dot(x, &a_inv_x).max(0.0).sqrt();

    (exploitation, exploration)
}

/// Compute alpha (exploration parameter) from observation count.
///
/// Exponential decay: `alpha = ALPHA_MIN + (ALPHA_MAX - ALPHA_MIN) * exp(-n / TAU)`
#[allow(clippy::cast_precision_loss)]
fn alpha_for_observations(n: u64) -> f64 {
    let n_f = n as f64;
    (ALPHA_MAX - ALPHA_MIN).mul_add((-n_f / ALPHA_TAU).exp(), ALPHA_MIN)
}

/// Default static routing table: tier -> model slug.
fn default_static_table(model_slugs: &[String]) -> HashMap<ModelTier, String> {
    let mut table = HashMap::new();

    table.insert(
        ModelTier::Fast,
        pick_static_slug(model_slugs, &["claude-haiku-3-5"]),
    );
    table.insert(
        ModelTier::Standard,
        pick_static_slug(
            model_slugs,
            &["glm-5.1", "claude-sonnet-4-6", "claude-sonnet-4-5"],
        ),
    );
    table.insert(
        ModelTier::Premium,
        pick_static_slug(model_slugs, &["claude-opus-4"]),
    );
    table
}

fn pick_static_from_candidates(candidate_slugs: &[String], tier: ModelTier) -> String {
    let candidates = match tier {
        ModelTier::Fast => &["claude-haiku-3-5"][..],
        ModelTier::Premium => &["claude-opus-4"][..],
        _ => &["glm-5.1", "claude-sonnet-4-6", "claude-sonnet-4-5"][..],
    };

    for candidate in candidates {
        if let Some(slug) = candidate_slugs
            .iter()
            .find(|slug| slugs_match(slug, candidate))
            .cloned()
        {
            return slug;
        }
    }

    candidate_slugs[0].clone()
}

fn pick_static_slug(model_slugs: &[String], candidates: &[&str]) -> String {
    for candidate in candidates {
        if let Some(slug) = model_slugs
            .iter()
            .find(|slug| slugs_match(slug, candidate))
            .cloned()
        {
            return slug;
        }
    }

    candidates[0].to_string()
}

/// Map complexity band to model tier.
const fn complexity_to_tier(band: TaskComplexityBand) -> ModelTier {
    match band {
        TaskComplexityBand::Fast => ModelTier::Fast,
        TaskComplexityBand::Complex => ModelTier::Premium,
        // Standard and forward-compat
        _ => ModelTier::Standard,
    }
}

/// Map a raw context vector back to a model tier for cold-start routing.
fn context_vec_to_tier(x: &[f64]) -> ModelTier {
    let band = match x.get(8).copied().unwrap_or(0.5) {
        v if v <= 0.25 => TaskComplexityBand::Fast,
        v if v >= 0.75 => TaskComplexityBand::Complex,
        _ => TaskComplexityBand::Standard,
    };
    complexity_to_tier(band)
}

fn slugs_match(lhs: &str, rhs: &str) -> bool {
    lhs == rhs || slug_family(lhs).is_some_and(|family| slug_family(rhs) == Some(family))
}

// Use the canonical slug_family from cascade_router.
use crate::cascade_router::slug_family;

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

    fn test_slugs() -> Vec<String> {
        vec![
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
            "claude-opus-4".to_string(),
        ]
    }

    fn default_ctx() -> RoutingContext {
        RoutingContext {
            task_category: TaskCategory::Implementation,
            complexity: TaskComplexityBand::Standard,
            iteration: 0,
            role: AgentRole::Implementer,
            crate_familiarity: 0.5,
            has_prior_failure: false,
            conductor_load: 0.0,
            active_agents: 0,
            ready_queue_depth: 0,
            max_queue_wait_hours: 0.0,
            daimon_policy: DaimonPolicy::new(0.5, roko_core::BehavioralState::Engaged),
            thinking_level: None,
            previous_model: None,
            plan_context_tokens: None,
        }
    }

    // ── Test 1: feature vector dimension ────────────────────────────────

    #[test]
    fn context_features_have_correct_dimension() {
        let ctx = default_ctx();
        let features = ctx.to_features();
        assert_eq!(features.len(), CONTEXT_DIM);
    }

    // ── Test 2: one-hot encoding for category ───────────────────────────

    #[test]
    fn category_one_hot_is_correct() {
        let mut ctx = default_ctx();

        // Implementation -> index 1
        ctx.task_category = TaskCategory::Implementation;
        let f = ctx.to_features();
        assert_eq!(f[0], 0.0); // scaffolding
        assert_eq!(f[1], 1.0); // implementation
        assert_eq!(f[2], 0.0); // integration

        // Docs -> index 7
        ctx.task_category = TaskCategory::Docs;
        let f = ctx.to_features();
        assert_eq!(f[7], 1.0);
        // All other category slots should be 0
        for i in 0..7 {
            assert_eq!(f[i], 0.0, "slot {i} should be 0 for Docs");
        }
    }

    // ── Test 3: complexity encoding ─────────────────────────────────────

    #[test]
    fn complexity_encoding_values() {
        let mut ctx = default_ctx();
        let complexity_idx = 8; // after 8 category slots

        ctx.complexity = TaskComplexityBand::Fast;
        assert!((ctx.to_features()[complexity_idx] - 0.0).abs() < f64::EPSILON);

        ctx.complexity = TaskComplexityBand::Standard;
        assert!((ctx.to_features()[complexity_idx] - 0.5).abs() < f64::EPSILON);

        ctx.complexity = TaskComplexityBand::Complex;
        assert!((ctx.to_features()[complexity_idx] - 1.0).abs() < f64::EPSILON);
    }

    // ── Test 4: iteration normalization ──────────────────────────────────

    #[test]
    fn iteration_normalized_and_capped() {
        let mut ctx = default_ctx();
        let iter_idx = 9;

        ctx.iteration = 5;
        assert!((ctx.to_features()[iter_idx] - 0.5).abs() < f64::EPSILON);

        ctx.iteration = 10;
        assert!((ctx.to_features()[iter_idx] - 1.0).abs() < f64::EPSILON);

        // Capped at 1.0
        ctx.iteration = 20;
        assert!((ctx.to_features()[iter_idx] - 1.0).abs() < f64::EPSILON);
    }

    // ── Test 5: role hash produces 4 floats in [0,1] ────────────────────

    #[test]
    fn role_hash_in_range() {
        for role in AgentRole::ALL_AGENTS {
            let h = hash_role(role);
            for (i, v) in h.iter().enumerate() {
                assert!(
                    (0.0..=1.0).contains(v),
                    "role {:?} hash[{i}] = {v} out of range",
                    role,
                );
            }
        }
    }

    // ── Test 6: different roles produce different hashes ─────────────────

    #[test]
    fn different_roles_have_different_hashes() {
        let h1 = hash_role(AgentRole::Implementer);
        let h2 = hash_role(AgentRole::Architect);
        assert_ne!(h1, h2);
    }

    // ── Test 7: bias term is always 1.0 ─────────────────────────────────

    #[test]
    fn bias_term_always_one() {
        let ctx = default_ctx();
        let features = ctx.to_features();
        assert!((features[16] - 1.0).abs() < f64::EPSILON);
    }

    // ── Test 8: cold start returns static model ─────────────────────────

    #[test]
    fn cold_start_uses_static_table() {
        let router = LinUCBRouter::new(test_slugs());
        let ctx = default_ctx();
        let model = router.select_model(&ctx);

        // Standard complexity -> Standard tier -> sonnet
        assert_eq!(model.slug, "claude-sonnet-4-5");
    }

    // ── Test 9: cold start respects complexity tiers ─────────────────────

    #[test]
    fn cold_start_maps_tiers_correctly() {
        let router = LinUCBRouter::new(test_slugs());

        let mut ctx = default_ctx();
        ctx.complexity = TaskComplexityBand::Fast;
        assert_eq!(router.select_model(&ctx).slug, "claude-haiku-3-5");

        ctx.complexity = TaskComplexityBand::Complex;
        assert_eq!(router.select_model(&ctx).slug, "claude-opus-4");
    }

    // ── Test 10: alpha decay starts at ~1.0 ─────────────────────────────

    #[test]
    fn alpha_starts_near_max() {
        let a = alpha_for_observations(0);
        assert!(
            (a - ALPHA_MAX).abs() < 0.01,
            "alpha at 0 obs should be ~{ALPHA_MAX}, got {a}"
        );
    }

    // ── Test 11: alpha decays over observations ─────────────────────────

    #[test]
    fn alpha_decays_monotonically() {
        let a0 = alpha_for_observations(0);
        let a50 = alpha_for_observations(50);
        let a100 = alpha_for_observations(100);
        let a200 = alpha_for_observations(200);

        assert!(a0 > a50, "alpha should decrease: {a0} > {a50}");
        assert!(a50 > a100, "alpha should decrease: {a50} > {a100}");
        assert!(a100 > a200, "alpha should decrease: {a100} > {a200}");
    }

    // ── Test 12: alpha converges toward minimum ─────────────────────────

    #[test]
    fn alpha_converges_to_min() {
        let a = alpha_for_observations(1000);
        assert!(
            (a - ALPHA_MIN).abs() < 0.01,
            "alpha at 1000 obs should be ~{ALPHA_MIN}, got {a}"
        );
    }

    // ── Test 13: reward computation ─────────────────────────────────────

    #[test]
    fn reward_formula_basic() {
        // Perfect outcome
        let r = compute_routing_reward(1.0, 0.0, 0.0);
        assert!(
            (r - 1.0).abs() < 1e-10,
            "perfect reward should be 1.0, got {r}"
        );

        // Worst outcome
        let r = compute_routing_reward(0.0, 1.0, 1.0);
        assert!(
            (r - 0.0).abs() < 1e-10,
            "worst reward should be 0.0, got {r}"
        );

        // Mixed
        let r = compute_routing_reward(0.5, 0.5, 0.5);
        // 0.5*0.5 + 0.5*0.3 + 0.5*0.2 = 0.25 + 0.15 + 0.10 = 0.50
        assert!(
            (r - 0.5).abs() < 1e-10,
            "mixed reward should be 0.5, got {r}"
        );
    }

    // ── Test 14: reward clamps inputs ───────────────────────────────────

    #[test]
    fn reward_clamps_out_of_range() {
        let r = compute_routing_reward(2.0, -1.0, 3.0);
        // clamped to (1.0, 0.0, 1.0) -> 1.0*0.5 + 1.0*0.3 + 0.0*0.2 = 0.80
        assert!(
            (r - 0.8).abs() < 1e-10,
            "clamped reward should be 0.8, got {r}"
        );
    }

    #[test]
    fn multi_objective_routing_default_weights_match_legacy_formula() {
        let legacy = compute_routing_reward(0.75, 0.2, 0.4);
        let weighted =
            compute_routing_reward_with_weights(0.75, 0.2, 0.4, &RewardWeights::default());
        assert!(
            (legacy - weighted).abs() < 1e-12,
            "default weights should preserve legacy reward, got {legacy} vs {weighted}"
        );
    }

    #[test]
    fn multi_objective_routing_scalarize_respects_weights() {
        let mut stats = MultiObjectiveStats::default();
        stats.observe(0.9, 0.2, 0.4);
        stats.observe(0.7, 0.4, 0.6);

        let cost_sensitive = RewardWeights {
            quality: 0.3,
            cost: 0.6,
            latency: 0.1,
        };
        let quality_sensitive = RewardWeights {
            quality: 0.8,
            cost: 0.1,
            latency: 0.1,
        };

        let cost_score = stats.scalarize(&cost_sensitive);
        let quality_score = stats.scalarize(&quality_sensitive);

        assert!(
            cost_score < quality_score,
            "quality-sensitive weights should favor this arm more: {cost_score} vs {quality_score}"
        );
    }

    // ── Cost normalization uses blended pricing ───────────────────────

    #[test]
    fn normalized_cost_uses_blended_pricing() {
        let table = CostTable {
            models: HashMap::new(),
        }
        .with_defaults();

        let glm_5_1 = normalized_cost("glm-5.1", &table);
        let claude_opus = normalized_cost("claude-opus-4-6", &table);
        let kimi_k2_5 = normalized_cost("kimi-k2.5", &table);

        assert!(
            (glm_5_1 - 0.0301).abs() < 0.001,
            "glm-5.1 normalized cost should be close to 0.0301, got {glm_5_1}"
        );
        assert!(
            (claude_opus - 0.4).abs() < 0.001,
            "claude-opus normalized cost should be close to 0.4, got {claude_opus}"
        );
        assert!(
            (kimi_k2_5 - 0.0157).abs() < 0.001,
            "kimi-k2.5 normalized cost should be close to 0.0157, got {kimi_k2_5}"
        );
    }

    // ── Latency-aware reward prefers faster models ─────────────────────

    #[test]
    fn routing_reward_v2_prefers_faster_models() {
        let faster = compute_routing_reward_v2(1.0, 0.25, 5_000.0, 10_000.0);
        let slower = compute_routing_reward_v2(1.0, 0.25, 9_500.0, 10_000.0);

        assert!(
            faster > slower,
            "faster model should earn higher reward: {faster} vs {slower}"
        );
    }

    // ── Test 15: update increases arm observations ──────────────────────

    #[test]
    fn update_increments_observation_count() {
        let router = LinUCBRouter::new(test_slugs());
        let ctx = default_ctx();

        router.update(&ctx, "claude-sonnet-4-5", 0.8);
        router.update(&ctx, "claude-sonnet-4-5", 0.6);

        assert_eq!(router.total_observations(), 2);

        let stats = router.arm_stats();
        let sonnet = stats
            .iter()
            .find(|a| a.slug == "claude-sonnet-4-5")
            .unwrap();
        assert_eq!(sonnet.observations, 2);
    }

    #[test]
    fn multi_objective_routing_tracks_per_arm_reward_vectors() {
        let router = LinUCBRouter::new(test_slugs());
        let ctx = default_ctx();
        let weights = RewardWeights {
            quality: 0.3,
            cost: 0.6,
            latency: 0.1,
        };

        router.update_with_metrics(&ctx, "claude-sonnet-4-5", 1.0, 0.2, 0.4, &weights);
        router.update_with_metrics(&ctx, "claude-sonnet-4-5", 0.8, 0.3, 0.5, &weights);

        let stats = router.arm_stats();
        let sonnet = stats
            .iter()
            .find(|a| a.slug == "claude-sonnet-4-5")
            .unwrap();

        assert_eq!(sonnet.observations, 2);
        assert_eq!(sonnet.reward_stats.observations, 2);
        assert!((sonnet.reward_stats.quality_sum - 1.8).abs() < 1e-10);
        assert!((sonnet.reward_stats.cost_sum - 0.5).abs() < 1e-10);
        assert!((sonnet.reward_stats.latency_sum - 0.9).abs() < 1e-10);
        assert!(
            sonnet.reward_stats.scalarize(&weights) > 0.0,
            "scalarized multi-objective reward should stay positive"
        );
    }

    // ── Test 16: update modifies A and b ────────────────────────────────

    #[test]
    fn update_modifies_a_and_b_matrices() {
        let router = LinUCBRouter::new(test_slugs());
        let ctx = default_ctx();

        let before = router.arm_stats();
        let arm_before = before
            .iter()
            .find(|a| a.slug == "claude-sonnet-4-5")
            .unwrap();
        let a_diag_before = arm_before.a_matrix[0][0];
        let b0_before = arm_before.b_vector[0];

        router.update(&ctx, "claude-sonnet-4-5", 1.0);

        let after = router.arm_stats();
        let arm_after = after
            .iter()
            .find(|a| a.slug == "claude-sonnet-4-5")
            .unwrap();

        // A should have changed (x * x^T added)
        let a_diag_after = arm_after.a_matrix[0][0];
        // b should have changed (reward * x added)
        let b0_after = arm_after.b_vector[0];

        // At least one of them should differ (unless x[0] == 0 for this context)
        let x = ctx.to_features();
        if x[0].abs() > f64::EPSILON {
            assert!(
                (a_diag_after - a_diag_before).abs() > f64::EPSILON,
                "A[0][0] should change after update"
            );
            assert!(
                (b0_after - b0_before).abs() > f64::EPSILON,
                "b[0] should change after update"
            );
        }
    }

    // ── Test 17: LinUCB selects after training ──────────────────────────

    #[test]
    fn linucb_selects_best_arm_after_training() {
        let router = LinUCBRouter::new(test_slugs());
        let ctx = default_ctx();

        // Train heavily: sonnet always gets reward 1.0, others get 0.0.
        for _ in 0..80 {
            router.update(&ctx, "claude-sonnet-4-5", 1.0);
            router.update(&ctx, "claude-haiku-3-5", 0.0);
            router.update(&ctx, "claude-opus-4", 0.0);
        }

        // Now past cold-start threshold, should pick sonnet.
        let model = router.select_model(&ctx);
        assert_eq!(
            model.slug, "claude-sonnet-4-5",
            "after heavy training, should select the highest-reward arm"
        );
    }

    // ── Test 18: cholesky inverse of identity ───────────────────────────

    #[test]
    fn cholesky_inverse_of_identity() {
        let n = 4;
        let mut id = vec![vec![0.0; n]; n];
        for i in 0..n {
            id[i][i] = 1.0;
        }

        let inv = cholesky_inverse(&id).expect("identity should be invertible");
        for i in 0..n {
            for j in 0..n {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (inv[i][j] - expected).abs() < 1e-10,
                    "inv[{i}][{j}] = {}, expected {expected}",
                    inv[i][j]
                );
            }
        }
    }

    // ── Test 19: cholesky inverse of known matrix ───────────────────────

    #[test]
    fn cholesky_inverse_known_matrix() {
        // 2x2 positive definite matrix: [[4,2],[2,3]]
        // det = 8, Inverse: [[3/8, -2/8],[-2/8, 4/8]] = [[0.375, -0.25],[-0.25, 0.5]]
        let a = vec![vec![4.0, 2.0], vec![2.0, 3.0]];
        let inv = cholesky_inverse(&a).expect("should be invertible");

        assert!(
            (inv[0][0] - 0.375).abs() < 1e-10,
            "inv[0][0] = {}",
            inv[0][0]
        );
        assert!(
            (inv[0][1] - (-0.25)).abs() < 1e-10,
            "inv[0][1] = {}",
            inv[0][1]
        );
        assert!(
            (inv[1][0] - (-0.25)).abs() < 1e-10,
            "inv[1][0] = {}",
            inv[1][0]
        );
        assert!((inv[1][1] - 0.5).abs() < 1e-10, "inv[1][1] = {}", inv[1][1]);
    }

    // ── Test 20: persistence round-trip ─────────────────────────────────

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let path = dir.path().join("model-router.json");

        let router = LinUCBRouter::new(test_slugs()).with_persist_path(&path);
        let ctx = default_ctx();
        router.update(&ctx, "claude-sonnet-4-5", 0.9);
        router.update(&ctx, "claude-haiku-3-5", 0.3);
        router.save().expect("save");

        let loaded = LinUCBRouter::load(&path, test_slugs()).expect("load");
        assert_eq!(loaded.total_observations(), 2);

        let stats = loaded.arm_stats();
        let sonnet = stats
            .iter()
            .find(|a| a.slug == "claude-sonnet-4-5")
            .unwrap();
        assert_eq!(sonnet.observations, 1);
    }

    // ── Test 21: load creates fresh on missing file ─────────────────────

    #[test]
    fn load_missing_file_creates_fresh() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let path = dir.path().join("does-not-exist.json");

        let router = LinUCBRouter::load(&path, test_slugs()).expect("load");
        assert_eq!(router.total_observations(), 0);
        assert_eq!(router.arm_stats().len(), 3);
    }

    // ── Test 22: update ignores unknown arm ─────────────────────────────

    #[test]
    fn update_unknown_arm_is_no_op() {
        let router = LinUCBRouter::new(test_slugs());
        let ctx = default_ctx();

        router.update(&ctx, "unknown-model-42", 1.0);
        assert_eq!(router.total_observations(), 0);
    }

    // ── Test 23: crate familiarity feature ──────────────────────────────

    #[test]
    fn crate_familiarity_encoded() {
        let fam_idx = 14; // 8 cat + 1 complexity + 1 iter + 4 role
        let mut ctx = default_ctx();

        ctx.crate_familiarity = 0.0;
        assert!((ctx.to_features()[fam_idx] - 0.0).abs() < f64::EPSILON);

        ctx.crate_familiarity = 1.0;
        assert!((ctx.to_features()[fam_idx] - 1.0).abs() < f64::EPSILON);

        // Clamped
        ctx.crate_familiarity = 2.0;
        assert!((ctx.to_features()[fam_idx] - 1.0).abs() < f64::EPSILON);
    }

    // ── Test 24: has_prior_failure feature ──────────────────────────────

    #[test]
    fn prior_failure_flag_encoded() {
        let fail_idx = 15; // 8 cat + 1 complexity + 1 iter + 4 role + 1 fam
        let mut ctx = default_ctx();

        ctx.has_prior_failure = false;
        assert!((ctx.to_features()[fail_idx] - 0.0).abs() < f64::EPSILON);

        ctx.has_prior_failure = true;
        assert!((ctx.to_features()[fail_idx] - 1.0).abs() < f64::EPSILON);
    }

    // ── Test 25: cache affinity feature ────────────────────────────────

    #[test]
    fn cache_affinity_feature() {
        let mut ctx = default_ctx();
        ctx.previous_model = Some("claude-sonnet-4-5".to_string());

        let features = ctx.to_features();
        assert_eq!(features.len(), CONTEXT_DIM);
        assert!((features[17] - 1.0).abs() < f64::EPSILON);

        let same = ctx.to_features_for_model(Some("claude-sonnet-4-5"));
        assert!((same[17] - 1.0).abs() < f64::EPSILON);

        let different = ctx.to_features_for_model(Some("claude-opus-4"));
        assert!((different[17] - 0.0).abs() < f64::EPSILON);
    }

    // ── Test 26: custom static table ────────────────────────────────────

    #[test]
    fn custom_static_table_used_in_cold_start() {
        let mut table = HashMap::new();
        table.insert(ModelTier::Fast, "gpt-5-mini".to_string());
        table.insert(ModelTier::Standard, "gpt-5".to_string());
        table.insert(ModelTier::Premium, "gpt-5-pro".to_string());

        let router = LinUCBRouter::new(test_slugs()).with_static_table(table);
        let mut ctx = default_ctx();
        ctx.complexity = TaskComplexityBand::Fast;

        let model = router.select_model(&ctx);
        assert_eq!(model.slug, "gpt-5-mini");
    }

    #[test]
    fn concurrent_routing_allows_parallel_readers() {
        let router = Arc::new(LinUCBRouter::new(test_slugs()));
        router.set_total_observations(COLD_START_THRESHOLD);

        let barrier = Arc::new(Barrier::new(11));
        let (tx, rx) = mpsc::channel();
        let held_read_guard = router.state.read();

        for _ in 0..10 {
            let router = Arc::clone(&router);
            let barrier = Arc::clone(&barrier);
            let tx = tx.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let model = router.select_features(&default_ctx().to_features());
                tx.send(model.slug).expect("send selected model");
            });
        }
        drop(tx);

        barrier.wait();

        for _ in 0..10 {
            let selected = rx
                .recv_timeout(Duration::from_secs(1))
                .expect("concurrent select_features should complete while a reader holds the lock");
            assert!(
                !selected.is_empty(),
                "selected model slug should not be empty"
            );
        }

        drop(held_read_guard);
    }

    #[test]
    fn cascade_router_glm_selects_glm_when_present() {
        let router =
            LinUCBRouter::new(vec!["claude-sonnet-4-6".to_string(), "glm-5.1".to_string()]);
        let ctx = default_ctx();

        let model = router.select_model(&ctx);
        assert!(
            matches!(model.slug.as_str(), "claude-sonnet-4-6" | "glm-5.1"),
            "cold-start routing should select one of the configured standard-tier models, got {}",
            model.slug
        );
    }

    // ── Test 27: exploration bonus decreases over time ───────────────────

    #[test]
    fn exploration_bonus_decreases_with_observations() {
        let router = LinUCBRouter::new(test_slugs());
        let ctx = default_ctx();
        let x = ctx.to_features();

        // Get initial score (with identity A, zero b)
        let arm_initial = &router.arm_stats()[0];
        let score_initial = linucb_score(arm_initial, &x, alpha_for_observations(0));

        // After many observations, alpha should be lower, thus lower exploration bonus
        let score_low_alpha = linucb_score(arm_initial, &x, alpha_for_observations(500));

        assert!(
            score_initial > score_low_alpha,
            "higher alpha should give higher score: {score_initial} vs {score_low_alpha}"
        );
    }

    // ── Test 28: alpha at specific observation counts ────────────────────

    #[test]
    fn alpha_at_specific_counts() {
        // At 0: should be ~1.0
        let a0 = alpha_for_observations(0);
        assert!((a0 - 1.0).abs() < 0.001);

        // At 200: should be between ALPHA_MIN and 0.1
        let a200 = alpha_for_observations(200);
        assert!(a200 > ALPHA_MIN);
        assert!(a200 < 0.15, "alpha at 200 should be < 0.15, got {a200}");

        // At 1000: should be very close to ALPHA_MIN
        let a1000 = alpha_for_observations(1000);
        assert!((a1000 - ALPHA_MIN).abs() < 0.001);
    }

    // ── Test 29: mat_vec_mul correctness ────────────────────────────────

    #[test]
    fn mat_vec_mul_works() {
        let a = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
        let x = vec![5.0, 6.0];
        let result = mat_vec_mul(&a, &x);
        assert!((result[0] - 17.0).abs() < f64::EPSILON); // 1*5 + 2*6
        assert!((result[1] - 39.0).abs() < f64::EPSILON); // 3*5 + 4*6
    }

    // ── Test 29: dot product ────────────────────────────────────────────

    #[test]
    fn dot_product_works() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        assert!((dot(&a, &b) - 32.0).abs() < f64::EPSILON); // 4+10+18
    }

    // ── Test 30: new router arm initialization ──────────────────────────

    #[test]
    fn new_arm_has_identity_a_and_zero_b() {
        let arm = ArmState::new("test", CONTEXT_DIM);
        assert_eq!(arm.a_matrix.len(), CONTEXT_DIM);
        assert_eq!(arm.b_vector.len(), CONTEXT_DIM);

        for i in 0..CONTEXT_DIM {
            for j in 0..CONTEXT_DIM {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (arm.a_matrix[i][j] - expected).abs() < f64::EPSILON,
                    "A[{i}][{j}] should be {expected}"
                );
            }
            assert!((arm.b_vector[i]).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn thompson_arm_sample_beta_in_unit_interval() {
        let arm = ThompsonArm {
            slug: "test".to_string(),
            alpha: 3.0,
            beta: 2.0,
            sum_reward: 0.0,
            sum_reward_sq: 0.0,
            observations: 0,
            discount: THOMPSON_DEFAULT_DISCOUNT,
        };
        let mut rng = ChaCha8Rng::seed_from_u64(7);

        for _ in 0..256 {
            let sample = arm.sample_with_rng(&mut rng);
            assert!(
                (0.0..=1.0).contains(&sample),
                "beta sample should stay in [0, 1], got {sample}"
            );
        }
    }

    #[test]
    fn thompson_arm_posterior_shifts_toward_more_successes() {
        let success_arm = ThompsonArm {
            slug: "success".to_string(),
            alpha: 9.0,
            beta: 2.0,
            sum_reward: 0.0,
            sum_reward_sq: 0.0,
            observations: 0,
            discount: THOMPSON_DEFAULT_DISCOUNT,
        };
        let failure_arm = ThompsonArm {
            slug: "failure".to_string(),
            alpha: 2.0,
            beta: 9.0,
            sum_reward: 0.0,
            sum_reward_sq: 0.0,
            observations: 0,
            discount: THOMPSON_DEFAULT_DISCOUNT,
        };
        let mut success_rng = ChaCha8Rng::seed_from_u64(11);
        let mut failure_rng = ChaCha8Rng::seed_from_u64(29);

        #[allow(clippy::cast_precision_loss)]
        let success_mean = (0..1024)
            .map(|_| success_arm.sample_with_rng(&mut success_rng))
            .sum::<f64>()
            / 1024.0;
        #[allow(clippy::cast_precision_loss)]
        let failure_mean = (0..1024)
            .map(|_| failure_arm.sample_with_rng(&mut failure_rng))
            .sum::<f64>()
            / 1024.0;

        assert!(
            success_mean > failure_mean,
            "posterior with more successes should sample higher on average: {success_mean} vs {failure_mean}"
        );
        assert!(
            success_mean > 0.5,
            "success-heavy posterior should skew above 0.5"
        );
        assert!(
            failure_mean < 0.5,
            "failure-heavy posterior should skew below 0.5"
        );
    }

    #[test]
    fn thompson_arm_update_applies_discount_and_accumulates_reward() {
        let mut arm = ThompsonArm {
            slug: "test".to_string(),
            alpha: 5.0,
            beta: 3.0,
            sum_reward: 1.5,
            sum_reward_sq: 1.25,
            observations: 4,
            discount: 0.99,
        };

        arm.update(0.8, true);

        assert!((arm.alpha - 5.96).abs() < 1e-10, "alpha = {}", arm.alpha);
        assert!((arm.beta - 2.98).abs() < 1e-10, "beta = {}", arm.beta);
        assert!(
            (arm.sum_reward - 2.3).abs() < 1e-10,
            "sum_reward = {}",
            arm.sum_reward
        );
        assert!(
            (arm.sum_reward_sq - 1.89).abs() < 1e-10,
            "sum_reward_sq = {}",
            arm.sum_reward_sq
        );
        assert_eq!(arm.observations, 5);
    }
}
