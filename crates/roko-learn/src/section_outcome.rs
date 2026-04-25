//! Prompt/context section outcome telemetry for future adaptive policy.
//!
//! This module records observations only. It does not mutate prompt, context,
//! model, or routing policy.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use roko_core::{
    CognitiveWorkspace, ContextRejectionAudit, ContextSectionAudit, InvocationGateOutcome,
    InvocationReviewVerdictOutcome, PromptSectionAudit,
};
use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Current JSON schema version for section outcome records.
pub const SECTION_OUTCOME_SCHEMA_VERSION: u32 = 1;

/// Section surface represented by an outcome observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    /// Prompt section assembled into or dropped from a final prompt.
    Prompt,
    /// Context section admitted into the prompt.
    ContextIncluded,
    /// Context candidate rejected before prompt assembly.
    ContextRejected,
}

/// Terminal outcome status used by section reward telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionOutcomeStatus {
    /// The invocation passed its terminal gate or review.
    Passed,
    /// The invocation failed its terminal gate or review.
    Failed,
    /// The invocation was blocked before a trustworthy verdict was available.
    Blocked,
    /// The invocation exceeded its time budget.
    TimedOut,
    /// The invocation was cancelled.
    Cancelled,
    /// The invocation requires a new plan.
    NeedsReplan,
    /// The invocation requires a retry.
    NeedsRetry,
    /// The invocation requires human review.
    NeedsHuman,
}

impl SectionOutcomeStatus {
    /// Return whether this status counts as a pass-rate success.
    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Passed)
    }
}

/// One gate result joined to a section observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionGateOutcome {
    /// Gate identifier.
    pub gate_id: String,
    /// Outcome status string emitted by the gate.
    pub outcome: String,
    /// Whether this gate blocks completion.
    pub required: bool,
}

impl From<&InvocationGateOutcome> for SectionGateOutcome {
    fn from(value: &InvocationGateOutcome) -> Self {
        Self {
            gate_id: value.gate_id.clone(),
            outcome: value.outcome.clone(),
            required: value.required,
        }
    }
}

/// One review verdict joined to a section observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionReviewVerdictOutcome {
    /// Stable verdict id.
    pub verdict_id: String,
    /// Reviewer role id.
    pub reviewer_role_id: String,
    /// Verdict status.
    pub status: String,
    /// Confidence in `[0.0, 1.0]` when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Required next action.
    pub required_next_action: String,
    /// Number of blocking findings.
    pub blocking_findings: usize,
    /// Number of non-blocking findings.
    pub non_blocking_findings: usize,
}

impl From<&InvocationReviewVerdictOutcome> for SectionReviewVerdictOutcome {
    fn from(value: &InvocationReviewVerdictOutcome) -> Self {
        Self {
            verdict_id: value.verdict_id.clone(),
            reviewer_role_id: value.reviewer_role_id.clone(),
            status: value.status.clone(),
            confidence: value.confidence,
            required_next_action: value.required_next_action.clone(),
            blocking_findings: value.blocking_findings.len(),
            non_blocking_findings: value.non_blocking_findings.len(),
        }
    }
}

/// One immutable prompt/context section outcome observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionOutcomeRecord {
    /// JSON schema version.
    pub schema_version: u32,
    /// ISO-8601 timestamp for when the observation was emitted.
    pub timestamp: String,
    /// Workspace id that produced the section.
    pub workspace_id: String,
    /// Invocation id that produced the section.
    pub invocation_id: String,
    /// Stable task id.
    pub task_id: String,
    /// Task type or category label used for aggregation.
    pub task_type: String,
    /// Role id/profile label.
    pub role_id: String,
    /// Provider identifier.
    pub provider: String,
    /// Model slug.
    pub model: String,
    /// Stable section id.
    pub section_id: String,
    /// Human-readable section name.
    pub section_name: String,
    /// Stable future-bandit action id.
    pub action_id: String,
    /// Section surface.
    pub section_kind: SectionKind,
    /// Whether the section was included in the prompt.
    pub included: bool,
    /// Estimated section tokens before final prompt rendering.
    pub estimated_tokens: usize,
    /// Tokens used in the final prompt. Dropped sections use zero.
    pub tokens_used: usize,
    /// Section token budget, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<usize>,
    /// Stable source type key, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    /// Stable source id, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Experiment id, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    /// Overall invocation outcome.
    pub status: SectionOutcomeStatus,
    /// Gate outcomes joined to this section.
    #[serde(default)]
    pub gate_outcomes: Vec<SectionGateOutcome>,
    /// Review verdicts joined to this section.
    #[serde(default)]
    pub review_verdicts: Vec<SectionReviewVerdictOutcome>,
}

