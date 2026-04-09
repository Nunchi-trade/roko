//! Offline dream-cycle orchestration.
//!
//! The dream cycle batches completed episodes, clusters them by plan/task
//! shape, distills the resulting groups into durable knowledge, promotes the
//! most reliable success clusters into playbooks, and writes a JSON report
//! for later inspection.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use roko_agent::{Agent, AgentResult, nl_to_format::NlToFormatConverter};
use roko_core::{Body, Context as RokoContext, Kind, Signal};
use roko_learn::{
    episode_logger::{Episode, EpisodeLogger, GateVerdict, Usage},
    playbook::{Playbook, PlaybookStep, PlaybookStore},
};
use roko_neuro::{KnowledgeEntry, KnowledgeKind, KnowledgeStore, tier_progression::{TierProgression, TierProgressionReport}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Agent hook used by the dream cycle to review a consolidation batch.
#[async_trait]
pub trait AgentDispatcher: Send + Sync {
    /// Dispatch a dream-review prompt through the configured agent.
    async fn dispatch(&self, input: &Signal, ctx: &RokoContext) -> AgentResult;
}

#[async_trait]
impl<T> AgentDispatcher for T
where
    T: Agent + Send + Sync,
{
    async fn dispatch(&self, input: &Signal, ctx: &RokoContext) -> AgentResult {
        self.run(input, ctx).await
    }
}

/// Summary of one completed dream cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamCycleReport {
    /// When the cycle started.
    pub started_at: DateTime<Utc>,
    /// When the cycle completed.
    pub completed_at: DateTime<Utc>,
    /// Number of episodes visible in the backing log.
    pub total_episodes: usize,
    /// Number of episodes included in this batch.
    pub processed_episodes: usize,
    /// Timestamp cutoff used to avoid reprocessing old episodes.
    pub processed_through: Option<DateTime<Utc>>,
    /// Batch analysis from the existing tier-progression pipeline.
    pub analysis: TierProgressionReport,
    /// Cluster summaries discovered during the dream cycle.
    pub clusters: Vec<DreamClusterReport>,
    /// Number of knowledge entries written to the durable store.
    pub knowledge_entries_written: usize,
    /// Number of playbooks written to the durable store.
    pub playbooks_created: usize,
    /// Failure-oriented knowledge entries created during the pass.
    pub regressions_detected: Vec<KnowledgeEntry>,
}

/// One cluster of episodes grouped by plan, task type, outcome, and model.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DreamClusterKey {
    /// Plan identifier.
    pub plan_id: String,
    /// Task category / task type.
    pub task_type: String,
    /// Successful or failed outcome.
    pub outcome: DreamOutcome,
    /// Model used for the clustered episodes.
    pub model: String,
}

impl DreamClusterKey {
    fn label(&self) -> String {
        format!(
            "plan={} task_type={} outcome={} model={}",
            self.plan_id, self.task_type, self.outcome, self.model
        )
    }
}

/// Outcome bucket for a dream cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamOutcome {
    /// Cluster contains successful episodes.
    Success,
    /// Cluster contains failed episodes.
    Failure,
}

impl std::fmt::Display for DreamOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => f.write_str("success"),
            Self::Failure => f.write_str("failure"),
        }
    }
}

/// Summary of one processed cluster.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamClusterReport {
    /// Grouping key for the cluster.
    pub key: DreamClusterKey,
    /// Number of episodes in the cluster.
    pub episode_count: usize,
    /// Number of successful episodes.
    pub success_count: usize,
    /// Number of failed episodes.
    pub failure_count: usize,
    /// First episode timestamp in the cluster.
    pub first_seen_at: DateTime<Utc>,
    /// Last episode timestamp in the cluster.
    pub last_seen_at: DateTime<Utc>,
    /// Episode ids that contributed to the cluster.
    pub episode_ids: Vec<String>,
    /// Entries distilled from the cluster context.
    pub knowledge_entries: Vec<KnowledgeEntry>,
    /// Playbook synthesized from repeated successful episodes.
    pub playbook: Option<Playbook>,
    /// Failure-oriented knowledge distilled from repeated failures.
    pub regression_entries: Vec<KnowledgeEntry>,
    /// Optional review emitted by the agent dispatcher.
    pub agent_review: Option<String>,
    /// Per-cluster warnings encountered during processing.
    pub warnings: Vec<String>,
}

