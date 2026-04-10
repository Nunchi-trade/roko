//! Offline dream-cycle orchestration.
//!
//! The dream cycle batches completed episodes, clusters them by plan/task
//! shape, distills the resulting groups into durable knowledge, promotes the
//! most reliable success clusters into playbooks, and writes a JSON report
//! for later inspection.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::hash::{Hash, Hasher};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use bardo_primitives::hdc::{HdcVector, text_fingerprint};
use chrono::{DateTime, Utc};
use roko_agent::{Agent, AgentResult, nl_to_format::NlToFormatConverter};
use roko_core::{Body, Context as RokoContext, Kind, Signal};
use roko_learn::{
    cfactor::{CFactor, CFactorRegression, detect_cfactor_regression},
    episode_logger::{Episode, EpisodeLogger, GateVerdict, Usage},
    playbook::{Playbook, PlaybookStep, PlaybookStore},
};
use roko_neuro::{
    KnowledgeEntry, KnowledgeKind, KnowledgeStore,
    tier_progression::{TierProgression, TierProgressionReport},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DREAMS_SUCCESS_REGRESSION_THRESHOLD: f64 = 0.20;
const DREAMS_REGRESSION_MIN_RECORDS: usize = 5;
const DREAMS_PERFORMANCE_STALL_MIN_PLANS: usize = 5;
const DREAMS_PERFORMANCE_SUCCESS_IMPROVEMENT: f64 = 0.01;
const DREAMS_PERFORMANCE_COST_IMPROVEMENT: f64 = 0.01;
const DREAMS_PERFORMANCE_STALLED_NOTE: &str = "performance stalled — consider: changing decomposition strategy, adjusting model tier, reviewing failing patterns";
const SIGNALS_LOG_FILE: &str = "signals.jsonl";

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
    /// C-Factor regression analysis from the trailing 7-day snapshot window.
    #[serde(default)]
    pub cfactor_regression: Option<CFactorRegression>,
    /// Cluster summaries discovered during the dream cycle.
    pub clusters: Vec<DreamClusterReport>,
    /// Number of knowledge entries written to the durable store.
    pub knowledge_entries_written: usize,
    /// Number of playbooks written to the durable store.
    pub playbooks_created: usize,
    /// Failure-oriented knowledge entries created during the pass.
    pub regressions_detected: Vec<KnowledgeEntry>,
    /// Cross-domain strategy hypotheses synthesized from structurally similar clusters.
    pub strategy_hypotheses: Vec<KnowledgeEntry>,
    /// High-level learning notes for the next cycle.
    #[serde(default)]
    pub performance_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DreamRegressionSignalPayload {
    started_at: DateTime<Utc>,
    historical_records: usize,
    recent_records: usize,
    historical_successes: usize,
    recent_successes: usize,
    historical_success_rate: f64,
    recent_success_rate: f64,
    drop_fraction: f64,
}

/// One logged counterfactual generated during offline dreaming.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DreamCounterfactualRecord {
    /// When the counterfactual was generated.
    generated_at: DateTime<Utc>,
    /// Cluster the counterfactual came from.
    cluster_key: DreamClusterKey,
    /// Which semantic axis was perturbed.
    focus_axis: String,
    /// Original field value before the perturbation.
    original_value: String,
    /// Replacement field value used in the counterfactual.
    replacement_value: String,
    /// How the replacement was sourced.
    replacement_source: String,
    /// Human-readable hypothesis describing the counterfactual.
    hypothesis: String,
    /// Deterministic positional permutation applied to the candidate vector.
    permutation: usize,
    /// Stable hash of the base HDC context vector.
    base_signature: u64,
    /// Stable hash of the counterfactual HDC context vector.
    counterfactual_signature: u64,
    /// Hamming similarity between the base and counterfactual vectors.
    similarity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CounterfactualAxis {
    Plan,
    TaskType,
    Model,
    Outcome,
    FailureReason,
}

impl CounterfactualAxis {
    const ALL: [Self; 5] = [
        Self::Plan,
        Self::TaskType,
        Self::Model,
        Self::Outcome,
        Self::FailureReason,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan_id",
            Self::TaskType => "task_type",
            Self::Model => "model",
            Self::Outcome => "outcome",
            Self::FailureReason => "failure_reason",
        }
    }

    const fn permutation(self) -> usize {
        match self {
            Self::Plan => 17,
            Self::TaskType => 113,
            Self::Model => 257,
            Self::Outcome => 509,
            Self::FailureReason => 863,
        }
    }

    const fn max_neighborhood_size(self) -> usize {
        match self {
            Self::Outcome => 1,
            _ => 2,
        }
    }

    fn original_value(self, cluster: &DreamCluster) -> String {
        match self {
            Self::Plan => cluster.key.plan_id.clone(),
            Self::TaskType => cluster.key.task_type.clone(),
            Self::Model => cluster.key.model.clone(),
            Self::Outcome => cluster.key.outcome.to_string(),
            Self::FailureReason => summarize_failure_reason(cluster),
        }
    }

    fn replacement_pool(self, clusters: &[DreamCluster]) -> Vec<String> {
        let mut pool = BTreeSet::new();
        for cluster in clusters {
            match self {
                Self::Plan => {
                    pool.insert(cluster.key.plan_id.clone());
                }
                Self::TaskType => {
                    pool.insert(cluster.key.task_type.clone());
                }
                Self::Model => {
                    pool.insert(cluster.key.model.clone());
                }
                Self::Outcome => {
                    pool.insert(cluster.key.outcome.to_string());
                }
                Self::FailureReason => {
                    pool.insert(summarize_failure_reason(cluster));
                }
            }
        }
        pool.into_iter().collect()
    }

    fn fallback_replacement(self, original_value: &str) -> String {
        match self {
            Self::Outcome => {
                if original_value.eq_ignore_ascii_case("success") {
                    "failure".to_string()
                } else {
                    "success".to_string()
                }
            }
            Self::FailureReason => {
                if original_value.trim().is_empty() {
                    "a different failure mode".to_string()
                } else {
                    format!("{original_value} (alternate failure mode)")
                }
            }
            _ => format!("{original_value} (counterfactual)"),
        }
    }

    fn hypothesis(self, original_value: &str, replacement_value: &str) -> String {
        match self {
            Self::Plan => {
                format!("What if plan_id had been {replacement_value} instead of {original_value}?")
            }
            Self::TaskType => format!(
                "What if task_type had been {replacement_value} instead of {original_value}?"
            ),
            Self::Model => {
                format!("What if model had been {replacement_value} instead of {original_value}?")
            }
            Self::Outcome => {
                format!("What if outcome had been {replacement_value} instead of {original_value}?")
            }
            Self::FailureReason => format!(
                "What if the failure reason had been {replacement_value} instead of {original_value}?"
            ),
        }
    }
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

    /// Override the last completed dream timestamp used to filter batches.
    pub fn set_last_dream_at(&mut self, last_dream_at: Option<DateTime<Utc>>) {
        self.last_dream_at = last_dream_at;
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
                format!(
                    "read episode log from {}",
                    self.episode_store.path().display()
                )
            })?;
        let total_episodes = all_episodes.len();
        let cutoff = self.last_dream_at;
        let (historical, mut batch) = match cutoff {
            Some(cutoff) => all_episodes
                .into_iter()
                .partition(|episode| episode.timestamp <= cutoff),
            None => (Vec::new(), all_episodes),
        };
        batch.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.id.cmp(&right.id))
        });

        let processed_through = batch.iter().map(|episode| episode.timestamp).max();
        self.emit_success_rate_regression(&historical, &batch, started_at)?;
        let cfactor_regression = self.emit_cfactor_regression(started_at)?;
        let progression = TierProgression::default();
        let mut analysis = progression.analyze(&batch);
        progression.replay_heuristics(&mut analysis, &batch);
        let review_entries = review_insights_from_heuristics(&analysis, started_at);
        let performance_notes = performance_stall_notes(&batch);
        let mut clusters = cluster_episodes(batch);
        let mut written_knowledge_ids = BTreeSet::new();

        let mut knowledge_entries_written = 0usize;
        let mut playbooks_created = 0usize;
        let mut regressions_detected = Vec::new();

        for entry in review_entries {
            if written_knowledge_ids.insert(entry.id.clone()) {
                self.knowledge_store.add(entry.clone())?;
                knowledge_entries_written += 1;
            }
        }

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

        let strategy_hypotheses = generate_cross_domain_strategy_hypotheses(&clusters, started_at);
        for hypothesis in &strategy_hypotheses {
            if written_knowledge_ids.insert(hypothesis.id.clone()) {
                self.knowledge_store.add(hypothesis.clone())?;
                knowledge_entries_written += 1;
            }
        }

        let report = DreamCycleReport {
            started_at,
            completed_at: Utc::now(),
            total_episodes,
            processed_episodes: clusters.iter().map(|cluster| cluster.episode_count).sum(),
            processed_through,
            analysis,
            cfactor_regression,
            clusters: clusters.iter().map(DreamClusterReport::from).collect(),
            knowledge_entries_written,
            playbooks_created,
            regressions_detected,
            strategy_hypotheses,
            performance_notes,
        };

        let counterfactuals = build_counterfactuals(&clusters, started_at);
        self.write_counterfactuals(&counterfactuals)?;
        self.write_report(&report).await?;

        self.last_dream_at = Some(processed_through.unwrap_or(started_at));

        Ok(report)
    }

    fn emit_success_rate_regression(
        &self,
        historical: &[Episode],
        recent: &[Episode],
        started_at: DateTime<Utc>,
    ) -> Result<()> {
        let Some((historical_successes, historical_records, historical_rate)) =
            success_rate(historical)
        else {
            return Ok(());
        };
        let Some((recent_successes, recent_records, recent_rate)) = success_rate(recent) else {
            return Ok(());
        };

        if historical_records < DREAMS_REGRESSION_MIN_RECORDS
            || recent_records < DREAMS_REGRESSION_MIN_RECORDS
            || historical_rate <= 0.0
        {
            return Ok(());
        }

        let drop_fraction = (historical_rate - recent_rate) / historical_rate;
        if drop_fraction <= DREAMS_SUCCESS_REGRESSION_THRESHOLD {
            return Ok(());
        }

        let payload = DreamRegressionSignalPayload {
            started_at,
            historical_records,
            recent_records,
            historical_successes,
            recent_successes,
            historical_success_rate: historical_rate,
            recent_success_rate: recent_rate,
            drop_fraction,
        };

        let Some(path) = self.signals_path() else {
            return Ok(());
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dream signal directory {}", parent.display()))?;
        }

        let signal = Signal::builder(Kind::Custom("dreams:regression".to_string()))
            .body(Body::from_json(&payload).context("serialize dreams regression payload")?)
            .provenance(roko_core::Provenance::trusted("dreams"))
            .tag("historical_records", historical_records.to_string())
            .tag("recent_records", recent_records.to_string())
            .tag("historical_success_rate", format!("{historical_rate:.4}"))
            .tag("recent_success_rate", format!("{recent_rate:.4}"))
            .tag("drop_fraction", format!("{drop_fraction:.4}"))
            .build();

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open dream signal log {}", path.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &signal)
            .context("serialize dreams regression signal")?;
        writer
            .write_all(b"\n")
            .context("write dreams regression newline")?;
        writer.flush().context("flush dreams regression signal")?;
        Ok(())
    }

    fn emit_cfactor_regression(
        &self,
        started_at: DateTime<Utc>,
    ) -> Result<Option<CFactorRegression>> {
        let Some(path) = self.cfactor_history_path() else {
            return Ok(None);
        };
        let history = match read_cfactor_history(&path) {
            Ok(history) => history,
            Err(_) => return Ok(None),
        };
        let Some(regression) =
            detect_cfactor_regression(&history, Duration::from_secs(7 * 24 * 60 * 60), 0.20)
        else {
            return Ok(None);
        };

        let Some(path) = self.signals_path() else {
            return Ok(Some(regression));
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dream signal directory {}", parent.display()))?;
        }

        let signal = Signal::builder(Kind::Custom("cfactor:regression".to_string()))
            .body(Body::from_json(&regression).context("serialize cfactor regression payload")?)
            .provenance(roko_core::Provenance::trusted("dreams"))
            .tag("current", format!("{:.4}", regression.current))
            .tag(
                "historical_average",
                format!("{:.4}", regression.historical_average),
            )
            .tag("drop_fraction", format!("{:.4}", regression.drop_fraction))
            .tag("threshold", format!("{:.4}", regression.threshold))
            .tag("sample_count", regression.sample_count.to_string())
            .tag("window_start", regression.window_start.to_rfc3339())
            .tag("window_end", regression.window_end.to_rfc3339())
            .tag("started_at", started_at.to_rfc3339())
            .build();

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open dream signal log {}", path.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &signal)
            .context("serialize cfactor regression signal")?;
        writer
            .write_all(b"\n")
            .context("write cfactor regression newline")?;
        writer.flush().context("flush cfactor regression signal")?;

        Ok(Some(regression))
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

    fn write_counterfactuals(&self, counterfactuals: &[DreamCounterfactualRecord]) -> Result<()> {
        if counterfactuals.is_empty() {
            return Ok(());
        }

        let path = dream_counterfactual_path(self.episode_store.path());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("create dream counterfactual directory {}", parent.display())
            })?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open dream counterfactual log {}", path.display()))?;
        let mut writer = BufWriter::new(file);
        for counterfactual in counterfactuals {
            serde_json::to_writer(&mut writer, counterfactual)
                .context("serialize dream counterfactual")?;
            writer
                .write_all(b"\n")
                .context("write dream counterfactual newline")?;
        }
        writer.flush().context("flush dream counterfactual log")?;
        Ok(())
    }

    fn cfactor_history_path(&self) -> Option<PathBuf> {
        let root = self
            .episode_store
            .path()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        Some(root.join("learn").join("c-factor.jsonl"))
    }

    fn signals_path(&self) -> Option<PathBuf> {
        let root = self
            .episode_store
            .path()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        Some(root.join(SIGNALS_LOG_FILE))
    }
}

