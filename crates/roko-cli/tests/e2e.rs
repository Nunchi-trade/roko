//! End-to-end CLI test.
//!
//! Spawns the `roko` binary against a tempdir: runs `init`, then `run`
//! with `cat` as the agent backend, and asserts that `.roko/signals.jsonl`
//! contains the full signal set (`Prompt`, `AgentOutput`, `Episode`, etc.).

use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

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
        workdir.join(".roko/signals.jsonl").exists(),
        "signals.jsonl missing"
    );
    assert!(workdir.join("roko.toml").exists(), "roko.toml missing");

    // `roko run "hello"` with cat as the agent (the default in roko.toml) and
    // the default `true` shell gate → both agent and gate pass.
    Command::cargo_bin("roko")
        .unwrap()
        .arg("run")
        .arg("write a hello function")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .success();

    // Read the JSONL log and check that every required kind is present.
    let log = fs::read_to_string(workdir.join(".roko/signals.jsonl")).unwrap();
    assert!(!log.is_empty(), "signals.jsonl is empty after run");

    let mut saw_prompt_section = false;
    let mut saw_prompt = false;
    let mut saw_agent_output = false;
    let mut saw_episode = false;
    let mut saw_verdict = false;
    for line in log.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // The JSONL kind field is a snake_case enum tag.
        if line.contains("\"prompt_section\"") {
            saw_prompt_section = true;
        }
        if line.contains("\"prompt\"") {
            saw_prompt = true;
        }
        if line.contains("\"agent_output\"") {
            saw_agent_output = true;
        }
        if line.contains("\"episode\"") {
            saw_episode = true;
        }
        if line.contains("\"gate_verdict\"") {
            saw_verdict = true;
        }
    }
    assert!(saw_prompt_section, "no PromptSection signal persisted");
    assert!(saw_prompt, "no Prompt signal persisted");
    assert!(saw_agent_output, "no AgentOutput signal persisted");
    assert!(saw_episode, "no Episode signal persisted");
    assert!(saw_verdict, "no GateVerdict signal persisted");

    // `roko status` should succeed and mention non-zero signals.
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
        stdout.contains("episode"),
        "status output missing episode kind: {stdout}"
    );
    assert!(
        stdout.contains("most recent episode"),
        "status did not report an episode: {stdout}"
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
    let failing_config = r#"
[agent]
command = "cat"
args = []
timeout_ms = 30000

[prompt]
token_budget = 1000
role = "You are a Roko agent."

[[gate]]
kind = "shell"
program = "false"
args = []
timeout_ms = 5000
"#;
    fs::write(workdir.join("roko.toml"), failing_config).unwrap();

    // Exit code should be non-zero because the gate failed.
    Command::cargo_bin("roko")
        .unwrap()
        .arg("run")
        .arg("smoke")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .failure();

    // But the signals should still be persisted — the failure is reported, not swallowed.
    let log = fs::read_to_string(workdir.join(".roko/signals.jsonl")).unwrap();
    assert!(log.contains("\"gate_verdict\""));
    assert!(log.contains("\"episode\""));
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
    let config = r#"
[agent]
command = "cat"
args = []
timeout_ms = 30000

[prompt]
token_budget = 10000
role = "You are a Rust engineer."

[[prompt.files]]
path = "issue.md"
name = "issue"
priority = "high"

[[gate]]
kind = "shell"
program = "true"
args = []
timeout_ms = 5000
"#;
    fs::write(workdir.join("roko.toml"), config).unwrap();

    let out = Command::cargo_bin("roko")
        .unwrap()
        .arg("run")
        .arg("Suggest a fix for the bug described in the issue file.")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .success();

    // The cat-echoed output should contain the injected file contents.
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        stdout.contains("cat"),
        "expected agent output header: {stdout}"
    );

    let log = fs::read_to_string(workdir.join(".roko/signals.jsonl")).unwrap();
    assert!(
        log.contains("Bug report"),
        "file contents should have reached the prompt: {log}"
    );
    // There should be 3 prompt sections now (role + issue-file + task).
    let section_count = log.matches("\"prompt_section\"").count();
    assert!(
        section_count >= 3,
        "expected >=3 prompt sections, got {section_count}"
    );
}

#[test]
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

    Command::cargo_bin("roko")
        .unwrap()
        .arg("run")
        .arg("whatever")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .success();

    // The cleaned AgentOutput should contain only "Final answer: 42".
    let log = fs::read_to_string(workdir.join(".roko/signals.jsonl")).unwrap();
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
