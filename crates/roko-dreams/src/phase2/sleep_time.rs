//! Phase 2 sleep-time compute and dream budget allocation.

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Dream compute budget (DREAM-12) ─────────────────────────────────────

/// Per-phase budget distribution for dream compute (DREAM-12).
///
/// Allocates the total dream budget across dream phases:
/// Hypnagogia 10%, NREM 30%, REM 50%, Integration 0%, Evolution 10%.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamComputeBudget {
    /// Total daily inference budget in USD.
    pub inference_daily_usd: f64,
    /// Fraction of daily budget allocated to dreaming (default 0.15 = 15%).
    pub dream_fraction: f64,
    /// Per-phase budget fractions (must sum to 1.0).
    pub phase_allocations: PhaseAllocations,
}

impl Default for DreamComputeBudget {
    fn default() -> Self {
        Self {
            inference_daily_usd: 10.0,
            dream_fraction: 0.15,
            phase_allocations: PhaseAllocations::default(),
        }
    }
}

impl DreamComputeBudget {
    /// Total dream budget in USD for this cycle.
    #[must_use]
    pub fn total_dream_budget_usd(&self) -> f64 {
        self.inference_daily_usd * self.dream_fraction
    }

    /// Budget for a specific phase.
    #[must_use]
    pub fn phase_budget_usd(&self, phase: DreamPhaseKind) -> f64 {
        let fraction = match phase {
            DreamPhaseKind::Hypnagogia => self.phase_allocations.hypnagogia,
            DreamPhaseKind::Nrem => self.phase_allocations.nrem,
            DreamPhaseKind::Rem => self.phase_allocations.rem,
            DreamPhaseKind::Integration => self.phase_allocations.integration,
            DreamPhaseKind::Evolution => self.phase_allocations.evolution,
        };
        self.total_dream_budget_usd() * fraction
    }
}

/// Per-phase allocation fractions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseAllocations {
    /// Hypnagogia fraction (default 0.10).
    pub hypnagogia: f64,
    /// NREM fraction (default 0.30).
    pub nrem: f64,
    /// REM fraction (default 0.50).
    pub rem: f64,
    /// Integration fraction (default 0.00 — pure computation).
    pub integration: f64,
    /// Evolution fraction (default 0.10).
    pub evolution: f64,
}

impl Default for PhaseAllocations {
    fn default() -> Self {
        Self {
            hypnagogia: 0.10,
            nrem: 0.30,
            rem: 0.50,
            integration: 0.0,
            evolution: 0.10,
        }
    }
}

/// Dream phase kinds for budget allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamPhaseKind {
    /// Hypnagogia (creative onset).
    Hypnagogia,
    /// NREM (replay).
    Nrem,
    /// REM (imagination).
    Rem,
    /// Integration (pure computation, no model calls).
    Integration,
    /// Evolution (MAP-Elites).
    Evolution,
}

impl DreamPhaseKind {
    /// Map each dream phase to a recommended model tier via `CascadeRouter`.
    ///
    /// Per the DREAM-12 spec:
    /// - NREM uses T0 (Fast/cheap) for replay analysis
    /// - Hypnagogia uses T0 (Fast) for fragment generation
    /// - REM uses T1 (Standard/capable) for imagination
    /// - Evolution uses T0 (Fast) for mutation evaluation
    /// - Integration uses no model (pure computation) -- returns `None`
    #[must_use]
    pub fn model_tier(self) -> Option<roko_core::agent::ModelTier> {
        match self {
            Self::Nrem => Some(roko_core::agent::ModelTier::Fast),
            Self::Hypnagogia => Some(roko_core::agent::ModelTier::Fast),
            Self::Rem => Some(roko_core::agent::ModelTier::Standard),
            Self::Evolution => Some(roko_core::agent::ModelTier::Fast),
            Self::Integration => None,
        }
    }
}

/// Tracks spend within a dream cycle for budget enforcement (DREAM-12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamBudgetTracker {
    /// Budget configuration.
    pub budget: DreamComputeBudget,
    /// Per-phase spend so far.
    pub phase_spend: std::collections::HashMap<String, f64>,
    /// Total spend so far.
    pub total_spend_usd: f64,
}

