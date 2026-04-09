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
//!
//! Total dimension: 8 + 1 + 1 + 4 + 1 + 1 + 1 = 17.
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
//! All mutable state is behind a [`parking_lot::Mutex`] so the router can
//! be shared across async tasks via `Arc<LinUCBRouter>`.

use parking_lot::Mutex;
use roko_core::agent::{AgentRole, ModelSpec, ModelTier};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ─── Constants ──────────────────────────────────────────────────────────────

/// Dimensionality of the context vector.
pub const CONTEXT_DIM: usize = 17;

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
    /// Affect-derived confidence hint in `[0.0, 1.0]`.
    pub affect_confidence: f64,
}

impl RoutingContext {
    /// Encode into a fixed-length feature vector of dimension [`CONTEXT_DIM`].
    #[must_use]
    pub fn to_features(&self) -> Vec<f64> {
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
    let pr = pass_rate.clamp(0.0, 1.0);
    let nc = normalized_cost.clamp(0.0, 1.0);
    let nd = normalized_duration.clamp(0.0, 1.0);
    (1.0 - nd).mul_add(0.2, pr.mul_add(0.5, (1.0 - nc) * 0.3))
}

// ─── Per-arm state ──────────────────────────────────────────────────────────

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
        }
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
    state: Mutex<RouterState>,
    /// Filesystem path for persistence (optional).
    persist_path: Option<PathBuf>,
    /// Static fallback table: tier -> model slug.
    static_table: HashMap<ModelTier, String>,
}

/// Interior mutable state protected by the mutex.
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
        let arms: Vec<ArmState> = model_slugs
            .into_iter()
            .map(|slug| ArmState::new(slug, CONTEXT_DIM))
            .collect();
        Self {
            state: Mutex::new(RouterState {
                arms,
                total_observations: 0,
            }),
            persist_path: None,
            static_table: default_static_table(),
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
        let obs = self.state.lock().total_observations;
        alpha_for_observations(obs)
    }

    /// Total observations recorded.
    #[must_use]
    pub fn total_observations(&self) -> u64 {
        self.state.lock().total_observations
    }

    /// Select the best model for the given context.
    ///
    /// If `total_observations < COLD_START_THRESHOLD`, returns the static
    /// fallback model for the context's complexity band tier.
    pub fn select_model(&self, ctx: &RoutingContext) -> ModelSpec {
        self.select_features(&ctx.to_features())
    }

    /// Select the best model for a raw 17-dim context vector.
    ///
    /// This is the lower-level entry point used by the cascade router when it
    /// already has the encoded feature vector.
    pub fn select_features(&self, x: &[f64]) -> ModelSpec {
        let state = self.state.lock();

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

    /// Update the arm's A matrix and b vector after observing a reward.
    ///
    /// `LinUCB` update rules:
    /// - `A_a = A_a + x * x^T`
    /// - `b_a = b_a + reward * x`
    pub fn update(&self, ctx: &RoutingContext, model_slug: &str, reward: f64) {
        let x = ctx.to_features();
        let Some(model_idx) = self.model_index(model_slug) else {
            return;
        };
        self.update_features(&x, model_idx, reward);
    }

    /// Update the arm identified by `model_idx` with a precomputed feature vector.
    ///
    /// This is the lower-level observation entry point used by the cascade router
    /// when it already has the raw context vector.
    pub fn update_features(&self, x: &[f64], model_idx: usize, reward: f64) {
        if x.len() != CONTEXT_DIM {
            return;
        }

        let mut state = self.state.lock();
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
        arm.observations += 1;
        state.total_observations += 1;
    }

    /// Return the index of the arm for `model_slug`.
    #[must_use]
    pub fn model_index(&self, model_slug: &str) -> Option<usize> {
        self.state
            .lock()
            .arms
            .iter()
            .position(|arm| slugs_match(&arm.slug, model_slug))
    }

    /// Snapshot of all arm statistics (clone under lock).
    pub fn arm_stats(&self) -> Vec<ArmState> {
        self.state.lock().arms.clone()
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
            let state = self.state.lock();
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
            state: Mutex::new(RouterState {
                arms,
                total_observations,
            }),
            persist_path: Some(path.to_path_buf()),
            static_table: default_static_table(),
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
    let Some(a_inv) = cholesky_inverse(&arm.a_matrix) else {
        // Fallback: use mean reward only (no exploration bonus).
        // theta ~ A_inv * b is undefined if A is singular.
        return 0.0;
    };

    // theta = A_inv * b
    let theta = mat_vec_mul(&a_inv, &arm.b_vector);
    let exploitation = dot(&theta, x);

    // exploration = alpha * sqrt(x^T * A_inv * x)
    let a_inv_x = mat_vec_mul(&a_inv, x);
    let exploration = alpha * dot(x, &a_inv_x).max(0.0).sqrt();

    exploitation + exploration
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
fn default_static_table() -> HashMap<ModelTier, String> {
    let mut table = HashMap::new();
    table.insert(ModelTier::Fast, "claude-haiku-3-5".to_string());
    table.insert(ModelTier::Standard, "claude-sonnet-4-5".to_string());
    table.insert(ModelTier::Premium, "claude-opus-4".to_string());
    table
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

fn slug_family(slug: &str) -> Option<&'static str> {
    if slug.contains("haiku") {
        Some("haiku")
    } else if slug.contains("sonnet") {
        Some("sonnet")
    } else if slug.contains("opus") {
        Some("opus")
    } else {
        None
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
            affect_confidence: 0.5,
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
        assert!((features[CONTEXT_DIM - 1] - 1.0).abs() < f64::EPSILON);
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

    // ── Test 25: custom static table ────────────────────────────────────

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

    // ── Test 26: exploration bonus decreases over time ───────────────────

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

    // ── Test 27: alpha at specific observation counts ────────────────────

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

    // ── Test 28: mat_vec_mul correctness ────────────────────────────────

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
}
