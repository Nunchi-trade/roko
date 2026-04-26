//! VerdictAwareScorer — weights gate verdict signals by recency and severity (GATE-05).
//!
//! This scorer implements the `ScoreFn` trait from `roko-core` and specifically
//! targets engrams of `Kind::GateVerdict`. It assigns higher scores to:
//!
//! 1. **Recent** verdicts (exponential time decay)
//! 2. **Severe** failures (compile errors > lint warnings)
//! 3. **Relevant** verdicts (matching the current task context)
//!
//! Non-verdict engrams receive a neutral score so the scorer composes cleanly
//! with other scorers in a `SumScorer` or `MulScorer` chain.

use roko_core::traits::Score as ScoreFn;
use roko_core::{Context, Engram, Kind, Score};

/// Weights for verdict scoring dimensions.
#[derive(Debug, Clone)]
pub struct VerdictScorerConfig {
    /// Half-life for recency decay in milliseconds (default: 10 minutes).
    pub recency_half_life_ms: f64,
    /// Weight given to recency dimension [0, 1] (default: 0.35).
    pub recency_weight: f32,
    /// Weight given to severity dimension [0, 1] (default: 0.40).
    pub severity_weight: f32,
    /// Weight given to relevance dimension [0, 1] (default: 0.25).
    pub relevance_weight: f32,
}

impl Default for VerdictScorerConfig {
    fn default() -> Self {
        Self {
            recency_half_life_ms: 600_000.0, // 10 minutes
            recency_weight: 0.35,
            severity_weight: 0.40,
            relevance_weight: 0.25,
        }
    }
}

/// ScoreFn that weights `Kind::GateVerdict` engrams by recency, severity, and relevance.
///
/// Designed to be composed with other scorers. Non-verdict engrams receive
/// `Score::ZERO` so they don't interfere in aggregate pipelines.
pub struct VerdictAwareScorer {
    config: VerdictScorerConfig,
}

impl VerdictAwareScorer {
    /// Create a scorer with default weights.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: VerdictScorerConfig::default(),
        }
    }

    /// Create a scorer with custom configuration.
    #[must_use]
    pub fn with_config(config: VerdictScorerConfig) -> Self {
        Self { config }
    }

    /// Compute recency factor using exponential decay.
    ///
    /// Returns 1.0 for current signals, decaying toward 0.0 with the
    /// configured half-life.
    fn recency_factor(&self, signal_ts_ms: i64, now_ms: i64) -> f32 {
        let age_ms = (now_ms - signal_ts_ms).max(0) as f64;
        let decay = (-age_ms * (2.0_f64.ln()) / self.config.recency_half_life_ms).exp();
        decay as f32
    }

    /// Compute severity factor from verdict metadata.
    ///
    /// Severity ordering: compile error (1.0) > test failure (0.8) >
    /// lint warning (0.5) > pass (0.1).
    fn severity_factor(&self, signal: &Engram) -> f32 {
        let gate_name = signal.tag("gate").unwrap_or("");

        let passed = signal
            .tag("verdict_passed")
            .map(|v| v == "true")
            .unwrap_or(true);

        if passed {
            return 0.1; // Passing verdicts have low severity signal.
        }

        // Severity by gate type (higher = more severe).
        match gate_name {
            g if g.contains("compile") => 1.0,
            g if g.contains("test") => 0.8,
            g if g.contains("lint") || g.contains("clippy") => 0.5,
            g if g.contains("diff") => 0.3,
            _ => 0.6, // Unknown gate — moderate severity.
        }
    }

    /// Compute relevance factor by matching tags against current context.
    ///
    /// Checks for matching `task_type`, `crate`, and `task_category` tags
    /// between the verdict engram and the current context.
    fn relevance_factor(&self, signal: &Engram, ctx: &Context) -> f32 {
        let mut relevance: f32 = 0.0;
        let mut checks: f32 = 0.0;

        // Match task_type.
        if let Some(ctx_task_type) = ctx.attr("roko.task_type") {
            checks += 1.0;
            if signal.tag("task_type") == Some(ctx_task_type) {
                relevance += 1.0;
            }
        }

        // Match crate.
        if let Some(ctx_crate) = ctx.attr("roko.crate") {
            checks += 1.0;
            if signal.tag("crate") == Some(ctx_crate) {
                relevance += 1.0;
            }
        }

        // Match task_category.
        if let Some(ctx_category) = ctx.attr("roko.task_category") {
            checks += 1.0;
            if signal.tag("task_category") == Some(ctx_category) {
                relevance += 1.0;
            }
        }

        if checks == 0.0 {
            return 0.5; // No context to match — neutral relevance.
        }
        relevance / checks
    }
}

