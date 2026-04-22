//! Per-agent HTTP server with additive feature modules.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::future::Future;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use axum::{Router, middleware};
use tokio::net::TcpListener;
use tokio::time::MissedTickBehavior;
use tower_http::trace::TraceLayer;

use roko_agent::dispatcher::ToolDispatcher;
use roko_agent::tool_loop::LlmBackend;
use roko_chain::ChainClient;
use roko_neuro::KnowledgeStore;

pub mod auth;
pub mod features;
pub mod registration;
pub mod state;

pub use auth::bearer::BearerAuth;
pub use features::relay_client::RelayClientConfig;
pub use registration::{
    AgentCard, AgentCardEndpoints, AgentCardPublisher, AgentRegistration, RegistrationOutcome,
};
pub use state::{
    AgentMetrics, AgentPrediction, AgentPredictionResidual, AgentRuntimeStats, AgentState,
    DispatchError, DispatchLike, MessageContext, PredictionCreateRequest,
};

type BoxFutureResult = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
type StartHook = Arc<dyn Fn(SocketAddr, AgentCard) -> BoxFutureResult + Send + Sync>;

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
struct FeatureFlags {
    messaging: bool,
    predictions: bool,
    research: bool,
    tasks: bool,
}

/// Running agent server definition.
pub struct AgentServer {
    bind: String,
    state: Arc<AgentState>,
    auth: Option<BearerAuth>,
    features: FeatureFlags,
    on_start: Option<StartHook>,
    registration: Option<AgentRegistration>,
    /// Base URL of the roko-serve control plane for heartbeat reporting.
    serve_url: Option<String>,
    /// Heartbeat interval in seconds.
    heartbeat_interval_secs: u64,
}

impl AgentServer {
    /// Start a builder for a new agent server.
    #[must_use]
    pub fn builder() -> AgentServerBuilder {
        AgentServerBuilder::default()
    }

    /// Borrow the shared state used by this server.
    #[must_use]
    pub fn state(&self) -> Arc<AgentState> {
        Arc::clone(&self.state)
    }

    /// Build the axum router for this server.
    pub fn router(&self) -> Router {
        let public = Router::new().merge(features::health::router());
        let protected = self.protected_router();

        let protected = if let Some(auth) = self.auth.clone() {
            protected.layer(middleware::from_fn_with_state(
                auth,
                auth::bearer::require_bearer_auth,
            ))
        } else {
            protected
        };

        public
            .merge(protected)
            .layer(TraceLayer::new_for_http())
            .with_state(Arc::clone(&self.state))
    }

    fn protected_router(&self) -> Router<Arc<AgentState>> {
        let mut router = Router::new()
            .route("/stats", axum::routing::get(features::health::stats))
            .merge(features::logs::router());
        if self.features.messaging {
            router = router.merge(features::messaging::router());
        }
        if self.features.predictions {
            router = router.merge(features::predictions::router());
        }
        if self.features.research {
            router = router.merge(features::research::router());
        }
        if self.features.tasks {
            router = router.merge(features::tasks::router());
        }
        router
    }

    /// Bind the configured address and serve until shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error if the bind address cannot be resolved or the TCP
    /// listener cannot be created, if the bound address cannot be read back,
    /// if optional registration or start hooks fail, or if serving the Axum
    /// router fails.
    pub async fn serve(self) -> Result<()> {
        let bind = resolve_addr(&self.bind)?;
        let listener = TcpListener::bind(bind)
            .await
            .with_context(|| format!("bind agent server to {}", self.bind))?;
        let local_addr = listener
            .local_addr()
            .context("read bound agent-server address")?;

        let card = self.registration.as_ref().map_or_else(
            || self.state.build_agent_card(local_addr),
            |r| r.build_card(&self.state, local_addr),
        );

        if let Some(registration) = &self.registration {
            registration
                .register(Arc::clone(&self.state), local_addr)
                .await
                .context("register agent card")?;
        }
        if let Some(on_start) = &self.on_start {
            on_start(local_addr, card)
                .await
                .context("run on_start hook")?;
        }

        // Spawn heartbeat loop if a serve URL is configured.
        if let Some(serve_url) = &self.serve_url {
            let heartbeat_state = Arc::clone(&self.state);
            let heartbeat_url = format!("{}/api/heartbeats", serve_url.trim_end_matches('/'));
            let interval_secs = self.heartbeat_interval_secs;
            tokio::spawn(heartbeat_loop(
                heartbeat_state,
                heartbeat_url,
                interval_secs,
            ));
        }

        axum::serve(listener, self.router())
            .await
            .context("serve agent http router")
    }
}

