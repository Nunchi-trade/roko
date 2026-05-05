//! Manifest contracts for role profiles and prompt policies.
//!
//! These types are intentionally runtime-agnostic. The orchestrator can load
//! them from TOML before it decides which agent backend or prompt builder will
//! consume the policy.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent::{AgentRole, ToolPermissions};

/// Current role/prompt manifest schema version.
pub const CURRENT_POLICY_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Repository path for the built-in role manifest shipped with this runtime.
pub const BUILTIN_ROLE_POLICY_MANIFEST_PATH: &str =
    "crates/roko-core/src/builtin_roles/core_roles.toml";

/// Built-in manifest-backed roles used by self-hosting flows.
pub const MANIFEST_BACKED_BUILTIN_ROLE_IDS: [&str; 6] = [
    "strategist",
    "implementer",
    "architect",
    "auditor",
    "quick-reviewer",
    "scribe",
];

/// TOML source for the built-in role manifest shipped with this runtime.
pub const BUILTIN_ROLE_POLICY_MANIFEST_TOML: &str = include_str!("builtin_roles/core_roles.toml");

fn default_manifest_schema_version() -> u32 {
    CURRENT_POLICY_MANIFEST_SCHEMA_VERSION
}

/// A role and prompt-policy manifest loaded from structured TOML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RolePolicyManifest {
    /// Schema version for manifest validation and future migration.
    #[serde(default = "default_manifest_schema_version")]
    pub schema_version: u32,
    /// Versioned role profiles.
    #[serde(default)]
    pub roles: Vec<RoleProfile>,
    /// Versioned prompt policies referenced by roles.
    #[serde(default)]
    pub prompt_policies: Vec<PromptPolicy>,
}

impl RolePolicyManifest {
    /// Load and validate the built-in role/prompt manifest bundled with Roko.
    ///
    /// This returns a `Result` intentionally: if the bundled manifest is broken,
    /// callers should fail closed instead of silently falling back to generated
    /// policy for roles that claim to be manifest-backed.
    pub fn builtin_manifest() -> Result<Self, ManifestError> {
        Self::from_toml_str(BUILTIN_ROLE_POLICY_MANIFEST_TOML)
    }

