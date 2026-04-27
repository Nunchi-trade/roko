//! PromptAssemblyService — concrete implementation of `PromptAssembler`.
//!
//! Wraps the existing `SystemPromptBuilder` with role resolution, convention
//! detection, and gate feedback injection.

use async_trait::async_trait;
use roko_core::foundation::{PromptAssembler, PromptSpec};
use roko_core::{AgentRole, Result};
use std::path::Path;

use crate::conventions::detect_conventions;
use crate::role_prompts::role_identity_for;
use crate::system_prompt_builder::SystemPromptBuilder;

const SOURCE_SAMPLE_LIMIT: usize = 12;

/// Service that assembles system prompts via the 9-layer `SystemPromptBuilder`.
///
/// This is the canonical way to build prompts in the workflow engine. It:
/// - Resolves role identity from role name
/// - Detects project conventions from the working directory
/// - Injects gate feedback from prior iterations
/// - Applies anti-patterns
pub struct PromptAssemblyService {
    /// Default conventions text used when workdir detection is unavailable.
    default_conventions: Option<String>,
}

impl PromptAssemblyService {
    /// Create a new `PromptAssemblyService`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            default_conventions: None,
        }
    }

    /// Create with default conventions text.
    #[must_use]
    pub fn with_conventions(mut self, conventions: String) -> Self {
        self.default_conventions = Some(conventions);
        self
    }
}

impl Default for PromptAssemblyService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PromptAssembler for PromptAssemblyService {
    async fn assemble(&self, spec: PromptSpec) -> Result<String> {
        let role = resolve_role(spec.role.as_deref());
        let identity = role_identity_for(role);

        let mut builder = SystemPromptBuilder::new(identity);

        if let Some(conventions) = conventions_for_spec(&spec, self.default_conventions.as_deref())
        {
            builder = builder.with_conventions(conventions);
        }

        if let Some(task) = spec.task {
            builder = builder.with_task(task);
        }

        for feedback in spec.gate_feedback {
            builder = builder.with_gate_feedback_text(feedback);
        }

        if !spec.anti_patterns.is_empty() {
            builder = builder.with_anti_patterns(spec.anti_patterns);
        }

        Ok(builder.build())
    }
}

fn resolve_role(role: Option<&str>) -> AgentRole {
    let Some(role) = role.map(str::trim).filter(|role| !role.is_empty()) else {
        return AgentRole::Implementer;
    };
    let normalized = role.to_ascii_lowercase().replace('_', "-");

    std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS)
        .find(|candidate| candidate.label() == normalized)
        .or_else(|| serde_json::from_value(serde_json::Value::String(normalized)).ok())
        .unwrap_or(AgentRole::Implementer)
}

fn conventions_for_spec(spec: &PromptSpec, default_conventions: Option<&str>) -> Option<String> {
    spec.workdir
        .as_deref()
        .and_then(detect_workdir_conventions)
        .or_else(|| default_conventions.map(ToOwned::to_owned))
}

fn detect_workdir_conventions(workdir: &Path) -> Option<String> {
    let cargo_toml = read_to_string_if_exists(&workdir.join("Cargo.toml")).unwrap_or_default();
    let (source_samples, file_listing) = collect_source_context(workdir);

    if cargo_toml.is_empty() && source_samples.is_empty() && file_listing.is_empty() {
        return None;
    }

    let source_refs = source_samples.iter().map(String::as_str).collect::<Vec<_>>();
    let file_refs = file_listing.iter().map(String::as_str).collect::<Vec<_>>();
    let conventions = detect_conventions(&cargo_toml, &source_refs, &file_refs);
    let fragment = conventions.to_prompt_fragment();

    (!fragment.trim().is_empty()).then_some(fragment)
}

fn collect_source_context(workdir: &Path) -> (Vec<String>, Vec<String>) {
    let mut source_samples = Vec::new();
    let mut file_listing = Vec::new();
    collect_source_context_from(
        &workdir.join("src"),
        workdir,
        &mut source_samples,
        &mut file_listing,
    );
    (source_samples, file_listing)
}

fn collect_source_context_from(
    dir: &Path,
    root: &Path,
    source_samples: &mut Vec<String>,
    file_listing: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_source_context_from(&path, root, source_samples, file_listing);
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }

        if let Some(relative) = relative_path_string(&path, root) {
            file_listing.push(relative);
        }

        if source_samples.len() < SOURCE_SAMPLE_LIMIT {
            if let Some(source) = read_to_string_if_exists(&path) {
                source_samples.push(source);
            }
        }
    }
}

fn read_to_string_if_exists(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn relative_path_string(path: &Path, root: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok().unwrap_or(path);
    path_to_string(relative)
}

fn path_to_string(path: &Path) -> Option<String> {
    path.to_str()
        .map(str::to_owned)
        .or_else(|| Some(path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn basic_assembly() {
        let svc = PromptAssemblyService::new();
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix the login bug".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
    }

    #[tokio::test]
    async fn assembly_with_gate_feedback() {
        let svc = PromptAssemblyService::new();
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix the build".into()),
                gate_feedback: vec!["error[E0308]: mismatched types".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
    }

    #[tokio::test]
    async fn default_role_is_implementer() {
        let svc = PromptAssemblyService::new();
        let prompt = svc
            .assemble(PromptSpec {
                task: Some("Do something".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
    }

    #[test]
    fn resolves_role_labels_and_serde_names() {
        assert_eq!(resolve_role(Some("quick-reviewer")), AgentRole::QuickReviewer);
        assert_eq!(resolve_role(Some("quick_reviewer")), AgentRole::QuickReviewer);
        assert_eq!(resolve_role(Some("dep-validator")), AgentRole::DependencyValidator);
        assert_eq!(resolve_role(Some("unknown")), AgentRole::Implementer);
    }

    #[test]
    fn uses_default_conventions_without_workdir() {
        let spec = PromptSpec::default();
        assert_eq!(
            conventions_for_spec(&spec, Some("Use workspace conventions")),
            Some("Use workspace conventions".to_string())
        );
    }
}