/// Builder for [`AgentServer`].
#[derive(Default)]
pub struct AgentServerBuilder {
    bind: Option<String>,
    agent_id: Option<String>,
    log_path: Option<PathBuf>,
    owner: Option<String>,
    version: Option<String>,
    capabilities: Vec<String>,
    auth: Option<BearerAuth>,
    chain_client: Option<Arc<dyn ChainClient>>,
    llm_backend: Option<Arc<dyn LlmBackend>>,
    knowledge_store: Option<Arc<KnowledgeStore>>,
    dispatcher: Option<Arc<ToolDispatcher>>,
    message_dispatcher: Option<Arc<dyn DispatchLike>>,
    features: FeatureFlags,
    on_start: Option<StartHook>,
    registration: Option<AgentRegistration>,
    serve_url: Option<String>,
    heartbeat_interval_secs: Option<u64>,
}

impl AgentServerBuilder {
    /// Set the unique agent identifier.
    #[must_use]
    pub fn agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    /// Set the address to bind, for example `0.0.0.0:0`.
    #[must_use]
    pub fn bind(mut self, bind: impl Into<String>) -> Self {
        self.bind = Some(bind.into());
        self
    }

    /// Override the sidecar log path exposed by `GET /logs`.
    #[must_use]
    pub fn log_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.log_path = Some(path.into());
        self
    }

    /// Record the logical owner for dashboard surfaces.
    #[must_use]
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }

    /// Override the advertised agent-card version.
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Add a capability to the advertised manifest.
    #[must_use]
    pub fn capability(mut self, capability: impl Into<String>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    /// Attach bearer auth to all non-public routes.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn auth(mut self, auth: BearerAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Attach an optional chain client for downstream use.
    #[must_use]
    pub fn chain_client(mut self, client: Arc<dyn ChainClient>) -> Self {
        self.chain_client = Some(client);
        self
    }

    /// Attach an optional tool dispatcher for message handling.
    #[must_use]
    pub fn with_dispatcher(mut self, dispatcher: Arc<ToolDispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Attach an optional message dispatcher for the messaging routes.
    #[must_use]
    pub fn with_message_dispatcher(mut self, dispatcher: Arc<dyn DispatchLike>) -> Self {
        self.message_dispatcher = Some(dispatcher);
        self
    }

    /// Attach an optional LLM backend for future message handling.
    #[must_use]
    pub fn llm_backend(mut self, backend: Arc<dyn LlmBackend>) -> Self {
        self.llm_backend = Some(backend);
        self
    }

    /// Attach an optional knowledge store for research endpoints.
    #[must_use]
    pub fn knowledge_store(mut self, store: Arc<KnowledgeStore>) -> Self {
        self.knowledge_store = Some(store);
        self
    }

    /// Enable the messaging feature surface.
    #[must_use]
    pub fn messaging(mut self) -> Self {
        self.features.messaging = true;
        self.capabilities.push("messaging".to_string());
        self
    }

    /// Enable the predictions feature surface.
    #[must_use]
    pub fn predictions(mut self) -> Self {
        self.features.predictions = true;
        self.capabilities.push("predictions".to_string());
        self
    }

    /// Enable the research feature surface.
    #[must_use]
    pub fn research(mut self) -> Self {
        self.features.research = true;
        self.capabilities.push("research".to_string());
        self
    }

    /// Enable the task queue feature surface.
    #[must_use]
    pub fn tasks(mut self) -> Self {
        self.features.tasks = true;
        self.capabilities.push("tasks".to_string());
        self
    }

    /// Set the roko-serve control plane URL for heartbeat reporting.
    #[must_use]
    pub fn serve_url(mut self, url: impl Into<String>) -> Self {
        self.serve_url = Some(url.into());
        self
    }

    /// Override the heartbeat interval (default: 30 seconds).
    #[must_use]
    pub const fn heartbeat_interval(mut self, secs: u64) -> Self {
        self.heartbeat_interval_secs = Some(secs);
        self
    }

    /// Configure automatic agent-card publishing and optional registry updates.
    #[must_use]
    pub fn registration(mut self, registration: AgentRegistration) -> Self {
        self.registration = Some(registration);
        self
    }

    /// Register a best-effort callback invoked after the server knows its bound address.
    #[must_use]
    pub fn on_start<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(SocketAddr, AgentCard) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.on_start = Some(Arc::new(move |addr, card| Box::pin(hook(addr, card))));
        self
    }

    /// Finish building the server definition.
    ///
    /// # Errors
    ///
    /// Returns an error if `agent_id` was not configured.
    pub fn build(self) -> Result<AgentServer> {
        let agent_id = self
            .agent_id
            .ok_or_else(|| anyhow!("agent_id is required"))?;
        let bind = self.bind.unwrap_or_else(|| "0.0.0.0:0".to_string());
        let capabilities = normalize_capabilities(self.capabilities, self.features);
        let mut state = AgentState::new(
            agent_id,
            self.owner,
            self.version.unwrap_or_else(|| "0.1.0".to_string()),
            capabilities,
            self.chain_client,
            self.llm_backend,
            self.knowledge_store,
        );
        if let Some(log_path) = self.log_path {
            state = state.with_log_path(log_path);
        }
        if let Some(dispatcher) = self.dispatcher {
            state = state.with_dispatcher(dispatcher);
        }
        if let Some(dispatcher) = self.message_dispatcher {
            state = state.with_message_dispatcher(dispatcher);
        }
        let state = Arc::new(state);

        Ok(AgentServer {
            bind,
            state,
            auth: self.auth,
            features: self.features,
            on_start: self.on_start,
            registration: self.registration,
            serve_url: self.serve_url,
            heartbeat_interval_secs: self
                .heartbeat_interval_secs
                .unwrap_or(roko_core::DEFAULT_HEARTBEAT_INTERVAL_SECS),
        })
    }
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if !out.iter().any(|existing| existing == &value) {
            out.push(value);
        }
    }
    out
}