/// Main offline learning process.
///
/// The cycle reads episode history, clusters it by plan/task/outcome/model,
/// distills each cluster with a haiku-tier agent pass, persists the resulting
/// knowledge, writes playbooks for repeated successful approaches, and emits
/// a JSON report.
pub struct DreamCycle {
    episode_store: Arc<EpisodeLogger>,
    knowledge_store: Arc<KnowledgeStore>,
    playbook_store: Arc<PlaybookStore>,
    dispatcher: Arc<dyn AgentDispatcher>,
    last_dream_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for DreamCycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DreamCycle")
            .field("episode_store", &self.episode_store.path())
            .field("knowledge_store", &self.knowledge_store.path())
            .field("playbook_store", &self.playbook_store.root())
            .field("dispatcher", &"<dispatcher>")
            .field("last_dream_at", &self.last_dream_at)
            .finish()
    }
}

impl DreamCycle {
    /// Construct a dream cycle around the existing stores and dispatcher.
    #[must_use]
    pub fn new(
        episode_store: Arc<EpisodeLogger>,
        knowledge_store: Arc<KnowledgeStore>,
        playbook_store: Arc<PlaybookStore>,
        dispatcher: Arc<dyn AgentDispatcher>,
    ) -> Self {
        Self {
            episode_store,
            knowledge_store,
            playbook_store,
            dispatcher,
            last_dream_at: None,
        }
    }

    /// Last completed cycle time, if any.
    #[must_use]
    pub const fn last_dream_at(&self) -> Option<DateTime<Utc>> {
        self.last_dream_at
    }

    /// Run a full offline learning pass.
    ///
    /// # Errors
    ///
    /// Returns an error if the episode log cannot be read, the stores cannot
    /// be updated, or the report cannot be written.
    pub async fn run(&mut self) -> Result<DreamCycleReport> {
        let started_at = Utc::now();
        let all_episodes = EpisodeLogger::read_all_lossy(self.episode_store.path())
            .await
            .with_context(|| {
                format!("read episode log from {}", self.episode_store.path().display())
            })?;
        let total_episodes = all_episodes.len();
        let cutoff = self.last_dream_at;
        let mut batch: Vec<_> = all_episodes
            .into_iter()
            .filter(|episode| cutoff.map(|ts| episode.timestamp > ts).unwrap_or(true))
            .collect();
        batch.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.id.cmp(&right.id))
        });

        let processed_through = batch.iter().map(|episode| episode.timestamp).max();
        let analysis = TierProgression::default().analyze(&batch);
        let mut clusters = cluster_episodes(batch);
        let mut written_knowledge_ids = BTreeSet::new();

        let mut knowledge_entries_written = 0usize;
        let mut playbooks_created = 0usize;
        let mut regressions_detected = Vec::new();

        for cluster in &mut clusters {
            let outcome = process_cluster(
                cluster,
                &self.dispatcher,
                &self.knowledge_store,
                &self.playbook_store,
                &mut written_knowledge_ids,
                started_at,
            )
            .await?;
            knowledge_entries_written += outcome.knowledge_entries_written;
            playbooks_created += usize::from(outcome.playbook_created);
            regressions_detected.extend(outcome.regression_entries.iter().cloned());
            cluster.knowledge_entries = outcome.knowledge_entries;
            cluster.playbook = outcome.playbook;
            cluster.regression_entries = outcome.regression_entries;
            cluster.agent_review = outcome.agent_review;
            cluster.warnings = outcome.warnings;
        }

        let report = DreamCycleReport {
            started_at,
            completed_at: Utc::now(),
            total_episodes,
            processed_episodes: clusters.iter().map(|cluster| cluster.episode_count).sum(),
            processed_through,
            analysis,
            clusters: clusters.iter().map(DreamClusterReport::from).collect(),
            knowledge_entries_written,
            playbooks_created,
            regressions_detected,
        };

        self.write_report(&report).await?;

        self.last_dream_at = Some(processed_through.unwrap_or(started_at));

        Ok(report)
    }

    async fn write_report(&self, report: &DreamCycleReport) -> Result<()> {
        let path = dream_report_path(self.episode_store.path(), report.started_at);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dream report directory {}", parent.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(report).context("serialize dream report")?;
        std::fs::write(&path, bytes)
            .with_context(|| format!("write dream report to {}", path.display()))?;
        Ok(())
    }
}

