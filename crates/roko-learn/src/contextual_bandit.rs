//! Conservative contextual bandit policy for routing and context decisions.
//!
//! The policy is intentionally generic: callers provide stable action ids,
//! context features, and bounded reward observations.  Selection can be
//! disabled or run in a conservative epsilon-greedy mode, and learning emits
//! [`PolicyUpdateCandidate`] records instead of mutating active production
//! manifests.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::provider_model_outcome::{ProviderModelOutcomeRecord, ProviderModelOutcomeStatus};
use crate::section_outcome::{SectionOutcomeRecord, SectionOutcomeStatus};

/// Current JSON schema version for bandit decisions and updates.
pub const CONTEXTUAL_BANDIT_SCHEMA_VERSION: u32 = 1;

/// Decision surface controlled by the contextual bandit layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BanditDecisionKind {
    /// Provider/model routing, keyed by provider/model action ids.
    ProviderModelRouting,
    /// Prompt and context section inclusion decisions.
    PromptContextSectionInclusion,
    /// Context bidder budget-share decisions.
    BidderBudgetShare,
    /// Reviewer model selection, when reviewer routing is available.
    ReviewerModelChoice,
}

impl BanditDecisionKind {
    /// Stable lowercase label used in context keys and update identifiers.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProviderModelRouting => "provider_model_routing",
            Self::PromptContextSectionInclusion => "prompt_context_section_inclusion",
            Self::BidderBudgetShare => "bidder_budget_share",
            Self::ReviewerModelChoice => "reviewer_model_choice",
        }
    }
}

/// Context features used when selecting among action ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanditContextFeatures {
    /// Decision surface being evaluated.
    pub decision_kind: BanditDecisionKind,
    /// Task type or category label.
    pub task_type: String,
    /// Crate, subsystem, or UI surface label.
    pub crate_surface: String,
    /// Role profile id.
    pub role_id: String,
    /// Count of prior gate failures in the relevant history window.
    pub prior_gate_failures: u32,
    /// Count of prior gate passes in the relevant history window.
    pub prior_gate_passes: u32,
    /// Average observed latency for the task/action slice, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_latency_ms: Option<f64>,
    /// Retry count for the current or observed attempt.
    pub retry_count: u32,
    /// Cost in USD for the current or observed attempt, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Total token count for the current or observed attempt, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// Additional stable categorical features.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

impl BanditContextFeatures {
    /// Build a minimal feature set for a decision surface.
    #[must_use]
    pub fn new(
        decision_kind: BanditDecisionKind,
        task_type: impl Into<String>,
        crate_surface: impl Into<String>,
        role_id: impl Into<String>,
    ) -> Self {
        Self {
            decision_kind,
            task_type: task_type.into(),
            crate_surface: crate_surface.into(),
            role_id: role_id.into(),
            prior_gate_failures: 0,
            prior_gate_passes: 0,
            avg_latency_ms: None,
            retry_count: 0,
            cost_usd: None,
            total_tokens: None,
            extra: BTreeMap::new(),
        }
    }

    /// Return the stable contextual bucket key for this feature vector.
    #[must_use]
    pub fn context_key(&self) -> String {
        format!(
            "{}|task:{}|surface:{}|role:{}|gate:{}-{}|retry:{}",
            self.decision_kind.label(),
            normalize_key_part(&self.task_type),
            normalize_key_part(&self.crate_surface),
            normalize_key_part(&self.role_id),
            bucket_count(self.prior_gate_failures),
            bucket_count(self.prior_gate_passes),
            bucket_count(self.retry_count)
        )
    }

    /// Build features from provider/model telemetry.
    #[must_use]
    pub fn from_provider_model_outcome(record: &ProviderModelOutcomeRecord) -> Self {
        let mut extra = BTreeMap::new();
        extra.insert("provider".to_string(), record.provider.clone());
        extra.insert("model".to_string(), record.model.clone());
        Self {
            decision_kind: BanditDecisionKind::ProviderModelRouting,
            task_type: record.task_type.clone(),
            crate_surface: "unknown".to_string(),
            role_id: record
                .role_id
                .clone()
                .unwrap_or_else(|| "unknown-role".to_string()),
            prior_gate_failures: 0,
            prior_gate_passes: 0,
            avg_latency_ms: record.usage.latency_ms.map(|value| value as f64),
            retry_count: record.retry_count,
            cost_usd: record.usage.cost_usd,
            total_tokens: record.usage.total_tokens,
            extra,
        }
    }

    /// Build features from prompt/context section telemetry.
    #[must_use]
    pub fn from_section_outcome(record: &SectionOutcomeRecord) -> Self {
        let mut extra = BTreeMap::new();
        extra.insert(
            "section_kind".to_string(),
            format!("{:?}", record.section_kind),
        );
        extra.insert("included".to_string(), record.included.to_string());
        Self {
            decision_kind: BanditDecisionKind::PromptContextSectionInclusion,
            task_type: record.task_type.clone(),
            crate_surface: record
                .source_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            role_id: record.role_id.clone(),
            prior_gate_failures: 0,
            prior_gate_passes: 0,
            avg_latency_ms: None,
            retry_count: 0,
            cost_usd: None,
            total_tokens: Some(record.tokens_used as u64),
            extra,
        }
    }
}

