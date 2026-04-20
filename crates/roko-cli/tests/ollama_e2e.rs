//! End-to-end integration tests using Ollama (llama3.2) as a real LLM backend.
//!
//! **Gated** behind `ROKO_TEST_OLLAMA=1`. When the env var is absent every test
//! in this module is silently skipped.
//!
//! Prerequisites:
//!   - Ollama running locally on port 11434
//!   - `ollama pull llama3.2` completed

use assert_cmd::Command;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

/// Returns `true` when the Ollama test gate is open.
fn ollama_gate() -> bool {
    std::env::var("ROKO_TEST_OLLAMA").is_ok()
}

/// Write a `roko.toml` that routes through Ollama via the provider registry.
fn write_ollama_config(workdir: &std::path::Path) {
    let config = r#"
[agent]
default_model = "llama32"
default_backend = "ollama"

[prompt]
token_budget = 2000
role = "You are a helpful assistant."

[providers.ollama]
kind = "openai_compat"
base_url = "http://localhost:11434"
timeout_ms = 120000

[models.llama32]
provider = "ollama"
slug = "llama3.2"
context_window = 8192
supports_tools = false

[[gate]]
kind = "shell"
program = "true"
args = []
timeout_ms = 5000
"#;
    fs::write(workdir.join("roko.toml"), config).unwrap();
}

/// Helper: build a `roko` command targeting `workdir` with a generous timeout.
fn roko_cmd(workdir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("roko").expect("roko binary");
    cmd.timeout(Duration::from_secs(90));
    cmd.current_dir(workdir);
    cmd
}

// -----------------------------------------------------------------------
// Test: roko init
// -----------------------------------------------------------------------

#[test]
fn ollama_e2e_init_creates_workspace() {
    if !ollama_gate() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    roko_cmd(workdir)
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    assert!(workdir.join(".roko").is_dir(), ".roko directory missing");
    assert!(workdir.join("roko.toml").exists(), "roko.toml missing");
}

// -----------------------------------------------------------------------
// Test: roko run with Ollama
// -----------------------------------------------------------------------

#[test]
fn ollama_e2e_run_produces_signals() {
    if !ollama_gate() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    // Initialize workspace.
    roko_cmd(workdir)
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    // Replace config with Ollama provider routing.
    write_ollama_config(workdir);

    // Run a simple prompt through Ollama.
    roko_cmd(workdir)
        .arg("run")
        .arg("Say hello in exactly three words.")
        .arg("--workdir")
        .arg(workdir)
        .timeout(Duration::from_secs(120))
        .assert()
        .success();

    // Verify signals were written.
    let engrams_path = workdir.join(".roko/engrams.jsonl");
    assert!(
        engrams_path.exists(),
        "engrams.jsonl missing after run"
    );
    let log = fs::read_to_string(&engrams_path).unwrap();
    assert!(!log.is_empty(), "engrams.jsonl is empty after run");

    // Check for key signal kinds.
    assert!(
        log.contains("\"prompt\"") || log.contains("\"prompt_section\""),
        "no prompt signal found in engrams.jsonl"
    );
    assert!(
        log.contains("\"agent_output\""),
        "no agent_output signal found in engrams.jsonl"
    );
    assert!(
        log.contains("\"episode\""),
        "no episode signal found in engrams.jsonl"
    );
}

// -----------------------------------------------------------------------
// Test: roko prd idea + prd list
// -----------------------------------------------------------------------

#[test]
fn ollama_e2e_prd_idea_and_list() {
    if !ollama_gate() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    // Initialize workspace.
    roko_cmd(workdir)
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    write_ollama_config(workdir);

    // Capture an idea.
    roko_cmd(workdir)
        .arg("--repo")
        .arg(workdir)
        .arg("prd")
        .arg("idea")
        .arg("Add Ollama integration tests")
        .assert()
        .success();

    // Verify idea appears in the ideas file.
    let ideas_path = workdir.join(".roko/prd/ideas.md");
    assert!(ideas_path.exists(), "ideas.md not created");
    let ideas = fs::read_to_string(&ideas_path).unwrap();
    assert!(
        ideas.contains("Add Ollama integration tests"),
        "idea text not found in ideas.md: {ideas}"
    );

    // List PRDs — should mention the idea.
    let list_assert = roko_cmd(workdir)
        .arg("--repo")
        .arg(workdir)
        .arg("prd")
        .arg("list")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&list_assert.get_output().stdout).into_owned();
    assert!(
        stdout.contains("idea") || stdout.contains("Idea") || stdout.contains("Ollama"),
        "prd list output does not reference ideas: {stdout}"
    );
}

// -----------------------------------------------------------------------
// Test: roko status
// -----------------------------------------------------------------------

#[test]
fn ollama_e2e_status_returns_valid_output() {
    if !ollama_gate() {
        return;
    }

    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    // Initialize workspace.
    roko_cmd(workdir)
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    write_ollama_config(workdir);

    // Status should succeed even on a fresh workspace.
    let status_assert = roko_cmd(workdir)
        .arg("status")
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&status_assert.get_output().stdout).into_owned();
    assert!(
        stdout.contains("signal counts"),
        "status output missing signal counts header: {stdout}"
    );
}
