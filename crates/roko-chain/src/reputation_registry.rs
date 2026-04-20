//! Reputation Registry with 7-domain EMA scoring.
//!
//! CHAIN-03: On-chain reputation registry storing per-domain EMA scores
//! for each agent passport.
//!
//! 7 base domains (spec-aligned): coding, security, research, chain,
//! knowledge, operations, strategy.
//!
//! EMA update formula: `new_score = alpha * observation + (1 - alpha) * old_score`
//! where alpha is job-count-based:
//!   - 0-10 jobs:   alpha = 0.30 (responsive to early feedback)
//!   - 11-50 jobs:  alpha = 0.15
//!   - 51-200 jobs: alpha = 0.08
//!   - 200+ jobs:   alpha = 0.04 (stable, well-established)
//!
//! 30-day half-life decay toward neutral (0.5):
//!   `effective_score = NEUTRAL + (score - NEUTRAL) * 0.5^((now - last_update) / (30 * 86400))`
//!
//! Four discipline states: GoodStanding, Probation, Suspended, Banned.
//! Thresholds per spec: Probation < 0.4, Suspension < 0.2.
//!
//! Seven violation types with spec-aligned slash rates.
//! Collusion applies feedback weight dilution (-50% for 30 days), NOT direct score slash.

use std::collections::HashMap;

use crate::phase2::u256;

/// Seconds in 30 days (half-life period).
const HALF_LIFE_SECS: f64 = 30.0 * 24.0 * 3600.0;

/// Neutral reputation score (decay converges here, not to 0).
const NEUTRAL: f64 = 0.5;

/// Duration in seconds for collusion feedback weight dilution (30 days).
const COLLUSION_DILUTION_DURATION_SECS: u64 = 30 * 24 * 3600;

/// Rolling window for slash counting (90 days in seconds).
const SLASH_WINDOW_SECS: u64 = 90 * 24 * 3600;

/// The 7 reputation domains (spec-aligned).
///
/// From `docs/08-chain/14-reputation-system-7-domain.md`:
/// 1. coding    — code quality, correctness, style
/// 2. security  — vulnerability assessment, safe patterns
/// 3. research  — information gathering, analysis depth
/// 4. chain     — on-chain operations, protocol understanding
/// 5. knowledge — domain expertise, factual accuracy
/// 6. operations — reliability, process adherence, DevOps
/// 7. strategy  — planning, architecture, decision-making
pub const REPUTATION_DOMAINS: &[&str] = &[
    "coding",
    "security",
    "research",
    "chain",
    "knowledge",
    "operations",
    "strategy",
];

/// Discipline states for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisciplineState {
    /// Agent is in good standing.
    GoodStanding,
    /// Agent is on probation (any domain score < 0.4).
    Probation,
    /// Agent is suspended (any domain score < 0.2, or 3+ slashes in 90 days).
    Suspended,
    /// Agent is banned (governance vote; appealable after 365 days).
    Banned,
}

/// Violation types that trigger reputation slashing.
///
/// Aligned with spec (`docs/08-chain/04-korai-passport-erc-721-soulbound.md` lines 85-93).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReputationViolation {
    /// Agent missed a deadline. Slash: -1%.
    MissedDeadline,
    /// Agent abandoned a job in progress. Slash: -3%.
    AbandonedJob,
    /// Agent produced work that failed quality review. Slash: -2%.
    QualityRejection,
    /// Agent repeatedly failed quality checks (pattern). Slash: -5%.
    RepeatedQualityFailure,
    /// Agent plagiarized another's work. Slash: -10%.
    Plagiarism,
    /// Agent manipulated results or gamed metrics. Slash: -10%.
    ResultManipulation,
    /// Agent violated TEE enclave integrity. Slash: -10%.
    TeeViolation,
    /// Agent engaged in collusion ring. Feedback weight dilution (NOT direct slash).
    Collusion,
}

