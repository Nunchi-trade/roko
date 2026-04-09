//! Agent template registry.
//!
//! Templates are TOML files stored under `.roko/templates/` that define
//! reusable agent configurations. The [`TemplateRegistry`] scans the
//! directory on startup and supports CRUD operations plus simple
//! `{{key}}` interpolation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use roko_core::{Body, Signal};

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
    /// System prompt template with `{{key}}` interpolation.
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

impl AgentTemplate {
    /// Validate the template for common startup-time issues.
    pub fn validate(&self, source_name: Option<&str>) -> std::result::Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.name.trim().is_empty() {
            errors.push("template name must not be empty".into());
        }
        if let Some(source_name) = source_name {
            if !source_name.is_empty() && self.name != source_name {
                errors.push(format!(
                    "template name '{}' does not match filename stem '{}'",
                    self.name, source_name
                ));
            }
        }
        if self.description.trim().is_empty() {
            errors.push(format!("template '{}' must have a description", self.name));
        }
        if self.role.trim().is_empty() {
            errors.push(format!("template '{}' must have a role", self.name));
        }
        if self.system_prompt.trim().is_empty() {
            errors.push(format!("template '{}' must have a system_prompt", self.name));
        }
        if self.max_turns == 0 {
            errors.push(format!("template '{}' must allow at least one turn", self.name));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Startup scan result for template loading.
#[derive(Debug, Default, Clone)]
pub struct TemplateLoadReport {
    /// Number of valid templates loaded into the registry.
    pub loaded: usize,
    /// Validation and parse errors encountered while loading.
    pub validation_errors: Vec<String>,
}

/// In-memory registry of agent templates backed by workspace template dirs.
pub struct TemplateRegistry {
    templates: HashMap<String, AgentTemplate>,
    workdir: PathBuf,
}

impl TemplateRegistry {
    /// Create an empty registry rooted at the workspace directory.
    pub fn new(workdir: PathBuf) -> Self {
        Self {
            templates: HashMap::new(),
            workdir,
        }
    }

    fn template_dirs(&self) -> [PathBuf; 2] {
        [
            self.workdir.join("templates"),
            self.workdir.join(".roko").join("templates"),
        ]
    }

    fn load_dir(&mut self, dir: &Path, report: &mut TemplateLoadReport) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        let mut entries = Vec::new();
        let read_dir =
            std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))?;
        for entry in read_dir {
            match entry {
                Ok(entry) => entries.push(entry.path()),
                Err(err) => report.validation_errors.push(format!(
                    "{}: failed to read directory entry: {err}",
                    dir.display()
                )),
            }
        }
        entries.sort();

        for path in entries {
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }

            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) => {
                    report
                        .validation_errors
                        .push(format!("{}: failed to read file: {err}", path.display()));
                    continue;
                }
            };

            let template: AgentTemplate = match toml::from_str(&text) {
                Ok(template) => template,
                Err(err) => {
                    report
                        .validation_errors
                        .push(format!("{}: failed to parse TOML: {err}", path.display()));
                    continue;
                }
            };

            let stem = path.file_stem().and_then(|stem| stem.to_str());
            if let Err(errors) = template.validate(stem) {
                for error in errors {
                    report
                        .validation_errors
                        .push(format!("{}: {error}", path.display()));
                }
                continue;
            }

            self.templates.insert(template.name.clone(), template);
        }

        Ok(())
    }

    /// Scan the workspace template directories for `*.toml` files and load them.
    pub fn scan(&mut self) -> TemplateLoadReport {
        self.templates.clear();
        let mut report = TemplateLoadReport::default();

        for dir in self.template_dirs() {
            if let Err(err) = self.load_dir(&dir, &mut report) {
                report
                    .validation_errors
                    .push(format!("{}: {err}", dir.display()));
            }
        }
        report.loaded = self.templates.len();

        info!(
            loaded = report.loaded,
            validation_errors = report.validation_errors.len(),
            "template scan completed"
        );
        for error in &report.validation_errors {
            warn!("{error}");
        }

        report
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
        let dir = self.workdir.join(".roko").join("templates");
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let path = dir.join(format!("{}.toml", template.name));
        let text = toml::to_string_pretty(&template).context("serialize template")?;
        std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
        self.templates.insert(template.name.clone(), template);
        Ok(())
    }

    /// Remove a template by name. Returns `true` if it existed.
    pub fn remove(&mut self, name: &str) -> Result<bool> {
        if self.templates.remove(name).is_some() {
            let path = self
                .workdir
                .join(".roko")
                .join("templates")
                .join(format!("{name}.toml"));
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("remove {}", path.display()))?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Render a template's system prompt by interpolating `{{key}}` markers
    /// with the supplied parameter values and built-in context values.
    pub fn render_prompt(template: &AgentTemplate, params: &HashMap<String, String>) -> String {
        Self::render_prompt_with_signal(template, params, None)
    }

    /// Render a template's system prompt with an attached signal context.
    ///
    /// This supports the same `{{key}}` replacement as [`render_prompt`],
    /// plus signal-derived fields such as `signal.payload.*`.
    pub fn render_prompt_with_signal(
        template: &AgentTemplate,
        params: &HashMap<String, String>,
        signal: Option<&Signal>,
    ) -> String {
        let mut values = params.clone();
        values.insert("timestamp".to_string(), Utc::now().to_rfc3339());
        values.insert(
            "env.GITHUB_TOKEN".to_string(),
            std::env::var("GITHUB_TOKEN").unwrap_or_default(),
        );

        if let Some(signal) = signal {
            values.insert(
                "signal.payload.pull_request.number".to_string(),
                signal_json_string(signal, &["pull_request", "number"]).unwrap_or_default(),
            );
            values.insert(
                "signal.payload.repository.full_name".to_string(),
                signal_json_string(signal, &["repository", "full_name"]).unwrap_or_default(),
            );
        }

        let mut out = template.system_prompt.clone();
        for (key, value) in values {
            let marker = format!("{{{{{key}}}}}");
            out = out.replace(&marker, &value);
        }
        out
    }
}