#[derive(Debug)]
struct ClusterOutcome {
    knowledge_entries_written: usize,
    knowledge_entries: Vec<KnowledgeEntry>,
    playbook_created: bool,
    playbook: Option<Playbook>,
    regression_entries: Vec<KnowledgeEntry>,
    agent_review: Option<String>,
    warnings: Vec<String>,
}

async fn process_cluster(
    cluster: &DreamCluster,
    dispatcher: &Arc<dyn AgentDispatcher>,
    knowledge_store: &Arc<KnowledgeStore>,
    playbook_store: &Arc<PlaybookStore>,
    written_knowledge_ids: &mut BTreeSet<String>,
    started_at: DateTime<Utc>,
) -> Result<ClusterOutcome> {
    let mut outcome = ClusterOutcome {
        knowledge_entries_written: 0,
        knowledge_entries: Vec::new(),
        playbook_created: false,
        playbook: None,
        regression_entries: Vec::new(),
        agent_review: None,
        warnings: Vec::new(),
    };

    let prompt = build_cluster_prompt(cluster, started_at)?;
    let signal = Signal::builder(Kind::Prompt)
        .body(Body::text(prompt))
        .build();
    let response = dispatcher.dispatch(&signal, &RokoContext::now()).await;
    let review_text = response.output.body.as_text().unwrap_or("").trim().to_string();
    if !review_text.is_empty() {
        outcome.agent_review = Some(review_text.clone());
    }

    let distilled_entries = match parse_cluster_response(&review_text, &cluster.episode_ids) {
        Ok(entries) => entries,
        Err(error) => {
            outcome
                .warnings
                .push(format!("failed to parse agent review: {error}"));
            Vec::new()
        }
    };
    for entry in distilled_entries {
        if written_knowledge_ids.insert(entry.id.clone()) {
            knowledge_store.add(entry.clone())?;
            outcome.knowledge_entries_written += 1;
            outcome.knowledge_entries.push(entry);
        }
    }

    if cluster.success_count > 3 {
        let playbook = build_playbook(cluster, started_at);
        playbook_store
            .save(&playbook)
            .await
            .context("save dream playbook")?;
        let playbook_entry = playbook_knowledge_entry(&playbook, &cluster.episode_ids, started_at);
        knowledge_store.add(playbook_entry.clone())?;
        outcome.knowledge_entries_written += 1;
        outcome.playbook_created = true;
        outcome.playbook = Some(playbook);
        outcome.knowledge_entries.push(playbook_entry);
    }

    if cluster.failure_count > 2 {
        let regression = build_regression_entry(cluster, started_at);
        knowledge_store.add(regression.clone())?;
        outcome.knowledge_entries_written += 1;
        outcome.regression_entries.push(regression);
    }

    if response.success {
        return Ok(outcome);
    }

    if let Some(text) = outcome.agent_review.as_deref() {
        outcome
            .warnings
            .push(format!("agent review returned a non-empty response: {text}"));
    } else {
        outcome
            .warnings
            .push("agent review returned an empty response".to_string());
    }
    Ok(outcome)
}

#[derive(Debug, Clone)]
struct DreamCluster {
    key: DreamClusterKey,
    episodes: Vec<Episode>,
    episode_ids: Vec<String>,
    episode_count: usize,
    success_count: usize,
    failure_count: usize,
    first_seen_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    knowledge_entries: Vec<KnowledgeEntry>,
    playbook: Option<Playbook>,
    regression_entries: Vec<KnowledgeEntry>,
    agent_review: Option<String>,
    warnings: Vec<String>,
}

impl From<&DreamCluster> for DreamClusterReport {
    fn from(cluster: &DreamCluster) -> Self {
        Self {
            key: cluster.key.clone(),
            episode_count: cluster.episode_count,
            success_count: cluster.success_count,
            failure_count: cluster.failure_count,
            first_seen_at: cluster.first_seen_at,
            last_seen_at: cluster.last_seen_at,
            episode_ids: cluster.episode_ids.clone(),
            knowledge_entries: cluster.knowledge_entries.clone(),
            playbook: cluster.playbook.clone(),
            regression_entries: cluster.regression_entries.clone(),
            agent_review: cluster.agent_review.clone(),
            warnings: cluster.warnings.clone(),
        }
    }
}

