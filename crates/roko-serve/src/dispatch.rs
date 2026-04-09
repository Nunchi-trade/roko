//! Event-driven dispatch loop for webhook signals.
//!
//! The loop listens on the shared server event bus, extracts webhook
//! signals, resolves matching subscriptions, and spawns agent dispatches
//! while enforcing per-subscription concurrency, cooldown, and dedup
//! constraints.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use roko_core::{ContentHash, Signal};
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
    template: String,
    trigger: String,
    enabled: bool,
    concurrency_limit: usize,
    cooldown: Duration,
    dedup_ttl: Duration,
    state: Arc<SubscriptionState>,
}

#[derive(Debug)]
struct SubscriptionState {
    active: AtomicUsize,
    last_triggered: Mutex<Option<Instant>>,
    recent_signals: Mutex<HashMap<ContentHash, Instant>>,
}

impl SubscriptionState {
    fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            last_triggered: Mutex::new(None),
            recent_signals: Mutex::new(HashMap::new()),
        }
    }
}

impl Clone for Subscription {
    fn clone(&self) -> Self {
        Self {
            template: self.template.clone(),
            trigger: self.trigger.clone(),
            enabled: self.enabled,
            concurrency_limit: self.concurrency_limit,
            cooldown: self.cooldown,
            dedup_ttl: self.dedup_ttl,
            state: Arc::clone(&self.state),
        }
    }
}

impl Subscription {
    /// Create a new enabled subscription with conservative defaults.
    #[must_use]
    pub fn new(template: impl Into<String>, trigger: impl Into<String>) -> Self {
        Self {
            template: template.into(),
            trigger: trigger.into(),
            enabled: true,
            concurrency_limit: 1,
            cooldown: Duration::ZERO,
            dedup_ttl: Duration::from_secs(60),
            state: Arc::new(SubscriptionState::new()),
        }
    }

    /// Disable the subscription.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Set the maximum number of concurrent dispatches allowed.
    #[must_use]
    pub const fn with_concurrency_limit(mut self, limit: usize) -> Self {
        self.concurrency_limit = limit;
        self
    }

    /// Set the minimum delay between dispatches.
    #[must_use]
    pub const fn with_cooldown(mut self, cooldown: Duration) -> Self {
        self.cooldown = cooldown;
        self
    }

    /// Set the deduplication window.
    #[must_use]
    pub const fn with_dedup_ttl(mut self, ttl: Duration) -> Self {
        self.dedup_ttl = ttl;
        self
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

    /// Whether the subscription is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Reserve a concurrency slot if the current active count is below the limit.
    #[must_use]
    pub fn check_concurrency_limit(&self) -> bool {
        if self.concurrency_limit == 0 {
            return false;
        }

        let mut current = self.state.active.load(Ordering::Acquire);
        loop {
            if current >= self.concurrency_limit {
                return false;
            }

            match self.state.active.compare_exchange(
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

    /// Release one reserved concurrency slot.
    pub fn release_concurrency(&self) {
        let _ = self
            .state
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
    }

    /// Check and update the cooldown gate.
    #[must_use]
    pub fn check_cooldown(&self) -> bool {
        if self.cooldown.is_zero() {
            return true;
        }

        let now = Instant::now();
        let mut last = self.state.last_triggered.lock();
        if let Some(previous) = *last {
            if now.duration_since(previous) < self.cooldown {
                return false;
            }
        }
        *last = Some(now);
        true
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
}

impl SubscriptionRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry seeded with subscriptions.
    #[must_use]
    pub fn with_subscriptions(subscriptions: Vec<Subscription>) -> Self {
        Self {
            subscriptions: Arc::new(RwLock::new(subscriptions)),
        }
    }

    /// Add a subscription to the registry.
    pub fn insert(&self, subscription: Subscription) {
        self.subscriptions.write().push(subscription);
    }

    /// Return subscriptions whose trigger matches `signal`.
    #[must_use]
    pub fn find_matching(&self, signal: &Signal) -> Vec<Subscription> {
        let kind = signal.kind.as_str();
        self.subscriptions
            .read()
            .iter()
            .filter(|subscription| {
                subscription.is_enabled() && glob_match(subscription.trigger(), kind)
            })
            .cloned()
            .collect()
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
            if !sub.check_concurrency_limit() {
                continue;
            }

            if sub.check_cooldown() && sub.check_dedup(&signal) {
                let signal = signal.clone();
                let dispatcher = Arc::clone(&dispatcher);
                let sub_for_task = sub.clone();
                tokio::spawn(async move {
                    dispatch_agent(sub_for_task.clone(), signal, dispatcher).await;
                    sub_for_task.release_concurrency();
                });
            } else {
                sub.release_concurrency();
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
}
