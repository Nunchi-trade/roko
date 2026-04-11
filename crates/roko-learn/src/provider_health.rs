//! Per-provider circuit breaker for LLM routing (§13.9).
//!
//! Tracks consecutive failures per provider and transitions through a
//! three-state machine:
//!
//! ```text
//! Healthy ──[N consecutive failures]──▶ Unhealthy { recovery_at }
//!     ▲                                        │
//!     │                                  [now ≥ recovery_at]
//!     │                                        ▼
//!     └────[record_success]──────────── Probing
//!                                 [record_failure]──▶ Unhealthy (timer reset)
//! ```
//!
//! # Thread safety
//!
//! All state is behind a [`parking_lot::RwLock`], making the tracker safe
//! for concurrent use from multiple tokio tasks.
//!
//! # `Instant` vs `SystemTime`
//!
//! Recovery timestamps use [`std::time::Instant`] so they are immune to
//! wall-clock adjustments. Because `Instant` is not serializable, the
//! tracker is an in-memory runtime component only. Persisted provider
//! snapshots use unix milliseconds and are handled by
//! [`ProviderHealthRegistry`].

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ─── Serializable health snapshot types ────────────────────────────────────

/// Serialized circuit state for persisted provider-health snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitState {
    /// Normal operation.
    Closed,
    /// Requests are blocked while the provider cools down.
    Open,
    /// One probe request is allowed after cooldown expires.
    HalfOpen,
}

/// Classified error category used to pick cooldown durations later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorClass {
    /// Provider returned a rate limit response.
    RateLimit,
    /// Provider returned an authentication or authorization failure.
    AuthFailure,
    /// Request timed out before completing.
    Timeout,
    /// Provider returned a 5xx or other transient server error.
    ServerError,
    /// Request was blocked by content policy.
    ContentPolicy,
    /// Context exceeded the provider's maximum window.
    ContextOverflow,
    /// Fallback classification when the exact class is unknown.
    Unknown,
}

/// Timestamped failure entry for the rolling failure window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureRecord {
    /// Failure timestamp in unix milliseconds.
    pub timestamp_ms: i64,
    /// Classified failure type.
    pub error_class: ErrorClass,
}

/// Serializable per-provider health snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealth {
    /// Stable provider identifier.
    pub provider_id: String,
    /// Snapshot circuit state.
    pub state: CircuitState,
    /// Consecutive failures seen most recently.
    pub consecutive_failures: u32,
    /// Lifetime request count.
    pub total_requests: u64,
    /// Lifetime failure count.
    pub total_failures: u64,
    /// Timestamp of the most recent failure, in unix milliseconds.
    pub last_failure_at: Option<i64>,
    /// Timestamp when the provider may be retried, in unix milliseconds.
    pub cooldown_until: Option<i64>,
    /// Rolling window of recent failures.
    pub failure_window: Vec<FailureRecord>,
}

impl ProviderHealth {
    /// Record a successful request.
    ///
    /// A successful probe from `HalfOpen` closes the circuit again.
    pub fn record_success(&mut self) {
        self.total_requests = self.total_requests.saturating_add(1);
        self.consecutive_failures = 0;
        self.cooldown_until = None;
        if self.state == CircuitState::HalfOpen {
            self.state = CircuitState::Closed;
        }
    }

    /// Record a failed request and update the circuit state.
    pub fn record_failure(&mut self, error: ErrorClass, now_ms: i64) {
        self.total_requests = self.total_requests.saturating_add(1);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.total_failures = self.total_failures.saturating_add(1);
        self.last_failure_at = Some(now_ms);
        self.failure_window.push(FailureRecord {
            timestamp_ms: now_ms,
            error_class: error,
        });
        if self.failure_window.len() > 20 {
            self.failure_window.remove(0);
        }

        // Trip to Open after 3 consecutive failures.
        if self.consecutive_failures >= 3 {
            self.state = CircuitState::Open;
            self.cooldown_until = Some(now_ms + self.cooldown_ms(error));
        }
    }

