//! Nexus relay boundary definitions.
//!
//! Nexus is a WebSocket relay that sits between roko-serve and remote
//! surfaces (dashboard, remote TUI). It forwards events and aggregates
//! heartbeats but does NOT serve as a second backend.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Connection state
// ---------------------------------------------------------------------------

/// Connection state for a relay-connected surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum RelayConnectionState {
    /// Connected directly to roko-serve (no relay).
    Direct,
    /// Connected via Nexus relay.
    Relayed {
        /// WebSocket URL of the relay.
        relay_url: String,
    },
    /// Relay was configured but is currently unreachable.
    Degraded {
        /// WebSocket URL of the relay.
        relay_url: String,
        /// ISO-8601 timestamp when degradation was first observed.
        since: String,
        /// Human-readable reason for the degradation.
        reason: String,
    },
    /// No relay configured; surface is local only.
    Local,
}

impl Default for RelayConnectionState {
    fn default() -> Self {
        Self::Local
    }
}

// ---------------------------------------------------------------------------
// Data freshness
// ---------------------------------------------------------------------------

/// Freshness indicator for relay-sourced data.
///
/// Surfaces use this to decide whether to show stale-state warnings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataFreshness {
    /// When the data was last confirmed from the source (Unix epoch seconds).
    pub last_confirmed_at: u64,
    /// Whether the data may be stale.
    pub stale: bool,
    /// How many seconds since last confirmation.
    pub age_secs: u64,
    /// Maximum age before data is considered stale (configurable).
    pub stale_threshold_secs: u64,
}

impl DataFreshness {
    /// Create a new freshness indicator in the "confirmed" state.
    #[must_use]
    pub fn new(stale_threshold_secs: u64) -> Self {
        let now = now_epoch_secs();
        Self {
            last_confirmed_at: now,
            stale: false,
            age_secs: 0,
            stale_threshold_secs,
        }
    }

    /// Mark the data as freshly confirmed from the source.
    pub fn mark_confirmed(&mut self) {
        self.last_confirmed_at = now_epoch_secs();
        self.age_secs = 0;
        self.stale = false;
    }

    /// Recompute age and staleness against the current time.
    pub fn check(&mut self) {
        let now = now_epoch_secs();
        self.age_secs = now.saturating_sub(self.last_confirmed_at);
        self.stale = self.age_secs > self.stale_threshold_secs;
    }
}

// ---------------------------------------------------------------------------
// Relay heartbeat
// ---------------------------------------------------------------------------

/// Aggregate heartbeat metrics from the relay connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayHeartbeat {
    /// Most recent round-trip ping time in milliseconds.
    pub last_ping_ms: u64,
    /// Rolling average round-trip ping time in milliseconds.
    pub avg_ping_ms: u64,
    /// Number of consecutive missed heartbeats.
    pub missed_heartbeats: u64,
}

// ---------------------------------------------------------------------------
// Relay health (top-level diagnostic)
// ---------------------------------------------------------------------------

/// Relay health status for operator diagnostics.
///
/// Exposed via `GET /api/relay/health` and consumed by the TUI status bar
/// and dashboard connection indicator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayHealth {
    /// Current connection state.
    pub connection: RelayConnectionState,
    /// Freshness of relay-sourced data.
    pub freshness: DataFreshness,
    /// Aggregate heartbeat stats (present only when relay is connected).
    pub heartbeat: Option<RelayHeartbeat>,
}

impl Default for RelayHealth {
    fn default() -> Self {
        Self {
            connection: RelayConnectionState::Local,
            freshness: DataFreshness::new(DEFAULT_STALE_THRESHOLD_SECS),
            heartbeat: None,
        }
    }
}

impl RelayHealth {
    /// Returns `true` if the relay connection is healthy.
    ///
    /// Healthy means the connection is `Direct`, `Relayed`, or `Local`
    /// **and** the data is not stale. `Degraded` is never healthy.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        let connected = matches!(
            self.connection,
            RelayConnectionState::Direct
                | RelayConnectionState::Relayed { .. }
                | RelayConnectionState::Local
        );
        connected && !self.freshness.stale
    }
}

// ---------------------------------------------------------------------------
// Workspace registration (roko-serve → relay)
// ---------------------------------------------------------------------------

use std::sync::Arc;

