//! Event-driven dispatch loop for webhook signals.
//!
//! The loop listens on the shared server event bus, extracts webhook
//! signals, resolves matching subscriptions, and spawns agent dispatches
//! while enforcing per-subscription concurrency, cooldown, and dedup
//! constraints.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use anyhow::{Context as _, Result};
use chrono::Utc;
use parking_lot::{Mutex, RwLock};
use roko_agent::{Agent, AgentResult, ClaudeCliAgent, ExecAgent};
use roko_core::{Body, Context as RokoContext, Gate, Kind, Provenance, Signal};
use roko_core::config::schema::{RokoConfig, SubscriptionConfig, SubscriptionFilterConfig};
use roko_core::{ContentHash, Verdict};
use roko_gate::{ClippyGate, CompileGate, DiffGate, DiffPayload, GatePayload, TestGate};
use roko_learn::episode_logger::{Episode, EpisodeLogger, GateVerdict, Usage as EpisodeUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use crate::events::ServerEvent;
use crate::state::{AppState, TemplateRunRecord};
use crate::templates::{AgentTemplate, TemplateRegistry};
use roko_cli::dispatch::{
    Subscription as SharedSubscription,
    SubscriptionRegistry as SharedSubscriptionRegistry,
};

/// Async agent-dispatch interface used by the routing loop.
#[async_trait]
pub trait AgentDispatcher: Send + Sync {
    /// Dispatch a signal through the agent template identified by `template`.
    async fn dispatch(&self, template: AgentTemplate, signal: Signal) -> Result<AgentResult>;
}

/// Record of an external side-effect performed by an event-driven agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalAction {
    /// External service that received the action.
    #[serde(default)]
    pub service: String,
    /// Service-specific action name, such as `review_pr` or `post_message`.
    #[serde(default)]
    pub action_type: String,
    /// Resource identifier the action targeted.
    #[serde(default)]
    pub resource_id: String,
    /// Additional structured metadata for the action.
    #[serde(default)]
    pub metadata: Value,
    /// Time when the action was performed.
    #[serde(default = "Utc::now")]
    pub performed_at: chrono::DateTime<Utc>,
}

/// Extended episode metadata for webhook- and event-driven agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebhookEpisodeMetadata {
    /// Signal kind that triggered the agent.
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

/// Template-backed agent runner used by the webhook dispatch loop.
#[derive(Clone, Debug)]
pub struct TemplateAgentDispatcher {
    workdir: PathBuf,
}

impl TemplateAgentDispatcher {
    /// Create a dispatcher rooted at `workdir`.
    #[must_use]
    pub fn new(workdir: PathBuf) -> Self {
        Self { workdir }
    }
}

#[async_trait]
impl AgentDispatcher for TemplateAgentDispatcher {
    async fn dispatch(&self, template: AgentTemplate, signal: Signal) -> Result<AgentResult> {
        let system_prompt = TemplateRegistry::render_prompt(&template, &HashMap::new());
        let agent = build_agent(&template, &system_prompt, &self.workdir)?;
        let ctx = dispatch_context(&template, &signal);
        Ok(agent.run(&signal, &ctx).await)
    }
}

