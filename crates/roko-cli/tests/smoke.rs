//! Smoke tests for the CLAUDE.md status-table claims covered by UX44.

mod common;

use std::fs;
use std::time::Duration;

use common::*;
use roko_agent::claude_cli_agent::build_settings_json;
use roko_agent::provider::claude_cli::ClaudeCliAdapter;
use roko_agent::provider::{AgentOptions, ProviderAdapter};
use roko_agent_server::AgentServer;
use roko_cli::prompting::{PromptBuildOptions, build_role_system_prompt};
use roko_compose::TaskContext;
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use roko_core::{AgentRole, Body, Context, Engram, Kind};
use tempfile::TempDir;

fn prompt(text: &str) -> Engram {
    Engram::builder(Kind::Prompt).body(Body::text(text)).build()
}

fn claude_model() -> ModelProfile {
    ModelProfile {
        provider: "claude_cli".to_string(),
        slug: "claude-sonnet-4-6".to_string(),
        context_window: 200_000,
        max_output: Some(8_192),
        supports_tools: true,
        supports_thinking: false,
        supports_vision: false,
        supports_web_search: false,
        supports_mcp_tools: false,
        supports_partial: false,
        supports_grounding: false,
        supports_code_execution: false,
        supports_caching: false,
        provider_routing: None,
        tool_format: "anthropic_blocks".to_string(),
        cost_input_per_m: None,
        cost_output_per_m: None,
        cost_input_per_m_high: None,
        cost_output_per_m_high: None,
        cost_cache_read_per_m: None,
        cost_cache_write_per_m: None,
        thinking_level: None,
        max_tools: None,
        tokenizer_ratio: None,
        ..Default::default()
    }
}

#[test]
fn item_01_workspace_build_passes() {
    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .expect("run rustc --version");
    assert!(
        rustc.status.success(),
        "CLAUDE.md item 01 invalidated: rustc --version failed with {}",
        rustc.status
    );
    let version = String::from_utf8_lossy(&rustc.stdout);
    assert!(
        rustc_version_ge(&version, "1.91.0"),
        "CLAUDE.md item 01 invalidated: rustc must be >= 1.91.0, got {version}"
    );

    let output = run_cargo(&["check", "--workspace", "--quiet"], &workspace_root());
    assert!(
        output.status.success(),
        "CLAUDE.md item 01 invalidated: cargo check --workspace failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn item_02_system_prompt_builder_renders_role_layers() {
    let task = TaskContext::new("Smoke-test the role prompt wiring")
        .with_plan_id("UX44")
        .with_goal("keep the system prompt path wired")
        .with_workspace("crates/roko-cli")
        .with_domain_notes("Exercise the live RoleSystemPromptSpec builder path.");
    let prompt = build_role_system_prompt(
        AgentRole::Implementer,
        task,
        "Read,Edit,Bash",
        PromptBuildOptions::default(),
    );

    assert!(
        prompt.starts_with("You are the Implementer."),
        "CLAUDE.md item 02 invalidated: role identity text did not lead the rendered prompt\n{prompt}"
    );
    assert!(
        prompt.contains("## Tool Instructions"),
        "CLAUDE.md item 02 invalidated: built prompt is missing the tool instructions section\n{prompt}"
    );
    assert!(
        prompt.contains("Claude tool allowlist: Read,Edit,Bash"),
        "CLAUDE.md item 02 invalidated: tool allowlist text did not reach the rendered prompt\n{prompt}"
    );
}

#[test]
fn item_03_episode_logger_appends_memory_log() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());
    fs::write(
        tmp.path().join("roko.toml"),
        r#"
[agent]
command = "cat"
args = []
timeout_ms = 30000

[prompt]
token_budget = 1000
role = "You are a Roko agent."

[[gate]]
kind = "shell"
program = "true"
args = []
timeout_ms = 5000
"#,
    )
    .expect("write roko.toml");

    let episodes_path = tmp.path().join(".roko").join("memory").join("episodes.jsonl");
    let before = fs::read_to_string(&episodes_path).unwrap_or_default();
    run_roko(
        tmp.path(),
        &["run", "write a hello function", "--workdir", tmp.path().to_str().expect("workdir")],
    )
    .success();
    let after = fs::read_to_string(&episodes_path).expect("read episodes.jsonl");

    assert!(
        after.lines().count() > before.lines().count(),
        "CLAUDE.md item 03 invalidated: .roko/memory/episodes.jsonl did not grow during a live run"
    );
    let last_episode: serde_json::Value = serde_json::from_str(
        after
            .lines()
            .last()
            .expect("episode log contains appended entry"),
    )
    .expect("parse appended episode");
    assert_eq!(
        last_episode
            .get("extra")
            .and_then(|extra| extra.get("plan_id"))
            .and_then(serde_json::Value::as_str),
        Some("cli-run"),
        "CLAUDE.md item 03 invalidated: appended learning episode is missing the cli-run marker\n{last_episode}"
    );
    assert!(
        last_episode
            .get("gate_verdicts")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|verdicts| !verdicts.is_empty()),
        "CLAUDE.md item 03 invalidated: appended learning episode is missing gate verdicts\n{last_episode}"
    );
}