impl ReputationViolation {
    /// Direct slash rate for this violation type (negative value subtracted from score).
    ///
    /// Note: Collusion returns 0.0 because it uses feedback weight dilution, not a direct slash.
    pub fn slash_rate(self) -> f64 {
        match self {
            Self::MissedDeadline => -0.01,
            Self::AbandonedJob => -0.03,
            Self::QualityRejection => -0.02,
            Self::RepeatedQualityFailure => -0.05,
            Self::Plagiarism => -0.10,
            Self::ResultManipulation => -0.10,
            Self::TeeViolation => -0.10,
            Self::Collusion => 0.0, // Handled via feedback weight dilution
        }
    }

    /// Whether this violation is a collusion offense (handled differently).
    pub fn is_collusion(self) -> bool {
        matches!(self, Self::Collusion)
    }
}

/// Feedback weight dilution applied when collusion is detected.
///
/// Per spec: "all members' feedback weight is reduced by 50% for 30 days."
/// This means their future feedback as job posters/raters carries half weight in EMA updates.
#[derive(Debug, Clone)]
pub struct FeedbackDilution {
    /// When the dilution was applied (unix timestamp seconds).
    pub applied_at: u64,
    /// Multiplier on feedback weight (0.5 = 50% dilution).
    pub multiplier: f64,
    /// Duration of dilution in seconds (30 days).
    pub duration_secs: u64,
}

impl FeedbackDilution {
    /// Create a standard collusion dilution.
    fn collusion(now: u64) -> Self {
        Self {
            applied_at: now,
            multiplier: 0.5,
            duration_secs: COLLUSION_DILUTION_DURATION_SECS,
        }
    }

    /// Whether this dilution is still active.
    pub fn is_active(&self, now: u64) -> bool {
        now < self.applied_at + self.duration_secs
    }

    /// Get effective multiplier (1.0 if expired).
    pub fn effective_multiplier(&self, now: u64) -> f64 {
        if self.is_active(now) {
            self.multiplier
        } else {
            1.0
        }
    }
}

/// Per-domain reputation record for a single agent.
#[derive(Debug, Clone)]
pub struct DomainReputation {
    /// EMA-smoothed score in [0.0, 1.0].
    pub score: f64,
    /// Number of completed jobs in this domain.
    pub job_count: u64,
    /// Unix timestamp (seconds) of the last update.
    pub last_update: u64,
    /// Timestamped slash events for 90-day rolling window.
    slash_timestamps: Vec<u64>,
}

impl DomainReputation {
    /// Create a new domain reputation with initial score.
    fn new(initial_score: f64, now: u64) -> Self {
        Self {
            score: initial_score.clamp(0.0, 1.0),
            job_count: 0,
            last_update: now,
            slash_timestamps: Vec::new(),
        }
    }

    /// Apply 30-day half-life decay toward neutral (0.5) on read.
    ///
    /// Formula: `effective = NEUTRAL + (score - NEUTRAL) * 0.5^(elapsed / half_life)`
    ///
    /// This converges toward 0.5 from both directions:
    /// - High scores (0.9) decay DOWN toward 0.5
    /// - Low scores (0.2) recover UP toward 0.5
    pub fn effective_score(&self, now: u64) -> f64 {
        if now <= self.last_update {
            return self.score;
        }
        let elapsed = (now - self.last_update) as f64;
        let decay = (0.5_f64).powf(elapsed / HALF_LIFE_SECS);
        NEUTRAL + (self.score - NEUTRAL) * decay
    }

    /// Compute adaptive alpha based on job count.
    ///
    /// Per spec (docs/08-chain/14-reputation-system-7-domain.md lines 58-68):
    /// - 0-10 jobs:   alpha = 0.30 (new agents, responsive to feedback)
    /// - 11-50 jobs:  alpha = 0.15
    /// - 51-200 jobs: alpha = 0.08
    /// - 200+ jobs:   alpha = 0.04 (established agents, stable score)
    fn adaptive_alpha(&self) -> f64 {
        match self.job_count {
            0..=10 => 0.30,
            11..=50 => 0.15,
            51..=200 => 0.08,
            _ => 0.04,
        }
    }

