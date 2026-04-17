//! Integration coverage for `roko agent serve`.

mod common;

use std::time::Duration;

use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;
use tempfile::TempDir;

use common::{AgentServeConfig, spawn_roko_agent_serve_on_random_port, wait_for_http_ok};

#[test]
fn agent_serve_help_exposes_batch_flags() {
    Command::cargo_bin("roko")
        .expect("roko binary")
        .arg("agent")
        .arg("serve")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--agent-id"))
        .stdout(contains("--bind"))
        .stdout(contains("--relay-url"))
        .stdout(contains("--chain-rpc-url"))
        .stdout(contains("--identity-registry"))
        .stdout(contains("--passport-id"))
        .stdout(contains("--wallet-key"));
}

#[tokio::test]
async fn roko_agent_serve_answers_health() {
    let workdir = TempDir::new().expect("tempdir");
    let mut config = AgentServeConfig::new("demo-1");
    config.relay_url = Some("https://relay.example");
    config.chain_rpc_url = Some("https://rpc.example");
    config.identity_registry = Some("0x1234");
    config.passport_id = Some("7");
    config.wallet_key = Some("0xdeadbeef");

    let serve = spawn_roko_agent_serve_on_random_port(
        workdir.path(),
        config,
    );

    let response = wait_for_http_ok(
        &format!("{}/health", serve.base_url),
        Duration::from_secs(10),
    )
    .await;
    let health: Value = response.json().await.expect("health json");

    assert_eq!(
        health.get("status").and_then(Value::as_str),
        Some("ok"),
        "`roko agent serve` did not return status=ok from /health\n{health}"
    );
    assert_eq!(
        health.get("agent_id").and_then(Value::as_str),
        Some("demo-1"),
        "`roko agent serve` did not wire --agent-id through to /health\n{health}"
    );
}