/// One action that can be selected by a bandit policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanditAction {
    /// Stable action id, such as `provider:openai|model:gpt-5.5`.
    pub action_id: String,
    /// Human-readable label.
    pub label: String,
    /// Whether the action is administratively enabled.
    pub enabled: bool,
    /// Estimated cost in USD for safety filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    /// Estimated latency in milliseconds for safety filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_latency_ms: Option<u64>,
    /// Estimated token budget for context/bidder decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    /// Proposed bidder budget share in `[0, 1]`, when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_share: Option<f64>,
    /// Action-specific safety bounds.
    #[serde(default)]
    pub safety_bounds: ActionSafetyBounds,
}

impl BanditAction {
    /// Construct an enabled action with default safety bounds.
    #[must_use]
    pub fn new(action_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            action_id: action_id.into(),
            label: label.into(),
            enabled: true,
            estimated_cost_usd: None,
            estimated_latency_ms: None,
            token_budget: None,
            budget_share: None,
            safety_bounds: ActionSafetyBounds::default(),
        }
    }
}

/// Hard safety bounds applied before a bandit action can be selected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionSafetyBounds {
    /// Maximum allowed estimated cost in USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    /// Maximum allowed estimated latency in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_latency_ms: Option<u64>,
    /// Maximum retry count allowed for this action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retry_count: Option<u32>,
    /// Maximum token budget allowed for this action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_token_budget: Option<u64>,
    /// Maximum bidder budget share allowed for this action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_budget_share: Option<f64>,
    /// Minimum observed success rate required after the observation floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_success_rate: Option<f64>,
    /// Observation count required before enforcing `min_success_rate`.
    pub min_observations_for_rate: u64,
    /// Whether exploration is allowed for this action.
    pub allow_exploration: bool,
}

impl Default for ActionSafetyBounds {
    fn default() -> Self {
        Self {
            max_cost_usd: None,
            max_latency_ms: None,
            max_retry_count: None,
            max_token_budget: None,
            max_budget_share: None,
            min_success_rate: None,
            min_observations_for_rate: 20,
            allow_exploration: true,
        }
    }
}

/// Operational metrics attached to a reward observation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RewardMetrics {
    /// Observed latency in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Observed cost in USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Observed total tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// Retry count before terminal outcome.
    pub retry_count: u32,
}

/// Bounded reward observation for an action/context pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanditRewardObservation {
    /// Stable action id receiving the observation.
    pub action_id: String,
    /// Contextual bucket key.
    pub context_key: String,
    /// Whether the terminal gate/review outcome passed.
    pub success: bool,
    /// Raw quality score in `[0, 1]`.
    pub quality: f64,
    /// Operational metrics that shape the bounded reward.
    #[serde(default)]
    pub metrics: RewardMetrics,
}

impl BanditRewardObservation {
    /// Build a reward observation from provider/model telemetry.
    #[must_use]
    pub fn from_provider_model_outcome(record: &ProviderModelOutcomeRecord) -> Self {
        let context = BanditContextFeatures::from_provider_model_outcome(record);
        let success = record.status == ProviderModelOutcomeStatus::Passed;
        Self {
            action_id: record.action_id.clone(),
            context_key: context.context_key(),
            success,
            quality: if success { 1.0 } else { 0.0 },
            metrics: RewardMetrics {
                latency_ms: record.usage.latency_ms,
                cost_usd: record.usage.cost_usd,
                total_tokens: record.usage.total_tokens,
                retry_count: record.retry_count,
            },
        }
    }

    /// Build a reward observation from prompt/context section telemetry.
    #[must_use]
    pub fn from_section_outcome(record: &SectionOutcomeRecord) -> Self {
        let context = BanditContextFeatures::from_section_outcome(record);
        let success = record.status == SectionOutcomeStatus::Passed;
        Self {
            action_id: record.action_id.clone(),
            context_key: context.context_key(),
            success,
            quality: if success { 1.0 } else { 0.0 },
            metrics: RewardMetrics {
                latency_ms: None,
                cost_usd: None,
                total_tokens: Some(record.tokens_used as u64),
                retry_count: 0,
            },
        }
    }

