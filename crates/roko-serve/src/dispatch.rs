//! Event-driven dispatch loop for webhook signals.
//!
//! The loop listens on the shared server event bus, extracts webhook
//! signals, resolves matching subscriptions, and spawns agent dispatches
//! while enforcing per-subscription concurrency, cooldown, and dedup
//! constraints.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use chrono::Utc;
use parking_lot::{Mutex, RwLock};
use regex::Regex;
use roko_agent::chat_types::FinishReason;
use roko_agent::mcp::{McpConfig, McpServerConfig, find_mcp_config};
use roko_agent::provider::{AgentOptions, create_agent_for_model};
use roko_agent::{Agent, AgentResult};
use roko_compose::SystemPromptBuilder;
use roko_core::OperatingFrequency;
use roko_core::agent::AgentRole;
use roko_core::config::schema::{RokoConfig, SubscriptionConfig, SubscriptionFilterConfig};
use roko_core::tool::ExternalAction;
use roko_core::tool::ToolRegistry;
use roko_core::tool::role_allowlist::role_allowlist;
use roko_core::{Body, Context as RokoContext, Engram, Kind, Provenance};
use roko_core::{ContentHash, Verdict};
use roko_daimon::{AffectEngine as _, AffectEvent};
use roko_learn::anomaly::{Anomaly, AnomalyDetector};
use roko_learn::cascade_router::CascadeRouter;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, EpisodeLogger, GateVerdict, Usage as EpisodeUsage};
use roko_learn::events::{AgentEvent, EventBus as LearningEventBus};
use roko_learn::prompt_experiment::ExperimentStore;
use roko_neuro::spawn_episode_distillation;
use roko_std::tool::StaticToolRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::events::ServerEvent;
use crate::runtime::RepoInfo;
use crate::state::{AppState, TemplateRunRecord};
use crate::templates::{AgentTemplate, TemplateRegistry};
use roko_fs::layout::RokoLayout;

/// Async agent-dispatch interface used by the routing loop.
#[async_trait]
pub trait AgentDispatcher: Send + Sync {
    /// Dispatch a signal through the agent template identified by `template`.
    async fn dispatch(&self, template: AgentTemplate, signal: Engram) -> Result<AgentResult>;
}

/// Public subscription filter type used by the subscription API.
pub type SubscriptionFilter = SubscriptionFilterConfig;

/// Extended episode metadata for webhook- and event-driven agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebhookEpisodeMetadata {
    /// Engram kind that triggered the agent.
    #[serde(default)]
    pub trigger_kind: String,
    /// Content hash of the trigger signal.
    #[serde(default)]
    pub trigger_signal_hash: String,
    /// Source integration that emitted the trigger.
    #[serde(default)]
    pub trigger_source: String,
    /// Template name used to dispatch the agent.
    #[serde(default)]
    pub agent_template: String,
    /// Optional experiment variant for A/B testing.
    #[serde(default)]
    pub experiment_variant: Option<String>,
    /// External actions performed while handling the trigger.
    #[serde(default)]
    pub external_actions: Vec<ExternalAction>,
}

impl WebhookEpisodeMetadata {
    fn new(
        trigger_kind: impl Into<String>,
        trigger_signal_hash: impl Into<String>,
        trigger_source: impl Into<String>,
        agent_template: impl Into<String>,
        experiment_variant: Option<String>,
        external_actions: Vec<ExternalAction>,
    ) -> Self {
        Self {
            trigger_kind: trigger_kind.into(),
            trigger_signal_hash: trigger_signal_hash.into(),
            trigger_source: trigger_source.into(),
            agent_template: agent_template.into(),
            experiment_variant,
            external_actions,
        }
    }
}

/// Resolved per-repo context for a dispatch. When a webhook signal comes
/// from a known repository, the dispatch uses repo-specific paths for data
/// isolation (episodes, learn artifacts, signals).
#[derive(Debug, Clone)]
struct RepoContext {
    /// Short repo name (e.g. `"roko"`) — used for `.roko/repos/{name}/`.
    name: String,
    /// Filesystem path to the repository root (the agent's working directory).
    repo_workdir: PathBuf,
    /// Per-repo layout under `.roko/repos/{name}/`.
    layout: RokoLayout,
    /// Optional merged `RokoConfig` from the repo's `.roko/roko.toml`.
    /// When present, repo-local settings take priority over globals.
    repo_config: Option<RokoConfig>,
}

/// Extract the repo full name (e.g. `"nunchi/roko"`) from a webhook signal's
/// JSON body by probing `repository.full_name` and `repository.name`.
fn signal_repo_full_name(signal: &Engram) -> Option<String> {
    let candidates = signal_repo_candidates(signal);
    candidates.into_iter().next().map(str::to_string)
}

/// Attempt to resolve a [`RepoContext`] from a signal using the runtime's
/// repo registry. Returns `None` when the signal's repo is not configured.
fn resolve_repo_context(state: &AppState, signal: &Engram) -> Option<RepoContext> {
    let full_name = signal_repo_full_name(signal)?;
    let repo_workdir = state.runtime.resolve_repo_workdir(&full_name)?;

    // Use the bare repo name for the per-repo data directory.
    let bare_name = full_name
        .rsplit('/')
        .next()
        .unwrap_or(&full_name)
        .to_string();
    let layout = RokoLayout::for_repo(&state.workdir, &bare_name);

    // Load repo-local config override (4B.05). When present, the repo's
    // .roko/roko.toml takes priority over the global config for settings
    // like model selection, gate thresholds, etc.
    let repo_config = state.runtime.repo_roko_config(&bare_name);
    if repo_config.is_some() {
        info!(repo = %bare_name, "loaded repo-local config override");
    }

    Some(RepoContext {
        name: bare_name,
        repo_workdir,
        layout,
        repo_config,
    })
}

/// Template-backed agent runner used by the webhook dispatch loop.
#[derive(Clone, Debug)]
pub struct TemplateAgentDispatcher {
    workdir: PathBuf,
    base_mcp_config: Option<PathBuf>,
    /// Full config used to resolve providers and models for dispatch.
    roko_config: RokoConfig,
    /// Optional per-repo working directory override. When set, the agent
    /// process runs in this directory instead of the global `workdir`.
    repo_workdir: Option<PathBuf>,
    /// Configured repos (injected for cross-repo context in prompts).
    repo_listing: Vec<RepoInfo>,
}

#[derive(Debug)]
struct DispatchOutcome {
    result: AgentResult,
    gate_verdicts: Vec<GateVerdict>,
    success: bool,
    model_used: String,
}

#[derive(Debug)]
struct QueuedAnomaly {
    anomaly: Anomaly,
    model_slug: String,
}

#[derive(Debug)]
struct DispatchAnomalySession {
    detector: AnomalyDetector,
    pending: VecDeque<QueuedAnomaly>,
}

impl DispatchAnomalySession {
    fn new() -> Self {
        Self {
            detector: AnomalyDetector::new(Utc::now().timestamp_millis()),
            pending: VecDeque::new(),
        }
    }
}

static DISPATCH_ANOMALY_SESSIONS: OnceLock<Mutex<HashMap<PathBuf, DispatchAnomalySession>>> =
    OnceLock::new();

#[derive(Debug, Clone)]
struct EfficiencyTracker {
    path: PathBuf,
}

impl EfficiencyTracker {
    fn new(workdir: &Path) -> Self {
        Self {
            path: RokoLayout::for_project(workdir).efficiency_path(),
        }
    }

    fn for_layout(layout: &RokoLayout) -> Self {
        Self {
            path: layout.efficiency_path(),
        }
    }

    async fn record_event(
        &self,
        template_name: &str,
        turns: u64,
        tokens: u64,
        success: bool,
    ) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create {}", parent.display()))?;
        }

        let event = AgentEfficiencyEvent {
            agent_id: template_name.to_string(),
            role: template_name.to_string(),
            backend: "roko-serve".to_string(),
            model: template_name.to_string(),
            plan_id: String::new(),
            task_id: String::new(),
            attempt_id: format!("roko-serve:{template_name}:{turns}"),
            input_tokens: tokens,
            output_tokens: 0,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.0,
            cost_usd_without_cache: 0.0,
            prompt_sections: Vec::new(),
            total_prompt_tokens: tokens,
            system_prompt_tokens: 0,
            tools_available: 0,
            tools_used: 0,
            tool_calls: Vec::new(),
            wall_time_ms: 0,
            duration_ms: 0,
            time_to_first_token_ms: 0,
            was_warm_start: false,
            iteration: turns.min(u64::from(u32::MAX)) as u32,
            gate_passed: success,
            outcome: if success {
                "success".to_string()
            } else {
                "failure".to_string()
            },
            gate_errors: Vec::new(),
            model_used: template_name.to_string(),
            frequency: OperatingFrequency::Theta,
            strategy_attempted: String::new(),
            timestamp: Utc::now().to_rfc3339(),
        };

        let mut line = serde_json::to_string(&event)?;
        line.push('\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .with_context(|| format!("open {}", self.path.display()))?;
        file.write_all(line.as_bytes()).await?;
        Ok(())
    }
}

fn dispatch_anomaly_sessions() -> &'static Mutex<HashMap<PathBuf, DispatchAnomalySession>> {
    DISPATCH_ANOMALY_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_dispatch_anomaly_session<T>(
    session_root: &Path,
    f: impl FnOnce(&mut DispatchAnomalySession) -> T,
) -> T {
    let sessions = dispatch_anomaly_sessions();
    let mut sessions = sessions.lock();
    let session = sessions
        .entry(session_root.to_path_buf())
        .or_insert_with(DispatchAnomalySession::new);
    f(session)
}

fn drain_pending_anomalies(session_root: &Path) -> Vec<QueuedAnomaly> {
    with_dispatch_anomaly_session(session_root, |session| session.pending.drain(..).collect())
}

fn queue_pending_anomaly(session_root: &Path, anomaly: Anomaly, model_slug: String) {
    with_dispatch_anomaly_session(session_root, |session| {
        session.pending.push_back(QueuedAnomaly {
            anomaly,
            model_slug,
        });
    });
}

fn prompt_hash_u64(signal: &Engram) -> u64 {
    let hash = signal.content_hash();
    let bytes: [u8; 8] = hash.0[..8].try_into().expect("content hash prefix");
    u64::from_be_bytes(bytes)
}

