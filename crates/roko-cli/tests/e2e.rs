//! End-to-end CLI test.
//!
//! Spawns the `roko` binary against a tempdir: runs `init`, then `run`
//! with a deterministic mock backend, and asserts that the current workflow
//! artifacts are persisted (`runtime-events.jsonl`, `episodes.jsonl`, and
//! `learn/efficiency.jsonl`).

use assert_cmd::Command;
use std::fs;
use std::process::Command as ProcessCommand;
use std::time::Duration;
use tempfile::TempDir;

const MOCK_DISPATCHER: &str = "mock-self-host-fixture";

fn workflow_test_config(gate_program: &str, extra_prompt_config: &str) -> String {
    format!(
        r#"
[agent]
default_model = "mock-model"
command = "cat"
args = []
timeout_ms = 30000

[prompt]
token_budget = 10000
role = "You are a Roko test agent."
{extra_prompt_config}

[providers.mock]
kind = "claude_cli"
command = "cat"
timeout_ms = 30000

[models.mock-model]
provider = "mock"
slug = "mock-model"
context_window = 8192
supports_tools = false

[pipeline.mechanical]
strategist = false
reviewers = false
max_iterations = 1

[pipeline.focused]
strategist = false
reviewers = false
max_iterations = 1

[[gate]]
kind = "shell"
program = "{gate_program}"
args = []
timeout_ms = 5000
"#
    )
}

fn isolated_roko(workdir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("roko").expect("roko binary");
    cmd.timeout(Duration::from_secs(90))
        .env("HOME", workdir)
        .env("ROKO_DISPATCHER", MOCK_DISPATCHER)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("XDG_CONFIG_HOME");
    cmd
}

fn seed_git_repo(workdir: &std::path::Path) {
    for args in [
        vec!["init"],
        vec!["config", "user.name", "Roko E2E"],
        vec!["config", "user.email", "roko-e2e@example.com"],
        vec!["add", "-A"],
        vec!["commit", "-m", "seed"],
    ] {
        let status = ProcessCommand::new("git")
            .current_dir(workdir)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success(), "git setup failed with {status}");
    }
}

#[test]
fn init_run_produces_expected_signals() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    // `roko init <workdir>`
    Command::cargo_bin("roko")
        .unwrap()
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    // Verify the init artifacts exist.
    assert!(workdir.join(".roko").is_dir(), ".roko directory missing");
    assert!(
        workdir.join(".roko/engrams.jsonl").exists(),
        "engrams.jsonl missing"
    );
    assert!(workdir.join("roko.toml").exists(), "roko.toml missing");

    // Replace the default config with a deterministic local backend so the
    // test does not depend on whatever provider `roko init` currently prefers.
    let config = workflow_test_config("true", "");
    fs::write(workdir.join("roko.toml"), config).unwrap();
    seed_git_repo(workdir);

    // `roko run "hello"` with cat as the agent and the default `true` shell
    // gate → both agent and gate pass.
    let run = isolated_roko(workdir)
        .arg("run")
        .arg("write a hello function")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&run.get_output().stdout);
    assert!(
        stdout.contains("workflow completed"),
        "run output missing completion summary: {stdout}"
    );
    assert!(
        stdout.contains("[PASS] shell"),
        "run output missing shell gate pass: {stdout}"
    );

    let runtime_log = fs::read_to_string(workdir.join(".roko/runtime-events.jsonl")).unwrap();
    assert!(
        runtime_log.contains("\"workflow_started\""),
        "workflow start event missing: {runtime_log}"
    );
    assert!(
        runtime_log.contains("\"agent_completed\""),
        "agent completion event missing: {runtime_log}"
    );
    assert!(
        runtime_log.contains("\"workflow_completed\""),
        "workflow completion event missing: {runtime_log}"
    );

    let efficiency = fs::read_to_string(workdir.join(".roko/learn/efficiency.jsonl")).unwrap();
    assert!(
        efficiency.contains("\"kind\":\"model_call\""),
        "model call feedback missing: {efficiency}"
    );
    assert!(
        efficiency.contains("\"kind\":\"gate_result\"") && efficiency.contains("\"passed\":true"),
        "gate pass feedback missing: {efficiency}"
    );
    assert!(
        efficiency.contains("\"kind\":\"workflow_completed\""),
        "workflow feedback missing: {efficiency}"
    );

    let episodes = fs::read_to_string(workdir.join(".roko/episodes.jsonl")).unwrap();
    assert!(
        episodes.contains("\"success\":true"),
        "successful episode missing: {episodes}"
    );

    // `roko status` should succeed and surface current workflow artifacts even
    // though the WorkflowEngine no longer writes legacy engram episode signals.
    let status = Command::cargo_bin("roko")
        .unwrap()
        .arg("status")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&status.get_output().stdout).into_owned();
    assert!(
        stdout.contains("signal counts"),
        "status output missing header: {stdout}"
    );
    assert!(
        stdout.contains("most recent episode: ep-"),
        "status did not report an episode: {stdout}"
    );
    assert!(
        stdout.contains("gate verdicts: 1 pass / 0 fail"),
        "status did not report gate result: {stdout}"
    );
}