    /// Convert quality and operational metrics into a bounded `[0, 1]` reward.
    #[must_use]
    pub fn bounded_reward(&self, bounds: &RewardNormalizationBounds) -> f64 {
        let quality = self.quality.clamp(0.0, 1.0);
        let latency_reward = self.metrics.latency_ms.map_or(1.0, |latency| {
            1.0 - ((latency as f64) / bounds.latency_sla_ms.max(1.0)).clamp(0.0, 1.0)
        });
        let cost_reward = self.metrics.cost_usd.map_or(1.0, |cost| {
            1.0 - (cost / bounds.max_cost_usd.max(f64::MIN_POSITIVE)).clamp(0.0, 1.0)
        });
        let token_reward = self.metrics.total_tokens.map_or(1.0, |tokens| {
            1.0 - ((tokens as f64) / bounds.max_tokens.max(1) as f64).clamp(0.0, 1.0)
        });
        let retry_reward = 1.0
            - (f64::from(self.metrics.retry_count) / f64::from(bounds.max_retries.max(1)))
                .clamp(0.0, 1.0);

        (quality * 0.60)
            + (latency_reward * 0.15)
            + (cost_reward * 0.15)
            + (token_reward * 0.05)
            + (retry_reward * 0.05)
    }
}

/// Normalization ceilings used to convert metrics into bounded rewards.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RewardNormalizationBounds {
    /// Latency SLA used to normalize latency penalty.
    pub latency_sla_ms: f64,
    /// Cost ceiling used to normalize cost penalty.
    pub max_cost_usd: f64,
    /// Token ceiling used to normalize token pressure.
    pub max_tokens: u64,
    /// Retry ceiling used to normalize retry pressure.
    pub max_retries: u32,
}

impl Default for RewardNormalizationBounds {
    fn default() -> Self {
        Self {
            latency_sla_ms: 120_000.0,
            max_cost_usd: 5.0,
            max_tokens: 200_000,
            max_retries: 5,
        }
    }
}

/// Bandit strategy used when policy selection is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BanditStrategy {
    /// Deterministic best-mean selection with bounded random exploration.
    ConservativeEpsilonGreedy,
}

/// React operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BanditPolicyMode {
    /// Deterministic first-safe action; updates are ignored.
    Disabled,
    /// Learn and emit candidate updates while selection stays deterministic.
    Shadow,
    /// Conservative exploration is allowed within safety bounds.
    Candidate,
}

/// Configuration for a contextual bandit policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanditPolicyConfig {
    /// Stable policy id.
    pub policy_id: String,
    /// React manifest version.
    pub policy_version: String,
    /// Operating mode.
    pub mode: BanditPolicyMode,
    /// Selection strategy.
    pub strategy: BanditStrategy,
    /// Exploration probability for epsilon-greedy candidate mode.
    pub exploration_rate: f64,
    /// Total observations required before non-deterministic selection.
    pub cold_start_observations: u64,
    /// Observations per action required before candidate updates can be emitted.
    pub candidate_min_observations: u64,
    /// Required reward lift over context baseline before emitting a candidate.
    pub candidate_min_reward_lift: f64,
    /// Reward normalization bounds.
    pub reward_bounds: RewardNormalizationBounds,
    /// Deterministic RNG seed for reproducible policy decisions.
    pub rng_seed: u64,
}

impl Default for BanditPolicyConfig {
    fn default() -> Self {
        Self {
            policy_id: "contextual-bandit.routing".to_string(),
            policy_version: "0.1.0".to_string(),
            mode: BanditPolicyMode::Disabled,
            strategy: BanditStrategy::ConservativeEpsilonGreedy,
            exploration_rate: 0.02,
            cold_start_observations: 25,
            candidate_min_observations: 20,
            candidate_min_reward_lift: 0.03,
            reward_bounds: RewardNormalizationBounds::default(),
            rng_seed: 0x524f_4b4f,
        }
    }
}

/// Per-action reward summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BanditActionStats {
    /// Number of observations.
    pub observations: u64,
    /// Number of successful terminal outcomes.
    pub successes: u64,
    /// Sum of bounded rewards.
    pub reward_sum: f64,
    /// Sum of observed latency values.
    pub latency_sum_ms: u64,
    /// Count of observations with latency.
    pub latency_observations: u64,
    /// Sum of observed costs.
    pub cost_sum_usd: f64,
    /// Count of observations with cost.
    pub cost_observations: u64,
    /// Sum of observed tokens.
    pub token_sum: u64,
    /// Sum of retry counts.
    pub retry_sum: u64,
}

impl BanditActionStats {
    /// Record one bounded reward observation.
    pub fn observe(&mut self, observation: &BanditRewardObservation, reward: f64) {
        self.observations = self.observations.saturating_add(1);
        if observation.success {
            self.successes = self.successes.saturating_add(1);
        }
        self.reward_sum += reward.clamp(0.0, 1.0);
        if let Some(latency) = observation.metrics.latency_ms {
            self.latency_sum_ms = self.latency_sum_ms.saturating_add(latency);
            self.latency_observations = self.latency_observations.saturating_add(1);
        }
        if let Some(cost) = observation.metrics.cost_usd {
            self.cost_sum_usd += cost.max(0.0);
            self.cost_observations = self.cost_observations.saturating_add(1);
        }
        if let Some(tokens) = observation.metrics.total_tokens {
            self.token_sum = self.token_sum.saturating_add(tokens);
        }
        self.retry_sum = self
            .retry_sum
            .saturating_add(u64::from(observation.metrics.retry_count));
    }