    /// Update the EMA score with a new observation.
    ///
    /// `feedback_weight` accounts for the rater's feedback dilution (1.0 = full weight).
    fn update(&mut self, observation: f64, feedback_weight: f64, now: u64) {
        let alpha = self.adaptive_alpha() * feedback_weight;
        self.score = (alpha * observation + (1.0 - alpha) * self.score).clamp(0.0, 1.0);
        self.job_count += 1;
        self.last_update = now;
    }

    /// Apply a slash penalty to the score.
    fn slash(&mut self, penalty: f64, now: u64) {
        self.score = (self.score + penalty).clamp(0.0, 1.0);
        self.slash_timestamps.push(now);
        self.last_update = now;
    }

    /// Count slashes within the rolling 90-day window.
    pub fn slash_count_in_window(&self, now: u64) -> u32 {
        let window_start = now.saturating_sub(SLASH_WINDOW_SECS);
        self.slash_timestamps
            .iter()
            .filter(|&&t| t >= window_start)
            .count() as u32
    }
}

/// Per-agent reputation profile across all 7 domains.
#[derive(Debug, Clone)]
pub struct AgentReputation {
    /// Reputation per domain.
    pub domains: HashMap<String, DomainReputation>,
    /// Manually set discipline state (for Banned via governance).
    discipline_override: Option<DisciplineState>,
    /// Active feedback weight dilutions (from collusion detection).
    pub feedback_dilutions: Vec<FeedbackDilution>,
}

impl AgentReputation {
    /// Create a new agent reputation with default scores across all domains.
    fn new(now: u64) -> Self {
        let mut domains = HashMap::new();
        for &domain in REPUTATION_DOMAINS {
            domains.insert(domain.to_string(), DomainReputation::new(0.5, now));
        }
        Self {
            domains,
            discipline_override: None,
            feedback_dilutions: Vec::new(),
        }
    }

    /// Get the current effective feedback weight for this agent as a rater.
    ///
    /// Multiple dilutions stack multiplicatively.
    pub fn feedback_weight(&self, now: u64) -> f64 {
        self.feedback_dilutions
            .iter()
            .map(|d| d.effective_multiplier(now))
            .product()
    }

    /// Apply a collusion dilution penalty.
    fn apply_collusion_dilution(&mut self, now: u64) {
        self.feedback_dilutions.push(FeedbackDilution::collusion(now));
    }

    /// Compute the discipline state from current scores and slash history.
    ///
    /// Thresholds per spec:
    /// - Probation: any domain < 0.4
    /// - Suspended: any domain < 0.2 OR 3+ slashes in 90-day window
    pub fn discipline_state(&self, now: u64) -> DisciplineState {
        if let Some(state) = self.discipline_override {
            return state;
        }

        // Check 90-day rolling slash count across all domains.
        let total_slashes: u32 = self
            .domains
            .values()
            .map(|d| d.slash_count_in_window(now))
            .sum();
        if total_slashes >= 3 {
            return DisciplineState::Suspended;
        }

        for domain in self.domains.values() {
            let effective = domain.effective_score(now);
            if effective < 0.2 {
                return DisciplineState::Suspended;
            }
            if effective < 0.4 {
                return DisciplineState::Probation;
            }
        }

        DisciplineState::GoodStanding
    }
}

/// The Reputation Registry managing all agent reputation across 7 domains.
#[derive(Debug, Clone, Default)]
pub struct ReputationRegistry {
    /// Per-passport reputation records.
    records: HashMap<u256, AgentReputation>,
}

impl ReputationRegistry {
    /// Create a new empty reputation registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize reputation for a new agent passport.
    pub fn register_agent(&mut self, passport_id: u256, now: u64) {
        self.records
            .entry(passport_id)
            .or_insert_with(|| AgentReputation::new(now));
    }

    /// Submit feedback for an agent in a specific domain.
    ///
    /// `quality` is in [0.0, 1.0] where 1.0 is perfect performance.
    /// `rater_passport_id` is used to look up the rater's feedback weight
    /// (may be diluted if the rater was caught in a collusion ring).
    pub fn submit_feedback(
        &mut self,
        passport_id: u256,
        domain: &str,
        quality: f64,
        now: u64,
    ) {
        self.submit_feedback_weighted(passport_id, domain, quality, 1.0, now);
    }

