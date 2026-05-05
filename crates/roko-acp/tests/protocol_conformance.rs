use std::collections::VecDeque;

use anyhow::{Result, anyhow, bail};
use roko_acp::{
    AcpConfig,
    handler::run_acp_server_with_transport,
    transport::StdioTransport,
    types::{METHOD_NOT_FOUND, PARSE_ERROR, SESSION_NOT_FOUND},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex},
    task::JoinHandle,
};

struct TestHarness {
    client: TestClient,
    server_task: JoinHandle<Result<()>>,
}

impl TestHarness {
    async fn new() -> Self {
        Self::new_with_config(AcpConfig::default()).await
    }

    async fn new_with_config(config: AcpConfig) -> Self {
        let (client_input, server_input) = duplex(16 * 1024);
        let (server_output, client_output) = duplex(16 * 1024);
        let mut transport = StdioTransport::from_io(server_input, server_output);
        let server_task = tokio::spawn(async move {
            run_acp_server_with_transport(config, &mut transport).await
        });

        Self {
            client: TestClient::new(client_input, client_output),
            server_task,
        }
    }

    async fn shutdown(self) -> Result<()> {
        drop(self.client);
        self.server_task.await.map_err(|error| anyhow!(error))?
    }
}

struct TestClient {
    writer: DuplexStream,
    reader: BufReader<DuplexStream>,
    next_id: u64,
    pending_notifications: VecDeque<(String, Value)>,
}

impl TestClient {
    fn new(writer: DuplexStream, reader: DuplexStream) -> Self {
        Self {
            writer,
            reader: BufReader::new(reader),
            next_id: 1,
            pending_notifications: VecDeque::new(),
        }
    }

    async fn initialize(&mut self) -> Result<Value> {
        self.send_request("initialize", json!({ "protocolVersion": 1 }))
            .await
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.start_request(method, params).await?;
        self.read_response(id).await
    }

