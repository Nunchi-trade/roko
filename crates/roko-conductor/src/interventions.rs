//! Intervention classification and policy trait.
//!
//! Roko's conductor uses a simplified 3-level intervention model
//! (§11.2: Continue / Restart / Fail). This module defines the
//! [`InterventionPolicy`] trait that watcher outputs are mapped through
//! to produce a final [`ConductorDecision`].

use roko_core::{ConductorDecision, Context, Signal};
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

/// Convenience: convert a batch of `WatcherOutput`s into signals for emission.
pub fn outputs_to_signals(outputs: &[WatcherOutput]) -> Vec<Signal> {
    outputs
        .iter()
        .filter_map(|o| {
            if o.severity == Severity::Info {
                return None;
            }
            let body = roko_core::Body::from_json(o).ok()?;
            Some(
                Signal::builder(roko_core::Kind::Custom(format!(
                    "conductor:alert:{}",
                    o.watcher
                )))
                    .body(body)
                    .tag("watcher", &o.watcher)
                    .tag("severity", format!("{:?}", o.severity))
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
        assert_eq!(signals[0].kind, roko_core::Kind::Custom("conductor:alert:b".into()));
    }

    #[test]
    fn watcher_output_serde_roundtrip() {
        let o = WatcherOutput::new("test", Severity::Warning, "desc").with_metric(0.85);
        let json = serde_json::to_string(&o).expect("serialize");
        let decoded: WatcherOutput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(o, decoded);
    }
}