fn downgrade_model_slug(current: &str, config: &RokoConfig) -> Option<String> {
    if let Some(fallback) = config.agent.fallback_model.as_ref()
        && !fallback.trim().is_empty()
        && fallback != current
    {
        return Some(fallback.clone());
    }

    if current.contains("opus") {
        return Some(current.replacen("opus", "sonnet", 1));
    }
    if current.contains("sonnet") {
        return Some(current.replacen("sonnet", "haiku", 1));
    }
    if current.contains("gpt-5-pro") {
        return Some(current.replacen("gpt-5-pro", "gpt-5", 1));
    }
    if current.contains("gpt-5") && !current.contains("mini") {
        return Some(current.replacen("gpt-5", "gpt-5-mini", 1));
    }
    None
}

async fn apply_pending_anomalies(
    state: &Arc<AppState>,
    template: &AgentTemplate,
    session_root: &Path,
    repo_layout: Option<&RokoLayout>,
    routed_model: &mut String,
    config: &RokoConfig,
) -> Result<()> {
    let pending = drain_pending_anomalies(session_root);
    for queued in pending {
        match queued.anomaly {
            Anomaly::CostSpike { z_score } => {
                warn!(
                    template = %template.name,
                    model = %routed_model,
                    z_score,
                    "cost spike detected; downgrading model"
                );
                if let Some(downgraded) = downgrade_model_slug(routed_model, config) {
                    warn!(
                        template = %template.name,
                        from_model = %routed_model,
                        to_model = %downgraded,
                        "applying anomaly-driven model downgrade"
                    );
                    *routed_model = downgraded;
                }
            }
            Anomaly::QualityDegradation { avg_drop } => {
                warn!(
                    template = %template.name,
                    model = %queued.model_slug,
                    avg_drop,
                    "quality degradation detected; recording negative router observation"
                );
                let mut observation_template = template.clone();
                observation_template.model = queued.model_slug;
                record_cascade_router_outcome_with_layout(
                    state,
                    &observation_template,
                    false,
                    repo_layout,
                )
                .await?;
            }
            Anomaly::PromptLoop { repeated_count } => {
                return Err(anyhow::anyhow!(
                    "prompt loop detected after {} identical prompts",
                    repeated_count
                ));
            }
            Anomaly::BudgetExhausted { used, limit } => {
                return Err(anyhow::anyhow!(
                    "session budget exhausted: ${used:.2} >= ${limit:.2}"
                ));
            }
        }
    }

    Ok(())
}

async fn run_anomaly_preflight(
    state: &Arc<AppState>,
    template: &AgentTemplate,
    dispatch_signal: &Engram,
    repo_ctx: Option<&RepoContext>,
    routed_model: &mut String,
) -> Result<()> {
    let session_root = repo_ctx
        .map(|ctx| ctx.repo_workdir.as_path())
        .unwrap_or_else(|| state.workdir.as_path());
    let repo_layout = repo_ctx.map(|ctx| &ctx.layout);
    let base_config = state.load_roko_config().as_ref().clone();
    let effective_config = repo_ctx
        .and_then(|ctx| ctx.repo_config.clone())
        .unwrap_or(base_config);

    apply_pending_anomalies(
        state,
        template,
        session_root,
        repo_layout,
        routed_model,
        &effective_config,
    )
    .await?;

    let prompt_hash = prompt_hash_u64(dispatch_signal);
    if let Some(Anomaly::PromptLoop { repeated_count }) =
        with_dispatch_anomaly_session(session_root, |session| {
            session.detector.check_prompt(prompt_hash)
        })
    {
        return Err(anyhow::anyhow!(
            "prompt loop detected after {} identical prompts",
            repeated_count
        ));
    }

    let budget_limit = f64::from(effective_config.budget.max_plan_usd);
    if let Some(Anomaly::BudgetExhausted { used, limit }) =
        with_dispatch_anomaly_session(session_root, |session| {
            session.detector.check_budget(budget_limit)
        })
    {
        let _ = (used, limit);
        return Err(anyhow::anyhow!(
            "session budget exhausted: ${used:.2} >= ${limit:.2}"
        ));
    }

    Ok(())
}

fn record_post_turn_anomalies(
    session_root: &Path,
    model_slug: &str,
    cost_usd: f64,
    quality_score: f64,
    budget_limit: f64,
) {
    let anomalies = with_dispatch_anomaly_session(session_root, |session| {
        let mut observed = Vec::new();
        if let Some(anomaly) = session.detector.check_cost(cost_usd) {
            observed.push(anomaly);
        }
        if let Some(anomaly) = session.detector.check_quality(quality_score) {
            observed.push(anomaly);
        }
        if let Some(anomaly) = session.detector.check_budget(budget_limit) {
            observed.push(anomaly);
        }
        observed
    });

    for anomaly in anomalies {
        queue_pending_anomaly(session_root, anomaly, model_slug.to_string());
    }
}

impl TemplateAgentDispatcher {
    /// Create a dispatcher rooted at `workdir`.
    #[must_use]
    pub fn new(
        workdir: PathBuf,
        base_mcp_config: Option<PathBuf>,
        roko_config: RokoConfig,
    ) -> Self {
        Self {
            workdir,
            base_mcp_config,
            roko_config,
            repo_workdir: None,
            repo_listing: Vec::new(),
        }
    }
}

/// Start the subscription dispatch loop in the background.
#[must_use]
pub fn start_dispatch_loop(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let roko_config = state.load_roko_config().as_ref().clone();
        let dispatcher = Arc::new(TemplateAgentDispatcher::new(
            state.workdir.clone(),
            None,
            roko_config,
        ));
        dispatch_loop(state, dispatcher).await;
    })
}

#[async_trait]
impl AgentDispatcher for TemplateAgentDispatcher {
    async fn dispatch(&self, template: AgentTemplate, signal: Engram) -> Result<AgentResult> {
        let experiment_variant = template.experiment.as_ref().and_then(|experiment| {
            load_template_experiment_variant(&self.workdir, &experiment.name)
        });
        let mut system_prompt = build_template_system_prompt(
            &template,
            Some(&signal),
            experiment_variant
                .as_ref()
                .map(|(_, content)| content.as_str()),
        );
        // 4B.06: Inject cross-repo context so the agent knows about
        // the multi-repo setup.
        if !self.repo_listing.is_empty() {
            system_prompt.push_str(&build_cross_repo_context(&self.repo_listing));
        }
        let allowed_tools = build_allowed_tools_csv(&template);
        // Use repo-specific workdir when available (4B.04).
        let effective_workdir = self.repo_workdir.as_deref().unwrap_or(&self.workdir);
        let mcp_config = resolve_template_mcp_config(
            self.base_mcp_config.as_ref(),
            effective_workdir,
            &template,
        )?;
        let agent = build_agent(
            &self.roko_config,
            &template,
            &system_prompt,
            &allowed_tools,
            mcp_config.as_ref(),
        )?;
        let ctx = dispatch_context(&template, &signal);
        let mut result = agent.run(&signal, &ctx).await;
        if let Some((variant_id, _)) = experiment_variant {
            result
                .output
                .tags
                .insert("experiment_variant".into(), variant_id.clone());
            result
                .output
                .tags
                .insert("experiment_variant_id".into(), variant_id);
            result.output.id = result.output.content_hash();
        }
        Ok(result)
    }
}

/// A subscription from signal trigger to agent template.
#[derive(Debug)]
pub struct Subscription {
    /// Unique ID for this subscription.
    pub id: String,
    /// Agent template name associated with this subscription.
    pub template: String,
    /// Engram kind glob used to match incoming signals.
    pub trigger: String,
    /// Additional filters applied after the trigger matches.
    pub filter: SubscriptionFilter,
    /// Maximum number of concurrent agents for this subscription.
    pub concurrency_limit: usize,
    /// Minimum seconds between dispatches.
    pub cooldown_secs: u64,
    /// Whether the subscription is enabled.
    pub enabled: bool,
    subscription_id: usize,
    dedup_ttl: Duration,
    state: Arc<SubscriptionState>,
}

#[derive(Debug)]
struct SubscriptionState {
    recent_signals: Mutex<HashMap<ContentHash, Instant>>,
}

impl SubscriptionState {
    fn new() -> Self {
        Self {
            recent_signals: Mutex::new(HashMap::new()),
        }
    }
}

impl Clone for Subscription {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            template: self.template.clone(),
            trigger: self.trigger.clone(),
            filter: self.filter.clone(),
            concurrency_limit: self.concurrency_limit,
            cooldown_secs: self.cooldown_secs,
            enabled: self.enabled,
            subscription_id: self.subscription_id,
            dedup_ttl: self.dedup_ttl,
            state: Arc::clone(&self.state),
        }
    }
}

impl Subscription {
    /// Create a new enabled subscription with conservative defaults.
    #[must_use]
    pub fn new(template: impl Into<String>, trigger: impl Into<String>) -> Self {
        Self::from_config(SubscriptionConfig {
            template: template.into(),
            trigger: trigger.into(),
            ..SubscriptionConfig::default()
        })
        .with_dedup_ttl(Duration::from_secs(60))
    }

    /// Build a subscription from a config entry.
    #[must_use]
    pub fn from_config(config: SubscriptionConfig) -> Self {
        Self {
            id: String::new(),
            template: config.template,
            trigger: config.trigger,
            filter: config.filter,
            concurrency_limit: config.concurrency_limit,
            cooldown_secs: config.cooldown_secs,
            enabled: config.enabled,
            subscription_id: usize::MAX,
            dedup_ttl: Duration::from_secs(60),
            state: Arc::new(SubscriptionState::new()),
        }
    }

    /// Replace the filter criteria for this subscription.
    #[must_use]
    pub fn with_filter(mut self, filter: SubscriptionFilterConfig) -> Self {
        self.filter = filter;
        self
    }

    /// Set the maximum number of concurrent dispatches allowed.
    #[must_use]
    pub fn with_concurrency_limit(mut self, limit: usize) -> Self {
        self.concurrency_limit = limit;
        self
    }

    /// Set the minimum delay between dispatches.
    #[must_use]
    pub fn with_cooldown(mut self, cooldown: Duration) -> Self {
        self.cooldown_secs = cooldown.as_secs();
        self
    }

    /// Set the deduplication window.
    #[must_use]
    pub fn with_dedup_ttl(mut self, ttl: Duration) -> Self {
        self.dedup_ttl = ttl;
        self
    }

    /// Disable the subscription.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Convert this runtime subscription back into its persisted config form.
    #[must_use]
    pub fn to_config(&self) -> SubscriptionConfig {
        SubscriptionConfig {
            template: self.template.clone(),
            trigger: self.trigger.clone(),
            trigger_config: None,
            filter: self.filter.clone(),
            concurrency_limit: self.concurrency_limit,
            cooldown_secs: self.cooldown_secs,
            debounce_ms: 0,
            enabled: self.enabled,
        }
    }