impl SectionOutcomeRecord {
    /// Build section outcome records from a cognitive workspace.
    #[must_use]
    pub fn from_workspace(
        workspace: &CognitiveWorkspace,
        task_type: impl Into<String>,
    ) -> Vec<Self> {
        let task_type = task_type.into();
        let timestamp = chrono::Utc::now().to_rfc3339();
        let status = status_from_workspace(workspace);
        let gate_outcomes = workspace
            .gate_outcomes
            .iter()
            .map(SectionGateOutcome::from)
            .collect::<Vec<_>>();
        let review_verdicts = workspace
            .review_verdicts
            .iter()
            .map(SectionReviewVerdictOutcome::from)
            .collect::<Vec<_>>();
        let mut records = Vec::new();

        records.extend(workspace.prompt_sections.iter().map(|section| {
            Self::from_prompt_section(
                workspace,
                &timestamp,
                &task_type,
                status,
                &gate_outcomes,
                &review_verdicts,
                section,
            )
        }));
        records.extend(workspace.included_context_sections.iter().map(|section| {
            Self::from_included_context(
                workspace,
                &timestamp,
                &task_type,
                status,
                &gate_outcomes,
                &review_verdicts,
                section,
            )
        }));
        records.extend(workspace.rejected_context_candidates.iter().map(|section| {
            Self::from_rejected_context(
                workspace,
                &timestamp,
                &task_type,
                status,
                &gate_outcomes,
                &review_verdicts,
                section,
            )
        }));

        records
    }

    fn common(
        workspace: &CognitiveWorkspace,
        timestamp: &str,
        task_type: &str,
        status: SectionOutcomeStatus,
        gate_outcomes: &[SectionGateOutcome],
        review_verdicts: &[SectionReviewVerdictOutcome],
    ) -> Self {
        Self {
            schema_version: SECTION_OUTCOME_SCHEMA_VERSION,
            timestamp: timestamp.to_string(),
            workspace_id: workspace.workspace_id.clone(),
            invocation_id: workspace.invocation_id.clone(),
            task_id: workspace.task_contract.task_id.clone(),
            task_type: task_type.to_string(),
            role_id: workspace.role_profile.id.clone(),
            provider: workspace.model_choice.provider.clone(),
            model: workspace.model_choice.model.clone(),
            section_id: String::new(),
            section_name: String::new(),
            action_id: String::new(),
            section_kind: SectionKind::Prompt,
            included: false,
            estimated_tokens: 0,
            tokens_used: 0,
            token_budget: None,
            source_type: None,
            source_id: None,
            experiment_id: None,
            status,
            gate_outcomes: gate_outcomes.to_vec(),
            review_verdicts: review_verdicts.to_vec(),
        }
    }

