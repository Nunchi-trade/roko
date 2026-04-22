//! Integration tests for `roko agent` lifecycle commands (list, start, stop, status).

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

/// Helper: run a roko CLI command against the given workdir.
fn roko(workdir: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("roko")
        .unwrap()
        .current_dir(workdir)
        .args(args)
        .env("ROKO_LOG", "warn")
        .assert()
}

#[test]
fn agent_create_and_list() {
    let dir = TempDir::new().unwrap();

    // Init workspace.
    roko(dir.path(), &["init"]).success();

    // Create agent.
    roko(
        dir.path(),
        &[
            "agent",
            "create",
            "--name",
            "test-agent",
            "--domain",
            "coding",
        ],
    )
    .success();

    // List agents — should show test-agent with "created" status.
    let out = roko(dir.path(), &["agent", "list"]).success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(
        stdout.contains("test-agent"),
        "agent list should contain 'test-agent', got: {stdout}"
    );
    assert!(
        stdout.contains("created"),
        "agent list should show 'created' status, got: {stdout}"
    );
    assert!(
        stdout.contains("coding"),
        "agent list should show 'coding' domain, got: {stdout}"
    );
}

#[test]
fn agent_status_shows_created() {
    let dir = TempDir::new().unwrap();
    roko(dir.path(), &["init"]).success();
    roko(
        dir.path(),
        &[
            "agent", "create", "--name", "my-agent", "--domain", "research",
        ],
    )
    .success();

    let out = roko(dir.path(), &["agent", "status", "--name", "my-agent"]).success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(
        stdout.contains("my-agent"),
        "status should contain agent name, got: {stdout}"
    );
    assert!(
        stdout.contains("created"),
        "status should show 'created', got: {stdout}"
    );
    assert!(
        stdout.contains("research"),
        "status should show 'research' domain, got: {stdout}"
    );
}

#[test]
fn agent_list_empty_workspace() {
    let dir = TempDir::new().unwrap();
    roko(dir.path(), &["init"]).success();

    let out = roko(dir.path(), &["agent", "list"]).success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(
        stdout.contains("No agents") || stdout.contains("NAME"),
        "empty list should show 'No agents' or header, got: {stdout}"
    );
}

#[test]
fn agent_stop_not_running() {
    let dir = TempDir::new().unwrap();
    roko(dir.path(), &["init"]).success();
    roko(dir.path(), &["agent", "create", "--name", "idle-agent"]).success();

    // Stopping an agent that was never started should not crash.
    let out = roko(dir.path(), &["agent", "stop", "--name", "idle-agent"]).success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(
        stdout.contains("not running"),
        "stop should report 'not running', got: {stdout}"
    );
}

#[test]
fn agent_status_not_found() {
    let dir = TempDir::new().unwrap();
    roko(dir.path(), &["init"]).success();

    // Status for a nonexistent agent should fail.
    roko(dir.path(), &["agent", "status", "--name", "ghost"]).failure();
}

#[test]
fn agent_list_skips_deleted() {
    let dir = TempDir::new().unwrap();
    roko(dir.path(), &["init"]).success();

    // Create two agents.
    roko(
        dir.path(),
        &["agent", "create", "--name", "keep-me", "--domain", "coding"],
    )
    .success();
    roko(
        dir.path(),
        &[
            "agent",
            "create",
            "--name",
            "delete-me",
            "--domain",
            "general",
        ],
    )
    .success();

    // Delete one (non-force leaves DELETED marker).
    roko(dir.path(), &["agent", "delete", "--name", "delete-me"]).success();

    // List should only show the surviving agent.
    let out = roko(dir.path(), &["agent", "list"]).success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(
        stdout.contains("keep-me"),
        "list should show surviving agent, got: {stdout}"
    );
    assert!(
        !stdout.contains("delete-me"),
        "list should skip deleted agent, got: {stdout}"
    );
}
