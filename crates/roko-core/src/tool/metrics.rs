//! Evaluation metrics for tool calls (§36.p, parity items 36.111–36.119).
//!
//! **Why structured metrics matter**: ToolRL (Qian et al., NeurIPS 2025)
//! showed that **fine-grained rewards are critical** for tool use —
//! coarse answer-matching fails. ComplexFuncBench introduced PHR / PMR;
//! the Galileo Agent Leaderboard introduced TSQ; PALADIN added
//! error-recognition/correction/recovery rates. This module bakes the
//! canonical metric set into Roko so every tool call feeds the
//! continuous-tuning loops (§35 Loop F/G/H) with the right signal.
//!
//! # Types
//!
//! - [`ToolMetrics`] — per-(tool × model × role × format) aggregated stats
//! - [`MetricsKey`] — aggregation key
//! - [`MetricsSink`] — runtime-agnostic emission trait
//! - [`RewardConfig`] — weights for [`compute_reward`]
//! - [`compute_reward`] — standard composite reward function
//!
//! Aggregation logic (online rolling windows, persistence) lives in
//! `roko-std` and `roko-learn`.

use serde::{Deserialize, Serialize};

use super::format::ToolFormat;
use super::trace::{FailureKind, ToolOutcome};
use crate::AgentRole;

// ─── MetricsKey ──────────────────────────────────────────────────────────

/// Aggregation key — one distinct (tool, model, role, format) quadruple.
///
/// The bandit (§36.l) and the Optimizer TUI (§36.86) both partition
/// metrics on this key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MetricsKey {
    /// Canonical tool name.
    pub tool: String,
    /// Model slug (as-recorded).
    pub model: String,
    /// The role whose agent emitted the call.
    pub role: AgentRole,
    /// Format used for the call.
    pub format: ToolFormat,
}

impl MetricsKey {
    /// Construct a metrics key.
    #[must_use]
    pub fn new(
        tool: impl Into<String>,
        model: impl Into<String>,
        role: AgentRole,
        format: ToolFormat,
    ) -> Self {
        Self {
            tool: tool.into(),
            model: model.into(),
            role,
            format,
        }
    }
}

// ─── ToolMetrics ──────────────────────────────────────────────────────────

/// Aggregated quality metrics for one [`MetricsKey`].
///
/// All rates are in `[0, 1]`. `samples` is the number of observed calls.
/// When `samples == 0`, all rates are `0.0` by convention.
///
/// | Field | Paper | Definition |
/// |---|---|---|
/// | `phr` | ComplexFuncBench | Parameter Hallucination Rate: fraction of calls with an arg not in the schema |
/// | `pmr` | ComplexFuncBench | Parameter Missing Rate: fraction of calls missing a required arg |
/// | `tsq` | Galileo | Tool Selection Quality (composite of correctness + argument quality) |
/// | `schema_compliance` | JSONSchemaBench | Fraction of calls that pass JSON-schema validation |
/// | `arg_validity` | — | Fraction of calls with type-valid arguments (post-schema) |
/// | `selection_accuracy` | BFCL | Fraction where the model picked the correct tool |
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolMetrics {
    /// Parameter Hallucination Rate — fraction of calls with an arg
    /// not in the schema.
    pub phr: f32,
    /// Parameter Missing Rate — fraction of calls missing a required arg.
    pub pmr: f32,
    /// Tool Selection Quality — composite quality score.
    pub tsq: f32,
    /// Fraction of calls passing JSON-schema validation.
    pub schema_compliance: f32,
    /// Fraction of calls with type-valid arguments.
    pub arg_validity: f32,
    /// Fraction of calls where the selected tool was correct for the task.
    pub selection_accuracy: f32,
    /// Number of samples contributing to these rates.
    pub samples: u32,
}