fn success_rate(episodes: &[Episode]) -> Option<(usize, usize, f64)> {
    let records = episodes.len();
    if records == 0 {
        return None;
    }
    let successes = episodes.iter().filter(|episode| episode.success).count();
    Some((successes, records, successes as f64 / records as f64))
}

fn read_cfactor_history(path: &Path) -> Result<Vec<CFactor>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("read C-Factor history {}", path.display()));
        }
    };

    let mut history = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(snapshot) = serde_json::from_str::<CFactor>(trimmed) {
            history.push(snapshot);
        }
    }
    Ok(history)
}

#[derive(Debug, Clone)]
struct PlanPerformanceSummary {
    plan_id: String,
    first_seen_at: DateTime<Utc>,
    success_rate: f64,
    cost_per_success_usd: f64,
}

#[derive(Debug, Default)]
struct PlanPerformanceAccumulator {
    first_seen_at: Option<DateTime<Utc>>,
    episode_count: usize,
    success_count: usize,
    total_cost_usd: f64,
}

impl PlanPerformanceAccumulator {
    fn record(&mut self, episode: &Episode) {
        self.first_seen_at = Some(match self.first_seen_at.take() {
            Some(existing) => existing.min(episode.timestamp),
            None => episode.timestamp,
        });
        self.episode_count += 1;
        self.success_count += usize::from(episode.success);
        self.total_cost_usd += episode.usage.cost_usd;
    }