    /// Return whether the provider can receive a request at `now_ms`.
    ///
    /// When an open circuit's cooldown expires, the state advances to
    /// `HalfOpen` so the next request can act as a probe.
    pub fn is_available(&mut self, now_ms: i64) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(until) = self.cooldown_until {
                    if now_ms >= until {
                        self.state = CircuitState::HalfOpen;
                        return true;
                    }
                }
                false
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Error-class-specific cooldown in milliseconds.
    fn cooldown_ms(&self, error: ErrorClass) -> i64 {
        match error {
            ErrorClass::RateLimit => 5_000,
            ErrorClass::Timeout => 10_000,
            ErrorClass::ServerError => 30_000,
            ErrorClass::AuthFailure => 300_000,
            _ => 5_000,
        }
    }
}

/// Persisted registry snapshot for loading and saving provider health.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ProviderHealthRegistrySnapshot {
    /// Per-provider health snapshots keyed by provider id.
    providers: HashMap<String, ProviderHealth>,
}

/// Thread-safe registry of provider health snapshots.
///
/// The registry stores [`ProviderHealth`] values keyed by provider id and
/// provides a disk-backed persistence layer for the runtime circuit breaker.
pub struct ProviderHealthRegistry {
    providers: Mutex<HashMap<String, ProviderHealth>>,
}

impl ProviderHealthRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            providers: Mutex::new(HashMap::new()),
        }
    }

    /// Record a successful request for `provider_id`.
    pub fn record_success(&self, provider_id: &str) {
        let mut providers = self.providers.lock();
        let health = providers
            .entry(provider_id.to_owned())
            .or_insert_with(|| new_provider_health(provider_id));
        health.record_success();
    }

    /// Record a failed request for `provider_id`.
    pub fn record_failure(&self, provider_id: &str, error: ErrorClass) {
        let mut providers = self.providers.lock();
        let health = providers
            .entry(provider_id.to_owned())
            .or_insert_with(|| new_provider_health(provider_id));
        health.record_failure(error, unix_ms_now());
    }

    /// Return whether `provider_id` is currently available for routing.
    ///
    /// Unknown providers are treated as available.
    pub fn is_available(&self, provider_id: &str) -> bool {
        let mut providers = self.providers.lock();
        match providers.get_mut(provider_id) {
            Some(health) => health.is_available(unix_ms_now()),
            None => true,
        }
    }

    /// Filter `candidates` to only providers that are currently available.
    pub fn available_providers(&self, candidates: &[String]) -> Vec<String> {
        candidates
            .iter()
            .filter(|provider_id| self.is_available(provider_id))
            .cloned()
            .collect()
    }

    /// Persist the registry to `path` as JSON.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let snapshot = ProviderHealthRegistrySnapshot {
            providers: self.providers.lock().clone(),
        };
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Load the registry from `path`, or return a new empty registry.
    pub fn load_or_new(path: &Path) -> Self {
        let snapshot = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<ProviderHealthRegistrySnapshot>(&s).ok());

        match snapshot {
            Some(snapshot) => Self {
                providers: Mutex::new(snapshot.providers),
            },
            None => Self::new(),
        }
    }
}

impl Default for ProviderHealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn new_provider_health(provider_id: &str) -> ProviderHealth {
    ProviderHealth {
        provider_id: provider_id.to_owned(),
        state: CircuitState::Closed,
        consecutive_failures: 0,
        total_requests: 0,
        total_failures: 0,
        last_failure_at: None,
        cooldown_until: None,
        failure_window: Vec::new(),
    }
}

fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

// ─── HealthState ─────────────────────────────────────────────────────────────

/// Circuit-breaker state for a single provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    /// Provider is accepting requests normally.
    Healthy,
    /// Provider has tripped the failure threshold and is cooling down.
    /// `recovery_at` is the earliest instant a probe may be attempted.
    Unhealthy {
        /// Earliest instant at which the provider may be probed.
        recovery_at: Instant,
    },
    /// One probe request has been allowed; awaiting its outcome.
    Probing,
}

// ─── ProviderStatus ──────────────────────────────────────────────────────────

/// Snapshot of a single provider's health bookkeeping.
#[derive(Debug, Clone)]
pub struct ProviderStatus {
    /// Provider identifier (e.g. `"openai"`, `"anthropic"`).
    pub provider: String,
    /// Current circuit-breaker state.
    pub state: HealthState,
    /// Number of failures since the last success.
    pub consecutive_failures: u32,
    /// When the most recent failure was recorded.
    pub last_failure_at: Option<Instant>,
    /// When the most recent success was recorded.
    pub last_success_at: Option<Instant>,
    /// Lifetime attempts routed through this provider.
    pub total_attempts: u64,
    /// Lifetime successful attempts.
    pub total_successes: u64,
}