    /// Parse and validate a manifest from TOML text.
    pub fn from_toml_str(text: &str) -> Result<Self, ManifestError> {
        let manifest: Self = toml::from_str(text).map_err(ManifestError::ParseToml)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Load and validate a manifest from disk.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| ManifestError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    /// Find a role profile by role id.
    #[must_use]
    pub fn role_profile(&self, role_id: &str) -> Option<&RoleProfile> {
        let role_id = role_id.trim();
        self.roles
            .iter()
            .find(|role| role.role_id.trim() == role_id)
    }

    /// Find a prompt policy by policy id.
    #[must_use]
    pub fn prompt_policy(&self, policy_id: &str) -> Option<&PromptPolicy> {
        let policy_id = policy_id.trim();
        self.prompt_policies
            .iter()
            .find(|policy| policy.policy_id.trim() == policy_id)
    }

    /// Resolve a role and its default prompt policy from the same manifest.
    pub fn role_with_default_prompt_policy(
        &self,
        role_id: &str,
    ) -> Result<(&RoleProfile, &PromptPolicy), ManifestLookupError> {
        let role = self
            .role_profile(role_id)
            .ok_or_else(|| ManifestLookupError::MissingRole {
                role_id: role_id.trim().to_string(),
            })?;
        let policy = self
            .prompt_policy(&role.default_prompt_policy)
            .ok_or_else(|| ManifestLookupError::MissingPromptPolicy {
                role_id: role.role_id.clone(),
                policy_id: role.default_prompt_policy.clone(),
            })?;
        Ok((role, policy))
    }

    /// Validate semantic integrity. Invalid manifests fail closed.
    pub fn validate(&self) -> Result<(), ManifestError> {
        let mut errors = Vec::new();

        if self.schema_version == 0 {
            errors.push(ManifestValidationError::new(
                "schema_version",
                "must be at least 1",
            ));
        } else if self.schema_version > CURRENT_POLICY_MANIFEST_SCHEMA_VERSION {
            errors.push(ManifestValidationError::new(
                "schema_version",
                format!(
                    "unsupported schema version {}; this runtime supports {}",
                    self.schema_version, CURRENT_POLICY_MANIFEST_SCHEMA_VERSION
                ),
            ));
        }

        if self.roles.is_empty() {
            errors.push(ManifestValidationError::new(
                "roles",
                "at least one role profile is required",
            ));
        }
        if self.prompt_policies.is_empty() {
            errors.push(ManifestValidationError::new(
                "prompt_policies",
                "at least one prompt policy is required",
            ));
        }

        let mut role_keys = HashSet::new();
        for (idx, role) in self.roles.iter().enumerate() {
            errors.extend(role.validation_errors(format!("roles[{idx}]")));
            let key = (role.role_id.trim(), role.version.trim());
            if !key.0.is_empty() && !key.1.is_empty() && !role_keys.insert((key.0, key.1)) {
                errors.push(ManifestValidationError::new(
                    format!("roles[{idx}]"),
                    format!(
                        "duplicate role profile '{}@{}'",
                        role.role_id.trim(),
                        role.version.trim()
                    ),
                ));
            }
        }

        let mut prompt_ids = HashSet::new();
        for (idx, policy) in self.prompt_policies.iter().enumerate() {
            errors.extend(policy.validation_errors(format!("prompt_policies[{idx}]")));
            let id = policy.policy_id.trim();
            if !id.is_empty() && !prompt_ids.insert(id) {
                errors.push(ManifestValidationError::new(
                    format!("prompt_policies[{idx}].policy_id"),
                    format!("duplicate prompt policy id '{id}'"),
                ));
            }
        }

        for (idx, role) in self.roles.iter().enumerate() {
            let policy_id = role.default_prompt_policy.trim();
            if !policy_id.is_empty() && !prompt_ids.contains(policy_id) {
                errors.push(ManifestValidationError::new(
                    format!("roles[{idx}].default_prompt_policy"),
                    format!("references missing prompt policy '{policy_id}'"),
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ManifestError::Validation { errors })
        }
    }

    /// Build a compatibility manifest for existing built-in roles.
    #[must_use]
    pub fn builtin_for_roles(roles: impl IntoIterator<Item = AgentRole>) -> Self {
        let mut manifest = Self {
            schema_version: CURRENT_POLICY_MANIFEST_SCHEMA_VERSION,
            roles: Vec::new(),
            prompt_policies: Vec::new(),
        };
        for role in roles {
            manifest.roles.push(RoleProfile::builtin(role));
            manifest.prompt_policies.push(PromptPolicy::builtin(role));
        }
        manifest
    }
}

/// Errors while resolving a role inside an already validated manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestLookupError {
    /// The requested role id is absent.
    #[error("manifest does not contain role profile '{role_id}'")]
    MissingRole {
        /// Requested role id.
        role_id: String,
    },
    /// The role points at a prompt policy absent from the manifest.
    #[error("role profile '{role_id}' references missing prompt policy '{policy_id}'")]
    MissingPromptPolicy {
        /// Role id whose policy was missing.
        role_id: String,
        /// Missing prompt policy id.
        policy_id: String,
    },
}

/// Versioned role profile independent of hardcoded orchestration branches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoleProfile {
    /// Stable role identifier, usually kebab-case.
    pub role_id: String,
    /// Profile version. Keep semantic-version compatible where practical.
    #[serde(default = "default_profile_version")]
    pub version: String,
    /// Outcomes the role is optimized to achieve.
    #[serde(default)]
    pub objectives: Vec<String>,
    /// Concrete responsibilities this role owns.
    #[serde(default)]
    pub responsibilities: Vec<String>,
    /// Prompt policy id used when the task does not override it.
    pub default_prompt_policy: String,
    /// Context policy selected for this role.
    #[serde(default)]
    pub context_policy: ContextPolicyRef,
    /// Tools and capabilities the role can request.
    #[serde(default)]
    pub tools: ToolCapabilityPolicy,
    /// Expected output shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<OutputSchemaExpectation>,
    /// Gates this role expects after execution.
    #[serde(default)]
    pub gate_expectations: Vec<GateExpectation>,
    /// Safety and escalation behavior.
    #[serde(default)]
    pub safety: RoleSafetyPolicy,
}

impl RoleProfile {
    /// Parse and validate one standalone role profile from TOML.
    pub fn from_toml_str(text: &str) -> Result<Self, ManifestError> {
        let role: Self = toml::from_str(text).map_err(ManifestError::ParseToml)?;
        let errors = role.validation_errors("role");
        if errors.is_empty() {
            Ok(role)
        } else {
            Err(ManifestError::Validation { errors })
        }
    }