use parking_lot::RwLock;
use roko_core::config::schema::RelayConfig;
use roko_core::defaults::{
    DEFAULT_RELAY_CIRCUIT_BREAKER_BASE_BACKOFF_SECS,
    DEFAULT_RELAY_CIRCUIT_BREAKER_MAX_BACKOFF_SECS, DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD,
    DEFAULT_RELAY_STALE_THRESHOLD_SECS,
};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Default staleness threshold in seconds.
pub const DEFAULT_STALE_THRESHOLD_SECS: u64 = DEFAULT_RELAY_STALE_THRESHOLD_SECS;

/// Resolve the public URL for this roko instance.
///
/// Priority: config > RAILWAY_PUBLIC_DOMAIN > FLY_APP_NAME > localhost fallback.
fn resolve_public_url(config: &RelayConfig, port: u16) -> String {
    if let Some(url) = &config.public_url {
        return url.clone();
    }
    if let Ok(domain) = std::env::var("RAILWAY_PUBLIC_DOMAIN") {
        return format!("https://{domain}");
    }
    if let Ok(app) = std::env::var("FLY_APP_NAME") {
        return format!("https://{app}.fly.dev");
    }
    format!("http://localhost:{port}")
}

/// Extract `scheme://host:port` from a relay URL, stripping any path.
/// Keeps the original scheme (ws/wss/http/https).
pub fn normalize_relay_base_url(url: &str) -> String {
    if let Some(idx) = url.find("://") {
        let after_scheme = &url[idx + 3..];
        if let Some(slash) = after_scheme.find('/') {
            return url[..idx + 3 + slash].to_string();
        }
    }
    url.to_string()
}

/// Extract `http(s)://host:port` from a WS relay URL, stripping any path.
fn normalize_ws_to_http_base(ws_url: &str) -> String {
    let base = normalize_relay_base_url(ws_url);
    base.replace("wss://", "https://")
        .replace("ws://", "http://")
}

/// Resolve the workspace name.
///
/// Priority: config > hostname > "roko".
fn resolve_workspace_name(config: &RelayConfig) -> String {
    if let Some(name) = &config.workspace_name {
        return name.clone();
    }
    gethostname::gethostname()
        .into_string()
        .unwrap_or_else(|_| "roko".into())
}

/// Compute the backoff delay for the circuit breaker.
///
/// After `DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD` consecutive failures, applies
/// exponential backoff capped by the relay defaults.
fn circuit_breaker_backoff(consecutive_failures: u32) -> std::time::Duration {
    if consecutive_failures < DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD {
        return std::time::Duration::ZERO;
    }
    let exponent = consecutive_failures.saturating_sub(DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD);
    let backoff_secs =
        DEFAULT_RELAY_CIRCUIT_BREAKER_BASE_BACKOFF_SECS.saturating_mul(1u64 << exponent.min(30));
    std::time::Duration::from_secs(backoff_secs.min(DEFAULT_RELAY_CIRCUIT_BREAKER_MAX_BACKOFF_SECS))
}

/// Transition the shared relay health to `Degraded` state.
fn mark_degraded(health: &RwLock<RelayHealth>, relay_url: &str, reason: &str) {
    let mut h = health.write();
    if !matches!(h.connection, RelayConnectionState::Degraded { .. }) {
        warn!(
            relay_url = %relay_url,
            reason = %reason,
            "relay connection degraded"
        );
    }
    h.connection = RelayConnectionState::Degraded {
        relay_url: relay_url.to_string(),
        since: chrono::Utc::now().to_rfc3339(),
        reason: reason.to_string(),
    };
    h.freshness.check();
}

/// Transition the shared relay health back to `Relayed` state.
fn mark_relayed(health: &RwLock<RelayHealth>, relay_url: &str) {
    let mut h = health.write();
    h.connection = RelayConnectionState::Relayed {
        relay_url: relay_url.to_string(),
    };
    h.freshness.mark_confirmed();
}

