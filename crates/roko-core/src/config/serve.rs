//! HTTP serving, server, deploy, scheduler, and webhook configuration sections.

use serde::{Deserialize, Serialize};

use super::agent::default_true;

// ---- [serve] -------------------------------------------------------------

/// API serving options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServeConfig {
    /// Port override for `roko serve`. Falls back to `server.port` (default 6677).
    #[serde(default)]
    pub port: Option<u16>,
    /// Shared transcript retention period in days.
    ///
    /// Newly created shares expire after this many days unless they are
    /// created with `--no-expire`.
    #[serde(default = "default_share_ttl_days")]
    pub share_ttl_days: u64,
    /// Whether to expose the PTY terminal routes.
    ///
    /// Disabled by default because the terminal is shell access.
    #[serde(default)]
    pub terminal_enabled: bool,
    /// Automatically orchestrate follow-up work when publish events arrive.
    #[serde(default = "default_true")]
    pub auto_orchestrate: bool,
    /// Authentication settings for `/api/*`.
    #[serde(default)]
    pub auth: ServeAuthConfig,
    /// Cloud deployment settings.
    #[serde(default)]
    pub deploy: ServeDeployConfig,
    /// Whether `roko` with no subcommand should auto-start the HTTP server.
    ///
    /// Disabled by default so the control plane is opt-in.
    #[serde(default)]
    pub auto_start: bool,
    /// Set to `true` to acknowledge the risk of a public bind without auth.
    ///
    /// Required when binding to a non-loopback address with `auth.enabled = false`.
    #[serde(default)]
    pub acknowledge_public_risk: bool,
    /// Additional remote IPs or CIDR ranges allowed to POST RuntimeEvent ingest without API auth.
    ///
    /// Loopback callers are always allowed. Public callers should normally use
    /// `serve.auth.enabled = true` instead of this allowlist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_ingest_allowlist: Vec<String>,
    /// Optional OTLP tracing export. Disabled when `otlp_endpoint` is absent.
    #[serde(default)]
    pub tracing: TracingConfig,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            port: None,
            share_ttl_days: default_share_ttl_days(),
            terminal_enabled: false,
            auto_orchestrate: true,
            auth: ServeAuthConfig::default(),
            deploy: ServeDeployConfig::default(),
            auto_start: false,
            acknowledge_public_risk: false,
            event_ingest_allowlist: Vec::new(),
            tracing: TracingConfig::default(),
        }
    }
}

fn default_share_ttl_days() -> u64 {
    7
}

/// Authentication settings for the HTTP API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeAuthConfig {
    /// Whether `/api/*` routes require an `X-Api-Key` header.
    #[serde(default)]
    pub enabled: bool,
    /// Shared API key expected in `X-Api-Key` (legacy single-key mode).
    #[serde(default)]
    pub api_key: String,
    /// Named API keys with scoped permissions (hashes stored in `.roko/api-keys.json`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_keys: Vec<ApiKeyEntry>,
    /// Privy application ID for JWT validation (Phase 1b -- stub only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privy_app_id: Option<String>,
}

impl Default for ServeAuthConfig {
    fn default() -> Self {
        Self {
            // Secure-by-default: `/api/*` requires an `X-Api-Key`. Local users
            // can opt back out via `serve.auth.enabled = false` in `roko.toml`,
            // which is what `roko init` writes for new workspaces.
            enabled: true,
            api_key: String::new(),
            api_keys: Vec::new(),
            privy_app_id: None,
        }
    }
}

/// A named API key entry with scoped permissions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    /// Human-readable name (e.g. "github-actions", "cli-default").
    pub name: String,
    /// SHA-256 hash of the plaintext key (hex-encoded).
    pub key_hash: String,
    /// Permission scope: "admin", "agent:write", "read", etc.
    #[serde(default = "default_api_key_scope")]
    pub scope: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// Optional ISO 8601 expiry timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

fn default_api_key_scope() -> String {
    "admin".into()
}

/// Cloud deployment settings attached to the API server configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeDeployConfig {
    /// Deployment provider, e.g. `"railway"` or `"fly"`.
    #[serde(default = "default_serve_deploy_provider")]
    pub provider: String,
    /// Environment variables that must be present for deployment.
    #[serde(default = "default_serve_deploy_environment")]
    pub environment: Vec<String>,
    /// Webhooks that should be registered after deploy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhooks: Vec<ServeDeployWebhookConfig>,
}