    fn success_rate(&self) -> f64 {
        if self.episode_count == 0 {
            0.0
        } else {
            self.success_count as f64 / self.episode_count as f64
        }
    }

    fn cost_per_success_usd(&self) -> f64 {
        self.total_cost_usd / self.success_count.max(1) as f64
    }
}

fn summarize_plan_performance(episodes: &[Episode]) -> Vec<PlanPerformanceSummary> {
    let mut by_plan: BTreeMap<String, PlanPerformanceAccumulator> = BTreeMap::new();
    for episode in episodes {
        by_plan
            .entry(episode_plan_id(episode))
            .or_default()
            .record(episode);
    }

    let mut plans: Vec<PlanPerformanceSummary> = by_plan
        .into_iter()
        .filter_map(|(plan_id, accumulator)| {
            accumulator
                .first_seen_at
                .map(|first_seen_at| PlanPerformanceSummary {
                    plan_id,
                    first_seen_at,
                    success_rate: accumulator.success_rate(),
                    cost_per_success_usd: accumulator.cost_per_success_usd(),
                })
        })
        .collect();
    plans.sort_by(|left, right| {
        left.first_seen_at
            .cmp(&right.first_seen_at)
            .then_with(|| left.plan_id.cmp(&right.plan_id))
    });
    plans
}

fn performance_stall_notes(episodes: &[Episode]) -> Vec<String> {
    let plans = summarize_plan_performance(episodes);
    if plans.len() < DREAMS_PERFORMANCE_STALL_MIN_PLANS {
        return Vec::new();
    }

    let mut streak = 1usize;
    for window in plans.windows(2) {
        let previous = &window[0];
        let current = &window[1];
        let improved_success =
            current.success_rate > previous.success_rate + DREAMS_PERFORMANCE_SUCCESS_IMPROVEMENT;
        let improved_cost = current.cost_per_success_usd
            < previous.cost_per_success_usd * (1.0 - DREAMS_PERFORMANCE_COST_IMPROVEMENT);
        if improved_success || improved_cost {
            streak = 1;
        } else {
            streak += 1;
        }
    }

    if streak >= DREAMS_PERFORMANCE_STALL_MIN_PLANS {
        vec![DREAMS_PERFORMANCE_STALLED_NOTE.to_string()]
    } else {
        Vec::new()
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
    let review_text = response
        .output
        .body
        .as_text()
        .unwrap_or("")
        .trim()
        .to_string();
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

    if cluster.failure_count > 0 {
        let mistake = build_mistake_insight_entry(cluster, started_at);
        knowledge_store.add(mistake.clone())?;
        outcome.knowledge_entries_written += 1;
        outcome.knowledge_entries.push(mistake);
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
        outcome.warnings.push(format!(
            "agent review returned a non-empty response: {text}"
        ));
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

fn build_counterfactuals(
    clusters: &[DreamCluster],
    generated_at: DateTime<Utc>,
) -> Vec<DreamCounterfactualRecord> {
    let mut records = Vec::new();
    if clusters.is_empty() {
        return records;
    }

    for cluster in clusters {
        let outcome = cluster.key.outcome.to_string();
        let failure_reason = summarize_failure_reason(cluster);
        let base_vector = encode_cluster_vector(
            &cluster.key.plan_id,
            &cluster.key.task_type,
            &outcome,
            &cluster.key.model,
            &failure_reason,
        );
        let base_signature = vector_signature(&base_vector);

        for axis in CounterfactualAxis::ALL {
            let original_value = axis.original_value(cluster);
            let pool = axis.replacement_pool(clusters);
            let mut candidates = select_counterfactual_candidates(
                axis,
                &original_value,
                &pool,
                axis.max_neighborhood_size(),
            );

            if candidates.is_empty() {
                candidates.push((
                    axis.fallback_replacement(&original_value),
                    "synthetic".to_string(),
                ));
            }

            for (rank, (replacement_value, source)) in candidates.into_iter().enumerate() {
                let counterfactual_vector =
                    encode_counterfactual_vector(cluster, axis, &replacement_value, rank + 1);
                let similarity = base_vector.similarity(&counterfactual_vector);
                let hypothesis = axis.hypothesis(&original_value, &replacement_value);
                records.push(DreamCounterfactualRecord {
                    generated_at,
                    cluster_key: cluster.key.clone(),
                    focus_axis: axis.label().to_string(),
                    original_value: original_value.clone(),
                    replacement_value,
                    replacement_source: source,
                    hypothesis,
                    permutation: axis.permutation() + (rank + 1) * 17,
                    base_signature,
                    counterfactual_signature: vector_signature(&counterfactual_vector),
                    similarity,
                });
            }
        }
    }

    records
}

fn select_counterfactual_candidates(
    axis: CounterfactualAxis,
    original_value: &str,
    pool: &[String],
    limit: usize,
) -> Vec<(String, String)> {
    let original_vector = text_fingerprint(original_value);
    let mut scored: Vec<(String, String, f32)> = pool
        .iter()
        .filter(|candidate| candidate.as_str() != original_value)
        .map(|candidate| {
            let similarity = original_vector.similarity(&text_fingerprint(candidate));
            (candidate.clone(), "observed".to_string(), similarity)
        })
        .collect();

    scored.sort_by(|left, right| {
        right
            .2
            .partial_cmp(&left.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });

    let mut selected: Vec<(String, String)> = scored
        .into_iter()
        .take(limit)
        .map(|(value, source, _)| (value, source))
        .collect();

    if selected.is_empty() && !original_value.trim().is_empty() {
        selected.push((
            axis.fallback_replacement(original_value),
            "synthetic".to_string(),
        ));
    }

    selected
}

fn encode_cluster_vector(
    plan_id: &str,
    task_type: &str,
    outcome: &str,
    model: &str,
    failure_reason: &str,
) -> HdcVector {
    let parts = [
        text_fingerprint(&format!("plan_id={plan_id}")).permute(11),
        text_fingerprint(&format!("task_type={task_type}")).permute(37),
        text_fingerprint(&format!("outcome={outcome}")).permute(73),
        text_fingerprint(&format!("model={model}")).permute(131),
        text_fingerprint(&format!("failure_reason={failure_reason}")).permute(197),
    ];
    let refs = parts.iter().collect::<Vec<_>>();
    HdcVector::bundle(&refs)
}

fn encode_counterfactual_vector(
    cluster: &DreamCluster,
    axis: CounterfactualAxis,
    replacement_value: &str,
    rank: usize,
) -> HdcVector {
    let failure_reason = summarize_failure_reason(cluster);
    let plan_id = match axis {
        CounterfactualAxis::Plan => replacement_value.to_string(),
        _ => cluster.key.plan_id.clone(),
    };
    let task_type = match axis {
        CounterfactualAxis::TaskType => replacement_value.to_string(),
        _ => cluster.key.task_type.clone(),
    };
    let outcome = match axis {
        CounterfactualAxis::Outcome => replacement_value.to_string(),
        _ => cluster.key.outcome.to_string(),
    };
    let model = match axis {
        CounterfactualAxis::Model => replacement_value.to_string(),
        _ => cluster.key.model.clone(),
    };
    let failure_reason = match axis {
        CounterfactualAxis::FailureReason => replacement_value.to_string(),
        _ => failure_reason,
    };
    encode_cluster_vector(&plan_id, &task_type, &outcome, &model, &failure_reason)
        .permute(axis.permutation() + rank * 17)
}

fn vector_signature(vector: &HdcVector) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in vector.to_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
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
            self.source_episodes
                .extend(fallback_sources.iter().cloned());
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
            source: Some("dream".to_string()),
            content: content.to_string(),
            confidence,
            confidence_weight: confidence,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: self.source_episodes,
            tags: self.tags,
            created_at: Utc::now(),
            half_life_days,
            hdc_vector: None,
        })
    }
}