    /// Build a migration-compatible profile for an existing built-in role.
    #[must_use]
    pub fn builtin(role: AgentRole) -> Self {
        let label = role.label();
        Self {
            role_id: label.to_string(),
            version: default_profile_version(),
            objectives: vec![format!(
                "Complete assigned {label} work with verifiable output."
            )],
            responsibilities: vec![
                "Respect role-scoped tools and budgets.".to_string(),
                "Produce output suitable for typed review, gates, and resume.".to_string(),
            ],
            default_prompt_policy: builtin_prompt_policy_id(role),
            context_policy: ContextPolicyRef {
                policy_id: "roko.builtin.context.default".to_string(),
                budget_tokens: None,
                bidders: vec![
                    "task-requirements".to_string(),
                    "docs-source-map".to_string(),
                    "recent-diff".to_string(),
                ],
            },
            tools: ToolCapabilityPolicy::from_permissions(role.tool_permissions()),
            output_schema: Some(OutputSchemaExpectation {
                schema_id: "roko.agent.output.unstructured-v1".to_string(),
                format: OutputFormat::Text,
                schema_ref: None,
                required: false,
            }),
            gate_expectations: vec![GateExpectation {
                gate_id: "task-contract".to_string(),
                required: true,
                command: None,
                outcome: "passed".to_string(),
            }],
            safety: RoleSafetyPolicy::default(),
        }
    }

    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        require_non_empty(&mut errors, &prefix, "role_id", &self.role_id);
        require_non_empty(&mut errors, &prefix, "version", &self.version);
        require_non_empty(
            &mut errors,
            &prefix,
            "default_prompt_policy",
            &self.default_prompt_policy,
        );
        require_non_empty_vec(&mut errors, &prefix, "objectives", &self.objectives);
        require_non_empty_vec(
            &mut errors,
            &prefix,
            "responsibilities",
            &self.responsibilities,
        );
        errors.extend(
            self.context_policy
                .validation_errors(format!("{prefix}.context_policy")),
        );
        errors.extend(self.tools.validation_errors(format!("{prefix}.tools")));
        if let Some(schema) = &self.output_schema {
            errors.extend(schema.validation_errors(format!("{prefix}.output_schema")));
        }
        for (idx, gate) in self.gate_expectations.iter().enumerate() {
            errors.extend(gate.validation_errors(format!("{prefix}.gate_expectations[{idx}]")));
        }
        errors.extend(self.safety.validation_errors(format!("{prefix}.safety")));
        errors
    }
}

/// Declarative policy for composing a prompt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptPolicy {
    /// Stable policy identifier referenced by role profiles.
    pub policy_id: String,
    /// React version.
    #[serde(default = "default_profile_version")]
    pub version: String,
    /// Ordered prompt sections.
    #[serde(default)]
    pub sections: Vec<PromptPolicySection>,
    /// React-level prompt budget.
    #[serde(default)]
    pub budget: PromptBudgetPolicy,
    /// Provenance for this policy.
    #[serde(default)]
    pub provenance: PolicyProvenance,
    /// Experiment identifiers active for this policy.
    #[serde(default)]
    pub experiment_ids: Vec<String>,
    /// Fallback if policy composition cannot include enough sections.
    #[serde(default)]
    pub fallback: FallbackBehavior,
}

impl PromptPolicy {
    /// Parse and validate one prompt policy from TOML.
    pub fn from_toml_str(text: &str) -> Result<Self, ManifestError> {
        let policy: Self = toml::from_str(text).map_err(ManifestError::ParseToml)?;
        let errors = policy.validation_errors("prompt_policy");
        if errors.is_empty() {
            Ok(policy)
        } else {
            Err(ManifestError::Validation { errors })
        }
    }

