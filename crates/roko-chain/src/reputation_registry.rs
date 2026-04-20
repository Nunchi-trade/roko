//! Reputation Registry with 7-domain EMA scoring.
//!
//! CHAIN-03: On-chain reputation registry storing per-domain EMA scores
//! for each agent passport.
//!
//! 7 base domains: code_quality, reliability, speed, knowledge,
//! collaboration, security, oracle.
//!
//! EMA update formula: `new_score = alpha * observation + (1 - alpha) * old_score`
//! where alpha adapts: `alpha = base_alpha * (1.0 + volatility)`,
//! `base_alpha = 0.1`, `volatility = stddev(recent_10_observations)`.
//!
//! 30-day half-life decay on reads:
//!   `effective_score = score * 0.5^((now - last_update) / (30 * 86400))`
//!
//! Four discipline states: GoodStanding, Probation, Suspended, Banned.
//!
//! Slash rates by violation type: incomplete_job=-0.05, quality_failure=-0.10,
//! timeout=-0.03, collusion=-0.50.

use std::collections::HashMap;

use crate::phase2::u256;

/// Seconds in 30 days (half-life period).
const HALF_LIFE_SECS: f64 = 30.0 * 24.0 * 3600.0;

/// Base EMA alpha for reputation updates.
const BASE_ALPHA: f64 = 0.1;

/// Number of recent observations used to compute volatility.
const VOLATILITY_WINDOW: usize = 10;

/// The 7 reputation domains.
pub const REPUTATION_DOMAINS: &[&str] = &[
    "code_quality",
    "reliability",
    "speed",
    "knowledge",
    "collaboration",
    "security",
    "oracle",
];

/// Discipline states for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisciplineState {
    /// Agent is in good standing.
    GoodStanding,
    /// Agent is on probation (any score < 0.3).
    Probation,
    /// Agent is suspended (any score < 0.15 or slash count >= 3).
    Suspended,
    /// Agent is banned (governance vote).
    Banned,
}

/// Violation types that trigger reputation slashing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReputationViolation {
    /// Agent did not complete a job.
    IncompleteJob,
    /// Agent produced low-quality work.
    QualityFailure,
    /// Agent timed out.
    Timeout,
    /// Agent engaged in collusion.
    Collusion,
}

impl ReputationViolation {
    /// Slash rate for this violation type (negative value to subtract from score).
    pub fn slash_rate(self) -> f64 {
        match self {
            Self::IncompleteJob => -0.05,
            Self::QualityFailure => -0.10,
            Self::Timeout => -0.03,
            Self::Collusion => -0.50,
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
    /// Recent observations for volatility computation.
    recent_observations: Vec<f64>,
    /// Number of slashes in this domain.
    pub slash_count: u32,
}

impl DomainReputation {
    /// Create a new domain reputation with initial score.
    fn new(initial_score: f64, now: u64) -> Self {
        Self {
            score: initial_score.clamp(0.0, 1.0),
            job_count: 0,
            last_update: now,
            recent_observations: Vec::new(),
            slash_count: 0,
        }
    }

    /// Apply 30-day half-life decay on read.
    ///
    /// `effective_score = score * 0.5^((now - last_update) / half_life)`
    pub fn effective_score(&self, now: u64) -> f64 {
        if now <= self.last_update {
            return self.score;
        }
        let elapsed = (now - self.last_update) as f64;
        let decay = (0.5_f64).powf(elapsed / HALF_LIFE_SECS);
        self.score * decay
    }

    /// Compute adaptive alpha based on recent observation volatility.
    fn adaptive_alpha(&self) -> f64 {
        if self.recent_observations.len() < 2 {
            return BASE_ALPHA;
        }
        let n = self.recent_observations.len() as f64;
        let mean: f64 = self.recent_observations.iter().sum::<f64>() / n;
        let variance: f64 = self
            .recent_observations
            .iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f64>()
            / n;
        let volatility = variance.sqrt();
        (BASE_ALPHA * (1.0 + volatility)).min(0.5)
    }

    /// Update the EMA score with a new observation.
    fn update(&mut self, observation: f64, now: u64) {
        let alpha = self.adaptive_alpha();
        self.score = (alpha * observation + (1.0 - alpha) * self.score).clamp(0.0, 1.0);
        self.job_count += 1;
        self.last_update = now;

        self.recent_observations.push(observation);
        if self.recent_observations.len() > VOLATILITY_WINDOW {
            self.recent_observations.remove(0);
        }
    }

    /// Apply a slash penalty to the score.
    fn slash(&mut self, penalty: f64, now: u64) {
        self.score = (self.score + penalty).clamp(0.0, 1.0);
        self.slash_count += 1;
        self.last_update = now;
    }
}

/// Per-agent reputation profile across all 7 domains.
#[derive(Debug, Clone)]
pub struct AgentReputation {
    /// Reputation per domain.
    pub domains: HashMap<String, DomainReputation>,
    /// Manually set discipline state (for Banned via governance).
    discipline_override: Option<DisciplineState>,
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
        }
    }

