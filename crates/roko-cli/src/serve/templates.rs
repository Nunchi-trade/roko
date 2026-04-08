//! Agent template registry.
//!
//! Templates are TOML files stored under `.roko/templates/` that define
//! reusable agent configurations (model, prompt, gates, parameters).
//! The [`TemplateRegistry`] scans the directory on startup and supports
//! CRUD operations plus simple `{{param}}` interpolation.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A reusable agent configuration template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplate {
    /// Unique template name (also the TOML filename stem).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Semver version string.
    pub version: String,
    /// Agent execution settings.
    pub agent: TemplateAgent,
    /// Prompt configuration.
    pub prompt: TemplatePrompt,
    /// Gate pipeline for this template.
    #[serde(default)]
    pub gates: TemplateGates,
    /// User-supplied parameters for prompt interpolation.
    #[serde(default)]
    pub params: Vec<TemplateParam>,
}

/// Agent execution settings within a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateAgent {
    /// CLI command to invoke (e.g. `"claude"`).
    pub command: String,
    /// Model slug (e.g. `"claude-sonnet-4-20250514"`).
    #[serde(default = "default_model")]
    pub model: String,
    /// Maximum agent turns before forced stop.
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
}

/// Prompt sections within a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplatePrompt {
    /// System prompt text (may contain `{{param}}` markers).
    pub system: String,
    /// Additional prompt section names to include.
    #[serde(default)]
    pub sections: Vec<String>,
}

/// Gate pipeline configuration within a template.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TemplateGates {
    /// Ordered list of gate names to run on agent output.
    #[serde(default)]
    pub names: Vec<String>,
}

/// A parameter that can be supplied at invocation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateParam {
    /// Parameter name (used as the `{{name}}` marker).
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Type hint: `"string"`, `"number"`, `"bool"`, `"select"`.
    pub param_type: String,
    /// For `"select"` types, the allowed values.
    #[serde(default)]
    pub options: Vec<String>,
    /// Default value (JSON-typed).
    #[serde(default)]
    pub default: serde_json::Value,
}

/// In-memory registry of agent templates backed by `.roko/templates/`.
pub struct TemplateRegistry {
    templates: HashMap<String, AgentTemplate>,
    dir: PathBuf,
}

impl TemplateRegistry {
    /// Create an empty registry backed by the given directory.
    pub fn new(dir: PathBuf) -> Self {
        Self {
            templates: HashMap::new(),
            dir,
        }
    }

    /// Scan the backing directory for `*.toml` files and load them.
    pub fn scan(&mut self) -> Result<()> {
        self.templates.clear();
        if !self.dir.exists() {
            return Ok(());
        }
        let entries =
            std::fs::read_dir(&self.dir).with_context(|| format!("read {}", self.dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("read {}", path.display()))?;
                let tpl: AgentTemplate = toml::from_str(&text)
                    .with_context(|| format!("parse {}", path.display()))?;
                self.templates.insert(tpl.name.clone(), tpl);
            }
        }
        Ok(())
    }

    /// Return all loaded templates.
    pub fn list(&self) -> Vec<&AgentTemplate> {
        self.templates.values().collect()
    }

    /// Look up a template by name.
    pub fn get(&self, name: &str) -> Option<&AgentTemplate> {
        self.templates.get(name)
    }

    /// Insert (or overwrite) a template and persist it to disk.
    pub fn insert(&mut self, template: AgentTemplate) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("create {}", self.dir.display()))?;
        let path = self.dir.join(format!("{}.toml", template.name));
        let text = toml::to_string_pretty(&template).context("serialize template")?;
        std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
        self.templates.insert(template.name.clone(), template);
        Ok(())
    }

    /// Remove a template by name. Returns `true` if it existed.
    pub fn remove(&mut self, name: &str) -> Result<bool> {
        if self.templates.remove(name).is_some() {
            let path = self.dir.join(format!("{name}.toml"));
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("remove {}", path.display()))?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Render a template's system prompt by interpolating `{{param}}` markers
    /// with the supplied parameter values.
    pub fn render_prompt(template: &AgentTemplate, params: &HashMap<String, String>) -> String {
        let mut out = template.prompt.system.clone();
        for (key, value) in params {
            let marker = format!("{{{{{key}}}}}");
            out = out.replace(&marker, value);
        }
        out
    }
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".into()
}

const fn default_max_turns() -> u32 {
    10
}