    fn with_subscription_id(mut self, subscription_id: usize) -> Self {
        self.subscription_id = subscription_id;
        if self.id.is_empty() {
            self.id = subscription_id.to_string();
        }
        self
    }

    /// Stable registry identifier for this subscription.
    #[must_use]
    pub const fn subscription_id(&self) -> usize {
        self.subscription_id
    }

    /// Agent template name associated with this subscription.
    #[must_use]
    pub fn template(&self) -> &str {
        &self.template
    }

    /// Trigger pattern used to match signal kinds.
    #[must_use]
    pub fn trigger(&self) -> &str {
        &self.trigger
    }

    /// Return the configured filter criteria.
    #[must_use]
    pub fn filter(&self) -> &SubscriptionFilterConfig {
        &self.filter
    }

    /// Whether the subscription is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check whether this subscription should trigger for `signal`.
    #[must_use]
    pub fn matches(&self, signal: &Engram) -> bool {
        self.is_enabled()
            && glob_match(self.trigger(), signal.kind.as_str())
            && subscription_filter_matches(self.filter(), signal)
    }

    /// Reserve a concurrency slot if the current active count is below the limit.
    #[must_use]
    pub fn check_concurrency_limit(&self, registry: &SubscriptionRegistry) -> bool {
        registry.check_concurrency_limit(self)
    }

    /// Release one reserved concurrency slot.
    pub fn release_concurrency(&self, registry: &SubscriptionRegistry) {
        registry.release_concurrency(self);
    }

    /// Check and update the deduplication gate using the signal content hash.
    #[must_use]
    pub fn check_dedup(&self, signal: &Engram) -> bool {
        if self.dedup_ttl.is_zero() {
            return true;
        }

        let now = Instant::now();
        let signal_hash = signal.content_hash();
        let mut recent = self.state.recent_signals.lock();

        recent.retain(|_, seen_at| now.duration_since(*seen_at) < self.dedup_ttl);

        if let Some(seen_at) = recent.get(&signal_hash) {
            if now.duration_since(*seen_at) < self.dedup_ttl {
                return false;
            }
        }

        recent.insert(signal_hash, now);
        true
    }
}

/// In-memory subscription registry.
#[derive(Clone, Debug, Default)]
pub struct SubscriptionRegistry {
    subscriptions: Arc<RwLock<Vec<Subscription>>>,
    active_counts: Arc<Mutex<HashMap<usize, AtomicUsize>>>,
    last_dispatches: Arc<Mutex<HashMap<usize, Instant>>>,
    next_subscription_id: Arc<AtomicUsize>,
}

impl SubscriptionRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load subscriptions from `roko.toml` and `.roko/subscriptions/*.toml`.
    #[must_use]
    pub fn load_from_project(workdir: impl AsRef<Path>, config: &RokoConfig) -> Self {
        let mut subscriptions: Vec<Subscription> = config
            .subscriptions
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, config)| {
                let mut subscription = Subscription::from_config(config);
                subscription.id = format!("config-{index}");
                subscription
            })
            .collect();

        let subs_dir = workdir.as_ref().join(".roko").join("subscriptions");
        if let Ok(entries) = fs::read_dir(&subs_dir) {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
                .collect();
            files.sort();

            for path in files {
                match load_subscription_file(&path) {
                    Ok(mut loaded) => subscriptions.append(&mut loaded),
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "failed to load subscription file")
                    }
                }
            }
        } else if subs_dir.exists() {
            warn!(path = %subs_dir.display(), "failed to read subscription directory");
        }

        let registry = Self::with_subscriptions(subscriptions);
        tracing::info!(
            count = registry.subscriptions.read().len(),
            "loaded subscriptions"
        );
        registry
    }

    /// Create a registry seeded with subscriptions.
    #[must_use]
    pub fn with_subscriptions(subscriptions: Vec<Subscription>) -> Self {
        let active_counts = Arc::new(Mutex::new(HashMap::new()));
        let next_subscription_id = Arc::new(AtomicUsize::new(0));
        let subscriptions = subscriptions
            .into_iter()
            .enumerate()
            .map(|(subscription_id, subscription)| {
                active_counts
                    .lock()
                    .insert(subscription_id, AtomicUsize::new(0));
                next_subscription_id.store(subscription_id + 1, Ordering::Release);
                subscription.with_subscription_id(subscription_id)
            })
            .collect();

        Self {
            subscriptions: Arc::new(RwLock::new(subscriptions)),
            active_counts,
            last_dispatches: Arc::new(Mutex::new(HashMap::new())),
            next_subscription_id,
        }
    }

    /// Add a subscription to the registry.
    pub fn insert(&self, subscription: Subscription) {
        let subscription_id = self.next_subscription_id.fetch_add(1, Ordering::AcqRel);
        self.active_counts
            .lock()
            .insert(subscription_id, AtomicUsize::new(0));
        self.last_dispatches.lock().remove(&subscription_id);
        self.subscriptions
            .write()
            .push(subscription.with_subscription_id(subscription_id));
    }

    /// Return a snapshot of all subscriptions in the registry.
    #[must_use]
    pub fn all(&self) -> Vec<Subscription> {
        self.subscriptions.read().clone()
    }

    /// Replace the registry contents with a fresh subscription snapshot.
    ///
    /// Existing active dispatches keep their current internal IDs because we
    /// only append new registry IDs for the replacement snapshot.
    pub fn replace_with(&self, subscriptions: Vec<Subscription>) -> usize {
        let mut snapshot = Vec::with_capacity(subscriptions.len());

        for subscription in subscriptions {
            let subscription_id = self.next_subscription_id.fetch_add(1, Ordering::AcqRel);
            self.active_counts
                .lock()
                .insert(subscription_id, AtomicUsize::new(0));
            snapshot.push(subscription.with_subscription_id(subscription_id));
        }

        *self.subscriptions.write() = snapshot;
        self.subscriptions.read().len()
    }

    /// Look up a subscription by its public ID.
    #[must_use]
    pub fn get_by_id(&self, id: &str) -> Option<Subscription> {
        self.subscriptions
            .read()
            .iter()
            .find(|subscription| subscription.id == id)
            .cloned()
    }

    /// Replace a subscription by public ID and preserve its internal registry ID.
    #[must_use]
    pub fn update_by_id(&self, id: &str, subscription: Subscription) -> Option<Subscription> {
        let mut subscriptions = self.subscriptions.write();
        let existing = subscriptions
            .iter_mut()
            .find(|candidate| candidate.id == id)?;
        let subscription_id = existing.subscription_id();
        *existing = subscription.with_subscription_id(subscription_id);
        Some(existing.clone())
    }

    /// Remove a subscription by public ID.
    pub fn remove_by_id(&self, id: &str) -> Option<Subscription> {
        let mut subscriptions = self.subscriptions.write();
        let index = subscriptions
            .iter()
            .position(|subscription| subscription.id == id)?;
        let removed = subscriptions.remove(index);
        self.active_counts.lock().remove(&removed.subscription_id());
        self.last_dispatches
            .lock()
            .remove(&removed.subscription_id());
        Some(removed)
    }

    /// Return subscriptions whose trigger and filters match `signal`.
    #[must_use]
    pub fn find_matching(&self, signal: &Engram) -> Vec<Subscription> {
        self.subscriptions
            .read()
            .iter()
            .filter(|subscription| subscription.matches(signal))
            .cloned()
            .collect()
    }

    /// Reserve a concurrency slot for `subscription` if it is below its limit.
    #[must_use]
    pub fn check_concurrency_limit(&self, subscription: &Subscription) -> bool {
        if subscription.concurrency_limit == 0 {
            return false;
        }

        let mut active_counts = self.active_counts.lock();
        let active = active_counts
            .entry(subscription.subscription_id())
            .or_insert_with(|| AtomicUsize::new(0));

        let mut current = active.load(Ordering::Acquire);
        loop {
            if current >= subscription.concurrency_limit {
                return false;
            }

            match active.compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(next) => current = next,
            }
        }
    }

    /// Release one reserved concurrency slot for `subscription`.
    pub fn release_concurrency(&self, subscription: &Subscription) {
        let active_counts = self.active_counts.lock();
        if let Some(active) = active_counts.get(&subscription.subscription_id()) {
            let _ = active.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
        }
    }

    /// Check and update the cooldown gate for `subscription`.
    #[must_use]
    pub fn check_cooldown(&self, subscription: &Subscription) -> bool {
        if subscription.cooldown_secs == 0 {
            return true;
        }

        let cooldown = Duration::from_secs(subscription.cooldown_secs);
        let now = Instant::now();
        let mut last_dispatches = self.last_dispatches.lock();
        let Some(previous) = last_dispatches.get(&subscription.subscription_id()) else {
            last_dispatches.insert(subscription.subscription_id(), now);
            return true;
        };

        if now.duration_since(*previous) < cooldown {
            return false;
        }

        last_dispatches.insert(subscription.subscription_id(), now);
        true
    }
}

#[derive(Debug, Default, Deserialize)]
struct SubscriptionFile {
    #[serde(default)]
    subscription: Vec<SubscriptionConfig>,
    #[serde(default)]
    subscriptions: Vec<SubscriptionConfig>,
}

fn load_subscription_file(path: &Path) -> anyhow::Result<Vec<Subscription>> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let base_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("subscription")
        .to_string();

    if let Ok(config) = toml::from_str::<SubscriptionConfig>(&text) {
        let mut subscription = Subscription::from_config(config);
        subscription.id = base_id;
        return Ok(vec![subscription]);
    }

    let file: SubscriptionFile =
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    let mut subscriptions = Vec::new();
    let mut sequence = 0usize;
    for config in file.subscription {
        let mut subscription = Subscription::from_config(config);
        subscription.id = if sequence == 0 {
            base_id.clone()
        } else {
            format!("{base_id}-{sequence}")
        };
        subscriptions.push(subscription);
        sequence += 1;
    }
    for config in file.subscriptions {
        let mut subscription = Subscription::from_config(config);
        subscription.id = if sequence == 0 {
            base_id.clone()
        } else {
            format!("{base_id}-{sequence}")
        };
        subscriptions.push(subscription);
        sequence += 1;
    }

    if subscriptions.is_empty() {
        anyhow::bail!("no subscriptions found");
    }

    Ok(subscriptions)
}