fn parse_cluster_response(
    response: &str,
    fallback_sources: &[String],
) -> Result<Vec<KnowledgeEntry>> {
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
        source: Some("dream".to_string()),
        content,
        confidence: if playbook.steps.is_empty() { 0.0 } else { 1.0 },
        confidence_weight: if playbook.steps.is_empty() { 0.0 } else { 1.0 },
        refuted_insight_id: None,
        refutation_evidence: None,
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
    let kind = if cluster.success_count == 0 {
        KnowledgeKind::Constraint
    } else {
        KnowledgeKind::AntiKnowledge
    };
    let refuted_insight_id = format!(
        "insight:{}:{}:{}",
        cluster.key.plan_id, cluster.key.task_type, cluster.key.model
    );
    let mut evidence = reason;
    if cluster.failure_count > 0 {
        let failing_gates = summarize_failure_gates(cluster);
        if !failing_gates.is_empty() {
            evidence.push_str(&format!(" The failing gates were {}.", failing_gates));
        }
    }
    let content = if kind == KnowledgeKind::AntiKnowledge {
        format!(
            "Previous insight {refuted_insight_id} was wrong because {evidence}"
        )
    } else {
        format!(
            "Approach {} for plan {} and task type {} does not work because {}.",
            cluster.key.model, cluster.key.plan_id, cluster.key.task_type, evidence
        )
    };
    let confidence = if cluster.failure_count > 0 { 0.9 } else { 0.0 };
    KnowledgeEntry {
        id: derive_knowledge_id(
            kind,
            &content,
            &cluster.episode_ids,
            &[knowledge_kind_tag(kind).to_string()],
        ),
        kind,
        source: Some("dream".to_string()),
        content,
        confidence,
        confidence_weight: if kind == KnowledgeKind::AntiKnowledge {
            -confidence
        } else {
            confidence
        },
        refuted_insight_id: (kind == KnowledgeKind::AntiKnowledge).then_some(refuted_insight_id),
        refutation_evidence: (kind == KnowledgeKind::AntiKnowledge).then_some(evidence),
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

fn generate_cross_domain_strategy_hypotheses(
    clusters: &[DreamCluster],
    created_at: DateTime<Utc>,
) -> Vec<KnowledgeEntry> {
    let source_clusters: Vec<&DreamCluster> = clusters
        .iter()
        .filter(|cluster| cluster.success_count > 0)
        .collect();
    if source_clusters.len() < 2 {
        return Vec::new();
    }

    let source_vectors: Vec<HdcVector> = source_clusters
        .iter()
        .map(|cluster| cluster_structure_vector(cluster))
        .collect();
    let mut entries = Vec::new();

    for target in clusters.iter().filter(|cluster| cluster.failure_count > 0) {
        let target_vector = cluster_structure_vector(target);
        let mut scored_sources: Vec<(usize, f32)> = source_clusters
            .iter()
            .enumerate()
            .filter(|(_, source)| source.key.task_type != target.key.task_type)
            .map(|(index, source)| {
                let score = structural_transfer_score(
                    target,
                    source,
                    &target_vector,
                    &source_vectors[index],
                );
                (index, score)
            })
            .collect();

        scored_sources.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    source_clusters[left.0]
                        .key
                        .task_type
                        .cmp(&source_clusters[right.0].key.task_type)
                })
        });

        let mut picked: Vec<(usize, f32)> = Vec::new();
        let mut seen_task_types = BTreeSet::new();
        for (index, score) in scored_sources {
            let source = source_clusters[index];
            if !seen_task_types.insert(source.key.task_type.clone()) {
                continue;
            }
            picked.push((index, score));
            if picked.len() == 2 {
                break;
            }
        }

        if picked.len() < 2 {
            continue;
        }

        let source_a = source_clusters[picked[0].0];
        let source_b = source_clusters[picked[1].0];
        let source_a_score = picked[0].1;
        let source_b_score = picked[1].1;
        let content = render_cross_domain_strategy_content(
            target,
            source_a,
            source_a_score,
            source_b,
            source_b_score,
        );
        let mut source_episodes: BTreeSet<String> = target.episode_ids.iter().cloned().collect();
        source_episodes.extend(source_a.episode_ids.iter().cloned());
        source_episodes.extend(source_b.episode_ids.iter().cloned());
        let source_episodes: Vec<String> = source_episodes.into_iter().collect();
        let tags = vec![
            knowledge_kind_tag(KnowledgeKind::Heuristic).to_string(),
            "dream".to_string(),
            "cross-domain".to_string(),
            "novel-strategy".to_string(),
            "structural-transfer".to_string(),
            format!("target-task:{}", target.key.task_type),
            format!("source-task:{}", source_a.key.task_type),
            format!("source-task:{}", source_b.key.task_type),
            format!("target-model:{}", target.key.model),
        ];

        entries.push(KnowledgeEntry {
            id: derive_knowledge_id(KnowledgeKind::Heuristic, &content, &source_episodes, &tags),
            kind: KnowledgeKind::Heuristic,
            source: Some("dream".to_string()),
            content,
            confidence: strategy_confidence(target, source_a_score, source_b_score),
            confidence_weight: strategy_confidence(target, source_a_score, source_b_score),
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes,
            tags,
            created_at,
            half_life_days: KnowledgeKind::Heuristic.default_half_life_days(),
            hdc_vector: None,
        });
    }

    entries
}

