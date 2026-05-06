use crate::Agent;
use crate::cursor_cli_agent::CursorCliAgent;
use crate::provider::{AgentCreationError, AgentOptions, ProviderAdapter, ProviderError};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS;
use serde_json::Value;
use std::path::PathBuf;

/// Adapter for the Cursor ACP subprocess protocol (`agent --force acp`).
pub struct CursorCliAdapter;

impl ProviderAdapter for CursorCliAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::CursorCli
    }

    fn create_agent(
        &self,
        provider: &ProviderConfig,
        model: &ModelProfile,
        options: &AgentOptions,
    ) -> Result<Box<dyn Agent>, AgentCreationError> {
        if provider.kind != self.kind() {
            return Err(AgentCreationError::InvalidKind(provider.kind));
        }

        let command = provider
            .command
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .unwrap_or("agent");

        let working_dir = options
            .working_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let timeout_ms = options
            .timeout_ms
            .or(provider.timeout_ms)
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS);

        let mut agent = CursorCliAgent::new(command, working_dir).with_timeout_ms(timeout_ms);

        if !model.slug.is_empty() {
            agent = agent.with_model(model.slug.clone());
        }
        if !options.name.is_empty() {
            agent = agent.with_name(options.name.clone());
        }

        Ok(Box::new(agent))
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
        let stderr = body
            .as_str()
            .or_else(|| body.pointer("/error").and_then(Value::as_str))
            .or_else(|| body.pointer("/message").and_then(Value::as_str))
            .unwrap_or("");
        let lower = stderr.to_ascii_lowercase();

        if lower.contains("rate limit") {
            return ProviderError::RateLimit {
                retry_after_ms: None,
            };
        }
        if lower.contains("unauthorized") || lower.contains("permission denied") {
            return ProviderError::AuthFailure;
        }
        if lower.contains("timed out") || lower.contains("timeout") {
            return ProviderError::Timeout;
        }

        match status {
            429 => ProviderError::RateLimit {
                retry_after_ms: None,
            },
            401 | 403 => ProviderError::AuthFailure,
            404 => ProviderError::ModelNotFound,
            408 => ProviderError::Timeout,
            500..=599 => ProviderError::ServerError(status),
            _ => {
                if stderr.is_empty() {
                    ProviderError::Other(format!("CLI exit status {status}"))
                } else {
                    ProviderError::Other(stderr.to_string())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::config::DEFAULT_TTFT_TIMEOUT_MS;
    use roko_core::{Body, Context, Kind, Signal};
    use std::fs;
    use tempfile::tempdir;

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn write_script(path: &std::path::Path, body: &str) {
        fs::write(path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).expect("script metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("chmod script");
        }
    }

    fn cursor_cli_model() -> ModelProfile {
        ModelProfile {
            provider: "cursor_cli".to_string(),
            slug: "composer-2-fast".to_string(),
            context_window: 200_000,
            max_output: Some(8_192),
            supports_tools: true,
            ..Default::default()
        }
    }

    fn mock_script_body() -> String {
        r#"#!/usr/bin/env python3
import sys
import json

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except json.JSONDecodeError:
        continue
    method = msg.get("method", "")
    msg_id = msg.get("id", 0)

    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": msg_id, "result": {"protocolVersion": 1}}), flush=True)
    elif method == "session/new":
        print(json.dumps({"jsonrpc": "2.0", "id": msg_id, "result": {"sessionId": "adapter-sess"}}), flush=True)
    elif method == "session/prompt":
        print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "adapter-ok"}}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "id": msg_id, "result": {"stopReason": "end_turn"}}), flush=True)
"#
        .to_string()
    }

    #[tokio::test]
    async fn cursor_cli_adapter_creates_agent_and_runs() {
        let tmp = tempdir().expect("tempdir");
        let script = tmp.path().join("mock-agent.sh");
        write_script(&script, &mock_script_body());

        let provider = ProviderConfig {
            kind: ProviderKind::CursorCli,
            base_url: None,
            api_key_env: None,
            command: Some(script.display().to_string()),
            args: None,
            timeout_ms: Some(10_000),
            ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
            connect_timeout_ms: None,
            extra_headers: None,
            max_concurrent: None,
        };
        let options = AgentOptions {
            timeout_ms: Some(10_000),
            name: "cursor-cli-adapter".to_string(),
            ..Default::default()
        };
        let model = cursor_cli_model();

        let adapter = CursorCliAdapter;
        assert_eq!(adapter.kind(), ProviderKind::CursorCli);

        let agent = adapter
            .create_agent(&provider, &model, &options)
            .expect("create agent");
        assert_eq!(agent.name(), "cursor-cli-adapter");

        let result = agent.run(&prompt("hello"), &Context::now()).await;
        assert!(
            result.success,
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );
        assert_eq!(result.output.body.as_text().unwrap_or(""), "adapter-ok");
    }

    #[tokio::test]
    async fn cursor_cli_adapter_defaults_command_to_agent() {
        let provider = ProviderConfig {
            kind: ProviderKind::CursorCli,
            base_url: None,
            api_key_env: None,
            command: None, // No command specified.
            args: None,
            timeout_ms: Some(1_000),
            ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
            connect_timeout_ms: None,
            extra_headers: None,
            max_concurrent: None,
        };
        let options = AgentOptions {
            name: "default-cmd".to_string(),
            ..Default::default()
        };
        let model = cursor_cli_model();

        let adapter = CursorCliAdapter;
        // Should succeed construction even without command (defaults to "agent").
        let agent = adapter.create_agent(&provider, &model, &options);
        assert!(agent.is_ok());
        assert_eq!(agent.unwrap().name(), "default-cmd");
    }
}
