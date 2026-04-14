//! Agent execution helper for direct CLI flows such as PRD/research/plan generation.
//!
//! Used by `roko prd`, `roko research`, and `roko plan generate` to invoke
//! an agent that can read/write files while preserving provider-aware routing,
//! safety scoping, resume threading, and learning-episode persistence.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context as _, Result};
use crate::agent_spawn::{SpawnAgentSpec, spawn_agent_scoped};
use roko_core::agent::ProviderKind;
use roko_core::agent::resolve_model;
use roko_core::{Body, ContentHash, Context, Engram, Kind};
use roko_learn::episode_logger::Episode;
use roko_learn::runtime_feedback::{CompletedRunInput, LearningRuntime};

/// Options for agent execution.
pub struct AgentExecOpts<'a> {
    /// The prompt to send to the agent.
    pub prompt: &'a str,
    /// Working directory for the agent.
    pub workdir: &'a Path,
    /// Model to use (e.g. "claude-sonnet-4-6"). If None, uses CLI default.
    pub model: Option<&'a str>,
    /// Reasoning effort label to pass to Claude.
    pub effort: Option<&'a str>,
    /// Additional system prompt to append.
    pub system_prompt: Option<&'a str>,
    /// Claude session id to resume, if any.
    pub resume_session: Option<&'a str>,
    /// Extra env vars for the child process (gateway config, etc).
    pub env_vars: &'a [(String, String)],
}

/// Episode metadata for agent execution paths that should persist learning data.
pub struct AgentExecEpisode<'a> {
    /// Logical task kind used for episode routing and summaries.
    pub task_kind: &'a str,
    /// Stable task identifier for the episode record.
    pub task_id: &'a str,
}

/// Run the configured direct agent path and return just the exit code.
///
/// Convenience wrapper around [`run_agent_capture`] for callers that
/// don't need the agent's text output.
pub async fn run_agent(opts: AgentExecOpts<'_>) -> Result<i32> {
    run_agent_capture(opts).await.map(|(code, _)| code)
}

/// Run the configured direct agent path, echo the output, and persist an episode.
pub async fn run_agent_logged(
    opts: AgentExecOpts<'_>,
    episode: AgentExecEpisode<'_>,
) -> Result<i32> {
    run_agent_capture_logged(opts, episode)
        .await
        .map(|(code, _)| code)
}

/// Run the configured direct agent path and return `(exit_code, output_text)`.
pub async fn run_agent_capture(opts: AgentExecOpts<'_>) -> Result<(i32, String)> {
    run_agent_capture_impl(opts, true, None).await
}

/// Run the configured direct agent path, echo the output, and persist an episode.
pub async fn run_agent_capture_logged(
    opts: AgentExecOpts<'_>,
    episode: AgentExecEpisode<'_>,
) -> Result<(i32, String)> {
    run_agent_capture_impl(opts, true, Some(episode)).await
}

/// Run the configured direct agent path and return `(exit_code, output_text)`
/// without echoing the agent's rendered output to stdout.
pub async fn run_agent_capture_silent(opts: AgentExecOpts<'_>) -> Result<(i32, String)> {
    run_agent_capture_impl(opts, false, None).await
}

async fn run_agent_capture_impl(
    opts: AgentExecOpts<'_>,
    echo_output: bool,
    episode: Option<AgentExecEpisode<'_>>,
) -> Result<(i32, String)> {
    let started = Instant::now();
    let mut routing_config = roko_core::config::load_config(opts.workdir)
        .with_context(|| format!("load routing config from {}", opts.workdir.display()))?;
    routing_config.apply_process_env();
    let routing_enabled = !routing_config.providers.is_empty() || !routing_config.models.is_empty();

    // Fail fast if the agent command is still the test-only default.
    // `"cat"` just echoes the prompt back, producing garbage output.
    if !routing_enabled {
        let cmd = command_from_config(opts.workdir).unwrap_or_default();
        if cmd == "cat" || cmd.is_empty() {
            anyhow::bail!(
                "agent command is {:?} (the test-only default). \
                 Set `command = \"claude\"` (or another agent CLI) in roko.toml under [agent], \
                 or re-run `roko init` to generate a working config.",
                if cmd.is_empty() { "cat" } else { &cmd }
            );
        }
    }
    let model = opts
        .model
        .map(str::to_string)
        .or_else(|| model_from_config(opts.workdir))
        .unwrap_or_else(|| {
            if routing_enabled {
                routing_config.agent.default_model.clone()
            } else {
                "claude-opus-4-6".to_string()
            }
        });
    let resolved = resolve_model(&routing_config, &model);
    let mut extra_args = Vec::new();
    if resolved.provider_kind == ProviderKind::ClaudeCli
        && let Some(session_id) = opts.resume_session
    {
        extra_args.push("--resume".to_string());
        extra_args.push(session_id.to_string());
    }
    let agent = spawn_agent_scoped(
        &routing_config,
        SpawnAgentSpec {
            model: model.clone(),
            command: routing_config.agent.command.clone(),
            timeout_ms: Some(600_000), // 10 min for plan generation / research tasks
            system_prompt: opts.system_prompt.map(str::to_string),
            cached_content: None,
            tools: None,
            mcp_config: None,
            working_dir: Some(opts.workdir.to_path_buf()),
            env: opts.env_vars.to_vec(),
            extra_args,
            effort: Some(opts.effort.unwrap_or("medium").to_string()),
            bare_mode: true,
            dangerously_skip_permissions: true,
            name: format!("{}:{model}", resolved.provider_kind.label()),
        },
        format!("create agent for model {model}"),
    )?;

    let prompt = Engram::builder(Kind::Prompt)
        .body(Body::text(opts.prompt))
        .build();
    let result = agent.run(&prompt, &Context::now()).await;

    let rendered = result.output.body.as_text().unwrap_or("").to_string();
    if echo_output && !rendered.is_empty() {
        print!("{rendered}");
    }

    let exit_code = i32::from(!result.success);
    if let Some(episode) = episode {
        persist_capture_episode(
            opts.workdir,
            resolved.provider_kind.label(),
            Some(&model),
            episode.task_kind,
            episode.task_id,
            opts.prompt,
            &rendered,
            exit_code == 0,
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            opts.resume_session,
        )
        .await?;
    }

    Ok((exit_code, rendered))
}

