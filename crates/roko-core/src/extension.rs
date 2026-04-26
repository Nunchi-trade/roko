//! Extension trait for composable agent behavior.
//!
//! Extensions hook into 8 layers of the agent tick pipeline. Each hook has
//! a default no-op implementation so extensions only override what they need.
//!
//! # Layers (execution order)
//!
//! | Layer | # | Hooks | Purpose |
//! |-------|---|-------|---------|
//! | Foundation | 0 | `on_init`, `on_shutdown` | Lifecycle setup/teardown |
//! | Perception | 1 | `on_observe`, `on_filter` | Raw input processing |
//! | Memory | 2 | `on_retrieve`, `on_store` | Knowledge access |
//! | Cognition | 3 | `pre_inference`, `post_inference`, `on_gate` | LLM interaction |
//! | Action | 4 | `pre_action`, `post_action`, `on_tool_call` | Tool execution |
//! | Social | 5 | `on_message_send`, `on_message_receive` | Inter-agent |
//! | Meta | 6 | `on_reflect`, `on_cost_update` | Self-monitoring |
//! | Recovery | 7 | `on_error` | Fault handling |
//!
//! Extensions are loaded from `roko.toml` under `[agent.extensions]` and
//! `[agent.roles.<role>.extensions]`.

use serde::{Deserialize, Serialize};

// ── Typed hook parameter structs (C2) ─────────────────────────────────

/// Pre-inference hook context. Passed mutably so extensions can modify
/// the request before it reaches the LLM.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InferenceRequest {
    /// Plan this inference belongs to.
    pub plan_id: String,
    /// Task being executed.
    pub task: String,
    /// Agent role (e.g. "engineer", "reviewer").
    pub role: String,
    /// Model being called (e.g. "claude-sonnet-4-20250514").
    pub model: String,
    /// Estimated prompt token count.
    pub prompt_tokens: usize,
    /// Escape hatch for truly dynamic / extension-specific data.
    pub extra: serde_json::Value,
}

/// Post-inference hook context. Passed mutably so extensions can annotate
/// or transform the response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InferenceResponse {
    /// Plan this inference belongs to.
    pub plan_id: String,
    /// Task that was executed.
    pub task: String,
    /// Agent role.
    pub role: String,
    /// Model that was called.
    pub model: String,
    /// Whether the inference succeeded.
    pub success: bool,
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Wall-clock duration in milliseconds.
    pub wall_ms: u64,
    /// Escape hatch for truly dynamic / extension-specific data.
    pub extra: serde_json::Value,
}

/// Verify evaluation result passed to `on_gate`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateEvent {
    /// Plan the gate belongs to.
    pub plan_id: String,
    /// Verify that ran (e.g. "compile", "test", "clippy").
    pub gate_name: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Verify rung (e.g. "rung-1", "rung-3").
    pub rung: String,
    /// How long the gate took in milliseconds.
    pub duration_ms: u64,
    /// Verify-specific details (diagnostics, counts, etc.).
    pub details: serde_json::Value,
}

/// Error context for recovery hooks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorEvent {
    /// Human-readable error message.
    pub error_message: String,
    /// Where the error originated (e.g. "agent_dispatch", "gate_pipeline").
    pub source: String,
    /// Escape hatch for extension-specific context.
    pub extra: serde_json::Value,
}

/// Generic observation for the perception layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    /// Where this observation came from.
    pub source: String,
    /// The observation payload.
    pub data: serde_json::Value,
}

/// Tool call event for action-layer hooks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallEvent {
    /// Name of the tool being invoked.
    pub tool_name: String,
    /// Arguments passed to the tool.
    pub arguments: serde_json::Value,
    /// Result of the tool call, if available (post-action only).
    pub result: Option<serde_json::Value>,
}

/// Cost update event for the meta layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostUpdate {
    /// Model that incurred the cost.
    pub model: String,
    /// Input tokens consumed.
    pub tokens_in: u64,
    /// Output tokens produced.
    pub tokens_out: u64,
    /// Cost in USD.
    pub cost_usd: f64,
}