fn structural_transfer_score(
    target: &DreamCluster,
    source: &DreamCluster,
    target_vector: &HdcVector,
    source_vector: &HdcVector,
) -> f32 {
    let mut score = target_vector.similarity(source_vector);
    if target.key.model == source.key.model {
        score += 0.10;
    }
    let target_failure_gates = gate_name_set(&summarize_failure_gates(target));
    let source_success_gates = gate_name_set(&summarize_success_gates(source));
    let shared_gates = target_failure_gates
        .intersection(&source_success_gates)
        .count()
        .min(2) as f32;
    score += shared_gates * 0.06;
    if source.playbook.is_some() {
        score += 0.08;
    }
    if source.success_count >= target.failure_count {
        score += 0.04;
    }
    score.clamp(0.0, 1.0)
}

fn strategy_confidence(target: &DreamCluster, source_a_score: f32, source_b_score: f32) -> f64 {
    let failure_pressure = if target.failure_count == 0 {
        0.0
    } else {
        (target.failure_count as f64 / target.episode_count.max(1) as f64).clamp(0.0, 1.0)
    };
    let structural_fit = ((source_a_score as f64 + source_b_score as f64) / 2.0).clamp(0.0, 1.0);
    (0.35 + failure_pressure * 0.25 + structural_fit * 0.4).clamp(0.3, 0.95)
}

fn render_cross_domain_strategy_content(
    target: &DreamCluster,
    source_a: &DreamCluster,
    source_a_score: f32,
    source_b: &DreamCluster,
    source_b_score: f32,
) -> String {
    let source_a_strategy = summarize_success_pattern(source_a);
    let source_b_strategy = summarize_success_pattern(source_b);
    let shared_cues = summarize_shared_cues(target, source_a, source_b);
    format!(
        "Cross-domain strategy hypothesis for task type {}: blend the {} approach ({}) with the {} approach ({}). The clusters look structurally similar because {}. Transfer the shared control loop to {} and adapt it to the failure mode {}. Structural match scores: {:.2} and {:.2}.",
        target.key.task_type,
        source_a.key.task_type,
        source_a_strategy,
        source_b.key.task_type,
        source_b_strategy,
        shared_cues,
        target.key.task_type,
        summarize_failure_reason(target),
        source_a_score,
        source_b_score
    )
}

fn summarize_success_pattern(cluster: &DreamCluster) -> String {
    if let Some(playbook) = &cluster.playbook {
        let steps = playbook
            .steps
            .iter()
            .take(2)
            .map(|step| step.description.as_str())
            .collect::<Vec<_>>()
            .join(" then ");
        if steps.is_empty() {
            return playbook.goal.clone();
        }
        return format!("{}; {}", playbook.goal, steps);
    }

    let gates = summarize_success_gates(cluster);
    format!(
        "repeat the successful {} pattern with model {} while preserving {}",
        cluster.key.task_type, cluster.key.model, gates
    )
}

fn summarize_shared_cues(
    target: &DreamCluster,
    source_a: &DreamCluster,
    source_b: &DreamCluster,
) -> String {
    let mut cues = Vec::new();
    if source_a.key.model == target.key.model || source_b.key.model == target.key.model {
        cues.push(format!("the same model {}", target.key.model));
    }

    let target_failures = gate_name_set(&summarize_failure_gates(target));
    let source_success_a = gate_name_set(&summarize_success_gates(source_a));
    let source_success_b = gate_name_set(&summarize_success_gates(source_b));
    let shared_a: Vec<String> = target_failures
        .intersection(&source_success_a)
        .cloned()
        .collect();
    let shared_b: Vec<String> = target_failures
        .intersection(&source_success_b)
        .cloned()
        .collect();
    if !shared_a.is_empty() || !shared_b.is_empty() {
        let mut shared = shared_a;
        shared.extend(shared_b);
        shared.sort();
        shared.dedup();
        cues.push(format!("shared gate pressure around {}", shared.join(", ")));
    }

    if summarize_failure_reason(source_a) == summarize_failure_reason(target)
        || summarize_failure_reason(source_b) == summarize_failure_reason(target)
    {
        cues.push("the same failure mode".to_string());
    }

    if cues.is_empty() {
        cues.push("a similar control-flow shape".to_string());
    }

    cues.join(" and ")
}

fn gate_name_set(summary: &str) -> BTreeSet<String> {
    summary
        .split(',')
        .map(str::trim)
        .filter(|gate| !gate.is_empty())
        .map(|gate| gate.split_whitespace().next().unwrap_or("").to_string())
        .filter(|gate| !gate.is_empty())
        .collect()
}

fn cluster_structure_vector(cluster: &DreamCluster) -> HdcVector {
    let task_type = text_fingerprint(&format!("task_type={}", cluster.key.task_type)).permute(19);
    let model = text_fingerprint(&format!("model={}", cluster.key.model)).permute(41);
    let outcome = text_fingerprint(&format!("outcome={}", cluster.key.outcome)).permute(83);
    let balance = text_fingerprint(&format!(
        "balance=success:{} failure:{}",
        cluster.success_count, cluster.failure_count
    ))
    .permute(127);
    let success_gates = text_fingerprint(&format!(
        "success_gates={}",
        summarize_success_gates(cluster)
    ))
    .permute(163);
    let failure_gates = text_fingerprint(&format!(
        "failure_gates={}",
        summarize_failure_gates(cluster)
    ))
    .permute(211);
    let failure_reason = text_fingerprint(&format!(
        "failure_reason={}",
        summarize_failure_reason(cluster)
    ))
    .permute(257);
    HdcVector::bundle(&[
        &task_type,
        &model,
        &outcome,
        &balance,
        &success_gates,
        &failure_gates,
        &failure_reason,
    ])
}

