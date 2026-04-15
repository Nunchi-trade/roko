//! Agent skill configuration endpoints.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
};
use tracing::warn;

use super::{ApiError, ApiState};
use crate::chain::agent::{AgentStats, SkillConfig};

/// `GET /api/agents/{id}/skills` — return all configured skills for an agent.
pub async fn get_agent_skills(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let chain = state.chain.read();
    if chain.agent_registry.get_agent(&id).is_none() {
        return Err(not_found(&id));
    }

    let skills = chain
        .agent_registry
        .get_skills(&id)
        .cloned()
        .unwrap_or_default();

    Ok(Json(serde_json::json!({
        "agent_id": id,
        "skills": skills,
    })))
}

/// `PUT /api/agents/{id}/skills` — replace all skill configs for an agent.
pub async fn update_agent_skills(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(skills): Json<HashMap<String, SkillConfig>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    validate_skill_set(&skills)?;

    let mut chain = state.chain.write();
    if !chain.agent_registry.set_skills(&id, skills.clone()) {
        return Err(not_found(&id));
    }

    notify_skill_change(&mut chain, &id);

    Ok(Json(serde_json::json!({
        "ok": true,
        "agent_id": id,
        "skills_updated": skills.len(),
    })))
}

/// `PUT /api/agents/{id}/skills/{skill}` — upsert a single skill config.
pub async fn update_single_skill(
    State(state): State<ApiState>,
    Path((id, skill)): Path<(String, String)>,
    Json(config): Json<SkillConfig>,
) -> Result<Json<serde_json::Value>, ApiError> {
    validate_skill_config(&skill, &config)?;

    let mut chain = state.chain.write();
    if !chain.agent_registry.set_skill(&id, &skill, config) {
        return Err(not_found(&id));
    }

    notify_skill_change(&mut chain, &id);

    Ok(Json(serde_json::json!({
        "ok": true,
        "agent_id": id,
        "skill": skill,
    })))
}

fn notify_skill_change(chain: &mut crate::chain_rpc::ChainContext, agent_id: &str) {
    let _ = chain.agent_bus.send(crate::chain::AgentEvent::Stats {
        agent_id: agent_id.to_owned(),
        delta: AgentStats::default(),
    });
}

fn not_found(agent_id: &str) -> ApiError {
    ApiError {
        error: format!("agent '{agent_id}' not found"),
        code: 404,
    }
}

fn validate_skill_set(skills: &HashMap<String, SkillConfig>) -> Result<(), ApiError> {
    for (skill, config) in skills {
        validate_skill_config(skill, config)?;
    }

    let inventory_managers = ["market-maker", "hedge-agent"]
        .into_iter()
        .filter(|skill| skills.get(*skill).is_some_and(|config| config.enabled))
        .count();
    if inventory_managers > 1 {
        warn!("multiple inventory-managing skills enabled for one agent");
    }

    Ok(())
}

fn validate_skill_config(skill: &str, config: &SkillConfig) -> Result<(), ApiError> {
    for (key, value) in &config.config {
        match key.as_str() {
            "gamma_interval_s" | "check_interval_s" => {
                let interval = value
                    .as_u64()
                    .ok_or_else(|| invalid(skill, key, "must be an integer"))?;
                if interval < 1 {
                    return Err(invalid(skill, key, "must be >= 1"));
                }
            }
            "confidence_threshold" => {
                let threshold = value
                    .as_u64()
                    .ok_or_else(|| invalid(skill, key, "must be an integer"))?;
                if threshold > 100 {
                    return Err(invalid(skill, key, "must be between 0 and 100"));
                }
            }
            "divergence_bps" => {
                let non_negative = value.as_u64().is_some()
                    || value.as_i64().is_some_and(|n| n >= 0)
                    || value.as_f64().is_some_and(|n| n >= 0.0);
                if !non_negative {
                    return Err(invalid(skill, key, "must be a non-negative number"));
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn invalid(skill: &str, field: &str, message: &str) -> ApiError {
    ApiError {
        error: format!("skill '{skill}' field '{field}' {message}"),
        code: 422,
    }
}