    fn from_prompt_section(
        workspace: &CognitiveWorkspace,
        timestamp: &str,
        task_type: &str,
        status: SectionOutcomeStatus,
        gate_outcomes: &[SectionGateOutcome],
        review_verdicts: &[SectionReviewVerdictOutcome],
        section: &PromptSectionAudit,
    ) -> Self {
        let mut record = Self::common(
            workspace,
            timestamp,
            task_type,
            status,
            gate_outcomes,
            review_verdicts,
        );
        record.section_id = section.section_id.clone();
        record.section_name = section.section_name.clone();
        record.action_id = section.action_id.clone();
        record.section_kind = SectionKind::Prompt;
        record.included = section.included;
        record.estimated_tokens = section.estimated_tokens;
        record.tokens_used = section.tokens_used;
        record.token_budget = section.token_budget;
        record.source_type = section.source_type.clone();
        record.source_id = section.source_id.clone();
        record.experiment_id = section.experiment_id.clone();
        record
    }

    fn from_included_context(
        workspace: &CognitiveWorkspace,
        timestamp: &str,
        task_type: &str,
        status: SectionOutcomeStatus,
        gate_outcomes: &[SectionGateOutcome],
        review_verdicts: &[SectionReviewVerdictOutcome],
        section: &ContextSectionAudit,
    ) -> Self {
        let mut record = Self::common(
            workspace,
            timestamp,
            task_type,
            status,
            gate_outcomes,
            review_verdicts,
        );
        record.section_id = section.section_id.clone();
        record.section_name = section.section_name.clone();
        record.action_id = section.action_id.clone();
        record.section_kind = SectionKind::ContextIncluded;
        record.included = true;
        record.estimated_tokens = section.estimated_tokens;
        record.tokens_used = section.estimated_tokens;
        record.token_budget = section.token_budget;
        record.source_type = Some(section.source_type.clone());
        record.source_id = section.source_id.clone();
        record.experiment_id = section.experiment_id.clone();
        record
    }

    fn from_rejected_context(
        workspace: &CognitiveWorkspace,
        timestamp: &str,
        task_type: &str,
        status: SectionOutcomeStatus,
        gate_outcomes: &[SectionGateOutcome],
        review_verdicts: &[SectionReviewVerdictOutcome],
        section: &ContextRejectionAudit,
    ) -> Self {
        let mut record = Self::common(
            workspace,
            timestamp,
            task_type,
            status,
            gate_outcomes,
            review_verdicts,
        );
        record.section_id = section.section_id.clone();
        record.section_name = section.section_name.clone();
        record.action_id = section.action_id.clone();
        record.section_kind = SectionKind::ContextRejected;
        record.included = false;
        record.estimated_tokens = section.estimated_tokens;
        record.tokens_used = 0;
        record.source_type = Some(section.source_type.clone());
        record.source_id = section.source_id.clone();
        record.experiment_id = section.experiment_id.clone();
        record
    }
}

/// Append-only JSONL store for prompt/context section outcomes.
#[derive(Debug, Clone)]
pub struct SectionOutcomeStore {
    path: PathBuf,
    fsync: bool,
}

impl SectionOutcomeStore {
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

    /// Append records as JSON lines.
    ///
    /// # Errors
    ///
    /// Returns an error for serialization or file I/O failures.
    pub async fn append_many(&self, records: &[SectionOutcomeRecord]) -> io::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut payload = String::new();
        for record in records {
            payload.push_str(
                &serde_json::to_string(record)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
            );
            payload.push('\n');
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(payload.as_bytes()).await?;
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
    pub async fn read_all(&self) -> io::Result<Vec<SectionOutcomeRecord>> {
        read_section_outcomes(&self.path).await
    }
}

/// Read section outcomes from a JSONL file.
///
/// Missing files produce an empty vector and malformed lines are skipped.
///
/// # Errors
///
/// Returns an error only for file open/read failures.
pub async fn read_section_outcomes(path: &Path) -> io::Result<Vec<SectionOutcomeRecord>> {
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
        if let Ok(record) = serde_json::from_str::<SectionOutcomeRecord>(trimmed) {
            out.push(record);
        }
    }
    Ok(out)
}

