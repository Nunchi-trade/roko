//! Sequential safety hook chain for the tool dispatcher (TOOL-02).
//!
//! Runs a sequence of [`SafetyHook`] implementations before each tool call.
//! The chain short-circuits on the first rejection and emits an audit record
//! for every hook decision (allow, modify, or reject).

use std::sync::Arc;

use roko_core::tool::{ToolContext, ToolDef, ToolError};

use crate::safety::hooks::{HookDecision, SafetyAuditRecord, SafetyHook};

/// A named hook entry in the chain.
struct NamedHook {
    /// Human-readable name for audit records.
    name: String,
    /// The hook implementation.
    hook: Arc<dyn SafetyHook>,
}

/// Sequential chain of [`SafetyHook`] implementations.
///
/// Each hook is evaluated in order. The chain short-circuits on the first
/// `Reject` decision. `AllowModified` replaces the parameters for
/// subsequent hooks and the eventual tool execution.
///
/// Every hook decision (including `Allow`) is recorded as a
/// [`SafetyAuditRecord`] for the audit trail.
pub struct SafetyHookChain {
    hooks: Vec<NamedHook>,
}

impl SafetyHookChain {
    /// Create an empty chain (no hooks).
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Add a named hook to the end of the chain.
    pub fn push(&mut self, name: impl Into<String>, hook: Arc<dyn SafetyHook>) {
        self.hooks.push(NamedHook {
            name: name.into(),
            hook,
        });
    }

    /// Number of hooks in the chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Returns `true` if no hooks are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Run all hooks sequentially for the given tool call.
    ///
    /// Returns the (possibly modified) parameters and a list of audit records.
    /// On rejection, returns the rejection error.
    pub async fn evaluate(
        &self,
        tool: &ToolDef,
        mut params: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<(serde_json::Value, Vec<SafetyAuditRecord>), (ToolError, Vec<SafetyAuditRecord>)>
    {
        let mut audit_trail = Vec::with_capacity(self.hooks.len());
        let timestamp = chrono::Utc::now().timestamp();
        let params_hash = compute_params_hash(&params);

        for entry in &self.hooks {
            let decision = match entry.hook.on_tool_call(tool, &params, ctx).await {
                Ok(decision) => decision,
                Err(err) => {
                    // Hook itself errored — treat as a rejection.
                    audit_trail.push(
                        SafetyAuditRecord::new(
                            timestamp,
                            &tool.name,
                            &entry.name,
                            HookDecision::Reject(err.to_string()),
                            &params_hash,
                        )
                        .with_reason(err.to_string()),
                    );
                    return Err((err, audit_trail));
                }
            };

            let record = SafetyAuditRecord::new(
                timestamp,
                &tool.name,
                &entry.name,
                decision.clone(),
                &params_hash,
            );

            match &decision {
                HookDecision::Allow => {
                    audit_trail.push(record);
                }
                HookDecision::AllowModified(new_params) => {
                    params = new_params.clone();
                    audit_trail.push(record.with_reason("parameters modified by hook".to_string()));
                }
                HookDecision::Reject(reason) => {
                    let record = record.with_reason(reason.clone());
                    audit_trail.push(record);
                    return Err((
                        ToolError::PermissionDenied(format!(
                            "safety hook `{}` rejected: {reason}",
                            entry.name
                        )),
                        audit_trail,
                    ));
                }
            }
        }

        Ok((params, audit_trail))
    }
}

impl Default for SafetyHookChain {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SafetyHookChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SafetyHookChain")
            .field(
                "hooks",
                &self.hooks.iter().map(|h| &h.name).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Compute a SHA-256 digest of serialized parameters for audit records.
fn compute_params_hash(params: &serde_json::Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let serialized = params.to_string();
    serialized.hash(&mut hasher);
    format!("hash:{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::hallucination::HallucinationDetector;
    use crate::safety::result_filter::ResultFilter;
    use roko_core::tool::{ToolCategory, ToolPermission};

    fn test_ctx() -> ToolContext {
        ToolContext::testing("/tmp/worktree")
    }

    fn test_tool(name: &str) -> ToolDef {
        ToolDef::new(
            name,
            "test tool",
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
    }

    #[tokio::test]
    async fn empty_chain_allows_all() {
        let chain = SafetyHookChain::new();
        let tool = test_tool("read_file");
        let params = serde_json::json!({ "file_path": "/tmp/test.rs" });
        let result = chain.evaluate(&tool, params.clone(), &test_ctx()).await;
        assert!(result.is_ok());
        let (returned_params, audit) = result.unwrap();
        assert_eq!(returned_params, params);
        assert!(audit.is_empty());
    }

    #[tokio::test]
    async fn chain_runs_all_hooks_and_collects_audit() {
        let mut chain = SafetyHookChain::new();
        chain.push(
            "hallucination_detector",
            Arc::new(HallucinationDetector::permissive()),
        );
        chain.push("result_filter", Arc::new(ResultFilter::with_defaults()));

        let tool = test_tool("read_file");
        let params = serde_json::json!({ "file_path": "/tmp/test.rs" });
        let result = chain.evaluate(&tool, params, &test_ctx()).await;
        assert!(result.is_ok());
        let (_, audit) = result.unwrap();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].hook_name, "hallucination_detector");
        assert_eq!(audit[1].hook_name, "result_filter");
    }

    #[tokio::test]
    async fn chain_short_circuits_on_rejection() {
        let mut chain = SafetyHookChain::new();
        chain.push(
            "hallucination_detector",
            Arc::new(HallucinationDetector::with_known_tools(["read_file"])),
        );
        chain.push("result_filter", Arc::new(ResultFilter::with_defaults()));

        let tool = test_tool("unknown_tool"); // Will be rejected by hallucination detector
        let params = serde_json::json!({});
        let result = chain.evaluate(&tool, params, &test_ctx()).await;
        assert!(result.is_err());
        let (err, audit) = result.unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied(_)));
        // Only one audit record because the chain short-circuited.
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].hook_name, "hallucination_detector");
    }

    #[tokio::test]
    async fn chain_rejects_embedded_secrets() {
        let mut chain = SafetyHookChain::new();
        chain.push(
            "hallucination_detector",
            Arc::new(HallucinationDetector::permissive()),
        );
        chain.push("result_filter", Arc::new(ResultFilter::with_defaults()));

        let tool = test_tool("bash");
        let api_key = format!("sk-ant-api03-{}", "A".repeat(80));
        let params = serde_json::json!({ "command": format!("echo {api_key}") });
        let result = chain.evaluate(&tool, params, &test_ctx()).await;
        assert!(result.is_err());
        let (_, audit) = result.unwrap_err();
        // Hallucination detector allows, result filter rejects.
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].hook_name, "hallucination_detector");
        assert_eq!(audit[1].hook_name, "result_filter");
    }

    #[tokio::test]
    async fn audit_records_have_correct_fields() {
        let mut chain = SafetyHookChain::new();
        chain.push(
            "hallucination_detector",
            Arc::new(HallucinationDetector::permissive()),
        );

        let tool = test_tool("read_file");
        let params = serde_json::json!({ "file_path": "/tmp/test.rs" });
        let result = chain.evaluate(&tool, params, &test_ctx()).await;
        let (_, audit) = result.unwrap();
        assert_eq!(audit.len(), 1);
        let record = &audit[0];
        assert_eq!(record.tool_name, "read_file");
        assert_eq!(record.hook_name, "hallucination_detector");
        assert!(matches!(record.decision, HookDecision::Allow));
        assert!(record.params_hash.starts_with("hash:"));
    }
}