    async fn start_request(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        self.write_json_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;
        Ok(id)
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        self.write_json_line(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn send_raw_line(&mut self, line: &str) -> Result<()> {
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    async fn read_response(&mut self, expected_id: u64) -> Result<Value> {
        loop {
            let message = self.read_message().await?;
            if let Some(method) = message.get("method").and_then(Value::as_str) {
                self.pending_notifications.push_back((
                    method.to_owned(),
                    message.get("params").cloned().unwrap_or(Value::Null),
                ));
                continue;
            }

            let actual_id = message
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("response did not contain numeric id: {message}"))?;
            if actual_id == expected_id {
                return Ok(message);
            }
        }
    }

    async fn read_notification(&mut self) -> Result<(String, Value)> {
        if let Some(notification) = self.pending_notifications.pop_front() {
            return Ok(notification);
        }

        loop {
            let message = self.read_message().await?;
            if let Some(method) = message.get("method").and_then(Value::as_str) {
                return Ok((
                    method.to_owned(),
                    message.get("params").cloned().unwrap_or(Value::Null),
                ));
            }
        }
    }

    async fn read_message(&mut self) -> Result<Value> {
        let mut line = String::new();
        let bytes_read = self.reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            bail!("server closed the stream before sending a message");
        }
        Ok(serde_json::from_str(&line)?)
    }

    async fn write_json_line(&mut self, value: &Value) -> Result<()> {
        let bytes = serde_json::to_vec(value)?;
        self.writer.write_all(&bytes).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}

fn response_result(message: &Value) -> &Value {
    message
        .get("result")
        .unwrap_or_else(|| panic!("missing result payload: {message}"))
}

fn response_error(message: &Value) -> &Value {
    message
        .get("error")
        .unwrap_or_else(|| panic!("missing error payload: {message}"))
}

async fn create_session(client: &mut TestClient, name: &str) -> Result<String> {
    let response = client
        .send_request(
            "session/new",
            json!({
                "sessionName": name,
                "mcpServers": [],
            }),
        )
        .await?;
    let result = response_result(&response);
    let session_id = result
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("session/new result did not include sessionId: {response}"))?;
    Ok(session_id.to_owned())
}

#[tokio::test]
async fn test_initialize() -> Result<()> {
    let mut harness = TestHarness::new().await;

    let response = harness.client.initialize().await?;
    let result = response_result(&response);

    assert_eq!(result["protocolVersion"], json!(1));
    assert_eq!(result["agentInfo"]["name"], json!("roko"));
    assert_eq!(result["agentCapabilities"]["loadSession"], json!(true));

    harness.shutdown().await
}

#[tokio::test]
async fn test_session_new() -> Result<()> {
    let mut harness = TestHarness::new().await;
    harness.client.initialize().await?;

    let response = harness
        .client
        .send_request(
            "session/new",
            json!({
                "sessionName": "alpha",
                "mcpServers": [],
            }),
        )
        .await?;
    let result = response_result(&response);

    let session_id = result["sessionId"]
        .as_str()
        .ok_or_else(|| anyhow!("missing sessionId in response: {response}"))?;
    assert!(session_id.starts_with("sess_"));
    assert!(result["configOptions"].is_array());

    harness.shutdown().await
}

#[tokio::test]
async fn test_session_list() -> Result<()> {
    let mut harness = TestHarness::new().await;
    harness.client.initialize().await?;

    let first = create_session(&mut harness.client, "alpha").await?;
    let second = create_session(&mut harness.client, "beta").await?;
    let response = harness
        .client
        .send_request("session/list", json!({}))
        .await?;
    let sessions = response_result(&response)["sessions"]
        .as_array()
        .ok_or_else(|| anyhow!("session/list result did not include sessions: {response}"))?;

    assert!(
        sessions.len() >= 2,
        "expected at least 2 sessions, got {}",
        sessions.len()
    );
    assert!(
        sessions
            .iter()
            .any(|session| session["sessionId"] == json!(first))
    );
    assert!(
        sessions
            .iter()
            .any(|session| session["sessionId"] == json!(second))
    );

    harness.shutdown().await
}

#[tokio::test]
async fn test_session_prompt_basic() -> Result<()> {
    let mut harness = TestHarness::new().await;
    harness.client.initialize().await?;
    let session_id = create_session(&mut harness.client, "prompt-basic").await?;

    let request_id = harness
        .client
        .start_request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "hello" }],
                "includeContext": false,
            }),
        )
        .await?;

    // Read notifications until we get an agent content chunk (skip command updates etc).
    loop {
        let (method, params) = harness.client.read_notification().await?;
        assert_eq!(method, "session/update");
        assert!(
            params["sessionId"].is_string(),
            "notification must include sessionId"
        );
        let update = &params["update"];
        let update_type = update["sessionUpdate"]
            .as_str()
            .expect("update must have sessionUpdate discriminant");
        if update_type == "agent_message_chunk" || update_type == "agent_thought_chunk" {
            assert_eq!(update["content"]["type"], json!("text"));
            break;
        }
        // Skip available_commands_update and other meta-notifications.
    }

    let response = harness.client.read_response(request_id).await?;
    let result = response_result(&response);
    let stop = result["stopReason"].as_str().expect("must have stopReason");
    assert!(
        stop == "end_turn" || stop == "cancelled",
        "unexpected stopReason: {stop}"
    );

    harness.shutdown().await
}

#[tokio::test]
async fn test_session_cancel() -> Result<()> {
    let mut harness = TestHarness::new().await;
    harness.client.initialize().await?;
    let session_id = create_session(&mut harness.client, "cancel").await?;

    let request_id = harness
        .client
        .start_request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "cancel me" }],
                "includeContext": false,
            }),
        )
        .await?;

    // Read at least one session/update notification (skip command updates).
    loop {
        let (method, params) = harness.client.read_notification().await?;
        assert_eq!(method, "session/update");
        let update_type = params
            .pointer("/update/sessionUpdate")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if update_type != "available_commands_update" {
            break;
        }
    }
    harness
        .client
        .send_notification(
            "session/cancel",
            json!({
                "sessionId": session_id,
            }),
        )
        .await?;

    let response = harness.client.read_response(request_id).await?;
    let result = response_result(&response);
    let stop = result["stopReason"].as_str().expect("must have stopReason");
    // The prompt may finish before the cancel is processed, so accept either.
    assert!(
        stop == "cancelled" || stop == "end_turn",
        "unexpected stopReason: {stop}"
    );

    harness.shutdown().await
}

#[tokio::test]
async fn test_unknown_method() -> Result<()> {
    let mut harness = TestHarness::new().await;

    let response = harness
        .client
        .send_request("nope/method", json!({}))
        .await?;

    assert_eq!(response_error(&response)["code"], json!(METHOD_NOT_FOUND));

    harness.shutdown().await
}