fn normalize_capabilities(values: Vec<String>, features: FeatureFlags) -> Vec<String> {
    dedupe(values)
        .into_iter()
        .filter(|value| capability_is_live(value, features))
        .collect()
}

fn capability_is_live(value: &str, features: FeatureFlags) -> bool {
    match value {
        "messaging" => features.messaging,
        "predictions" => features.predictions,
        "research" => features.research,
        "tasks" => features.tasks,
        _ => true,
    }
}

/// Background task that periodically POSTs a [`roko_core::HeartbeatPayload`] to
/// the roko-serve control plane so that the discovery registry stays fresh.
#[allow(clippy::cast_precision_loss)]
async fn heartbeat_loop(state: Arc<state::AgentState>, url: String, interval_secs: u64) {
    let client = ::reqwest::Client::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        let payload = roko_core::HeartbeatPayload {
            sender_id: state.agent_id().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            active_tasks: 0,
            completed_tasks: 0,
            failed_tasks: 0,
            active_agents: 1,
            frequency: 1.0 / interval_secs as f64,
            metrics: std::collections::HashMap::new(),
        };

        match client.post(&url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::trace!(
                    agent_id = state.agent_id(),
                    url = %url,
                    "heartbeat sent"
                );
            }
            Ok(resp) => {
                tracing::debug!(
                    agent_id = state.agent_id(),
                    status = %resp.status(),
                    "heartbeat rejected by control plane"
                );
            }
            Err(err) => {
                tracing::debug!(
                    agent_id = state.agent_id(),
                    error = %err,
                    "heartbeat send failed (control plane unreachable?)"
                );
            }
        }
    }
}

fn resolve_addr(bind: &str) -> Result<SocketAddr> {
    bind.to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("could not resolve bind address {bind}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_is_public_but_message_requires_auth() {
        let server = AgentServer::builder()
            .agent_id("agent-1")
            .messaging()
            .auth(BearerAuth::new("super-secret"))
            .build()
            .expect("server");

        let router = server.router();

        let health = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("health response");
        assert_eq!(health.status(), StatusCode::OK);

        let message = router
            .oneshot(
                Request::builder()
                    .uri("/message")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"prompt":"hello"}"#))
                    .expect("request"),
            )
            .await
            .expect("message response");
        assert_eq!(message.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn prediction_create_and_list_round_trip() {
        let server = AgentServer::builder()
            .agent_id("agent-1")
            .predictions()
            .build()
            .expect("server");
        let router = server.router();

        let create = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/predictions")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"market":"ETH-USD","direction":"up","confidence":0.75}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("prediction create");
        assert_eq!(create.status(), StatusCode::OK);

        let list = router
            .oneshot(
                Request::builder()
                    .uri("/predictions")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("prediction list");
        assert_eq!(list.status(), StatusCode::OK);
        let body = to_bytes(list.into_body(), usize::MAX)
            .await
            .expect("list body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload.as_array().map_or(0, Vec::len), 1);
    }

    #[tokio::test]
    async fn reserved_feature_capability_does_not_overclaim_routes() {
        let server = AgentServer::builder()
            .agent_id("agent-1")
            .capability("messaging")
            .capability("custom-skill")
            .build()
            .expect("server");
        let router = server.router();

        let capabilities = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/capabilities")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("capabilities response");
        assert_eq!(capabilities.status(), StatusCode::OK);
        let body = to_bytes(capabilities.into_body(), usize::MAX)
            .await
            .expect("capabilities body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");

        assert_eq!(payload["features"], json!(["custom-skill"]));
        assert_eq!(
            payload["routes"],
            json!(["/health", "/capabilities", "/stats", "/logs"])
        );
        assert_eq!(
            payload["skills"],
            json!({
                "custom-skill": {
                    "enabled": true,
                    "config": {}
                }
            })
        );

        let message = router
            .oneshot(
                Request::builder()
                    .uri("/message")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"prompt":"hello"}"#))
                    .expect("request"),
            )
            .await
            .expect("message response");
        assert_eq!(message.status(), StatusCode::NOT_FOUND);
    }
}
