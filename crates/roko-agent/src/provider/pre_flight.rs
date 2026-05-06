//! Startup pre-flight provider readiness checks.
//!
//! These checks run before any long-running CLI operation (plan run, chat,
//! prd pipeline) to detect provider misconfigurations early — before spending
//! time on context assembly and prompt building that would fail at dispatch.
//!
//! Checks are cheap: PATH lookups and environment variable presence only.
//! No network requests are made.

use roko_core::agent::ProviderKind;
use roko_core::config::schema::{ProviderConfig, RokoConfig};
use std::collections::HashSet;

/// A single provider readiness issue detected during pre-flight.
#[derive(Debug, Clone)]
pub struct ProviderReadinessIssue {
    /// The name of the provider (key in `[providers.*]`).
    pub provider_name: String,
    /// Human-readable description of the issue.
    pub message: String,
}

/// Check providers referenced by configured models for readiness.
///
/// Only warns loudly about the provider backing `default_model`. Issues for
/// other providers are logged at `debug` level to avoid noisy startup output
/// when the user has many providers configured but only uses one.
///
/// Returns an empty `Vec` when all referenced providers are ready.
///
/// No network requests are made.
#[must_use]
pub fn check_provider_readiness(config: &RokoConfig) -> Vec<ProviderReadinessIssue> {
    let mut issues = Vec::new();

    // Determine which provider backs the default model — only warn loudly about it.
    let default_provider: Option<&str> = if config.agent.default_model.is_empty() {
        None
    } else {
        config
            .models
            .get(config.agent.default_model.as_str())
            .map(|m| m.provider.as_str())
    };

    // Collect the set of provider keys referenced by model profiles.
    let referenced_providers: HashSet<&str> = config
        .models
        .values()
        .map(|m| m.provider.as_str())
        .collect();

    if referenced_providers.is_empty() {
        return issues;
    }

    let effective_providers = config.effective_providers();

    for provider_name in &referenced_providers {
        let is_default = default_provider == Some(provider_name);

        let Some(provider) = effective_providers.get(*provider_name) else {
            if is_default {
                issues.push(ProviderReadinessIssue {
                    provider_name: provider_name.to_string(),
                    message: format!(
                        "provider '{}' referenced by a model but not defined in config.",
                        provider_name
                    ),
                });
            } else {
                tracing::debug!(
                    provider = provider_name,
                    "provider referenced by a model but not defined in config (not default, suppressing warning)"
                );
            }
            continue;
        };

        if is_default {
            check_single_provider(provider_name, provider, &mut issues);
        } else {
            // Non-default providers: check silently, log at debug level.
            let mut non_default_issues = Vec::new();
            check_single_provider(provider_name, provider, &mut non_default_issues);
            for issue in &non_default_issues {
                tracing::debug!(provider = %issue.provider_name, "{}", issue.message);
            }
        }
    }

    issues
}

fn check_single_provider(
    provider_name: &str,
    provider: &ProviderConfig,
    issues: &mut Vec<ProviderReadinessIssue>,
) {
    match provider.kind {
        ProviderKind::ClaudeCli => {
            let command = provider.command.as_deref().unwrap_or("claude");
            if !binary_on_path(command) {
                issues.push(ProviderReadinessIssue {
                    provider_name: provider_name.to_string(),
                    message: "claude CLI not found on PATH. Install: https://claude.ai/cli"
                        .to_string(),
                });
            }
        }
        ProviderKind::CursorAcp => {
            // Cursor handles auth/process ownership differently.
            // Only check if an explicit command is configured.
            if let Some(ref command) = provider.command {
                if !binary_on_path(command) {
                    issues.push(ProviderReadinessIssue {
                        provider_name: provider_name.to_string(),
                        message: format!("Cursor command '{}' not found on PATH.", command),
                    });
                }
            }
        }
        ProviderKind::AnthropicApi
        | ProviderKind::OpenAiCompat
        | ProviderKind::PerplexityApi
        | ProviderKind::GeminiApi
        | ProviderKind::CerebrasApi => {
            check_api_key_env(provider_name, provider, issues);
        }
    }
}

fn check_api_key_env(
    provider_name: &str,
    provider: &ProviderConfig,
    issues: &mut Vec<ProviderReadinessIssue>,
) {
    let Some(ref env_var) = provider.api_key_env else {
        issues.push(ProviderReadinessIssue {
            provider_name: provider_name.to_string(),
            message: format!(
                "Missing api_key_env for provider '{}'. Configure it in roko.toml [providers.{}].",
                provider_name, provider_name
            ),
        });
        return;
    };

    if env_var.trim().is_empty() {
        issues.push(ProviderReadinessIssue {
            provider_name: provider_name.to_string(),
            message: format!(
                "Empty api_key_env for provider '{}'. Configure it in roko.toml [providers.{}].",
                provider_name, provider_name
            ),
        });
        return;
    }

    match std::env::var(env_var) {
        Ok(val) if val.trim().is_empty() => {
            issues.push(ProviderReadinessIssue {
                provider_name: provider_name.to_string(),
                message: format!(
                    "Missing {} for provider '{}'. Export it in your shell or in a .env file.",
                    env_var, provider_name
                ),
            });
        }
        Err(_) => {
            issues.push(ProviderReadinessIssue {
                provider_name: provider_name.to_string(),
                message: format!(
                    "Missing {} for provider '{}'. Export it in your shell or in a .env file.",
                    env_var, provider_name
                ),
            });
        }
        Ok(_) => {
            // Key is present and non-empty — ready.
        }
    }
}

/// Check whether a binary name is findable on the system PATH.
fn binary_on_path(name: &str) -> bool {
    // If the name contains a path separator, check directly.
    if name.contains('/') || name.contains('\\') {
        return std::path::Path::new(name).exists();
    }

    // Otherwise search PATH directories.
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return true;
        }
    }
    false
}

