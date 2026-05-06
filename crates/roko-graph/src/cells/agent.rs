//! AgentCell — wraps LLM agent dispatch for use in graph execution.
//!
//! Sends a prompt to an LLM backend and returns the response as a node output.
//! Configuration includes model, provider, system prompt, and available tools.

use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::{Node, NodeOutput};

/// Configuration for an AgentCell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCellConfig {
    /// The model to use (e.g. "claude-sonnet-4-20250514", "gpt-4o").
    pub model: String,
    /// The provider backend (e.g. "anthropic", "openai", "ollama").
    #[serde(default = "default_provider")]
    pub provider: String,
    /// System prompt to prepend to the conversation.
    #[serde(default)]
    pub system_prompt: String,
    /// Available tool names for the agent.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Maximum tokens for the response.
    #[serde(default = "default_max_tokens")]
    pub max_response_tokens: u32,
    /// Temperature for sampling.
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_provider() -> String {
    "anthropic".into()
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_temperature() -> f32 {
    0.7
}

impl Default for AgentCellConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".into(),
            provider: default_provider(),
            system_prompt: String::new(),
            tools: Vec::new(),
            max_response_tokens: default_max_tokens(),
            temperature: default_temperature(),
        }
    }
}

impl AgentCellConfig {
    /// Parse config from a node's TOML config value.
    ///
    /// Expects a TOML table; returns defaults if the value is not a table.
    pub fn from_node_config(config: &toml::Value) -> Self {
        let table = match config.as_table() {
            Some(t) => t,
            None => return Self::default(),
        };

        let model = table
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("claude-sonnet-4-20250514")
            .to_string();

        let provider = table
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("anthropic")
            .to_string();

        let system_prompt = table
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let tools = table
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let max_response_tokens = table
            .get("max_response_tokens")
            .and_then(|v| v.as_integer())
            .map_or_else(default_max_tokens, |v| v as u32);

        let temperature = table
            .get("temperature")
            .and_then(|v| v.as_float())
            .map_or_else(default_temperature, |v| v as f32);

        Self {
            model,
            provider,
            system_prompt,
            tools,
            max_response_tokens,
            temperature,
        }
    }
}

/// AgentCell: dispatches a prompt to an LLM and returns the response.
///
/// In graph execution, the prompt is constructed from:
/// 1. The system_prompt in config
/// 2. Input data from upstream nodes (used as user message context)
///
/// The actual LLM dispatch is handled by the provided `AgentDispatcher` trait,
/// allowing the cell to be tested without real API calls.
pub struct AgentCell {
    config: AgentCellConfig,
    dispatcher: Box<dyn AgentDispatcher>,
}

/// Trait for LLM dispatch — abstracts the actual API call.
#[async_trait]
pub trait AgentDispatcher: Send + Sync {
    /// Send a prompt to the LLM and return the response text.
    async fn dispatch(
        &self,
        model: &str,
        provider: &str,
        system_prompt: &str,
        user_message: &str,
        tools: &[String],
        max_tokens: u32,
        temperature: f32,
    ) -> std::result::Result<AgentResponse, String>;
}

/// Response from an agent dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// The generated text.
    pub text: String,
    /// Tokens used for input.
    pub input_tokens: u64,
    /// Tokens used for output.
    pub output_tokens: u64,
    /// Estimated cost in USD.
    pub cost_usd: f64,
}

impl AgentCell {
    /// Create a new AgentCell with the given config and dispatcher.
    pub fn new(config: AgentCellConfig, dispatcher: Box<dyn AgentDispatcher>) -> Self {
        Self { config, dispatcher }
    }

    /// Create from a graph Node definition.
    pub fn from_node(node: &Node, dispatcher: Box<dyn AgentDispatcher>) -> Self {
        let config = AgentCellConfig::from_node_config(&node.config);
        Self::new(config, dispatcher)
    }

    /// Execute this cell: build prompt from inputs, dispatch to LLM, return output.
    pub async fn execute(&self, node_id: &str, inputs: &[NodeOutput]) -> NodeOutput {
        let start = Instant::now();

        // Build user message from upstream outputs.
        let user_message = self.build_user_message(inputs);

        match self
            .dispatcher
            .dispatch(
                &self.config.model,
                &self.config.provider,
                &self.config.system_prompt,
                &user_message,
                &self.config.tools,
                self.config.max_response_tokens,
                self.config.temperature,
            )
            .await
        {
            Ok(response) => {
                let mut output = NodeOutput::success(
                    node_id,
                    serde_json::json!({
                        "text": response.text,
                        "model": self.config.model,
                        "provider": self.config.provider,
                    }),
                );
                output.tokens_used = response.input_tokens + response.output_tokens;
                output.cost_usd = response.cost_usd;
                output.duration = start.elapsed();
                output
            }
            Err(err) => {
                let mut output = NodeOutput::failed(node_id, err);
                output.duration = start.elapsed();
                output
            }
        }
    }