    /// Mean bounded reward.
    #[must_use]
    pub fn mean_reward(&self) -> f64 {
        ratio_f64(self.reward_sum, self.observations)
    }

    /// Terminal success rate.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        ratio_u64(self.successes, self.observations)
    }

    /// Average latency for observations that reported latency.
    #[must_use]
    pub fn avg_latency_ms(&self) -> Option<f64> {
        (self.latency_observations > 0)
            .then(|| self.latency_sum_ms as f64 / self.latency_observations as f64)
    }

    /// Average cost for observations that reported cost.
    #[must_use]
    pub fn avg_cost_usd(&self) -> Option<f64> {
        (self.cost_observations > 0).then(|| self.cost_sum_usd / self.cost_observations as f64)
    }
}

/// Summary for a proposed policy update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanditRewardSummary {
    /// Observations in the action/context evidence window.
    pub observations: u64,
    /// Successes in the action/context evidence window.
    pub successes: u64,
    /// Mean bounded reward for the action/context.
    pub mean_reward: f64,
    /// Baseline mean bounded reward for the context.
    pub baseline_mean_reward: f64,
    /// Reward lift over baseline.
    pub reward_lift: f64,
    /// Average latency, if observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_latency_ms: Option<f64>,
    /// Average cost, if observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_cost_usd: Option<f64>,
}

/// Admission status for a policy update record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyUpdateAdmissionStatus {
    /// Candidate is recorded for later admission review.
    Candidate,
    /// Candidate has been admitted by an explicit external process.
    Active,
    /// Candidate was rejected by an explicit external process.
    Rejected,
}

/// Candidate policy update emitted by the bandit learner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyUpdateCandidate {
    /// JSON schema version.
    pub schema_version: u32,
    /// Stable update id.
    pub update_id: String,
    /// React id the update targets.
    pub policy_id: String,
    /// React version the update was generated against.
    pub policy_version: String,
    /// Decision surface.
    pub decision_kind: BanditDecisionKind,
    /// Action proposed for promotion.
    pub action_id: String,
    /// Context bucket where the action performed well.
    pub context_key: String,
    /// Evidence window description.
    pub evidence_window: String,
    /// Reward summary supporting the candidate.
    pub reward_summary: BanditRewardSummary,
    /// Safety bounds that were applied before generating this update.
    pub safety_bounds: ActionSafetyBounds,
    /// Candidate admission status.
    pub admission_status: PolicyUpdateAdmissionStatus,
    /// Rollback path for future active policy mutation.
    pub rollback_path: String,
    /// RFC3339 creation timestamp.
    pub created_at: String,
}

/// Score and safety result for one considered action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanditActionDecisionDetail {
    /// Action id.
    pub action_id: String,
    /// Whether this action was selected.
    pub selected: bool,
    /// Mean reward score used for exploitation.
    pub score: f64,
    /// Number of observations for this action/context.
    pub observations: u64,
    /// Whether the action passed safety filters.
    pub safe: bool,
    /// Optional reason the action was filtered out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disqualified_reason: Option<String>,
}

/// Structured observable decision returned by a policy selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanditDecision {
    /// JSON schema version.
    pub schema_version: u32,
    /// Deterministic decision id.
    pub decision_id: String,
    /// React id used for selection.
    pub policy_id: String,
    /// React version used for selection.
    pub policy_version: String,
    /// Operating mode used for selection.
    pub mode: BanditPolicyMode,
    /// Context bucket key.
    pub context_key: String,
    /// Selected action id.
    pub selected_action_id: String,
    /// Whether this decision was exploratory.
    pub exploratory: bool,
    /// Machine-readable selection reason.
    pub reason: String,
    /// Considered action details.
    pub actions: Vec<BanditActionDecisionDetail>,
    /// RFC3339 decision timestamp.
    pub created_at: String,
}

/// Conservative contextual bandit policy state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextualBanditPolicy {
    /// React configuration.
    pub config: BanditPolicyConfig,
    /// Per-context, per-action statistics.
    pub stats: HashMap<String, HashMap<String, BanditActionStats>>,
    #[serde(default)]
    emitted_updates: HashSet<String>,
    #[serde(skip, default = "default_rng")]
    rng: ChaCha8Rng,
}