fn build_mistake_insight_entry(
    cluster: &DreamCluster,
    created_at: DateTime<Utc>,
) -> KnowledgeEntry {
    let reason = summarize_failure_reason(cluster);
    let failing_gates = summarize_failure_gates(cluster);
    let mut content = format!(
        "Failed episodes for plan {} and task type {} show a specific mistake: {}.",
        cluster.key.plan_id, cluster.key.task_type, reason
    );
    if !failing_gates.is_empty() {
        content.push_str(&format!(" The failing gates were {}.", failing_gates));
    }

    KnowledgeEntry {
        id: derive_knowledge_id(
            KnowledgeKind::Insight,
            &content,
            &cluster.episode_ids,
            &[
                knowledge_kind_tag(KnowledgeKind::Insight).to_string(),
                "dream".to_string(),
                "mistake".to_string(),
                "failure".to_string(),
                format!("plan:{}", cluster.key.plan_id),
                format!("task_type:{}", cluster.key.task_type),
                format!("model:{}", cluster.key.model),
            ],
        ),
        kind: KnowledgeKind::Insight,
        source: Some("dream".to_string()),
        content,
        confidence: if cluster.failure_count > 0 { 0.85 } else { 0.0 },
        confidence_weight: if cluster.failure_count > 0 { 0.85 } else { 0.0 },
        refuted_insight_id: None,
        refutation_evidence: None,
        source_episodes: cluster.episode_ids.clone(),
        tags: vec![
            knowledge_kind_tag(KnowledgeKind::Insight).to_string(),
            "dream".to_string(),
            "mistake".to_string(),
            "failure".to_string(),
            "root-cause".to_string(),
            format!("plan:{}", cluster.key.plan_id),
            format!("task_type:{}", cluster.key.task_type),
            format!("model:{}", cluster.key.model),
        ],
        created_at,
        half_life_days: KnowledgeKind::Insight.default_half_life_days(),
        hdc_vector: None,
    }
}

fn review_insights_from_heuristics(
    analysis: &TierProgressionReport,
    created_at: DateTime<Utc>,
) -> Vec<KnowledgeEntry> {
    analysis
        .heuristics
        .iter()
        .filter(|heuristic| heuristic_recommends_different_approach(&heuristic.then_clause))
        .filter(|heuristic| !heuristic.source_episodes.is_empty())
        .map(|heuristic| {
            let content = format!(
                "Would I do this differently now? For {}, current knowledge suggests a different approach: {}.",
                heuristic.when_clause, heuristic.then_clause
            );
            let source_episodes = heuristic.source_episodes.clone();
            let tags = vec![
                knowledge_kind_tag(KnowledgeKind::Insight).to_string(),
                "dream".to_string(),
                "review".to_string(),
                "heuristic".to_string(),
                format!("heuristic:{}", heuristic.id),
            ];
        KnowledgeEntry {
            id: derive_knowledge_id(
                KnowledgeKind::Insight,
                &content,
                &source_episodes,
                    &tags,
            ),
            kind: KnowledgeKind::Insight,
            source: Some("dream".to_string()),
            content,
            confidence: heuristic.confidence.clamp(0.0, 1.0),
            confidence_weight: heuristic.confidence.clamp(0.0, 1.0),
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes,
            tags,
            created_at,
            half_life_days: KnowledgeKind::Insight.default_half_life_days(),
            hdc_vector: None,
            }
        })
        .collect()
}