fn subscription_filter_matches(filter: &SubscriptionFilterConfig, signal: &Engram) -> bool {
    if filter.is_empty() {
        return true;
    }

    if !filter.repo.is_empty() && !matches_any_glob(signal_repo_candidates(signal), &filter.repo) {
        return false;
    }

    if !filter.branch.is_empty()
        && !matches_any_regex(signal_branch_candidates(signal), &filter.branch)
    {
        return false;
    }

    if !filter.path.is_empty() && !matches_any_glob(signal_path_candidates(signal), &filter.path) {
        return false;
    }

    if !filter.label.is_empty()
        && !matches_any_exact(signal_label_candidates(signal), &filter.label)
    {
        return false;
    }

    if !filter.author.is_empty()
        && !matches_any_exact(signal_author_candidates(signal), &filter.author)
    {
        return false;
    }

    true
}

fn matches_any_glob<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
    patterns: &[String],
) -> bool {
    let candidates: Vec<&'a str> = candidates.into_iter().collect();
    patterns.iter().any(|pattern| {
        candidates
            .iter()
            .copied()
            .any(|candidate| glob_match(pattern, candidate))
    })
}

fn matches_any_regex<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
    patterns: &[String],
) -> bool {
    let candidates: Vec<&'a str> = candidates.into_iter().collect();
    patterns.iter().any(|pattern| {
        Regex::new(pattern).ok().is_some_and(|regex: Regex| {
            candidates
                .iter()
                .copied()
                .any(|candidate| regex.is_match(candidate))
        })
    })
}

fn matches_any_exact<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
    patterns: &[String],
) -> bool {
    let candidates: Vec<&'a str> = candidates.into_iter().collect();
    patterns.iter().any(|pattern| {
        candidates
            .iter()
            .copied()
            .any(|candidate| candidate == pattern)
    })
}

fn signal_repo_candidates(signal: &Engram) -> Vec<&str> {
    json_string_fields(
        &signal.body,
        &[
            &["repository", "full_name"],
            &["repository", "name"],
            &["repo", "full_name"],
            &["repo", "name"],
        ],
    )
}

fn signal_branch_candidates(signal: &Engram) -> Vec<&str> {
    let mut values = json_string_fields(
        &signal.body,
        &[
            &["ref"],
            &["branch"],
            &["repository", "default_branch"],
            &["pull_request", "base", "ref"],
            &["pull_request", "head", "ref"],
        ],
    );

    let mut normalized = Vec::new();
    for value in &values {
        if let Some(branch) = value.strip_prefix("refs/heads/") {
            normalized.push(branch);
        }
        if let Some(branch) = value.strip_prefix("refs/tags/") {
            normalized.push(branch);
        }
    }
    values.extend(normalized);
    values
}

fn signal_path_candidates(signal: &Engram) -> Vec<&str> {
    let mut values = Vec::new();
    values.extend(json_string_array_fields(
        &signal.body,
        &[
            &["paths"],
            &["files"],
            &["changed_files"],
            &["head_commit", "added"],
            &["head_commit", "modified"],
            &["head_commit", "removed"],
        ],
    ));
    values.extend(json_string_array_fields(
        &signal.body,
        &[
            &["commits", "*", "added"],
            &["commits", "*", "modified"],
            &["commits", "*", "removed"],
        ],
    ));
    values
}

fn signal_label_candidates(signal: &Engram) -> Vec<&str> {
    json_stringish_array_fields(
        &signal.body,
        &[
            &["labels"],
            &["issue", "labels"],
            &["pull_request", "labels"],
        ],
    )
}

fn signal_author_candidates(signal: &Engram) -> Vec<&str> {
    json_loginish_fields(
        &signal.body,
        &[
            &["sender"],
            &["user"],
            &["issue", "user"],
            &["pull_request", "user"],
            &["pull_request_review", "user"],
            &["review", "user"],
            &["comment", "user"],
            &["head_commit", "author"],
            &["head_commit", "committer"],
        ],
    )
}

fn json_string_fields<'a>(body: &'a Body, paths: &[&[&str]]) -> Vec<&'a str> {
    match body {
        Body::Json(value) => paths
            .iter()
            .filter_map(|path| json_string_at(value, path))
            .collect(),
        _ => Vec::new(),
    }
}

fn json_stringish_array_fields<'a>(body: &'a Body, paths: &[&[&str]]) -> Vec<&'a str> {
    match body {
        Body::Json(value) => paths
            .iter()
            .flat_map(|path| json_stringish_array_at(value, path))
            .collect(),
        _ => Vec::new(),
    }
}

fn json_loginish_fields<'a>(body: &'a Body, paths: &[&[&str]]) -> Vec<&'a str> {
    match body {
        Body::Json(value) => paths
            .iter()
            .flat_map(|path| json_loginish_at(value, path))
            .collect(),
        _ => Vec::new(),
    }
}

fn json_string_array_fields<'a>(body: &'a Body, paths: &[&[&str]]) -> Vec<&'a str> {
    match body {
        Body::Json(value) => paths
            .iter()
            .flat_map(|path| json_string_array_at(value, path))
            .collect(),
        _ => Vec::new(),
    }
}

fn json_string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str()
}

fn json_string_array_at<'a>(value: &'a Value, path: &[&str]) -> Vec<&'a str> {
    if let Some((head, tail)) = path.split_first() {
        if *head == "*" {
            return match value {
                Value::Array(items) => items
                    .iter()
                    .flat_map(|item| json_string_array_at(item, tail))
                    .collect(),
                _ => Vec::new(),
            };
        }

        return match value.get(*head) {
            Some(next) => json_string_array_at(next, tail),
            None => Vec::new(),
        };
    }

    match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn json_stringish_array_at<'a>(value: &'a Value, path: &[&str]) -> Vec<&'a str> {
    if let Some((head, tail)) = path.split_first() {
        if *head == "*" {
            return match value {
                Value::Array(items) => items
                    .iter()
                    .flat_map(|item| json_stringish_array_at(item, tail))
                    .collect(),
                _ => Vec::new(),
            };
        }

        return match value.get(*head) {
            Some(next) => json_stringish_array_at(next, tail),
            None => Vec::new(),
        };
    }

    match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(items) => items.iter().flat_map(json_label_candidates).collect(),
        Value::Object(_) => json_label_candidates(value).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn json_loginish_at<'a>(value: &'a Value, path: &[&str]) -> Vec<&'a str> {
    if let Some((head, tail)) = path.split_first() {
        if *head == "*" {
            return match value {
                Value::Array(items) => items
                    .iter()
                    .flat_map(|item| json_loginish_at(item, tail))
                    .collect(),
                _ => Vec::new(),
            };
        }

        return match value.get(*head) {
            Some(next) => json_loginish_at(next, tail),
            None => Vec::new(),
        };
    }

    match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(items) => items.iter().flat_map(json_login_candidates).collect(),
        Value::Object(_) => json_login_candidates(value).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn json_label_candidates<'a>(value: &'a Value) -> Vec<&'a str> {
    if let Some(label) = value.as_str() {
        return vec![label];
    }

    value
        .get("name")
        .and_then(Value::as_str)
        .map(|name| vec![name])
        .unwrap_or_default()
}

fn json_login_candidates<'a>(value: &'a Value) -> Vec<&'a str> {
    if let Some(login) = value.as_str() {
        return vec![login];
    }

    value
        .get("login")
        .and_then(Value::as_str)
        .or_else(|| value.get("username").and_then(Value::as_str))
        .map(|login| vec![login])
        .unwrap_or_default()
}

/// Convert an in-process `ServerEvent` into a synthetic `Engram` so that
/// subscriptions can trigger on plan completions, gate results, episodes,
/// and job transitions -- not just external webhooks.
fn server_event_to_synthetic_signal(event: &ServerEvent) -> Option<Engram> {
    let (kind_str, body_json) = match event {
        ServerEvent::PlanCompleted {
            plan_id, success, ..
        } => (
            "plan_completed",
            serde_json::json!({ "plan_id": plan_id, "success": success }),
        ),
        ServerEvent::GateResult {
            plan_id,
            task_id,
            gate,
            rung,
            passed,
        } => (
            "gate_result",
            serde_json::json!({
                "plan_id": plan_id,
                "task_id": task_id,
                "gate": gate,
                "rung": rung,
                "passed": passed,
            }),
        ),
        ServerEvent::Episode {
            plan_id,
            task_id,
            passed,
        } => (
            "episode_recorded",
            serde_json::json!({
                "plan_id": plan_id,
                "task_id": task_id,
                "passed": passed,
            }),
        ),
        ServerEvent::JobTransitioned {
            job_id,
            from,
            to,
            assigned_to,
        } => (
            "job_transitioned",
            serde_json::json!({
                "job_id": job_id,
                "from": from,
                "to": to,
                "assigned_to": assigned_to,
            }),
        ),
        ServerEvent::RunCompleted { run_id, success } => (
            "run_completed",
            serde_json::json!({ "run_id": run_id, "success": success }),
        ),
        ServerEvent::DeploymentReady { id, url } => (
            "deployment_ready",
            serde_json::json!({ "deployment_id": id, "url": url }),
        ),
        ServerEvent::DeploymentFailed { id, reason } => (
            "deployment_failed",
            serde_json::json!({ "deployment_id": id, "reason": reason }),
        ),
        _ => return None,
    };

    let body = Body::from_json(&body_json).unwrap_or_else(|_| Body::text("{}"));
    let signal = Engram::builder(Kind::Custom(kind_str.into()))
        .body(body)
        .provenance(Provenance::trusted("roko-serve/dispatch"))
        .build();
    Some(signal)
}