impl Default for VerdictAwareScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl ScoreFn for VerdictAwareScorer {
    fn score(&self, signal: &Engram, ctx: &Context) -> Score {
        // Only score GateVerdict engrams.
        if signal.kind != Kind::GateVerdict {
            return Score::ZERO;
        }

        let now_ms = ctx.now_ms;
        let recency = self.recency_factor(signal.created_at_ms, now_ms);
        let severity = self.severity_factor(signal);
        let relevance = self.relevance_factor(signal, ctx);

        // Weighted combination.
        let salience = recency * self.config.recency_weight
            + severity * self.config.severity_weight
            + relevance * self.config.relevance_weight;

        Score {
            confidence: recency,
            novelty: if severity > 0.5 { severity * 0.5 } else { 0.0 },
            utility: salience,
            reputation: 1.0, // Verdicts come from our own gates — trusted.
            precision: relevance,
            salience,
            coherence: if severity > 0.5 { 0.8 } else { 0.5 },
        }
    }

    fn name(&self) -> &'static str {
        "verdict_aware_scorer"
    }
}

// ─── Verdict history for CascadeRouter integration ───────────────────────────

/// A recorded verdict observation for verdict-aware routing.
#[derive(Debug, Clone)]
pub struct VerdictRecord {
    /// Which model produced the code that triggered this verdict.
    pub model_slug: String,
    /// Task type (e.g., "implement", "fix", "refactor").
    pub task_type: String,
    /// Target crate.
    pub target_crate: String,
    /// The gate that rendered this verdict.
    pub gate: String,
    /// Whether the verdict passed.
    pub passed: bool,
    /// Unix milliseconds when this verdict was recorded.
    pub timestamp_ms: i64,
}

/// Tracks recent verdicts and computes routing penalties for models with
/// compile failure streaks.
///
/// The router queries this to adjust bandit rewards: a model with a streak
/// of >2 consecutive compile failures on a `(task_type, crate)` pair receives
/// a 0.5x reward penalty, encouraging the router to try a different model.
#[derive(Debug, Clone, Default)]
pub struct VerdictHistory {
    /// Rolling window of recent verdict records.
    records: Vec<VerdictRecord>,
    /// Maximum history size.
    max_records: usize,
}

