//! Shared helpers for command-backed agent configuration and roko.toml lookups.

use std::path::Path;

use roko_core::config::schema::RokoConfig;

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

/// Build a synthetic Claude CLI config for direct command-backed execution.
#[must_use]
pub fn synthesize_claude_cli_config(command: &str, model: &str) -> RokoConfig {
    let mut config = RokoConfig::default();
    config.agent.command = Some(command.to_string());
    config.agent.default_model = model.to_string();
    config.agent.default_backend = "claude".to_string();
    config
}

/// Build a synthetic known-protocol CLI config for direct command-backed execution.
#[must_use]
pub fn synthesize_known_protocol_config(command: &str, model: &str) -> RokoConfig {
    let mut config = RokoConfig::default();
    config.agent.command = Some(command.to_string());
    config.agent.default_model = model.to_string();
    config.agent.default_backend = command.to_string();
    config
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        GatewayEnv, RokoConfig, command_from_config, load_gateway_env, model_from_config,
        synthesize_claude_cli_config, synthesize_known_protocol_config,
        synthesize_subprocess_config,
    };

    fn temp_workdir() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("roko-agent-config-test-{unique}"));
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
    }

    #[test]
    fn synthesize_known_protocol_config_uses_command_as_backend() {
        let config = synthesize_known_protocol_config("gemini", "gemini-2.5-pro");
        assert_eq!(config.agent.command.as_deref(), Some("gemini"));
        assert_eq!(config.agent.default_model, "gemini-2.5-pro");
        assert_eq!(config.agent.default_backend, "gemini");
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
