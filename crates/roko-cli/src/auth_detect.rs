//! Auth auto-detection for the unified CLI experience.
//!
//! Probes available authentication methods in priority order:
//! 1. `claude` CLI (logged in and reachable)
//! 2. `ANTHROPIC_API_KEY` environment variable
//! 3. `OPENAI_API_KEY` environment variable (OpenAI-compatible)
//! 4. Falls back to `NeedsSetup`

#[cfg(feature = "legacy-orchestrate")]
use std::process::Command;

/// Detected authentication method for agent dispatch.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// The `claude` CLI is installed and logged in.
    ClaudeCli,
    /// Anthropic API key from env or config.
    AnthropicApi {
        key: String,
        /// Model override (e.g. "claude-haiku-4-5"). Defaults to claude-sonnet-4-6.
        model: Option<String>,
    },
    /// OpenAI-compatible endpoint (OpenAI, Azure, local, etc.).
    OpenAiCompat {
        key: String,
        base_url: String,
        /// Model to use (e.g. "gpt-4o", "glm-5.1"). Falls back to "gpt-4o".
        model: Option<String>,
    },
    /// No auth found — user needs to set up.
    NeedsSetup,
}

impl AuthMethod {
    /// Human-readable label for status display.
    pub fn label(&self) -> String {
        match self {
            Self::ClaudeCli => "claude CLI".to_string(),
            Self::AnthropicApi { model, .. } => {
                if let Some(m) = model {
                    format!("{m} (Anthropic API)")
                } else {
                    "claude-sonnet-4-6 (Anthropic API)".to_string()
                }
            }
            Self::OpenAiCompat { model, .. } => {
                if let Some(m) = model {
                    format!("{m} (OpenAI-compat)")
                } else {
                    "OpenAI-compatible API".to_string()
                }
            }
            Self::NeedsSetup => "none".to_string(),
        }
    }
}

/// Detect the best available authentication method.
///
/// Checks (in order):
/// 1. API keys from environment (faster, more reliable than CLI probes)
/// 2. `claude` CLI as fallback
///
/// API keys are preferred because CLI probes can succeed (`claude --version`)
/// yet fail at dispatch time (login expired, rate limits, etc.).
pub fn detect_auth() -> AuthMethod {
    // 1. Zhipu/GLM (OpenAI-compatible)
    if let Ok(key) = std::env::var("ZAI_API_KEY") {
        if !key.is_empty() {
            let model = std::env::var("ZAI_MODEL").ok().filter(|s| !s.is_empty());
            return AuthMethod::OpenAiCompat {
                key,
                base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
                model: Some(model.unwrap_or_else(|| "glm-5.1".to_string())),
            };
        }
    }

    // 2. Anthropic API key
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return AuthMethod::AnthropicApi { key, model: None };
        }
    }

    // 3. OpenAI-compatible
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        if !key.is_empty() {
            let base_url = std::env::var("OPENAI_API_BASE")
                .or_else(|_| std::env::var("OPENAI_BASE_URL"))
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
            return AuthMethod::OpenAiCompat {
                key,
                base_url,
                model: None,
            };
        }
    }

    // 4. Claude CLI (legacy fallback — can succeed at version check but fail at dispatch)
    #[cfg(feature = "legacy-orchestrate")]
    if let Ok(output) = Command::new("claude").arg("--version").output() {
        if output.status.success() {
            return AuthMethod::ClaudeCli;
        }
    }

    AuthMethod::NeedsSetup
}

/// Print setup instructions when no auth is detected.
pub fn print_setup_instructions() {
    eprintln!("No authentication method detected.\n");
    eprintln!("Set up one of the following:\n");
    eprintln!("  1. Install and login to Claude CLI:");
    eprintln!("     npm install -g @anthropic-ai/claude-code && claude\n");
    eprintln!("  2. Set an Anthropic API key:");
    eprintln!("     export ANTHROPIC_API_KEY=sk-ant-...\n");
    eprintln!("  3. Set an OpenAI-compatible API key:");
    eprintln!("     export OPENAI_API_KEY=sk-...\n");
    eprintln!("Then run `roko` again.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_method_labels() {
        assert_eq!(AuthMethod::ClaudeCli.label(), "claude CLI");
        assert_eq!(
            AuthMethod::AnthropicApi {
                key: "k".into(),
                model: None,
            }
            .label(),
            "claude-sonnet-4-6 (Anthropic API)"
        );
        assert_eq!(
            AuthMethod::OpenAiCompat {
                key: "k".into(),
                base_url: "u".into(),
                model: None,
            }
            .label(),
            "OpenAI-compatible API"
        );
        assert_eq!(
            AuthMethod::OpenAiCompat {
                key: "k".into(),
                base_url: "u".into(),
                model: Some("glm-5.1".into()),
            }
            .label(),
            "glm-5.1 (OpenAI-compat)"
        );
        assert_eq!(AuthMethod::NeedsSetup.label(), "none");
    }
}
