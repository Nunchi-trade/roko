//! Shared model dispatch plan data types.
//!
//! This module is intentionally data-only. Resolution and execution remain in
//! the agent/provider layers until the dispatch migration wires them together.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::ProviderKind;
use crate::config::schema::{ModelProfile, ProviderConfig};
use crate::foundation::{CachePolicy, ModelCallRequest, TokenBudget};

/// Request envelope for resolving an executable model dispatch plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchRequest {
    /// Surface that originated the request.
    pub caller: DispatchCaller,
    /// Working directory used for config, policy, and routing context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<PathBuf>,
    /// Role or persona requesting the call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Existing model-call request payload.
    pub model_call: ModelCallRequest,
    /// Hard model override, if supplied by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// Hard provider override, if supplied by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_override: Option<String>,
    /// Required execution capabilities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<DispatchRequirement>,
    /// Cache behavior requested by the caller.
    #[serde(default)]
    pub cache_policy: CachePolicy,
    /// Per-call budget requested by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<TokenBudget>,
    /// Fallback behavior allowed for this request.
    #[serde(default)]
    pub fallback_policy: FallbackPolicy,
}

/// Caller surface that originated a dispatch request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchCaller {
    Acp,
    CliChat,
    CliOneShot,
    Runner,
    Serve,
}

/// Capability or caller contract required from a resolved dispatch plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchRequirement {
    Streaming,
    Tools,
    McpTools,
    Vision,
    Thinking,
    WebSearch,
    Resume,
    EditorMediatedAuth,
    SideEffects,
}

/// Execution-authorizing model/provider/transport plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchPlan {
    /// Original model key or slug requested by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_model: Option<String>,
    /// Original provider id or kind requested by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_provider: Option<String>,
    /// Resolved model key from config.
    pub effective_model_key: String,
    /// Concrete provider model slug sent over the transport.
    pub model_slug: String,
    /// Resolved provider registry id.
    pub provider_id: String,
    /// Resolved provider protocol family.
    pub provider_kind: ProviderKind,
    /// Provider config snapshot used to authorize execution.
    pub provider_config: ProviderConfig,
    /// Model profile snapshot used to authorize execution.
    pub model_profile: ModelProfile,
    /// Concrete transport selected for execution.
    pub transport: TransportPlan,
    /// Auth validation state for the selected transport.
    pub auth_status: DispatchAuthStatus,
    /// Capability requirements validated for this plan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<DispatchRequirement>,
    /// Fallback behavior allowed for execution.
    #[serde(default)]
    pub fallback_policy: FallbackPolicy,
    /// Ordered fallback model keys or slugs that may be attempted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_candidates: Vec<String>,
    /// Planned or completed attempts for the request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<DispatchAttempt>,
    /// Routing/config notes that should remain visible to callers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

/// Concrete transport selected for a dispatch attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransportPlan {
    Cli {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        protocol: String,
    },
    Http {
        base_url: String,
        auth: TransportAuth,
        protocol: String,
    },
    Acp {
        command_or_endpoint: String,
        protocol: String,
    },
    Unsupported {
        reason: String,
    },
}

/// Auth material expected by a transport plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransportAuth {
    None,
    EnvVar { name: String },
    EditorMediated,
    Unknown { reason: String },
}

/// Auth validation status for a resolved dispatch plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DispatchAuthStatus {
    Validated,
    Missing { auth_method: String },
    Unvalidated { reason: String },
    NotRequired,
}

/// Fallback behavior allowed for a dispatch request or plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum FallbackPolicy {
    Disabled,
    ConfigOrdered { models: Vec<String> },
    SameProviderOnly,
    AllowCrossProvider { reason: String },
}

impl Default for FallbackPolicy {
    fn default() -> Self {
        Self::Disabled
    }
}

/// A primary or fallback dispatch attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchAttempt {
    pub kind: DispatchAttemptKind,
    pub model_key: String,
    pub model_slug: String,
    pub provider_id: String,
    pub provider_kind: ProviderKind,
    pub transport: TransportPlan,
}

/// Attempt position in a dispatch plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchAttemptKind {
    Primary,
    Fallback,
}