/// Aggregate pass-rate summary for a prompt/context section action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionOutcomeSummary {
    /// Stable future-bandit action id.
    pub action_id: String,
    /// Stable section id.
    pub section_id: String,
    /// Human-readable section name.
    pub section_name: String,
    /// Section surface.
    pub section_kind: SectionKind,
    /// Total observations in the full record set.
    pub observations: u64,
    /// Total successful observations in the full record set.
    pub successes: u64,
    /// Full-history pass rate.
    pub pass_rate: f64,
    /// Average estimated tokens.
    pub avg_estimated_tokens: f64,
    /// Average used tokens.
    pub avg_tokens_used: f64,
    /// Task type labels seen for this action.
    pub task_types: Vec<String>,
    /// Role labels seen for this action.
    pub role_ids: Vec<String>,
    /// Provider/model labels seen for this action.
    pub provider_models: Vec<String>,
    /// Experiment ids seen for this action.
    pub experiment_ids: Vec<String>,
}

/// Response payload for section outcome summaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionOutcomeReport {
    /// Total outcome records included in the report.
    pub total_records: usize,
    /// Per-section action summaries.
    pub sections: Vec<SectionOutcomeSummary>,
}

/// Build summaries grouped by section action id.
#[must_use]
pub fn summarize_section_outcomes(records: &[SectionOutcomeRecord]) -> SectionOutcomeReport {
    let mut grouped: HashMap<String, Vec<&SectionOutcomeRecord>> = HashMap::new();
    for record in records {
        grouped
            .entry(record.action_id.clone())
            .or_default()
            .push(record);
    }

    let mut sections: Vec<_> = grouped
        .into_values()
        .map(|group| summarize_group(&group))
        .collect();
    sections.sort_by(|a, b| {
        b.pass_rate
            .partial_cmp(&a.pass_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.observations.cmp(&a.observations))
            .then_with(|| a.action_id.cmp(&b.action_id))
    });

    SectionOutcomeReport {
        total_records: records.len(),
        sections,
    }
}

fn summarize_group(group: &[&SectionOutcomeRecord]) -> SectionOutcomeSummary {
    let first = group[0];
    let observations = group.len() as u64;
    let successes = group
        .iter()
        .filter(|record| record.status.is_success())
        .count() as u64;
    let estimated_tokens = group
        .iter()
        .map(|record| record.estimated_tokens as u64)
        .sum::<u64>();
    let tokens_used = group
        .iter()
        .map(|record| record.tokens_used as u64)
        .sum::<u64>();

    SectionOutcomeSummary {
        action_id: first.action_id.clone(),
        section_id: first.section_id.clone(),
        section_name: first.section_name.clone(),
        section_kind: first.section_kind,
        observations,
        successes,
        pass_rate: ratio(successes, observations),
        avg_estimated_tokens: ratio(estimated_tokens, observations),
        avg_tokens_used: ratio(tokens_used, observations),
        task_types: sorted_labels(group.iter().map(|record| record.task_type.as_str())),
        role_ids: sorted_labels(group.iter().map(|record| record.role_id.as_str())),
        provider_models: sorted_owned_labels(
            group
                .iter()
                .map(|record| format!("{}/{}", record.provider, record.model)),
        ),
        experiment_ids: sorted_labels(
            group
                .iter()
                .filter_map(|record| record.experiment_id.as_deref()),
        ),
    }
}

fn status_from_workspace(workspace: &CognitiveWorkspace) -> SectionOutcomeStatus {
    if let Some(status) = workspace
        .review_verdicts
        .iter()
        .map(|verdict| verdict.status.as_str())
        .find(|status| !status.eq_ignore_ascii_case("passed"))
    {
        return status_from_str(status);
    }
    if let Some(outcome) = workspace
        .gate_outcomes
        .iter()
        .filter(|gate| gate.required)
        .map(|gate| gate.outcome.as_str())
        .find(|outcome| !outcome.eq_ignore_ascii_case("passed"))
    {
        return status_from_str(outcome);
    }
    if workspace
        .gate_outcomes
        .iter()
        .any(|gate| gate.outcome.eq_ignore_ascii_case("passed"))
        || workspace
            .review_verdicts
            .iter()
            .any(|verdict| verdict.status.eq_ignore_ascii_case("passed"))
    {
        SectionOutcomeStatus::Passed
    } else {
        SectionOutcomeStatus::Blocked
    }
}

