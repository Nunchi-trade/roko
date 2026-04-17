//! OpenAPI surface for the roko HTTP server.
//!
//! The document is assembled here so the route handlers can stay focused on
//! behavior while this module tracks the public HTTP surface.
#![allow(missing_docs)]
#![allow(clippy::needless_for_each)]

use std::sync::Arc;

use axum::Json;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{OpenApi, ToSchema};

use crate::state::AppState;

/// Build the OpenAPI routes served under `/api`.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/openapi.json", get(openapi_json))
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "roko-serve API",
        version = env!("CARGO_PKG_VERSION"),
        description = "HTTP API exposed by roko-serve."
    ),
    servers((url = "/api")),
    tags(
        (name = "status", description = "Health, metrics, and dashboard endpoints"),
        (name = "plans", description = "Plan CRUD and execution"),
        (name = "run", description = "Single prompt execution endpoints"),
        (name = "templates", description = "Template CRUD and deploy endpoints"),
        (name = "deployments", description = "Cloud deployment endpoints"),
        (name = "agents", description = "Agent registration and lifecycle endpoints"),
        (name = "research", description = "Research and enhancement endpoints"),
        (name = "config", description = "Configuration endpoints"),
        (name = "subscriptions", description = "Subscription endpoints"),
        (name = "prds", description = "PRD endpoints"),
        (name = "webhooks", description = "Webhook ingress endpoints"),
        (name = "providers", description = "Provider and routing endpoints"),
        (name = "learning", description = "Learning and cascade endpoints"),
        (name = "aggregator", description = "Aggregation and knowledge endpoints"),
        (name = "diagnosis", description = "Diagnosis endpoints")
    ),
    paths(
        health,
        session_status,
        metrics_summary,
        dashboard,
        episodes,
        signals,
        operation_status,
        list_plans,
        get_plan,
        create_plan,
        execute_plan,
        plan_status,
        generate_plan,
        start_run,
        run_status,
        list_templates,
        create_template,
        get_template,
        delete_template,
        deploy_template,
        list_deployments,
        get_deployment,
        get_deployment_logs,
        proxy_task,
        receive_callback,
        list_managed_agents,
        register_agent,
        get_agent,
        stop_agent,
        agent_episodes,
        proxy_agent_logs,
        send_message,
        token_status,
        issue_token,
        list_research,
        research_topic,
        enhance_prd,
        enhance_plan,
        enhance_tasks,
        analyze,
        get_config,
        update_config,
        reload_config,
        list_subscriptions,
        create_subscription,
        update_subscription,
        delete_subscription,
        enable_subscription,
        disable_subscription,
        list_prds,
        post_idea,
        get_prd,
        draft_prd,
        promote_prd,
        plan_from_prd,
        github_webhook,
        slack_webhook,
        generic_webhook,
        list_providers,
        provider_health,
        test_provider,
        list_models,
        explain_routing,
        diagnosis_recent,
        learning_efficiency,
        learning_cascade_router,
        learning_cascade,
        learning_cost_tiers,
        learning_experiments,
        learning_adaptive_thresholds,
        learning_gate_thresholds,
        list_agents,
        agent_topology,
        agent_stats,
        agent_skills,
        agent_heartbeat,
        agent_trace,
        list_prediction_sessions,
        get_prediction_session,
        list_prediction_claims,
        list_knowledge_entries,
        list_knowledge_edges,
        search_knowledge,
        list_knowledge_kinds,
        list_tasks,
        task_stats,
        get_task
    ),
    components(schemas(
        ApiErrorResponse,
        HealthResponse,
        SessionStatusResponse,
        ReloadResponse,
        IdResponse,
        NameResponse,
        OperationResponse,
        DeploymentCreateRequest,
        DeploymentCreateResponse,
        PlanCreateRequest,
        PlanCreateTask,
        RunRequest,
        TemplateCreateRequest,
        TemplateDeployRequest,
        AgentRegisterRequest,
        AgentMessageRequest,
        TopicRequest,
        ConfigUpdateRequest,
        SubscriptionCreateRequest,
        SubscriptionUpdateRequest,
        PrdIdeaRequest,
        DeploymentCallbackRequest,
        WebhookPayload,
        SearchQueryRequest
    ))
)]
struct ApiDoc;

