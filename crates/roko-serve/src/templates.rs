//! Agent template registry.
//!
//! Templates are TOML files stored under `.roko/templates/` that define
//! reusable agent configurations. The [`TemplateRegistry`] scans the
//! directory on startup and supports CRUD operations plus simple
//! `{{key}}` interpolation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use roko_agent::mcp::find_mcp_config;
use roko_core::{Body, Engram};

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
    /// Optional prompt experiment hook for variant selection.
    #[serde(default)]
    pub experiment: Option<TemplateExperiment>,
}

/// Optional prompt experiment attached to a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateExperiment {
    /// Experiment name used to resolve a variant from the experiment store.
    pub name: String,
    /// Prompt variants assigned by the experiment store.
    #[serde(default)]
    pub variants: Vec<String>,
    /// Metric key recorded for the experiment.
    #[serde(default)]
    pub metric: String,
    /// Human-readable description of how the metric is measured.
    #[serde(default)]
    pub measured_by: String,
}

impl AgentTemplate {
    /// Validate the template for common startup-time issues.
    ///
    /// # Errors
    ///
    /// Returns a list of validation errors when required fields are blank,
    /// names do not match the source filename, model or role values are
    /// invalid, experiment configuration is incomplete, or required MCP
    /// servers are not configured.
    pub fn validate(
        &self,
        source_name: Option<&str>,
        configured_mcp_servers: Option<&HashSet<String>>,
    ) -> std::result::Result<(), Vec<String>> {
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
        if !is_valid_model_name(&self.model) {
            errors.push(format!(
                "template '{}' has invalid model '{}'",
                self.name, self.model
            ));
        }
        if self.role.trim().is_empty() {
            errors.push(format!("template '{}' must have a role", self.name));
        } else if !is_valid_template_role(&self.role) {
            errors.push(format!(
                "template '{}' has invalid role '{}'",
                self.name, self.role
            ));
        }
        if self.system_prompt.trim().is_empty() {
            errors.push(format!(
                "template '{}' must have a system_prompt",
                self.name
            ));
        }
        if self.max_turns == 0 {
            errors.push(format!(
                "template '{}' must allow at least one turn",
                self.name
            ));
        }
        if let Some(experiment) = &self.experiment
            && experiment.name.trim().is_empty()
        {
            errors.push(format!(
                "template '{}' has an experiment section with an empty name",
                self.name
            ));
        }
        if let Some(experiment) = &self.experiment {
            if experiment.variants.len() < 2 {
                errors.push(format!(
                    "template '{}' experiment '{}' must define at least two variants",
                    self.name, experiment.name
                ));
            }
            if experiment
                .variants
                .iter()
                .any(|variant| variant.trim().is_empty())
            {
                errors.push(format!(
                    "template '{}' experiment '{}' has an empty variant name",
                    self.name, experiment.name
                ));
            }
            if experiment.metric.trim().is_empty() {
                errors.push(format!(
                    "template '{}' experiment '{}' must define a metric",
                    self.name, experiment.name
                ));
            }
            if experiment.measured_by.trim().is_empty() {
                errors.push(format!(
                    "template '{}' experiment '{}' must define how the metric is measured",
                    self.name, experiment.name
                ));
            }
        }

        if !self.mcp_servers.is_empty() {
            match configured_mcp_servers {
                Some(configured) => {
                    let mut missing: Vec<String> = self
                        .mcp_servers
                        .iter()
                        .filter(|name| !configured.contains(name.as_str()))
                        .cloned()
                        .collect();
                    missing.sort();
                    missing.dedup();
                    if !missing.is_empty() {
                        errors.push(format!(
                            "template '{}' references MCP servers not configured: {}",
                            self.name,
                            missing.join(", ")
                        ));
                    }
                }
                None => {
                    errors.push(format!(
                        "template '{}' requires MCP servers {:?}, but no MCP config was found",
                        self.name, self.mcp_servers
                    ));
                }
            }
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

    fn load_dir(
        &mut self,
        dir: &Path,
        report: &mut TemplateLoadReport,
        configured_mcp_servers: Option<&HashSet<String>>,
    ) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        let mut entries = Vec::new();
        let read_dir = std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))?;
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
            if let Err(errors) = template.validate(stem, configured_mcp_servers) {
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
        let configured_mcp_servers = load_configured_mcp_servers(&self.workdir, &mut report);

        for dir in self.template_dirs() {
            if let Err(err) = self.load_dir(&dir, &mut report, configured_mcp_servers.as_ref()) {
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
    ///
    /// # Errors
    ///
    /// Returns an error if the `.roko/templates` directory cannot be created,
    /// the template cannot be serialized to TOML, or the rendered TOML cannot
    /// be written to disk.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the template existed on disk but its TOML file
    /// could not be removed.
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
        signal: Option<&Engram>,
    ) -> String {
        let mut values = params.clone();
        let variant = values.remove("variant").unwrap_or_default();
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

        let mut out = match variant.as_str() {
            "concise" => format!(
                "{}\n\n[STYLE: Be concise. Max 5 inline comments.]",
                template.system_prompt
            ),
            "thorough" => format!(
                "{}\n\n[STYLE: Be thorough. Review every file.]",
                template.system_prompt
            ),
            _ => template.system_prompt.clone(),
        };
        for (key, value) in values {
            let marker = format!("{{{{{key}}}}}");
            out = out.replace(&marker, &value);
        }
        out
    }
}

fn signal_json_string(signal: &Engram, path: &[&str]) -> Option<String> {
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

fn load_configured_mcp_servers(
    workdir: &Path,
    report: &mut TemplateLoadReport,
) -> Option<HashSet<String>> {
    match find_mcp_config(workdir) {
        Some(Ok((_path, config))) => Some(
            config
                .servers
                .into_iter()
                .map(|server| server.name)
                .collect(),
        ),
        Some(Err(err)) => {
            report.validation_errors.push(err.to_string());
            None
        }
        None => None,
    }
}

fn is_valid_model_name(model: &str) -> bool {
    let model = model.trim();
    !model.is_empty()
        && model
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':'))
}

fn is_valid_template_role(role: &str) -> bool {
    const VALID_TEMPLATE_ROLES: [&str; 7] = [
        "implementer",
        "operator",
        "planner",
        "researcher",
        "reviewer",
        "scribe",
        "verifier",
    ];

    let role = role.trim();
    !role.is_empty()
        && VALID_TEMPLATE_ROLES
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(role))
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

