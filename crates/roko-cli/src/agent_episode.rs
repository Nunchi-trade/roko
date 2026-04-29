//! Shared helpers for building lightweight CLI agent execution episodes.

use roko_core::ContentHash;
use roko_learn::episode_logger::Episode;

fn resolved_capture_model(agent_command: &str, model: Option<&str>) -> String {
    if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
        return model.to_string();
    }
    if agent_command.eq_ignore_ascii_case("claude") {
        "claude-opus-4-6".to_string()
    } else {
        String::new()
    }
}

fn capture_provider(agent_command: &str, resolved_model: &str) -> String {
    let command = agent_command.trim();
    let model = resolved_model.to_ascii_lowercase();
    if command.eq_ignore_ascii_case("claude") || model.starts_with("claude") {
        "anthropic".to_string()
    } else if command.eq_ignore_ascii_case("codex")
        || command.eq_ignore_ascii_case("openai")
        || model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
    {
        "openai".to_string()
    } else if command.eq_ignore_ascii_case("ollama") || model.starts_with("ollama/") {
        "ollama".to_string()
    } else {
        command.to_string()
    }
}

fn capture_role(task_kind: &str) -> &'static str {
    if task_kind.starts_with("research-") {
        "Researcher"
    } else {
        "Strategist"
    }
}

fn capture_task_category(task_kind: &str) -> &'static str {
    if task_kind.starts_with("research-") {
        "research"
    } else if task_kind.starts_with("prd-plan") || task_kind.starts_with("plan-") {
        "scaffolding"
    } else {
        "docs"
    }
}

fn capture_complexity_band(task_kind: &str) -> &'static str {
    if task_kind == "research-analyze" {
        "standard"
    } else if task_kind.starts_with("research-") {
        "deep"
    } else {
        "standard"
    }
}

fn capture_plan_id(task_id: &str) -> Option<&str> {
    task_id
        .rsplit(':')
        .next()
        .filter(|segment| !segment.is_empty())
}

/// Build a lightweight learning episode for a direct CLI agent execution.
#[allow(clippy::too_many_arguments)]
pub fn build_capture_episode(
    agent_command: &str,
    model: Option<&str>,
    task_kind: &str,
    task_id: &str,
    prompt: &str,
    output: &str,
    success: bool,
    wall_time_ms: u64,
    resume_session: Option<&str>,
) -> (Episode, String) {
    let resolved_model = resolved_capture_model(agent_command, model);
    let provider = capture_provider(agent_command, &resolved_model);
    let role = capture_role(task_kind);
    let task_category = capture_task_category(task_kind);
    let complexity_band = capture_complexity_band(task_kind);
    let mut episode = Episode::new(agent_command.to_string(), task_id.to_string());
    episode.kind = "agent_turn".to_string();
    episode.trigger_kind = task_kind.to_string();
    episode.agent_template = role.to_string();
    episode.episode_id = episode.id.clone();
    episode.model = resolved_model.clone();
    episode.input_signal_hash = ContentHash::of(prompt.as_bytes()).to_hex();
    episode.output_signal_hash = ContentHash::of(output.as_bytes()).to_hex();
    episode.duration_secs = wall_time_ms as f64 / 1000.0;
    episode.usage.wall_ms = wall_time_ms;
    episode.success = success;
    episode.turns = 1;
    if !success {
        episode.failure_reason = Some("agent returned non-zero exit code".to_string());
    }
    episode
        .extra
        .insert("role".to_string(), serde_json::json!(role));
    episode
        .extra
        .insert("command".to_string(), serde_json::json!(agent_command));
    episode
        .extra
        .insert("backend".to_string(), serde_json::json!(agent_command));
    episode
        .extra
        .insert("task_kind".to_string(), serde_json::json!(task_kind));
    episode
        .extra
        .insert("task_id".to_string(), serde_json::json!(task_id));
    episode
        .extra
        .insert("model".to_string(), serde_json::json!(resolved_model));
    episode
        .extra
        .insert("provider".to_string(), serde_json::json!(provider.clone()));
    episode.extra.insert(
        "task_category".to_string(),
        serde_json::json!(task_category),
    );
    episode.extra.insert(
        "complexity_band".to_string(),
        serde_json::json!(complexity_band),
    );
    if let Some(plan_id) = capture_plan_id(task_id) {
        episode
            .extra
            .insert("plan_id".to_string(), serde_json::json!(plan_id));
    }
    if let Some(session_id) = resume_session.filter(|value| !value.trim().is_empty()) {
        episode
            .extra
            .insert("session_id".to_string(), serde_json::json!(session_id));
    }
    episode.extra.insert(
        "prompt_chars".to_string(),
        serde_json::json!(prompt.chars().count()),
    );
    episode.extra.insert(
        "output_chars".to_string(),
        serde_json::json!(output.chars().count()),
    );
    episode
        .extra
        .insert("success".to_string(), serde_json::json!(success));
    (episode, provider)
}

#[cfg(test)]
mod tests {
    use super::build_capture_episode;

    #[test]
    fn build_capture_episode_maps_scaffolding_metadata() {
        let (episode, provider) = build_capture_episode(
            "claude",
            Some("claude-sonnet-4-6"),
            "prd-plan-generate",
            "prd:plan:demo",
            "prompt body",
            "output body",
            true,
            42,
            Some("sess-1"),
        );

        assert_eq!(provider, "anthropic");
        assert_eq!(episode.kind, "agent_turn");
        assert_eq!(episode.model, "claude-sonnet-4-6");
        assert_eq!(
            episode.extra.get("task_category"),
            Some(&serde_json::json!("scaffolding"))
        );
        assert_eq!(
            episode.extra.get("plan_id"),
            Some(&serde_json::json!("demo"))
        );
        assert_eq!(
            episode.extra.get("session_id"),
            Some(&serde_json::json!("sess-1"))
        );
    }
}