fn cluster_episodes(episodes: Vec<Episode>) -> Vec<DreamCluster> {
    let mut by_key: BTreeMap<DreamClusterKey, Vec<Episode>> = BTreeMap::new();
    for episode in episodes {
        let key = DreamClusterKey {
            plan_id: episode_plan_id(&episode),
            task_type: episode_task_type(&episode),
            outcome: if episode.success {
                DreamOutcome::Success
            } else {
                DreamOutcome::Failure
            },
            model: episode_model(&episode),
        };
        by_key.entry(key).or_default().push(episode);
    }

    by_key
        .into_iter()
        .map(|(key, mut episodes)| {
            episodes.sort_by(|left, right| {
                left.timestamp
                    .cmp(&right.timestamp)
                    .then_with(|| left.id.cmp(&right.id))
            });
            let episode_ids = episodes
                .iter()
                .map(|episode| episode_source_id(episode).to_string())
                .collect::<Vec<_>>();
            let episode_count = episodes.len();
            let success_count = episodes.iter().filter(|episode| episode.success).count();
            let failure_count = episode_count.saturating_sub(success_count);
            let first_seen_at = episodes
                .first()
                .map(|episode| episode.timestamp)
                .unwrap_or_else(Utc::now);
            let last_seen_at = episodes
                .last()
                .map(|episode| episode.timestamp)
                .unwrap_or(first_seen_at);
            DreamCluster {
                key,
                episodes,
                episode_ids,
                episode_count,
                success_count,
                failure_count,
                first_seen_at,
                last_seen_at,
                knowledge_entries: Vec::new(),
                playbook: None,
                regression_entries: Vec::new(),
                agent_review: None,
                warnings: Vec::new(),
            }
        })
        .collect()
}

fn build_cluster_prompt(cluster: &DreamCluster, started_at: DateTime<Utc>) -> Result<String> {
    let episodes: Vec<DreamEpisodeRecord> = cluster
        .episodes
        .iter()
        .map(DreamEpisodeRecord::from_episode)
        .collect();
    let corpus_json = serde_json::to_string_pretty(&episodes)?;
    let schema = dream_distillation_schema();
    let extractor = NlToFormatConverter::new();
    Ok(format!(
        "You are Roko's haiku-tier dream distiller.\n\
         Review this cluster and answer:\n\
         - What patterns do you see?\n\
         - What knowledge should be extracted?\n\
         - What failed repeatedly?\n\n\
         Cluster key: {}\n\
         Cycle start: {}\n\
         Episode corpus:\n\
         ```json\n{}\n```\n\n\
         Return only structured JSON that matches the schema below.\n{}\n",
        cluster.key.label(),
        started_at.to_rfc3339(),
        corpus_json,
        extractor.extraction_prompt(&schema),
    ))
}

#[derive(Debug, Serialize)]
struct DreamEpisodeRecord {
    source_id: String,
    id: String,
    episode_id: String,
    kind: String,
    agent_id: String,
    task_id: String,
    plan_id: String,
    task_type: String,
    outcome: String,
    input_signal_hash: String,
    output_signal_hash: String,
    model: String,
    trigger_kind: String,
    success: bool,
    turns: u64,
    tokens_used: u64,
    duration_secs: f64,
    failure_reason: Option<String>,
    gate_verdicts: Vec<GateVerdict>,
    usage: Usage,
    external_actions: Vec<Value>,
    headline: bool,
    extra: Value,
    timestamp: chrono::DateTime<Utc>,
    started_at: chrono::DateTime<Utc>,
    completed_at: chrono::DateTime<Utc>,
}