/// Central event routing loop for webhook and in-process signals.
///
/// Handles both external webhook events (which arrive as `WebhookReceived`
/// signals) and in-process events (plan completions, gate results, episodes,
/// job transitions) that are converted into synthetic `Engram`s and matched
/// against subscriptions.
pub async fn dispatch_loop(state: Arc<AppState>, dispatcher: Arc<dyn AgentDispatcher>) {
    let subscriptions: SubscriptionRegistry = state.subscriptions.clone();
    let mut rx = state.event_bus.subscribe();
    let mut draining = false;

    loop {
        let envelope = if draining {
            match rx.try_recv() {
                Ok(envelope) => Some(envelope),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => None,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    warn!(n, "dispatch loop lagged, skipped events");
                    continue;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    warn!("dispatch event bus closed, stopping loop");
                    break;
                }
            }
        } else {
            tokio::select! {
                _ = state.cancel.cancelled() => {
                    draining = true;
                    continue;
                }
                result = rx.recv() => match result {
                    Ok(envelope) => Some(envelope),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(n, "dispatch loop lagged, skipped events");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        warn!("dispatch event bus closed, stopping loop");
                        break;
                    }
                },
            }
        };
        let Some(envelope) = envelope else {
            if draining {
                break;
            }
            continue;
        };

        // Extract a signal from either a webhook event or a synthetic
        // in-process event. Non-triggerable events are silently skipped.
        let signal = match envelope.payload {
            ServerEvent::WebhookReceived { signal } => signal,
            ref event => match server_event_to_synthetic_signal(event) {
                Some(signal) => signal,
                None => continue,
            },
        };

        if state.cancel.is_cancelled() {
            draining = true;
            continue;
        }

        // Resolve per-repo context from the signal's repository info.
        // When matched, data (episodes, learn artifacts) is isolated under
        // .roko/repos/{repo_name}/ and the agent runs in the repo's directory.
        let repo_ctx = resolve_repo_context(&state, &signal);
        if let Some(ref ctx) = repo_ctx {
            info!(
                repo = %ctx.name,
                workdir = %ctx.repo_workdir.display(),
                "resolved per-repo context for dispatch"
            );
        }

        let matched = subscriptions.find_matching(&signal);
        if matched.is_empty() {
            // Use per-repo episodes path for similarity suggestion when available.
            let episodes_path = repo_ctx
                .as_ref()
                .map(|ctx| ctx.layout.episodes_path())
                .unwrap_or_else(|| state.layout.episodes_path());
            match EpisodeLogger::suggest_template_from_recent_episodes(&episodes_path, &signal)
                .await
            {
                Ok(Some(template_name)) => {
                    info!(
                        signal = %signal.kind.as_str(),
                        template = %template_name,
                        "using similarity-based template suggestion"
                    );
                    let signal = signal.clone();
                    let dispatcher = Arc::clone(&dispatcher);
                    let state = Arc::clone(&state);
                    let repo_ctx = repo_ctx.clone();
                    let suggested_subscription =
                        Subscription::new(template_name.clone(), signal.kind.as_str());
                    tokio::spawn(async move {
                        Box::pin(dispatch_agent(
                            state,
                            suggested_subscription,
                            signal,
                            dispatcher,
                            repo_ctx,
                        ))
                        .await;
                    });
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(error = %err, "failed to suggest template from recent episodes");
                }
            }
            continue;
        }

        for sub in matched {
            if !subscriptions.check_concurrency_limit(&sub) {
                continue;
            }

            if subscriptions.check_cooldown(&sub) && sub.check_dedup(&signal) {
                let signal = signal.clone();
                let dispatcher = Arc::clone(&dispatcher);
                let state = Arc::clone(&state);
                let subscriptions = subscriptions.clone();
                let sub_for_task = sub.clone();
                let repo_ctx = repo_ctx.clone();
                tokio::spawn(async move {
                    Box::pin(dispatch_agent(
                        state,
                        sub_for_task.clone(),
                        signal,
                        dispatcher,
                        repo_ctx,
                    ))
                    .await;
                    sub_for_task.release_concurrency(&subscriptions);
                });
            } else {
                subscriptions.release_concurrency(&sub);
            }
        }
    }
}

async fn dispatch_agent(
    state: Arc<AppState>,
    subscription: Subscription,
    signal: Engram,
    dispatcher: Arc<dyn AgentDispatcher>,
    repo_ctx: Option<RepoContext>,
) {
    let template_name = subscription.template().to_owned();
    let started_at = Utc::now();
    let started = Instant::now();
    let template = {
        let templates = state.templates.read().await;
        templates.get(&template_name).cloned()
    };

    let Some(template) = template else {
        warn!(
            template = %template_name,
            "no template found for matched subscription"
        );
        return;
    };

    let outcome = match Box::pin(dispatch_template(
        state.clone(),
        template.clone(),
        signal.clone(),
        dispatcher,
        repo_ctx.as_ref(),
    ))
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            warn!(error = %err, template = %template_name, "agent dispatch failed");
            let fallback_output = signal
                .derive(
                    Kind::AgentOutput,
                    Body::text(format!("dispatch failed: {err}")),
                )
                .provenance(Provenance::trusted("roko-serve"))
                .tag("agent", "claude")
                .tag("failed", "true")
                .build();
            DispatchOutcome {
                result: AgentResult::fail(fallback_output),
                gate_verdicts: Vec::new(),
                success: false,
                model_used: template.model.clone(),
            }
        }
    };
    let completed_at = Utc::now();

    append_dispatch_episode(
        &state,
        &template,
        &signal,
        &outcome,
        started_at,
        completed_at,
        started.elapsed().as_secs_f64(),
        repo_ctx.as_ref(),
    )
    .await;
}

async fn dispatch_template(
    state: Arc<AppState>,
    template: AgentTemplate,
    signal: Engram,
    dispatcher: Arc<dyn AgentDispatcher>,
    repo_ctx: Option<&RepoContext>,
) -> Result<DispatchOutcome> {
    let dispatch_started = Instant::now();
    let dispatch_signal = build_dispatch_signal(&template, &signal)?;
    let mut routed_template = template.clone();
    run_anomaly_preflight(
        &state,
        &template,
        &dispatch_signal,
        repo_ctx,
        &mut routed_template.model,
    )
    .await?;

    // When a repo context is available, create a repo-aware dispatcher
    // so the agent runs in the repo's directory with cross-repo context.
    let effective_dispatcher: Arc<dyn AgentDispatcher> = if let Some(ctx) = repo_ctx {
        let repo_listing = state.runtime.list_repos();
        let roko_config = match ctx.repo_config.clone() {
            Some(config) => config,
            None => state.load_roko_config().as_ref().clone(),
        };
        let mut repo_dispatcher =
            TemplateAgentDispatcher::new(state.workdir.clone(), None, roko_config);
        repo_dispatcher.repo_workdir = Some(ctx.repo_workdir.clone());
        repo_dispatcher.repo_listing = repo_listing;
        Arc::new(repo_dispatcher)
    } else {
        Arc::clone(&dispatcher)
    };

    let dispatch_result = match effective_dispatcher
        .dispatch(routed_template.clone(), dispatch_signal.clone())
        .await
    {
        Ok(result) => result,
        Err(err) => {
            warn!(error = %err, template = %template.name, "agent backend failed");
            let failure_output = dispatch_signal
                .derive(
                    Kind::AgentOutput,
                    Body::text(format!("dispatch failed: {err}")),
                )
                .provenance(Provenance::trusted("roko-serve"))
                .tag("agent", "claude")
                .tag("failed", "true")
                .build();
            let result = AgentResult::fail(failure_output);
            record_template_run(&state, &template.name, false).await;
            state.event_bus.publish(ServerEvent::OperationCompleted {
                op_id: format!("{}:{}", template.name, signal.id.to_hex()),
                kind: format!("template_dispatch:{}", template.name),
                success: false,
            });
            return Ok(DispatchOutcome {
                result,
                gate_verdicts: Vec::new(),
                success: false,
                model_used: routed_template.model.clone(),
            });
        }
    };
    let output = dispatch_result.output.clone();
    let output_text = signal_body_to_text(&output.body);
    let agent_name = output
        .tag("agent")
        .map_or_else(|| "claude".to_string(), ToString::to_string);

    state.event_bus.publish(ServerEvent::AgentOutput {
        agent_id: agent_name.clone(),
        run_id: None,
        content: output_text.clone(),
        done: true,
        metadata: None,
    });

    let mut gate_verdicts = Vec::new();
    let gate_outputs = run_template_gates(&state, &template, &output).await;
    for verdict in gate_outputs {
        gate_verdicts.push(GateVerdict::new(verdict.gate.clone(), verdict.passed));
    }

    let success = dispatch_result.success && gate_verdicts.iter().all(|verdict| verdict.passed);
    let model_used = output
        .tag("model")
        .map(str::to_string)
        .unwrap_or_else(|| routed_template.model.clone());
    let session_root = repo_ctx
        .map(|ctx| ctx.repo_workdir.as_path())
        .unwrap_or_else(|| state.workdir.as_path());
    let budget_limit = {
        let roko_config = state.load_roko_config();
        f64::from(roko_config.budget.max_plan_usd)
    };
    record_post_turn_anomalies(
        session_root,
        &model_used,
        f64::from(dispatch_result.usage.cost_usd),
        if success { 1.0 } else { 0.0 },
        budget_limit,
    );
    record_template_run(&state, &template.name, success).await;

    let completion_kind = format!("template_dispatch:{}", template.name);
    state.event_bus.publish(ServerEvent::OperationCompleted {
        op_id: format!("{}:{}", template.name, signal.id.to_hex()),
        kind: completion_kind,
        success,
    });

    info!(
        template = %template.name,
        success,
        elapsed_ms = dispatch_started.elapsed().as_millis(),
        "template dispatch completed"
    );

    Ok(DispatchOutcome {
        result: dispatch_result,
        gate_verdicts,
        success,
        model_used,
    })
}

fn build_agent(
    roko_config: &RokoConfig,
    template: &AgentTemplate,
    system_prompt: &str,
    allowed_tools: &str,
    mcp_config: Option<&PathBuf>,
) -> Result<Box<dyn Agent>> {
    create_agent_for_model(
        roko_config,
        &template.model,
        AgentOptions {
            command: roko_config.agent.command.clone(),
            timeout_ms: None,
            system_prompt: Some(system_prompt.to_string()),
            cached_content: None,
            tools: Some(allowed_tools.to_string()),
            mcp_config: mcp_config.cloned(),
            working_dir: None,
            provider_semaphores: None,
            env: Vec::new(),
            extra_args: Vec::new(),
            effort: None,
            bare_mode: roko_config.agent.bare_mode,
            dangerously_skip_permissions: true,
            name: String::new(),
        },
    )
    .with_context(|| format!("create agent for template '{}'", template.name))
}

fn build_template_system_prompt(
    template: &AgentTemplate,
    signal: Option<&Engram>,
    experiment_variant: Option<&str>,
) -> String {
    let role_prompt = match signal {
        Some(signal) => {
            TemplateRegistry::render_prompt_with_signal(template, &HashMap::new(), Some(signal))
        }
        None => TemplateRegistry::render_prompt(template, &HashMap::new()),
    };
    let mut prompt = role_prompt;
    if let Some(variant) = experiment_variant
        && !variant.trim().is_empty()
    {
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str("## Experiment Variant\n\n");
        prompt.push_str(variant);
    }
    if let Some(format_instructions) = output_format_instructions(&template.output_format) {
        if !format_instructions.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(&format_instructions);
        }
    }
    SystemPromptBuilder::new(prompt).build()
}

