//! Configuration read/write endpoints.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use roko_core::config::hot_reload;
use roko_core::config::schema::RokoConfig;
use roko_core::config::{LoadConfigError, load_config};

use crate::error::ApiError;
use crate::events::ServerEvent;
use crate::extract::ApiJson;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/config", get(get_config).put(update_config))
        .route("/config/reload", post(reload_config))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReloadResponse {
    pub success: bool,
    pub warnings: Vec<String>,
    pub timestamp: String,
}

/// `GET /api/config` — return the current `RokoConfig` as JSON.
async fn get_config(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let cfg = state.load_roko_config();
    let mut value = serde_json::to_value(cfg.as_ref())
        .map_err(|e| ApiError::internal(format!("serialize config: {e}")))?;
    expose_dashboard_config_fields(&mut value, cfg.as_ref());
    mask_secret_fields(&mut value);
    Ok(Json(value))
}

/// `PUT /api/config` — merge partial config JSON into the current config,
/// then write the result to `roko.toml`.
async fn update_config(
    State(state): State<Arc<AppState>>,
    ApiJson(partial): ApiJson<Value>,
) -> Result<Json<Value>, ApiError> {
    // Read current config, merge the partial update, and write back.
    let cfg = state.load_roko_config();

    // Serialize current to Value, deep-merge, then deserialize back.
    let mut current = serde_json::to_value(cfg.as_ref())
        .map_err(|e| ApiError::internal(format!("serialize current config: {e}")))?;

    merge_json(&mut current, &partial);

    let updated: RokoConfig = serde_json::from_value(current.clone())
        .map_err(|e| ApiError::bad_request(format!("invalid config after merge: {e}")))?;

    // Write to roko.toml.
    let toml_str = toml::to_string_pretty(&updated)
        .map_err(|e| ApiError::internal(format!("serialize toml: {e}")))?;
    let config_path = state.workdir.join("roko.toml");
    tokio::fs::write(&config_path, toml_str)
        .await
        .map_err(|e| ApiError::internal(format!("write roko.toml: {e}")))?;

    expose_dashboard_config_fields(&mut current, &updated);
    state.store_roko_config(updated);

    mask_secret_fields(&mut current);
    Ok(Json(current))
}

fn expose_dashboard_config_fields(value: &mut Value, config: &RokoConfig) {
    if let Some(map) = value.as_object_mut() {
        let default_model = config.agent.default_model.trim();
        map.insert(
            "default_model".to_string(),
            if default_model.is_empty() {
                Value::Null
            } else {
                Value::String(default_model.to_string())
            },
        );
    }
}

/// `POST /api/config/reload` — reload `roko.toml` from disk and swap it into
/// the live server state without a restart.
pub async fn reload_config(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ReloadResponse>, ApiError> {
    let warnings = reload_config_from_disk(&state).map_err(map_load_config_error)?;

    Ok(Json(ReloadResponse {
        success: true,
        warnings,
        timestamp: Utc::now().to_rfc3339(),
    }))
}

fn map_load_config_error(err: LoadConfigError) -> ApiError {
    match err {
        LoadConfigError::Read { .. } => ApiError::internal(err.to_string()),
        LoadConfigError::Parse { .. } => ApiError::bad_request(err.to_string()),
    }
}

/// Reload `roko.toml` from disk into the live state and return validation warnings.
///
/// Uses the hot-reload diff engine: sections classified as hot-reloadable
/// (`[budget]`, `[tools]`, `[learning]`, `[gates]`, etc.) are applied
/// immediately. Non-hot-reloadable sections (`[agent]`, `[providers]`,
/// `[models]`) log a warning suggesting a restart.
///
/// A `ConfigReloaded` event is published on the server event bus so
/// dashboards and SSE clients can react to config changes.
///
/// # Errors
///
/// Returns [`LoadConfigError::Read`] when `roko.toml` cannot be read and
/// [`LoadConfigError::Parse`] when the file contents are not valid config.
pub fn reload_config_from_disk(state: &AppState) -> Result<Vec<String>, LoadConfigError> {
    let new_config = load_config(&state.workdir)?;
    let mut warnings = validate_references(&new_config);

    // Compute diff and apply only hot-reloadable sections.
    let old_config = state.load_roko_config();
    let changes = hot_reload::config_diff(&old_config, &new_config);

    if changes.is_empty() {
        return Ok(warnings);
    }

    let mut current = old_config.as_ref().clone();
    let result = hot_reload::apply_hot_reload(&mut current, &new_config, &changes);

    // For non-hot-reloadable changes, we still store the full new config
    // so a subsequent restart picks up the new values. But we surface
    // the restart-required warning.
    for change in &result.needs_restart {
        warnings.push(format!(
            "{}: {} (requires restart)",
            change.summary,
            change.section.is_hot_reloadable(),
        ));
    }

    // Store the fully merged config (hot-reloaded + pending restart sections).
    state.store_roko_config(new_config);

    // Emit config-changed event for dashboard visibility.
    let applied_count = result.applied.len();
    let restart_count = result.needs_restart.len();
    if applied_count > 0 || restart_count > 0 {
        state.event_bus.publish(ServerEvent::ConfigReloaded {
            applied_sections: result.applied.iter().map(|c| c.summary.clone()).collect(),
            restart_required: result
                .needs_restart
                .iter()
                .map(|c| c.summary.clone())
                .collect(),
        });
    }

    Ok(warnings)
}

/// Reload STRATEGY.md from disk and return the parsed strategy document.
///
/// Returns `None` if the file does not exist or cannot be read.
pub fn reload_strategy_from_disk(state: &AppState) -> Option<hot_reload::StrategyDocument> {
    let strategy_path = state.workdir.join("STRATEGY.md");
    let content = std::fs::read_to_string(&strategy_path).ok()?;
    let doc = hot_reload::parse_strategy_md(&content);

    tracing::info!(
        goals = doc.goals.len(),
        tactics = doc.tactics.len(),
        risk_bounds = doc.risk_bounds.len(),
        "STRATEGY.md reloaded"
    );

    state.event_bus.publish(ServerEvent::StrategyReloaded {
        goals_count: doc.goals.len(),
        tactics_count: doc.tactics.len(),
    });

    Some(doc)
}

fn validate_references(config: &RokoConfig) -> Vec<String> {
    let providers = config.effective_providers();
    let mut warnings: Vec<String> = config
        .models
        .iter()
        .filter_map(|(model_key, profile)| {
            (!providers.contains_key(&profile.provider)).then(|| {
                format!(
                    "model `{model_key}` references provider `{}` which is not configured",
                    profile.provider
                )
            })
        })
        .collect();
    warnings.sort();
    warnings
}

/// Recursively merge `patch` into `base`. Object keys from `patch` override
/// those in `base`; everything else is replaced wholesale.
fn merge_json(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                let entry = base_map.entry(k.clone()).or_insert(Value::Null);
                merge_json(entry, v);
            }
        }
        (base, patch) => {
            *base = patch.clone();
        }
    }
}