    /// Build a migration-compatible policy for an existing built-in role.
    #[must_use]
    pub fn builtin(role: AgentRole) -> Self {
        Self {
            policy_id: builtin_prompt_policy_id(role),
            version: default_profile_version(),
            sections: vec![
                PromptPolicySection::required(
                    "role_identity",
                    10,
                    PromptSectionSource::builtin(format!("roko.builtin.role.{}", role.label())),
                ),
                PromptPolicySection::required(
                    "tools",
                    20,
                    PromptSectionSource::builtin("roko.builtin.tools.role-allowlist"),
                ),
                PromptPolicySection {
                    section_id: "task".to_string(),
                    order: 90,
                    purpose: "Current task contract and acceptance criteria.".to_string(),
                    source: PromptSectionSource::context("task-contract"),
                    inclusion: InclusionRule {
                        mode: InclusionMode::Required,
                        when: Vec::new(),
                    },
                    budget: SectionBudget {
                        max_tokens: None,
                        reserve_tokens: None,
                    },
                    provenance: PolicyProvenance {
                        source_id: "roko.builtin.prompt.task".to_string(),
                        path: None,
                        owner: Some("roko-compose".to_string()),
                        generated_by: None,
                    },
                    experiment_ids: Vec::new(),
                    fallback: FallbackBehavior::FailClosed,
                },
            ],
            budget: PromptBudgetPolicy {
                max_tokens: None,
                max_cost_usd_cents: None,
                reserve_tokens: Some(512),
            },
            provenance: PolicyProvenance {
                source_id: format!("roko.builtin.prompt.{}", role.label()),
                path: Some("crates/roko-compose/src/role_prompts.rs".to_string()),
                owner: Some("roko-compose".to_string()),
                generated_by: Some("RoleSystemPromptSpec".to_string()),
            },
            experiment_ids: Vec::new(),
            fallback: FallbackBehavior::FailClosed,
        }
    }

    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        require_non_empty(&mut errors, &prefix, "policy_id", &self.policy_id);
        require_non_empty(&mut errors, &prefix, "version", &self.version);
        if self.sections.is_empty() {
            errors.push(ManifestValidationError::new(
                format!("{prefix}.sections"),
                "at least one prompt section is required",
            ));
        }
        errors.extend(self.budget.validation_errors(format!("{prefix}.budget")));
        errors.extend(
            self.provenance
                .validation_errors(format!("{prefix}.provenance"), false),
        );
        errors.extend(validate_non_empty_unique_strings(
            format!("{prefix}.experiment_ids"),
            &self.experiment_ids,
        ));
        let mut section_ids = HashSet::new();
        let mut section_orders = HashSet::new();
        for (idx, section) in self.sections.iter().enumerate() {
            errors.extend(section.validation_errors(format!("{prefix}.sections[{idx}]")));
            let id = section.section_id.trim();
            if !id.is_empty() && !section_ids.insert(id) {
                errors.push(ManifestValidationError::new(
                    format!("{prefix}.sections[{idx}].section_id"),
                    format!("duplicate section id '{id}'"),
                ));
            }
            if !section_orders.insert(section.order) {
                errors.push(ManifestValidationError::new(
                    format!("{prefix}.sections[{idx}].order"),
                    format!("duplicate section order {}", section.order),
                ));
            }
        }
        errors
    }
}

/// Reference to a role context policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicyRef {
    /// Stable context policy id.
    #[serde(default = "default_context_policy_id")]
    pub policy_id: String,
    /// Optional token budget for context sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
    /// Context bidders allowed to contribute.
    #[serde(default)]
    pub bidders: Vec<String>,
}

impl Default for ContextPolicyRef {
    fn default() -> Self {
        Self {
            policy_id: default_context_policy_id(),
            budget_tokens: None,
            bidders: Vec::new(),
        }
    }
}

impl ContextPolicyRef {
    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        require_non_empty(&mut errors, &prefix, "policy_id", &self.policy_id);
        errors.extend(validate_non_empty_unique_strings(
            format!("{prefix}.bidders"),
            &self.bidders,
        ));
        if matches!(self.budget_tokens, Some(0)) {
            errors.push(ManifestValidationError::new(
                format!("{prefix}.budget_tokens"),
                "must be greater than zero when set",
            ));
        }
        errors
    }
}

/// Role-local tool and capability declarations.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCapabilityPolicy {
    /// Tool names this role may call.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tool names expected to be available.
    #[serde(default)]
    pub required_tools: Vec<String>,
    /// Higher-level capabilities granted to the role.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl ToolCapabilityPolicy {
    /// Convert legacy boolean tool permissions into manifest capabilities.
    #[must_use]
    pub fn from_permissions(permissions: ToolPermissions) -> Self {
        let mut capabilities = Vec::new();
        if permissions.read {
            capabilities.push("filesystem.read".to_string());
        }
        if permissions.write {
            capabilities.push("filesystem.write".to_string());
        }
        if permissions.exec {
            capabilities.push("process.exec".to_string());
        }
        if permissions.git {
            capabilities.push("vcs.git".to_string());
        }
        if permissions.network {
            capabilities.push("network.http".to_string());
        }
        Self {
            allowed_tools: Vec::new(),
            required_tools: Vec::new(),
            capabilities,
        }
    }

    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        errors.extend(validate_non_empty_unique_strings(
            format!("{prefix}.allowed_tools"),
            &self.allowed_tools,
        ));
        errors.extend(validate_non_empty_unique_strings(
            format!("{prefix}.required_tools"),
            &self.required_tools,
        ));
        errors.extend(validate_non_empty_unique_strings(
            format!("{prefix}.capabilities"),
            &self.capabilities,
        ));
        let allowed = self
            .allowed_tools
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        for tool in &self.required_tools {
            if !allowed.contains(tool.as_str()) {
                errors.push(ManifestValidationError::new(
                    format!("{prefix}.required_tools"),
                    format!("required tool '{tool}' is not present in allowed_tools"),
                ));
            }
        }
        if self.allowed_tools.is_empty() && self.capabilities.is_empty() {
            errors.push(ManifestValidationError::new(
                prefix,
                "at least one allowed tool or capability is required",
            ));
        }
        errors
    }
}