impl DreamEpisodeRecord {
    fn from_episode(episode: &Episode) -> Self {
        Self {
            source_id: episode_source_id(episode).to_string(),
            id: episode.id.clone(),
            episode_id: episode.episode_id.clone(),
            kind: episode.kind.clone(),
            agent_id: episode.agent_id.clone(),
            task_id: episode.task_id.clone(),
            plan_id: episode_plan_id(episode),
            task_type: episode_task_type(episode),
            outcome: if episode.success {
                "success".to_string()
            } else {
                "failure".to_string()
            },
            input_signal_hash: episode.input_signal_hash.clone(),
            output_signal_hash: episode.output_signal_hash.clone(),
            model: episode_model(episode),
            trigger_kind: episode.trigger_kind.clone(),
            success: episode.success,
            turns: episode.turns,
            tokens_used: episode.tokens_used,
            duration_secs: episode.duration_secs,
            failure_reason: episode.failure_reason.clone(),
            gate_verdicts: episode.gate_verdicts.clone(),
            usage: episode.usage.clone(),
            external_actions: episode.external_actions.clone(),
            headline: episode.headline,
            extra: json!(&episode.extra),
            timestamp: episode.timestamp,
            started_at: episode.started_at,
            completed_at: episode.completed_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct DreamDistillationEnvelope {
    #[serde(default, alias = "knowledge", alias = "candidates", alias = "items")]
    entries: Vec<DreamDistillationCandidate>,
}

#[derive(Debug, Deserialize)]
struct DreamDistillationCandidate {
    #[serde(default)]
    kind: KnowledgeKind,
    #[serde(default)]
    content: String,
    #[serde(default = "default_candidate_confidence")]
    confidence: f64,
    #[serde(default, alias = "episode_ids", alias = "source_episode_ids")]
    source_episodes: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    half_life_days: Option<f64>,
}

impl DreamDistillationCandidate {
    fn into_entry(mut self, fallback_sources: &[String]) -> Option<KnowledgeEntry> {
        let content = self.content.trim();
        if content.is_empty() {
            return None;
        }

        if self.source_episodes.is_empty() {
            self.source_episodes.extend(fallback_sources.iter().cloned());
        }

        self.source_episodes.sort();
        self.source_episodes.dedup();

        let kind_tag = knowledge_kind_tag(self.kind);
        if !self.tags.iter().any(|tag| tag == kind_tag) {
            self.tags.push(kind_tag.to_string());
        }
        self.tags.sort();
        self.tags.dedup();

        let confidence = self.confidence.clamp(0.0, 1.0);
        let half_life_days = self
            .half_life_days
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or_else(|| self.kind.default_half_life_days());

        Some(KnowledgeEntry {
            id: derive_knowledge_id(self.kind, content, &self.source_episodes, &self.tags),
            kind: self.kind,
            content: content.to_string(),
            confidence,
            source_episodes: self.source_episodes,
            tags: self.tags,
            created_at: Utc::now(),
            half_life_days,
            hdc_vector: None,
        })
    }
}

fn parse_cluster_response(response: &str, fallback_sources: &[String]) -> Result<Vec<KnowledgeEntry>> {
    let schema = dream_distillation_schema();
    let extractor = NlToFormatConverter::new();
    let extracted = extractor
        .convert(response, &schema)
        .context("extract dream JSON from model response")?;
    let envelope: DreamDistillationEnvelope =
        serde_json::from_value(extracted).context("decode dream JSON envelope")?;
    Ok(envelope
        .entries
        .into_iter()
        .filter_map(|candidate| candidate.into_entry(fallback_sources))
        .collect())
}

fn dream_distillation_schema() -> Value {
    json!({
        "type": "object",
        "required": ["entries"],
        "properties": {
            "entries": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string" },
                        "content": { "type": "string" },
                        "confidence": { "type": "number" },
                        "source_episodes": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "half_life_days": { "type": "number" }
                    }
                }
            }
        }
    })
}

