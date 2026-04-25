//! Provider/model outcome telemetry for future routing bandits.
//!
//! This module records observations only. It intentionally does not mutate
//! routing policy, static tables, or experiment state.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::efficiency::AgentEfficiencyEvent;
use crate::episode_logger::{Episode, GateVerdict};

/// Current JSON schema version for provider/model outcome records.
pub const PROVIDER_MODEL_OUTCOME_SCHEMA_VERSION: u32 = 1;

/// Terminal outcome status used by provider/model reward telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderModelOutcomeStatus {
    /// The attempt passed its final gate or terminal acceptance check.
    Passed,
    /// The attempt failed its final gate or terminal acceptance check.
    Failed,
    /// The attempt was blocked before a trustworthy verdict was available.
    Blocked,
    /// The attempt exceeded its time budget.
    TimedOut,
    /// The attempt was cancelled.
    Cancelled,
    /// The attempt requires a new plan.
    NeedsReplan,
    /// The attempt requires a retry.
    NeedsRetry,
    /// The attempt requires human review.
    NeedsHuman,
}

impl ProviderModelOutcomeStatus {
    /// Return whether this status counts as a pass-rate success.
    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Passed)
    }
}

/// One gate result joined to a model/provider choice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderModelGateOutcome {
    /// Gate identifier, such as `compile`, `test`, or `lint`.
    pub gate_name: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Optional numeric score when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    /// Optional wall-clock duration for the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl From<&GateVerdict> for ProviderModelGateOutcome {
    fn from(verdict: &GateVerdict) -> Self {
        Self {
            gate_name: verdict.gate.clone(),
            passed: verdict.passed,
            score: None,
            duration_ms: None,
        }
    }
}

/// Token, cost, and latency fields attached to a model/provider observation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderModelUsageTelemetry {
    /// Input tokens, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Output tokens, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Provider cache-read tokens, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    /// Provider cache-write tokens, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    /// Total billable-ish tokens for aggregation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// Cost in USD, if reported or estimated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Wall-clock latency for the model invocation or turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

/// One immutable provider/model outcome observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderModelOutcomeRecord {
    /// JSON schema version.
    pub schema_version: u32,
    /// ISO-8601 timestamp for when the observation was emitted.
    pub timestamp: String,
    /// Stable future-bandit action id for this provider/model choice.
    pub action_id: String,
    /// Provider identifier.
    pub provider: String,
    /// Model slug.
    pub model: String,
    /// Task identifier when available.
    #[serde(default)]
    pub task_id: String,
    /// Task type or category label used for aggregation.
    pub task_type: String,
    /// Role id/profile label when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    /// Overall outcome status.
    pub status: ProviderModelOutcomeStatus,
    /// Gate outcomes joined to this model/provider choice.
    #[serde(default)]
    pub gate_outcomes: Vec<ProviderModelGateOutcome>,
    /// Number of retries before this terminal result.
    pub retry_count: u32,
    /// Token/cost/latency fields, all optional.
    #[serde(default)]
    pub usage: ProviderModelUsageTelemetry,
    /// Optional run or session id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

impl ProviderModelOutcomeRecord {
    /// Build a stable action id for provider/model bandit arms.
    #[must_use]
    pub fn action_id(provider: &str, model: &str) -> String {
        format!(
            "provider:{}|model:{}",
            normalize_action_part(provider),
            normalize_action_part(model)
        )
    }

