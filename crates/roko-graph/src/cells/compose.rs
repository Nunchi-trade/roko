//! ComposeCell — runs prompt assembly using template variable substitution.
//!
//! Takes a template string and a set of variables, performs substitution,
//! and outputs the assembled prompt for downstream AgentCells to consume.

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::types::{Node, NodeOutput};

/// Configuration for a ComposeCell.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeCellConfig {
    /// The template string with `{{variable}}` placeholders.
    #[serde(default)]
    pub template: String,
    /// Static variables to substitute into the template.
    #[serde(default)]
    pub variables: HashMap<String, String>,
}

impl ComposeCellConfig {
    /// Parse config from a node's TOML config value.
    ///
    /// Expects a TOML table; returns defaults if the value is not a table.
    pub fn from_node_config(config: &toml::Value) -> Self {
        let table = match config.as_table() {
            Some(t) => t,
            None => return Self::default(),
        };

        let template = table
            .get("template")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let variables = table
            .get("variables")
            .and_then(|v| v.as_table())
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        Self {
            template,
            variables,
        }
    }
}

/// ComposeCell: assembles prompts from templates and variables.
///
/// Variable resolution order:
/// 1. Static variables from config
/// 2. Variables extracted from upstream node outputs (`data.text` or stringified `data`)
///    — upstream node IDs become available as `{{node_id}}` variables
/// 3. Special variables: `{{inputs}}` contains all upstream outputs joined
pub struct ComposeCell {
    config: ComposeCellConfig,
}

impl ComposeCell {
    /// Create a new ComposeCell with the given config.
    pub fn new(config: ComposeCellConfig) -> Self {
        Self { config }
    }

    /// Create from a graph Node definition.
    pub fn from_node(node: &Node) -> Self {
        let config = ComposeCellConfig::from_node_config(&node.config);
        Self::new(config)
    }

    /// Execute this cell: substitute variables into template, return assembled text.
    pub fn execute(&self, node_id: &str, inputs: &[NodeOutput]) -> NodeOutput {
        let start = Instant::now();

        // Build variable map: static config vars + upstream output vars.
        let mut vars = self.config.variables.clone();

        // Add upstream outputs as variables keyed by node_id.
        for input in inputs {
            if input.status.is_success() {
                let text = extract_text(&input.data);
                vars.insert(input.node_id.clone(), text);
            }
        }

        // Add special {{inputs}} variable: all upstream texts joined.
        let all_inputs: String = inputs
            .iter()
            .filter(|i| i.status.is_success())
            .map(|i| extract_text(&i.data))
            .collect::<Vec<_>>()
            .join("\n\n");
        vars.insert("inputs".to_string(), all_inputs);

        // Perform template substitution.
        let assembled = substitute_template(&self.config.template, &vars);

        let mut output = NodeOutput::success(
            node_id,
            serde_json::json!({
                "text": assembled,
                "template": self.config.template,
                "variables_used": vars.keys().collect::<Vec<_>>(),
            }),
        );
        output.duration = start.elapsed();
        output
    }
}

/// Substitute `{{variable}}` placeholders in a template string.
fn substitute_template(template: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{key}}}}}");
        result = result.replace(&placeholder, value);
    }
    result
}

/// Extract text from a node output's data field.
fn extract_text(data: &serde_json::Value) -> String {
    // Prefer a "text" field if present.
    if let Some(text) = data.get("text").and_then(|v| v.as_str()) {
        return text.to_string();
    }
    // Fall back to stringified JSON.
    if data.is_null() {
        return String::new();
    }
    serde_json::to_string_pretty(data).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn basic_template_substitution() {
        let config = ComposeCellConfig {
            template: "Hello, {{name}}! Your task is: {{task}}".into(),
            variables: HashMap::from([
                ("name".into(), "Agent".into()),
                ("task".into(), "implement the feature".into()),
            ]),
        };
        let cell = ComposeCell::new(config);
        let output = cell.execute("compose-1", &[]);

        assert!(output.status.is_success());
        let text = output.data["text"].as_str().unwrap();
        assert_eq!(text, "Hello, Agent! Your task is: implement the feature");
    }

    #[test]
    fn upstream_output_as_variable() {
        let config = ComposeCellConfig {
            template: "Context: {{research}}\n\nPlease implement based on the above.".into(),
            variables: HashMap::new(),
        };
        let cell = ComposeCell::new(config);

        let inputs = vec![NodeOutput::success(
            "research",
            json!({"text": "The API uses REST with JSON payloads"}),
        )];

        let output = cell.execute("compose-1", &inputs);
        let text = output.data["text"].as_str().unwrap();
        assert!(text.contains("The API uses REST with JSON payloads"));
        assert!(text.contains("Please implement based on the above."));
    }

    #[test]
    fn inputs_special_variable() {
        let config = ComposeCellConfig {
            template: "All inputs:\n{{inputs}}".into(),
            variables: HashMap::new(),
        };
        let cell = ComposeCell::new(config);

        let inputs = vec![
            NodeOutput::success("a", json!({"text": "First input"})),
            NodeOutput::success("b", json!({"text": "Second input"})),
        ];

        let output = cell.execute("compose-1", &inputs);
        let text = output.data["text"].as_str().unwrap();
        assert!(text.contains("First input"));
        assert!(text.contains("Second input"));
    }

    #[test]
    fn missing_variable_left_as_is() {
        let config = ComposeCellConfig {
            template: "Hello {{name}}, your ID is {{id}}".into(),
            variables: HashMap::from([("name".into(), "Agent".into())]),
        };
        let cell = ComposeCell::new(config);
        let output = cell.execute("compose-1", &[]);

        let text = output.data["text"].as_str().unwrap();
        assert_eq!(text, "Hello Agent, your ID is {{id}}");
    }

    #[test]
    fn empty_template_returns_empty() {
        let config = ComposeCellConfig::default();
        let cell = ComposeCell::new(config);
        let output = cell.execute("compose-1", &[]);

        let text = output.data["text"].as_str().unwrap();
        assert_eq!(text, "");
    }

    #[test]
    fn skipped_inputs_excluded() {
        let config = ComposeCellConfig {
            template: "{{inputs}}".into(),
            variables: HashMap::new(),
        };
        let cell = ComposeCell::new(config);

        let inputs = vec![
            NodeOutput::success("a", json!({"text": "good"})),
            NodeOutput::skipped("b", "pruned"),
            NodeOutput::failed("c", "error"),
        ];

        let output = cell.execute("compose-1", &inputs);
        let text = output.data["text"].as_str().unwrap();
        assert_eq!(text, "good"); // Only "a" contributes.
    }

    #[test]
    fn config_from_node_config() {
        // Parse a TOML snippet to get a proper config value.
        let toml_str = r#"
template = "Task: {{task_description}}"

[variables]
task_description = "build the thing"
"#;
        let config: toml::Value = toml::from_str(toml_str).unwrap();

        let parsed = ComposeCellConfig::from_node_config(&config);
        assert_eq!(parsed.template, "Task: {{task_description}}");
        assert_eq!(
            parsed.variables.get("task_description").unwrap(),
            "build the thing"
        );
    }
}
