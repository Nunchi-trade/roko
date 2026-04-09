//! Configuration read/write endpoints.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

use roko_core::config::schema::RokoConfig;

use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/config", get(get_config).put(update_config))
}

/// `GET /api/config` — return the current `RokoConfig` as JSON.
async fn get_config(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let cfg = state.roko_config.read().await;
    let mut value = serde_json::to_value(&*cfg)
        .map_err(|e| ApiError::internal(format!("serialize config: {e}")))?;
    mask_secret_fields(&mut value);
    drop(cfg);
    Ok(Json(value))
}

/// `PUT /api/config` — merge partial config JSON into the current config,
/// then write the result to `roko.toml`.
async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(partial): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    // Read current config, merge the partial update, and write back.
    let mut cfg = state.roko_config.write().await;

    // Serialize current to Value, deep-merge, then deserialize back.
    let mut current = serde_json::to_value(&*cfg)
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

    *cfg = updated;
    drop(cfg);

    mask_secret_fields(&mut current);
    Ok(Json(current))
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
}
