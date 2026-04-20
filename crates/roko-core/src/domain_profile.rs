//! Domain profiles for agent specialization (AGT-08).
//!
//! A [`DomainProfile`] bundles tool-set hints, gate configurations, context
//! templates, and operational defaults for a task domain.  The six canonical
//! profiles (Coding, Research, Chain, DataMl, Ops, Writing) provide sensible
//! defaults; users can extend or override them via [`TypedContext`].

use serde::{Deserialize, Serialize};

// ─── DomainProfile ──────────────────────────────────────────────────────

/// Canonical domain classifications for agent profiles.
///
/// Each variant carries domain-specific defaults for gate selection, tool
/// filtering, context templates, and operational parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DomainProfile {
    /// Software engineering: code generation, refactoring, testing.
    Coding,
    /// Literature review, web research, citation gathering.
    Research,
    /// On-chain / DeFi: transaction analysis, smart contracts, MEV.
    Chain,
    /// Data science and machine learning workflows.
    DataMl,
    /// Operations / SRE: deployments, monitoring, incident response.
    Ops,
    /// Documentation, technical writing, content creation.
    Writing,
}

impl DomainProfile {
    /// Stable string label for serialization and config lookup.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Research => "research",
            Self::Chain => "chain",
            Self::DataMl => "data_ml",
            Self::Ops => "ops",
            Self::Writing => "writing",
        }
    }

    /// Parse a domain profile from a string label.
    ///
    /// Returns `None` for unrecognized labels.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        match label.to_ascii_lowercase().as_str() {
            "coding" | "code" => Some(Self::Coding),
            "research" => Some(Self::Research),
            "chain" | "defi" | "onchain" => Some(Self::Chain),
            "data_ml" | "data" | "ml" => Some(Self::DataMl),
            "ops" | "sre" | "devops" => Some(Self::Ops),
            "writing" | "docs" | "documentation" => Some(Self::Writing),
            _ => None,
        }
    }

    /// Default gate rungs for this domain.
    ///
    /// Coding tasks need compile + test + clippy; research tasks only need
    /// content review; chain tasks need simulation + audit.
    #[must_use]
    pub fn default_gate_rungs(&self) -> &'static [&'static str] {
        match self {
            Self::Coding => &["compile", "test", "clippy", "diff_review"],
            Self::Research => &["content_review", "citation_check"],
            Self::Chain => &["compile", "simulation", "audit", "diff_review"],
            Self::DataMl => &["compile", "test", "notebook_check"],
            Self::Ops => &["dry_run", "diff_review", "approval"],
            Self::Writing => &["content_review", "spell_check"],
        }
    }

    /// Suggested tool category allowlist for this domain.
    ///
    /// These are category hints (not exact tool names) used by the tool
    /// selector to filter the available tool set.
    #[must_use]
    pub fn tool_categories(&self) -> &'static [&'static str] {
        match self {
            Self::Coding => &["read", "write", "edit", "search", "exec", "test"],
            Self::Research => &["read", "search", "web"],
            Self::Chain => &["read", "search", "web", "exec"],
            Self::DataMl => &["read", "write", "edit", "search", "exec", "notebook"],
            Self::Ops => &["read", "exec", "search"],
            Self::Writing => &["read", "write", "edit", "search"],
        }
    }

    /// Recommended maximum context window fraction for tasks in this domain.
    ///
    /// Research tasks tend to need more context; mechanical coding tasks less.
    #[must_use]
    pub const fn context_fraction(&self) -> f64 {
        match self {
            Self::Coding => 0.6,
            Self::Research => 0.8,
            Self::Chain => 0.7,
            Self::DataMl => 0.7,
            Self::Ops => 0.5,
            Self::Writing => 0.7,
        }
    }
}

impl std::fmt::Display for DomainProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ─── TypedContext ────────────────────────────────────────────────────────

/// Domain-typed execution context for an agent task.
///
/// Bundles the domain classification with profile-specific overrides that
/// control how gates, tools, and context are assembled for the task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedContext {
    /// The domain profile driving defaults.
    pub domain: DomainProfile,

    /// Override gate rungs (replaces [`DomainProfile::default_gate_rungs`] when set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_rungs: Option<Vec<String>>,

    /// Override tool categories (replaces [`DomainProfile::tool_categories`] when set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_categories: Option<Vec<String>>,

    /// Override context window fraction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_fraction: Option<f64>,

    /// Domain-specific key-value metadata for templates and custom logic.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub metadata: std::collections::HashMap<String, String>,
}

impl TypedContext {
    /// Create a typed context for the given domain with all-default overrides.
    #[must_use]
    pub fn new(domain: DomainProfile) -> Self {
        Self {
            domain,
            gate_rungs: None,
            tool_categories: None,
            context_fraction: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Effective gate rungs: override if set, otherwise domain default.
    #[must_use]
    pub fn effective_gate_rungs(&self) -> Vec<String> {
        self.gate_rungs.clone().unwrap_or_else(|| {
            self.domain
                .default_gate_rungs()
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        })
    }

    /// Effective tool categories: override if set, otherwise domain default.
    #[must_use]
    pub fn effective_tool_categories(&self) -> Vec<String> {
        self.tool_categories.clone().unwrap_or_else(|| {
            self.domain
                .tool_categories()
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        })
    }

    /// Effective context fraction: override if set, otherwise domain default.
    #[must_use]
    pub fn effective_context_fraction(&self) -> f64 {
        self.context_fraction
            .unwrap_or_else(|| self.domain.context_fraction())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_profile_label_roundtrip() {
        for domain in [
            DomainProfile::Coding,
            DomainProfile::Research,
            DomainProfile::Chain,
            DomainProfile::DataMl,
            DomainProfile::Ops,
            DomainProfile::Writing,
        ] {
            let label = domain.label();
            let parsed = DomainProfile::from_label(label).unwrap();
            assert_eq!(parsed, domain, "roundtrip failed for {label}");
        }
    }

    #[test]
    fn domain_profile_aliases() {
        assert_eq!(DomainProfile::from_label("code"), Some(DomainProfile::Coding));
        assert_eq!(DomainProfile::from_label("defi"), Some(DomainProfile::Chain));
        assert_eq!(DomainProfile::from_label("sre"), Some(DomainProfile::Ops));
        assert_eq!(DomainProfile::from_label("docs"), Some(DomainProfile::Writing));
        assert_eq!(DomainProfile::from_label("ml"), Some(DomainProfile::DataMl));
        assert!(DomainProfile::from_label("unknown").is_none());
    }

    #[test]
    fn typed_context_defaults() {
        let ctx = TypedContext::new(DomainProfile::Coding);
        assert_eq!(ctx.effective_gate_rungs(), vec!["compile", "test", "clippy", "diff_review"]);
        assert!(ctx.effective_tool_categories().contains(&"exec".to_string()));
        assert!((ctx.effective_context_fraction() - 0.6).abs() < f64::EPSILON);
    }

    #[test]
    fn typed_context_overrides() {
        let mut ctx = TypedContext::new(DomainProfile::Research);
        ctx.gate_rungs = Some(vec!["custom_review".to_string()]);
        ctx.context_fraction = Some(0.95);
        assert_eq!(ctx.effective_gate_rungs(), vec!["custom_review"]);
        assert!((ctx.effective_context_fraction() - 0.95).abs() < f64::EPSILON);
        // tool_categories falls back to domain default since not overridden
        assert!(ctx.effective_tool_categories().contains(&"web".to_string()));
    }
}