    /// Build an outcome record from a completed episode.
    #[must_use]
    pub fn from_episode(episode: &Episode, provider_override: Option<&str>) -> Option<Self> {
        let model = first_non_empty_owned([
            Some(episode.model.clone()),
            extra_string_ref(episode, "model"),
            extra_string_ref(episode, "model_used"),
        ])?;
        let provider = first_non_empty_owned([
            provider_override.map(ToString::to_string),
            Some(episode.backend.clone()),
            extra_string_ref(episode, "provider"),
            extra_string_ref(episode, "backend"),
        ])
        .unwrap_or_else(|| "unknown-provider".to_string());
        let task_type = first_non_empty_owned([
            extra_string_ref(episode, "task_type"),
            extra_string_ref(episode, "task_category"),
            Some(episode.trigger_kind.clone()),
            Some(episode.kind.clone()),
        ])
        .unwrap_or_else(|| "unknown".to_string());
        let role_id = first_non_empty_owned([
            extra_string_ref(episode, "role_id"),
            extra_string_ref(episode, "role"),
            Some(episode.agent_template.clone()),
        ]);
        let status = status_from_episode(episode);
        let retry_count = retry_count_from_episode(episode);
        let usage = ProviderModelUsageTelemetry {
            input_tokens: nonzero_u64(episode.usage.input_tokens),
            output_tokens: nonzero_u64(episode.usage.output_tokens),
            cache_read_tokens: nonzero_u64(episode.usage.cache_read_tokens),
            cache_write_tokens: nonzero_u64(episode.usage.cache_write_tokens),
            total_tokens: nonzero_u64(
                episode
                    .usage
                    .input_tokens
                    .saturating_add(episode.usage.output_tokens)
                    .saturating_add(episode.usage.cache_write_tokens),
            ),
            cost_usd: positive_f64(episode.usage.cost_usd),
            latency_ms: nonzero_u64(episode.usage.wall_ms),
        };

        Some(Self {
            schema_version: PROVIDER_MODEL_OUTCOME_SCHEMA_VERSION,
            timestamp: episode.completed_at.to_rfc3339(),
            action_id: Self::action_id(&provider, &model),
            provider,
            model,
            task_id: episode.task_id.clone(),
            task_type,
            role_id,
            status,
            gate_outcomes: episode.gate_verdicts.iter().map(Into::into).collect(),
            retry_count,
            usage,
            run_id: first_non_empty_owned([
                extra_string_ref(episode, "run_id"),
                extra_string_ref(episode, "session_id"),
                Some(episode.episode_id.clone()),
            ]),
        })
    }

    /// Build an outcome record from a persisted efficiency event.
    #[must_use]
    pub fn from_efficiency_event(event: &AgentEfficiencyEvent) -> Option<Self> {
        let model = first_non_empty([Some(event.model.as_str()), Some(event.model_used.as_str())])?;
        let provider =
            first_non_empty([Some(event.backend.as_str())]).unwrap_or("unknown-provider");
        let task_type = if event.outcome.trim().is_empty() {
            "unknown".to_string()
        } else {
            event.outcome.clone()
        };
        let retry_count = event.iteration.saturating_sub(1);
        let usage = ProviderModelUsageTelemetry {
            input_tokens: nonzero_u64(event.input_tokens),
            output_tokens: nonzero_u64(event.output_tokens),
            cache_read_tokens: nonzero_u64(event.cache_read_tokens),
            cache_write_tokens: nonzero_u64(event.cache_write_tokens),
            total_tokens: nonzero_u64(event.total_tokens()),
            cost_usd: positive_f64(event.cost_usd),
            latency_ms: nonzero_u64(event.duration_ms.max(event.wall_time_ms)),
        };

        Some(Self {
            schema_version: PROVIDER_MODEL_OUTCOME_SCHEMA_VERSION,
            timestamp: event.timestamp.clone(),
            action_id: Self::action_id(provider, model),
            provider: provider.to_string(),
            model: model.to_string(),
            task_id: event.task_id.clone(),
            task_type,
            role_id: non_empty(event.role.as_str()).map(ToString::to_string),
            status: if event.gate_passed {
                ProviderModelOutcomeStatus::Passed
            } else {
                ProviderModelOutcomeStatus::Failed
            },
            gate_outcomes: vec![ProviderModelGateOutcome {
                gate_name: "terminal".to_string(),
                passed: event.gate_passed,
                score: None,
                duration_ms: None,
            }],
            retry_count,
            usage,
            run_id: non_empty(event.plan_id.as_str()).map(ToString::to_string),
        })
    }
}

/// Append-only JSONL store for provider/model outcomes.
#[derive(Debug, Clone)]
pub struct ProviderModelOutcomeStore {
    path: PathBuf,
    fsync: bool,
}

impl ProviderModelOutcomeStore {
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