/// Expected output contract for a role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSchemaExpectation {
    /// Stable schema id.
    pub schema_id: String,
    /// Output format.
    #[serde(default)]
    pub format: OutputFormat,
    /// Optional reference to a JSON schema or Rust type path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_ref: Option<String>,
    /// Whether the role output must parse as this schema.
    #[serde(default = "default_true")]
    pub required: bool,
}

impl OutputSchemaExpectation {
    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        require_non_empty(&mut errors, &prefix, "schema_id", &self.schema_id);
        if let Some(schema_ref) = &self.schema_ref {
            require_non_empty(&mut errors, &prefix, "schema_ref", schema_ref);
        }
        errors
    }
}

/// Output formats understood by role manifests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// Plain text output.
    #[default]
    Text,
    /// JSON object or array output.
    Json,
    /// TOML output.
    Toml,
    /// Markdown output.
    Markdown,
}

/// Verify expected after a role invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateExpectation {
    /// Verify identifier.
    pub gate_id: String,
    /// Whether failure blocks task completion.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Optional command associated with the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Expected outcome state.
    #[serde(default = "default_passed_outcome")]
    pub outcome: String,
}

impl GateExpectation {
    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        require_non_empty(&mut errors, &prefix, "gate_id", &self.gate_id);
        require_non_empty(&mut errors, &prefix, "outcome", &self.outcome);
        if let Some(command) = &self.command {
            require_non_empty(&mut errors, &prefix, "command", command);
        }
        errors
    }
}

/// Safety and escalation behavior for a role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleSafetyPolicy {
    /// What to do when the role cannot satisfy its contract.
    #[serde(default = "default_escalation")]
    pub escalation: String,
    /// Hard behavioral bounds for this role.
    #[serde(default)]
    pub bounds: Vec<String>,
}

impl Default for RoleSafetyPolicy {
    fn default() -> Self {
        Self {
            escalation: default_escalation(),
            bounds: Vec::new(),
        }
    }
}

impl RoleSafetyPolicy {
    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        require_non_empty(&mut errors, &prefix, "escalation", &self.escalation);
        errors.extend(validate_non_empty_unique_strings(
            format!("{prefix}.bounds"),
            &self.bounds,
        ));
        errors
    }
}

/// One section in an ordered prompt policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptPolicySection {
    /// Stable section id.
    pub section_id: String,
    /// Sort order. Lower values render first.
    pub order: u16,
    /// Why this section exists.
    pub purpose: String,
    /// Structured source of the section content.
    pub source: PromptSectionSource,
    /// Inclusion policy.
    #[serde(default)]
    pub inclusion: InclusionRule,
    /// Section-local budget.
    #[serde(default)]
    pub budget: SectionBudget,
    /// Section-level provenance.
    #[serde(default)]
    pub provenance: PolicyProvenance,
    /// Section experiment ids.
    #[serde(default)]
    pub experiment_ids: Vec<String>,
    /// Section fallback behavior.
    #[serde(default)]
    pub fallback: FallbackBehavior,
}

impl PromptPolicySection {
    /// Create a required section.
    #[must_use]
    pub fn required(
        section_id: impl Into<String>,
        order: u16,
        source: PromptSectionSource,
    ) -> Self {
        let section_id = section_id.into();
        Self {
            purpose: format!("Required {section_id} prompt section."),
            section_id,
            order,
            source,
            inclusion: InclusionRule {
                mode: InclusionMode::Required,
                when: Vec::new(),
            },
            budget: SectionBudget::default(),
            provenance: PolicyProvenance::default(),
            experiment_ids: Vec::new(),
            fallback: FallbackBehavior::FailClosed,
        }
    }

    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        require_non_empty(&mut errors, &prefix, "section_id", &self.section_id);
        require_non_empty(&mut errors, &prefix, "purpose", &self.purpose);
        errors.extend(self.source.validation_errors(format!("{prefix}.source")));
        errors.extend(
            self.inclusion
                .validation_errors(format!("{prefix}.inclusion")),
        );
        errors.extend(self.budget.validation_errors(format!("{prefix}.budget")));
        errors.extend(
            self.provenance
                .validation_errors(format!("{prefix}.provenance"), true),
        );
        errors.extend(validate_non_empty_unique_strings(
            format!("{prefix}.experiment_ids"),
            &self.experiment_ids,
        ));
        errors
    }
}

/// Structured source for prompt section content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSectionSource {
    /// Source kind, for example `builtin`, `manifest`, `context`, or `file`.
    pub kind: String,
    /// Source identifier within the kind.
    pub id: String,
}

impl PromptSectionSource {
    /// Built-in source reference.
    #[must_use]
    pub fn builtin(id: impl Into<String>) -> Self {
        Self {
            kind: "builtin".to_string(),
            id: id.into(),
        }
    }