/// Inter-agent message for social-layer hooks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentMessage {
    /// Sender agent name.
    pub from: String,
    /// Recipient agent name.
    pub to: String,
    /// Message payload.
    pub payload: serde_json::Value,
}

/// Memory retrieval context.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetrievalResult {
    /// The query that was issued.
    pub query: String,
    /// Retrieved entries (mutable so extensions can augment).
    pub entries: Vec<serde_json::Value>,
}

/// Memory store context.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoreEntry {
    /// Key or topic being stored.
    pub key: String,
    /// The entry payload.
    pub data: serde_json::Value,
}

/// Reflection state for the meta layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReflectionState {
    /// Current agent state snapshot.
    pub state: serde_json::Value,
}

// ── Existing enums & structs ──────────────────────────────────────────

/// Layer in the agent tick pipeline where an extension runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionLayer {
    /// Lifecycle: init/shutdown.
    Foundation = 0,
    /// Input processing: observe/filter.
    Perception = 1,
    /// Knowledge store: retrieve/store.
    Memory = 2,
    /// LLM interaction: pre/post inference, gating.
    Cognition = 3,
    /// Tool execution: pre/post action, tool calls.
    Action = 4,
    /// Inter-agent messaging.
    Social = 5,
    /// Self-monitoring: reflect, cost tracking.
    Meta = 6,
    /// Fault handling and recovery.
    Recovery = 7,
}

/// What to do before an action executes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionDecision {
    /// Allow the action to proceed.
    Proceed,
    /// Block the action with an explanation.
    Block(String),
    /// Rewrite the action with a modified version.
    Rewrite(String),
}

/// What to do when a tool is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolDecision {
    /// Allow the tool call.
    Allow,
    /// Deny the tool call with a reason.
    Deny(String),
    /// Allow but with modified arguments.
    Rewrite(String),
}

/// What to do when an error occurs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Propagate the error up.
    Propagate,
    /// Retry the failed operation.
    Retry,
    /// Skip the failed step and continue.
    Skip,
    /// Substitute a fallback value.
    Fallback(String),
}

/// An adjustment suggested during reflection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Adjustment {
    /// What is being adjusted.
    pub target: String,
    /// The adjustment to make.
    pub value: serde_json::Value,
    /// Confidence in this adjustment (0.0-1.0).
    pub confidence: f64,
}

/// Metadata about a loaded extension.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtensionMeta {
    /// Unique extension name.
    pub name: String,
    /// Layer this extension operates in.
    pub layer: ExtensionLayer,
    /// Whether failure in this extension is fatal.
    #[serde(default)]
    pub optional: bool,
    /// Dependencies (other extension names that must load first).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Extension version.
    #[serde(default)]
    pub version: String,
}

// ── Extension trait (async + typed parameters) ────────────────────────

/// Extension trait for composable agent behavior.
///
/// All hooks have default no-op implementations. Extensions only override
/// the hooks they need. Hooks are called in layer order (Foundation first,
/// Recovery last), and within a layer in the order extensions are listed
/// in the configuration.
///
/// All hooks are async (E1) and use typed parameter structs (C2) instead
/// of raw `serde_json::Value`.
///
/// # Error handling
///
/// If an extension hook returns `Err`, the error is:
/// - Logged and ignored if `optional = true`
/// - Propagated to the caller if `optional = false` (default)
#[async_trait::async_trait]
pub trait Extension: Send + Sync {
    /// Unique name identifying this extension.
    fn name(&self) -> &str;

    /// Which layer this extension belongs to.
    fn layer(&self) -> ExtensionLayer;

    /// Metadata for this extension (name, layer, optional, dependencies).
    fn meta(&self) -> ExtensionMeta {
        ExtensionMeta {
            name: self.name().to_string(),
            layer: self.layer(),
            optional: false,
            depends_on: Vec::new(),
            version: String::new(),
        }
    }