impl ToolMetrics {
    /// Empty (zero-sample) metrics.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            phr: 0.0,
            pmr: 0.0,
            tsq: 0.0,
            schema_compliance: 0.0,
            arg_validity: 0.0,
            selection_accuracy: 0.0,
            samples: 0,
        }
    }

    /// Merge a new sample's per-call indicators into rolling averages.
    ///
    /// `phr_hit`, `pmr_hit`, `schema_ok`, `args_ok`, `selected_correct`
    /// are the boolean outcomes for this single call; `tsq_score` is a
    /// value in `[0, 1]` computed by a domain scorer (e.g.
    /// [`galileo_tsq`]). The merge computes incremental means.
    #[allow(
        clippy::cast_precision_loss,
        clippy::too_many_arguments,
        clippy::fn_params_excessive_bools,
        clippy::similar_names,
        clippy::suboptimal_flops
    )]
    pub fn observe(
        &mut self,
        phr_hit: bool,
        pmr_hit: bool,
        schema_ok: bool,
        args_ok: bool,
        selected_correct: bool,
        tsq_score: f32,
    ) {
        let total = self.samples as f32;
        let next_total = total + 1.0;
        let inc = |avg: f32, new: f32| (avg * total + new) / next_total;
        self.phr = inc(self.phr, if phr_hit { 1.0 } else { 0.0 });
        self.pmr = inc(self.pmr, if pmr_hit { 1.0 } else { 0.0 });
        self.schema_compliance = inc(self.schema_compliance, if schema_ok { 1.0 } else { 0.0 });
        self.arg_validity = inc(self.arg_validity, if args_ok { 1.0 } else { 0.0 });
        self.selection_accuracy = inc(
            self.selection_accuracy,
            if selected_correct { 1.0 } else { 0.0 },
        );
        self.tsq = inc(self.tsq, tsq_score.clamp(0.0, 1.0));
        self.samples = self.samples.saturating_add(1);
    }

    /// Health score in `[0, 1]` — higher is better.
    ///
    /// Composite of `schema_compliance * arg_validity * selection_accuracy *
    /// (1 - phr) * (1 - pmr)`. Used by the Optimizer TUI to color-code
    /// per-key status.
    #[must_use]
    pub fn health(&self) -> f32 {
        if self.samples == 0 {
            return 0.0;
        }
        self.schema_compliance
            * self.arg_validity
            * self.selection_accuracy
            * (1.0 - self.phr)
            * (1.0 - self.pmr)
    }
}

// ─── galileo_tsq ──────────────────────────────────────────────────────────

/// Galileo-style Tool Selection Quality score for a single call.
///
/// Inputs: four independent `[0, 1]` scores averaged with fixed weights
/// (selection 0.4, schema 0.2, args 0.3, completion 0.1). Any cheaper
/// scoring function can replace this at the call site; it's provided as
/// a sensible default.
#[must_use]
pub fn galileo_tsq(
    selection_score: f32,
    schema_score: f32,
    arg_score: f32,
    completion_score: f32,
) -> f32 {
    let s = selection_score.clamp(0.0, 1.0);
    let sc = schema_score.clamp(0.0, 1.0);
    let a = arg_score.clamp(0.0, 1.0);
    let c = completion_score.clamp(0.0, 1.0);
    s.mul_add(0.4, sc.mul_add(0.2, a.mul_add(0.3, c * 0.1)))
}

// ─── compute_reward ───────────────────────────────────────────────────────

/// Weights + budgets for [`compute_reward`].
///
/// Defaults mirror the §36.119 default composition:
/// `reward = success × (1 − 0.3 × latency_norm − 0.2 × cost_norm −
///                      0.1 × recovery_norm)`.
/// Overrides are allowed via `[metrics.reward]` config.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RewardConfig {
    /// Latency budget (ms) that maps to `latency_norm = 1.0`.
    pub latency_budget_ms: u64,
    /// Cost budget (USD) that maps to `cost_norm = 1.0`.
    pub cost_budget_usd: f32,
    /// Max recovery attempts that maps to `recovery_norm = 1.0`.
    pub max_recovery_attempts: u8,
    /// Weight applied to normalized latency penalty.
    pub latency_weight: f32,
    /// Weight applied to normalized cost penalty.
    pub cost_weight: f32,
    /// Weight applied to normalized recovery penalty.
    pub recovery_weight: f32,
    /// Additional flat penalty applied to failures in `[0, 1]` on top of
    /// zero. Example: `hallucinated_param_penalty: 0.1` subtracts 0.1
    /// from a failure's already-zero base to produce `-0.1`.
    pub hallucination_penalty: f32,
}

impl Default for RewardConfig {
    fn default() -> Self {
        Self {
            latency_budget_ms: 30_000,
            cost_budget_usd: 0.10,
            max_recovery_attempts: 5,
            latency_weight: 0.3,
            cost_weight: 0.2,
            recovery_weight: 0.1,
            hallucination_penalty: 0.1,
        }
    }
}