impl ProviderStatus {
    /// Create a fresh status entry for `provider`.
    const fn new(provider: String) -> Self {
        Self {
            provider,
            state: HealthState::Healthy,
            consecutive_failures: 0,
            last_failure_at: None,
            last_success_at: None,
            total_attempts: 0,
            total_successes: 0,
        }
    }
}

// ─── ProviderHealthTracker ───────────────────────────────────────────────────

/// Per-provider circuit breaker that gates bandit arm selection.
///
/// Use [`record_success`](Self::record_success) and
/// [`record_failure`](Self::record_failure) after each LLM call, then
/// call [`is_healthy`](Self::is_healthy) or
/// [`filter_arms`](Self::filter_arms) before selecting the next provider.
pub struct ProviderHealthTracker {
    /// Per-provider status, keyed by provider name.
    providers: RwLock<HashMap<String, ProviderStatus>>,
    /// Number of consecutive failures required to trip the breaker.
    failure_threshold: u32,
    /// Duration a provider stays in `Unhealthy` before a probe is allowed.
    recovery_window: Duration,
}

impl ProviderHealthTracker {
    /// Create a tracker with default thresholds (3 failures, 120 s recovery).
    pub fn new() -> Self {
        Self::with_config(3, Duration::from_secs(120))
    }

    /// Create a tracker with custom thresholds.
    pub fn with_config(failure_threshold: u32, recovery_window: Duration) -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
            failure_threshold,
            recovery_window,
        }
    }

    /// Record a successful LLM call for `provider`.
    ///
    /// Resets `consecutive_failures` to 0 and transitions the provider to
    /// [`HealthState::Healthy`] regardless of current state.
    #[allow(clippy::significant_drop_tightening)]
    pub fn record_success(&self, provider: &str) {
        let now = Instant::now();
        let mut map = self.providers.write();
        let status = map
            .entry(provider.to_owned())
            .or_insert_with(|| ProviderStatus::new(provider.to_owned()));

        status.total_attempts += 1;
        status.total_successes += 1;
        status.consecutive_failures = 0;
        status.last_success_at = Some(now);
        status.state = HealthState::Healthy;
    }

    /// Record a failed LLM call for `provider`.
    ///
    /// Increments consecutive failures. When the counter reaches the
    /// configured threshold the provider transitions to
    /// [`HealthState::Unhealthy`].
    #[allow(clippy::significant_drop_tightening)]
    pub fn record_failure(&self, provider: &str) {
        let now = Instant::now();
        let mut map = self.providers.write();
        let status = map
            .entry(provider.to_owned())
            .or_insert_with(|| ProviderStatus::new(provider.to_owned()));

        status.total_attempts += 1;
        status.consecutive_failures = status.consecutive_failures.saturating_add(1);
        status.last_failure_at = Some(now);

        // Transition on threshold or re-trip from Probing.
        if status.consecutive_failures >= self.failure_threshold
            || status.state == HealthState::Probing
        {
            status.state = HealthState::Unhealthy {
                recovery_at: now + self.recovery_window,
            };
        }
    }

    /// Returns `true` if the provider should receive traffic.
    ///
    /// - [`HealthState::Healthy`] → `true`
    /// - [`HealthState::Unhealthy`] with expired recovery window → transitions
    ///   to [`HealthState::Probing`] and returns `true` **once**.
    /// - [`HealthState::Probing`] (already transitioned) → `false`
    /// - [`HealthState::Unhealthy`] not yet expired → `false`
    /// - Unknown provider → `true` (lazily treated as healthy).
    pub fn is_healthy(&self, provider: &str) -> bool {
        // Fast path: read lock only.
        {
            let map = self.providers.read();
            match map.get(provider) {
                None => return true,
                Some(s) => match s.state {
                    HealthState::Healthy => return true,
                    HealthState::Probing => return false,
                    HealthState::Unhealthy { recovery_at } => {
                        if Instant::now() < recovery_at {
                            return false;
                        }
                        // Need to transition — fall through to write path.
                    }
                },
            }
        }

        // Slow path: upgrade to write lock and transition to Probing.
        let mut map = self.providers.write();
        if let Some(status) = map.get_mut(provider) {
            // Re-check after acquiring write lock (another thread may have
            // already transitioned).
            match status.state {
                HealthState::Unhealthy { recovery_at } if Instant::now() >= recovery_at => {
                    status.state = HealthState::Probing;
                    true
                }
                HealthState::Healthy => true,
                _ => false,
            }
        } else {
            // Inserted between our read and write — treat as healthy.
            true
        }
    }

    /// Filter a set of bandit arms, removing those whose provider is
    /// currently unhealthy.
    ///
    /// `provider_of` maps each arm identifier to its provider name.
    pub fn filter_arms<F>(&self, arms: &[String], provider_of: F) -> Vec<String>
    where
        F: Fn(&str) -> String,
    {
        arms.iter()
            .filter(|arm| self.is_healthy(&provider_of(arm)))
            .cloned()
            .collect()
    }

    /// Return a snapshot of every tracked provider's status.
    pub fn snapshot(&self) -> Vec<ProviderStatus> {
        self.providers.read().values().cloned().collect()
    }
}