impl ContextualBanditPolicy {
    /// Create a policy from configuration.
    #[must_use]
    pub fn new(config: BanditPolicyConfig) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(config.rng_seed),
            config,
            stats: HashMap::new(),
            emitted_updates: HashSet::new(),
        }
    }

    /// Select an action for the supplied context and candidates.
    #[must_use]
    pub fn select_action(
        &mut self,
        context: &BanditContextFeatures,
        actions: &[BanditAction],
    ) -> BanditDecision {
        let context_key = context.context_key();
        let now = Utc::now().to_rfc3339();
        let mut details = self.decision_details(&context_key, context, actions);
        let safe_indices: Vec<usize> = details
            .iter()
            .enumerate()
            .filter_map(|(idx, detail)| detail.safe.then_some(idx))
            .collect();
        let selected_idx = if safe_indices.is_empty() {
            0
        } else {
            self.selected_index(&safe_indices, &details, actions)
        };
        let selected_action_id = details
            .get(selected_idx)
            .map(|detail| detail.action_id.clone())
            .unwrap_or_default();
        for (idx, detail) in details.iter_mut().enumerate() {
            detail.selected = idx == selected_idx;
        }
        let total_observations = self.total_observations_for_context(&context_key);
        let exploratory = self.config.mode == BanditPolicyMode::Candidate
            && total_observations >= self.config.cold_start_observations
            && details
                .get(selected_idx)
                .is_some_and(|detail| detail.score < self.best_safe_score(&details));
        let reason = self.decision_reason(total_observations, exploratory, safe_indices.is_empty());

        BanditDecision {
            schema_version: CONTEXTUAL_BANDIT_SCHEMA_VERSION,
            decision_id: stable_decision_id(&self.config.policy_id, &context_key, &now),
            policy_id: self.config.policy_id.clone(),
            policy_version: self.config.policy_version.clone(),
            mode: self.config.mode,
            context_key,
            selected_action_id,
            exploratory,
            reason,
            actions: details,
            created_at: now,
        }
    }

    /// Record one reward observation and return a candidate update when earned.
    pub fn record_reward(
        &mut self,
        observation: BanditRewardObservation,
        action_bounds: ActionSafetyBounds,
    ) -> Option<PolicyUpdateCandidate> {
        if self.config.mode == BanditPolicyMode::Disabled {
            return None;
        }
        let reward = observation.bounded_reward(&self.config.reward_bounds);
        let context_stats = self
            .stats
            .entry(observation.context_key.clone())
            .or_default();
        let action_stats = context_stats
            .entry(observation.action_id.clone())
            .or_default();
        action_stats.observe(&observation, reward);

        self.candidate_update_for(
            &observation.context_key,
            &observation.action_id,
            action_bounds,
        )
    }

    /// Return stats for an action/context pair.
    #[must_use]
    pub fn action_stats(&self, context_key: &str, action_id: &str) -> Option<&BanditActionStats> {
        self.stats
            .get(context_key)
            .and_then(|actions| actions.get(action_id))
    }

    /// Return the total observation count across all contexts.
    #[must_use]
    pub fn total_observations(&self) -> u64 {
        self.stats
            .values()
            .flat_map(HashMap::values)
            .map(|stats| stats.observations)
            .sum()
    }

    fn selected_index(
        &mut self,
        safe_indices: &[usize],
        details: &[BanditActionDecisionDetail],
        actions: &[BanditAction],
    ) -> usize {
        if self.config.mode == BanditPolicyMode::Disabled {
            return safe_indices[0];
        }

        let total_observations = details
            .iter()
            .map(|detail| detail.observations)
            .sum::<u64>();
        if self.config.mode == BanditPolicyMode::Shadow
            || total_observations < self.config.cold_start_observations
        {
            return safe_indices[0];
        }

        if self.config.strategy == BanditStrategy::ConservativeEpsilonGreedy
            && self
                .rng
                .gen_bool(self.config.exploration_rate.clamp(0.0, 1.0))
        {
            let exploratory: Vec<usize> = safe_indices
                .iter()
                .copied()
                .filter(|idx| {
                    details[*idx].safe
                        && actions
                            .get(*idx)
                            .is_some_and(|action| action.safety_bounds.allow_exploration)
                })
                .collect();
            if !exploratory.is_empty() {
                return exploratory[self.rng.gen_range(0..exploratory.len())];
            }
        }

        safe_indices
            .iter()
            .copied()
            .max_by(|left, right| {
                details[*left]
                    .score
                    .total_cmp(&details[*right].score)
                    .then_with(|| details[*right].action_id.cmp(&details[*left].action_id))
            })
            .unwrap_or(safe_indices[0])
    }

    fn decision_details(
        &self,
        context_key: &str,
        context: &BanditContextFeatures,
        actions: &[BanditAction],
    ) -> Vec<BanditActionDecisionDetail> {
        actions
            .iter()
            .map(|action| {
                let stats = self.action_stats(context_key, &action.action_id);
                let safety = safety_check(action, context, stats);
                BanditActionDecisionDetail {
                    action_id: action.action_id.clone(),
                    selected: false,
                    score: stats.map_or(0.0, BanditActionStats::mean_reward),
                    observations: stats.map_or(0, |stats| stats.observations),
                    safe: safety.is_none(),
                    disqualified_reason: safety,
                }
            })
            .collect()
    }

    fn candidate_update_for(
        &mut self,
        context_key: &str,
        action_id: &str,
        safety_bounds: ActionSafetyBounds,
    ) -> Option<PolicyUpdateCandidate> {
        let stats = self.action_stats(context_key, action_id)?.clone();
        if stats.observations < self.config.candidate_min_observations {
            return None;
        }
        let baseline = self.baseline_mean_reward(context_key, action_id);
        let lift = stats.mean_reward() - baseline;
        if lift < self.config.candidate_min_reward_lift {
            return None;
        }

        let update_id = stable_update_id(&self.config.policy_id, context_key, action_id);
        if !self.emitted_updates.insert(update_id.clone()) {
            return None;
        }

        Some(PolicyUpdateCandidate {
            schema_version: CONTEXTUAL_BANDIT_SCHEMA_VERSION,
            update_id,
            policy_id: self.config.policy_id.clone(),
            policy_version: self.config.policy_version.clone(),
            decision_kind: decision_kind_from_context_key(context_key),
            action_id: action_id.to_string(),
            context_key: context_key.to_string(),
            evidence_window: format!("last_{}_observations", stats.observations),
            reward_summary: BanditRewardSummary {
                observations: stats.observations,
                successes: stats.successes,
                mean_reward: stats.mean_reward(),
                baseline_mean_reward: baseline,
                reward_lift: lift,
                avg_latency_ms: stats.avg_latency_ms(),
                avg_cost_usd: stats.avg_cost_usd(),
            },
            safety_bounds,
            admission_status: PolicyUpdateAdmissionStatus::Candidate,
            rollback_path: format!(
                "remove candidate {} from policy {}",
                action_id, self.config.policy_id
            ),
            created_at: Utc::now().to_rfc3339(),
        })
    }

    fn total_observations_for_context(&self, context_key: &str) -> u64 {
        self.stats
            .get(context_key)
            .map(|actions| actions.values().map(|stats| stats.observations).sum())
            .unwrap_or(0)
    }

    fn baseline_mean_reward(&self, context_key: &str, excluded_action_id: &str) -> f64 {
        let Some(actions) = self.stats.get(context_key) else {
            return 0.0;
        };
        let mut reward_sum = 0.0;
        let mut observations = 0;
        for (action_id, stats) in actions {
            if action_id == excluded_action_id {
                continue;
            }
            reward_sum += stats.reward_sum;
            observations += stats.observations;
        }
        ratio_f64(reward_sum, observations)
    }

    fn best_safe_score(&self, details: &[BanditActionDecisionDetail]) -> f64 {
        details
            .iter()
            .filter(|detail| detail.safe)
            .map(|detail| detail.score)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    fn decision_reason(
        &self,
        total_observations: u64,
        exploratory: bool,
        no_safe_actions: bool,
    ) -> String {
        if no_safe_actions {
            return "no_safe_action_fallback".to_string();
        }
        match self.config.mode {
            BanditPolicyMode::Disabled => "disabled_deterministic".to_string(),
            BanditPolicyMode::Shadow => "shadow_deterministic".to_string(),
            BanditPolicyMode::Candidate
                if total_observations < self.config.cold_start_observations =>
            {
                "cold_start_deterministic".to_string()
            }
            BanditPolicyMode::Candidate if exploratory => "epsilon_explore".to_string(),
            BanditPolicyMode::Candidate => "epsilon_greedy_exploit".to_string(),
        }
    }
}