#[test]
fn item_04_plan_runner_reports_non_zero_agent_calls() {
    let tmp = TempDir::new().expect("tempdir");
    setup_sample_plan_workspace(tmp.path());

    let report = run_sample_plan(tmp.path());
    let total_agent_calls = report
        .get("total_agent_calls")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    assert!(
        report
            .get("succeeded")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        "CLAUDE.md item 04 invalidated: plan run did not succeed\n{report}"
    );
    assert!(
        total_agent_calls > 0,
        "CLAUDE.md item 04 invalidated: PlanRunner reported zero agent calls\n{report}"
    );
}

#[tokio::test]
async fn item_05_mcp_config_passthrough_reaches_agent_cli() {
    let tmp = TempDir::new().expect("tempdir");
    let script = tmp.path().join("claude-capture.sh");
    let args_file = tmp.path().join("args.txt");
    let prompt_file = tmp.path().join("prompt.txt");
    let mcp_config = tmp.path().join("mcp.json");
    fs::write(&mcp_config, "{}").expect("write mcp config");

    write_executable(
        &script,
        &format!(
            r#"#!/bin/sh
set -eu
printf '%s\n' "$@" > "{args_file}"
cat > "{prompt_file}"
printf '%s\n' '{{"type":"content_block_delta","delta":{{"text":"mcp-ok"}}}}'
"#,
            args_file = args_file.display(),
            prompt_file = prompt_file.display(),
        ),
    );

    let provider = ProviderConfig {
        kind: ProviderKind::ClaudeCli,
        base_url: None,
        api_key_env: None,
        command: Some(script.display().to_string()),
        args: Some(vec!["--provider-flag".to_string()]),
        timeout_ms: Some(1_500),
        ttft_timeout_ms: Some(15_000),
        connect_timeout_ms: Some(5_000),
        extra_headers: None,
        max_concurrent: None,
    };
    let options = AgentOptions {
        command: None,
        timeout_ms: Some(1_500),
        system_prompt: Some("system guidance".to_string()),
        cached_content: None,
        tools: Some("Read,Edit".to_string()),
        mcp_config: Some(mcp_config.clone()),
        working_dir: Some(tmp.path().to_path_buf()),
        provider_semaphores: None,
        env: Vec::new(),
        extra_args: vec!["--extra-flag".to_string()],
        effort: Some("high".to_string()),
        bare_mode: false,
        dangerously_skip_permissions: false,
        name: "ux44-mcp-smoke".to_string(),
    };

    let agent = ClaudeCliAdapter
        .create_agent(&provider, &claude_model(), &options)
        .expect("create Claude CLI agent");
    let result = agent.run(&prompt("hello"), &Context::now()).await;
    assert!(
        result.success,
        "CLAUDE.md item 05 invalidated: Claude CLI mock run failed: {}",
        result.output.body.as_text().unwrap_or("unknown")
    );

    let args_text = fs::read_to_string(&args_file).expect("read args");
    assert!(
        args_text.contains("--mcp-config"),
        "CLAUDE.md item 05 invalidated: MCP config flag was not forwarded to the agent CLI\n{args_text}"
    );
    assert!(
        args_text.contains(mcp_config.to_str().expect("mcp path")),
        "CLAUDE.md item 05 invalidated: MCP config path was not forwarded to the agent CLI\n{args_text}"
    );
    assert!(
        args_text.contains("--strict-mcp-config"),
        "CLAUDE.md item 05 invalidated: strict MCP config flag was not forwarded to the agent CLI\n{args_text}"
    );
    assert!(
        args_text.contains(&build_settings_json()),
        "CLAUDE.md item 05 invalidated: Claude settings payload was not forwarded alongside MCP config\n{args_text}"
    );
}

