//! Agent execution helper for direct CLI flows such as PRD/research/plan generation.
//!
//! Used by `roko prd`, `roko research`, and `roko plan generate` to invoke
//! an agent that can read/write files while preserving provider-aware routing,
//! safety scoping, resume threading, and learning-episode persistence.

use std::path::Path;
use std::time::Instant;

use crate::agent_config::{command_from_config, model_from_config};
use crate::agent_episode::build_capture_episode;
use crate::agent_spawn::{SpawnAgentSpec, spawn_agent_scoped};
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
            role: None,
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
