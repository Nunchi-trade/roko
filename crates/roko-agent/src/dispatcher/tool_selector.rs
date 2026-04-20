//! Role-based tool selection for the ToolDispatcher (AGT-09).
//!
//! Narrows the tool set exposed to an agent based on its role, reducing noise
//! and improving accuracy. Research shows agents perform worse when exposed to
//! >50 tools (Qwen3-coder format switching, WildToolBench <15% session accuracy).
//!
//! The selector applies a role -> tool-set mapping and optionally consults
//! learned [`ToolUsageProfile`] data to further refine the set.

use roko_core::AgentRole;
use std::collections::HashSet;

/// A tool selector that filters available tools based on role and task metadata.
///
/// # Usage
///
/// ```ignore
/// let selector = ToolSelector::for_role(AgentRole::Implementer);
/// let allowed = selector.filter_tools(&all_tool_names);
/// ```
#[derive(Debug, Clone)]
pub struct ToolSelector {
    /// Base tool set for the role.
    base_tools: HashSet<String>,
    /// Additional tools explicitly granted (e.g., from task config).
    extra_tools: HashSet<String>,
    /// Tools explicitly blocked (takes priority over everything).
    blocked_tools: HashSet<String>,
}

impl ToolSelector {
    /// Create a selector for the given agent role.
    ///
    /// Uses built-in role -> tool-set mappings. All roles get the
    /// read-only baseline; write/exec tools are added by role.
    #[must_use]
    pub fn for_role(role: AgentRole) -> Self {
        Self {
            base_tools: tools_for_role(role),
            extra_tools: HashSet::new(),
            blocked_tools: HashSet::new(),
        }
    }

    /// Create a selector that allows all tools (no filtering).
    #[must_use]
    pub fn allow_all() -> Self {
        Self {
            base_tools: HashSet::new(),
            extra_tools: HashSet::new(),
            blocked_tools: HashSet::new(),
        }
    }

    /// Grant additional tools beyond the role baseline.
    #[must_use]
    pub fn with_extra_tools(mut self, tools: impl IntoIterator<Item = String>) -> Self {
        self.extra_tools.extend(tools);
        self
    }

    /// Block specific tools (overrides base and extra).
    #[must_use]
    pub fn with_blocked_tools(mut self, tools: impl IntoIterator<Item = String>) -> Self {
        self.blocked_tools.extend(tools);
        self
    }

    /// Check whether a specific tool is allowed by this selector.
    ///
    /// Returns `true` if:
    /// - The base set is empty (allow-all mode), OR
    /// - The tool is in `base_tools` or `extra_tools`
    /// AND the tool is NOT in `blocked_tools`.
    #[must_use]
    pub fn is_allowed(&self, tool_name: &str) -> bool {
        if self.blocked_tools.contains(tool_name) {
            return false;
        }
        if self.base_tools.is_empty() && self.extra_tools.is_empty() {
            return true; // allow-all mode
        }
        self.base_tools.contains(tool_name) || self.extra_tools.contains(tool_name)
    }

    /// Filter a list of tool names, returning only allowed tools.
    pub fn filter_tools<'a>(&self, tools: &[&'a str]) -> Vec<&'a str> {
        tools
            .iter()
            .copied()
            .filter(|t| self.is_allowed(t))
            .collect()
    }

    /// Number of tools in the base set (0 means allow-all).
    #[must_use]
    pub fn base_tool_count(&self) -> usize {
        self.base_tools.len()
    }
}

// ── Read-only tools (available to ALL roles) ────────────────────────────