fn status_from_str(value: &str) -> SectionOutcomeStatus {
    match value.trim().to_ascii_lowercase().as_str() {
        "passed" | "pass" | "success" => SectionOutcomeStatus::Passed,
        "failed" | "fail" | "rejected" => SectionOutcomeStatus::Failed,
        "timed_out" | "timeout" => SectionOutcomeStatus::TimedOut,
        "cancelled" | "canceled" => SectionOutcomeStatus::Cancelled,
        "needs_replan" => SectionOutcomeStatus::NeedsReplan,
        "needs_retry" => SectionOutcomeStatus::NeedsRetry,
        "needs_human" => SectionOutcomeStatus::NeedsHuman,
        _ => SectionOutcomeStatus::Blocked,
    }
}

fn sorted_labels<'a>(labels: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut set = HashSet::new();
    for label in labels {
        let trimmed = label.trim();
        if !trimmed.is_empty() {
            set.insert(trimmed.to_string());
        }
    }
    let mut out: Vec<_> = set.into_iter().collect();
    out.sort();
    out
}

fn sorted_owned_labels(labels: impl Iterator<Item = String>) -> Vec<String> {
    let mut set = HashSet::new();
    for label in labels {
        let trimmed = label.trim();
        if !trimmed.is_empty() {
            set.insert(trimmed.to_string());
        }
    }
    let mut out: Vec<_> = set.into_iter().collect();
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

#[cfg(test)]
mod tests {
    use roko_core::{
        CognitiveWorkspace, ContextScopeAudit, ContextSectionAudit, InvocationGateOutcome,
        InvocationReviewVerdictOutcome, ModelChoice, PolicyVersionRef, PromptSectionAudit,
        TaskInvocationContract,
    };

    use super::*;

    #[test]
    fn section_records_join_metadata_to_gates_and_reviews_without_raw_prompt() {
        let mut workspace = base_workspace();
        workspace.prompt_sections.push(PromptSectionAudit {
            section_id: "prompt:workspace-map".into(),
            section_name: "workspace_map".into(),
            action_id: "prompt_section:workspace-map|experiment:exp-a".into(),
            included: true,
            estimated_tokens: 20,
            tokens_used: 20,
            token_budget: Some(100),
            priority: "high".into(),
            cache_layer: "workspace".into(),
            placement: "middle".into(),
            bidder: "task_context".into(),
            source_type: Some("manifest".into()),
            source_id: Some("role-policy".into()),
            provenance: Some("policy".into()),
            experiment_id: Some("exp-a".into()),
            reason: "included_by_prompt_assembler".into(),
        });
        workspace
            .included_context_sections
            .push(ContextSectionAudit {
                section_id: "context:source:file:src-lib-rs".into(),
                section_name: "source".into(),
                action_id: "context_section:source:file:src-lib-rs".into(),
                source_type: "file".into(),
                source_id: Some("src/lib.rs".into()),
                purpose: "source_evidence".into(),
                scope: ContextScopeAudit::Task {
                    plan_id: "P1".into(),
                    task_id: "T1".into(),
                },
                inclusion_reason: "inspect public API".into(),
                estimated_tokens: 42,
                token_budget: Some(128),
                experiment_id: None,
            });
        workspace.gate_outcomes.push(InvocationGateOutcome {
            gate_id: "cargo_test".into(),
            outcome: "passed".into(),
            required: true,
            command: Some("cargo test".into()),
            summary: None,
        });
        workspace
            .review_verdicts
            .push(InvocationReviewVerdictOutcome {
                verdict_id: "rv-1".into(),
                reviewer_role_id: "reviewer".into(),
                status: "passed".into(),
                confidence: Some(0.9),
                blocking_findings: Vec::new(),
                non_blocking_findings: vec!["nit".into()],
                required_next_action: "none".into(),
                evidence_refs: vec!["review.json".into()],
            });

        let records = SectionOutcomeRecord::from_workspace(&workspace, "coding_task");
        assert_eq!(records.len(), 2);
        assert!(
            records
                .iter()
                .all(|record| record.status == SectionOutcomeStatus::Passed)
        );
        assert!(
            records
                .iter()
                .all(|record| record.gate_outcomes[0].gate_id == "cargo_test")
        );
        assert!(
            records
                .iter()
                .all(|record| record.review_verdicts[0].verdict_id == "rv-1")
        );

        let encoded = serde_json::to_string(&records).expect("serialize records");
        assert!(!encoded.contains("secret prompt"));
    }

    #[test]
    fn section_summary_aggregates_by_action_role_provider_and_task_type() {
        let mut first = sample_record("prompt_section:workspace-map", true);
        first.task_type = "coding_task".into();
        first.role_id = "implementer".into();
        first.provider = "codex".into();
        first.model = "gpt-5.5".into();
        let mut second = sample_record("prompt_section:workspace-map", false);
        second.task_type = "review".into();
        second.role_id = "reviewer".into();
        second.provider = "openai".into();
        second.model = "gpt-5.4".into();

        let report = summarize_section_outcomes(&[first, second]);
        assert_eq!(report.total_records, 2);
        assert_eq!(report.sections.len(), 1);
        let summary = &report.sections[0];
        assert_eq!(summary.observations, 2);
        assert_eq!(summary.successes, 1);
        assert_eq!(summary.task_types, vec!["coding_task", "review"]);
        assert_eq!(summary.role_ids, vec!["implementer", "reviewer"]);
        assert_eq!(
            summary.provider_models,
            vec!["codex/gpt-5.5", "openai/gpt-5.4"]
        );
    }

    #[tokio::test]
    async fn section_outcome_store_roundtrips_jsonl() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("section-outcomes.jsonl");
        let store = SectionOutcomeStore::open_creating(&path)
            .await
            .expect("open store")
            .without_fsync();
        let record = sample_record("prompt_section:task", true);

        store
            .append_many(std::slice::from_ref(&record))
            .await
            .expect("append");
        let loaded = store.read_all().await.expect("read");

        assert_eq!(loaded, vec![record]);
    }

    fn base_workspace() -> CognitiveWorkspace {
        CognitiveWorkspace::new(
            "cw-1",
            "invoke-1",
            TaskInvocationContract::from_prompt_context(Some("P1".into()), "T1", "Task"),
            PolicyVersionRef::new("implementer", "1.0.0"),
            PolicyVersionRef::new("prompt.implementer", "1.0.0"),
            ModelChoice::new("codex", "gpt-5.5"),
        )
    }

    fn sample_record(action_id: &str, passed: bool) -> SectionOutcomeRecord {
        SectionOutcomeRecord {
            schema_version: SECTION_OUTCOME_SCHEMA_VERSION,
            timestamp: "2026-04-25T00:00:00Z".into(),
            workspace_id: "cw-1".into(),
            invocation_id: "invoke-1".into(),
            task_id: "T1".into(),
            task_type: "coding_task".into(),
            role_id: "implementer".into(),
            provider: "codex".into(),
            model: "gpt-5.5".into(),
            section_id: "prompt:workspace-map".into(),
            section_name: "workspace_map".into(),
            action_id: action_id.into(),
            section_kind: SectionKind::Prompt,
            included: true,
            estimated_tokens: 10,
            tokens_used: 10,
            token_budget: Some(100),
            source_type: None,
            source_id: None,
            experiment_id: None,
            status: if passed {
                SectionOutcomeStatus::Passed
            } else {
                SectionOutcomeStatus::Failed
            },
            gate_outcomes: Vec::new(),
            review_verdicts: Vec::new(),
        }
    }
}
