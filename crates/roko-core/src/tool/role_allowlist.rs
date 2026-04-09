//! Per-role default tool allowlists (§36.10).
//!
//! This module derives the per-[`AgentRole`] allowlist from each role's
//! [`ToolPermissions`](crate::ToolPermissions) and each tool's
//! [`ToolPermission`](crate::tool::ToolPermission). It's the single
//! authoritative mapping that keeps hosted backends (Claude `--tools`
//! CSV) and raw backends (Ollama tool-call loops) in lock-step:
//! both consult this list so the same agent sees the same tools
//! everywhere.
//!
//! Overrides (e.g. allowing `Scribe` to call `bash` for spellcheck) live
//! in `[agents.<role>.extra_tools]` config and are applied by the
//! registry on top of this base set.

use super::def::ToolDef;
use crate::AgentRole;

/// Filter `tools` to the subset a given `role` is permitted to call.
///
/// The decision uses only each tool's
/// [`ToolPermission`](crate::tool::ToolPermission) against the role's
/// [`ToolPermissions`](crate::ToolPermissions) — no role- or name-based
/// special cases. Tools with no required capabilities (all flags false)
/// are visible to every role.
pub fn role_allowlist(role: AgentRole, tools: &[ToolDef]) -> Vec<&ToolDef> {
    let perms = role.tool_permissions();
    tools
        .iter()
        .filter(|t| t.permission.satisfied_by(&perms))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{ToolCategory, ToolPermission};

    fn def(name: &str, perm: ToolPermission) -> ToolDef {
        ToolDef::new(name, name, ToolCategory::Read, perm)
    }

    fn all_builtins() -> Vec<ToolDef> {
        vec![
            def("read_file", ToolPermission::read_only()),
            def("grep", ToolPermission::read_only()),
            def("write_file", ToolPermission::writes()),
            def("bash", ToolPermission::executes()),
            def("web_fetch", ToolPermission::networked()),
        ]
    }

    #[test]
    fn auditor_sees_only_read_only_tools() {
        let tools = all_builtins();
        let visible = role_allowlist(AgentRole::Auditor, &tools);
        let names: Vec<&str> = visible.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["read_file", "grep"]);
    }

    #[test]
    fn implementer_sees_rw_and_exec_tools_but_not_network() {
        let tools = all_builtins();
        let visible = role_allowlist(AgentRole::Implementer, &tools);
        let names: Vec<&str> = visible.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["read_file", "grep", "write_file", "bash"]);
    }

    #[test]
    fn researcher_sees_read_network_and_exec_but_not_write() {
        let tools = all_builtins();
        let visible = role_allowlist(AgentRole::Researcher, &tools);
        let names: Vec<&str> = visible.iter().map(|t| t.name.as_str()).collect();
        // Researcher has read + exec + network (ToolPermissions::networked() grants exec too),
        // and no write. See roko-core::agent::ToolPermissions::networked.
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"web_fetch"));
        assert!(names.contains(&"bash"));
        assert!(!names.contains(&"write_file"));
    }

    #[test]
    fn integration_tester_sees_read_and_exec_not_write() {
        let tools = all_builtins();
        let visible = role_allowlist(AgentRole::IntegrationTester, &tools);
        let names: Vec<&str> = visible.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"bash"));
        assert!(!names.contains(&"write_file"));
        assert!(!names.contains(&"web_fetch"));
    }

    #[test]
    fn empty_tool_list_yields_empty_allowlist() {
        let visible = role_allowlist(AgentRole::Implementer, &[]);
        assert!(visible.is_empty());
    }

    #[test]
    fn every_role_enumerated_without_panic() {
        let tools = all_builtins();
        let all_roles: Vec<AgentRole> = std::iter::once(AgentRole::Conductor)
            .chain(AgentRole::ALL_AGENTS.iter().copied())
            .collect();
        for role in all_roles {
            let _ = role_allowlist(role, &tools);
        }
    }
}