/// Read model from roko.toml config if available.
pub fn model_from_config(workdir: &Path) -> Option<String> {
    let config_path = workdir.join("roko.toml");
    if !config_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&config_path).ok()?;
    // Simple extraction — avoid pulling in full config parsing
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("model") {
            let rest = rest.trim().strip_prefix('=')?;
            let rest = rest.trim().trim_matches('"');
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Read agent command from roko.toml config if available.
pub fn command_from_config(workdir: &Path) -> Option<String> {
    let config_path = workdir.join("roko.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("command") {
            let rest = rest.trim().strip_prefix('=')?;
            let rest = rest.trim().trim_matches('"');
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Gateway env vars extracted from roko.toml agent.env.
pub struct GatewayEnv {
    /// Key-value pairs to set on child processes.
    pub vars: Vec<(String, String)>,
}

/// Load gateway env vars from roko.toml's agent.env entries.
/// Returns them as key-value pairs to pass to child processes (avoids unsafe `set_var`).
pub fn load_gateway_env(workdir: &Path) -> GatewayEnv {
    let mut vars = Vec::new();
    let config_path = workdir.join("roko.toml");
    if !config_path.exists() {
        return GatewayEnv { vars };
    }
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return GatewayEnv { vars };
    };
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.contains("ANTHROPIC_") {
            let inner = line.trim_matches(|c| c == '[' || c == ']');
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() == 2 {
                let key = parts[0].trim().trim_matches('"');
                let val = parts[1].trim().trim_matches('"');
                if !key.is_empty() {
                    vars.push((key.to_string(), val.to_string()));
                }
            }
        }
    }
    GatewayEnv { vars }
}

fn resolved_capture_model(agent_command: &str, model: Option<&str>) -> String {
    if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
        return model.to_string();
    }
    if agent_command.eq_ignore_ascii_case("claude") {
        "claude-opus-4-6".to_string()
    } else {
        "unknown-model".to_string()
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

fn build_capture_episode(
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

/// Persist a lightweight learning episode for a direct agent-exec CLI path.
pub async fn persist_capture_episode(
    workdir: &Path,
    agent_command: &str,
    model: Option<&str>,
    task_kind: &str,
    task_id: &str,
    prompt: &str,
    output: &str,
    success: bool,
    wall_time_ms: u64,
    resume_session: Option<&str>,
) -> Result<()> {
    let (episode, provider) = build_capture_episode(
        agent_command,
        model,
        task_kind,
        task_id,
        prompt,
        output,
        success,
        wall_time_ms,
        resume_session,
    );

    let mut runtime = LearningRuntime::open_under(workdir.join(".roko").join("memory"))
        .await
        .map_err(|e| anyhow::anyhow!("open learning runtime: {e}"))?;
    let distillation_workdir = workdir.to_path_buf();
    runtime.set_episode_completion_hook(move |episode| {
        roko_neuro::spawn_episode_distillation(distillation_workdir.clone(), episode);
    });

    let mut completed = CompletedRunInput::from_episode(episode);
    completed.provider = Some(provider);
    runtime
        .record_completed_run(completed)
        .await
        .map_err(|e| anyhow::anyhow!("record learning feedback: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_learn::episode_logger::EpisodeLogger;
    use tempfile::TempDir;

    #[tokio::test]
    async fn persist_capture_episode_records_memory_episode() {
        let tmp = TempDir::new().expect("tempdir");

        persist_capture_episode(
            tmp.path(),
            "claude",
            Some("claude-sonnet-4-6"),
            "prd-plan-generate",
            "prd:plan:demo",
            "prompt body",
            "output body",
            true,
            42,
            Some("sess-1"),
        )
        .await
        .expect("persist capture episode");

        let episodes_path = tmp
            .path()
            .join(".roko")
            .join("memory")
            .join("episodes.jsonl");
        let episodes = EpisodeLogger::read_all_lossy(&episodes_path).await.unwrap();
        assert_eq!(episodes.len(), 1);
        let episode = &episodes[0];
        assert_eq!(episode.agent_id, "claude");
        assert_eq!(episode.task_id, "prd:plan:demo");
        assert_eq!(episode.kind, "agent_turn");
        assert_eq!(episode.model, "claude-sonnet-4-6");
        assert!(episode.success);
        assert_eq!(
            episode.extra.get("task_kind"),
            Some(&serde_json::json!("prd-plan-generate"))
        );
        assert_eq!(
            episode.extra.get("task_category"),
            Some(&serde_json::json!("scaffolding"))
        );
        assert_eq!(
            episode.extra.get("plan_id"),
            Some(&serde_json::json!("demo"))
        );
    }
}