fn default_serve_deploy_provider() -> String {
    "railway".into()
}

fn default_serve_deploy_environment() -> Vec<String> {
    vec![
        "GITHUB_TOKEN".into(),
        "GITHUB_WEBHOOK_SECRET".into(),
        "SLACK_BOT_TOKEN".into(),
        "SLACK_SIGNING_SECRET".into(),
    ]
}

impl Default for ServeDeployConfig {
    fn default() -> Self {
        Self {
            provider: default_serve_deploy_provider(),
            environment: default_serve_deploy_environment(),
            webhooks: Vec::new(),
        }
    }
}

/// A webhook registration entry to run after deployment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeDeployWebhookConfig {
    /// Webhook provider.
    #[serde(default = "default_serve_deploy_webhook_provider")]
    pub provider: String,
    /// Repository owner.
    #[serde(default)]
    pub owner: String,
    /// Repository name.
    #[serde(default)]
    pub repo: String,
}

fn default_serve_deploy_webhook_provider() -> String {
    "github".into()
}

impl Default for ServeDeployWebhookConfig {
    fn default() -> Self {
        Self {
            provider: default_serve_deploy_webhook_provider(),
            owner: String::new(),
            repo: String::new(),
        }
    }
}

// ---- [serve.tracing] -----------------------------------------------------

/// Optional distributed tracing export configuration.
///
/// Parsed from `[serve.tracing]` in `roko.toml`. When `otlp_endpoint` is absent,
/// tracing export is disabled and no OTLP dependencies are loaded at runtime.
///
/// ```toml
/// [serve.tracing]
/// otlp_endpoint = "http://localhost:4317"
/// service_name = "roko-serve-dev"
/// sample_rate = 0.1
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TracingConfig {
    /// OTLP gRPC endpoint for trace export (e.g. `"http://localhost:4317"`).
    /// When absent, tracing export is disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otlp_endpoint: Option<String>,
    /// Service name reported in OTLP spans. Defaults to `"roko-serve"`.
    #[serde(default = "default_tracing_service_name")]
    pub service_name: String,
    /// Sample rate 0.0--1.0. Default 1.0 (trace everything).
    #[serde(default = "default_tracing_sample_rate")]
    pub sample_rate: f64,
}

fn default_tracing_service_name() -> String {
    "roko-serve".to_string()
}