macro_rules! doc_get {
    ($name:ident, $path:literal, $tag:literal) => {
        #[utoipa::path(
            get,
            path = $path,
            tag = $tag,
            responses(
                (status = 200, description = "Successful response", body = Value),
                (status = 400, description = "Bad request", body = ApiErrorResponse),
                (status = 404, description = "Not found", body = ApiErrorResponse),
                (status = 500, description = "Internal error", body = ApiErrorResponse)
            )
        )]
        fn $name() {}
    };
}

macro_rules! doc_get_param {
    ($name:ident, $path:literal, $tag:literal, $param:literal) => {
        #[utoipa::path(
            get,
            path = $path,
            tag = $tag,
            params(($param = String, Path, description = "Path parameter")),
            responses(
                (status = 200, description = "Successful response", body = Value),
                (status = 400, description = "Bad request", body = ApiErrorResponse),
                (status = 404, description = "Not found", body = ApiErrorResponse),
                (status = 500, description = "Internal error", body = ApiErrorResponse)
            )
        )]
        fn $name() {}
    };
}

macro_rules! doc_post_value {
    ($name:ident, $path:literal, $tag:literal) => {
        #[utoipa::path(
            post,
            path = $path,
            tag = $tag,
            request_body = Value,
            responses(
                (status = 200, description = "Successful response", body = Value),
                (status = 201, description = "Created", body = Value),
                (status = 202, description = "Accepted", body = Value),
                (status = 400, description = "Bad request", body = ApiErrorResponse),
                (status = 401, description = "Unauthorized", body = ApiErrorResponse),
                (status = 404, description = "Not found", body = ApiErrorResponse),
                (status = 409, description = "Conflict", body = ApiErrorResponse),
                (status = 500, description = "Internal error", body = ApiErrorResponse)
            )
        )]
        fn $name() {}
    };
}

macro_rules! doc_put_value {
    ($name:ident, $path:literal, $tag:literal) => {
        #[utoipa::path(
            put,
            path = $path,
            tag = $tag,
            request_body = Value,
            responses(
                (status = 200, description = "Successful response", body = Value),
                (status = 400, description = "Bad request", body = ApiErrorResponse),
                (status = 404, description = "Not found", body = ApiErrorResponse),
                (status = 500, description = "Internal error", body = ApiErrorResponse)
            )
        )]
        fn $name() {}
    };
}

macro_rules! doc_delete {
    ($name:ident, $path:literal, $tag:literal) => {
        #[utoipa::path(
            delete,
            path = $path,
            tag = $tag,
            params(("id" = String, Path, description = "Path parameter")),
            responses(
                (status = 200, description = "Successful response", body = Value),
                (status = 400, description = "Bad request", body = ApiErrorResponse),
                (status = 404, description = "Not found", body = ApiErrorResponse),
                (status = 500, description = "Internal error", body = ApiErrorResponse)
            )
        )]
        fn $name() {}
    };
}

doc_get!(health, "/health", "status");
doc_get!(session_status, "/status", "status");
doc_get!(metrics_summary, "/metrics/summary", "status");
doc_get!(dashboard, "/dashboard", "status");
doc_get!(episodes, "/episodes", "status");
doc_get!(signals, "/signals", "status");
doc_get_param!(operation_status, "/operations/{id}", "status", "id");

doc_get!(list_plans, "/plans", "plans");
doc_get_param!(get_plan, "/plans/{id}", "plans", "id");
doc_post_value!(create_plan, "/plans", "plans");
doc_post_value!(execute_plan, "/plans/{id}/execute", "plans");
doc_get_param!(plan_status, "/plans/{id}/status", "plans", "id");
doc_post_value!(generate_plan, "/plans/generate", "plans");