/// Typed dispatch resolution or execution error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DispatchError {
    MissingAuth {
        provider_id: String,
        auth_method: String,
    },
    UnsupportedProvider {
        provider_id: String,
        provider_kind: ProviderKind,
    },
    CapabilityMismatch {
        requirement: DispatchRequirement,
        provider_id: String,
        model_slug: String,
    },
    AmbiguousProvider {
        requested_provider: String,
        candidates: Vec<String>,
    },
    AmbiguousModel {
        requested_model: String,
        candidates: Vec<String>,
    },
    ProviderFailure {
        provider_id: String,
        message: String,
    },
    Cancelled,
    BudgetExceeded {
        detail: String,
    },
    ConfigInvalid {
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::{ChatMessage, MessageRole};

    fn provider_config() -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: Some("https://example.test/v1".to_string()),
            api_key_env: Some("EXAMPLE_API_KEY".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(120_000),
            ttft_timeout_ms: Some(15_000),
            connect_timeout_ms: Some(5_000),
            extra_headers: None,
            max_concurrent: None,
        }
    }

    fn model_profile() -> ModelProfile {
        ModelProfile {
            provider: "example".to_string(),
            slug: "example-model".to_string(),
            context_window: 128_000,
            tool_format: "openai_json".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn dispatch_plan_serializes_core_resolution_shape() {
        let plan = DispatchPlan {
            requested_model: Some("fast".to_string()),
            requested_provider: Some("example".to_string()),
            effective_model_key: "fast".to_string(),
            model_slug: "example-model".to_string(),
            provider_id: "example".to_string(),
            provider_kind: ProviderKind::OpenAiCompat,
            provider_config: provider_config(),
            model_profile: model_profile(),
            transport: TransportPlan::Http {
                base_url: "https://example.test/v1".to_string(),
                auth: TransportAuth::EnvVar {
                    name: "EXAMPLE_API_KEY".to_string(),
                },
                protocol: "chat_completions".to_string(),
            },
            auth_status: DispatchAuthStatus::Unvalidated {
                reason: "resolver skeleton".to_string(),
            },
            requirements: vec![DispatchRequirement::Streaming],
            fallback_policy: FallbackPolicy::Disabled,
            fallback_candidates: Vec::new(),
            attempts: vec![DispatchAttempt {
                kind: DispatchAttemptKind::Primary,
                model_key: "fast".to_string(),
                model_slug: "example-model".to_string(),
                provider_id: "example".to_string(),
                provider_kind: ProviderKind::OpenAiCompat,
                transport: TransportPlan::Http {
                    base_url: "https://example.test/v1".to_string(),
                    auth: TransportAuth::EnvVar {
                        name: "EXAMPLE_API_KEY".to_string(),
                    },
                    protocol: "chat_completions".to_string(),
                },
            }],
            diagnostics: vec!["capabilities pending resolver validation".to_string()],
        };

        let value = serde_json::to_value(&plan).expect("serialize dispatch plan");

        assert_eq!(value["provider_kind"], "openai_compat");
        assert_eq!(value["transport"]["kind"], "http");
        assert_eq!(value["auth_status"]["status"], "unvalidated");
        assert_eq!(value["fallback_policy"]["policy"], "disabled");
    }

    #[test]
    fn dispatch_request_serializes_existing_model_call_payload() {
        let request = DispatchRequest {
            caller: DispatchCaller::CliOneShot,
            workdir: Some(PathBuf::from("/repo")),
            role: Some("implementer".to_string()),
            model_call: ModelCallRequest {
                model: "example-model".to_string(),
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: "hello".to_string(),
                }],
                cache_policy: CachePolicy::Bypass,
                ..Default::default()
            },
            model_override: Some("example-model".to_string()),
            provider_override: None,
            requirements: vec![DispatchRequirement::Tools],
            cache_policy: CachePolicy::Bypass,
            budget: Some(TokenBudget {
                max_input: Some(1000),
                max_output: Some(500),
                max_cost_usd: Some(0.25),
            }),
            fallback_policy: FallbackPolicy::SameProviderOnly,
        };

        let json = serde_json::to_string(&request).expect("serialize dispatch request");
        let decoded: DispatchRequest =
            serde_json::from_str(&json).expect("deserialize dispatch request");

        assert!(matches!(decoded.caller, DispatchCaller::CliOneShot));
        assert!(matches!(decoded.cache_policy, CachePolicy::Bypass));
        assert_eq!(decoded.model_call.messages.len(), 1);
        assert_eq!(decoded.requirements, vec![DispatchRequirement::Tools]);
    }
}