/// Build a cross-repo context section for the system prompt.
///
/// Lists all known repositories so the agent can reference files across
/// repos. This is a stub — a future iteration will inject richer context
/// (recent commits, open PRs, etc.).
fn build_cross_repo_context(repos: &[RepoInfo]) -> String {
    if repos.is_empty() {
        return String::new();
    }
    let mut section = String::from("\n\n## Available Repositories\n\n");
    for repo in repos {
        section.push_str(&format!(
            "- **{}**: `{}` (branch: {})\n",
            repo.name,
            repo.path.display(),
            repo.branch,
        ));
    }
    section
}

fn output_format_instructions(format: &crate::templates::TemplateOutputFormat) -> Option<String> {
    match format {
        crate::templates::TemplateOutputFormat::Markdown => Some(
            "Output a valid Markdown document. Use prose, headings, lists, and fenced code blocks when helpful."
                .to_string(),
        ),
        crate::templates::TemplateOutputFormat::Json => Some(
            "Output valid JSON only. Do not wrap the response in markdown fences or extra prose."
                .to_string(),
        ),
        crate::templates::TemplateOutputFormat::Toml => Some(
            "Output valid TOML only. Do not wrap the response in markdown fences or extra prose."
                .to_string(),
        ),
        crate::templates::TemplateOutputFormat::None => None,
    }
}

fn build_allowed_tools_csv(template: &AgentTemplate) -> String {
    let mut names = if template.allowed_tools.is_empty() {
        default_allowed_tools_for_role(&template.role)
    } else {
        template.allowed_tools.clone()
    };

    if !template.denied_tools.is_empty() {
        let denied: std::collections::HashSet<&str> =
            template.denied_tools.iter().map(String::as_str).collect();
        names.retain(|name| !denied.contains(name.as_str()));
    }

    names.dedup();
    names.join(",")
}

fn load_template_experiment_variant(
    workdir: &Path,
    experiment_name: &str,
) -> Option<(String, String)> {
    let path = workdir.join(".roko").join("learn").join("experiments.json");
    let store = ExperimentStore::load_or_new(&path);
    store.assign_variant(experiment_name)
}

fn default_allowed_tools_for_role(role_name: &str) -> Vec<String> {
    let Some(role) = parse_agent_role(role_name) else {
        return Vec::new();
    };

    let registry = StaticToolRegistry::new();
    role_allowlist(role, registry.all())
        .into_iter()
        .map(|tool| tool.name.clone())
        .collect()
}

fn parse_agent_role(role_name: &str) -> Option<AgentRole> {
    let role_name = role_name.trim();
    if role_name.is_empty() {
        return None;
    }

    std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS.iter().copied())
        .find(|role| role.label().eq_ignore_ascii_case(role_name))
}

fn resolve_template_mcp_config(
    base_mcp_config: Option<&PathBuf>,
    workdir: &Path,
    template: &AgentTemplate,
) -> Result<Option<PathBuf>> {
    if template.mcp_servers.is_empty() {
        return Ok(base_mcp_config.cloned().or_else(|| {
            find_mcp_config(workdir).and_then(|result| match result {
                Ok((path, _)) => Some(path),
                Err(err) => {
                    warn!(error = %err, "failed to discover MCP config for template");
                    None
                }
            })
        }));
    }

    let discovered = if let Some(path) = base_mcp_config {
        Some(path.clone())
    } else {
        match find_mcp_config(workdir) {
            Some(Ok((path, _))) => Some(path),
            Some(Err(err)) => return Err(err.into()),
            None => None,
        }
    };

    let Some(base_path) = discovered else {
        anyhow::bail!(
            "template '{}' requires MCP servers {:?}, but no MCP config was found",
            template.name,
            template.mcp_servers
        );
    };

    let base_config = McpConfig::load(&base_path)
        .with_context(|| format!("load MCP config {}", base_path.display()))?;
    let requested: std::collections::HashSet<&str> =
        template.mcp_servers.iter().map(String::as_str).collect();
    let servers: Vec<McpServerConfig> = base_config
        .servers
        .into_iter()
        .filter(|server| requested.contains(server.name.as_str()))
        .collect();

    if servers.len() != requested.len() {
        let mut missing: Vec<String> = requested
            .iter()
            .copied()
            .filter(|name| !servers.iter().any(|server| server.name == *name))
            .map(str::to_string)
            .collect();
        missing.sort();
        anyhow::bail!(
            "template '{}' requires MCP servers that are missing from {}: {}",
            template.name,
            base_path.display(),
            missing.join(", ")
        );
    }

    let generated = McpConfig { servers };
    let dir = std::env::temp_dir().join("roko-template-mcp");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!("{}-{}.mcp.json", template.name, Uuid::new_v4()));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&generated).context("serialize template MCP config")?,
    )
    .with_context(|| format!("write {}", path.display()))?;
    Ok(Some(path))
}

fn dispatch_context(template: &AgentTemplate, signal: &Engram) -> RokoContext {
    let mut ctx = RokoContext::now()
        .with_attr("template", template.name.clone())
        .with_attr("signal.id", signal.id.to_hex())
        .with_attr("signal.kind", signal.kind.as_str().to_string())
        .with_attr("signal.provenance", signal.provenance.author.clone());
    if let Some(session) = signal.provenance.session.as_deref() {
        ctx = ctx.with_attr("signal.session", session.to_string());
    }
    ctx.with_attr(
        "signal.payload",
        serde_json::to_string(&signal.body).unwrap_or_else(|_| signal.body.kind_hint().into()),
    )
}

fn build_dispatch_signal(template: &AgentTemplate, signal: &Engram) -> Result<Engram> {
    let mut context = serde_json::Map::new();
    context.insert("signal".into(), serde_json::to_value(signal)?);
    context.insert("template".into(), serde_json::to_value(template)?);

    let mut body = String::new();
    body.push_str("Engram context:\n");
    body.push_str(&serde_json::to_string_pretty(&context)?);

    Ok(Engram::builder(Kind::Prompt)
        .body(Body::text(body))
        .provenance(Provenance::trusted("roko-serve"))
        .lineage([signal.id])
        .tag("template", &template.name)
        .tag("signal_kind", signal.kind.as_str())
        .build())
}

fn signal_body_to_text(body: &Body) -> String {
    match body {
        Body::Text(text) => text.clone(),
        Body::Json(value) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        Body::Bytes(bytes) => format!("<{} bytes>", bytes.len()),
        Body::Empty => String::new(),
    }
}

async fn run_template_gates(
    _state: &Arc<AppState>,
    _template: &AgentTemplate,
    output: &Engram,
) -> Vec<Verdict> {
    let _ = output;
    Vec::new()
}

async fn record_template_run(state: &Arc<AppState>, template_name: &str, success: bool) {
    state
        .template_runs
        .write()
        .await
        .entry(template_name.to_string())
        .or_default()
        .push(TemplateRunRecord {
            timestamp: chrono::Utc::now(),
            trigger_kind: "webhook_dispatch".into(),
            success,
        });
}

async fn append_dispatch_episode(
    state: &Arc<AppState>,
    template: &AgentTemplate,
    signal: &Engram,
    outcome: &DispatchOutcome,
    started_at: chrono::DateTime<Utc>,
    completed_at: chrono::DateTime<Utc>,
    duration_secs: f64,
    repo_ctx: Option<&RepoContext>,
) {
    let agent_id = outcome
        .result
        .output
        .tag("agent")
        .map_or_else(|| "claude".to_string(), ToString::to_string);
    let mut recording_template = template.clone();
    recording_template.model = outcome.model_used.clone();
    let episode_id = Uuid::new_v4().to_string();
    let turns = 1_u64;
    let tokens_used = u64::from(outcome.result.usage.total_tokens());
    let mut episode = Episode::new(agent_id, signal.id.to_hex());
    episode.kind = "agent_turn".into();
    episode.timestamp = completed_at;
    episode.id = episode_id.clone();
    episode.episode_id = episode_id;
    episode.agent_template = template.name.clone();
    episode.model = recording_template.model.clone();
    episode.trigger_kind = signal.kind.as_str().to_string();
    episode.trigger_signal_hash = signal.id.to_hex();
    episode.started_at = started_at;
    episode.completed_at = completed_at;
    episode.duration_secs = duration_secs;
    episode.input_signal_hash = signal.id.to_hex();
    episode.output_signal_hash = outcome.result.output.id.to_hex();
    episode.gate_verdicts = outcome.gate_verdicts.clone();
    episode.success = outcome.success;
    episode.turns = turns;
    episode.tokens_used = tokens_used;
    episode.external_actions = Vec::new();
    let webhook_metadata = WebhookEpisodeMetadata::new(
        episode.trigger_kind.clone(),
        episode.trigger_signal_hash.clone(),
        signal.provenance.author.clone(),
        episode.agent_template.clone(),
        outcome
            .result
            .output
            .tag("experiment_variant")
            .map(ToString::to_string),
        Vec::new(),
    );

    if let Some(variant) = webhook_metadata.experiment_variant.as_deref() {
        episode.extra.insert(
            "experiment_variant".into(),
            Value::String(variant.to_string()),
        );
    }
    if let Some(variant_id) = outcome.result.output.tag("experiment_variant_id") {
        episode.extra.insert(
            "experiment_variant_id".into(),
            Value::String(variant_id.to_string()),
        );
    }
    if !outcome.success {
        episode.failure_reason = Some("agent dispatch or template gate failure".into());
    }
    episode.usage = EpisodeUsage {
        input_tokens: outcome.result.usage.input_tokens.into(),
        output_tokens: outcome.result.usage.output_tokens.into(),
        cache_read_tokens: outcome.result.usage.cache_read_tokens.into(),
        cache_write_tokens: outcome.result.usage.cache_create_tokens.into(),
        cost_usd: f64::from(outcome.result.usage.cost_usd),
        cost_usd_without_cache: f64::from(outcome.result.usage.cost_usd),
        wall_ms: outcome.result.usage.wall_ms,
    };
    // Tag the episode with the repo name when dispatched in a per-repo context.
    if let Some(ctx) = repo_ctx {
        episode
            .extra
            .insert("repo".into(), Value::String(ctx.name.clone()));
    }

    episode.attach_all_fingerprints();
    apply_affect_signature(state, &mut episode);

    // Use per-repo layout for data isolation when a repo context is available.
    // This writes episodes, efficiency, and cascade router data under
    // .roko/repos/{repo_name}/ instead of the global .roko/.
    let repo_layout = repo_ctx.map(|ctx| &ctx.layout);
    let episodes_path = repo_layout
        .map(RokoLayout::episodes_path)
        .unwrap_or_else(|| state.layout.episodes_path());

    // Ensure parent directories exist for per-repo paths.
    if let Some(parent) = episodes_path.parent() {
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            warn!(error = %err, path = %parent.display(), "failed to create per-repo episodes dir");
        }
    }

    let logger = EpisodeLogger::new(episodes_path);
    if let Err(err) = logger.append(&episode).await {
        warn!(error = %err, template = %template.name, "failed to append episode");
        return;
    }

    let distill_workdir = repo_ctx
        .map(|ctx| ctx.repo_workdir.clone())
        .unwrap_or_else(|| state.workdir.clone());
    let distillation_caller: Arc<dyn roko_core::foundation::ModelCaller> =
        state.model_call_service.clone();
    spawn_episode_distillation(distill_workdir, episode.clone(), Some(distillation_caller));

    if let Err(err) = publish_dispatch_learning_feedback(
        state,
        &recording_template,
        signal,
        repo_layout,
        &template.name,
        turns,
        tokens_used,
        outcome,
    )
    .await
    {
        warn!(error = %err, template = %template.name, "failed to apply learning events");
    }
}

