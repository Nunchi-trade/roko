//! Auth auto-detection for the unified CLI experience.
//!
//! Probes available authentication methods in priority order:
//! 1. Config-aware: loads roko.toml, checks each configured provider's API key
//! 2. Env-var fallback: `claude` CLI → `ANTHROPIC_API_KEY` → `ZAI_API_KEY` → `OPENAI_API_KEY`
//! 3. Falls back to `NeedsSetup`

use std::path::Path;
use std::process::Command;

use roko_core::agent::ProviderKind;

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
        /// Model to use (e.g. "gpt-5.4-mini", "glm-5.1"). Falls back to "gpt-5.4-mini".
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
                    format!("{} (Anthropic API)", roko_core::defaults::MODEL_FOCUSED)
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

/// Detect which auth method will ACTUALLY be used for dispatch.
///
/// Loads `roko.toml` from `workdir` if available and checks each configured
/// provider's `api_key_env` in order. Returns the first provider with valid
/// credentials. Falls back to `detect_auth_from_env()` when config is missing
/// or no configured provider has credentials.
pub fn detect_auth_from_config(workdir: &Path) -> AuthMethod {
    if let Ok(config) = roko_core::config::loader::load_config_unified(workdir) {
        if let Some(method) = detect_from_config(&config) {
            return method;
        }
    }
    detect_auth_from_env()
}

fn detect_from_config(config: &roko_core::config::schema::RokoConfig) -> Option<AuthMethod> {
    for (_name, provider) in &config.providers {
        // CLI providers don't need an API key — check binary availability.
        if provider.kind == ProviderKind::ClaudeCli {
            let cmd = provider.command.as_deref().unwrap_or("claude");
            if Command::new(cmd)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return Some(AuthMethod::ClaudeCli);
            }
            continue;
        }

        // API providers need a resolvable key.
        let Some(api_key_env) = provider.api_key_env.as_deref() else {
            continue;
        };
        let Some(key) = std::env::var(api_key_env).ok().filter(|k| !k.is_empty()) else {
            continue;
        };

        return Some(match provider.kind {
            ProviderKind::AnthropicApi => AuthMethod::AnthropicApi { key, model: None },
            _ => AuthMethod::OpenAiCompat {
                key,
                base_url: provider.base_url.clone().unwrap_or_default(),
                model: None,
            },
        });
    }
    None
}

/// Detect the best available authentication method using env var probing only.
///
/// Checks (in order):
/// 1. `claude` CLI — matches `roko.toml` defaults (claude-sonnet via claude_cli)
/// 2. `ANTHROPIC_API_KEY`
/// 3. `ZAI_API_KEY` (Zhipu/GLM, OpenAI-compatible)
/// 4. `OPENAI_API_KEY`
/// 5. `NeedsSetup`
///
/// Prefer `detect_auth_from_config()` for config-aware detection that matches
/// what dispatch will actually use.
pub fn detect_auth_from_env() -> AuthMethod {
    // 1. Claude CLI — lightweight probe via `claude --version`
    if let Ok(output) = Command::new("claude").arg("--version").output() {
        if output.status.success() {
            return AuthMethod::ClaudeCli;
        }
    }

    // 2. Anthropic API key
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return AuthMethod::AnthropicApi { key, model: None };
        }
    }

    // 3. Zhipu/GLM (OpenAI-compatible)
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

    // 4. OpenAI-compatible
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

    AuthMethod::NeedsSetup
}

/// Print setup instructions when no auth is detected.
pub fn print_setup_instructions() {
    eprintln!("error: no LLM provider configured.\n");
    eprintln!("To get started, either:");
    eprintln!("  1. Run `roko init` to create a workspace with default config");
    eprintln!("  2. Set ANTHROPIC_API_KEY, OPENAI_API_KEY, or ZAI_API_KEY");
    eprintln!("  3. Edit roko.toml to configure a provider");
    eprintln!("\n  hint: run `roko doctor` to diagnose your setup");
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
            format!("{} (Anthropic API)", roko_core::defaults::MODEL_FOCUSED)
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