fn mask_secret_fields(value: &mut Value) {
    mask_secret_field(
        value,
        &["serve", "auth"],
        "api_key",
        "ROKO_SERVE_AUTH_API_KEY",
    );
    mask_secret_field(value, &["server"], "auth_token", "ROKO_SERVER_AUTH_TOKEN");
    mask_secret_field(
        value,
        &["deploy"],
        "railway_api_token",
        "ROKO_DEPLOY_RAILWAY_API_TOKEN",
    );
}

fn mask_secret_field(value: &mut Value, path: &[&str], field: &str, env_var: &str) {
    let mut cursor = value;
    for key in path {
        let Some(next) = cursor.get_mut(*key) else {
            return;
        };
        cursor = next;
    }

    let Some(map) = cursor.as_object_mut() else {
        return;
    };

    if map.contains_key(field) {
        map.insert(field.to_string(), Value::String("***".to_string()));
        map.insert(
            format!("{field}_note"),
            Value::String(format!("Set `{env_var}` in the environment.")),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expose_dashboard_config_fields_adds_default_model() {
        let mut config = RokoConfig::default();
        config.agent.default_model = "claude-sonnet-4-20250514".into();
        let mut value = serde_json::to_value(&config).expect("serialize config");

        expose_dashboard_config_fields(&mut value, &config);

        assert_eq!(value["default_model"], "claude-sonnet-4-20250514");
        assert_eq!(value["agent"]["default_model"], "claude-sonnet-4-20250514");
    }

    #[test]
    fn mask_secret_fields_redacts_config_secrets() {
        let mut value = serde_json::json!({
            "serve": {
                "auth": {
                    "enabled": true,
                    "api_key": "secret"
                }
            },
            "server": {
                "auth_token": "server-secret"
            },
            "deploy": {
                "railway_api_token": "railway-secret"
            }
        });

        mask_secret_fields(&mut value);

        assert_eq!(value["serve"]["auth"]["api_key"], "***");
        assert_eq!(
            value["serve"]["auth"]["api_key_note"],
            "Set `ROKO_SERVE_AUTH_API_KEY` in the environment."
        );
        assert_eq!(value["server"]["auth_token"], "***");
        assert_eq!(
            value["server"]["auth_token_note"],
            "Set `ROKO_SERVER_AUTH_TOKEN` in the environment."
        );
        assert_eq!(value["deploy"]["railway_api_token"], "***");
        assert_eq!(
            value["deploy"]["railway_api_token_note"],
            "Set `ROKO_DEPLOY_RAILWAY_API_TOKEN` in the environment."
        );
    }

    #[tokio::test]
    async fn reload_config_reloads_state_from_disk() {
        use std::sync::Arc;

        use axum::body::{Body, to_bytes};
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        use crate::deploy::create_backend;
        use crate::runtime::NoOpRuntime;

        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let state = Arc::new(AppState::new(
            workdir.clone(),
            Arc::new(NoOpRuntime),
            RokoConfig::default(),
            deploy_backend,
        ));

        tokio::fs::write(
            workdir.join("roko.toml"),
            r#"
[server]
port = 4567
"#,
        )
        .await
        .expect("write roko.toml");

        let response = routes()
            .with_state(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/config/reload")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("reload response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.load_roko_config().server.port, 4567);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: ReloadResponse = serde_json::from_slice(&body).expect("parse reload response");
        assert!(payload.success);
        // [server] is a non-hot-reloadable section, so a restart warning is expected.
        assert!(
            payload.warnings.iter().any(|w| w.contains("server")),
            "expected a restart warning for [server] change, got: {:?}",
            payload.warnings
        );
        assert!(!payload.timestamp.is_empty());
    }
}