    // ── Foundation (Layer 0) ────────────────────────────────────────

    /// Called once when the agent starts. Use for setup, connections, etc.
    async fn on_init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called once when the agent shuts down. Use for cleanup.
    async fn on_shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Perception (Layer 1) ────────────────────────────────────────

    /// Called when new observations arrive. Can enrich or annotate them.
    async fn on_observe(
        &self,
        _observation: &mut Observation,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called to filter observations before they reach cognition.
    async fn on_filter(
        &self,
        _observations: &mut Vec<Observation>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Memory (Layer 2) ────────────────────────────────────────────

    /// Called when retrieving from knowledge store. Can augment results.
    async fn on_retrieve(
        &self,
        _results: &mut RetrievalResult,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called when storing to knowledge. Can transform or filter.
    async fn on_store(
        &self,
        _entry: &mut StoreEntry,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Cognition (Layer 3) ─────────────────────────────────────────

    /// Called before sending a request to the LLM. Can modify the request.
    async fn pre_inference(
        &self,
        _request: &mut InferenceRequest,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called after receiving a response from the LLM. Can modify or log.
    async fn post_inference(
        &self,
        _response: &mut InferenceResponse,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called during gating decisions. Can influence pass/fail.
    async fn on_gate(
        &self,
        _event: &mut GateEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Action (Layer 4) ────────────────────────────────────────────

    /// Called before an action executes. Can block, allow, or rewrite.
    async fn pre_action(
        &self,
        _event: &ToolCallEvent,
    ) -> Result<ActionDecision, Box<dyn std::error::Error + Send + Sync>> {
        Ok(ActionDecision::Proceed)
    }

    /// Called after an action completes.
    async fn post_action(
        &self,
        _event: &ToolCallEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called when a tool is invoked. Can allow, deny, or rewrite.
    async fn on_tool_call(
        &self,
        _event: &ToolCallEvent,
    ) -> Result<ToolDecision, Box<dyn std::error::Error + Send + Sync>> {
        Ok(ToolDecision::Allow)
    }

    // ── Social (Layer 5) ────────────────────────────────────────────

    /// Called before sending a message to another agent.
    async fn on_message_send(
        &self,
        _message: &mut AgentMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called when receiving a message from another agent.
    async fn on_message_receive(
        &self,
        _message: &mut AgentMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Meta (Layer 6) ──────────────────────────────────────────────

    /// Called during the reflection phase. Returns suggested adjustments.
    async fn on_reflect(
        &self,
        _state: &ReflectionState,
    ) -> Result<Vec<Adjustment>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Vec::new())
    }

    /// Called when cost data is updated (tokens, USD).
    async fn on_cost_update(
        &self,
        _cost: &CostUpdate,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Recovery (Layer 7) ──────────────────────────────────────────

    /// Called when an error occurs. Determines recovery strategy.
    async fn on_error(
        &self,
        _event: &ErrorEvent,
    ) -> Result<RecoveryAction, Box<dyn std::error::Error + Send + Sync>> {
        Ok(RecoveryAction::Propagate)
    }
}

// ── ExtensionChain ────────────────────────────────────────────────────

/// An ordered chain of extensions, executed in layer order.
pub struct ExtensionChain {
    extensions: Vec<Box<dyn Extension>>,
}

impl ExtensionChain {
    /// Create an empty chain.
    pub fn new() -> Self {
        Self {
            extensions: Vec::new(),
        }
    }

    /// Add an extension to the chain. Extensions are sorted by layer on build.
    pub fn add(&mut self, ext: Box<dyn Extension>) {
        self.extensions.push(ext);
    }

    /// Sort extensions by layer (stable sort preserves config order within layer).
    pub fn sort_by_layer(&mut self) {
        self.extensions.sort_by_key(|e| e.layer() as u8);
    }

    /// Number of loaded extensions.
    pub fn len(&self) -> usize {
        self.extensions.len()
    }

    /// Whether the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }

    /// Initialize all extensions in order.
    pub async fn init_all(&mut self) -> Vec<(String, Box<dyn std::error::Error + Send + Sync>)> {
        let mut errors = Vec::new();
        for ext in &mut self.extensions {
            if let Err(e) = ext.on_init().await {
                errors.push((ext.name().to_string(), e));
            }
        }
        errors
    }

    /// Shut down all extensions in reverse order.
    pub async fn shutdown_all(
        &mut self,
    ) -> Vec<(String, Box<dyn std::error::Error + Send + Sync>)> {
        let mut errors = Vec::new();
        for ext in self.extensions.iter_mut().rev() {
            if let Err(e) = ext.on_shutdown().await {
                errors.push((ext.name().to_string(), e));
            }
        }
        errors
    }

    /// Run pre_inference hooks (Cognition layer only).
    pub async fn run_pre_inference(
        &self,
        request: &mut InferenceRequest,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Cognition)
        {
            ext.pre_inference(request).await?;
        }
        Ok(())
    }

    /// Run post_inference hooks (Cognition layer only).
    pub async fn run_post_inference(
        &self,
        response: &mut InferenceResponse,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Cognition)
        {
            ext.post_inference(response).await?;
        }
        Ok(())
    }

    /// Run on_gate hooks (Cognition layer only).
    pub async fn run_on_gate(
        &self,
        event: &mut GateEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Cognition)
        {
            ext.on_gate(event).await?;
        }
        Ok(())
    }

    /// Run pre_action hooks (Action layer only). Returns first Block/Rewrite.
    pub async fn run_pre_action(
        &self,
        event: &ToolCallEvent,
    ) -> Result<ActionDecision, Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Action)
        {
            match ext.pre_action(event).await? {
                ActionDecision::Proceed => continue,
                decision => return Ok(decision),
            }
        }
        Ok(ActionDecision::Proceed)
    }

    /// Run on_tool_call hooks (Action layer only). Returns first Deny/Rewrite.
    pub async fn run_on_tool_call(
        &self,
        event: &ToolCallEvent,
    ) -> Result<ToolDecision, Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Action)
        {
            match ext.on_tool_call(event).await? {
                ToolDecision::Allow => continue,
                decision => return Ok(decision),
            }
        }
        Ok(ToolDecision::Allow)
    }

    /// Run on_gate hooks (Cognition layer only).
    pub fn run_on_gate(
        &self,
        gate_name: &str,
        passed: bool,
        details: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Cognition)
        {
            ext.on_gate(gate_name, passed, details)?;
        }
        Ok(())
    }

    /// Run on_error hooks (Recovery layer only). Returns first non-Propagate.
    pub async fn run_on_error(
        &self,
        event: &ErrorEvent,
    ) -> Result<RecoveryAction, Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Recovery)
        {
            match ext.on_error(event).await? {
                RecoveryAction::Propagate => continue,
                action => return Ok(action),
            }
        }
        Ok(RecoveryAction::Propagate)
    }

    /// List all extension metadata.
    pub fn metadata(&self) -> Vec<ExtensionMeta> {
        self.extensions.iter().map(|e| e.meta()).collect()
    }
}