async fn publish_dispatch_learning_feedback(
    state: &Arc<AppState>,
    template: &AgentTemplate,
    signal: &Engram,
    repo_layout: Option<&RokoLayout>,
    template_name: &str,
    turns: u64,
    tokens_used: u64,
    outcome: &DispatchOutcome,
) -> Result<()> {
    let event_bus = LearningEventBus::new(16);
    let mut rx = event_bus.subscribe();
    let provider = outcome
        .result
        .output
        .tag("agent")
        .map_or_else(|| "roko-serve".to_string(), ToString::to_string);

    event_bus.publish(AgentEvent::TurnStarted {
        task_id: signal.id.to_hex(),
        model: template.model.clone(),
        provider,
        timestamp_ms: signal.created_at_ms,
    });
    event_bus.publish(AgentEvent::TurnCompleted {
        turn: turns.min(u64::from(u32::MAX)) as u32,
        usage: outcome.result.usage,
        tool_call_count: 0,
        gate_passed: Some(outcome.success),
        finish_reason: if outcome.success {
            FinishReason::Stop
        } else {
            FinishReason::Error("dispatch failed".to_string())
        },
    });

    drain_dispatch_learning_events(
        &mut rx,
        state,
        template,
        repo_layout,
        template_name,
        turns,
        tokens_used,
        outcome,
    )
    .await
}

async fn drain_dispatch_learning_events(
    rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>,
    state: &Arc<AppState>,
    template: &AgentTemplate,
    repo_layout: Option<&RokoLayout>,
    template_name: &str,
    turns: u64,
    tokens_used: u64,
    outcome: &DispatchOutcome,
) -> Result<()> {
    loop {
        match rx.try_recv() {
            Ok(AgentEvent::TurnCompleted { .. }) => {
                record_cascade_router_outcome_with_layout(
                    state,
                    template,
                    outcome.result.success,
                    repo_layout,
                )
                .await?;

                let efficiency = match repo_layout {
                    Some(layout) => EfficiencyTracker::for_layout(layout),
                    None => EfficiencyTracker::new(&state.workdir),
                };
                efficiency
                    .record_event(template_name, turns, tokens_used, outcome.success)
                    .await?;
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                warn!(
                    skipped,
                    "dispatch learning feedback lagged behind event stream"
                );
            }
        }
    }

    Ok(())
}

fn apply_affect_signature(state: &AppState, episode: &mut Episode) {
    let task_key = if episode.task_id.trim().is_empty() {
        episode.agent_id.clone()
    } else {
        episode.task_id.clone()
    };

    let mut engine = state.affect_engine.lock();
    for (rung, verdict) in episode.gate_verdicts.iter().enumerate() {
        let _ = engine.appraise(AffectEvent::GateResult {
            plan_id: String::new(),
            task_id: task_key.clone(),
            passed: verdict.passed,
            rung: rung as u32,
        });
    }
    if episode.success {
        let _ = engine.appraise(AffectEvent::TaskOutcome {
            task_id: task_key.clone(),
            succeeded: true,
        });
    } else {
        let _ = engine.appraise(AffectEvent::TaskOutcome {
            task_id: task_key.clone(),
            succeeded: false,
        });
    }

    let state = engine.query();
    episode.extra.insert(
        "pad".into(),
        serde_json::json!({
            "pleasure": state.pad.pleasure,
            "arousal": state.pad.arousal,
            "dominance": state.pad.dominance,
        }),
    );
}

#[allow(dead_code)]
async fn record_cascade_router_outcome(
    state: &Arc<AppState>,
    template: &AgentTemplate,
    success: bool,
) -> Result<()> {
    record_cascade_router_outcome_with_layout(state, template, success, None).await
}

async fn record_cascade_router_outcome_with_layout(
    state: &Arc<AppState>,
    template: &AgentTemplate,
    success: bool,
    repo_layout: Option<&RokoLayout>,
) -> Result<()> {
    let model_slugs = {
        let templates = state.templates.read().await;
        let mut slugs = Vec::new();
        let mut seen = HashSet::new();

        for loaded in templates.list() {
            let model = loaded.model.clone();
            if seen.insert(model.clone()) {
                slugs.push(model);
            }
        }

        let model = template.model.clone();
        if seen.insert(model.clone()) {
            slugs.push(model);
        }

        slugs
    };

    let path = repo_layout
        .map(RokoLayout::cascade_router_path)
        .unwrap_or_else(|| RokoLayout::for_project(&state.workdir).cascade_router_path());
    record_cascade_router_observation_at(&path, model_slugs, &template.model, success)?;
    Ok(())
}

pub(crate) fn record_cascade_router_observation(
    workdir: &Path,
    model_slugs: Vec<String>,
    model_slug: &str,
    success: bool,
) -> Result<bool> {
    let path = RokoLayout::for_project(workdir).cascade_router_path();
    record_cascade_router_observation_at(&path, model_slugs, model_slug, success)
}