    /// Context source reference.
    #[must_use]
    pub fn context(id: impl Into<String>) -> Self {
        Self {
            kind: "context".to_string(),
            id: id.into(),
        }
    }

    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        require_non_empty(&mut errors, &prefix, "kind", &self.kind);
        require_non_empty(&mut errors, &prefix, "id", &self.id);
        errors
    }
}

/// Inclusion policy for a prompt section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionRule {
    /// Required, optional, or conditional.
    #[serde(default)]
    pub mode: InclusionMode,
    /// Predicate labels or feature expressions that activate this section.
    #[serde(default)]
    pub when: Vec<String>,
}

impl Default for InclusionRule {
    fn default() -> Self {
        Self {
            mode: InclusionMode::Required,
            when: Vec::new(),
        }
    }
}

impl InclusionRule {
    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = validate_non_empty_unique_strings(format!("{prefix}.when"), &self.when);
        if self.mode == InclusionMode::Conditional && self.when.is_empty() {
            errors.push(ManifestValidationError::new(
                prefix,
                "conditional sections require at least one inclusion predicate",
            ));
        }
        errors
    }
}

/// Inclusion modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InclusionMode {
    /// Include or fail.
    #[default]
    Required,
    /// Include only if budget and scoring allow it.
    Optional,
    /// Include only when predicates match.
    Conditional,
}

/// Prompt or section budget settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptBudgetPolicy {
    /// Maximum prompt tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Maximum prompt cost in USD cents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd_cents: Option<u32>,
    /// Tokens reserved for task/runtime context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserve_tokens: Option<u32>,
}

impl PromptBudgetPolicy {
    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        if matches!(self.max_tokens, Some(0)) {
            errors.push(ManifestValidationError::new(
                format!("{prefix}.max_tokens"),
                "must be greater than zero when set",
            ));
        }
        if matches!(self.max_cost_usd_cents, Some(0)) {
            errors.push(ManifestValidationError::new(
                format!("{prefix}.max_cost_usd_cents"),
                "must be greater than zero when set",
            ));
        }
        errors
    }
}

/// Per-section budget.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionBudget {
    /// Maximum section tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Minimum reserved tokens for required sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserve_tokens: Option<u32>,
}

impl SectionBudget {
    fn validation_errors(&self, prefix: impl Into<String>) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        if matches!(self.max_tokens, Some(0)) {
            errors.push(ManifestValidationError::new(
                format!("{prefix}.max_tokens"),
                "must be greater than zero when set",
            ));
        }
        if matches!(self.reserve_tokens, Some(0)) {
            errors.push(ManifestValidationError::new(
                format!("{prefix}.reserve_tokens"),
                "must be greater than zero when set",
            ));
        }
        errors
    }
}

/// Provenance metadata attached to prompt policies and sections.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyProvenance {
    /// Source id for audits and CognitiveWorkspace records.
    #[serde(default)]
    pub source_id: String,
    /// File path or module path, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Owning crate/system/team.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Generator identifier, when the policy was generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_by: Option<String>,
}

impl PolicyProvenance {
    fn validation_errors(
        &self,
        prefix: impl Into<String>,
        allow_empty_source_id: bool,
    ) -> Vec<ManifestValidationError> {
        let prefix = prefix.into();
        let mut errors = Vec::new();
        if !allow_empty_source_id {
            require_non_empty(&mut errors, &prefix, "source_id", &self.source_id);
        } else if self.source_id.trim().is_empty()
            && (self.path.is_some() || self.owner.is_some() || self.generated_by.is_some())
        {
            errors.push(ManifestValidationError::new(
                format!("{prefix}.source_id"),
                "is required when other provenance fields are set",
            ));
        }
        if let Some(path) = &self.path {
            require_non_empty(&mut errors, &prefix, "path", path);
        }
        if let Some(owner) = &self.owner {
            require_non_empty(&mut errors, &prefix, "owner", owner);
        }
        if let Some(generated_by) = &self.generated_by {
            require_non_empty(&mut errors, &prefix, "generated_by", generated_by);
        }
        errors
    }
}

/// Fallback behavior when prompt composition cannot satisfy policy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "strategy", content = "value")]
pub enum FallbackBehavior {
    /// Fail validation/composition instead of silently omitting required context.
    #[default]
    FailClosed,
    /// Omit this optional section.
    Omit,
    /// Use the given static text.
    StaticText(String),
}

/// Validation error returned by semantic manifest checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestValidationError {
    /// Path to the invalid field.
    pub path: String,
    /// Actionable validation message.
    pub message: String,
}