fn signal_json_string(signal: &Signal, path: &[&str]) -> Option<String> {
    let Body::Json(value) = &signal.body else {
        return None;
    };

    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }

    match current {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => Some(String::new()),
        other => Some(other.to_string()),
    }
}

fn default_model() -> String {
    "sonnet".into()
}

const fn default_max_turns() -> u32 {
    20
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_template(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn scan_loads_both_template_roots_and_reports_validation_errors() {
        let tempdir = tempfile::tempdir().unwrap();
        let workdir = tempdir.path();

        write_template(
            &workdir.join("templates").join("planner.toml"),
            r#"
name = "planner"
description = "Plan work"
role = "planner"
system_prompt = "You are a planner."
max_turns = 12
"#,
        );
        write_template(
            &workdir.join(".roko").join("templates").join("reviewer.toml"),
            r#"
name = "reviewer"
description = "Review work"
role = "reviewer"
system_prompt = "You are a reviewer."
max_turns = 8
"#,
        );
        write_template(
            &workdir.join(".roko").join("templates").join("broken.toml"),
            r#"
name = "broken"
description = ""
role = ""
system_prompt = ""
max_turns = 0
"#,
        );

        let mut registry = TemplateRegistry::new(workdir.to_path_buf());
        let report = registry.scan();

        assert_eq!(report.loaded, 2);
        assert!(report
            .validation_errors
            .iter()
            .any(|error| error.contains("broken.toml")));
        assert!(registry.get("planner").is_some());
        assert!(registry.get("reviewer").is_some());
        assert!(registry.get("broken").is_none());
    }

    #[test]
    fn render_prompt_interpolates_builtin_and_signal_values() {
        let template = AgentTemplate {
            name: "demo".into(),
            description: "Demo".into(),
            model: "sonnet".into(),
            role: "implementer".into(),
            system_prompt: [
                "PR: {{signal.payload.pull_request.number}}",
                "Repo: {{signal.payload.repository.full_name}}",
                "Token: {{env.GITHUB_TOKEN}}",
                "Time: {{timestamp}}",
            ]
            .join("\n"),
            max_turns: 1,
            output_format: TemplateOutputFormat::Markdown,
            mcp_servers: Vec::new(),
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
        };

        let signal = roko_core::Signal::builder(roko_core::Kind::Task)
            .body(roko_core::Body::from_json(&serde_json::json!({
                "pull_request": { "number": 42 },
                "repository": { "full_name": "roko/example" }
            }))
            .unwrap())
            .build();

        let rendered = TemplateRegistry::render_prompt_with_signal(
            &template,
            &HashMap::new(),
            Some(&signal),
        );

        assert!(rendered.contains("PR: 42"));
        assert!(rendered.contains("Repo: roko/example"));
        assert!(!rendered.contains("{{signal.payload.pull_request.number}}"));
        assert!(!rendered.contains("{{signal.payload.repository.full_name}}"));
        assert!(!rendered.contains("{{env.GITHUB_TOKEN}}"));
        assert!(!rendered.contains("{{timestamp}}"));
    }
}