/// Append-only JSONL store for policy update candidates.
#[derive(Debug, Clone)]
pub struct PolicyUpdateCandidateStore {
    path: PathBuf,
    fsync: bool,
}

impl PolicyUpdateCandidateStore {
    /// Construct a store at `path`.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            fsync: true,
        }
    }

    /// Create parent directories and return a store at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if parent directory creation fails.
    pub async fn open_creating(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        Ok(Self { path, fsync: true })
    }

    /// Disable fsync after appends.
    #[must_use]
    pub const fn without_fsync(mut self) -> Self {
        self.fsync = false;
        self
    }

    /// Path to the underlying JSONL file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one policy update candidate as one JSON line.
    ///
    /// # Errors
    ///
    /// Returns an error for serialization or file I/O failures.
    pub async fn append(&self, candidate: &PolicyUpdateCandidate) -> io::Result<()> {
        let mut line = serde_json::to_string(candidate)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        if self.fsync {
            file.sync_data().await?;
        }
        Ok(())
    }

    /// Read all valid policy update candidates.
    ///
    /// # Errors
    ///
    /// Returns an error only for file open/read failures.
    pub async fn read_all(&self) -> io::Result<Vec<PolicyUpdateCandidate>> {
        read_policy_update_candidates(&self.path).await
    }
}

/// Read policy update candidates from JSONL.
///
/// Missing files produce an empty vector and malformed lines are skipped.
///
/// # Errors
///
/// Returns an error only for file open/read failures.
pub async fn read_policy_update_candidates(path: &Path) -> io::Result<Vec<PolicyUpdateCandidate>> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(candidate) = serde_json::from_str::<PolicyUpdateCandidate>(trimmed) {
            out.push(candidate);
        }
    }
    Ok(out)
}