#[test]
fn status_cfactor_reports_trend_and_components() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    Command::cargo_bin("roko")
        .unwrap()
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    let learn_dir = workdir.join(".roko").join("learn");
    fs::create_dir_all(&learn_dir).unwrap();

    // Episode must live under learn/ — that is where LearningPaths::under()
    // resolves episodes_jsonl when refresh_cfactor_snapshot is called with
    // the learn directory as root.
    let now = chrono::Utc::now();
    let episode_ts = now.to_rfc3339();
    let episode = serde_json::json!({
        "kind": "agent_turn",
        "id": "ep-1",
        "timestamp": episode_ts,
        "agent_id": "agent-a",
        "task_id": "task-a",
        "input_signal_hash": "",
        "output_signal_hash": "",
        "episode_id": "episode-1",
        "agent_template": "Implementer",
        "model": "claude-sonnet",
        "trigger_kind": "manual",
        "trigger_signal_hash": "",
        "started_at": episode_ts,
        "completed_at": episode_ts,
        "duration_secs": 1.0,
        "gate_verdicts": [],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_read_tokens": 0,
            "cache_write_tokens": 0,
            "cost_usd": 1.0,
            "cost_usd_without_cache": 1.0,
            "wall_ms": 100
        },
        "success": true,
        "turns": 1,
        "tokens_used": 150,
        "external_actions": [],
        "failure_reason": null,
        "headline": false,
        "extra": {}
    });
    fs::write(learn_dir.join("episodes.jsonl"), episode.to_string() + "\n").unwrap();

    // Use timestamps relative to now so the history entries are always
    // inside the 7-day trend window used by trend_arrow.
    let earlier_ts = (now - chrono::Duration::days(5)).to_rfc3339();
    let recent_ts = (now - chrono::Duration::days(2)).to_rfc3339();
    let earlier = serde_json::json!({
        "overall": 0.25,
        "components": {
            "gate_pass_rate": 0.20,
            "cost_efficiency": 0.20,
            "speed": 0.20,
            "first_try_rate": 0.20,
            "knowledge_growth": 0.20,
            "turn_taking_equality": 0.20
        },
        "computed_at": earlier_ts,
        "episode_count": 1
    });
    let recent = serde_json::json!({
        "overall": 0.40,
        "components": {
            "gate_pass_rate": 0.30,
            "cost_efficiency": 0.30,
            "speed": 0.30,
            "first_try_rate": 0.30,
            "knowledge_growth": 0.10,
            "turn_taking_equality": 0.30
        },
        "computed_at": recent_ts,
        "episode_count": 1
    });

    fs::write(
        learn_dir.join("c-factor.jsonl"),
        [earlier.to_string(), recent.to_string()].join("\n") + "\n",
    )
    .unwrap();

    let status = Command::cargo_bin("roko")
        .unwrap()
        .arg("status")
        .arg("--cfactor")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&status.get_output().stdout).into_owned();
    assert!(
        stdout.contains("c-factor:"),
        "status output missing c-factor summary: {stdout}"
    );
    assert!(
        stdout.contains("trend="),
        "status output missing trend: {stdout}"
    );
    assert!(
        stdout.contains("gate="),
        "status output missing components: {stdout}"
    );
    assert!(
        stdout.contains('↑'),
        "status output missing upward trend arrow: {stdout}"
    );
}