    /// Compute the discipline state from current scores and slash history.
    pub fn discipline_state(&self, now: u64) -> DisciplineState {
        if let Some(state) = self.discipline_override {
            return state;
        }

        let total_slashes: u32 = self.domains.values().map(|d| d.slash_count).sum();
        if total_slashes >= 3 {
            return DisciplineState::Suspended;
        }

        for domain in self.domains.values() {
            let effective = domain.effective_score(now);
            if effective < 0.15 {
                return DisciplineState::Suspended;
            }
            if effective < 0.3 {
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
    pub fn submit_feedback(
        &mut self,
        passport_id: u256,
        domain: &str,
        quality: f64,
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

        domain_rep.update(quality.clamp(0.0, 1.0), now);
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

    /// Slash an agent's reputation in a domain.
    pub fn slash(
        &mut self,
        passport_id: u256,
        domain: &str,
        violation: ReputationViolation,
        now: u64,
    ) {
        let penalty = violation.slash_rate();

        if let Some(agent) = self.records.get_mut(&passport_id) {
            if let Some(domain_rep) = agent.domains.get_mut(domain) {
                domain_rep.slash(penalty, now);
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
            registry.submit_feedback(1, "code_quality", 0.95, now + i);
        }

        let score = registry.get_score(1, "code_quality", now + 20);
        assert!(score > 0.7, "score should rise with high feedback, got {score}");
    }

    #[test]
    fn half_life_decay_on_read() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        // Set a high score.
        for _ in 0..20 {
            registry.submit_feedback(1, "reliability", 0.95, now);
        }

        let score_at_0 = registry.get_score(1, "reliability", now);

        // After 30 days, score should be halved.
        let thirty_days = (HALF_LIFE_SECS as u64) + now;
        let score_at_30d = registry.get_score(1, "reliability", thirty_days);

        assert!(
            (score_at_30d - score_at_0 * 0.5).abs() < 0.05,
            "score should be ~50% after 30d: {score_at_0} -> {score_at_30d}"
        );
    }

    #[test]
    fn slash_reduces_score() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        let before = registry.get_score(1, "speed", now);
        registry.slash(1, "speed", ReputationViolation::QualityFailure, now);
        let after = registry.get_score(1, "speed", now);

        assert!(
            after < before,
            "slash should reduce score: {before} -> {after}"
        );
        assert!(
            (after - (before - 0.10)).abs() < 0.01,
            "quality_failure slash should reduce by 0.10"
        );
    }

    #[test]
    fn slash_rates_match_spec() {
        assert!((ReputationViolation::IncompleteJob.slash_rate() - (-0.05)).abs() < f64::EPSILON);
        assert!((ReputationViolation::QualityFailure.slash_rate() - (-0.10)).abs() < f64::EPSILON);
        assert!((ReputationViolation::Timeout.slash_rate() - (-0.03)).abs() < f64::EPSILON);
        assert!((ReputationViolation::Collusion.slash_rate() - (-0.50)).abs() < f64::EPSILON);
    }

    #[test]
    fn discipline_states() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        // Default should be GoodStanding.
        assert_eq!(
            registry.discipline_state(1, now),
            DisciplineState::GoodStanding
        );

        // Push a domain below 0.3 but above 0.15 -> Probation.
        // Use a value that will converge to around 0.25 via EMA.
        for _ in 0..20 {
            registry.submit_feedback(1, "security", 0.2, now);
        }
        let score = registry.get_score(1, "security", now);
        assert!(
            score < 0.3 && score >= 0.15,
            "score should be in probation range [0.15, 0.3), got {score}"
        );
        assert_eq!(
            registry.discipline_state(1, now),
            DisciplineState::Probation
        );
    }

    #[test]
    fn suspended_after_three_slashes() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        registry.slash(1, "code_quality", ReputationViolation::IncompleteJob, now);
        registry.slash(1, "reliability", ReputationViolation::Timeout, now);
        registry.slash(1, "speed", ReputationViolation::IncompleteJob, now);

        assert_eq!(
            registry.discipline_state(1, now),
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
    fn adaptive_alpha_increases_with_volatility() {
        let mut rep = DomainReputation::new(0.5, 0);

        // Low volatility: all same observations.
        for _ in 0..10 {
            rep.recent_observations.push(0.5);
        }
        let low_alpha = rep.adaptive_alpha();

        // High volatility: alternating observations.
        rep.recent_observations.clear();
        for i in 0..10 {
            rep.recent_observations.push(if i % 2 == 0 { 0.0 } else { 1.0 });
        }
        let high_alpha = rep.adaptive_alpha();

        assert!(
            high_alpha > low_alpha,
            "higher volatility should increase alpha: {low_alpha} vs {high_alpha}"
        );
    }

    #[test]
    fn collusion_slash_is_severe() {
        let mut registry = ReputationRegistry::new();
        let now = 1_000_000;
        registry.register_agent(1, now);

        registry.slash(1, "oracle", ReputationViolation::Collusion, now);
        let score = registry.get_score(1, "oracle", now);

        // 0.5 - 0.50 = 0.0.
        assert!(
            score < 0.01,
            "collusion slash should drop score to ~0.0, got {score}"
        );
    }
}