fn safety_check(
    action: &BanditAction,
    context: &BanditContextFeatures,
    stats: Option<&BanditActionStats>,
) -> Option<String> {
    if !action.enabled {
        return Some("action_disabled".to_string());
    }
    let bounds = &action.safety_bounds;
    if let (Some(cost), Some(max)) = (action.estimated_cost_usd, bounds.max_cost_usd)
        && cost > max
    {
        return Some("estimated_cost_exceeds_bound".to_string());
    }
    if let (Some(latency), Some(max)) = (action.estimated_latency_ms, bounds.max_latency_ms)
        && latency > max
    {
        return Some("estimated_latency_exceeds_bound".to_string());
    }
    if let Some(max) = bounds.max_retry_count
        && context.retry_count > max
    {
        return Some("retry_count_exceeds_bound".to_string());
    }
    if let (Some(tokens), Some(max)) = (action.token_budget, bounds.max_token_budget)
        && tokens > max
    {
        return Some("token_budget_exceeds_bound".to_string());
    }
    if let (Some(share), Some(max)) = (action.budget_share, bounds.max_budget_share)
        && share > max
    {
        return Some("budget_share_exceeds_bound".to_string());
    }
    if let (Some(min_rate), Some(stats)) = (bounds.min_success_rate, stats)
        && stats.observations >= bounds.min_observations_for_rate
        && stats.success_rate() < min_rate
    {
        return Some("success_rate_below_bound".to_string());
    }
    None
}

fn default_rng() -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(0x524f_4b4f)
}

fn ratio_u64(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn ratio_f64(numerator: f64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator / denominator as f64
    }
}

fn normalize_key_part(value: &str) -> String {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '-'
                }
            })
            .collect()
    }
}

fn bucket_count(value: u32) -> &'static str {
    match value {
        0 => "0",
        1 => "1",
        2..=3 => "2_3",
        _ => "4_plus",
    }
}

fn stable_decision_id(policy_id: &str, context_key: &str, timestamp: &str) -> String {
    format!(
        "bandit-decision:{}:{}:{}",
        normalize_key_part(policy_id),
        stable_hash(context_key),
        stable_hash(timestamp)
    )
}

fn stable_update_id(policy_id: &str, context_key: &str, action_id: &str) -> String {
    format!(
        "policy-update:{}:{}:{}",
        normalize_key_part(policy_id),
        stable_hash(context_key),
        stable_hash(action_id)
    )
}