    /// Path to the underlying JSONL file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Disable fsync after appends.
    #[must_use]
    pub const fn without_fsync(mut self) -> Self {
        self.fsync = false;
        self
    }

    /// Append one record as one JSON line.
    ///
    /// # Errors
    ///
    /// Returns an error for serialization or file I/O failures.
    pub async fn append(&self, record: &ProviderModelOutcomeRecord) -> io::Result<()> {
        let mut line = serde_json::to_string(record)
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

    /// Read all valid records; malformed lines are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error only for file open/read failures.
    pub async fn read_all(&self) -> io::Result<Vec<ProviderModelOutcomeRecord>> {
        read_provider_model_outcomes(&self.path).await
    }
}

/// Read provider/model outcomes from a JSONL file.
///
/// Missing files produce an empty vector and malformed lines are skipped.
///
/// # Errors
///
/// Returns an error only for file open/read failures.
pub async fn read_provider_model_outcomes(
    path: &Path,
) -> io::Result<Vec<ProviderModelOutcomeRecord>> {
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
        if let Ok(record) = serde_json::from_str::<ProviderModelOutcomeRecord>(trimmed) {
            out.push(record);
        }
    }
    Ok(out)
}

/// Aggregate pass-rate summaries for a provider/model action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderModelPassRateSummary {
    /// Stable future-bandit action id.
    pub action_id: String,
    /// Provider identifier.
    pub provider: String,
    /// Model slug.
    pub model: String,
    /// Total observations in the full record set.
    pub observations: u64,
    /// Total successful observations in the full record set.
    pub successes: u64,
    /// Full-history pass rate.
    pub pass_rate: f64,
    /// Number of observations in the rolling window.
    pub rolling_observations: u64,
    /// Number of successes in the rolling window.
    pub rolling_successes: u64,
    /// Rolling pass rate.
    pub rolling_pass_rate: f64,
    /// Average latency across observations with latency.
    pub avg_latency_ms: Option<f64>,
    /// Total reported cost.
    pub total_cost_usd: f64,
    /// Total reported tokens.
    pub total_tokens: u64,
    /// Task type labels seen for this action.
    pub task_types: Vec<String>,
    /// Role labels seen for this action.
    pub role_ids: Vec<String>,
}

/// Response payload for rolling provider/model pass-rate summaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderModelPassRateReport {
    /// Rolling window size used for each action.
    pub window_size: usize,
    /// Total outcome records included in the report.
    pub total_records: usize,
    /// Per-action pass-rate summaries.
    pub actions: Vec<ProviderModelPassRateSummary>,
}

/// Build rolling summaries grouped by provider/model action id.
#[must_use]
pub fn summarize_provider_model_outcomes(
    records: &[ProviderModelOutcomeRecord],
    window_size: usize,
) -> ProviderModelPassRateReport {
    let window_size = window_size.max(1);
    let mut grouped: HashMap<String, Vec<&ProviderModelOutcomeRecord>> = HashMap::new();
    for record in records {
        grouped
            .entry(record.action_id.clone())
            .or_default()
            .push(record);
    }

    let mut actions: Vec<ProviderModelPassRateSummary> = grouped
        .into_values()
        .map(|mut group| {
            group.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
            summarize_group(&group, window_size)
        })
        .collect();
    actions.sort_by(|a, b| {
        b.rolling_pass_rate
            .partial_cmp(&a.rolling_pass_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.rolling_observations.cmp(&a.rolling_observations))
            .then_with(|| a.action_id.cmp(&b.action_id))
    });

    ProviderModelPassRateReport {
        window_size,
        total_records: records.len(),
        actions,
    }
}