impl VerdictHistory {
    /// Create a new history with default capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            max_records: 500,
        }
    }

    /// Create a history with custom capacity.
    #[must_use]
    pub fn with_capacity(max_records: usize) -> Self {
        Self {
            records: Vec::new(),
            max_records: max_records.max(10),
        }
    }

    /// Record a verdict observation.
    pub fn record(&mut self, record: VerdictRecord) {
        if self.records.len() >= self.max_records {
            self.records.remove(0);
        }
        self.records.push(record);
    }

    /// Count consecutive compile failures for a model on a `(task_type, crate)` pair.
    ///
    /// Scans backward from the most recent record, counting consecutive compile
    /// failures. Stops at the first non-compile or passing verdict.
    #[must_use]
    pub fn compile_failure_streak(
        &self,
        model_slug: &str,
        task_type: &str,
        target_crate: &str,
    ) -> usize {
        let mut streak = 0;
        for record in self.records.iter().rev() {
            if record.model_slug != model_slug
                || record.task_type != task_type
                || record.target_crate != target_crate
            {
                continue;
            }
            if !record.passed && record.gate.contains("compile") {
                streak += 1;
            } else {
                break;
            }
        }
        streak
    }

    /// Compute a reward penalty multiplier for a model based on verdict history.
    ///
    /// Returns 1.0 (no penalty) if the model has no concerning streaks.
    /// Returns 0.5 if the model has >2 consecutive compile failures.
    /// Returns 0.25 if the model has >5 consecutive compile failures.
    #[must_use]
    pub fn reward_penalty(&self, model_slug: &str, task_type: &str, target_crate: &str) -> f64 {
        let streak = self.compile_failure_streak(model_slug, task_type, target_crate);
        match streak {
            0..=2 => 1.0,
            3..=5 => 0.5,
            _ => 0.25,
        }
    }

    /// Return the most recent verdicts for a `(task_type, crate)` pair.
    #[must_use]
    pub fn recent_verdicts(
        &self,
        task_type: &str,
        target_crate: &str,
        limit: usize,
    ) -> Vec<&VerdictRecord> {
        self.records
            .iter()
            .rev()
            .filter(|r| r.task_type == task_type && r.target_crate == target_crate)
            .take(limit)
            .collect()
    }

    /// Return the overall failure rate for a model across all recent observations.
    #[must_use]
    pub fn model_failure_rate(&self, model_slug: &str) -> f64 {
        let model_records: Vec<&VerdictRecord> = self
            .records
            .iter()
            .filter(|r| r.model_slug == model_slug)
            .collect();

        if model_records.is_empty() {
            return 0.0;
        }

        let failures = model_records.iter().filter(|r| !r.passed).count();
        failures as f64 / model_records.len() as f64
    }

    /// Total number of recorded verdicts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the history is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Context, Engram, Kind, Score};

    fn verdict_engram(gate: &str, passed: bool, age_ms: i64) -> Engram {
        let now = chrono::Utc::now().timestamp_millis();
        let mut e = Engram::builder(Kind::GateVerdict)
            .body(Body::empty())
            .build();
        e.created_at_ms = now - age_ms;
        e.tags.insert("gate".to_string(), gate.to_string());
        e.tags
            .insert("verdict_passed".to_string(), passed.to_string());
        e
    }

    fn non_verdict_engram() -> Engram {
        Engram::builder(Kind::Task).body(Body::empty()).build()
    }

    fn ctx_at_now() -> Context {
        Context::at(chrono::Utc::now().timestamp_millis())
    }

    // ─── VerdictAwareScorer tests ────────────────────────────────────

    #[test]
    fn non_verdict_engrams_get_zero_score() {
        let scorer = VerdictAwareScorer::new();
        let ctx = ctx_at_now();
        let score = scorer.score(&non_verdict_engram(), &ctx);
        assert_eq!(score.utility, 0.0);
        assert_eq!(score.salience, 0.0);
    }

    #[test]
    fn recent_verdict_scores_higher_than_old() {
        let scorer = VerdictAwareScorer::new();
        let ctx = ctx_at_now();

        let recent = verdict_engram("compile", false, 1_000); // 1 second ago
        let old = verdict_engram("compile", false, 3_600_000); // 1 hour ago

        let recent_score = scorer.score(&recent, &ctx);
        let old_score = scorer.score(&old, &ctx);

        assert!(
            recent_score.salience > old_score.salience,
            "recent={} > old={}",
            recent_score.salience,
            old_score.salience,
        );
    }

    #[test]
    fn compile_failure_has_higher_severity_than_lint() {
        let scorer = VerdictAwareScorer::new();
        let ctx = ctx_at_now();

        let compile_fail = verdict_engram("compile", false, 100);
        let lint_fail = verdict_engram("clippy_lint", false, 100);

        let compile_score = scorer.score(&compile_fail, &ctx);
        let lint_score = scorer.score(&lint_fail, &ctx);

        assert!(
            compile_score.salience > lint_score.salience,
            "compile={} > lint={}",
            compile_score.salience,
            lint_score.salience,
        );
    }

    #[test]
    fn passing_verdict_has_low_severity() {
        let scorer = VerdictAwareScorer::new();
        let ctx = ctx_at_now();

        let pass = verdict_engram("compile", true, 100);
        let fail = verdict_engram("compile", false, 100);

        let pass_score = scorer.score(&pass, &ctx);
        let fail_score = scorer.score(&fail, &ctx);

        assert!(
            fail_score.salience > pass_score.salience,
            "fail={} > pass={}",
            fail_score.salience,
            pass_score.salience,
        );
    }

    #[test]
    fn scorer_name_is_correct() {
        let scorer = VerdictAwareScorer::new();
        assert_eq!(scorer.name(), "verdict_aware_scorer");
    }

    // ─── VerdictHistory tests ────────────────────────────────────────

    #[test]
    fn empty_history_returns_no_penalty() {
        let history = VerdictHistory::new();
        assert_eq!(
            history.reward_penalty("model-a", "implement", "roko-core"),
            1.0
        );
        assert!(history.is_empty());
    }

    #[test]
    fn compile_failure_streak_counted_correctly() {
        let mut history = VerdictHistory::new();
        let now = chrono::Utc::now().timestamp_millis();

        // 3 consecutive compile failures.
        for i in 0..3 {
            history.record(VerdictRecord {
                model_slug: "model-a".to_string(),
                task_type: "implement".to_string(),
                target_crate: "roko-core".to_string(),
                gate: "compile".to_string(),
                passed: false,
                timestamp_ms: now + i * 1000,
            });
        }

        assert_eq!(
            history.compile_failure_streak("model-a", "implement", "roko-core"),
            3
        );
        assert_eq!(
            history.reward_penalty("model-a", "implement", "roko-core"),
            0.5
        );
    }

    #[test]
    fn passing_verdict_breaks_streak() {
        let mut history = VerdictHistory::new();
        let now = chrono::Utc::now().timestamp_millis();

        // 2 failures, then a pass, then 1 failure.
        history.record(VerdictRecord {
            model_slug: "model-a".to_string(),
            task_type: "fix".to_string(),
            target_crate: "roko-gate".to_string(),
            gate: "compile".to_string(),
            passed: false,
            timestamp_ms: now,
        });
        history.record(VerdictRecord {
            model_slug: "model-a".to_string(),
            task_type: "fix".to_string(),
            target_crate: "roko-gate".to_string(),
            gate: "compile".to_string(),
            passed: false,
            timestamp_ms: now + 1000,
        });
        history.record(VerdictRecord {
            model_slug: "model-a".to_string(),
            task_type: "fix".to_string(),
            target_crate: "roko-gate".to_string(),
            gate: "compile".to_string(),
            passed: true,
            timestamp_ms: now + 2000,
        });
        history.record(VerdictRecord {
            model_slug: "model-a".to_string(),
            task_type: "fix".to_string(),
            target_crate: "roko-gate".to_string(),
            gate: "compile".to_string(),
            passed: false,
            timestamp_ms: now + 3000,
        });

        // Only 1 failure after the pass.
        assert_eq!(
            history.compile_failure_streak("model-a", "fix", "roko-gate"),
            1
        );
        assert_eq!(history.reward_penalty("model-a", "fix", "roko-gate"), 1.0);
    }

    #[test]
    fn different_model_not_counted_in_streak() {
        let mut history = VerdictHistory::new();
        let now = chrono::Utc::now().timestamp_millis();

        for i in 0..4 {
            history.record(VerdictRecord {
                model_slug: "model-b".to_string(),
                task_type: "implement".to_string(),
                target_crate: "roko-core".to_string(),
                gate: "compile".to_string(),
                passed: false,
                timestamp_ms: now + i * 1000,
            });
        }

        // model-a has no failures.
        assert_eq!(
            history.compile_failure_streak("model-a", "implement", "roko-core"),
            0
        );
        // model-b has 4.
        assert_eq!(
            history.compile_failure_streak("model-b", "implement", "roko-core"),
            4
        );
    }

    #[test]
    fn model_failure_rate_computed_correctly() {
        let mut history = VerdictHistory::new();
        let now = chrono::Utc::now().timestamp_millis();

        for i in 0..10 {
            history.record(VerdictRecord {
                model_slug: "model-c".to_string(),
                task_type: "implement".to_string(),
                target_crate: "roko-core".to_string(),
                gate: "compile".to_string(),
                passed: i % 2 == 0, // 50% pass rate
                timestamp_ms: now + i * 1000,
            });
        }

        let rate = history.model_failure_rate("model-c");
        assert!((rate - 0.5).abs() < 0.01);
    }

    #[test]
    fn high_streak_gets_severe_penalty() {
        let mut history = VerdictHistory::new();
        let now = chrono::Utc::now().timestamp_millis();

        for i in 0..8 {
            history.record(VerdictRecord {
                model_slug: "model-d".to_string(),
                task_type: "implement".to_string(),
                target_crate: "roko-gate".to_string(),
                gate: "compile".to_string(),
                passed: false,
                timestamp_ms: now + i * 1000,
            });
        }

        assert_eq!(
            history.reward_penalty("model-d", "implement", "roko-gate"),
            0.25
        );
    }

    #[test]
    fn recent_verdicts_returns_limited_results() {
        let mut history = VerdictHistory::new();
        let now = chrono::Utc::now().timestamp_millis();

        for i in 0..20 {
            history.record(VerdictRecord {
                model_slug: "model-e".to_string(),
                task_type: "fix".to_string(),
                target_crate: "roko-learn".to_string(),
                gate: "test".to_string(),
                passed: true,
                timestamp_ms: now + i * 1000,
            });
        }

        let recent = history.recent_verdicts("fix", "roko-learn", 5);
        assert_eq!(recent.len(), 5);
    }

    #[test]
    fn history_respects_capacity() {
        let mut history = VerdictHistory::with_capacity(10);
        let now = chrono::Utc::now().timestamp_millis();

        for i in 0..20 {
            history.record(VerdictRecord {
                model_slug: "model-f".to_string(),
                task_type: "implement".to_string(),
                target_crate: "roko-core".to_string(),
                gate: "compile".to_string(),
                passed: true,
                timestamp_ms: now + i * 1000,
            });
        }

        assert_eq!(history.len(), 10);
    }
}