/// Start a background task that registers this roko instance with the relay
/// and sends periodic heartbeats with a circuit breaker.
///
/// Returns `None` if relay is not configured.
pub fn start_workspace_registration(
    relay_config: RelayConfig,
    port: u16,
    agent_count: Arc<std::sync::atomic::AtomicU32>,
    relay_health: Arc<RwLock<RelayHealth>>,
) -> Option<JoinHandle<()>> {
    let relay_url = relay_config.url.as_deref()?.to_string();
    let public_url = resolve_public_url(&relay_config, port);
    let workspace_name = resolve_workspace_name(&relay_config);
    let workspace_id = format!("ws-{}", uuid::Uuid::new_v4());
    let heartbeat_secs = relay_config.heartbeat_interval_secs;

    info!(
        relay_url = %relay_url,
        public_url = %public_url,
        workspace_name = %workspace_name,
        "starting workspace relay registration"
    );

    Some(tokio::spawn(async move {
        // Convert relay WS URL to HTTP base (scheme + authority only).
        let relay_http = normalize_ws_to_http_base(&relay_url);
        let client = reqwest::Client::new();

        // Register workspace.
        let register_url = format!("{relay_http}/relay/workspaces/register");
        let body = serde_json::json!({
            "workspace_id": workspace_id,
            "name": workspace_name,
            "url": public_url,
            "version": env!("CARGO_PKG_VERSION"),
            "agents_count": agent_count.load(std::sync::atomic::Ordering::Relaxed),
        });

        match client.post(&register_url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!(workspace_id = %workspace_id, "registered with relay");
                mark_relayed(&relay_health, &relay_url);
            }
            Ok(resp) => {
                warn!(
                    status = %resp.status(),
                    "relay registration returned non-success"
                );
            }
            Err(e) => {
                warn!(error = %e, "failed to register with relay (will retry on heartbeat)");
            }
        }

        // Periodic heartbeat loop with circuit breaker.
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(heartbeat_secs));
        let mut consecutive_failures: u32 = 0;

        loop {
            interval.tick().await;

            // Apply exponential backoff when the circuit breaker is open.
            let backoff = circuit_breaker_backoff(consecutive_failures);
            if !backoff.is_zero() {
                debug!(
                    failures = consecutive_failures,
                    backoff_secs = backoff.as_secs(),
                    "circuit breaker active, applying backoff"
                );
                tokio::time::sleep(backoff).await;
            }

            let heartbeat_url = format!("{relay_http}/relay/workspaces/{workspace_id}/heartbeat");
            let hb_body = serde_json::json!({
                "agents_count": agent_count.load(std::sync::atomic::Ordering::Relaxed),
            });

            match client.post(&heartbeat_url).json(&hb_body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    debug!(workspace_id = %workspace_id, "relay heartbeat sent");
                    if consecutive_failures > 0 {
                        info!(
                            previous_failures = consecutive_failures,
                            "relay heartbeat recovered"
                        );
                    }
                    consecutive_failures = 0;
                    mark_relayed(&relay_health, &relay_url);
                }
                Ok(resp) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    debug!(
                        status = %resp.status(),
                        consecutive_failures,
                        "relay heartbeat returned non-success, re-registering"
                    );
                    if consecutive_failures >= DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD {
                        mark_degraded(
                            &relay_health,
                            &relay_url,
                            &format!("heartbeat non-success: {}", resp.status()),
                        );
                    }
                    // Re-register in case the relay restarted.
                    let body = serde_json::json!({
                        "workspace_id": workspace_id,
                        "name": workspace_name,
                        "url": public_url,
                        "version": env!("CARGO_PKG_VERSION"),
                        "agents_count": agent_count.load(std::sync::atomic::Ordering::Relaxed),
                    });
                    let _ = client.post(&register_url).json(&body).send().await;
                }
                Err(e) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    debug!(
                        error = %e,
                        consecutive_failures,
                        "relay heartbeat failed"
                    );
                    if consecutive_failures >= DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD {
                        mark_degraded(&relay_health, &relay_url, &format!("heartbeat error: {e}"));
                    }
                }
            }
        }
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_data_is_not_stale() {
        let freshness = DataFreshness::new(DEFAULT_STALE_THRESHOLD_SECS);
        assert!(!freshness.stale);
        assert_eq!(freshness.age_secs, 0);
    }

    #[test]
    fn mark_confirmed_resets_staleness() {
        let mut freshness = DataFreshness {
            last_confirmed_at: 0,
            stale: true,
            age_secs: 999,
            stale_threshold_secs: DEFAULT_STALE_THRESHOLD_SECS,
        };
        freshness.mark_confirmed();
        assert!(!freshness.stale);
        assert_eq!(freshness.age_secs, 0);
    }

    #[test]
    fn check_detects_stale_data() {
        let mut freshness = DataFreshness {
            last_confirmed_at: now_epoch_secs().saturating_sub(DEFAULT_STALE_THRESHOLD_SECS * 2),
            stale: false,
            age_secs: 0,
            stale_threshold_secs: DEFAULT_STALE_THRESHOLD_SECS,
        };
        freshness.check();
        assert!(freshness.stale);
        assert!(freshness.age_secs >= 59);
    }

    #[test]
    fn relay_health_direct_and_fresh_is_healthy() {
        let health = RelayHealth {
            connection: RelayConnectionState::Direct,
            freshness: DataFreshness::new(DEFAULT_STALE_THRESHOLD_SECS),
            heartbeat: None,
        };
        assert!(health.is_healthy());
    }

    #[test]
    fn relay_health_degraded_is_not_healthy() {
        let health = RelayHealth {
            connection: RelayConnectionState::Degraded {
                relay_url: "wss://relay.example.com".into(),
                since: "2026-04-21T00:00:00Z".into(),
                reason: "connection refused".into(),
            },
            freshness: DataFreshness::new(DEFAULT_STALE_THRESHOLD_SECS),
            heartbeat: None,
        };
        assert!(!health.is_healthy());
    }

    #[test]
    fn relay_health_stale_data_is_not_healthy() {
        let mut freshness = DataFreshness::new(DEFAULT_STALE_THRESHOLD_SECS);
        freshness.last_confirmed_at =
            now_epoch_secs().saturating_sub(DEFAULT_STALE_THRESHOLD_SECS * 2);
        freshness.check();

        let health = RelayHealth {
            connection: RelayConnectionState::Direct,
            freshness,
            heartbeat: None,
        };
        assert!(!health.is_healthy());
    }

    #[test]
    fn default_relay_health_is_local() {
        let health = RelayHealth::default();
        assert_eq!(health.connection, RelayConnectionState::Local);
        assert!(health.is_healthy()); // local + fresh = healthy
    }

    #[test]
    fn circuit_breaker_no_backoff_below_threshold() {
        for failures in 0..DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD {
            assert_eq!(circuit_breaker_backoff(failures), std::time::Duration::ZERO);
        }
    }

    #[test]
    fn circuit_breaker_exponential_backoff_at_threshold() {
        assert_eq!(
            circuit_breaker_backoff(DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD),
            std::time::Duration::from_secs(DEFAULT_RELAY_CIRCUIT_BREAKER_BASE_BACKOFF_SECS)
        );
        assert_eq!(
            circuit_breaker_backoff(DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD + 1),
            std::time::Duration::from_secs(DEFAULT_RELAY_CIRCUIT_BREAKER_BASE_BACKOFF_SECS * 2)
        );
        assert_eq!(
            circuit_breaker_backoff(DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD + 2),
            std::time::Duration::from_secs(DEFAULT_RELAY_CIRCUIT_BREAKER_BASE_BACKOFF_SECS * 4)
        );
    }

    #[test]
    fn circuit_breaker_caps_at_max() {
        assert_eq!(
            circuit_breaker_backoff(100),
            std::time::Duration::from_secs(DEFAULT_RELAY_CIRCUIT_BREAKER_MAX_BACKOFF_SECS)
        );
    }

    #[test]
    fn mark_degraded_transitions_health() {
        let health = Arc::new(RwLock::new(RelayHealth::default()));
        mark_degraded(&health, "wss://relay.example.com", "test failure");
        let h = health.read();
        assert!(matches!(
            h.connection,
            RelayConnectionState::Degraded { .. }
        ));
        assert!(!h.is_healthy());
    }

    #[test]
    fn mark_relayed_recovers_health() {
        let health = Arc::new(RwLock::new(RelayHealth::default()));
        mark_degraded(&health, "wss://relay.example.com", "test failure");
        mark_relayed(&health, "wss://relay.example.com");
        let h = health.read();
        assert!(matches!(h.connection, RelayConnectionState::Relayed { .. }));
        assert!(h.is_healthy());
    }

    #[test]
    fn start_workspace_registration_returns_none_without_url() {
        let config = RelayConfig::default();
        let agent_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let relay_health = Arc::new(RwLock::new(RelayHealth::default()));
        assert!(start_workspace_registration(config, 6677, agent_count, relay_health).is_none());
    }

    #[test]
    fn relay_connection_state_serializes() {
        let state = RelayConnectionState::Relayed {
            relay_url: "wss://relay.example.com".into(),
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("relayed"));
        assert!(json.contains("relay.example.com"));

        let roundtrip: RelayConnectionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, roundtrip);
    }
}