impl Default for ProviderHealthTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Unknown provider is implicitly healthy.
    #[test]
    fn unknown_provider_is_healthy() {
        let tracker = ProviderHealthTracker::new();
        assert!(tracker.is_healthy("never-seen"));
    }

    /// Three consecutive failures trip the breaker.
    #[test]
    fn three_failures_trips_breaker() {
        let tracker = ProviderHealthTracker::new();
        tracker.record_failure("p");
        tracker.record_failure("p");
        assert!(tracker.is_healthy("p"), "still healthy after 2 failures");

        tracker.record_failure("p");
        assert!(!tracker.is_healthy("p"), "unhealthy after 3 failures");
    }

    /// Two failures then a success resets the counter — stays healthy.
    #[test]
    fn success_resets_failure_counter() {
        let tracker = ProviderHealthTracker::new();
        tracker.record_failure("p");
        tracker.record_failure("p");
        tracker.record_success("p");
        tracker.record_failure("p");
        tracker.record_failure("p");
        assert!(
            tracker.is_healthy("p"),
            "counter was reset so 2 failures is still healthy"
        );
    }

    /// Unhealthy provider before recovery window expires → false.
    #[test]
    fn unhealthy_before_recovery() {
        let tracker = ProviderHealthTracker::with_config(1, Duration::from_secs(600));
        tracker.record_failure("p");
        assert!(!tracker.is_healthy("p"));
    }

    /// After recovery window the first `is_healthy` call returns true
    /// (transitions to Probing).
    #[test]
    fn unhealthy_expires_into_probing() {
        let tracker = ProviderHealthTracker::with_config(1, Duration::from_millis(0));
        tracker.record_failure("p");
        // recovery_at is effectively in the past immediately.
        assert!(
            tracker.is_healthy("p"),
            "first call after recovery → true (Probing)"
        );
    }

    /// While Probing, a second `is_healthy` call returns false.
    #[test]
    fn probing_only_allows_one_request() {
        let tracker = ProviderHealthTracker::with_config(1, Duration::from_millis(0));
        tracker.record_failure("p");
        assert!(tracker.is_healthy("p"), "first probe allowed");
        assert!(
            !tracker.is_healthy("p"),
            "second call while probing → false"
        );
    }

    /// Probing + success → Healthy, counter reset.
    #[test]
    fn probing_success_restores_healthy() {
        let tracker = ProviderHealthTracker::with_config(1, Duration::from_millis(0));
        tracker.record_failure("p");
        assert!(tracker.is_healthy("p")); // transitions to Probing
        tracker.record_success("p");

        // Now it should be Healthy again.
        assert!(tracker.is_healthy("p"));
        // And the counter is reset — one failure alone shouldn't trip it.
        // (Actually threshold is 1 here, so one failure *will* trip it — use 2)
        let snap: Vec<_> = tracker
            .snapshot()
            .into_iter()
            .filter(|s| s.provider == "p")
            .collect();
        assert_eq!(snap[0].consecutive_failures, 0);
    }

    /// Probing + failure → Unhealthy with a new recovery timer.
    #[test]
    fn probing_failure_retrips_breaker() {
        let tracker = ProviderHealthTracker::with_config(1, Duration::from_millis(0));
        tracker.record_failure("p"); // trip
        assert!(tracker.is_healthy("p")); // → Probing

        // Now set a long recovery so re-trip is observable.
        // We can't change config, so instead just check state after failure.
        tracker.record_failure("p");
        // The provider should be Unhealthy again. With 0 ms recovery it will
        // immediately allow probing, but the state transition happened.
        let snap: Vec<_> = tracker
            .snapshot()
            .into_iter()
            .filter(|s| s.provider == "p")
            .collect();
        assert!(
            matches!(snap[0].state, HealthState::Unhealthy { .. }),
            "should be Unhealthy after probe failure"
        );
    }

    /// `filter_arms` removes arms whose provider is unhealthy.
    #[test]
    fn filter_arms_drops_unhealthy() {
        let tracker = ProviderHealthTracker::with_config(1, Duration::from_secs(600));
        tracker.record_failure("bad");
        tracker.record_success("good");

        let arms = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let result = tracker.filter_arms(&arms, |arm| {
            if arm == "b" {
                "bad".to_owned()
            } else {
                "good".to_owned()
            }
        });
        assert_eq!(result, vec!["a", "c"]);
    }

    /// `filter_arms` with empty input returns empty output.
    #[test]
    fn filter_arms_empty_input() {
        let tracker = ProviderHealthTracker::new();
        let result = tracker.filter_arms(&[], |arm| arm.to_owned());
        assert!(result.is_empty());
    }

    /// `snapshot` returns all tracked providers.
    #[test]
    fn snapshot_returns_all_providers() {
        let tracker = ProviderHealthTracker::new();
        tracker.record_success("alpha");
        tracker.record_failure("beta");
        tracker.record_success("gamma");

        let snap = tracker.snapshot();
        let mut names: Vec<_> = snap.iter().map(|s| s.provider.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    /// Concurrent access: 100 tasks each record a failure; final counter
    /// must equal 100.
    #[tokio::test]
    async fn concurrent_failures_are_consistent() {
        let tracker = Arc::new(ProviderHealthTracker::with_config(
            200,
            Duration::from_secs(600),
        ));
        let mut handles = Vec::new();

        for _ in 0..100 {
            let t = Arc::clone(&tracker);
            handles.push(tokio::spawn(async move {
                t.record_failure("contended");
            }));
        }

        for h in handles {
            h.await.expect("task panicked");
        }

        let snap: Vec<_> = tracker
            .snapshot()
            .into_iter()
            .filter(|s| s.provider == "contended")
            .collect();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].consecutive_failures, 100);
        assert_eq!(snap[0].total_attempts, 100);
    }

    /// Concurrent mixed operations: successes and failures interleaved.
    #[tokio::test]
    async fn concurrent_mixed_operations() {
        let tracker = Arc::new(ProviderHealthTracker::with_config(
            200,
            Duration::from_secs(600),
        ));
        let mut handles = Vec::new();

        for i in 0..100 {
            let t = Arc::clone(&tracker);
            handles.push(tokio::spawn(async move {
                if i % 2 == 0 {
                    t.record_success("mixed");
                } else {
                    t.record_failure("mixed");
                }
            }));
        }

        for h in handles {
            h.await.expect("task panicked");
        }

        let snap: Vec<_> = tracker
            .snapshot()
            .into_iter()
            .filter(|s| s.provider == "mixed")
            .collect();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].total_attempts, 100);
        assert_eq!(snap[0].total_successes, 50);
    }

    /// Serializable snapshot types round-trip through JSON.
    #[test]
    fn provider_health_types() {
        let health = ProviderHealth {
            provider_id: "anthropic".to_owned(),
            state: CircuitState::HalfOpen,
            consecutive_failures: 3,
            total_requests: 42,
            total_failures: 7,
            last_failure_at: Some(1_725_000_000_000),
            cooldown_until: Some(1_725_000_030_000),
            failure_window: vec![
                FailureRecord {
                    timestamp_ms: 1_725_000_000_000,
                    error_class: ErrorClass::RateLimit,
                },
                FailureRecord {
                    timestamp_ms: 1_725_000_010_000,
                    error_class: ErrorClass::Timeout,
                },
            ],
        };

        let json = serde_json::to_string(&health).expect("serialize provider health");
        let decoded: ProviderHealth =
            serde_json::from_str(&json).expect("deserialize provider health");
        assert_eq!(decoded, health);

        let state_json = serde_json::to_string(&CircuitState::Open).expect("serialize state");
        let decoded_state: CircuitState =
            serde_json::from_str(&state_json).expect("deserialize state");
        assert_eq!(decoded_state, CircuitState::Open);
    }

    /// Three consecutive failures trip the circuit to Open, and cooldown
    /// expiry advances it to HalfOpen.
    #[test]
    fn provider_health_circuit_breaker_transitions() {
        let mut health = ProviderHealth {
            provider_id: "openai".to_owned(),
            state: CircuitState::Closed,
            consecutive_failures: 0,
            total_requests: 0,
            total_failures: 0,
            last_failure_at: None,
            cooldown_until: None,
            failure_window: Vec::new(),
        };

        health.record_failure(ErrorClass::Timeout, 1_000);
        health.record_failure(ErrorClass::Timeout, 2_000);
        assert_eq!(health.state, CircuitState::Closed);
        assert!(health.is_available(2_500));

        health.record_failure(ErrorClass::Timeout, 3_000);
        assert_eq!(health.state, CircuitState::Open);
        assert_eq!(health.cooldown_until, Some(13_000));
        assert!(!health.is_available(12_999));
        assert!(health.is_available(13_000));
        assert_eq!(health.state, CircuitState::HalfOpen);

        health.record_success();
        assert_eq!(health.state, CircuitState::Closed);
        assert_eq!(health.consecutive_failures, 0);
    }

    /// Error classes map to distinct cooldown durations.
    #[test]
    fn provider_health_circuit_breaker_cooldowns() {
        let mut health = ProviderHealth {
            provider_id: "anthropic".to_owned(),
            state: CircuitState::Closed,
            consecutive_failures: 0,
            total_requests: 0,
            total_failures: 0,
            last_failure_at: None,
            cooldown_until: None,
            failure_window: Vec::new(),
        };

        health.record_failure(ErrorClass::RateLimit, 10);
        health.record_failure(ErrorClass::RateLimit, 20);
        health.record_failure(ErrorClass::RateLimit, 30);
        assert_eq!(health.cooldown_until, Some(5_030));

        health.state = CircuitState::Closed;
        health.consecutive_failures = 0;
        health.cooldown_until = None;

        health.record_failure(ErrorClass::AuthFailure, 100);
        health.record_failure(ErrorClass::AuthFailure, 200);
        health.record_failure(ErrorClass::AuthFailure, 300);
        assert_eq!(health.cooldown_until, Some(300_300));
    }

    /// Registry stores per-provider state and filters unavailable providers.
    #[test]
    fn provider_health_registry_filters_unavailable_providers() {
        let registry = ProviderHealthRegistry::new();
        registry.record_success("good");
        registry.record_failure("bad", ErrorClass::Timeout);
        registry.record_failure("bad", ErrorClass::Timeout);
        registry.record_failure("bad", ErrorClass::Timeout);

        let candidates = vec!["good".to_owned(), "bad".to_owned(), "unknown".to_owned()];
        assert_eq!(
            registry.available_providers(&candidates),
            vec!["good".to_owned(), "unknown".to_owned()]
        );
    }

    /// Registry snapshots persist to disk and load back intact.
    #[test]
    fn provider_health_registry_roundtrip() {
        let tmp = TempDir::new().expect("create tempdir");
        let path = tmp.path().join("provider-health.json");

        let registry = ProviderHealthRegistry::new();
        registry.record_success("alpha");
        registry.record_failure("beta", ErrorClass::RateLimit);
        registry.record_failure("beta", ErrorClass::RateLimit);
        registry.record_failure("beta", ErrorClass::RateLimit);
        registry.save(&path).expect("save registry");

        let loaded = ProviderHealthRegistry::load_or_new(&path);
        assert!(loaded.is_available("alpha"));
        assert!(!loaded.is_available("beta"));

        let mut providers = loaded
            .providers
            .lock()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        providers.sort();
        assert_eq!(providers, vec!["alpha".to_owned(), "beta".to_owned()]);
    }
}
