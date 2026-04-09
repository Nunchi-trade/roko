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
use anyhow::Context;
use parking_lot::{Mutex, RwLock};
use roko_core::config::schema::{RokoConfig, SubscriptionConfig, SubscriptionFilterConfig};
use roko_core::{Body, ContentHash, Signal};
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::event_bus::EventBus;
use crate::events::ServerEvent;

/// Async agent-dispatch interface used by the routing loop.
#[async_trait]
pub trait AgentDispatcher: Send + Sync {
    /// Dispatch a signal through the agent template identified by `template`.
    async fn dispatch(&self, template: String, signal: Signal);
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
    event_bus: EventBus<ServerEvent>,
    subscriptions: SubscriptionRegistry,
    dispatcher: Arc<dyn AgentDispatcher>,
) {
    let mut rx = event_bus.subscribe();

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
                let subscriptions = subscriptions.clone();
                let sub_for_task = sub.clone();
                tokio::spawn(async move {
                    dispatch_agent(sub_for_task.clone(), signal, dispatcher).await;
                    sub_for_task.release_concurrency(&subscriptions);
                });
            } else {
                subscriptions.release_concurrency(&sub);
            }
        }
    }
}

async fn dispatch_agent(
    subscription: Subscription,
    signal: Signal,
    dispatcher: Arc<dyn AgentDispatcher>,
) {
    dispatcher
        .dispatch(subscription.template().to_owned(), signal)
        .await;
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