        let mcp_config = roko_agent::mcp::McpConfig {
            servers: vec![roko_agent::mcp::McpServerConfig {
                name: "github".to_string(),
                command: "npx".to_string(),
                args: vec![],
                env: HashMap::new(),
            }],
        };
        std::fs::write(
            workdir.join(".mcp.json"),
            serde_json::to_string(&mcp_config).unwrap(),
        )
        .unwrap();

        write_template(
            &workdir.join("templates").join("planner.toml"),
            r#"
name = "planner"
description = "Plan work"
role = "planner"
system_prompt = "You are a planner."
max_turns = 12
mcp_servers = ["github"]
"#,
        );
        write_template(
            &workdir
                .join(".roko")
                .join("templates")
                .join("reviewer.toml"),
            r#"
name = "reviewer"
description = "Review work"
role = "reviewer"
system_prompt = "You are a reviewer."
max_turns = 8
"#,
        );
        write_template(
            &workdir
                .join(".roko")
                .join("templates")
                .join("reviewer-exp.toml"),
            r#"
name = "reviewer-exp"
description = "Review work with an experiment"
role = "reviewer"
system_prompt = "You are a reviewer."
max_turns = 8

[experiment]
name = "review-depth"
variants = ["concise", "thorough"]
metric = "review_resolution_rate"
measured_by = "% of review comments resolved (feedback)"
"#,
        );
        write_template(
            &workdir.join(".roko").join("templates").join("broken.toml"),
            r#"
name = "broken"
description = ""
model = "bad model"
role = ""
system_prompt = ""
max_turns = 0
mcp_servers = ["github", "missing-server"]
"#,
        );

        let mut registry = TemplateRegistry::new(workdir.to_path_buf());
        let report = registry.scan();

        assert_eq!(report.loaded, 3);
        assert!(
            report
                .validation_errors
                .iter()
                .any(|error| error.contains("broken.toml"))
        );
        assert!(
            report
                .validation_errors
                .iter()
                .any(|error| error.contains("invalid model"))
        );
        assert!(
            report
                .validation_errors
                .iter()
                .any(|error| error.contains("must have a role"))
        );
        assert!(
            report
                .validation_errors
                .iter()
                .any(|error| error.contains("missing-server"))
        );
        assert!(registry.get("planner").is_some());
        assert!(registry.get("reviewer").is_some());
        assert!(registry.get("reviewer-exp").is_some());
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
            experiment: None,
        };

        let signal = roko_core::Engram::builder(roko_core::Kind::Task)
            .body(
                roko_core::Body::from_json(&serde_json::json!({
                    "pull_request": { "number": 42 },
                    "repository": { "full_name": "roko/example" }
                }))
                .unwrap(),
            )
            .build();

        let rendered =
            TemplateRegistry::render_prompt_with_signal(&template, &HashMap::new(), Some(&signal));

        assert!(rendered.contains("PR: 42"));
        assert!(rendered.contains("Repo: roko/example"));
        assert!(!rendered.contains("{{signal.payload.pull_request.number}}"));
        assert!(!rendered.contains("{{signal.payload.repository.full_name}}"));
        assert!(!rendered.contains("{{env.GITHUB_TOKEN}}"));
        assert!(!rendered.contains("{{timestamp}}"));
    }

    #[test]
    fn render_prompt_applies_variant_style_suffixes() {
        let template = AgentTemplate {
            name: "demo".into(),
            description: "Demo".into(),
            model: "sonnet".into(),
            role: "implementer".into(),
            system_prompt: "You are a reviewer.".into(),
            max_turns: 1,
            output_format: TemplateOutputFormat::Markdown,
            mcp_servers: Vec::new(),
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            experiment: None,
        };

        let concise = TemplateRegistry::render_prompt(
            &template,
            &HashMap::from([(String::from("variant"), String::from("concise"))]),
        );
        assert_eq!(
            concise,
            "You are a reviewer.\n\n[STYLE: Be concise. Max 5 inline comments.]"
        );

        let thorough = TemplateRegistry::render_prompt(
            &template,
            &HashMap::from([(String::from("variant"), String::from("thorough"))]),
        );
        assert_eq!(
            thorough,
            "You are a reviewer.\n\n[STYLE: Be thorough. Review every file.]"
        );

        let default = TemplateRegistry::render_prompt(&template, &HashMap::new());
        assert_eq!(default, "You are a reviewer.");
    }
}
