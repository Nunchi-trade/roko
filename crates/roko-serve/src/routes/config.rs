//! Configuration read/write endpoints.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use roko_core::config::schema::RokoConfig;
use roko_core::config::{LoadConfigError, load_config};

use crate::error::ApiError;
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
    mask_secret_fields(&mut value);
    Ok(Json(value))
}

/// `PUT /api/config` — merge partial config JSON into the current config,
/// then write the result to `roko.toml`.
async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(partial): Json<Value>,
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

    state.store_roko_config(updated);

    mask_secret_fields(&mut current);
    Ok(Json(current))
}

/// `POST /api/config/reload` — reload `roko.toml` from disk and swap it into
/// the live server state without a restart.
pub async fn reload_config(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ReloadResponse>, ApiError> {
    let new_config = load_config(&state.workdir).map_err(map_load_config_error)?;
    let warnings = validate_references(&new_config);

    state.store_roko_config(new_config);

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
        assert!(payload.warnings.is_empty());
        assert!(!payload.timestamp.is_empty());
    }
}