    /// Submit feedback with an explicit feedback weight (for rater dilution).
    pub fn submit_feedback_weighted(
        &mut self,
        passport_id: u256,
        domain: &str,
        quality: f64,
        feedback_weight: f64,
        now: u64,
    ) {
        let agent = self
            .records
            .entry(passport_id)
            .or_insert_with(|| AgentReputation::new(now));

        let domain_rep = agent
            .domains
            .entry(domain.to_string())
            .or_insert_with(|| DomainReputation::new(0.5, now));

        domain_rep.update(quality.clamp(0.0, 1.0), feedback_weight.clamp(0.0, 1.0), now);
    }

    /// Get the effective (decay-adjusted) score for an agent in a domain.
    pub fn get_score(&self, passport_id: u256, domain: &str, now: u64) -> f64 {
        self.records
            .get(&passport_id)
            .and_then(|agent| agent.domains.get(domain))
            .map(|d| d.effective_score(now))
            .unwrap_or(0.0)
    }

    /// Get all domain scores for an agent (decay-adjusted).
    pub fn get_all_scores(
        &self,
        passport_id: u256,
        now: u64,
    ) -> HashMap<String, f64> {
        self.records
            .get(&passport_id)
            .map(|agent| {
                agent
                    .domains
                    .iter()
                    .map(|(domain, rep)| (domain.clone(), rep.effective_score(now)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the feedback weight for an agent acting as a rater.
    pub fn feedback_weight(&self, passport_id: u256, now: u64) -> f64 {
        self.records
            .get(&passport_id)
            .map(|agent| agent.feedback_weight(now))
            .unwrap_or(1.0)
    }

    /// Slash an agent's reputation in a domain.
    ///
    /// For collusion violations, this applies feedback weight dilution instead of a direct slash.
    pub fn slash(
        &mut self,
        passport_id: u256,
        domain: &str,
        violation: ReputationViolation,
        now: u64,
    ) {
        if let Some(agent) = self.records.get_mut(&passport_id) {
            if violation.is_collusion() {
                // Collusion: apply feedback weight dilution, not direct score penalty.
                agent.apply_collusion_dilution(now);
            } else {
                // Direct slash for non-collusion violations.
                let penalty = violation.slash_rate();
                if let Some(domain_rep) = agent.domains.get_mut(domain) {
                    domain_rep.slash(penalty, now);
                }
            }
        }
    }

    /// Get the discipline state for an agent.
    pub fn discipline_state(&self, passport_id: u256, now: u64) -> DisciplineState {
        self.records
            .get(&passport_id)
            .map(|agent| agent.discipline_state(now))
            .unwrap_or(DisciplineState::GoodStanding)
    }

    /// Ban an agent (governance action).
    pub fn ban_agent(&mut self, passport_id: u256) {
        if let Some(agent) = self.records.get_mut(&passport_id) {
            agent.discipline_override = Some(DisciplineState::Banned);
        }
    }

    /// Number of registered agents.
    pub fn agent_count(&self) -> usize {
        self.records.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_get_default_scores() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        // All 7 domains should have default score 0.5.
        let scores = registry.get_all_scores(1, now);
        assert_eq!(scores.len(), 7);
        for &domain in REPUTATION_DOMAINS {
            let score = scores.get(domain).copied().unwrap_or(0.0);
            assert!(
                (score - 0.5).abs() < 0.01,
                "default score for {domain} should be ~0.5, got {score}"
            );
        }
    }

    #[test]
    fn feedback_updates_ema_score() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        // Feed consistently high quality.
        for i in 0..20 {
            registry.submit_feedback(1, "coding", 0.95, now + i);
        }

        let score = registry.get_score(1, "coding", now + 20);
        assert!(score > 0.7, "score should rise with high feedback, got {score}");
    }

    #[test]
    fn half_life_decay_toward_neutral() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        // Set a high score.
        for _ in 0..20 {
            registry.submit_feedback(1, "security", 0.95, now);
        }

        let score_at_0 = registry.get_score(1, "security", now);
        assert!(score_at_0 > 0.7, "score should be high after good feedback");

        // After 30 days, score should decay toward neutral (0.5).
        // Formula: effective = 0.5 + (score - 0.5) * 0.5
        let thirty_days = (HALF_LIFE_SECS as u64) + now;
        let score_at_30d = registry.get_score(1, "security", thirty_days);

        let expected = NEUTRAL + (score_at_0 - NEUTRAL) * 0.5;
        assert!(
            (score_at_30d - expected).abs() < 0.02,
            "score should decay toward 0.5: {score_at_0} -> {score_at_30d}, expected {expected}"
        );
    }

    #[test]
    fn low_scores_recover_toward_neutral() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        // Set a low score.
        for _ in 0..20 {
            registry.submit_feedback(1, "research", 0.1, now);
        }

        let score_at_0 = registry.get_score(1, "research", now);
        assert!(score_at_0 < 0.4, "score should be low, got {score_at_0}");

        // After 30 days, low score should recover TOWARD neutral (0.5).
        let thirty_days = (HALF_LIFE_SECS as u64) + now;
        let score_at_30d = registry.get_score(1, "research", thirty_days);

        assert!(
            score_at_30d > score_at_0,
            "low score should recover toward neutral: {score_at_0} -> {score_at_30d}"
        );
        let expected = NEUTRAL + (score_at_0 - NEUTRAL) * 0.5;
        assert!(
            (score_at_30d - expected).abs() < 0.02,
            "recovery formula: {score_at_0} -> {score_at_30d}, expected {expected}"
        );
    }

    #[test]
    fn adaptive_alpha_job_count_tiers() {
        let mut rep = DomainReputation::new(0.5, 0);

        // 0 jobs -> alpha = 0.30
        assert!((rep.adaptive_alpha() - 0.30).abs() < f64::EPSILON);

        // 10 jobs -> still 0.30
        rep.job_count = 10;
        assert!((rep.adaptive_alpha() - 0.30).abs() < f64::EPSILON);

        // 11 jobs -> alpha = 0.15
        rep.job_count = 11;
        assert!((rep.adaptive_alpha() - 0.15).abs() < f64::EPSILON);

        // 50 jobs -> still 0.15
        rep.job_count = 50;
        assert!((rep.adaptive_alpha() - 0.15).abs() < f64::EPSILON);

        // 51 jobs -> alpha = 0.08
        rep.job_count = 51;
        assert!((rep.adaptive_alpha() - 0.08).abs() < f64::EPSILON);

        // 200 jobs -> still 0.08
        rep.job_count = 200;
        assert!((rep.adaptive_alpha() - 0.08).abs() < f64::EPSILON);

        // 201 jobs -> alpha = 0.04
        rep.job_count = 201;
        assert!((rep.adaptive_alpha() - 0.04).abs() < f64::EPSILON);
    }

    #[test]
    fn slash_reduces_score() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        let before = registry.get_score(1, "chain", now);
        registry.slash(1, "chain", ReputationViolation::QualityRejection, now);
        let after = registry.get_score(1, "chain", now);

        assert!(
            after < before,
            "slash should reduce score: {before} -> {after}"
        );
        assert!(
            (after - (before - 0.02)).abs() < 0.01,
            "quality_rejection slash should reduce by 0.02"
        );
    }

    #[test]
    fn slash_rates_match_spec() {
        assert!((ReputationViolation::MissedDeadline.slash_rate() - (-0.01)).abs() < f64::EPSILON);
        assert!((ReputationViolation::AbandonedJob.slash_rate() - (-0.03)).abs() < f64::EPSILON);
        assert!((ReputationViolation::QualityRejection.slash_rate() - (-0.02)).abs() < f64::EPSILON);
        assert!(
            (ReputationViolation::RepeatedQualityFailure.slash_rate() - (-0.05)).abs()
                < f64::EPSILON
        );
        assert!((ReputationViolation::Plagiarism.slash_rate() - (-0.10)).abs() < f64::EPSILON);
        assert!(
            (ReputationViolation::ResultManipulation.slash_rate() - (-0.10)).abs() < f64::EPSILON
        );
        assert!((ReputationViolation::TeeViolation.slash_rate() - (-0.10)).abs() < f64::EPSILON);
        assert!((ReputationViolation::Collusion.slash_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn collusion_applies_feedback_dilution_not_direct_slash() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        let before = registry.get_score(1, "strategy", now);
        registry.slash(1, "strategy", ReputationViolation::Collusion, now);
        let after = registry.get_score(1, "strategy", now);

        // Score should NOT change for collusion.
        assert!(
            (after - before).abs() < f64::EPSILON,
            "collusion should not slash score directly: {before} -> {after}"
        );

        // But feedback weight should be halved.
        let weight = registry.feedback_weight(1, now);
        assert!(
            (weight - 0.5).abs() < f64::EPSILON,
            "collusion should halve feedback weight, got {weight}"
        );

        // After 30 days, dilution should expire.
        let after_30d = now + COLLUSION_DILUTION_DURATION_SECS;
        let weight_expired = registry.feedback_weight(1, after_30d);
        assert!(
            (weight_expired - 1.0).abs() < f64::EPSILON,
            "feedback weight should recover after 30 days, got {weight_expired}"
        );
    }

    #[test]
    fn discipline_states_with_correct_thresholds() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        // Default should be GoodStanding.
        assert_eq!(
            registry.discipline_state(1, now),
            DisciplineState::GoodStanding
        );

        // Push a domain below 0.4 but above 0.2 -> Probation.
        for _ in 0..30 {
            registry.submit_feedback(1, "operations", 0.25, now);
        }
        let score = registry.get_score(1, "operations", now);
        assert!(
            score < 0.4 && score >= 0.2,
            "score should be in probation range [0.2, 0.4), got {score}"
        );
        assert_eq!(
            registry.discipline_state(1, now),
            DisciplineState::Probation
        );
    }

    #[test]
    fn suspended_after_three_slashes_in_window() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        registry.slash(1, "coding", ReputationViolation::MissedDeadline, now);
        registry.slash(1, "security", ReputationViolation::AbandonedJob, now);
        registry.slash(1, "research", ReputationViolation::MissedDeadline, now);

        assert_eq!(
            registry.discipline_state(1, now),
            DisciplineState::Suspended
        );
    }

    #[test]
    fn old_slashes_outside_window_dont_count() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        // Two slashes now.
        registry.slash(1, "coding", ReputationViolation::MissedDeadline, now);
        registry.slash(1, "security", ReputationViolation::MissedDeadline, now);

        // One slash 91 days later (the first two are now outside the 90-day window).
        let later = now + SLASH_WINDOW_SECS + 86400;
        registry.slash(1, "research", ReputationViolation::MissedDeadline, later);

        // Only 1 slash in window -> not suspended.
        assert_ne!(
            registry.discipline_state(1, later),
            DisciplineState::Suspended
        );
    }

