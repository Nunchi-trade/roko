//! Intervention classification and policy trait.
//!
//! Roko's conductor uses a simplified 3-level intervention model
//! (§11.2: Continue / Restart / Fail). This module defines the
//! [`InterventionPolicy`] trait that watcher outputs are mapped through
//! to produce a final [`ConductorDecision`].
//!
//! The [`BanditPolicy`] blends learned Thompson Sampling recommendations
//! from [`ConductorBandit`](roko_learn::conductor::ConductorBandit) with
//! the static [`WorstSeverityPolicy`] at a 65/35 ratio after a 50-observation
//! warmup period.

use parking_lot::Mutex;
use roko_core::{ConductorDecision, Context, Engram};
use roko_learn::conductor::{ConductorAction, ConductorBandit, ConductorState, ErrorPattern};
use serde::{Deserialize, Serialize};

// ─── Severity classification ────────────────────────────────────────────

/// Severity of a detected anomaly.
///
/// Watchers classify their findings by severity; the conductor then maps
/// severity to a [`ConductorDecision`] via the escalation policy.
///
/// The three levels mirror `ConductorDecision` semantics:
/// - `Info` = log but continue
/// - `Warning` = restart the phase
/// - `Critical` = abort the plan
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Informational — logged, no action taken.
    Info = 0,
    /// Warning — triggers a restart of the current phase.
    Warning = 1,
    /// Critical — triggers terminal failure.
    Critical = 2,
}

impl Severity {
    /// Convert severity to the corresponding conductor decision.
    #[must_use]
    pub fn to_decision(self, watcher: &str, reason: &str) -> ConductorDecision {
        match self {
            Self::Info => ConductorDecision::cont(),
            Self::Warning => ConductorDecision::restart(watcher, reason),
            Self::Critical => {
                ConductorDecision::fail(watcher, roko_core::FailureKind::Other(reason.to_owned()))
            }
        }
    }
}

// ─── WatcherOutput ──────────────────────────────────────────────────────

/// Structured output from a watcher: what it detected and how severe it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatcherOutput {
    /// Name of the watcher that produced this finding.
    pub watcher: String,
    /// Severity of the detected anomaly.
    pub severity: Severity,
    /// Human-readable description.
    pub description: String,
    /// Optional metric value (e.g. "`pass_rate=0.82`").
    pub metric: Option<f64>,
}

impl WatcherOutput {
    /// Create a new watcher output.
    #[must_use]
    pub fn new(
        watcher: impl Into<String>,
        severity: Severity,
        description: impl Into<String>,
    ) -> Self {
        Self {
            watcher: watcher.into(),
            severity,
            description: description.into(),
            metric: None,
        }
    }

    /// Attach a metric value.
    #[must_use]
    pub const fn with_metric(mut self, v: f64) -> Self {
        self.metric = Some(v);
        self
    }

    /// Convert to a conductor decision.
    #[must_use]
    pub fn to_decision(&self) -> ConductorDecision {
        self.severity.to_decision(&self.watcher, &self.description)
    }
}

// ─── InterventionPolicy trait ───────────────────────────────────────────

/// Maps a sequence of watcher outputs to a single conductor decision.
///
/// The default implementation uses worst-severity-wins: if any watcher
/// emitted `Critical`, the decision is `Fail`. If any emitted `Warning`,
/// the decision is `Restart`. Otherwise `Continue`.
pub trait InterventionPolicy: Send + Sync {
    /// Evaluate all watcher outputs and produce a single decision.
    fn evaluate(&self, outputs: &[WatcherOutput], ctx: &Context) -> ConductorDecision;

    /// Human-readable name.
    fn name(&self) -> &str;
}

/// Default escalation policy: worst severity wins.
#[derive(Debug, Clone, Default)]
pub struct WorstSeverityPolicy;

impl InterventionPolicy for WorstSeverityPolicy {
    fn evaluate(&self, outputs: &[WatcherOutput], _ctx: &Context) -> ConductorDecision {
        let worst = outputs.iter().max_by_key(|o| o.severity);
        worst.map_or_else(ConductorDecision::cont, WatcherOutput::to_decision)
    }

    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "worst-severity"
    }
}

// ─── BanditPolicy ────────────────────────────────────────────────────────

/// Minimum total observations across all arms before the bandit is consulted.
const BANDIT_WARMUP_THRESHOLD: u64 = 50;

/// Weight given to the bandit recommendation after warmup (0.65 = 65%).
const BANDIT_BLEND_WEIGHT: f64 = 0.65;

/// Learned intervention policy that blends Thompson Sampling with worst-severity.
///
/// During warmup (< 50 total observations), this delegates entirely to
/// [`WorstSeverityPolicy`]. After warmup, it asks the [`ConductorBandit`]
/// for the best action and blends with the static policy at a 65/35 ratio.
///
/// The blending works as follows:
/// - The bandit selects an action (Continue, Restart, Abort, etc.)
/// - The static policy selects worst-severity decision
/// - If they agree, that's the result
/// - If they disagree, the bandit's recommendation wins 65% of the time
///   (determined by comparing bandit confidence against the threshold)
pub struct BanditPolicy {
    bandit: Mutex<ConductorBandit>,
    fallback: WorstSeverityPolicy,
}