fn summarize_group(
    group: &[&ProviderModelOutcomeRecord],
    window_size: usize,
) -> ProviderModelPassRateSummary {
    let first = group[0];
    let observations = group.len() as u64;
    let successes = group
        .iter()
        .filter(|record| record.status.is_success())
        .count() as u64;
    let window_start = group.len().saturating_sub(window_size);
    let rolling = &group[window_start..];
    let rolling_observations = rolling.len() as u64;
    let rolling_successes = rolling
        .iter()
        .filter(|record| record.status.is_success())
        .count() as u64;
    let latencies: Vec<u64> = group
        .iter()
        .filter_map(|record| record.usage.latency_ms)
        .collect();
    let avg_latency_ms = if latencies.is_empty() {
        None
    } else {
        Some(latencies.iter().sum::<u64>() as f64 / latencies.len() as f64)
    };
    let total_cost_usd = group
        .iter()
        .filter_map(|record| record.usage.cost_usd)
        .sum::<f64>();
    let total_tokens = group
        .iter()
        .filter_map(|record| record.usage.total_tokens)
        .sum::<u64>();

    ProviderModelPassRateSummary {
        action_id: first.action_id.clone(),
        provider: first.provider.clone(),
        model: first.model.clone(),
        observations,
        successes,
        pass_rate: ratio(successes, observations),
        rolling_observations,
        rolling_successes,
        rolling_pass_rate: ratio(rolling_successes, rolling_observations),
        avg_latency_ms,
        total_cost_usd,
        total_tokens,
        task_types: sorted_labels(group.iter().map(|record| record.task_type.as_str())),
        role_ids: sorted_labels(group.iter().filter_map(|record| record.role_id.as_deref())),
    }
}

fn sorted_labels<'a>(labels: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut set = HashSet::new();
    for label in labels {
        if let Some(label) = non_empty(label) {
            set.insert(label.to_string());
        }
    }
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort();
    out
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn status_from_episode(episode: &Episode) -> ProviderModelOutcomeStatus {
    if let Some(status) = extra_string_ref(episode, "provider_model_outcome_status")
        .as_deref()
        .and_then(parse_status)
    {
        return status;
    }
    if episode.success {
        ProviderModelOutcomeStatus::Passed
    } else {
        ProviderModelOutcomeStatus::Failed
    }
}

fn parse_status(raw: &str) -> Option<ProviderModelOutcomeStatus> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "passed" | "pass" | "success" => Some(ProviderModelOutcomeStatus::Passed),
        "failed" | "fail" | "failure" => Some(ProviderModelOutcomeStatus::Failed),
        "blocked" => Some(ProviderModelOutcomeStatus::Blocked),
        "timed_out" | "timeout" => Some(ProviderModelOutcomeStatus::TimedOut),
        "cancelled" | "canceled" => Some(ProviderModelOutcomeStatus::Cancelled),
        "needs_replan" => Some(ProviderModelOutcomeStatus::NeedsReplan),
        "needs_retry" => Some(ProviderModelOutcomeStatus::NeedsRetry),
        "needs_human" => Some(ProviderModelOutcomeStatus::NeedsHuman),
        _ => None,
    }
}

fn retry_count_from_episode(episode: &Episode) -> u32 {
    extra_u64(episode, "retry_count")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_else(|| episode.turns.saturating_sub(1).min(u64::from(u32::MAX)) as u32)
}

fn normalize_action_part(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(' ', "-")
}

fn first_non_empty<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Option<&'a str> {
    values.into_iter().flatten().find_map(non_empty)
}