    /// Build a user message from upstream node outputs.
    fn build_user_message(&self, inputs: &[NodeOutput]) -> String {
        if inputs.is_empty() {
            return String::new();
        }

        inputs
            .iter()
            .filter(|i| i.status.is_success())
            .map(|input| {
                // If the input has a "text" field, use that directly.
                if let Some(text) = input.data.get("text").and_then(|v| v.as_str()) {
                    text.to_string()
                } else {
                    // Otherwise serialize the whole data as context.
                    serde_json::to_string_pretty(&input.data).unwrap_or_default()
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }
}

/// A mock dispatcher for testing.
pub struct MockAgentDispatcher {
    /// The response to return.
    response: AgentResponse,
}

impl MockAgentDispatcher {
    /// Create a mock that always returns the given text.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            response: AgentResponse {
                text: text.into(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.003,
            },
        }
    }

    /// Create with custom token counts.
    pub fn with_usage(text: impl Into<String>, input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            response: AgentResponse {
                text: text.into(),
                input_tokens,
                output_tokens,
                cost_usd: (input_tokens as f64 * 0.00001) + (output_tokens as f64 * 0.00003),
            },
        }
    }
}

#[async_trait]
impl AgentDispatcher for MockAgentDispatcher {
    async fn dispatch(
        &self,
        _model: &str,
        _provider: &str,
        _system_prompt: &str,
        _user_message: &str,
        _tools: &[String],
        _max_tokens: u32,
        _temperature: f32,
    ) -> std::result::Result<AgentResponse, String> {
        Ok(self.response.clone())
    }
}

/// A failing dispatcher for testing error paths.
pub struct FailingAgentDispatcher {
    error_message: String,
}

impl FailingAgentDispatcher {
    /// Create a dispatcher that always fails with the given message.
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            error_message: msg.into(),
        }
    }
}

#[async_trait]
impl AgentDispatcher for FailingAgentDispatcher {
    async fn dispatch(
        &self,
        _model: &str,
        _provider: &str,
        _system_prompt: &str,
        _user_message: &str,
        _tools: &[String],
        _max_tokens: u32,
        _temperature: f32,
    ) -> std::result::Result<AgentResponse, String> {
        Err(self.error_message.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn agent_cell_success() {
        let config = AgentCellConfig::default();
        let dispatcher = Box::new(MockAgentDispatcher::new("Hello, world!"));
        let cell = AgentCell::new(config, dispatcher);

        let output = cell.execute("agent-1", &[]).await;
        assert!(output.status.is_success());
        assert_eq!(output.data["text"], "Hello, world!");
        assert_eq!(output.tokens_used, 150); // 100 input + 50 output
    }

    #[tokio::test]
    async fn agent_cell_failure() {
        let config = AgentCellConfig::default();
        let dispatcher = Box::new(FailingAgentDispatcher::new("API rate limited"));
        let cell = AgentCell::new(config, dispatcher);

        let output = cell.execute("agent-1", &[]).await;
        assert!(output.status.is_failed());
    }

    #[tokio::test]
    async fn agent_cell_receives_upstream_text() {
        use std::sync::{Arc, Mutex};

        // Capture what user_message the dispatcher receives.
        struct CapturingDispatcher {
            captured: Arc<Mutex<String>>,
        }

        #[async_trait]
        impl AgentDispatcher for CapturingDispatcher {
            async fn dispatch(
                &self,
                _model: &str,
                _provider: &str,
                _system_prompt: &str,
                user_message: &str,
                _tools: &[String],
                _max_tokens: u32,
                _temperature: f32,
            ) -> std::result::Result<AgentResponse, String> {
                *self.captured.lock().unwrap() = user_message.to_string();
                Ok(AgentResponse {
                    text: "response".into(),
                    input_tokens: 10,
                    output_tokens: 5,
                    cost_usd: 0.001,
                })
            }
        }

        let captured = Arc::new(Mutex::new(String::new()));
        let dispatcher = Box::new(CapturingDispatcher {
            captured: captured.clone(),
        });
        let cell = AgentCell::new(AgentCellConfig::default(), dispatcher);

        let inputs = vec![NodeOutput::success(
            "compose-1",
            json!({"text": "Please implement the feature"}),
        )];

        cell.execute("agent-1", &inputs).await;
        assert_eq!(*captured.lock().unwrap(), "Please implement the feature");
    }

    #[test]
    fn config_from_node() {
        let mut table = toml::map::Map::new();
        table.insert("model".into(), toml::Value::String("gpt-4o".into()));
        table.insert("provider".into(), toml::Value::String("openai".into()));
        table.insert("temperature".into(), toml::Value::Float(0.5));
        let config = toml::Value::Table(table);

        let parsed = AgentCellConfig::from_node_config(&config);
        assert_eq!(parsed.model, "gpt-4o");
        assert_eq!(parsed.provider, "openai");
        assert!((parsed.temperature - 0.5).abs() < f32::EPSILON);
    }
}