    #[test]
    fn ban_overrides_everything() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        registry.ban_agent(1);
        assert_eq!(
            registry.discipline_state(1, now),
            DisciplineState::Banned
        );
    }

    #[test]
    fn domains_match_spec() {
        assert_eq!(REPUTATION_DOMAINS.len(), 7);
        assert_eq!(REPUTATION_DOMAINS[0], "coding");
        assert_eq!(REPUTATION_DOMAINS[1], "security");
        assert_eq!(REPUTATION_DOMAINS[2], "research");
        assert_eq!(REPUTATION_DOMAINS[3], "chain");
        assert_eq!(REPUTATION_DOMAINS[4], "knowledge");
        assert_eq!(REPUTATION_DOMAINS[5], "operations");
        assert_eq!(REPUTATION_DOMAINS[6], "strategy");
    }

    #[test]
    fn severe_slash_triggers_suspension() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        // Push a domain very low then slash it further below 0.2.
        for _ in 0..30 {
            registry.submit_feedback(1, "coding", 0.15, now);
        }
        // Score should be around 0.2, one more slash should push below.
        registry.slash(1, "coding", ReputationViolation::Plagiarism, now);

        let score = registry.get_score(1, "coding", now);
        assert!(score < 0.2, "score should be below 0.2 after plagiarism slash, got {score}");
        assert_eq!(
            registry.discipline_state(1, now),
            DisciplineState::Suspended
        );
    }
}