fn default_tracing_sample_rate() -> f64 {
    1.0
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: None,
            service_name: default_tracing_service_name(),
            sample_rate: default_tracing_sample_rate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_share_ttl_days_is_seven() {
        assert_eq!(ServeConfig::default().share_ttl_days, 7);
    }

    #[test]
    fn default_acknowledge_public_risk_is_false() {
        assert!(!ServeConfig::default().acknowledge_public_risk);
    }

    #[test]
    fn default_auto_start_is_false() {
        assert!(!ServeConfig::default().auto_start);
    }

    #[test]
    fn default_auth_is_enabled() {
        assert!(ServeAuthConfig::default().enabled);
    }

    #[test]
    fn tracing_config_defaults_are_disabled() {
        let tc = TracingConfig::default();
        assert!(tc.otlp_endpoint.is_none());
        assert_eq!(tc.service_name, "roko-serve");
        assert!((tc.sample_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn serve_config_tracing_absent_uses_defaults() {
        let toml_text = "port = 8080\n";
        let cfg: ServeConfig = toml::from_str(toml_text).expect("parse serve config");
        assert!(cfg.tracing.otlp_endpoint.is_none());
        assert_eq!(cfg.tracing.service_name, "roko-serve");
        assert!((cfg.tracing.sample_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn serve_config_tracing_parses_full_block() {
        let toml_text = r#"
port = 6677

[tracing]
otlp_endpoint = "http://localhost:4317"
service_name = "roko-serve-dev"
sample_rate = 0.1
"#;
        let cfg: ServeConfig = toml::from_str(toml_text).expect("parse serve config");
        assert_eq!(
            cfg.tracing.otlp_endpoint.as_deref(),
            Some("http://localhost:4317")
        );
        assert_eq!(cfg.tracing.service_name, "roko-serve-dev");
        assert!((cfg.tracing.sample_rate - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn serve_config_tracing_sample_rate_defaults_to_one() {
        let toml_text = r#"
[tracing]
otlp_endpoint = "http://otel:4317"
"#;
        let cfg: ServeConfig = toml::from_str(toml_text).expect("parse serve config");
        assert!((cfg.tracing.sample_rate - 1.0).abs() < f64::EPSILON);
        assert_eq!(cfg.tracing.service_name, "roko-serve");
    }
}

// ---- [server] ------------------------------------------------------------

/// HTTP server / gateway settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address to bind to.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Port number.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Allowed CORS origins. Empty = local-only.
    #[serde(default)]
    pub cors_origins: Vec<String>,
    /// Optional bearer token for API authentication.
    #[serde(default)]
    pub auth_token: Option<String>,
    /// Allow all origins when `cors_origins` is empty.
    #[serde(default)]
    pub unsafe_public_cors: bool,
    /// Workspace GC interval in seconds. Defaults to 300 (5 minutes).
    #[serde(default = "default_workspace_gc_interval_secs")]
    pub workspace_gc_interval_secs: u64,
}

fn default_bind() -> String {
    "127.0.0.1".into()
}

fn default_workspace_gc_interval_secs() -> u64 {
    crate::defaults::DEFAULT_WORKSPACE_GC_INTERVAL_SECS
}

const fn default_port() -> u16 {
    6677
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            cors_origins: Vec::new(),
            auth_token: None,
            unsafe_public_cors: false,
            workspace_gc_interval_secs: default_workspace_gc_interval_secs(),
        }
    }
}

// ---- [deploy] ------------------------------------------------------------

/// Cloud deployment configuration.
///
/// ```toml
/// [deploy]
/// backend = "railway-api"
/// railway_api_token = "..."
/// project_id = "..."
/// environment_id = "..."
/// worker_image = "ghcr.io/example/roko-worker:latest"
/// default_region = "us-west1"
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeployConfig {
    /// Which deploy backend to use: `"railway-api"`, `"railway-cli"`, `"manual"`.
    #[serde(default = "default_deploy_backend")]
    pub backend: String,

    /// Railway API token (for the `railway-api` backend).
    #[serde(default)]
    pub railway_api_token: Option<String>,

    /// Railway project ID.
    #[serde(default)]
    pub project_id: Option<String>,

    /// Railway environment ID.
    #[serde(default)]
    pub environment_id: Option<String>,

    /// Docker image for worker containers.
    #[serde(default)]
    pub worker_image: Option<String>,

    /// Default region for deployments.
    #[serde(default)]
    pub default_region: Option<String>,
}

fn default_deploy_backend() -> String {
    "manual".into()
}

impl Default for DeployConfig {
    fn default() -> Self {
        Self {
            backend: default_deploy_backend(),
            railway_api_token: None,
            project_id: None,
            environment_id: None,
            worker_image: Some("ghcr.io/nunchi-trade/roko-worker:latest".into()),
            default_region: None,
        }
    }
}

// ---- [scheduler] ---------------------------------------------------------

/// Cron scheduler configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Cron jobs configured at startup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cron: Vec<SchedulerCronConfig>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { cron: Vec::new() }
    }
}

impl SchedulerConfig {
    /// Returns `true` when no cron jobs are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cron.is_empty()
    }
}

/// One cron job configuration entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerCronConfig {
    /// Human-readable schedule name.
    pub name: String,
    /// Standard cron expression.
    pub expression: String,
    /// Engram kind emitted when the schedule fires.
    pub signal_kind: String,
    /// Extra structured metadata included in the emitted signal body.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl Default for SchedulerCronConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            expression: String::new(),
            signal_kind: String::new(),
            metadata: serde_json::Value::Null,
        }
    }
}

// ---- [webhooks] ----------------------------------------------------------

/// Webhook ingress configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhooksConfig {
    /// GitHub webhook configuration.
    #[serde(default)]
    pub github: GithubWebhookConfig,
}

impl Default for WebhooksConfig {
    fn default() -> Self {
        Self {
            github: GithubWebhookConfig::default(),
        }
    }
}

/// GitHub webhook configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubWebhookConfig {
    /// Shared secret used to verify `X-Hub-Signature-256`.
    #[serde(default)]
    pub secret: String,
}

impl Default for GithubWebhookConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
        }
    }
}
