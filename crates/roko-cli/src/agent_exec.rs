//! Agent execution helper for direct CLI flows such as PRD/research/plan generation.
//!
//! Used by `roko prd`, `roko research`, and `roko plan generate` to invoke
//! an agent that can read/write files while preserving provider-aware routing,
//! safety scoping, resume threading, and learning-episode persistence.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::agent_config::{command_from_config, model_from_config};
use crate::agent_episode::build_capture_episode;
use crate::agent_spawn::{SpawnAgentSpec, spawn_agent_scoped};
use crate::learning_helpers::{
    capture_runtime_model_slugs, distillation_model_caller, provider_id_for_model,
    record_persisted_provider_health, resolve_capture_model_slug,
};
use anyhow::{Context as _, Result};
use roko_core::agent::ProviderKind;
use roko_core::agent::resolve_model;
use roko_core::{Body, Context, Engram, Kind};
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
    /// Logical role used to scope safety policies and model routing.
    ///
    /// When set, the safety layer applies role-specific policies and the
    /// CascadeRouter can make role-aware model selection decisions.
    pub role: Option<&'a str>,
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
    let routing_config = roko_core::config::loader::load_config_unified(opts.workdir)
        .with_context(|| format!("load routing config from {}", opts.workdir.display()))?;
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
            role: opts.role.map(str::to_string),
        },
        format!("create agent for model {model}"),
    )?;

    let prompt = Engram::builder(Kind::Prompt)
        .body(Body::text(opts.prompt))
        .build();
    tracing::info!(
        model = %model,
        role = ?opts.role,
        provider = %resolved.provider_kind.label(),
        prompt_len = opts.prompt.len(),
        "agent_exec: dispatching prompt"
    );
    let result = agent.run(&prompt, &Context::now()).await;

    let rendered = result.output.body.as_text().unwrap_or("").to_string();
    let elapsed_ms = started.elapsed().as_millis();
    tracing::info!(
        model = %model,
        success = result.success,
        output_len = rendered.len(),
        output_empty = rendered.trim().is_empty(),
        elapsed_ms = elapsed_ms,
        "agent_exec: agent returned"
    );
    if rendered.trim().is_empty() {
        tracing::warn!(
            model = %model,
            role = ?opts.role,
            "agent_exec: agent returned empty output text"
        );
    }
    if echo_output && !rendered.is_empty() {
        print!("{rendered}");
    }

    let exit_code = i32::from(!result.success);
    if let Some(episode) = episode {
        persist_capture_episode(
            opts.workdir,
            resolved.provider_kind.label(),
            Some(&resolved.slug),
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
    let config = roko_core::config::loader::load_config_unified(workdir).unwrap_or_default();
    let capture_model = resolve_capture_model_slug(&config, model);
    let provider_from_config = capture_model
        .as_deref()
        .and_then(|model_slug| provider_id_for_model(&config, model_slug))
        .or_else(|| model.and_then(|model_key| provider_id_for_model(&config, model_key)));

    let episode_model = capture_model.as_deref().or(model);
    let (mut episode, fallback_provider) = build_capture_episode(
        agent_command,
        episode_model,
        task_kind,
        task_id,
        prompt,
        output,
        success,
        wall_time_ms,
        resume_session,
    );
    let provider = provider_from_config.unwrap_or(fallback_provider);
    if !provider.trim().is_empty() {
        episode
            .extra
            .insert("provider".to_string(), serde_json::json!(provider.clone()));
    }

    let learn_root = workdir.join(".roko").join("learn");
    let model_slugs = capture_runtime_model_slugs(&config, episode.model.as_str());
    let mut runtime = if model_slugs.is_empty() {
        LearningRuntime::open_under(&learn_root).await
    } else {
        LearningRuntime::open_under_with_models(&learn_root, model_slugs).await
    }
    .map_err(|e| anyhow::anyhow!("open learning runtime: {e}"))?;
    let distillation_workdir = workdir.to_path_buf();
    let distillation_caller = distillation_model_caller(workdir);
    runtime.set_episode_completion_hook(move |episode| {
        roko_neuro::spawn_episode_distillation(
            distillation_workdir.clone(),
            episode,
            Some(Arc::clone(&distillation_caller)),
        );
    });

    let mut completed = CompletedRunInput::from_episode(episode);
    completed.provider = (!provider.trim().is_empty()).then_some(provider.clone());
    runtime
        .record_completed_run(completed)
        .await
        .map_err(|e| anyhow::anyhow!("record learning feedback: {e}"))?;
    record_persisted_provider_health(workdir, &provider, success)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_learn::episode_logger::EpisodeLogger;
    use tempfile::TempDir;

    #[tokio::test]
    async fn persist_capture_episode_records_learning_episode() {
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
            .join("learn")
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
        assert!(
            !tmp.path()
                .join(".roko")
                .join("memory")
                .join("episodes.jsonl")
                .exists()
        );
    }

    #[tokio::test]
    async fn persist_capture_episode_resolves_model_key_to_slug_and_provider() {
        let tmp = TempDir::new().expect("tempdir");
        std::fs::write(
            tmp.path().join("roko.toml"),
            r#"
[agent]
default_model = "glm-mini"
command = "claude"

[providers.zai]
kind = "openai_compat"
base_url = "https://api.z.ai/api/paas/v4"
api_key_env = ""

[models.glm-mini]
provider = "zai"
slug = "glm-5.1"
context_window = 131072
tool_format = "openai_json"
"#,
        )
        .expect("write roko.toml");

        persist_capture_episode(
            tmp.path(),
            "claude",
            Some("glm-mini"),
            "prd-plan-generate",
            "prd:plan:glm",
            "prompt body",
            "output body",
            true,
            42,
            None,
        )
        .await
        .expect("persist capture episode");

        let episodes_path = tmp
            .path()
            .join(".roko")
            .join("learn")
            .join("episodes.jsonl");
        let episodes = EpisodeLogger::read_all_lossy(&episodes_path).await.unwrap();
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].model, "glm-5.1");
        assert_eq!(
            episodes[0].extra.get("provider"),
            Some(&serde_json::json!("zai"))
        );

        let cascade_path = tmp
            .path()
            .join(".roko")
            .join("learn")
            .join("cascade-router.json");
        let cascade: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(cascade_path).unwrap()).unwrap();
        assert_eq!(
            cascade
                .pointer("/confidence_stats/glm-5.1/trials")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );

        let health_path = tmp
            .path()
            .join(".roko")
            .join("learn")
            .join("provider-health.json");
        let health: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(health_path).unwrap()).unwrap();
        assert_eq!(
            health
                .pointer("/providers/zai/total_requests")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }
}