fn read_only_tools() -> HashSet<String> {
    [
        "read_file",
        "read",
        "glob",
        "grep",
        "list_directory",
        "search_files",
        "find_definition",
        "find_references",
        "get_symbols",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

// ── Write tools (coding agents) ─────────────────────────────────────────

fn write_tools() -> HashSet<String> {
    [
        "write_file",
        "write",
        "edit_file",
        "edit",
        "create_file",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

// ── Exec tools (agents that run commands) ───────────────────────────────

fn exec_tools() -> HashSet<String> {
    ["bash", "execute_command", "shell"]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

// ── Git tools ───────────────────────────────────────────────────────────

fn git_tools() -> HashSet<String> {
    ["git_status", "git_diff", "git_log", "git_commit"]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Map a role to its base tool set.
///
/// Follows the principle of least privilege: each role gets only the tools
/// it needs. Read-only tools are universal; write/exec are added per role.
fn tools_for_role(role: AgentRole) -> HashSet<String> {
    let mut tools = read_only_tools();

    match role {
        // Coding roles: full read + write + exec.
        AgentRole::Implementer | AgentRole::AutoFixer | AgentRole::Refactorer => {
            tools.extend(write_tools());
            tools.extend(exec_tools());
            tools.extend(git_tools());
        }

        // Strategy/planning roles: read-only + limited exec for cargo check.
        AgentRole::Strategist | AgentRole::PrePlanner => {
            tools.extend(exec_tools());
        }

        // Review roles: read-only + exec for running tests.
        AgentRole::Architect | AgentRole::Auditor | AgentRole::QuickReviewer | AgentRole::Critic => {
            tools.extend(exec_tools());
        }

        // Research role: read-only (no writes, no exec).
        AgentRole::Researcher => {
            // read_only_tools only
        }

        // Documentation roles: read + write (for docs), limited exec.
        AgentRole::Scribe | AgentRole::DocVerifier => {
            tools.extend(write_tools());
            tools.extend(exec_tools());
        }

        // Test roles: read + exec.
        AgentRole::IntegrationTester => {
            tools.extend(exec_tools());
        }

        // Conductor: read-only (orchestration, not implementation).
        AgentRole::Conductor => {
            // read_only_tools only
        }

        // Merge resolver: full access needed for conflict resolution.
        AgentRole::MergeResolver => {
            tools.extend(write_tools());
            tools.extend(exec_tools());
            tools.extend(git_tools());
        }

        // Catch-all for unknown future roles: allow all tools.
        _ => {
            return HashSet::new();
        }
    }

    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implementer_has_write_and_exec() {
        let selector = ToolSelector::for_role(AgentRole::Implementer);
        assert!(selector.is_allowed("read_file"));
        assert!(selector.is_allowed("write_file"));
        assert!(selector.is_allowed("bash"));
        assert!(selector.is_allowed("git_status"));
    }

    #[test]
    fn researcher_has_read_only() {
        let selector = ToolSelector::for_role(AgentRole::Researcher);
        assert!(selector.is_allowed("read_file"));
        assert!(selector.is_allowed("grep"));
        assert!(!selector.is_allowed("write_file"));
        assert!(!selector.is_allowed("bash"));
        assert!(!selector.is_allowed("git_commit"));
    }

    #[test]
    fn conductor_has_read_only() {
        let selector = ToolSelector::for_role(AgentRole::Conductor);
        assert!(selector.is_allowed("read_file"));
        assert!(!selector.is_allowed("write_file"));
        assert!(!selector.is_allowed("bash"));
    }

    #[test]
    fn reviewer_has_read_and_exec() {
        let selector = ToolSelector::for_role(AgentRole::Architect);
        assert!(selector.is_allowed("read_file"));
        assert!(selector.is_allowed("bash"));
        assert!(!selector.is_allowed("write_file"));
    }

    #[test]
    fn blocked_tools_override_base() {
        let selector = ToolSelector::for_role(AgentRole::Implementer)
            .with_blocked_tools(vec!["bash".to_string()]);
        assert!(selector.is_allowed("read_file"));
        assert!(selector.is_allowed("write_file"));
        assert!(!selector.is_allowed("bash"));
    }

    #[test]
    fn extra_tools_extend_base() {
        let selector = ToolSelector::for_role(AgentRole::Researcher)
            .with_extra_tools(vec!["custom_search".to_string()]);
        assert!(selector.is_allowed("read_file"));
        assert!(selector.is_allowed("custom_search"));
        assert!(!selector.is_allowed("write_file"));
    }

    #[test]
    fn allow_all_allows_everything() {
        let selector = ToolSelector::allow_all();
        assert!(selector.is_allowed("read_file"));
        assert!(selector.is_allowed("write_file"));
        assert!(selector.is_allowed("bash"));
        assert!(selector.is_allowed("any_random_tool"));
    }

    #[test]
    fn filter_tools_returns_subset() {
        let selector = ToolSelector::for_role(AgentRole::Researcher);
        let all = &["read_file", "write_file", "grep", "bash", "glob"];
        let filtered = selector.filter_tools(all);
        assert_eq!(filtered, vec!["read_file", "grep", "glob"]);
    }

    #[test]
    fn all_roles_have_read_access() {
        let roles = [
            AgentRole::Conductor,
            AgentRole::Strategist,
            AgentRole::Implementer,
            AgentRole::Architect,
            AgentRole::Researcher,
            AgentRole::Auditor,
            AgentRole::Scribe,
            AgentRole::Refactorer,
        ];
        for role in roles {
            let selector = ToolSelector::for_role(role);
            assert!(
                selector.is_allowed("read_file"),
                "{role:?} should have read_file access"
            );
        }
    }

    #[test]
    fn base_tool_count_reflects_role() {
        let researcher = ToolSelector::for_role(AgentRole::Researcher);
        let implementer = ToolSelector::for_role(AgentRole::Implementer);
        // Implementer should have more tools than researcher.
        assert!(implementer.base_tool_count() > researcher.base_tool_count());
    }
}
