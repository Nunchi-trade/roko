//! Agent template registry.
//!
//! Templates are TOML files stored under `.roko/templates/` that define
//! reusable agent configurations. The [`TemplateRegistry`] scans the
//! directory on startup and supports CRUD operations plus simple
//! `{{param}}` interpolation.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The expected output shape for an agent template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateOutputFormat {
    /// Markdown-formatted output.
    Markdown,
    /// JSON-formatted output.
    Json,
    /// TOML-formatted output.
    Toml,
    /// No structured output requirement.
    None,
}

impl Default for TemplateOutputFormat {
    fn default() -> Self {
        Self::Markdown
    }
}

/// A reusable agent configuration template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplate {
    /// Unique template name (also the TOML filename stem).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Default model slug or tier label.
    #[serde(default = "default_model")]
    pub model: String,
    /// Role used to derive tool restrictions and prompt defaults.
    pub role: String,
    /// System prompt template with `{{variables}}` interpolation.
    pub system_prompt: String,
    /// Maximum agent turns before forced stop.
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    /// Expected output format.
    #[serde(default)]
    pub output_format: TemplateOutputFormat,
    /// MCP server names required by this template.
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    /// Tool allowlist glob patterns.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tool denylist glob patterns.
    #[serde(default)]
    pub denied_tools: Vec<String>,
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
                let tpl: AgentTemplate =
                    toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
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
        let mut out = template.system_prompt.clone();
        for (key, value) in params {
            let marker = format!("{{{{{key}}}}}");
            out = out.replace(&marker, value);
        }
        out
    }
}

fn default_model() -> String {
    "sonnet".into()
}

const fn default_max_turns() -> u32 {
    20
}