fn build_playbook(cluster: &DreamCluster, started_at: DateTime<Utc>) -> Playbook {
    let mut playbook = Playbook::new(
        playbook_id_for(cluster),
        format!(
            "For tasks of type {}, this approach works: reuse the successful cluster pattern.",
            cluster.key.task_type
        ),
    );
    playbook.name = format!(
        "Dream playbook {} / {} / {}",
        cluster.key.plan_id, cluster.key.task_type, cluster.key.model
    );
    playbook.steps = vec![
        PlaybookStep::new(
            0,
            format!(
                "Anchor the work in plan {} and task type {}.",
                cluster.key.plan_id, cluster.key.task_type
            ),
            "align_context",
            vec![
                format!("plan:{}", cluster.key.plan_id),
                format!("task_type:{}", cluster.key.task_type),
            ],
        ),
        PlaybookStep::new(
            1,
            format!(
                "Use the approach that produced {} successful episode(s) with model {}.",
                cluster.success_count, cluster.key.model
            ),
            "repeat_successful_sequence",
            vec![
                format!("model:{}", cluster.key.model),
                "outcome:success".to_string(),
            ],
        ),
        PlaybookStep::new(
            2,
            format!(
                "Verify the gates that stayed green in the repeated successful runs: {}.",
                summarize_success_gates(cluster)
            ),
            "verify_success_criteria",
            summarize_success_gate_signals(cluster),
        ),
    ];
    playbook.created_at_ms = started_at.timestamp_millis();
    playbook
}

fn playbook_knowledge_entry(
    playbook: &Playbook,
    source_episodes: &[String],
    created_at: DateTime<Utc>,
) -> KnowledgeEntry {
    let content = render_playbook_content(playbook);
    KnowledgeEntry {
        id: derive_knowledge_id(
            KnowledgeKind::Playbook,
            &content,
            source_episodes,
            &["playbook".to_string(), "dream".to_string()],
        ),
        kind: KnowledgeKind::Playbook,
        content,
        confidence: if playbook.steps.is_empty() { 0.0 } else { 1.0 },
        source_episodes: source_episodes.to_vec(),
        tags: vec![
            "dream".to_string(),
            "playbook".to_string(),
            "task-reusable".to_string(),
        ],
        created_at,
        half_life_days: KnowledgeKind::Playbook.default_half_life_days(),
        hdc_vector: None,
    }
}

fn build_regression_entry(cluster: &DreamCluster, created_at: DateTime<Utc>) -> KnowledgeEntry {
    let reason = summarize_failure_reason(cluster);
    let content = format!(
        "Approach {} for plan {} and task type {} does not work because {}.",
        cluster.key.model, cluster.key.plan_id, cluster.key.task_type, reason
    );
    let kind = if cluster.success_count == 0 {
        KnowledgeKind::Constraint
    } else {
        KnowledgeKind::AntiKnowledge
    };
    KnowledgeEntry {
        id: derive_knowledge_id(
            kind,
            &content,
            &cluster.episode_ids,
            &[knowledge_kind_tag(kind).to_string()],
        ),
        kind,
        content,
        confidence: if cluster.failure_count > 0 { 0.9 } else { 0.0 },
        source_episodes: cluster.episode_ids.clone(),
        tags: vec![
            knowledge_kind_tag(kind).to_string(),
            "dream".to_string(),
            "regression".to_string(),
            format!("plan:{}", cluster.key.plan_id),
            format!("task_type:{}", cluster.key.task_type),
            format!("model:{}", cluster.key.model),
        ],
        created_at,
        half_life_days: kind.default_half_life_days(),
        hdc_vector: None,
    }
}

fn summarize_failure_reason(cluster: &DreamCluster) -> String {
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for episode in cluster.episodes.iter().filter(|episode| !episode.success) {
        if let Some(reason) = episode
            .failure_reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
        {
            *reasons.entry(reason.to_string()).or_insert(0) += 1;
        }
    }

    if let Some((reason, _)) = reasons
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
    {
        return reason;
    }

    let failing_gates = summarize_failure_gates(cluster);
    if !failing_gates.is_empty() {
        return format!("the same gates kept failing: {failing_gates}");
    }

    "the cluster repeatedly failed without a more specific recorded reason".to_string()
}

fn summarize_success_gates(cluster: &DreamCluster) -> String {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for episode in cluster.episodes.iter().filter(|episode| episode.success) {
        for verdict in &episode.gate_verdicts {
            if verdict.passed {
                *counts.entry(verdict.gate.clone()).or_insert(0) += 1;
            }
        }
    }

    let mut items: Vec<(String, usize)> = counts.into_iter().collect();
    items.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let gates: Vec<String> = items
        .into_iter()
        .take(3)
        .map(|(gate, count)| format!("{gate} ({count})"))
        .collect();
    if gates.is_empty() {
        "recorded success criteria".to_string()
    } else {
        gates.join(", ")
    }
}