impl std::fmt::Debug for BanditPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BanditPolicy")
            .field("fallback", &self.fallback)
            .finish_non_exhaustive()
    }
}

impl BanditPolicy {
    /// Create a new bandit policy wrapping a given [`ConductorBandit`].
    #[must_use]
    pub fn new(bandit: ConductorBandit) -> Self {
        Self {
            bandit: Mutex::new(bandit),
            fallback: WorstSeverityPolicy,
        }
    }

    /// Create a bandit policy that loads from disk or starts fresh.
    #[must_use]
    pub fn load_or_new(path: &std::path::Path) -> Self {
        Self::new(ConductorBandit::load_or_new(path))
    }

    /// Access the inner bandit (e.g. to record outcomes or persist).
    pub fn with_bandit<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut ConductorBandit) -> R,
    {
        f(&mut self.bandit.lock())
    }

    /// Total observations across all arms.
    fn total_observations(&self) -> u64 {
        self.bandit.lock().total_observations()
    }

    /// Whether the bandit has accumulated enough data to be consulted.
    fn is_warmed_up(&self) -> bool {
        self.total_observations() >= BANDIT_WARMUP_THRESHOLD
    }

    /// Build a [`ConductorState`] from watcher outputs for the bandit.
    fn state_from_outputs(outputs: &[WatcherOutput]) -> ConductorState {
        let worst_severity = outputs.iter().map(|o| o.severity).max();
        let consecutive_failures = outputs
            .iter()
            .filter(|o| o.severity >= Severity::Warning)
            .count() as u32;

        let error_pattern = outputs
            .iter()
            .filter(|o| o.severity >= Severity::Warning)
            .find_map(|o| match o.watcher.as_str() {
                "compile-fail-repeat" => Some(ErrorPattern::Compile),
                "test-failure-budget" => Some(ErrorPattern::Test),
                "iteration-loop" | "stuck-pattern" => Some(ErrorPattern::LoopDetected),
                "context-window-pressure" => Some(ErrorPattern::ContextOverflow),
                "time-overrun" => Some(ErrorPattern::Timeout),
                "cost-overrun" => Some(ErrorPattern::RateLimit),
                _ => None,
            })
            .unwrap_or(ErrorPattern::Unknown);

        ConductorState {
            iteration: consecutive_failures.max(1),
            consecutive_failures,
            error_pattern,
            elapsed_ms: 0,
            cost_so_far_usd: 0.0,
            model_tier: "standard".to_string(),
            task_complexity: match worst_severity {
                Some(Severity::Critical) => "architectural".to_string(),
                Some(Severity::Warning) => "focused".to_string(),
                _ => "mechanical".to_string(),
            },
        }
    }
}

impl InterventionPolicy for BanditPolicy {
    fn evaluate(&self, outputs: &[WatcherOutput], ctx: &Context) -> ConductorDecision {
        let static_decision = self.fallback.evaluate(outputs, ctx);

        // During warmup, use the static policy only.
        if !self.is_warmed_up() {
            return static_decision;
        }

        // Ask the bandit for its recommendation.
        let state = Self::state_from_outputs(outputs);
        let bandit_action = self.bandit.lock().select_action(&state);

        // Map bandit action to a ConductorDecision.
        let bandit_decision = match bandit_action {
            ConductorAction::Continue | ConductorAction::InjectHint(_) => {
                ConductorDecision::cont()
            }
            ConductorAction::SwitchModel => {
                // SwitchModel is a soft intervention -- continue but signal escalation.
                ConductorDecision::cont()
            }
            ConductorAction::Restart => {
                ConductorDecision::restart("bandit-policy", "learned: restart recommended")
            }
            ConductorAction::Abort => ConductorDecision::fail(
                "bandit-policy",
                roko_core::FailureKind::Other("learned: abort recommended".to_owned()),
            ),
        };

        // Blend: if both agree, use that. Otherwise, the bandit wins at BANDIT_BLEND_WEIGHT.
        if static_decision.label() == bandit_decision.label() {
            return static_decision;
        }

        // Use a deterministic blend based on context timestamp to avoid
        // non-determinism in tests while still varying over time.
        let blend_value = (ctx.now_ms as f64 * 0.001).fract();
        if blend_value < BANDIT_BLEND_WEIGHT {
            bandit_decision
        } else {
            static_decision
        }
    }

    fn name(&self) -> &str {
        "bandit-policy"
    }
}