impl Default for ExtensionChain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestExtension {
        name: String,
        layer: ExtensionLayer,
    }

    #[async_trait::async_trait]
    impl Extension for TestExtension {
        fn name(&self) -> &str {
            &self.name
        }
        fn layer(&self) -> ExtensionLayer {
            self.layer
        }
    }

    #[test]
    fn chain_sorts_by_layer() {
        let mut chain = ExtensionChain::new();
        chain.add(Box::new(TestExtension {
            name: "recovery-ext".into(),
            layer: ExtensionLayer::Recovery,
        }));
        chain.add(Box::new(TestExtension {
            name: "cognition-ext".into(),
            layer: ExtensionLayer::Cognition,
        }));
        chain.add(Box::new(TestExtension {
            name: "foundation-ext".into(),
            layer: ExtensionLayer::Foundation,
        }));

        chain.sort_by_layer();
        let names: Vec<_> = chain.extensions.iter().map(|e| e.name()).collect();
        assert_eq!(names, &["foundation-ext", "cognition-ext", "recovery-ext"]);
    }

    #[tokio::test]
    async fn chain_init_shutdown_order() {
        let mut chain = ExtensionChain::new();
        chain.add(Box::new(TestExtension {
            name: "ext-a".into(),
            layer: ExtensionLayer::Foundation,
        }));
        chain.add(Box::new(TestExtension {
            name: "ext-b".into(),
            layer: ExtensionLayer::Cognition,
        }));

        let init_errors = chain.init_all().await;
        assert!(init_errors.is_empty());

        let shutdown_errors = chain.shutdown_all().await;
        assert!(shutdown_errors.is_empty());
    }

    #[test]
    fn metadata_reflects_extensions() {
        let mut chain = ExtensionChain::new();
        chain.add(Box::new(TestExtension {
            name: "my-ext".into(),
            layer: ExtensionLayer::Social,
        }));

        let meta = chain.metadata();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].name, "my-ext");
        assert_eq!(meta[0].layer, ExtensionLayer::Social);
    }

    #[tokio::test]
    async fn tool_call_allow_by_default() {
        let chain = ExtensionChain::new();
        let event = ToolCallEvent {
            tool_name: "bash".into(),
            arguments: serde_json::json!({}),
            result: None,
        };
        let decision = chain.run_on_tool_call(&event).await.unwrap();
        assert_eq!(decision, ToolDecision::Allow);
    }

    #[tokio::test]
    async fn action_proceed_by_default() {
        let chain = ExtensionChain::new();
        let event = ToolCallEvent {
            tool_name: "bash".into(),
            arguments: serde_json::json!({}),
            result: None,
        };
        let decision = chain.run_pre_action(&event).await.unwrap();
        assert_eq!(decision, ActionDecision::Proceed);
    }

    #[tokio::test]
    async fn error_propagate_by_default() {
        let chain = ExtensionChain::new();
        let event = ErrorEvent {
            error_message: "test error".into(),
            source: "test".into(),
            extra: serde_json::Value::Null,
        };
        let action = chain.run_on_error(&event).await.unwrap();
        assert_eq!(action, RecoveryAction::Propagate);
    }

    #[tokio::test]
    async fn pre_inference_typed_struct() {
        let chain = ExtensionChain::new();
        let mut req = InferenceRequest {
            plan_id: "plan-1".into(),
            task: "task-1".into(),
            role: "engineer".into(),
            model: "claude-sonnet-4-20250514".into(),
            prompt_tokens: 1000,
            extra: serde_json::Value::Null,
        };
        // No extensions, should pass through cleanly.
        chain.run_pre_inference(&mut req).await.unwrap();
        assert_eq!(req.plan_id, "plan-1");
    }

    #[tokio::test]
    async fn post_inference_typed_struct() {
        let chain = ExtensionChain::new();
        let mut resp = InferenceResponse {
            plan_id: "plan-1".into(),
            task: "task-1".into(),
            role: "engineer".into(),
            model: "claude-sonnet-4-20250514".into(),
            success: true,
            cost_usd: 0.01,
            wall_ms: 500,
            extra: serde_json::Value::Null,
        };
        chain.run_post_inference(&mut resp).await.unwrap();
        assert!(resp.success);
    }

    #[tokio::test]
    async fn on_gate_typed_struct() {
        let chain = ExtensionChain::new();
        let mut event = GateEvent {
            plan_id: "plan-1".into(),
            gate_name: "compile".into(),
            passed: true,
            rung: "rung-1".into(),
            duration_ms: 200,
            details: serde_json::Value::Null,
        };
        chain.run_on_gate(&mut event).await.unwrap();
        assert!(event.passed);
    }
}