impl DreamBudgetTracker {
    /// Create a new tracker from a budget configuration.
    #[must_use]
    pub fn new(budget: DreamComputeBudget) -> Self {
        Self {
            budget,
            phase_spend: std::collections::HashMap::new(),
            total_spend_usd: 0.0,
        }
    }

    /// Record spend for a phase. Returns true if the phase budget is exhausted.
    pub fn record_spend(&mut self, phase: DreamPhaseKind, amount_usd: f64) -> bool {
        let key = format!("{phase:?}");
        *self.phase_spend.entry(key).or_default() += amount_usd;
        self.total_spend_usd += amount_usd;
        self.is_phase_exhausted(phase)
    }

    /// Check if a phase's budget is exhausted.
    #[must_use]
    pub fn is_phase_exhausted(&self, phase: DreamPhaseKind) -> bool {
        let key = format!("{phase:?}");
        let spent = self.phase_spend.get(&key).copied().unwrap_or(0.0);
        spent >= self.budget.phase_budget_usd(phase)
    }

    /// Check if the total dream budget is exhausted.
    #[must_use]
    pub fn is_total_exhausted(&self) -> bool {
        self.total_spend_usd >= self.budget.total_dream_budget_usd()
    }
}

/// Sleepwalker mode: reduced-capability state during dreaming (DREAM-12).
///
/// When active, the agent responds only to urgent interrupts (process
/// supervisor events, critical errors) via a minimal perception-decision loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SleepwalkerMode {
    /// Normal operation — full agent capabilities.
    Awake,
    /// Dreaming — only urgent signals processed.
    Dreaming {
        /// Types of signals that can wake the agent from dreaming.
        urgent_signal_types: Vec<String>,
    },
}

impl Default for SleepwalkerMode {
    fn default() -> Self {
        Self::Awake
    }
}

impl SleepwalkerMode {
    /// Create a dreaming state with default urgent signal types.
    #[must_use]
    pub fn dreaming() -> Self {
        Self::Dreaming {
            urgent_signal_types: vec![
                "process_crash".to_string(),
                "critical_error".to_string(),
                "operator_interrupt".to_string(),
            ],
        }
    }

    /// Check if a signal type is urgent enough to interrupt dreaming.
    #[must_use]
    pub fn is_urgent(&self, signal_type: &str) -> bool {
        match self {
            Self::Awake => true,
            Self::Dreaming {
                urgent_signal_types,
            } => urgent_signal_types
                .iter()
                .any(|t| t.eq_ignore_ascii_case(signal_type)),
        }
    }

    /// Whether the agent is currently dreaming.
    #[must_use]
    pub const fn is_dreaming(&self) -> bool {
        matches!(self, Self::Dreaming { .. })
    }
}

/// Sleep-time pre-computation settings for predictable query patterns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepTimePrecompute {
    /// Whether to generate pre-computed summaries during NREM.
    pub enable_precompute: bool,
    /// Maximum summary token count per cached chunk.
    pub max_summary_tokens: usize,
    /// Minimum predictability required before pre-computing.
    pub predictability_threshold: f64,
    /// Maximum cached summaries to retain.
    pub max_cached_summaries: usize,
    /// Cache time-to-live in hours.
    pub cache_ttl_hours: u64,
    /// Whether savings should be measured and logged.
    pub measure_savings: bool,
}

impl Default for SleepTimePrecompute {
    fn default() -> Self {
        Self {
            enable_precompute: true,
            max_summary_tokens: 512,
            predictability_threshold: 0.60,
            max_cached_summaries: 100,
            cache_ttl_hours: 24,
            measure_savings: true,
        }
    }
}

impl SleepTimePrecompute {
    /// Construct the documented default pre-compute settings.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            enable_precompute: true,
            max_summary_tokens: 512,
            predictability_threshold: 0.60,
            max_cached_summaries: 100,
            cache_ttl_hours: 24,
            measure_savings: true,
        }
    }
}

