use crate::Agent;
use crate::claude_cli_agent::{ClaudeCliAgent, build_settings_json};
use crate::provider::{AgentCreationError, AgentOptions, ProviderAdapter, ProviderError};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig};
use serde_json::Value;
use std::path::PathBuf;

/// Adapter for the `claude` CLI subprocess protocol.
pub struct ClaudeCliAdapter;

impl ProviderAdapter for ClaudeCliAdapter {
    fn kind(&self) -> ProviderKind {
        ProviderKind::ClaudeCli
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
            .filter(|command| !command.is_empty())
            .ok_or_else(|| AgentCreationError::MissingConfig("providers.*.command".to_string()))?;

        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let timeout_ms = options
            .timeout_ms
            .or(provider.timeout_ms)
            .unwrap_or(120_000);

        let mut agent = ClaudeCliAgent::new(command, current_dir, model.slug.clone())
            .with_timeout_ms(timeout_ms)
            .with_settings_json(build_settings_json())
            .with_bare_mode(options.bare_mode)
            .with_dangerously_skip_permissions(options.dangerously_skip_permissions);

        if let Some(args) = &provider.args {
            agent = agent.with_extra_args(args.clone());
        }
        if let Some(prompt) = &options.system_prompt {
            agent = agent.with_system_prompt(prompt.clone());
        }
        if let Some(tools) = &options.tools {
            agent = agent.with_tools(tools.clone());
        }
        if let Some(mcp_config) = &options.mcp_config {
            agent = agent.with_mcp_config(mcp_config.clone());
        }
        if let Some(effort) = &options.effort {
            agent = agent.with_effort(effort.clone());
        }
        if !options.name.is_empty() {
            agent = agent.with_name(options.name.clone());
        }
        if !options.extra_args.is_empty() {
            agent = agent.with_extra_args(options.extra_args.clone());
        }
        for (key, value) in &options.env {
            agent = agent.with_env_var(key.clone(), value.clone());
        }

        Ok(Box::new(agent))
    }