/// A subscription from signal trigger to agent template.
#[derive(Debug)]
pub struct Subscription {
    subscription_id: usize,
    config: SubscriptionConfig,
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
            subscription_id: self.subscription_id,
            config: self.config.clone(),
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
            subscription_id: usize::MAX,
            config,
            dedup_ttl: Duration::from_secs(60),
            state: Arc::new(SubscriptionState::new()),
        }
    }

    /// Replace the filter criteria for this subscription.
    #[must_use]
    pub fn with_filter(mut self, filter: SubscriptionFilterConfig) -> Self {
        self.config.filter = filter;
        self
    }

    /// Set the maximum number of concurrent dispatches allowed.
    #[must_use]
    pub fn with_concurrency_limit(mut self, limit: usize) -> Self {
        self.config.concurrency_limit = limit;
        self
    }

    /// Set the minimum delay between dispatches.
    #[must_use]
    pub fn with_cooldown(mut self, cooldown: Duration) -> Self {
        self.config.cooldown_secs = cooldown.as_secs();
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
        self.config.enabled = false;
        self
    }

    fn with_subscription_id(mut self, subscription_id: usize) -> Self {
        self.subscription_id = subscription_id;
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
        &self.config.template
    }

    /// Trigger pattern used to match signal kinds.
    #[must_use]
    pub fn trigger(&self) -> &str {
        &self.config.trigger
    }

    /// Return the configured filter criteria.
    #[must_use]
    pub fn filter(&self) -> &SubscriptionFilterConfig {
        &self.config.filter
    }

    /// Whether the subscription is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Check whether this subscription should trigger for `signal`.
    #[must_use]
    pub fn matches(&self, signal: &Signal) -> bool {
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
    pub fn check_dedup(&self, signal: &Signal) -> bool {
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
            .map(Subscription::from_config)
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
                    Err(e) => warn!(path = %path.display(), error = %e, "failed to load subscription file"),
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

    /// Return subscriptions whose trigger and filters match `signal`.
    #[must_use]
    pub fn find_matching(&self, signal: &Signal) -> Vec<Subscription> {
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
        if subscription.config.concurrency_limit == 0 {
            return false;
        }

        let mut active_counts = self.active_counts.lock();
        let active = active_counts
            .entry(subscription.subscription_id())
            .or_insert_with(|| AtomicUsize::new(0));

        let mut current = active.load(Ordering::Acquire);
        loop {
            if current >= subscription.config.concurrency_limit {
                return false;
            }

            match active.compare_exchange(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
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
        if subscription.config.cooldown_secs == 0 {
            return true;
        }

        let cooldown = Duration::from_secs(subscription.config.cooldown_secs);
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

    if let Ok(config) = toml::from_str::<SubscriptionConfig>(&text) {
        return Ok(vec![Subscription::from_config(config)]);
    }

    let file: SubscriptionFile =
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    let mut subscriptions = Vec::new();
    subscriptions.extend(file.subscription.into_iter().map(Subscription::from_config));
    subscriptions.extend(file.subscriptions.into_iter().map(Subscription::from_config));

    if subscriptions.is_empty() {
        anyhow::bail!("no subscriptions found");
    }

    Ok(subscriptions)
}

fn subscription_filter_matches(filter: &SubscriptionFilterConfig, signal: &Signal) -> bool {
    if filter.is_empty() {
        return true;
    }

    if !filter.repo.is_empty() && !matches_any_glob(signal_repo_candidates(signal), &filter.repo) {
        return false;
    }

    if !filter.branch.is_empty()
        && !matches_any_glob(signal_branch_candidates(signal), &filter.branch)
    {
        return false;
    }

    if !filter.path.is_empty() && !matches_any_glob(signal_path_candidates(signal), &filter.path) {
        return false;
    }

    true
}

fn matches_any_glob<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
    patterns: &[String],
) -> bool {
    let candidates: Vec<&'a str> = candidates.into_iter().collect();
    patterns
        .iter()
        .any(|pattern| candidates.iter().copied().any(|candidate| glob_match(pattern, candidate)))
}

fn signal_repo_candidates(signal: &Signal) -> Vec<&str> {
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

fn signal_branch_candidates(signal: &Signal) -> Vec<&str> {
    json_string_fields(
        &signal.body,
        &[
            &["ref"],
            &["branch"],
            &["repository", "default_branch"],
            &["pull_request", "base", "ref"],
            &["pull_request", "head", "ref"],
        ],
    )
}

fn signal_path_candidates(signal: &Signal) -> Vec<&str> {
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

fn json_string_fields<'a>(body: &'a Body, paths: &[&[&str]]) -> Vec<&'a str> {
    match body {
        Body::Json(value) => paths
            .iter()
            .filter_map(|path| json_string_at(value, path))
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

/// Central event routing loop for webhook-driven signals.
pub async fn dispatch_loop(
    state: Arc<AppState>,
    dispatcher: Arc<dyn AgentDispatcher>,
) {
    let subscriptions: SharedSubscriptionRegistry = state.subscriptions.clone();
    let mut rx = state.event_bus.subscribe();

    loop {
        let envelope = match rx.recv().await {
            Ok(envelope) => envelope,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!(n, "dispatch loop lagged, skipped events");
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                warn!("dispatch event bus closed, stopping loop");
                break;
            }
        };

        let ServerEvent::WebhookReceived { signal } = envelope.payload else {
            continue;
        };

        let matched = subscriptions.find_matching(&signal);
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
                tokio::spawn(async move {
                    dispatch_agent(state, sub_for_task.clone(), signal, dispatcher).await;
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
    subscription: SharedSubscription,
    signal: Signal,
    dispatcher: Arc<dyn AgentDispatcher>,
) {
    let template_name = subscription.template().to_owned();
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

    if let Err(err) = dispatch_template(state, template, signal, dispatcher).await {
        warn!(error = %err, template = %template_name, "agent dispatch failed");
    }
}

async fn dispatch_template(
    state: Arc<AppState>,
    template: AgentTemplate,
    signal: Signal,
    dispatcher: Arc<dyn AgentDispatcher>,
) -> Result<()> {
    let dispatch_started = Instant::now();
    let dispatch_signal = build_dispatch_signal(&template, &signal)?;
    let dispatch_result = match dispatcher.dispatch(template.clone(), dispatch_signal.clone()).await
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
                .tag("agent", &template.agent.command)
                .tag("failed", "true")
                .build();
            let result = AgentResult::fail(failure_output);
            record_template_run(&state, &template.name, false).await;
            append_episode(
                &state,
                &template,
                &signal,
                &result,
                &[],
                false,
            )
            .await;
            state.event_bus.publish(ServerEvent::OperationCompleted {
                op_id: format!("{}:{}", template.name, signal.id.to_hex()),
                kind: format!("template_dispatch:{}", template.name),
                success: false,
            });
            return Ok(());
        }
    };
    let output = dispatch_result.output.clone();
    let output_text = signal_body_to_text(&output.body);
    let agent_name = output
        .tag("agent")
        .map_or_else(|| template.agent.command.clone(), ToString::to_string);

    state.event_bus.publish(ServerEvent::AgentOutput {
        agent_id: agent_name.clone(),
        content: output_text.clone(),
    });

    let mut gate_verdicts = Vec::new();
    let gate_outputs = run_template_gates(&state, &template, &output).await;
    for verdict in gate_outputs {
        gate_verdicts.push(GateVerdict::new(verdict.gate.clone(), verdict.passed));
    }

    let success = dispatch_result.success && gate_verdicts.iter().all(|verdict| verdict.passed);
    record_template_run(&state, &template.name, success).await;
    append_episode(
        &state,
        &template,
        &signal,
        &dispatch_result,
        &gate_verdicts,
        success,
    )
    .await;

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

    Ok(())
}

fn build_agent(
    template: &AgentTemplate,
    system_prompt: &str,
    workdir: &Path,
) -> Result<Box<dyn Agent>> {
    if template.agent.command == "claude" {
        let agent = ClaudeCliAgent::new(
            &template.agent.command,
            workdir,
            template.agent.model.clone(),
        )
        .with_system_prompt(system_prompt.to_string())
        .with_timeout_ms(120_000);
        Ok(Box::new(agent))
    } else {
        let agent = ExecAgent::new(&template.agent.command, Vec::new())
            .with_timeout_ms(120_000)
            .with_name(format!("exec:{}", template.agent.command));
        Ok(Box::new(agent))
    }
}

fn dispatch_context(template: &AgentTemplate, signal: &Signal) -> RokoContext {
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

fn build_dispatch_signal(template: &AgentTemplate, signal: &Signal) -> Result<Signal> {
    let mut context = serde_json::Map::new();
    context.insert("signal".into(), serde_json::to_value(signal)?);
    context.insert("template".into(), serde_json::to_value(template)?);

    let mut body = String::new();
    body.push_str("Signal context:\n");
    body.push_str(&serde_json::to_string_pretty(&context)?);
    if !template.prompt.sections.is_empty() {
        body.push_str("\n\nTemplate sections:\n");
        for section in &template.prompt.sections {
            body.push_str("- ");
            body.push_str(section);
            body.push('\n');
        }
    }

    Ok(Signal::builder(Kind::Prompt)
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
        Body::Json(value) => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
        Body::Bytes(bytes) => format!("<{} bytes>", bytes.len()),
        Body::Empty => String::new(),
    }
}

async fn run_template_gates(
    state: &Arc<AppState>,
    template: &AgentTemplate,
    output: &Signal,
) -> Vec<Verdict> {
    if template.gates.names.is_empty() {
        return Vec::new();
    }

    let mut verdicts = Vec::new();
    for gate_name in &template.gates.names {
        let verdict = run_named_gate(state, gate_name, output).await;
        verdicts.push(verdict);
    }
    verdicts
}

async fn run_named_gate(
    state: &Arc<AppState>,
    gate_name: &str,
    output: &Signal,
) -> Verdict {
    let ctx = RokoContext::now().with_attr("gate", gate_name.to_string());
    let payload_signal = match gate_payload_signal(&state.workdir, gate_name, output) {
        Ok(signal) => signal,
        Err(err) => {
            warn!(error = %err, gate = gate_name, "failed to build gate payload");
            return Verdict::fail(gate_name, format!("failed to build gate payload: {err}"));
        }
    };

    match gate_name {
        "compile" => CompileGate::cargo().verify(&payload_signal, &ctx).await,
        "test" => TestGate::cargo().verify(&payload_signal, &ctx).await,
        "clippy" => ClippyGate::cargo().verify(&payload_signal, &ctx).await,
        "diff" => DiffGate::new().verify(&payload_signal, &ctx).await,
        other => {
            warn!(gate = other, "unsupported template gate");
            Verdict::fail(other, "unsupported template gate")
        }
    }
}

fn gate_payload_signal(workdir: &Path, gate_name: &str, output: &Signal) -> Result<Signal> {
    let label = format!("{gate_name}:{}", output.id.to_hex());
    if gate_name == "diff" {
        let diff = std::process::Command::new("git")
            .arg("diff")
            .current_dir(workdir)
            .output()
            .context("run git diff")?;
        let diff_text = String::from_utf8_lossy(&diff.stdout).into_owned();
        let payload = DiffPayload::new(diff_text);
        Ok(Signal::builder(Kind::Task)
            .body(Body::from_json(&payload)?)
            .provenance(Provenance::trusted("roko-serve"))
            .tag("gate", gate_name)
            .tag("label", label)
            .build())
    } else {
        let payload = GatePayload::in_dir(workdir).with_label(label);
        Ok(Signal::builder(Kind::Task)
            .body(Body::from_json(&payload)?)
            .provenance(Provenance::trusted("roko-serve"))
            .tag("gate", gate_name)
            .build())
    }
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

async fn append_episode(
    state: &Arc<AppState>,
    template: &AgentTemplate,
    signal: &Signal,
    result: &AgentResult,
    gate_verdicts: &[GateVerdict],
    success: bool,
) {
    let agent_id = result
        .output
        .tag("agent")
        .map_or_else(|| template.agent.command.clone(), ToString::to_string);
    let mut episode = Episode::new(agent_id, signal.id.to_hex());
    episode.kind = "agent_turn".into();
    episode.input_signal_hash = signal.id.to_hex();
    episode.output_signal_hash = result.output.id.to_hex();
    episode.gate_verdicts = gate_verdicts.to_vec();
    episode.success = success;
    if !success {
        episode.failure_reason = Some("agent dispatch or template gate failure".into());
    }
    episode.usage = EpisodeUsage {
        input_tokens: result.usage.input_tokens.into(),
        output_tokens: result.usage.output_tokens.into(),
        cache_read_tokens: result.usage.cache_read_tokens.into(),
        cache_write_tokens: result.usage.cache_create_tokens.into(),
        cost_usd: f64::from(result.usage.cost_usd),
        cost_usd_without_cache: f64::from(result.usage.cost_usd),
        wall_ms: result.usage.wall_ms,
    };

    let logger = EpisodeLogger::new(state.layout.episodes_path());
    if let Err(err) = logger.append(&episode).await {
        warn!(error = %err, template = %template.name, "failed to append episode");
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
        let signal = Signal::builder(Kind::Custom("github:push".into()))
            .body(Body::Json(serde_json::json!({"repo": "roko"})))
            .provenance(Provenance::external("github:webhook"))
            .build();

        let matched = registry.find_matching(&signal);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].template(), "reviewer");
    }

    #[test]
    fn dedup_blocks_repeat_signals_within_window() {
        let sub = Subscription::new("reviewer", "github:*").with_dedup_ttl(Duration::from_secs(60));
        let signal = Signal::builder(Kind::Custom("github:push".into()))
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
        let signal = Signal::builder(Kind::Custom("github:push".into()))
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

        let signal = Signal::builder(Kind::Custom("github:push".into()))
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
filter = { path = "src/**/*.rs" }
"#;
        std::fs::write(subscriptions_dir.join("path-review.toml"), file_toml)
            .expect("write subscription file");

        let config = RokoConfig::from_toml(roko_toml).expect("parse roko.toml");
        let registry = SubscriptionRegistry::load_from_project(&workdir, &config);

        let signal = Signal::builder(Kind::Custom("github:push".into()))
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
}