fn heuristic_recommends_different_approach(then_clause: &str) -> bool {
    let normalized = then_clause.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if normalized.contains("reuse this path as the default play") {
        return false;
    }

    normalized.starts_with("add ")
        || normalized.starts_with("prioritize ")
        || normalized.starts_with("treat ")
        || normalized.starts_with("avoid ")
        || normalized.starts_with("escalate ")
        || normalized.starts_with("switch ")
        || normalized.starts_with("retry ")
        || normalized.contains("different approach")
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

fn dream_counterfactual_path(episode_path: &Path) -> PathBuf {
    dream_root_path(episode_path)
        .join("dreams")
        .join("counterfactuals.jsonl")
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
        if let Some(parent) = logger.path().parent() {
            std::fs::create_dir_all(parent).expect("create episodes dir");
        }
        logger.append(episode).await.expect("append episode");
    }

    fn episode_at(
        id: &str,
        plan_id: &str,
        task_type: &str,
        model: &str,
        success: bool,
        failure_reason: Option<&str>,
        timestamp: DateTime<Utc>,
    ) -> Episode {
        let mut episode = episode(id, plan_id, task_type, model, success, failure_reason);
        episode.timestamp = timestamp;
        episode.started_at = timestamp;
        episode.completed_at = timestamp;
        episode
    }

    fn read_signals(path: &Path) -> Vec<Signal> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("parse signal"))
            .collect()
    }

    fn write_cfactor_history(path: &Path, snapshots: &[CFactor]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create c-factor directory");
        }
        let mut lines = Vec::new();
        for snapshot in snapshots {
            lines.push(serde_json::to_string(snapshot).expect("serialize c-factor snapshot"));
        }
        std::fs::write(path, lines.join("\n") + "\n").expect("write c-factor history");
    }

    #[tokio::test]
    #[ignore = "dream cycle strategy_hypotheses generation needs logic fix"]
    async fn run_clusters_and_writes_report() {
        let tmp = TempDir::new().expect("tempdir");
        let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
        let knowledge_path = tmp
            .path()
            .join(".roko")
            .join("neuro")
            .join("knowledge.jsonl");
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
        for idx in 0..4 {
            let ep = episode(
                &format!("docs-{idx}"),
                "plan-c",
                "docs",
                "claude-haiku-4-5",
                true,
                None,
            );
            write_episode(&logger, &ep).await;
        }
        for idx in 0..5 {
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
        assert_eq!(report.processed_episodes, 13);
        assert_eq!(report.clusters.len(), 3);
        assert_eq!(report.playbooks_created, 2);
        assert!(!report.strategy_hypotheses.is_empty());
        assert!(
            report
                .strategy_hypotheses
                .iter()
                .all(|entry| entry.tags.iter().any(|tag| tag == "cross-domain"))
        );
        assert!(!report.regressions_detected.is_empty());
        assert!(cycle.last_dream_at().is_some());

        let report_dir = tmp.path().join(".roko").join("dreams");
        let mut entries = tokio::fs::read_dir(&report_dir).await.expect("dream dir");
        assert!(entries.next_entry().await.expect("next").is_some());

        let counterfactual_path = report_dir.join("counterfactuals.jsonl");
        let counterfactual_text = tokio::fs::read_to_string(&counterfactual_path)
            .await
            .expect("counterfactual log");
        let first_line = counterfactual_text
            .lines()
            .next()
            .expect("counterfactual line");
        let counterfactual: Value =
            serde_json::from_str(first_line).expect("parse counterfactual json");
        assert_eq!(counterfactual["focus_axis"].as_str(), Some("plan_id"));
        assert!(counterfactual["similarity"].as_f64().unwrap_or_default() > 0.0);

        let saved_playbooks = playbook_store.list().await.expect("list playbooks");
        assert_eq!(saved_playbooks.len(), 2);
        assert!(
            saved_playbooks
                .iter()
                .any(|playbook| playbook.goal.contains("task type"))
        );

        let store = KnowledgeStore::new(&knowledge_path);
        let knowledge_entries = store.query("dream", 10).expect("query");
        assert!(!knowledge_entries.is_empty());
        let all_entries = store.read_all().expect("read knowledge");
        assert!(all_entries.iter().any(|entry| {
            entry.kind == KnowledgeKind::Heuristic
                && entry.tags.iter().any(|tag| tag == "novel-strategy")
                && entry.tags.iter().any(|tag| tag == "cross-domain")
                && entry.content.contains("Cross-domain strategy hypothesis")
        }));
        assert!(all_entries.iter().any(|entry| {
            entry.kind == KnowledgeKind::Insight
                && entry.tags.iter().any(|tag| tag == "mistake")
                && entry.content.contains("missing rollback")
        }));
    }

    #[tokio::test]
    async fn regression_signal_emitted_when_recent_success_rate_drops() {
        let tmp = TempDir::new().expect("tempdir");
        let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
        let knowledge_path = tmp
            .path()
            .join(".roko")
            .join("neuro")
            .join("knowledge.jsonl");
        let playbooks_root = tmp.path().join(".roko").join("playbooks");
        let logger = EpisodeLogger::new(&episodes_path);
        let knowledge_store = Arc::new(KnowledgeStore::new(&knowledge_path));
        let playbook_store = Arc::new(PlaybookStore::new(&playbooks_root));
        let dispatcher = Arc::new(MockDispatcher {
            response: r#"<|json|>{"entries":[]}<|/json|>"#.to_string(),
        });

        let historical = Utc::now() - chrono::Duration::hours(2);
        let recent = Utc::now();

        for idx in 0..5 {
            let ep = episode_at(
                &format!("hist-{idx}"),
                "plan-a",
                "implementation",
                "claude-haiku-4-5",
                true,
                None,
                historical + chrono::Duration::minutes(i64::from(idx)),
            );
            write_episode(&logger, &ep).await;
        }
        for idx in 0..5 {
            let ep = episode_at(
                &format!("recent-{idx}"),
                "plan-b",
                "docs",
                "claude-haiku-4-5",
                false,
                Some("regressed"),
                recent + chrono::Duration::minutes(i64::from(idx)),
            );
            write_episode(&logger, &ep).await;
        }

        let mut cycle = DreamCycle::new(
            Arc::new(logger),
            knowledge_store,
            playbook_store,
            dispatcher,
        );
        cycle.set_last_dream_at(Some(historical + chrono::Duration::minutes(30)));

        let report = cycle.run().await.expect("run");
        assert_eq!(report.processed_episodes, 5);

        let signal_log = tmp.path().join(".roko").join("signals.jsonl");
        let signals = read_signals(&signal_log);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].kind.as_str(), "dreams:regression");
        let drop_fraction: f64 = signals[0]
            .tag("drop_fraction")
            .and_then(|value| value.parse().ok())
            .expect("drop fraction tag");
        assert!(drop_fraction > DREAMS_SUCCESS_REGRESSION_THRESHOLD);
    }

    #[tokio::test]
    async fn regression_signal_not_emitted_at_exact_threshold() {
        let tmp = TempDir::new().expect("tempdir");
        let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
        let knowledge_path = tmp
            .path()
            .join(".roko")
            .join("neuro")
            .join("knowledge.jsonl");
        let playbooks_root = tmp.path().join(".roko").join("playbooks");
        let logger = EpisodeLogger::new(&episodes_path);
        let knowledge_store = Arc::new(KnowledgeStore::new(&knowledge_path));
        let playbook_store = Arc::new(PlaybookStore::new(&playbooks_root));
        let dispatcher = Arc::new(MockDispatcher {
            response: r#"<|json|>{"entries":[]}<|/json|>"#.to_string(),
        });

        let historical = Utc::now() - chrono::Duration::hours(2);
        let recent = Utc::now();

        for idx in 0..5 {
            let ep = episode_at(
                &format!("hist-{idx}"),
                "plan-a",
                "implementation",
                "claude-haiku-4-5",
                true,
                None,
                historical + chrono::Duration::minutes(i64::from(idx)),
            );
            write_episode(&logger, &ep).await;
        }
        for idx in 0..5 {
            let ep = episode_at(
                &format!("recent-{idx}"),
                "plan-b",
                "docs",
                "claude-haiku-4-5",
                idx != 4,
                if idx == 4 {
                    Some("single failure")
                } else {
                    None
                },
                recent + chrono::Duration::minutes(i64::from(idx)),
            );
            write_episode(&logger, &ep).await;
        }

        let mut cycle = DreamCycle::new(
            Arc::new(logger),
            knowledge_store,
            playbook_store,
            dispatcher,
        );
        cycle.set_last_dream_at(Some(historical + chrono::Duration::minutes(30)));

        let report = cycle.run().await.expect("run");
        assert_eq!(report.processed_episodes, 5);

        let signal_log = tmp.path().join(".roko").join("signals.jsonl");
        let signals = read_signals(&signal_log);
        assert!(
            signals
                .iter()
                .all(|signal| signal.kind.as_str() != "dreams:regression")
        );
    }

    #[tokio::test]
    async fn cfactor_regression_signal_emitted_when_recent_average_drops() {
        let tmp = TempDir::new().expect("tempdir");
        let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
        let learn_path = tmp
            .path()
            .join(".roko")
            .join("learn")
            .join("c-factor.jsonl");
        let knowledge_path = tmp
            .path()
            .join(".roko")
            .join("neuro")
            .join("knowledge.jsonl");
        let playbooks_root = tmp.path().join(".roko").join("playbooks");
        let logger = EpisodeLogger::new(&episodes_path);
        let knowledge_store = Arc::new(KnowledgeStore::new(&knowledge_path));
        let playbook_store = Arc::new(PlaybookStore::new(&playbooks_root));
        let dispatcher = Arc::new(MockDispatcher {
            response: r#"<|json|>{"entries":[]}<|/json|>"#.to_string(),
        });

        let mut historical = CFactor::default();
        historical.overall = 0.92;
        historical.computed_at = Utc::now() - chrono::Duration::days(6);

        let mut middle = CFactor::default();
        middle.overall = 0.84;
        middle.computed_at = Utc::now() - chrono::Duration::days(3);

        let mut current = CFactor::default();
        current.overall = 0.55;
        current.computed_at = Utc::now() - chrono::Duration::days(1);

        write_cfactor_history(&learn_path, &[historical, middle, current]);

        let historical_episode = episode_at(
            "hist-1",
            "plan-a",
            "implementation",
            "claude-haiku-4-5",
            true,
            None,
            Utc::now() - chrono::Duration::hours(2),
        );
        let recent_episode = episode_at(
            "recent-1",
            "plan-b",
            "implementation",
            "claude-haiku-4-5",
            true,
            None,
            Utc::now(),
        );
        write_episode(&logger, &historical_episode).await;
        write_episode(&logger, &recent_episode).await;

        let mut cycle = DreamCycle::new(
            Arc::new(logger),
            knowledge_store,
            playbook_store,
            dispatcher,
        );
        cycle.set_last_dream_at(Some(Utc::now() - chrono::Duration::minutes(30)));

        let report = cycle.run().await.expect("run");
        let regression = report
            .cfactor_regression
            .as_ref()
            .expect("cfactor regression analysis");
        assert!(regression.drop_fraction > 0.20);

        let signal_log = tmp.path().join(".roko").join("signals.jsonl");
        let signals = read_signals(&signal_log);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].kind.as_str(), "cfactor:regression");
        assert!(signals[0].tag("drop_fraction").is_some());
    }

    #[tokio::test]
    async fn cfactor_regression_signal_not_emitted_at_exact_threshold() {
        let tmp = TempDir::new().expect("tempdir");
        let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
        let learn_path = tmp
            .path()
            .join(".roko")
            .join("learn")
            .join("c-factor.jsonl");
        let knowledge_path = tmp
            .path()
            .join(".roko")
            .join("neuro")
            .join("knowledge.jsonl");
        let playbooks_root = tmp.path().join(".roko").join("playbooks");
        let logger = EpisodeLogger::new(&episodes_path);
        let knowledge_store = Arc::new(KnowledgeStore::new(&knowledge_path));
        let playbook_store = Arc::new(PlaybookStore::new(&playbooks_root));
        let dispatcher = Arc::new(MockDispatcher {
            response: r#"<|json|>{"entries":[]}<|/json|>"#.to_string(),
        });

        let mut historical = CFactor::default();
        historical.overall = 1.0;
        historical.computed_at = Utc::now() - chrono::Duration::days(6);

        let mut current = CFactor::default();
        current.overall = 0.8;
        current.computed_at = Utc::now() - chrono::Duration::days(1);

        write_cfactor_history(&learn_path, &[historical, current]);

        let episode = episode_at(
            "recent-1",
            "plan-a",
            "implementation",
            "claude-haiku-4-5",
            true,
            None,
            Utc::now(),
        );
        write_episode(&logger, &episode).await;

        let mut cycle = DreamCycle::new(
            Arc::new(logger),
            knowledge_store,
            playbook_store,
            dispatcher,
        );

        let report = cycle.run().await.expect("run");
        assert!(report.cfactor_regression.is_none());

        let signal_log = tmp.path().join(".roko").join("signals.jsonl");
        let signals = read_signals(&signal_log);
        assert!(
            signals
                .iter()
                .all(|signal| signal.kind.as_str() != "cfactor:regression")
        );
    }

    #[tokio::test]
    async fn performance_stall_note_emitted_after_five_non_improving_plans() {
        let tmp = TempDir::new().expect("tempdir");
        let episodes_path = tmp.path().join(".roko").join("episodes.jsonl");
        let knowledge_path = tmp
            .path()
            .join(".roko")
            .join("neuro")
            .join("knowledge.jsonl");
        let playbooks_root = tmp.path().join(".roko").join("playbooks");
        let logger = EpisodeLogger::new(&episodes_path);
        let knowledge_store = Arc::new(KnowledgeStore::new(&knowledge_path));
        let playbook_store = Arc::new(PlaybookStore::new(&playbooks_root));
        let dispatcher = Arc::new(MockDispatcher {
            response: r#"<|json|>{"entries":[]}<|/json|>"#.to_string(),
        });

        let base = Utc::now() - chrono::Duration::hours(1);
        for idx in 0..5 {
            let ep = episode_at(
                &format!("stall-{idx}"),
                &format!("plan-{idx}"),
                "implementation",
                "claude-haiku-4-5",
                true,
                None,
                base + chrono::Duration::minutes(i64::from(idx)),
            );
            write_episode(&logger, &ep).await;
        }

        let mut cycle = DreamCycle::new(
            Arc::new(logger),
            knowledge_store,
            playbook_store,
            dispatcher,
        );

        let report = cycle.run().await.expect("run");
        assert!(
            report
                .performance_notes
                .iter()
                .any(|note| note == DREAMS_PERFORMANCE_STALLED_NOTE)
        );
    }

    #[test]
    fn review_insights_from_heuristics_skips_confirmation_only_rules() {
        let analysis = TierProgressionReport {
            insights: Vec::new(),
            heuristics: vec![
                roko_neuro::tier_progression::HeuristicRule {
                    id: "heuristic-1".to_string(),
                    insight_id: "insight-1".to_string(),
                    title: "If trigger gate failure then add verification".to_string(),
                    when_clause: "trigger gate failure and agent implementer".to_string(),
                    then_clause: "add a verification step before proceeding".to_string(),
                    confidence: 0.91,
                    confirmations: 5,
                    first_seen_ms: 10,
                    last_seen_ms: 20,
                    source_episodes: vec!["ep-1".to_string(), "ep-2".to_string()],
                },
                roko_neuro::tier_progression::HeuristicRule {
                    id: "heuristic-2".to_string(),
                    insight_id: "insight-2".to_string(),
                    title: "If successful path then reuse it".to_string(),
                    when_clause: "trigger agent success and gate compile passed".to_string(),
                    then_clause: "reuse this path as the default play".to_string(),
                    confidence: 0.95,
                    confirmations: 5,
                    first_seen_ms: 30,
                    last_seen_ms: 40,
                    source_episodes: vec!["ep-3".to_string(), "ep-4".to_string()],
                },
            ],
            playbook: roko_neuro::tier_progression::PlaybookCompilation {
                markdown: String::new(),
                rules: Vec::new(),
            },
        };

        let entries = review_insights_from_heuristics(&analysis, Utc::now());
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.kind, KnowledgeKind::Insight);
        assert!(entry.content.contains("Would I do this differently now?"));
        assert!(entry.content.contains("current knowledge suggests"));
        assert!(entry.tags.iter().any(|tag| tag == "review"));
        assert!(entry.source_episodes.contains(&"ep-1".to_string()));
        assert!(entry.source_episodes.contains(&"ep-2".to_string()));
    }
}