/// Pre-computed summary for a recurring query pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrecomputedSummary {
    /// Stable summary identifier.
    pub id: String,
    /// Recurring query pattern that triggered the summary.
    pub query_pattern: String,
    /// Cached summary content.
    pub summary_content: String,
    /// Token count of the cached summary.
    pub token_count: usize,
    /// Predictability score for the query pattern.
    pub predictability_score: f64,
    /// Time at which the summary was created.
    pub created_at: DateTime<Utc>,
    /// Expiration time for the summary.
    pub expires_at: DateTime<Utc>,
    /// Number of times the summary has been used.
    pub times_used: usize,
    /// Estimated tokens saved by caching the summary.
    pub estimated_tokens_saved: usize,
}

impl PrecomputedSummary {
    /// Construct a pre-computed summary record with neutral stub metadata.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        query_pattern: impl Into<String>,
        summary_content: impl Into<String>,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        let summary_content = summary_content.into();
        Self {
            id: id.into(),
            query_pattern: query_pattern.into(),
            token_count: summary_content.split_whitespace().count(),
            summary_content,
            predictability_score: 0.0,
            created_at,
            expires_at,
            times_used: 0,
            estimated_tokens_saved: 0,
        }
    }

    /// Check whether the summary has expired at the supplied time.
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dream_budget_total_and_phase() {
        let budget = DreamComputeBudget {
            inference_daily_usd: 20.0,
            dream_fraction: 0.15,
            phase_allocations: PhaseAllocations::default(),
        };
        assert!((budget.total_dream_budget_usd() - 3.0).abs() < 1e-9);
        assert!((budget.phase_budget_usd(DreamPhaseKind::Nrem) - 0.9).abs() < 1e-9);
        assert!((budget.phase_budget_usd(DreamPhaseKind::Rem) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn budget_tracker_records_spend_and_exhaustion() {
        let budget = DreamComputeBudget {
            inference_daily_usd: 10.0,
            dream_fraction: 0.10,
            phase_allocations: PhaseAllocations::default(),
        };
        let mut tracker = DreamBudgetTracker::new(budget);
        assert!(!tracker.is_total_exhausted());

        // NREM budget = 1.0 * 0.30 = 0.30
        tracker.record_spend(DreamPhaseKind::Nrem, 0.25);
        assert!(!tracker.is_phase_exhausted(DreamPhaseKind::Nrem));

        tracker.record_spend(DreamPhaseKind::Nrem, 0.10);
        assert!(tracker.is_phase_exhausted(DreamPhaseKind::Nrem));
    }

    #[test]
    fn sleepwalker_mode_filters_urgent_signals() {
        let dreaming = SleepwalkerMode::dreaming();
        assert!(dreaming.is_dreaming());
        assert!(dreaming.is_urgent("critical_error"));
        assert!(dreaming.is_urgent("CRITICAL_ERROR"));
        assert!(!dreaming.is_urgent("normal_task"));

        let awake = SleepwalkerMode::Awake;
        assert!(!awake.is_dreaming());
        assert!(awake.is_urgent("anything"));
    }

    #[test]
    fn phase_kind_model_tier_mapping() {
        use roko_core::agent::ModelTier;

        assert_eq!(DreamPhaseKind::Nrem.model_tier(), Some(ModelTier::Fast));
        assert_eq!(DreamPhaseKind::Hypnagogia.model_tier(), Some(ModelTier::Fast));
        assert_eq!(DreamPhaseKind::Rem.model_tier(), Some(ModelTier::Standard));
        assert_eq!(DreamPhaseKind::Evolution.model_tier(), Some(ModelTier::Fast));
        assert_eq!(DreamPhaseKind::Integration.model_tier(), None);
    }

    #[test]
    fn precomputed_summary_expiry() {
        let now = Utc::now();
        let future = now + chrono::Duration::hours(1);
        let past = now - chrono::Duration::hours(1);

        let summary = PrecomputedSummary::new("s1", "pattern", "content", now, future);
        assert!(!summary.is_expired(now));
        assert!(summary.is_expired(future));

        let expired = PrecomputedSummary::new("s2", "pattern", "content", past, now);
        assert!(expired.is_expired(now));
    }
}
