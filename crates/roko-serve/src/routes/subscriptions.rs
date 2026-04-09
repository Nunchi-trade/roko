//! Subscription CRUD endpoints.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use roko_core::config::schema::SubscriptionConfig;

use crate::dispatch::Subscription;
use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/subscriptions",
            get(list_subscriptions).post(create_subscription),
        )
        .route(
            "/subscriptions/{id}",
            put(update_subscription).delete(delete_subscription),
        )
        .route("/subscriptions/{id}/enable", post(enable_subscription))
        .route("/subscriptions/{id}/disable", post(disable_subscription))
}

#[derive(Clone, Debug, Serialize)]
struct SubscriptionResponse {
    id: String,
    template: String,
    trigger: String,
    filter: roko_core::config::schema::SubscriptionFilterConfig,
    concurrency_limit: usize,
    cooldown_secs: u64,
    enabled: bool,
    status: &'static str,
}

impl From<&Subscription> for SubscriptionResponse {
    fn from(subscription: &Subscription) -> Self {
        Self {
            id: subscription.id.clone(),
            template: subscription.template.clone(),
            trigger: subscription.trigger.clone(),
            filter: subscription.filter.clone(),
            concurrency_limit: subscription.concurrency_limit,
            cooldown_secs: subscription.cooldown_secs,
            enabled: subscription.enabled,
            status: subscription_status(subscription.enabled),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SubscriptionUpdateRequest(SubscriptionConfig);

/// `GET /api/subscriptions` — list all subscriptions with their enabled status.
async fn list_subscriptions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let subscriptions = state.subscriptions.all();
    let items: Vec<SubscriptionResponse> = subscriptions
        .iter()
        .map(SubscriptionResponse::from)
        .collect();
    Ok(Json(json!({ "subscriptions": items })))
}

/// `POST /api/subscriptions` — create a new subscription file and register it.
async fn create_subscription(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SubscriptionConfig>,
) -> Result<impl IntoResponse, ApiError> {
    let config = validate_subscription(body)?;
    let id = next_subscription_id(&state, &config);
    let path = subscription_path(&state, &id);

    write_subscription_file(&path, &config).await?;

    let mut subscription = Subscription::from_config(config);
    subscription.id = id.clone();
    state.subscriptions.insert(subscription.clone());

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({ "subscription": SubscriptionResponse::from(&subscription) })),
    ))
}

/// `PUT /api/subscriptions/:id` — replace an existing subscription.
async fn update_subscription(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SubscriptionUpdateRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let config = validate_subscription(body.0)?;
    let path = subscription_path(&state, &id);

    if state.subscriptions.get_by_id(&id).is_none() {
        return Err(ApiError::not_found(format!(
            "subscription '{id}' not found"
        )));
    }

    write_subscription_file(&path, &config).await?;

    let mut subscription = Subscription::from_config(config);
    subscription.id = id.clone();
    let updated = state
        .subscriptions
        .update_by_id(&id, subscription)
        .ok_or_else(|| ApiError::not_found(format!("subscription '{id}' not found")))?;

    Ok(Json(
        json!({ "subscription": SubscriptionResponse::from(&updated) }),
    ))
}

/// `DELETE /api/subscriptions/:id` — remove the subscription file and registry entry.
async fn delete_subscription(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let path = subscription_path(&state, &id);
    let removed = state
        .subscriptions
        .remove_by_id(&id)
        .ok_or_else(|| ApiError::not_found(format!("subscription '{id}' not found")))?;

    if path.exists() {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| ApiError::internal(format!("remove subscription file: {e}")))?;
    }

    Ok(Json(
        json!({ "deleted": true, "subscription": SubscriptionResponse::from(&removed) }),
    ))
}

/// `POST /api/subscriptions/:id/enable` — mark a subscription enabled.
async fn enable_subscription(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    set_subscription_enabled(&state, &id, true).await
}

/// `POST /api/subscriptions/:id/disable` — mark a subscription disabled.
async fn disable_subscription(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    set_subscription_enabled(&state, &id, false).await
}

async fn set_subscription_enabled(
    state: &Arc<AppState>,
    id: &str,
    enabled: bool,
) -> Result<Json<serde_json::Value>, ApiError> {
    let path = subscription_path(state, id);
    let current = state
        .subscriptions
        .get_by_id(id)
        .ok_or_else(|| ApiError::not_found(format!("subscription '{id}' not found")))?;

    let mut config = current.to_config();
    config.enabled = enabled;
    write_subscription_file(&path, &config).await?;

    let mut updated = Subscription::from_config(config);
    updated.id = id.to_string();
    let updated = state
        .subscriptions
        .update_by_id(id, updated)
        .ok_or_else(|| ApiError::not_found(format!("subscription '{id}' not found")))?;

    Ok(Json(
        json!({ "subscription": SubscriptionResponse::from(&updated) }),
    ))
}

fn validate_subscription(config: SubscriptionConfig) -> Result<SubscriptionConfig, ApiError> {
    if config.template.trim().is_empty() {
        return Err(ApiError::bad_request(
            "subscription template must not be empty",
        ));
    }
    if config.trigger.trim().is_empty() {
        return Err(ApiError::bad_request(
            "subscription trigger must not be empty",
        ));
    }
    Ok(config)
}

fn subscription_status(enabled: bool) -> &'static str {
    if enabled { "enabled" } else { "disabled" }
}

fn subscription_path(state: &AppState, id: &str) -> std::path::PathBuf {
    state
        .workdir
        .join(".roko")
        .join("subscriptions")
        .join(format!("{id}.toml"))
}

fn next_subscription_id(state: &AppState, config: &SubscriptionConfig) -> String {
    let base = slugify_subscription_id(&config.template, &config.trigger);
    let mut candidate = base.clone();
    let mut suffix = 2usize;

    while state.subscriptions.get_by_id(&candidate).is_some()
        || subscription_path(state, &candidate).exists()
    {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }

    candidate
}

fn slugify_subscription_id(template: &str, trigger: &str) -> String {
    let mut slug = format!("{template}-{trigger}");
    slug = slug
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "subscription".to_string()
    } else {
        slug
    }
}

async fn write_subscription_file(
    path: &std::path::Path,
    config: &SubscriptionConfig,
) -> Result<(), ApiError> {
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::internal("invalid subscription path"))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| ApiError::internal(format!("create subscriptions dir: {e}")))?;

    let rendered = toml::to_string_pretty(config)
        .map_err(|e| ApiError::internal(format!("serialize subscription: {e}")))?;
    tokio::fs::write(path, rendered)
        .await
        .map_err(|e| ApiError::internal(format!("write subscription: {e}")))?;
    Ok(())
}