#[tokio::test]
async fn test_invalid_session() -> Result<()> {
    let mut harness = TestHarness::new().await;
    harness.client.initialize().await?;

    let response = harness
        .client
        .send_request(
            "session/prompt",
            json!({
                "sessionId": "sess_missing",
                "prompt": [{ "type": "text", "text": "hello" }],
                "includeContext": false,
            }),
        )
        .await?;

    assert_eq!(response_error(&response)["code"], json!(SESSION_NOT_FOUND));

    harness.shutdown().await
}

#[tokio::test]
async fn test_malformed_json() -> Result<()> {
    let mut harness = TestHarness::new().await;

    harness.client.send_raw_line("{").await?;
    let response = harness.client.read_message().await?;

    assert_eq!(response_error(&response)["code"], json!(PARSE_ERROR));
    assert!(response["id"].is_null());

    harness.shutdown().await
}

// --- Startup resilience tests (task 073) ---

#[tokio::test]
async fn initialize_with_no_roko_toml_returns_empty_warnings() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // No roko.toml in dir -- use defaults.
    // Use a nonexistent global config to isolate from the developer machine.
    let config = AcpConfig::new(
        dir.path(),
        "default",
        None,
        dir.path().join(".roko/acp.log"),
    )
    .with_global_config(Some(dir.path().join("nonexistent-global.toml")));

    let mut harness = TestHarness::new_with_config(config).await;
    let response = harness.client.initialize().await?;
    let result = response_result(&response);

    // No roko.toml = no config parse warnings. Provider warning may or may not
    // be present depending on env, but configWarnings must be absent (empty) or
    // an array per the skip_serializing_if contract.
    let warnings = &result["configWarnings"];
    assert!(
        warnings.is_null() || warnings.is_array(),
        "configWarnings must be null (omitted) or an array, got: {warnings}"
    );

    harness.shutdown().await
}

#[tokio::test]
async fn initialize_with_malformed_roko_toml_returns_config_warning() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // Write invalid TOML.
    std::fs::write(dir.path().join("roko.toml"), "this is { not valid toml")?;

    let config = AcpConfig::new(
        dir.path(),
        "default",
        None,
        dir.path().join(".roko/acp.log"),
    )
    .with_global_config(Some(dir.path().join("nonexistent-global.toml")));

    let mut harness = TestHarness::new_with_config(config).await;
    let response = harness.client.initialize().await?;
    let result = response_result(&response);

    let warnings = result["configWarnings"]
        .as_array()
        .expect("configWarnings must be a non-empty array");
    assert!(
        !warnings.is_empty(),
        "expected a config parse warning for malformed roko.toml"
    );
    let warning_text = warnings[0].as_str().unwrap_or_default();
    assert!(
        warning_text.contains("parse error"),
        "warning should mention parse error, got: {warning_text}"
    );

    harness.shutdown().await
}

#[tokio::test]
async fn initialize_with_unavailable_provider_credentials_returns_warning() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // Write a valid roko.toml with a provider that references a missing env var.
    std::fs::write(
        dir.path().join("roko.toml"),
        r#"
config_version = 2
schema_version = 2

[providers.test-provider]
kind = "openai_compat"
base_url = "https://api.example.com/v1"
api_key_env = "ROKO_TEST_MISSING_KEY_XYZ_DOES_NOT_EXIST"
"#,
    )?;

    let config = AcpConfig::new(
        dir.path(),
        "default",
        None,
        dir.path().join(".roko/acp.log"),
    )
    .with_global_config(Some(dir.path().join("nonexistent-global.toml")));

    let mut harness = TestHarness::new_with_config(config).await;
    let response = harness.client.initialize().await?;
    let result = response_result(&response);

    let warnings = result["configWarnings"]
        .as_array()
        .expect("configWarnings must be a non-empty array");
    assert!(
        !warnings.is_empty(),
        "expected a provider readiness warning"
    );
    let has_credential_warning = warnings.iter().any(|w| {
        let text = w.as_str().unwrap_or_default();
        text.contains("credentials") || text.contains("api_key_env")
    });
    assert!(
        has_credential_warning,
        "expected a provider credentials warning in: {warnings:?}"
    );

    harness.shutdown().await
}