fn first_non_empty_owned(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find_map(|value| non_empty(&value).map(ToString::to_string))
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn extra_string_ref(episode: &Episode, key: &str) -> Option<String> {
    episode
        .extra
        .get(key)
        .and_then(serde_json::Value::as_str)
        .and_then(non_empty)
        .map(ToString::to_string)
}

fn extra_u64(episode: &Episode, key: &str) -> Option<u64> {
    episode.extra.get(key).and_then(serde_json::Value::as_u64)
}

fn nonzero_u64(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

fn positive_f64(value: f64) -> Option<f64> {
    (value > 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::TempDir;

    use super::*;
    use crate::episode_logger::Usage;

    fn episode(success: bool, model: &str, provider: &str) -> Episode {
        let mut episode = Episode::new("agent-1", "task-1");
        episode.completed_at = Utc::now();
        episode.model = model.to_string();
        episode.backend = provider.to_string();
        episode.agent_template = "implementer".to_string();
        episode.success = success;
        episode.turns = 2;
        episode.usage = Usage {
            input_tokens: 100,
            output_tokens: 25,
            cache_read_tokens: 10,
            cache_write_tokens: 5,
            cost_usd: 0.42,
            cost_usd_without_cache: 0.5,
            wall_ms: 1_250,
        };
        episode
            .gate_verdicts
            .push(GateVerdict::new("cargo_test", success));
        episode.extra.insert(
            "task_category".to_string(),
            serde_json::json!("implementation"),
        );
        episode
            .extra
            .insert("role".to_string(), serde_json::json!("implementer"));
        episode
    }

    #[test]
    fn outcome_record_from_episode_joins_provider_model_gate_and_usage() {
        let record =
            ProviderModelOutcomeRecord::from_episode(&episode(true, "glm-5.1", "zai"), None)
                .expect("record");

        assert_eq!(record.provider, "zai");
        assert_eq!(record.model, "glm-5.1");
        assert_eq!(record.action_id, "provider:zai|model:glm-5.1");
        assert_eq!(record.status, ProviderModelOutcomeStatus::Passed);
        assert_eq!(record.gate_outcomes[0].gate_name, "cargo_test");
        assert_eq!(record.role_id.as_deref(), Some("implementer"));
        assert_eq!(record.task_type, "implementation");
        assert_eq!(record.retry_count, 1);
        assert_eq!(record.usage.total_tokens, Some(130));
        assert_eq!(record.usage.latency_ms, Some(1_250));
    }

    #[test]
    fn outcome_record_tolerates_missing_optional_metadata() {
        let mut episode = Episode::new("agent-1", "task-1");
        episode.model = "claude-sonnet-4-6".to_string();
        episode.success = false;

        let record =
            ProviderModelOutcomeRecord::from_episode(&episode, Some("anthropic")).expect("record");

        assert_eq!(record.provider, "anthropic");
        assert_eq!(record.role_id, None);
        assert_eq!(record.task_type, "unknown");
        assert!(record.gate_outcomes.is_empty());
        assert_eq!(record.retry_count, 0);
        assert_eq!(record.usage.cost_usd, None);
        assert_eq!(record.status, ProviderModelOutcomeStatus::Failed);
    }

    #[test]
    fn summarizer_computes_full_and_rolling_pass_rates() {
        let mut records = Vec::new();
        for success in [true, false, true] {
            records.push(
                ProviderModelOutcomeRecord::from_episode(&episode(success, "glm-5.1", "zai"), None)
                    .expect("record"),
            );
        }
        records.push(
            ProviderModelOutcomeRecord::from_episode(
                &episode(false, "claude-sonnet-4-6", "anthropic"),
                None,
            )
            .expect("record"),
        );

        let report = summarize_provider_model_outcomes(&records, 2);
        let glm = report
            .actions
            .iter()
            .find(|summary| summary.model == "glm-5.1")
            .expect("glm summary");

        assert_eq!(report.total_records, 4);
        assert_eq!(glm.observations, 3);
        assert_eq!(glm.successes, 2);
        assert!((glm.pass_rate - (2.0 / 3.0)).abs() < 1e-9);
        assert_eq!(glm.rolling_observations, 2);
        assert_eq!(glm.rolling_successes, 1);
        assert!((glm.rolling_pass_rate - 0.5).abs() < 1e-9);
        assert_eq!(glm.task_types, vec!["implementation"]);
        assert_eq!(glm.role_ids, vec!["implementer"]);
    }

    #[tokio::test]
    async fn store_appends_reads_and_skips_malformed_lines() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("provider-model-outcomes.jsonl");
        let store = ProviderModelOutcomeStore::at(&path).without_fsync();
        let record =
            ProviderModelOutcomeRecord::from_episode(&episode(true, "glm-5.1", "zai"), None)
                .expect("record");
        store.append(&record).await.expect("append");
        tokio::fs::write(
            &path,
            format!(
                "{}\n{{ bad json\n{}\n",
                serde_json::to_string(&record).expect("serialize"),
                serde_json::to_string(&record).expect("serialize"),
            ),
        )
        .await
        .expect("rewrite");

        let all = store.read_all().await.expect("read");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].action_id, "provider:zai|model:glm-5.1");
    }
}