    fn classify_error(&self, status: u16, body: &Value) -> ProviderError {
        match status {
            429 => ProviderError::RateLimit {
                retry_after_ms: body
                    .pointer("/retry_after")
                    .and_then(|value| value.as_u64())
                    .map(|seconds| seconds * 1000),
            },
            401 | 403 => ProviderError::AuthFailure,
            404 => ProviderError::ModelNotFound,
            408 => ProviderError::Timeout,
            500..=599 => ProviderError::ServerError(status),
            _ => ProviderError::Other(format!("HTTP {}", status)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            provider_routing: None,
            tool_format: "anthropic_blocks".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            max_tools: None,
            tokenizer_ratio: None,
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        }
    }

    #[tokio::test]
    async fn claude_cli_adapter_creates_agent_with_all_options_applied() {
        let tmp = tempdir().expect("tempdir");
        let script = tmp.path().join("claude-fake.sh");
        let args_file = tmp.path().join("args.txt");
        let prompt_file = tmp.path().join("prompt.txt");
        let env_file = tmp.path().join("env.txt");
        let mcp_config = tmp.path().join("mcp.json");
        fs::write(&mcp_config, "{}").expect("write mcp config");
        let mcp_config_arg = mcp_config.clone();

        let script_body = format!(
            r#"#!/bin/sh
set -eu
args_file="{args_file}"
prompt_file="{prompt_file}"
env_file="{env_file}"
printf '%s\n' "$@" > "$args_file"
printf '%s\n' "${{CLAUDE_TEST_ENV-}}" > "$env_file"
cat > "$prompt_file"
printf '%s\n' '{{"type":"content_block_delta","delta":{{"text":"adapter-ok"}}}}'
"#,
            args_file = args_file.display(),
            prompt_file = prompt_file.display(),
            env_file = env_file.display(),
        );
        write_script(&script, &script_body);

        let provider = ProviderConfig {
            kind: ProviderKind::ClaudeCli,
            base_url: None,
            api_key_env: None,
            command: Some(script.display().to_string()),
            args: Some(vec![
                "--provider-flag".to_string(),
                "provider-value".to_string(),
            ]),
            timeout_ms: Some(2_500),
            extra_headers: None,
            max_concurrent: None,
        };
        let options = AgentOptions {
            timeout_ms: Some(1_500),
            system_prompt: Some("system guidance".to_string()),
            tools: Some("Read,Edit".to_string()),
            mcp_config: Some(mcp_config_arg),
            env: vec![("CLAUDE_TEST_ENV".to_string(), "env-value".to_string())],
            extra_args: vec!["--option-flag".to_string(), "option-value".to_string()],
            effort: Some("high".to_string()),
            bare_mode: false,
            dangerously_skip_permissions: false,
            name: "claude-cli-adapter".to_string(),
        };
        let model = claude_model();

        let adapter = ClaudeCliAdapter;
        assert_eq!(adapter.kind(), ProviderKind::ClaudeCli);

        let agent = adapter
            .create_agent(&provider, &model, &options)
            .expect("create agent");
        assert_eq!(agent.name(), "claude-cli-adapter");

        let result = agent.run(&prompt("hello"), &Context::now()).await;
        assert!(
            result.success,
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );
        assert_eq!(result.output.body.as_text().unwrap_or(""), "adapter-ok");

        let args_text = fs::read_to_string(&args_file).expect("read args");
        assert!(args_text.contains("--provider-flag"));
        assert!(args_text.contains("provider-value"));
        assert!(args_text.contains("--option-flag"));
        assert!(args_text.contains("option-value"));
        assert!(args_text.contains("--model"));
        assert!(args_text.contains("claude-sonnet-4-6"));
        assert!(args_text.contains("--effort"));
        assert!(args_text.contains("high"));
        assert!(args_text.contains("--settings"));
        assert!(args_text.contains("--append-system-prompt"));
        assert!(args_text.contains("system guidance"));
        assert!(args_text.contains("--allowedTools"));
        assert!(args_text.contains("Read,Edit"));
        assert!(args_text.contains("--mcp-config"));
        assert!(args_text.contains(mcp_config.to_str().expect("mcp path")));
        assert!(args_text.contains("--strict-mcp-config"));
        assert!(!args_text.contains("--bare"));
        assert!(!args_text.contains("--dangerously-skip-permissions"));

        let provider_pos = args_text.find("--provider-flag").expect("provider args");
        let option_pos = args_text.find("--option-flag").expect("option args");
        assert!(provider_pos < option_pos);

        let prompt_text = fs::read_to_string(&prompt_file).expect("read prompt");
        assert_eq!(prompt_text, "hello");
        let env_text = fs::read_to_string(&env_file).expect("read env");
        assert_eq!(env_text.trim(), "env-value");
    }

    #[tokio::test]
    async fn claude_cli_adapter_timeout_comes_from_agent_options() {
        let tmp = tempdir().expect("tempdir");
        let script = tmp.path().join("claude-fake.sh");
        let script_body = r#"#!/bin/sh
set -eu
sleep 1
printf '%s\n' '{"type":"content_block_delta","delta":{"text":"late"}}'
"#;
        write_script(&script, script_body);

        let provider = ProviderConfig {
            kind: ProviderKind::ClaudeCli,
            base_url: None,
            api_key_env: None,
            command: Some(script.display().to_string()),
            args: None,
            timeout_ms: Some(1_000),
            extra_headers: None,
            max_concurrent: None,
        };
        let options = AgentOptions {
            timeout_ms: Some(100),
            name: "claude-cli-timeout".to_string(),
            ..Default::default()
        };
        let model = claude_model();

        let adapter = ClaudeCliAdapter;
        let agent = adapter
            .create_agent(&provider, &model, &options)
            .expect("create agent");

        let result = agent.run(&prompt("slow"), &Context::now()).await;
        assert!(!result.success);
        assert!(
            result
                .output
                .body
                .as_text()
                .unwrap_or("")
                .contains("timed out after 100 ms"),
            "{}",
            result.output.body.as_text().unwrap_or("unknown")
        );
    }
}