#[test]
fn item_06_plan_run_persists_learning_feedback() {
    let tmp = TempDir::new().expect("tempdir");
    setup_sample_plan_workspace(tmp.path());

    let _report = run_sample_plan(tmp.path());

    let efficiency_path = tmp.path().join(".roko").join("learn").join("efficiency.jsonl");
    let efficiency = fs::read_to_string(&efficiency_path).expect("read efficiency.jsonl");
    assert!(
        !efficiency.trim().is_empty(),
        "CLAUDE.md item 06 invalidated: .roko/learn/efficiency.jsonl did not grow"
    );

    let cascade_path = tmp
        .path()
        .join(".roko")
        .join("learn")
        .join("cascade-router.json");
    let cascade = fs::read_to_string(&cascade_path).expect("read cascade-router.json");
    let parsed: serde_json::Value = serde_json::from_str(&cascade).unwrap_or_else(|err| {
        panic!(
            "CLAUDE.md item 06 invalidated: cascade-router.json is not valid JSON: {err}\n{cascade}"
        )
    });
    assert!(
        parsed.is_object(),
        "CLAUDE.md item 06 invalidated: cascade-router.json did not persist an object payload\n{parsed}"
    );
}

#[test]
fn item_07_dashboard_text_renders_once() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    let assert = run_roko(
        tmp.path(),
        &[
            "dashboard",
            "--text",
            "--workdir",
            tmp.path().to_str().expect("workdir"),
        ],
    )
    .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    assert!(
        stdout.contains("dashboard scaffold:"),
        "CLAUDE.md item 07 invalidated: `roko dashboard --text` did not render dashboard text\n{stdout}"
    );
}

#[tokio::test]
async fn item_08_agent_sidecar_starts_and_reports_health() {
    let port = pick_unused_port();
    let server = AgentServer::builder()
        .agent_id("ux44-smoke-agent")
        .bind(format!("127.0.0.1:{port}"))
        .build()
        .expect("build agent server");

    let task = tokio::spawn(async move { server.serve().await });
    let response =
        wait_for_http_ok(&format!("http://127.0.0.1:{port}/health"), Duration::from_secs(5)).await;

    assert!(
        response.status().is_success(),
        "CLAUDE.md item 08 invalidated: /health did not return HTTP 200"
    );

    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn item_09_roko_serve_serves_api_status() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    let serve = spawn_roko_serve_on_random_port(tmp.path());
    let response =
        wait_for_http_ok(&format!("{}/api/status", serve.base_url), Duration::from_secs(10)).await;
    let status: serde_json::Value = response.json().await.expect("status json");

    assert!(
        status.get("workdir").is_some(),
        "CLAUDE.md item 09 invalidated: /api/status payload is missing the workdir field\n{status}"
    );
}