fn summarize_success_gate_signals(cluster: &DreamCluster) -> Vec<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for episode in cluster.episodes.iter().filter(|episode| episode.success) {
        for verdict in &episode.gate_verdicts {
            if verdict.passed {
                *counts.entry(format!("gate:{}", verdict.gate)).or_insert(0) += 1;
            }
        }
    }

    let mut items: Vec<(String, usize)> = counts.into_iter().collect();
    items.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let mut signals: Vec<String> = items.into_iter().take(3).map(|(gate, _)| gate).collect();
    if signals.is_empty() {
        signals.push("outcome:success".to_string());
    }
    signals
}

fn summarize_failure_gates(cluster: &DreamCluster) -> String {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for episode in cluster.episodes.iter().filter(|episode| !episode.success) {
        for verdict in &episode.gate_verdicts {
            if !verdict.passed {
                *counts.entry(verdict.gate.clone()).or_insert(0) += 1;
            }
        }
    }

    let mut items: Vec<(String, usize)> = counts.into_iter().collect();
    items.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    items
        .into_iter()
        .take(3)
        .map(|(gate, count)| format!("{gate} ({count})"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_playbook_content(playbook: &Playbook) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", playbook.name));
    out.push_str(&format!("Goal: {}\n\n", playbook.goal));
    for step in &playbook.steps {
        out.push_str(&format!(
            "{}. {} [{}]\n",
            step.index + 1,
            step.description,
            step.action_kind
        ));
        if !step.expected_signals.is_empty() {
            out.push_str(&format!(
                "   expected: {}\n",
                step.expected_signals.join(", ")
            ));
        }
    }
    out
}

fn dream_report_path(episode_path: &Path, started_at: DateTime<Utc>) -> PathBuf {
    dream_root_path(episode_path)
        .join("dreams")
        .join(format!("dream-{}.json", started_at.timestamp_millis()))
}

fn dream_root_path(path: &Path) -> PathBuf {
    let mut ancestor = path;
    while let Some(parent) = ancestor.parent() {
        if parent.file_name() == Some(OsStr::new(".roko")) {
            return parent.to_path_buf();
        }
        ancestor = parent;
    }
    path.parent().unwrap_or(path).to_path_buf()
}

fn episode_plan_id(episode: &Episode) -> String {
    extra_string(episode, "plan_id").unwrap_or_else(|| {
        if episode.task_id.trim().is_empty() {
            "unknown-plan".to_string()
        } else {
            episode.task_id.clone()
        }
    })
}

fn episode_task_type(episode: &Episode) -> String {
    extra_string(episode, "task_category")
        .or_else(|| extra_string(episode, "task_type"))
        .or_else(|| extra_string(episode, "complexity_band"))
        .unwrap_or_else(|| {
            if episode.agent_template.trim().is_empty() {
                "unknown-task".to_string()
            } else {
                episode.agent_template.clone()
            }
        })
}

fn episode_model(episode: &Episode) -> String {
    if !episode.model.trim().is_empty() {
        episode.model.clone()
    } else {
        extra_string(episode, "model").unwrap_or_else(|| "unknown-model".to_string())
    }
}

fn extra_string(episode: &Episode, key: &str) -> Option<String> {
    episode
        .extra
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn episode_source_id(episode: &Episode) -> &str {
    if episode.episode_id.trim().is_empty() {
        &episode.id
    } else {
        &episode.episode_id
    }
}

fn default_candidate_confidence() -> f64 {
    0.75
}

fn knowledge_kind_tag(kind: KnowledgeKind) -> &'static str {
    match kind {
        KnowledgeKind::Fact => "fact",
        KnowledgeKind::Insight => "insight",
        KnowledgeKind::Procedure => "procedure",
        KnowledgeKind::Heuristic => "heuristic",
        KnowledgeKind::Playbook => "playbook",
        KnowledgeKind::Constraint => "constraint",
        KnowledgeKind::AntiKnowledge => "anti_knowledge",
    }
}

fn derive_knowledge_id(
    kind: KnowledgeKind,
    content: &str,
    source_episodes: &[String],
    tags: &[String],
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    knowledge_kind_tag(kind).hash(&mut hasher);
    content.hash(&mut hasher);
    for source in source_episodes {
        source.hash(&mut hasher);
    }
    for tag in tags {
        tag.hash(&mut hasher);
    }
    format!("dream_{:016x}", hasher.finish())
}

fn playbook_id_for(cluster: &DreamCluster) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cluster.key.plan_id.hash(&mut hasher);
    cluster.key.task_type.hash(&mut hasher);
    cluster.key.model.hash(&mut hasher);
    cluster.key.outcome.hash(&mut hasher);
    format!("dream-playbook-{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[derive(Debug)]
    struct MockDispatcher {
        response: String,
    }

    #[async_trait]
    impl AgentDispatcher for MockDispatcher {
        async fn dispatch(&self, _input: &Signal, _ctx: &RokoContext) -> AgentResult {
            AgentResult::ok(
                Signal::builder(Kind::Prompt)
                    .body(Body::text(self.response.clone()))
                    .build(),
            )
        }
    }

    fn episode(
        id: &str,
        plan_id: &str,
        task_type: &str,
        model: &str,
        success: bool,
        failure_reason: Option<&str>,
    ) -> Episode {
        let mut episode = Episode::new("agent-a", id);
        episode.id = id.to_string();
        episode.episode_id = id.to_string();
        episode.task_id = format!("task-{id}");
        episode.kind = "agent_turn".to_string();
        episode.model = model.to_string();
        episode.success = success;
        episode.failure_reason = failure_reason.map(ToOwned::to_owned);
        episode.extra.insert("plan_id".to_string(), json!(plan_id));
        episode
            .extra
            .insert("task_category".to_string(), json!(task_type));
        episode.gate_verdicts = vec![GateVerdict::new("compile", success)];
        episode
    }

    async fn write_episode(logger: &EpisodeLogger, episode: &Episode) {
        logger.append(episode).await.expect("append episode");
    }

    #[tokio::test]
    async fn run_clusters_and_writes_report() {
        let tmp = TempDir::new().expect("tempdir");
        let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
        let knowledge_path = tmp.path().join(".roko").join("neuro").join("knowledge.jsonl");
        let playbooks_root = tmp.path().join(".roko").join("playbooks");
        let logger = EpisodeLogger::new(&episodes_path);
        let knowledge_store = Arc::new(KnowledgeStore::new(&knowledge_path));
        let playbook_store = Arc::new(PlaybookStore::new(&playbooks_root));
        let dispatcher = Arc::new(MockDispatcher {
            response: r#"<|json|>{"entries":[{"kind":"insight","content":"clustered episodes prefer the same compile-first approach","confidence":0.8,"tags":["dream","cluster"],"source_episodes":["ep-1"]}]}<|/json|>"#.to_string(),
        });

        for idx in 0..4 {
            let ep = episode(
                &format!("ep-{idx}"),
                "plan-a",
                "implementation",
                "claude-haiku-4-5",
                true,
                None,
            );
            write_episode(&logger, &ep).await;
        }
        for idx in 0..3 {
            let ep = episode(
                &format!("fail-{idx}"),
                "plan-b",
                "docs",
                "claude-haiku-4-5",
                false,
                Some("missing rollback"),
            );
            write_episode(&logger, &ep).await;
        }

        let mut cycle = DreamCycle::new(
            Arc::new(logger),
            knowledge_store.clone(),
            playbook_store.clone(),
            dispatcher,
        );

        let report = cycle.run().await.expect("run");
        assert_eq!(report.processed_episodes, 7);
        assert_eq!(report.clusters.len(), 2);
        assert_eq!(report.playbooks_created, 1);
        assert!(!report.regressions_detected.is_empty());
        assert!(cycle.last_dream_at().is_some());

        let report_dir = tmp.path().join(".roko").join("dreams");
        let mut entries = tokio::fs::read_dir(&report_dir).await.expect("dream dir");
        assert!(entries.next_entry().await.expect("next").is_some());

        let saved_playbooks = playbook_store.list().await.expect("list playbooks");
        assert_eq!(saved_playbooks.len(), 1);
        assert!(saved_playbooks[0].goal.contains("task type"));

        let store = KnowledgeStore::new(&knowledge_path);
        let knowledge_entries = store.query("dream", 10).expect("query");
        assert!(!knowledge_entries.is_empty());
    }
}