doc_post_value!(start_run, "/run", "run");
doc_get_param!(run_status, "/run/{id}/status", "run", "id");

doc_get!(list_templates, "/templates", "templates");
doc_post_value!(create_template, "/templates", "templates");
doc_get_param!(get_template, "/templates/{name}", "templates", "name");
doc_get_param!(delete_template, "/templates/{name}", "templates", "name");
doc_post_value!(deploy_template, "/templates/{name}/deploy", "templates");

doc_get!(list_deployments, "/deployments", "deployments");
doc_get_param!(get_deployment, "/deployments/{id}", "deployments", "id");
doc_get_param!(get_deployment_logs, "/deployments/{id}/logs", "deployments", "id");
doc_post_value!(proxy_task, "/deployments/{id}/task", "deployments");
doc_post_value!(receive_callback, "/deployments/{id}/callback", "deployments");

doc_get!(list_managed_agents, "/managed-agents", "agents");
doc_post_value!(register_agent, "/agents/register", "agents");
doc_get_param!(get_agent, "/agents/{id}", "agents", "id");
doc_post_value!(stop_agent, "/agents/{id}/stop", "agents");
doc_get_param!(agent_episodes, "/agents/{id}/episodes", "agents", "id");
doc_get_param!(proxy_agent_logs, "/agents/{id}/logs", "agents", "id");
doc_post_value!(send_message, "/agents/{id}/message", "agents");
doc_get_param!(token_status, "/agents/{id}/token", "agents", "id");
doc_post_value!(issue_token, "/agents/{id}/token", "agents");

doc_get!(list_research, "/research", "research");
doc_post_value!(research_topic, "/research/topic", "research");
doc_post_value!(enhance_prd, "/research/enhance-prd/{slug}", "research");
doc_post_value!(enhance_plan, "/research/enhance-plan/{plan}", "research");
doc_post_value!(enhance_tasks, "/research/enhance-tasks/{plan}", "research");
doc_post_value!(analyze, "/research/analyze", "research");

doc_get!(get_config, "/config", "config");
doc_put_value!(update_config, "/config", "config");
doc_post_value!(reload_config, "/config/reload", "config");

doc_get!(list_subscriptions, "/subscriptions", "subscriptions");
doc_post_value!(create_subscription, "/subscriptions", "subscriptions");
doc_put_value!(update_subscription, "/subscriptions/{id}", "subscriptions");
doc_delete!(delete_subscription, "/subscriptions/{id}", "subscriptions");
doc_post_value!(enable_subscription, "/subscriptions/{id}/enable", "subscriptions");
doc_post_value!(disable_subscription, "/subscriptions/{id}/disable", "subscriptions");

doc_get!(list_prds, "/prds", "prds");
doc_post_value!(post_idea, "/prds/ideas", "prds");
doc_get_param!(get_prd, "/prds/{slug}", "prds", "slug");
doc_post_value!(draft_prd, "/prds/{slug}/draft", "prds");
doc_post_value!(promote_prd, "/prds/{slug}/promote", "prds");
doc_post_value!(plan_from_prd, "/prds/{slug}/plan", "prds");

doc_post_value!(github_webhook, "/webhooks/github", "webhooks");
doc_post_value!(slack_webhook, "/webhooks/slack", "webhooks");
doc_post_value!(generic_webhook, "/webhooks/generic", "webhooks");

doc_get!(list_providers, "/providers", "providers");
doc_get_param!(provider_health, "/providers/{id}/health", "providers", "id");
doc_post_value!(test_provider, "/providers/{id}/test", "providers");
doc_get!(list_models, "/models", "providers");
doc_get!(explain_routing, "/routing/explain", "providers");

doc_get!(diagnosis_recent, "/diagnosis/recent", "diagnosis");