fn record_cascade_router_observation_at(
    path: &Path,
    model_slugs: Vec<String>,
    model_slug: &str,
    success: bool,
) -> Result<bool> {
    // Ensure parent directory exists for per-repo paths.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let cascade_router = CascadeRouter::load_or_new(path, model_slugs);
    if cascade_router.record_outcome(model_slug, success) {
        cascade_router
            .save(path)
            .with_context(|| format!("save {}", path.display()))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();

    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star = None;
    let mut match_index = 0usize;

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == b'?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == b'*' {
            star = Some(pi);
            match_index = ti;
            pi += 1;
        } else if let Some(star_index) = star {
            pi = star_index + 1;
            match_index += 1;
            ti = match_index;
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }

    pi == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind, Provenance};
    use uuid::Uuid;

    #[test]
    fn glob_match_supports_prefix_patterns() {
        assert!(glob_match("github:*", "github:push"));
        assert!(glob_match("github:**", "github:pull_request:opened"));
        assert!(!glob_match("slack:*", "github:push"));
    }

    #[test]
    fn registry_finds_matching_subscription() {
        let registry = SubscriptionRegistry::with_subscriptions(vec![
            Subscription::new("reviewer", "github:*"),
            Subscription::new("ops", "slack:*").disabled(),
        ]);
        let signal = Engram::builder(Kind::Custom("github:push".into()))
            .body(Body::Json(serde_json::json!({"repo": "roko"})))
            .provenance(Provenance::external("github:webhook"))
            .build();

        let matched = registry.find_matching(&signal);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].template(), "reviewer");
    }

    #[test]
    fn registry_applies_repo_branch_path_label_and_author_filters() {
        let registry = SubscriptionRegistry::with_subscriptions(vec![
            Subscription::new("repo", "github:**").with_filter(SubscriptionFilterConfig {
                repo: vec!["roko/*".into()],
                ..SubscriptionFilterConfig::default()
            }),
            Subscription::new("branch", "github:**").with_filter(SubscriptionFilterConfig {
                branch: vec!["main".into()],
                ..SubscriptionFilterConfig::default()
            }),
            Subscription::new("paths", "github:**").with_filter(SubscriptionFilterConfig {
                path: vec!["src/*.rs".into()],
                ..SubscriptionFilterConfig::default()
            }),
            Subscription::new("labels", "github:**").with_filter(SubscriptionFilterConfig {
                label: vec!["bug".into()],
                ..SubscriptionFilterConfig::default()
            }),
            Subscription::new("authors", "github:**").with_filter(SubscriptionFilterConfig {
                author: vec!["octocat".into()],
                ..SubscriptionFilterConfig::default()
            }),
        ]);

        let signal = Engram::builder(Kind::Custom("github:push".into()))
            .body(Body::Json(serde_json::json!({
                "repository": { "full_name": "roko/roko" },
                "ref": "refs/heads/main",
                "head_commit": {
                    "modified": ["src/lib.rs", "README.md"],
                    "author": { "username": "octocat" }
                },
                "labels": [{ "name": "bug" }]
            })))
            .provenance(Provenance::external("github:webhook"))
            .build();

        let matched = registry.find_matching(&signal);
        let templates = matched.iter().map(|sub| sub.template()).collect::<Vec<_>>();

        assert!(templates.contains(&"repo"));
        assert!(templates.contains(&"branch"));
        assert!(templates.contains(&"paths"));
        assert!(templates.contains(&"labels"));
        assert!(templates.contains(&"authors"));
    }

    #[test]
    fn dedup_blocks_repeat_signals_within_window() {
        let sub = Subscription::new("reviewer", "github:*").with_dedup_ttl(Duration::from_secs(60));
        let signal = Engram::builder(Kind::Custom("github:push".into()))
            .body(Body::Json(serde_json::json!({"repo": "roko"})))
            .provenance(Provenance::external("github:webhook"))
            .build();

        assert!(sub.check_dedup(&signal));
        assert!(!sub.check_dedup(&signal));
    }

    #[test]
    fn cooldown_blocks_repeat_dispatches_within_window() {
        let registry = SubscriptionRegistry::with_subscriptions(vec![
            Subscription::new("reviewer", "github:*").with_cooldown(Duration::from_secs(60)),
        ]);
        let signal = Engram::builder(Kind::Custom("github:push".into()))
            .body(Body::Json(serde_json::json!({"repo": "roko"})))
            .provenance(Provenance::external("github:webhook"))
            .build();

        let matched = registry.find_matching(&signal);
        let sub = matched.first().expect("subscription");

        assert!(registry.check_cooldown(sub));
        assert!(!registry.check_cooldown(sub));
    }

    #[test]
    fn concurrency_limit_is_tracked_per_subscription() {
        let registry = SubscriptionRegistry::with_subscriptions(vec![
            Subscription::new("reviewer", "github:*").with_concurrency_limit(1),
            Subscription::new("ops", "slack:*").with_concurrency_limit(2),
        ]);

        let signal = Engram::builder(Kind::Custom("github:push".into()))
            .body(Body::Json(serde_json::json!({"repo": "roko"})))
            .provenance(Provenance::external("github:webhook"))
            .build();

        let matched = registry.find_matching(&signal);
        let reviewer = matched
            .iter()
            .find(|sub| sub.template() == "reviewer")
            .cloned()
            .expect("reviewer subscription");

        assert!(registry.check_concurrency_limit(&reviewer));
        assert!(!registry.check_concurrency_limit(&reviewer));

        registry.release_concurrency(&reviewer);
        assert!(registry.check_concurrency_limit(&reviewer));
    }

    #[test]
    fn registry_loads_inline_and_file_subscriptions() {
        let workdir = std::env::temp_dir().join(format!("roko-subscriptions-{}", Uuid::new_v4()));
        let subscriptions_dir = workdir.join(".roko").join("subscriptions");
        std::fs::create_dir_all(&subscriptions_dir).expect("create subscriptions dir");

        let roko_toml = r#"
[[subscriptions]]
template = "inline-review"
trigger = "github:*"
filter = { repo = "roko/*", branch = "refs/heads/main" }
concurrency_limit = 2
cooldown_secs = 30
"#;
        std::fs::write(workdir.join("roko.toml"), roko_toml).expect("write roko.toml");

        let file_toml = r#"
template = "path-review"
trigger = "github:push"
concurrency_limit = 1
cooldown_secs = 10
filter = { path = "src/*.rs" }
"#;
        std::fs::write(subscriptions_dir.join("path-review.toml"), file_toml)
            .expect("write subscription file");

        let config = RokoConfig::from_toml(roko_toml).expect("parse roko.toml");
        let registry = SubscriptionRegistry::load_from_project(&workdir, &config);

        let signal = Engram::builder(Kind::Custom("github:push".into()))
            .body(Body::Json(serde_json::json!({
                "repository": { "full_name": "roko/roko" },
                "ref": "refs/heads/main",
                "head_commit": { "modified": ["src/lib.rs", "README.md"] }
            })))
            .provenance(Provenance::external("github:webhook"))
            .build();

        let matched = registry.find_matching(&signal);
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].template(), "inline-review");
        assert_eq!(matched[1].template(), "path-review");

        let _ = std::fs::remove_dir_all(&workdir);
    }

    #[test]
    fn template_system_prompt_includes_output_format_guidance() {
        let template = AgentTemplate {
            name: "json-template".into(),
            description: "Test template".into(),
            model: "claude-test".into(),
            role: "implementer".into(),
            system_prompt: "You are the template role.".into(),
            max_turns: 4,
            output_format: crate::templates::TemplateOutputFormat::Json,
            mcp_servers: Vec::new(),
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            experiment: None,
            provider: None,
        };

        let prompt = build_template_system_prompt(&template, None, None);
        assert!(prompt.contains("You are the template role."));
        assert!(prompt.contains("Output valid JSON only"));
    }

    #[test]
    fn allowed_tools_csv_respects_explicit_allowlist_and_denylist() {
        let template = AgentTemplate {
            name: "tools-template".into(),
            description: "Test template".into(),
            model: "claude-test".into(),
            role: "implementer".into(),
            system_prompt: "You are the template role.".into(),
            max_turns: 4,
            output_format: crate::templates::TemplateOutputFormat::Markdown,
            mcp_servers: Vec::new(),
            allowed_tools: vec!["read_file".into(), "grep".into(), "bash".into()],
            denied_tools: vec!["grep".into()],
            experiment: None,
            provider: None,
        };

        let tools_csv = build_allowed_tools_csv(&template);
        assert_eq!(tools_csv, "read_file,bash");
    }

    #[test]
    fn template_mcp_config_filters_requested_servers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mcp_path = tmp.path().join(".mcp.json");
        let config = McpConfig {
            servers: vec![
                McpServerConfig {
                    name: "filesystem".into(),
                    command: "npx".into(),
                    args: vec![
                        "-y".into(),
                        "@modelcontextprotocol/server-filesystem".into(),
                    ],
                    ..Default::default()
                },
                McpServerConfig {
                    name: "git".into(),
                    command: "mcp-git".into(),
                    ..Default::default()
                },
            ],
        };
        std::fs::write(
            &mcp_path,
            serde_json::to_string_pretty(&config).expect("serialize"),
        )
        .expect("write mcp config");

        let template = AgentTemplate {
            name: "mcp-template".into(),
            description: "Test template".into(),
            model: "claude-test".into(),
            role: "implementer".into(),
            system_prompt: "You are the template role.".into(),
            max_turns: 4,
            output_format: crate::templates::TemplateOutputFormat::Markdown,
            mcp_servers: vec!["filesystem".into()],
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            experiment: None,
            provider: None,
        };

        let generated = resolve_template_mcp_config(None, tmp.path(), &template).expect("resolve");
        let generated = generated.expect("generated path");
        let rendered = std::fs::read_to_string(&generated).expect("read generated config");
        let parsed: McpConfig = serde_json::from_str(&rendered).expect("parse generated config");
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers[0].name, "filesystem");
    }

    #[test]
    fn signal_repo_full_name_extracts_from_github_payload() {
        let signal = Engram::builder(Kind::Custom("github:push".into()))
            .body(Body::Json(serde_json::json!({
                "repository": { "full_name": "nunchi/roko", "name": "roko" }
            })))
            .provenance(Provenance::external("github:webhook"))
            .build();

        let full_name = signal_repo_full_name(&signal);
        assert_eq!(full_name.as_deref(), Some("nunchi/roko"));
    }

    #[test]
    fn signal_repo_full_name_returns_none_for_non_repo_signals() {
        let signal = Engram::builder(Kind::Custom("cron:tick".into()))
            .body(Body::Json(serde_json::json!({ "schedule": "hourly" })))
            .provenance(Provenance::trusted("scheduler"))
            .build();

        assert!(signal_repo_full_name(&signal).is_none());
    }

    #[test]
    fn repo_context_layout_isolates_per_repo_data() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let layout = RokoLayout::for_repo(tmp.path(), "my-repo");

        // Per-repo paths should be under .roko/repos/my-repo/
        let episodes = layout.episodes_path();
        assert!(
            episodes.to_str().unwrap().contains("repos/my-repo"),
            "episodes path should be under repos/my-repo: {episodes:?}"
        );

        let efficiency = layout.efficiency_path();
        assert!(
            efficiency.to_str().unwrap().contains("repos/my-repo"),
            "efficiency path should be under repos/my-repo: {efficiency:?}"
        );

        let cascade_router = layout.cascade_router_path();
        assert!(
            cascade_router.to_str().unwrap().contains("repos/my-repo"),
            "cascade router path should be under repos/my-repo: {cascade_router:?}"
        );
    }

    #[test]
    fn efficiency_tracker_for_layout_uses_repo_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let layout = RokoLayout::for_repo(tmp.path(), "test-repo");
        let tracker = EfficiencyTracker::for_layout(&layout);

        assert!(
            tracker.path.to_str().unwrap().contains("repos/test-repo"),
            "tracker path should be under repos/test-repo: {:?}",
            tracker.path
        );
        assert!(
            tracker.path.to_str().unwrap().ends_with("efficiency.jsonl"),
            "tracker path should end with efficiency.jsonl: {:?}",
            tracker.path
        );
    }

    #[test]
    fn cross_repo_context_is_empty_for_no_repos() {
        let repos: Vec<RepoInfo> = Vec::new();
        assert!(build_cross_repo_context(&repos).is_empty());
    }

    #[test]
    fn cross_repo_context_lists_all_configured_repos() {
        let repos = vec![
            RepoInfo {
                name: "roko".into(),
                path: PathBuf::from("/repos/roko"),
                branch: "main".into(),
            },
            RepoInfo {
                name: "collab".into(),
                path: PathBuf::from("/repos/collab"),
                branch: "develop".into(),
            },
        ];
        let section = build_cross_repo_context(&repos);
        assert!(section.contains("roko"));
        assert!(section.contains("collab"));
        assert!(section.contains("/repos/roko"));
        assert!(section.contains("/repos/collab"));
        assert!(section.contains("main"));
        assert!(section.contains("develop"));
        assert!(section.contains("## Available Repositories"));
    }

    #[test]
    fn template_dispatcher_uses_repo_workdir_over_global() {
        let global_dir = tempfile::tempdir().expect("global tempdir");
        let repo_dir = tempfile::tempdir().expect("repo tempdir");

        let mut dispatcher = TemplateAgentDispatcher::new(
            global_dir.path().to_path_buf(),
            None,
            RokoConfig::default(),
        );
        dispatcher.repo_workdir = Some(repo_dir.path().to_path_buf());

        // Verify the effective workdir resolves to repo_workdir.
        let effective = dispatcher
            .repo_workdir
            .as_deref()
            .unwrap_or(global_dir.path());
        assert_eq!(effective, repo_dir.path());
    }

    #[test]
    fn anomaly_dispatch_prompt_loop_halts_after_five_identical_prompts() {
        let session_root = tempfile::tempdir().expect("session tempdir");
        let signal = Engram::builder(Kind::Prompt)
            .body(Body::text("looping prompt"))
            .provenance(Provenance::trusted("test"))
            .build();
        let prompt_hash = prompt_hash_u64(&signal);

        for _ in 0..4 {
            let anomaly = with_dispatch_anomaly_session(session_root.path(), |session| {
                session.detector.check_prompt(prompt_hash)
            });
            assert!(anomaly.is_none());
        }

        let anomaly = with_dispatch_anomaly_session(session_root.path(), |session| {
            session.detector.check_prompt(prompt_hash)
        });

        assert!(matches!(
            anomaly,
            Some(Anomaly::PromptLoop { repeated_count: 5 })
        ));
    }

    #[test]
    fn anomaly_dispatch_model_downgrade_prefers_configured_fallback() {
        let mut config = RokoConfig::default();
        config.agent.fallback_model = Some("claude-haiku-3-5".to_string());

        let downgraded = downgrade_model_slug("claude-opus-4", &config);
        assert_eq!(downgraded.as_deref(), Some("claude-haiku-3-5"));
    }
}
