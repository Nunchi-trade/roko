//! Shared helpers for command-backed agent configuration and roko.toml lookups.

use std::path::Path;

use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_core::defaults::{
    DEFAULT_CONNECT_TIMEOUT_MS, DEFAULT_REQUEST_TIMEOUT_MS, DEFAULT_TTFT_TIMEOUT_MS,
};

fn read_roko_toml(workdir: &Path) -> Option<String> {
    let config_path = workdir.join("roko.toml");
    std::fs::read_to_string(config_path).ok()
}

/// Read model from roko.toml config if available.
#[must_use]
pub fn model_from_config(workdir: &Path) -> Option<String> {
    let content = read_roko_toml(workdir)?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("model") {
            let rest = rest.trim().strip_prefix('=')?;
            let rest = rest.trim().trim_matches('"');
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Read agent command from roko.toml config if available.
#[must_use]
pub fn command_from_config(workdir: &Path) -> Option<String> {
    let content = read_roko_toml(workdir)?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("command") {
            let rest = rest.trim().strip_prefix('=')?;
            let rest = rest.trim().trim_matches('"');
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Gateway env vars extracted from roko.toml agent.env.
pub struct GatewayEnv {
    /// Key-value pairs to set on child processes.
    pub vars: Vec<(String, String)>,
}

/// Load gateway env vars from roko.toml's agent.env entries.
#[must_use]
pub fn load_gateway_env(workdir: &Path) -> GatewayEnv {
    let Some(content) = read_roko_toml(workdir) else {
        return GatewayEnv { vars: Vec::new() };
    };
    let mut vars = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.contains("ANTHROPIC_") {
            let inner = line.trim_matches(|c| c == '[' || c == ']');
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() == 2 {
                let key = parts[0].trim().trim_matches('"');
                let val = parts[1].trim().trim_matches('"');
                if !key.is_empty() {
                    vars.push((key.to_string(), val.to_string()));
                }
            }
        }
    }
    GatewayEnv { vars }
}

fn provider_kind_for_command(command: &str) -> ProviderKind {
    let executable = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command);

    match executable {
        "claude" => ProviderKind::ClaudeCli,
        "cursor-agent" | "cursor_agent" => ProviderKind::CursorAcp,
        "codex" => ProviderKind::OpenAiCompat,
        _ => ProviderKind::ClaudeCli,
    }
}

fn provider_api_key_env(kind: ProviderKind) -> Option<String> {
    match kind {
        ProviderKind::OpenAiCompat => Some("OPENAI_API_KEY".to_string()),
        ProviderKind::AnthropicApi => Some("ANTHROPIC_API_KEY".to_string()),
        ProviderKind::PerplexityApi => Some("PERPLEXITY_API_KEY".to_string()),
        ProviderKind::GeminiApi => Some("GEMINI_API_KEY".to_string()),
        ProviderKind::CerebrasApi => Some("CEREBRAS_API_KEY".to_string()),
        ProviderKind::ClaudeCli | ProviderKind::CursorAcp => None,
    }
}

fn provider_command(kind: ProviderKind, command: &str) -> Option<String> {
    matches!(kind, ProviderKind::ClaudeCli | ProviderKind::CursorAcp).then(|| command.to_string())
}

fn command_backed_config(command: &str, model: &str, kind: ProviderKind) -> RokoConfig {
    let provider_id = kind.label().to_string();
    let mut config = RokoConfig::default();
    config.agent.command = Some(command.to_string());
    config.agent.default_model = model.to_string();
    config.agent.default_backend = command.to_string();
    config.providers.insert(
        provider_id.clone(),
        ProviderConfig {
            kind,
            base_url: None,
            api_key_env: provider_api_key_env(kind),
            command: provider_command(kind, command),
            args: None,
            timeout_ms: Some(DEFAULT_REQUEST_TIMEOUT_MS),
            ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
            connect_timeout_ms: Some(DEFAULT_CONNECT_TIMEOUT_MS),
            extra_headers: None,
            max_concurrent: None,
        },
    );
    config.models.insert(
        model.to_string(),
        ModelProfile {
            provider: provider_id,
            slug: model.to_string(),
            ..Default::default()
        },
    );
    config
}

/// Build an explicit transient Claude CLI config for direct command-backed execution.
#[must_use]
pub fn synthesize_claude_cli_config(command: &str, model: &str) -> RokoConfig {
    let mut config = command_backed_config(command, model, ProviderKind::ClaudeCli);
    config.agent.default_backend = "claude".to_string();
    config
}

/// Build an explicit transient known-protocol config for direct command-backed execution.
#[must_use]
pub fn synthesize_known_protocol_config(command: &str, model: &str) -> RokoConfig {
    command_backed_config(command, model, provider_kind_for_command(command))
}

/// Build a synthetic generic subprocess config for direct command-backed execution.
#[must_use]
pub fn synthesize_subprocess_config(command: &str) -> RokoConfig {
    let mut config = RokoConfig::default();
    config.agent.command = Some(command.to_string());
    config
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        GatewayEnv, RokoConfig, command_from_config, load_gateway_env, model_from_config,
        synthesize_claude_cli_config, synthesize_known_protocol_config,
        synthesize_subprocess_config,
    };

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_workdir() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir =
            std::env::temp_dir().join(format!("roko-agent-config-test-{pid}-{unique}-{counter}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_roko_toml(workdir: &std::path::Path, content: &str) {
        std::fs::write(workdir.join("roko.toml"), content).unwrap();
    }

    #[test]
    fn synthesize_claude_cli_config_sets_backend_and_model() {
        let config = synthesize_claude_cli_config("claude", "claude-opus-4-6");
        assert_eq!(config.agent.command.as_deref(), Some("claude"));
        assert_eq!(config.agent.default_model, "claude-opus-4-6");
        assert_eq!(config.agent.default_backend, "claude");
        assert_eq!(
            config
                .providers
                .get("claude_cli")
                .and_then(|provider| provider.command.as_deref()),
            Some("claude")
        );
        assert_eq!(
            config
                .models
                .get("claude-opus-4-6")
                .map(|model| model.provider.as_str()),
            Some("claude_cli")
        );
    }

    #[test]
    fn synthesize_known_protocol_config_uses_command_as_backend() {
        let config = synthesize_known_protocol_config("cursor-agent", "composer-2-fast");
        assert_eq!(config.agent.command.as_deref(), Some("cursor-agent"));
        assert_eq!(config.agent.default_model, "composer-2-fast");
        assert_eq!(config.agent.default_backend, "cursor-agent");
        assert_eq!(
            config
                .providers
                .get("cursor_acp")
                .and_then(|provider| provider.command.as_deref()),
            Some("cursor-agent")
        );
        assert_eq!(
            config
                .models
                .get("composer-2-fast")
                .map(|model| model.provider.as_str()),
            Some("cursor_acp")
        );
    }

    #[test]
    fn synthesize_subprocess_config_only_sets_command() {
        let config = synthesize_subprocess_config("cursor-agent");
        assert_eq!(config.agent.command.as_deref(), Some("cursor-agent"));
        let defaults = RokoConfig::default();
        assert_eq!(config.agent.default_model, defaults.agent.default_model);
        assert_eq!(config.agent.default_backend, defaults.agent.default_backend);
    }

    #[test]
    fn model_and_command_from_config_read_simple_values() {
        let workdir = temp_workdir();
        write_roko_toml(
            &workdir,
            r#"
model = "claude-opus-4-6"
command = "claude"
"#,
        );

        assert_eq!(
            model_from_config(&workdir).as_deref(),
            Some("claude-opus-4-6")
        );
        assert_eq!(command_from_config(&workdir).as_deref(), Some("claude"));
    }

    #[test]
    fn load_gateway_env_collects_anthropic_entries() {
        let workdir = temp_workdir();
        write_roko_toml(
            &workdir,
            r#"
["ANTHROPIC_BASE_URL", "https://example.test"]
["ANTHROPIC_AUTH_TOKEN", "secret"]
["IGNORED_KEY", "value"]
"#,
        );

        let GatewayEnv { vars } = load_gateway_env(&workdir);
        assert_eq!(
            vars,
            vec![
                (
                    "ANTHROPIC_BASE_URL".to_string(),
                    "https://example.test".to_string()
                ),
                ("ANTHROPIC_AUTH_TOKEN".to_string(), "secret".to_string()),
            ]
        );
    }
}