fn stable_hash(value: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn decision_kind_from_context_key(context_key: &str) -> BanditDecisionKind {
    if context_key.starts_with(BanditDecisionKind::PromptContextSectionInclusion.label()) {
        BanditDecisionKind::PromptContextSectionInclusion
    } else if context_key.starts_with(BanditDecisionKind::BidderBudgetShare.label()) {
        BanditDecisionKind::BidderBudgetShare
    } else if context_key.starts_with(BanditDecisionKind::ReviewerModelChoice.label()) {
        BanditDecisionKind::ReviewerModelChoice
    } else {
        BanditDecisionKind::ProviderModelRouting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate_config() -> BanditPolicyConfig {
        BanditPolicyConfig {
            mode: BanditPolicyMode::Candidate,
            exploration_rate: 0.0,
            cold_start_observations: 3,
            candidate_min_observations: 3,
            candidate_min_reward_lift: 0.02,
            ..BanditPolicyConfig::default()
        }
    }

    fn context() -> BanditContextFeatures {
        BanditContextFeatures::new(
            BanditDecisionKind::ProviderModelRouting,
            "implementation",
            "roko-learn",
            "implementer",
        )
    }

    fn actions() -> Vec<BanditAction> {
        vec![
            BanditAction::new("provider:cheap|model:small", "small"),
            BanditAction::new("provider:strong|model:large", "large"),
        ]
    }

    fn provider_record(passed: bool) -> ProviderModelOutcomeRecord {
        ProviderModelOutcomeRecord {
            schema_version: crate::provider_model_outcome::PROVIDER_MODEL_OUTCOME_SCHEMA_VERSION,
            timestamp: "2026-04-25T10:00:00Z".to_string(),
            action_id: "provider:zai|model:glm-5.1".to_string(),
            provider: "zai".to_string(),
            model: "glm-5.1".to_string(),
            task_id: "task-1".to_string(),
            task_type: "implementation".to_string(),
            role_id: Some("implementer".to_string()),
            status: if passed {
                ProviderModelOutcomeStatus::Passed
            } else {
                ProviderModelOutcomeStatus::Failed
            },
            gate_outcomes: vec![crate::provider_model_outcome::ProviderModelGateOutcome {
                gate_name: "cargo_test".to_string(),
                passed,
                score: None,
                duration_ms: None,
            }],
            retry_count: 0,
            usage: crate::provider_model_outcome::ProviderModelUsageTelemetry::default(),
            run_id: None,
        }
    }

    fn observation(context_key: &str, action_id: &str, success: bool) -> BanditRewardObservation {
        BanditRewardObservation {
            action_id: action_id.to_string(),
            context_key: context_key.to_string(),
            success,
            quality: if success { 1.0 } else { 0.0 },
            metrics: RewardMetrics {
                latency_ms: Some(if success { 1_000 } else { 3_000 }),
                cost_usd: Some(0.01),
                total_tokens: Some(1_000),
                retry_count: if success { 0 } else { 1 },
            },
        }
    }

    #[test]
    fn cold_start_selection_is_deterministic() {
        let mut policy = ContextualBanditPolicy::new(candidate_config());
        let decision = policy.select_action(&context(), &actions());

        assert_eq!(decision.selected_action_id, "provider:cheap|model:small");
        assert_eq!(decision.reason, "cold_start_deterministic");
        assert!(!decision.exploratory);
    }

    #[test]
    fn terminal_outcome_does_not_change_provider_context_key() {
        let passed = provider_record(true);
        let failed = provider_record(false);

        let passed_context = BanditContextFeatures::from_provider_model_outcome(&passed);
        let failed_context = BanditContextFeatures::from_provider_model_outcome(&failed);

        assert_eq!(passed_context.context_key(), failed_context.context_key());
        assert_eq!(
            BanditRewardObservation::from_provider_model_outcome(&passed).context_key,
            BanditRewardObservation::from_provider_model_outcome(&failed).context_key
        );
    }

    #[test]
    fn disabled_mode_ignores_rewards_and_uses_first_safe_action() {
        let cfg = BanditPolicyConfig {
            mode: BanditPolicyMode::Disabled,
            ..candidate_config()
        };
        let mut policy = ContextualBanditPolicy::new(cfg);
        let ctx = context();
        let update = policy.record_reward(
            observation(&ctx.context_key(), "provider:strong|model:large", true),
            ActionSafetyBounds::default(),
        );
        let decision = policy.select_action(&ctx, &actions());

        assert!(update.is_none());
        assert_eq!(decision.selected_action_id, "provider:cheap|model:small");
        assert_eq!(policy.total_observations(), 0);
    }

    #[test]
    fn reward_updates_drive_action_selection() {
        let mut policy = ContextualBanditPolicy::new(candidate_config());
        let ctx = context();
        let key = ctx.context_key();

        for _ in 0..3 {
            policy.record_reward(
                observation(&key, "provider:cheap|model:small", false),
                ActionSafetyBounds::default(),
            );
            policy.record_reward(
                observation(&key, "provider:strong|model:large", true),
                ActionSafetyBounds::default(),
            );
        }

        let decision = policy.select_action(&ctx, &actions());
        assert_eq!(decision.selected_action_id, "provider:strong|model:large");
        assert_eq!(decision.reason, "epsilon_greedy_exploit");
        assert!(
            policy
                .action_stats(&key, "provider:strong|model:large")
                .is_some_and(|stats| stats.success_rate() == 1.0)
        );
    }

    #[test]
    fn unsafe_actions_are_filtered_by_bounds() {
        let mut policy = ContextualBanditPolicy::new(candidate_config());
        let mut candidate_actions = actions();
        candidate_actions[0].estimated_cost_usd = Some(10.0);
        candidate_actions[0].safety_bounds.max_cost_usd = Some(1.0);

        let decision = policy.select_action(&context(), &candidate_actions);

        assert_eq!(decision.selected_action_id, "provider:strong|model:large");
        assert_eq!(
            decision.actions[0].disqualified_reason.as_deref(),
            Some("estimated_cost_exceeds_bound")
        );
    }

    #[test]
    fn reward_update_emits_candidate_policy_update() {
        let mut policy = ContextualBanditPolicy::new(candidate_config());
        let ctx = context();
        let key = ctx.context_key();
        let mut last_update = None;

        for _ in 0..3 {
            last_update = policy.record_reward(
                observation(&key, "provider:strong|model:large", true),
                ActionSafetyBounds::default(),
            );
        }

        let update = last_update.expect("candidate update");
        assert_eq!(
            update.admission_status,
            PolicyUpdateAdmissionStatus::Candidate
        );
        assert_eq!(update.action_id, "provider:strong|model:large");
        assert!(update.reward_summary.reward_lift > 0.0);

        let duplicate = policy.record_reward(
            observation(&key, "provider:strong|model:large", true),
            ActionSafetyBounds::default(),
        );
        assert!(duplicate.is_none());
    }

    #[tokio::test]
    async fn policy_update_store_roundtrips_jsonl() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("policy-updates.jsonl");
        let store = PolicyUpdateCandidateStore::open_creating(&path)
            .await
            .expect("open store")
            .without_fsync();
        let mut policy = ContextualBanditPolicy::new(candidate_config());
        let ctx = context();
        let key = ctx.context_key();
        let mut update = None;
        for _ in 0..3 {
            update = policy.record_reward(
                observation(&key, "provider:strong|model:large", true),
                ActionSafetyBounds::default(),
            );
        }
        let update = update.expect("candidate update");

        store.append(&update).await.expect("append");
        let loaded = store.read_all().await.expect("read");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].update_id, update.update_id);
        assert_eq!(loaded[0].action_id, update.action_id);
        assert!(
            (loaded[0].reward_summary.mean_reward - update.reward_summary.mean_reward).abs()
                < 0.000_001
        );
    }
}