impl ManifestValidationError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ManifestValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Errors while loading or validating role/prompt manifests.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// File read failed.
    #[error("failed to read manifest {path}: {source}")]
    Read {
        /// Manifest path.
        path: String,
        /// IO source error.
        source: std::io::Error,
    },
    /// TOML parse failed.
    #[error("failed to parse manifest TOML: {0}")]
    ParseToml(#[from] toml::de::Error),
    /// Semantic validation failed.
    #[error("manifest validation failed: {}", format_validation_errors(.errors))]
    Validation {
        /// Validation failures.
        errors: Vec<ManifestValidationError>,
    },
}

fn format_validation_errors(errors: &[ManifestValidationError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn default_profile_version() -> String {
    "1.0.0".to_string()
}

fn default_context_policy_id() -> String {
    "roko.context.default".to_string()
}

fn default_passed_outcome() -> String {
    "passed".to_string()
}

fn default_escalation() -> String {
    "needs_human".to_string()
}

const fn default_true() -> bool {
    true
}

fn builtin_prompt_policy_id(role: AgentRole) -> String {
    format!("roko.builtin.prompt.{}", role.label())
}

fn require_non_empty(
    errors: &mut Vec<ManifestValidationError>,
    prefix: &str,
    field: &str,
    value: &str,
) {
    if value.trim().is_empty() {
        errors.push(ManifestValidationError::new(
            format!("{prefix}.{field}"),
            "must not be empty",
        ));
    }
}

fn require_non_empty_vec(
    errors: &mut Vec<ManifestValidationError>,
    prefix: &str,
    field: &str,
    values: &[String],
) {
    if values.is_empty() {
        errors.push(ManifestValidationError::new(
            format!("{prefix}.{field}"),
            "at least one entry is required",
        ));
    }
    errors.extend(validate_non_empty_unique_strings(
        format!("{prefix}.{field}"),
        values,
    ));
}

fn validate_non_empty_unique_strings(
    path: impl Into<String>,
    values: &[String],
) -> Vec<ManifestValidationError> {
    let path = path.into();
    let mut errors = Vec::new();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (idx, value) in values.iter().enumerate() {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            errors.push(ManifestValidationError::new(
                format!("{path}[{idx}]"),
                "must not be empty",
            ));
        } else if let Some(first_idx) = seen.insert(trimmed, idx) {
            errors.push(ManifestValidationError::new(
                format!("{path}[{idx}]"),
                format!("duplicates entry at index {first_idx}: '{trimmed}'"),
            ));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentRole;

    const VALID_MANIFEST: &str = r#"
schema_version = 1

[[roles]]
role_id = "implementer"
version = "1.0.0"
objectives = ["Implement code"]
responsibilities = ["Edit files", "Run focused gates"]
default_prompt_policy = "prompt.implementer.v1"

[roles.context_policy]
policy_id = "context.coding.v1"
budget_tokens = 12000
bidders = ["task-requirements", "recent-diff"]

[roles.tools]
allowed_tools = ["read_file", "write_file", "bash"]
required_tools = ["read_file"]
capabilities = ["filesystem.read", "filesystem.write", "process.exec"]

[roles.output_schema]
schema_id = "agent.patch.v1"
format = "json"
schema_ref = "schemas/agent-patch.schema.json"
required = true

[[roles.gate_expectations]]
gate_id = "cargo-check"
required = true
command = "cargo check"
outcome = "passed"

[roles.safety]
escalation = "needs_replan"
bounds = ["no destructive git commands"]

[[prompt_policies]]
policy_id = "prompt.implementer.v1"
version = "1.0.0"
experiment_ids = ["prompt-rt11-a"]
fallback = { strategy = "fail_closed" }

[prompt_policies.budget]
max_tokens = 10000
reserve_tokens = 1000

[prompt_policies.provenance]
source_id = "manifest.test"
path = "roles/implementer.toml"
owner = "roko-core"

[[prompt_policies.sections]]
section_id = "role_identity"
order = 10
purpose = "Defines the agent role."
source = { kind = "manifest", id = "roles.implementer.identity" }
inclusion = { mode = "required" }
budget = { max_tokens = 400 }
provenance = { source_id = "manifest.test.role_identity" }

[[prompt_policies.sections]]
section_id = "task"
order = 20
purpose = "Current task contract."
source = { kind = "context", id = "task-contract" }
inclusion = { mode = "conditional", when = ["task.present"] }
fallback = { strategy = "fail_closed" }
"#;

    #[test]
    fn valid_manifest_parses_and_validates() {
        let manifest = RolePolicyManifest::from_toml_str(VALID_MANIFEST).expect("valid manifest");

        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.roles[0].role_id, "implementer");
        assert_eq!(
            manifest.roles[0].default_prompt_policy,
            "prompt.implementer.v1"
        );
        assert_eq!(manifest.prompt_policies[0].sections.len(), 2);
    }

    #[test]
    fn invalid_manifest_fails_closed_with_actionable_errors() {
        let bad = r#"
schema_version = 1

[[roles]]
role_id = ""
objectives = []
responsibilities = ["Do work"]
default_prompt_policy = "missing"

[roles.tools]
allowed_tools = ["read_file", "read_file"]
required_tools = ["bash"]

[[prompt_policies]]
policy_id = "prompt.empty"
version = "1.0.0"
"#;

        let err = RolePolicyManifest::from_toml_str(bad).expect_err("should fail");
        let ManifestError::Validation { errors } = err else {
            panic!("expected validation error");
        };
        let rendered = format_validation_errors(&errors);
        assert!(rendered.contains("roles[0].role_id: must not be empty"));
        assert!(rendered.contains("roles[0].objectives: at least one entry is required"));
        assert!(rendered.contains("duplicates entry"));
        assert!(rendered.contains("required tool 'bash' is not present in allowed_tools"));
        assert!(rendered.contains("references missing prompt policy 'missing'"));
        assert!(rendered.contains("at least one prompt section is required"));
    }

    #[test]
    fn manifest_defaults_version_and_fallback() {
        let text = r#"
[[roles]]
role_id = "reviewer"
objectives = ["Review"]
responsibilities = ["Find blocking issues"]
default_prompt_policy = "prompt.reviewer"

[roles.tools]
capabilities = ["filesystem.read"]

[[prompt_policies]]
policy_id = "prompt.reviewer"

[prompt_policies.provenance]
source_id = "test"

[[prompt_policies.sections]]
section_id = "role"
order = 1
purpose = "Role identity"
source = { kind = "manifest", id = "role" }
"#;

        let manifest = RolePolicyManifest::from_toml_str(text).expect("defaults are valid");
        assert_eq!(
            manifest.schema_version,
            CURRENT_POLICY_MANIFEST_SCHEMA_VERSION
        );
        assert_eq!(manifest.roles[0].version, "1.0.0");
        assert_eq!(
            manifest.prompt_policies[0].fallback,
            FallbackBehavior::FailClosed
        );
        assert_eq!(
            manifest.prompt_policies[0].sections[0].inclusion.mode,
            InclusionMode::Required
        );
    }

    #[test]
    fn unsupported_manifest_versions_fail() {
        let text = VALID_MANIFEST.replace("schema_version = 1", "schema_version = 999");
        let err = RolePolicyManifest::from_toml_str(&text).expect_err("unsupported version");
        assert!(err.to_string().contains("unsupported schema version 999"));
    }

    #[test]
    fn duplicate_section_order_fails() {
        let text = VALID_MANIFEST.replace("order = 20", "order = 10");
        let err = RolePolicyManifest::from_toml_str(&text).expect_err("duplicate order");
        assert!(err.to_string().contains("duplicate section order 10"));
    }

    #[test]
    fn built_in_roles_have_manifest_migration_path() {
        let manifest = RolePolicyManifest::builtin_for_roles([
            AgentRole::Implementer,
            AgentRole::QuickReviewer,
        ]);

        manifest.validate().expect("built-in manifest is valid");
        assert_eq!(manifest.roles[0].role_id, "implementer");
        assert_eq!(
            manifest.roles[0].default_prompt_policy,
            "roko.builtin.prompt.implementer"
        );
        assert!(
            manifest.prompt_policies[0]
                .sections
                .iter()
                .any(|section| section.section_id == "role_identity")
        );
    }

    #[test]
    fn bundled_role_manifest_loads_core_self_hosting_roles() {
        let manifest = RolePolicyManifest::builtin_manifest().expect("bundled manifest");

        for role_id in MANIFEST_BACKED_BUILTIN_ROLE_IDS {
            let (role, policy) = manifest
                .role_with_default_prompt_policy(role_id)
                .expect("role and prompt policy");
            assert_eq!(role.role_id, role_id);
            assert_eq!(role.default_prompt_policy, policy.policy_id);
            assert_eq!(role.version, "1.0.0");
            assert!(
                policy
                    .sections
                    .iter()
                    .any(|section| section.section_id == "role_identity"
                        && section.source.kind == "manifest"),
                "{role_id} should source role identity from the manifest"
            );
        }
    }

    #[test]
    fn broken_role_manifest_fails_closed() {
        let broken = BUILTIN_ROLE_POLICY_MANIFEST_TOML.replace(
            "default_prompt_policy = \"roko.builtin.prompt.implementer\"",
            "default_prompt_policy = \"roko.builtin.prompt.missing\"",
        );

        let err = RolePolicyManifest::from_toml_str(&broken).expect_err("broken manifest fails");
        assert!(
            err.to_string()
                .contains("references missing prompt policy 'roko.builtin.prompt.missing'"),
            "{err}"
        );
    }
}