doc_get!(learning_efficiency, "/learning/efficiency", "learning");
doc_get!(learning_cascade_router, "/learning/cascade-router", "learning");
doc_get!(learning_cascade, "/learning/cascade", "learning");
doc_get!(learning_cost_tiers, "/learning/cost-tiers", "learning");
doc_get!(learning_experiments, "/learning/experiments", "learning");
doc_get!(learning_adaptive_thresholds, "/learning/adaptive-thresholds", "learning");
doc_get!(learning_gate_thresholds, "/learning/gate-thresholds", "learning");

doc_get!(list_agents, "/agents", "aggregator");
doc_get!(agent_topology, "/agents/topology", "aggregator");
doc_get_param!(agent_stats, "/agents/{id}/stats", "aggregator", "id");
doc_get_param!(agent_skills, "/agents/{id}/skills", "aggregator", "id");
doc_get_param!(agent_heartbeat, "/agents/{id}/heartbeat", "aggregator", "id");
doc_get_param!(agent_trace, "/agents/{id}/trace", "aggregator", "id");
doc_get!(list_prediction_sessions, "/predictions/sessions", "aggregator");
doc_get_param!(
    get_prediction_session,
    "/predictions/sessions/{id}",
    "aggregator",
    "id"
);
doc_get!(list_prediction_claims, "/predictions/claims", "aggregator");
doc_get!(list_knowledge_entries, "/knowledge/entries", "aggregator");
doc_get!(list_knowledge_edges, "/knowledge/edges", "aggregator");
doc_get!(search_knowledge, "/knowledge/search", "aggregator");
doc_get!(list_knowledge_kinds, "/knowledge/kinds", "aggregator");
doc_get!(list_tasks, "/tasks", "aggregator");
doc_get!(task_stats, "/tasks/stats", "aggregator");
doc_get_param!(get_task, "/tasks/{id}", "aggregator", "id");

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorResponse {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct IdResponse {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NameResponse {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_secs: u64,
    pub active_plans: usize,
    pub active_agents: usize,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SessionStatusResponse {
    pub session_id: Option<String>,
    pub workdir: String,
    pub daemon_running: bool,
    pub signal_count: usize,
    pub episode_count: usize,
    pub last_episode_passed: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReloadResponse {
    pub success: bool,
    pub warnings: Vec<String>,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OperationResponse {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PlanCreateRequest {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub tasks: Vec<PlanCreateTask>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PlanCreateTask {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RunRequest {
    pub prompt: String,
    #[serde(default)]
    pub workdir: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TemplateCreateRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TemplateDeployRequest {
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub backend: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeploymentCreateRequest {
    pub template: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub backend: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeploymentCreateResponse {
    pub id: String,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AgentRegisterRequest {
    pub agent_id: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AgentMessageRequest {
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TopicRequest {
    pub topic: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ConfigUpdateRequest {
    #[serde(flatten)]
    pub value: Value,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SubscriptionCreateRequest {
    #[serde(flatten)]
    pub value: Value,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SubscriptionUpdateRequest {
    #[serde(flatten)]
    pub value: Value,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PrdIdeaRequest {
    pub idea: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeploymentCallbackRequest {
    #[serde(flatten)]
    pub value: Value,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WebhookPayload {
    #[serde(flatten)]
    pub value: Value,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SearchQueryRequest {
    #[serde(default)]
    pub q: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use roko_core::config::ServeAuthConfig;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::deploy::create_backend;
    use crate::routes::build_router;
    use crate::runtime::NoOpRuntime;
    use crate::state::AppState;

    #[tokio::test]
    async fn openapi_endpoint_is_served_under_api() {
        let dir = tempdir().expect("tempdir");
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(AppState::new(
            dir.path().to_path_buf(),
            Arc::new(NoOpRuntime),
            roko_core::config::schema::RokoConfig::default(),
            deploy_backend,
        ));

        let app = build_router(Arc::clone(&state), &[], ServeAuthConfig::default());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/openapi.json")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: Value = serde_json::from_slice(&body).expect("parse response body");
        assert_eq!(payload["openapi"], "3.1.0");
        assert!(payload["paths"]["/plans"].is_object());
    }
}
