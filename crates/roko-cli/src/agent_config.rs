//! Shared helpers for synthesized command-backed agent configurations.

use roko_core::config::schema::RokoConfig;

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
    use super::{
        synthesize_claude_cli_config, synthesize_known_protocol_config,
        synthesize_subprocess_config,
    };

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
        assert!(config.agent.default_model.is_empty());
        assert!(config.agent.default_backend.is_empty());
    }
}
