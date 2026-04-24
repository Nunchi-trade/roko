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

/// Extension trait for composable agent behavior.
///
/// All hooks have default no-op implementations. Extensions only override
/// the hooks they need. Hooks are called in layer order (Foundation first,
/// Recovery last), and within a layer in the order extensions are listed
/// in the configuration.
///
/// # Error handling
///
/// If an extension hook returns `Err`, the error is:
/// - Logged and ignored if `optional = true`
/// - Propagated to the caller if `optional = false` (default)
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
    fn on_init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called once when the agent shuts down. Use for cleanup.
    fn on_shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Perception (Layer 1) ────────────────────────────────────────

    /// Called when new observations arrive. Can enrich or annotate them.
    fn on_observe(
        &self,
        _observations: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called to filter observations before they reach cognition.
    fn on_filter(
        &self,
        _observations: &mut Vec<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Memory (Layer 2) ────────────────────────────────────────────

    /// Called when retrieving from knowledge store. Can augment results.
    fn on_retrieve(
        &self,
        _query: &str,
        _results: &mut Vec<serde_json::Value>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called when storing to knowledge. Can transform or filter.
    fn on_store(
        &self,
        _entry: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Cognition (Layer 3) ─────────────────────────────────────────

    /// Called before sending a request to the LLM. Can modify the request.
    fn pre_inference(
        &self,
        _request: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called after receiving a response from the LLM. Can modify or log.
    fn post_inference(
        &self,
        _response: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called during gating decisions. Can influence pass/fail.
    fn on_gate(
        &self,
        _gate_name: &str,
        _passed: bool,
        _details: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Action (Layer 4) ────────────────────────────────────────────

    /// Called before an action executes. Can block, allow, or rewrite.
    fn pre_action(
        &self,
        _action: &serde_json::Value,
    ) -> Result<ActionDecision, Box<dyn std::error::Error + Send + Sync>> {
        Ok(ActionDecision::Proceed)
    }

    /// Called after an action completes.
    fn post_action(
        &self,
        _action: &serde_json::Value,
        _result: &serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called when a tool is invoked. Can allow, deny, or rewrite.
    fn on_tool_call(
        &self,
        _tool_name: &str,
        _args: &serde_json::Value,
    ) -> Result<ToolDecision, Box<dyn std::error::Error + Send + Sync>> {
        Ok(ToolDecision::Allow)
    }

    // ── Social (Layer 5) ────────────────────────────────────────────

    /// Called before sending a message to another agent.
    fn on_message_send(
        &self,
        _message: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called when receiving a message from another agent.
    fn on_message_receive(
        &self,
        _message: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Meta (Layer 6) ──────────────────────────────────────────────

    /// Called during the reflection phase. Returns suggested adjustments.
    fn on_reflect(
        &self,
        _state: &serde_json::Value,
    ) -> Result<Vec<Adjustment>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Vec::new())
    }

    /// Called when cost data is updated (tokens, USD).
    fn on_cost_update(
        &self,
        _cost: &serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    // ── Recovery (Layer 7) ──────────────────────────────────────────

    /// Called when an error occurs. Determines recovery strategy.
    fn on_error(
        &self,
        _error: &dyn std::error::Error,
    ) -> Result<RecoveryAction, Box<dyn std::error::Error + Send + Sync>> {
        Ok(RecoveryAction::Propagate)
    }
}

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
    pub fn init_all(&mut self) -> Vec<(String, Box<dyn std::error::Error + Send + Sync>)> {
        let mut errors = Vec::new();
        for ext in &mut self.extensions {
            if let Err(e) = ext.on_init() {
                errors.push((ext.name().to_string(), e));
            }
        }
        errors
    }

    /// Shut down all extensions in reverse order.
    pub fn shutdown_all(&mut self) -> Vec<(String, Box<dyn std::error::Error + Send + Sync>)> {
        let mut errors = Vec::new();
        for ext in self.extensions.iter_mut().rev() {
            if let Err(e) = ext.on_shutdown() {
                errors.push((ext.name().to_string(), e));
            }
        }
        errors
    }

    /// Run pre_inference hooks (Cognition layer only).
    pub fn run_pre_inference(
        &self,
        request: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Cognition)
        {
            ext.pre_inference(request)?;
        }
        Ok(())
    }

    /// Run post_inference hooks (Cognition layer only).
    pub fn run_post_inference(
        &self,
        response: &mut serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Cognition)
        {
            ext.post_inference(response)?;
        }
        Ok(())
    }

    /// Run pre_action hooks (Action layer only). Returns first Block/Rewrite.
    pub fn run_pre_action(
        &self,
        action: &serde_json::Value,
    ) -> Result<ActionDecision, Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Action)
        {
            match ext.pre_action(action)? {
                ActionDecision::Proceed => continue,
                decision => return Ok(decision),
            }
        }
        Ok(ActionDecision::Proceed)
    }

    /// Run on_tool_call hooks (Action layer only). Returns first Deny/Rewrite.
    pub fn run_on_tool_call(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<ToolDecision, Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Action)
        {
            match ext.on_tool_call(tool_name, args)? {
                ToolDecision::Allow => continue,
                decision => return Ok(decision),
            }
        }
        Ok(ToolDecision::Allow)
    }

    /// Run on_error hooks (Recovery layer only). Returns first non-Propagate.
    pub fn run_on_error(
        &self,
        error: &dyn std::error::Error,
    ) -> Result<RecoveryAction, Box<dyn std::error::Error + Send + Sync>> {
        for ext in self
            .extensions
            .iter()
            .filter(|e| e.layer() == ExtensionLayer::Recovery)
        {
            match ext.on_error(error)? {
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

    #[test]
    fn chain_init_shutdown_order() {
        let mut chain = ExtensionChain::new();
        chain.add(Box::new(TestExtension {
            name: "ext-a".into(),
            layer: ExtensionLayer::Foundation,
        }));
        chain.add(Box::new(TestExtension {
            name: "ext-b".into(),
            layer: ExtensionLayer::Cognition,
        }));

        let init_errors = chain.init_all();
        assert!(init_errors.is_empty());

        let shutdown_errors = chain.shutdown_all();
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

    #[test]
    fn tool_call_allow_by_default() {
        let chain = ExtensionChain::new();
        let decision = chain
            .run_on_tool_call("bash", &serde_json::json!({}))
            .unwrap();
        assert_eq!(decision, ToolDecision::Allow);
    }

    #[test]
    fn action_proceed_by_default() {
        let chain = ExtensionChain::new();
        let decision = chain.run_pre_action(&serde_json::json!({})).unwrap();
        assert_eq!(decision, ActionDecision::Proceed);
    }

    #[test]
    fn error_propagate_by_default() {
        let chain = ExtensionChain::new();
        let err = std::io::Error::new(std::io::ErrorKind::Other, "test");
        let action = chain.run_on_error(&err).unwrap();
        assert_eq!(action, RecoveryAction::Propagate);
    }
}