/// Convenience: convert a batch of `WatcherOutput`s into signals for emission.
pub fn outputs_to_signals(outputs: &[WatcherOutput]) -> Vec<Engram> {
    outputs
        .iter()
        .filter_map(|o| {
            if o.severity == Severity::Info {
                return None;
            }
            let body = roko_core::Body::from_json(o).ok()?;
            Some(
                Engram::builder(roko_core::Kind::Custom(format!(
                    "conductor:alert:{}",
                    o.watcher
                )))
                .body(body)
                .tag("watcher", &o.watcher)
                .tag(
                    "severity",
                    match o.severity {
                        Severity::Info => "info",
                        Severity::Warning => "warning",
                        Severity::Critical => "critical",
                    },
                )
                .build(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Critical);
    }

    #[test]
    fn severity_to_decision_info_is_continue() {
        let d = Severity::Info.to_decision("test", "all good");
        assert!(d.is_continue());
    }

    #[test]
    fn severity_to_decision_warning_is_restart() {
        let d = Severity::Warning.to_decision("test", "slow progress");
        assert_eq!(d.label(), "restart");
    }

    #[test]
    fn severity_to_decision_critical_is_fail() {
        let d = Severity::Critical.to_decision("test", "stuck");
        assert!(d.is_terminal());
    }

    #[test]
    fn watcher_output_to_decision() {
        let o = WatcherOutput::new("ghost-turn", Severity::Warning, "no output for 60s");
        let d = o.to_decision();
        assert_eq!(d.label(), "restart");
    }

    #[test]
    fn worst_severity_policy_empty_is_continue() {
        let policy = WorstSeverityPolicy;
        let d = policy.evaluate(&[], &Context::at(0));
        assert!(d.is_continue());
    }

    #[test]
    fn worst_severity_policy_picks_worst() {
        let policy = WorstSeverityPolicy;
        let outputs = vec![
            WatcherOutput::new("a", Severity::Info, "ok"),
            WatcherOutput::new("b", Severity::Warning, "slow"),
            WatcherOutput::new("c", Severity::Info, "ok too"),
        ];
        let d = policy.evaluate(&outputs, &Context::at(0));
        assert_eq!(d.label(), "restart");
    }

    #[test]
    fn worst_severity_policy_critical_wins() {
        let policy = WorstSeverityPolicy;
        let outputs = vec![
            WatcherOutput::new("a", Severity::Warning, "slow"),
            WatcherOutput::new("b", Severity::Critical, "stuck"),
        ];
        let d = policy.evaluate(&outputs, &Context::at(0));
        assert!(d.is_terminal());
    }

    #[test]
    fn outputs_to_signals_filters_info() {
        let outputs = vec![
            WatcherOutput::new("a", Severity::Info, "ok"),
            WatcherOutput::new("b", Severity::Warning, "slow"),
        ];
        let signals = outputs_to_signals(&outputs);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].tag("watcher"), Some("b"));
        assert_eq!(
            signals[0].kind,
            roko_core::Kind::Custom("conductor:alert:b".into())
        );
    }

    #[test]
    fn watcher_output_serde_roundtrip() {
        let o = WatcherOutput::new("test", Severity::Warning, "desc").with_metric(0.85);
        let json = serde_json::to_string(&o).expect("serialize");
        let decoded: WatcherOutput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(o, decoded);
    }

    // ── BanditPolicy tests ────────────────────────────────────────────

    #[test]
    fn bandit_policy_delegates_during_warmup() {
        let bandit = ConductorBandit::new();
        let policy = BanditPolicy::new(bandit);

        // Fresh bandit has 0 observations -> warmup -> delegates to WorstSeverity.
        assert!(!policy.is_warmed_up());

        let outputs = vec![WatcherOutput::new(
            "ghost-turn",
            Severity::Warning,
            "no progress",
        )];
        let d = policy.evaluate(&outputs, &Context::at(0));
        assert_eq!(d.label(), "restart");
    }

    #[test]
    fn bandit_policy_empty_outputs_is_continue() {
        let policy = BanditPolicy::new(ConductorBandit::new());
        let d = policy.evaluate(&[], &Context::at(0));
        assert!(d.is_continue());
    }

    #[test]
    fn bandit_policy_state_from_outputs_maps_watchers() {
        let outputs = vec![
            WatcherOutput::new("compile-fail-repeat", Severity::Warning, "3 fails"),
            WatcherOutput::new("cost-overrun", Severity::Info, "fine"),
        ];
        let state = BanditPolicy::state_from_outputs(&outputs);
        assert_eq!(state.error_pattern, ErrorPattern::Compile);
        assert_eq!(state.consecutive_failures, 1);
        assert_eq!(state.task_complexity, "focused");
    }

    #[test]
    fn bandit_policy_state_critical_maps_architectural() {
        let outputs = vec![WatcherOutput::new(
            "stuck-pattern",
            Severity::Critical,
            "stuck",
        )];
        let state = BanditPolicy::state_from_outputs(&outputs);
        assert_eq!(state.task_complexity, "architectural");
        assert_eq!(state.error_pattern, ErrorPattern::LoopDetected);
    }

    #[test]
    fn bandit_policy_name() {
        let policy = BanditPolicy::new(ConductorBandit::new());
        assert_eq!(policy.name(), "bandit-policy");
    }

    #[test]
    fn bandit_policy_with_bandit_access() {
        let policy = BanditPolicy::new(ConductorBandit::new());
        let obs = policy.with_bandit(|b| b.total_observations());
        assert_eq!(obs, 0);
    }
}