/// Print provider readiness issues to stderr and return whether all providers are blocked.
///
/// Returns `true` if every referenced provider has an issue (none are ready),
/// meaning the operation should abort. Returns `false` if at least one provider
/// is ready (fallback-capable configurations can proceed).
pub fn report_readiness_issues(issues: &[ProviderReadinessIssue], config: &RokoConfig) -> bool {
    if issues.is_empty() {
        return false;
    }

    for issue in issues {
        eprintln!("warning: {}", issue.message);
    }

    // Check if ALL referenced providers are blocked.
    let referenced_providers: HashSet<&str> = config
        .models
        .values()
        .map(|m| m.provider.as_str())
        .collect();

    let blocked_providers: HashSet<&str> =
        issues.iter().map(|i| i.provider_name.as_str()).collect();

    // If every referenced provider has at least one issue, all are blocked.
    referenced_providers
        .iter()
        .all(|p| blocked_providers.contains(p))
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use roko_core::config::schema::ModelProfile;

    #[test]
    fn readiness_empty_models_returns_no_issues() {
        let config = RokoConfig::default();
        let issues = check_provider_readiness(&config);
        assert!(issues.is_empty());
    }

    #[test]
    fn readiness_missing_provider_definition() {
        let mut config = RokoConfig::default();
        config.agent.default_model = "test-model".to_string();
        config.models.insert(
            "test-model".to_string(),
            ModelProfile {
                provider: "nonexistent".to_string(),
                slug: "test-slug".to_string(),
                ..Default::default()
            },
        );
        let issues = check_provider_readiness(&config);
        assert!(!issues.is_empty());
        assert!(issues[0].message.contains("not defined"));
    }

    #[test]
    fn readiness_missing_api_key_env_variable() {
        let mut config = RokoConfig::default();
        config.agent.default_model = "test-model".to_string();
        config.providers.insert(
            "test-api".to_string(),
            ProviderConfig {
                kind: ProviderKind::AnthropicApi,
                base_url: Some("https://api.anthropic.com/v1".to_string()),
                api_key_env: Some("ROKO_TEST_NONEXISTENT_KEY_XYZ_090".to_string()),
                command: None,
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        );
        config.models.insert(
            "test-model".to_string(),
            ModelProfile {
                provider: "test-api".to_string(),
                slug: "claude-sonnet-4-20250514".to_string(),
                ..Default::default()
            },
        );
        // Ensure the env var is NOT set.
        // SAFETY: test is single-threaded; no other thread reads this env var.
        unsafe { std::env::remove_var("ROKO_TEST_NONEXISTENT_KEY_XYZ_090") };
        let issues = check_provider_readiness(&config);
        assert!(!issues.is_empty());
        assert!(
            issues[0]
                .message
                .contains("ROKO_TEST_NONEXISTENT_KEY_XYZ_090")
        );
        assert!(issues[0].message.contains("test-api"));
    }

    #[test]
    fn readiness_claude_cli_nonexistent_command() {
        let mut config = RokoConfig::default();
        config.agent.default_model = "test-model".to_string();
        config.providers.insert(
            "claude-local".to_string(),
            ProviderConfig {
                kind: ProviderKind::ClaudeCli,
                base_url: None,
                api_key_env: None,
                command: Some("roko-nonexistent-binary-xyz-090".to_string()),
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        );
        config.models.insert(
            "test-model".to_string(),
            ModelProfile {
                provider: "claude-local".to_string(),
                slug: "claude-sonnet-4-20250514".to_string(),
                ..Default::default()
            },
        );
        let issues = check_provider_readiness(&config);
        assert!(!issues.is_empty());
        assert!(issues[0].message.contains("claude CLI not found on PATH"));
    }

    #[test]
    fn readiness_unreferenced_provider_ignored() {
        let mut config = RokoConfig::default();
        // Provider configured but no model references it.
        config.providers.insert(
            "unused-provider".to_string(),
            ProviderConfig {
                kind: ProviderKind::AnthropicApi,
                base_url: None,
                api_key_env: Some("ROKO_NONEXISTENT_UNUSED_KEY_090".to_string()),
                command: None,
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        );
        // SAFETY: test is single-threaded; no other thread reads this env var.
        unsafe { std::env::remove_var("ROKO_NONEXISTENT_UNUSED_KEY_090") };
        let issues = check_provider_readiness(&config);
        assert!(
            issues.is_empty(),
            "unreferenced providers should not be checked"
        );
    }

    #[test]
    fn report_readiness_all_blocked() {
        let mut config = RokoConfig::default();
        config.models.insert(
            "m1".to_string(),
            ModelProfile {
                provider: "p1".to_string(),
                slug: "s1".to_string(),
                ..Default::default()
            },
        );
        let issues = vec![ProviderReadinessIssue {
            provider_name: "p1".to_string(),
            message: "test issue".to_string(),
        }];
        assert!(report_readiness_issues(&issues, &config));
    }

    #[test]
    fn report_readiness_partial_blocked() {
        let mut config = RokoConfig::default();
        config.models.insert(
            "m1".to_string(),
            ModelProfile {
                provider: "p1".to_string(),
                slug: "s1".to_string(),
                ..Default::default()
            },
        );
        config.models.insert(
            "m2".to_string(),
            ModelProfile {
                provider: "p2".to_string(),
                slug: "s2".to_string(),
                ..Default::default()
            },
        );
        // Only p1 is blocked, p2 is fine.
        let issues = vec![ProviderReadinessIssue {
            provider_name: "p1".to_string(),
            message: "test issue".to_string(),
        }];
        assert!(!report_readiness_issues(&issues, &config));
    }
}