/// Compose a bandit reward in `[-hallucination_penalty, 1.0]` from a
/// [`ToolOutcome`] and a [`RewardConfig`].
///
/// ```text
/// success = 1.0, cost_usd = 0.01, latency = 100ms, recoveries = 0
///   → reward ≈ 1 - 0.3 * (100/30000) - 0.2 * (0.01/0.10) - 0.1 * 0
///           ≈ 1 - 0.001 - 0.02 - 0 = 0.979
///
/// failure (malformed_json) → reward = 0 - 0.1 = -0.1
/// ```
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn compute_reward(outcome: &ToolOutcome, cfg: &RewardConfig) -> f32 {
    if !outcome.success {
        // Failure floor: small negative penalty for hallucination-class
        // errors so the bandit actively avoids them.
        let penalty = match outcome.failure {
            Some(
                FailureKind::HallucinatedParam
                | FailureKind::UnknownTool
                | FailureKind::MalformedJson,
            ) => cfg.hallucination_penalty,
            _ => 0.0,
        };
        return -penalty;
    }
    let latency_norm = (outcome.latency_ms as f32 / cfg.latency_budget_ms as f32).clamp(0.0, 1.0);
    let cost_norm = (outcome.cost_usd / cfg.cost_budget_usd).clamp(0.0, 1.0);
    let recovery_norm = (f32::from(outcome.recovery_attempts)
        / f32::from(cfg.max_recovery_attempts))
    .clamp(0.0, 1.0);
    let penalty = cfg.latency_weight.mul_add(
        latency_norm,
        cfg.cost_weight
            .mul_add(cost_norm, cfg.recovery_weight * recovery_norm),
    );
    (1.0 - penalty).clamp(0.0, 1.0)
}

// ─── MetricsSink trait ────────────────────────────────────────────────────

/// Sink for per-call metrics snapshots.
///
/// Implementors (in roko-std / roko-fs): `JsonlMetricsSink` (persistent),
/// `InMemoryMetricsSink` (test helper), `NoopMetricsSink` (default).
pub trait MetricsSink: Send + Sync {
    /// Record a per-call metrics snapshot. Called after each tool call
    /// with the updated aggregate for the key.
    fn record(&self, key: &MetricsKey, metrics: &ToolMetrics);
}

/// No-op metrics sink — drops every record.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopMetricsSink;

impl MetricsSink for NoopMetricsSink {
    fn record(&self, _key: &MetricsKey, _metrics: &ToolMetrics) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentRole;

    fn key() -> MetricsKey {
        MetricsKey::new(
            "read_file",
            "claude-sonnet-4-5",
            AgentRole::Implementer,
            ToolFormat::AnthropicBlocks,
        )
    }

    #[test]
    fn empty_metrics_zero() {
        let m = ToolMetrics::empty();
        assert_eq!(m.samples, 0);
        assert!(m.phr.abs() < f32::EPSILON);
        assert!(m.tsq.abs() < f32::EPSILON);
        assert!(m.health().abs() < f32::EPSILON);
    }

    #[test]
    fn metrics_observe_computes_incremental_averages() {
        let mut m = ToolMetrics::empty();
        m.observe(false, false, true, true, true, 0.8);
        m.observe(false, false, true, true, true, 1.0);
        m.observe(true, false, false, true, false, 0.0);
        assert_eq!(m.samples, 3);
        // phr: [0, 0, 1] → 1/3 ≈ 0.333
        assert!((m.phr - 0.333_333).abs() < 0.01);
        // pmr: [0, 0, 0] → 0
        assert!(m.pmr.abs() < f32::EPSILON);
        // schema_compliance: [1, 1, 0] → 2/3 ≈ 0.667
        assert!((m.schema_compliance - 0.666_667).abs() < 0.01);
        // arg_validity: [1, 1, 1] → 1
        assert!((m.arg_validity - 1.0).abs() < 0.01);
        // selection_accuracy: [1, 1, 0] → 2/3
        assert!((m.selection_accuracy - 0.666_667).abs() < 0.01);
        // tsq: avg of 0.8, 1.0, 0.0 = 0.6
        assert!((m.tsq - 0.6).abs() < 0.01);
    }

    #[test]
    fn health_zero_with_no_samples() {
        let m = ToolMetrics::empty();
        assert!(m.health().abs() < f32::EPSILON);
    }

    #[test]
    fn health_composite_product() {
        let mut m = ToolMetrics::empty();
        // 4 good calls, 0 hallucinations.
        for _ in 0..4 {
            m.observe(false, false, true, true, true, 1.0);
        }
        assert!((m.health() - 1.0).abs() < 0.01);
    }