#[test]
fn run_fails_when_gate_fails() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    Command::cargo_bin("roko")
        .unwrap()
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    // Replace roko.toml with one whose gate is `false` (always fails).
    let failing_config = workflow_test_config("false", "");
    fs::write(workdir.join("roko.toml"), failing_config).unwrap();
    seed_git_repo(workdir);

    // Exit code should be non-zero because the gate failed.
    let run = isolated_roko(workdir)
        .arg("run")
        .arg("smoke")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&run.get_output().stdout);
    assert!(
        stdout.contains("workflow failed"),
        "run output missing failure summary: {stdout}"
    );
    assert!(
        stdout.contains("[FAIL] shell"),
        "run output missing shell gate failure: {stdout}"
    );

    // But the workflow artifacts should still be persisted — the failure is reported, not swallowed.
    let efficiency = fs::read_to_string(workdir.join(".roko/learn/efficiency.jsonl")).unwrap();
    assert!(
        efficiency.contains("\"kind\":\"gate_result\"") && efficiency.contains("\"passed\":false"),
        "gate failure feedback missing: {efficiency}"
    );
    assert!(
        efficiency.contains("\"kind\":\"workflow_failed\""),
        "workflow failure feedback missing: {efficiency}"
    );
    let episodes = fs::read_to_string(workdir.join(".roko/episodes.jsonl")).unwrap();
    assert!(
        episodes.contains("\"success\":false"),
        "failed episode missing: {episodes}"
    );
}

#[test]
fn prompt_files_are_injected_as_sections() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    Command::cargo_bin("roko")
        .unwrap()
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    // Write a fixture file and a config that injects it.
    fs::write(
        workdir.join("issue.md"),
        "# Bug report\nThe `greet()` function returns the wrong string.\n",
    )
    .unwrap();
    let config = workflow_test_config(
        "true",
        r#"
[[prompt.files]]
path = "issue.md"
name = "issue"
priority = "high"
"#,
    );
    fs::write(workdir.join("roko.toml"), config).unwrap();
    seed_git_repo(workdir);

    isolated_roko(workdir)
        .arg("run")
        .arg("Suggest a fix for the bug described in the issue file.")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .success();

    let log = fs::read_to_string(workdir.join(".roko/runtime-events.jsonl")).unwrap();
    assert!(
        log.contains("Bug report"),
        "file contents should have reached the prompt: {log}"
    );
    // The workflow prompt should include the task plus the injected file section.
    let efficiency = fs::read_to_string(workdir.join(".roko/learn/efficiency.jsonl")).unwrap();
    assert!(
        efficiency.contains("role_identity") && efficiency.contains("task_context"),
        "prompt section feedback missing: {efficiency}"
    );
}

#[test]
#[cfg(feature = "legacy-orchestrate")]
fn clean_output_strips_thinking_trace() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    Command::cargo_bin("roko")
        .unwrap()
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    // Use `printf` as the agent backend — it emits a fake thinking trace
    // followed by a short answer. This bypasses needing a real LLM.
    let config = r#"
[agent]
command = "sh"
args = ["-c", "printf 'Thinking...\\nstep 1\\nstep 2\\n...done thinking.\\n\\nFinal answer: 42\\n'"]
timeout_ms = 10000
clean_output = true

[prompt]
token_budget = 1000
role = "You are a test agent."

[[gate]]
kind = "shell"
program = "true"
args = []
timeout_ms = 5000
"#;
    fs::write(workdir.join("roko.toml"), config).unwrap();

    // Isolate from the user's global config and API keys so the test
    // uses the subprocess path (`sh`) instead of a real provider.
    Command::cargo_bin("roko")
        .unwrap()
        .arg("run")
        .arg("whatever")
        .arg("--workdir")
        .arg(workdir)
        .env("HOME", workdir)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("XDG_CONFIG_HOME")
        .assert()
        .success();

    // The cleaned AgentOutput should contain only "Final answer: 42".
    let log = fs::read_to_string(workdir.join(".roko/engrams.jsonl")).unwrap();
    // Raw AgentMessage trace has the full thinking block
    assert!(
        log.contains("...done thinking."),
        "raw trace not persisted: {log}"
    );
    // At least one AgentOutput signal must have cleaned=true and NOT contain 'step 1'.
    let has_cleaned = log
        .lines()
        .filter(|l| l.contains("\"cleaned\":\"true\""))
        .any(|l| l.contains("Final answer") && !l.contains("step 1"));
    assert!(
        has_cleaned,
        "cleaned AgentOutput missing or not sanitized: {log}"
    );
}
