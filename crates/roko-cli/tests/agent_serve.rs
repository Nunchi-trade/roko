//! Integration coverage for `roko agent serve`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use agent_relay::{app, protocol::ConnectedAgent, state::RelayState};
use assert_cmd::Command;
use predicates::str::contains;
use reqwest::Client;
use serde_json::Value;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use common::{AgentServeConfig, spawn_roko_agent_serve_on_random_port, wait_for_http_ok};

fn write_agent_config(workdir: &std::path::Path) {
    std::fs::write(
        workdir.join("roko.toml"),
        r#"
[agent]
default_model = "test-agent"
command = "cat"
timeout_ms = 10000
bare_mode = true
"#,
    )
    .expect("write roko.toml");
}

struct TestRelay {
    base_url: String,
    client: Client,
    task: JoinHandle<()>,
}

impl TestRelay {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind relay");
        let addr = listener.local_addr().expect("relay addr");
        let base_url = format!("http://{addr}");
        let state = Arc::new(RelayState::new());
        let task = tokio::spawn(async move {
            axum::serve(listener, app(state))
                .await
                .expect("serve relay");
        });

        let relay = Self {
            base_url,
            client: Client::new(),
            task,
        };
        relay.wait_until_ready().await;
        relay
    }

    async fn wait_until_ready(&self) {
        for _ in 0..120 {
            let response = self
                .client
                .get(format!("{}/relay/health", self.base_url))
                .send()
                .await;
            if matches!(response, Ok(response) if response.status().is_success()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("relay did not become ready");
    }

    async fn wait_for_agent(&self, agent_id: &str) -> ConnectedAgent {
        for _ in 0..120 {
            let response = self
                .client
                .get(format!("{}/relay/agents", self.base_url))
                .send()
                .await
                .expect("GET /relay/agents");
            let agents: Vec<ConnectedAgent> = response.json().await.expect("relay agents");
            if let Some(agent) = agents.into_iter().find(|agent| agent.agent_id == agent_id) {
                return agent;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("agent {agent_id} did not appear in relay");
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

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
    write_agent_config(workdir.path());
    let serve =
        spawn_roko_agent_serve_on_random_port(workdir.path(), AgentServeConfig::new("demo-1"));

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

    let message: Value = Client::new()
        .post(format!("{}/message", serve.base_url))
        .json(&serde_json::json!({ "prompt": "hello from health test" }))
        .send()
        .await
        .expect("POST /message")
        .json()
        .await
        .expect("message json");
    assert_eq!(
        message.get("response").and_then(Value::as_str),
        Some("hello from health test")
    );
}

#[tokio::test]
async fn roko_agent_serve_honors_roko_config_env_override() {
    let workdir = TempDir::new().expect("tempdir");
    let override_path = workdir.path().join("demo-agent.toml");
    std::fs::write(
        &override_path,
        r#"
[agent]
default_model = "demo-echo"
command = "cat"
timeout_ms = 10000
bare_mode = true
"#,
    )
    .expect("write override config");

    let mut config = AgentServeConfig::new("env-override");
    config.roko_config = Some(&override_path);
    let serve = spawn_roko_agent_serve_on_random_port(workdir.path(), config);

    wait_for_http_ok(
        &format!("{}/health", serve.base_url),
        Duration::from_secs(10),
    )
    .await;

    let message: Value = Client::new()
        .post(format!("{}/message", serve.base_url))
        .json(&serde_json::json!({ "prompt": "hello from env override" }))
        .send()
        .await
        .expect("POST /message")
        .json()
        .await
        .expect("message json");
    assert_eq!(
        message.get("response").and_then(Value::as_str),
        Some("hello from env override")
    );
}

#[tokio::test]
async fn roko_agent_serve_registers_with_relay_and_handles_messages() {
    let workdir = TempDir::new().expect("tempdir");
    write_agent_config(workdir.path());
    let relay = TestRelay::spawn().await;

    let mut config = AgentServeConfig::new("relay-demo");
    config.relay_url = Some(relay.base_url.as_str());
    let serve = spawn_roko_agent_serve_on_random_port(workdir.path(), config);

    let connected = relay.wait_for_agent("relay-demo").await;
    let expected_card_uri = format!("{}/relay/cards/relay-demo", relay.base_url);
    assert!(connected.relay_backed);
    assert_eq!(connected.rest_endpoint, None);
    assert_eq!(
        connected.card_uri.as_deref(),
        Some(expected_card_uri.as_str())
    );

    let direct_message: Value = Client::new()
        .post(format!("{}/message", serve.base_url))
        .json(&serde_json::json!({ "prompt": "hello direct" }))
        .send()
        .await
        .expect("POST /message")
        .json()
        .await
        .expect("direct message json");
    assert_eq!(
        direct_message.get("response").and_then(Value::as_str),
        Some("hello direct")
    );

    let relay_message: Value = relay
        .client
        .post(format!("{}/relay/messages", relay.base_url))
        .json(&serde_json::json!({
            "agent_id": "relay-demo",
            "message": { "prompt": "hello over relay" },
            "timeout_ms": 5_000
        }))
        .send()
        .await
        .expect("POST /relay/messages")
        .json()
        .await
        .expect("relay message json");
    assert_eq!(
        relay_message["agent_id"],
        Value::String("relay-demo".to_string())
    );
    assert_eq!(
        relay_message["response"]["response"],
        Value::String("hello over relay".to_string())
    );
}