    #[test]
    fn health_degrades_with_hallucinations() {
        let mut m = ToolMetrics::empty();
        m.observe(true, false, true, true, true, 1.0); // hallucinated
        m.observe(false, false, true, true, true, 1.0);
        // phr = 0.5, others ~= 1, (1-0.5) = 0.5
        assert!(m.health() < 0.6);
        assert!(m.health() > 0.4);
    }

    #[test]
    fn galileo_tsq_weights_sum_to_one() {
        let a = galileo_tsq(1.0, 1.0, 1.0, 1.0);
        assert!((a - 1.0).abs() < f32::EPSILON);
        let b = galileo_tsq(0.0, 0.0, 0.0, 0.0);
        assert!(b.abs() < f32::EPSILON);
    }

    #[test]
    fn galileo_tsq_clamps_inputs() {
        let a = galileo_tsq(2.0, -0.5, 1.5, 0.5);
        assert!((0.0..=1.0).contains(&a));
    }

    #[test]
    fn reward_success_below_budgets_near_one() {
        let cfg = RewardConfig::default();
        let outcome = ToolOutcome::success(100, 0.001);
        let r = compute_reward(&outcome, &cfg);
        assert!(r > 0.97);
        assert!(r <= 1.0);
    }

    #[test]
    fn reward_success_at_budget_hits_weights() {
        let cfg = RewardConfig::default();
        let outcome = ToolOutcome {
            success: true,
            latency_ms: 30_000,
            cost_usd: 0.10,
            recovery_attempts: 5,
            failure: None,
            reward: 0.0,
        };
        let r = compute_reward(&outcome, &cfg);
        // 1 - 0.3 - 0.2 - 0.1 = 0.4
        assert!((r - 0.4).abs() < 0.01);
    }

    #[test]
    fn reward_failure_floor_zero_for_non_hallucination() {
        let cfg = RewardConfig::default();
        let outcome = ToolOutcome::failure(FailureKind::Timeout, 30_000, 0.05);
        let r = compute_reward(&outcome, &cfg);
        assert!(r.abs() < f32::EPSILON);
    }

    #[test]
    fn reward_failure_negative_for_hallucination() {
        let cfg = RewardConfig::default();
        let outcome = ToolOutcome::failure(FailureKind::HallucinatedParam, 500, 0.01);
        let r = compute_reward(&outcome, &cfg);
        assert!((r + 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn reward_negative_also_for_malformed_json_and_unknown_tool() {
        let cfg = RewardConfig::default();
        for k in [FailureKind::MalformedJson, FailureKind::UnknownTool] {
            let outcome = ToolOutcome::failure(k, 500, 0.01);
            let r = compute_reward(&outcome, &cfg);
            assert!(r < 0.0);
        }
    }

    #[test]
    fn reward_config_overridable() {
        let cfg = RewardConfig {
            latency_budget_ms: 10_000,
            cost_budget_usd: 0.02,
            max_recovery_attempts: 2,
            latency_weight: 0.5,
            cost_weight: 0.5,
            recovery_weight: 0.0,
            hallucination_penalty: 0.0,
        };
        let outcome = ToolOutcome::success(10_000, 0.02);
        let r = compute_reward(&outcome, &cfg);
        // 1 - 0.5 - 0.5 = 0.0
        assert!(r.abs() < 0.01);
    }

    #[test]
    fn metrics_key_roundtrips_serde() {
        let k = key();
        let json = serde_json::to_string(&k).unwrap();
        let decoded: MetricsKey = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, k);
    }

    #[test]
    fn tool_metrics_roundtrips_serde() {
        let mut m = ToolMetrics::empty();
        m.observe(false, true, true, true, false, 0.7);
        let json = serde_json::to_string(&m).unwrap();
        let decoded: ToolMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, m);
    }

    #[test]
    fn noop_metrics_sink_accepts_records() {
        let sink = NoopMetricsSink;
        let m = ToolMetrics::empty();
        sink.record(&key(), &m);
    }

    #[test]
    fn reward_never_exceeds_one() {
        let cfg = RewardConfig::default();
        let outcome = ToolOutcome::success(0, 0.0);
        let r = compute_reward(&outcome, &cfg);
        assert!(r <= 1.0);
    }

    #[test]
    fn metrics_samples_saturate_not_overflow() {
        let mut m = ToolMetrics {
            samples: u32::MAX,
            ..ToolMetrics::empty()
        };
        m.observe(false, false, true, true, true, 1.0);
        assert_eq!(m.samples, u32::MAX);
    }
}
