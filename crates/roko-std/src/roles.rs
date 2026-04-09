//! Role-based tool profiles.
//!
//! These profiles define the canonical tool-name allow/deny sets used by
//! higher-level orchestration code when constructing per-role tool menus.
//! The names are the internal snake_case tool identifiers from
//! `roko-std`'s built-in registry.

use crate::tool::builtin::{
    apply_patch, bash, edit_file, exit_plan_mode, glob, grep, multi_edit, notebook_edit, read_file,
    run_tests, task_agent, todo_write, web_fetch, web_search, write_file,
};

/// Named tool-profile archetypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoleToolProfileKind {
    /// Code-producing role. No extra filtering beyond the global registry.
    Implementer,
    /// Evidence-gathering role. Read-only tools only.
    Researcher,
    /// Diff-review role. Read tools plus comment/note tools.
    Reviewer,
    /// Planning role. Read tools plus plan-management tools.
    Strategist,
}

/// Canonical tool profile for a role archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleToolProfile {
    /// The archetype this profile represents.
    pub kind: RoleToolProfileKind,
    /// Optional allowlist of canonical tool names.
    ///
    /// `None` means the profile does not impose an allowlist.
    pub allowed_tools: Option<&'static [&'static str]>,
    /// Canonical tool names that should always be blocked.
    pub denied_tools: &'static [&'static str],
}

impl RoleToolProfile {
    /// Construct a profile with no allowlist and no denylist.
    pub const fn allow_all(kind: RoleToolProfileKind) -> Self {
        Self {
            kind,
            allowed_tools: None,
            denied_tools: &[],
        }
    }

    /// Construct a profile with an explicit allowlist and denylist.
    pub const fn allow_deny(
        kind: RoleToolProfileKind,
        allowed_tools: &'static [&'static str],
        denied_tools: &'static [&'static str],
    ) -> Self {
        Self {
            kind,
            allowed_tools: Some(allowed_tools),
            denied_tools,
        }
    }
}

/// Read-only tools shared by research, review, and planning profiles.
pub const READ_TOOLS: [&str; 5] = [
    read_file::NAME,
    grep::NAME,
    glob::NAME,
    web_search::NAME,
    web_fetch::NAME,
];

/// Comment / note-taking tools for reviewer-style profiles.
pub const COMMENT_TOOLS: [&str; 1] = [todo_write::NAME];

/// Plan-management tools for strategist-style profiles.
pub const PLAN_TOOLS: [&str; 3] = [todo_write::NAME, exit_plan_mode::NAME, task_agent::NAME];

/// Canonical reviewer allowlist.
pub const REVIEWER_TOOLS: [&str; 6] = [
    read_file::NAME,
    grep::NAME,
    glob::NAME,
    web_search::NAME,
    web_fetch::NAME,
    todo_write::NAME,
];

/// Canonical strategist allowlist.
pub const STRATEGIST_TOOLS: [&str; 8] = [
    read_file::NAME,
    grep::NAME,
    glob::NAME,
    web_search::NAME,
    web_fetch::NAME,
    todo_write::NAME,
    exit_plan_mode::NAME,
    task_agent::NAME,
];

/// Tools that mutate code or execute potentially destructive commands.
pub const DESTRUCTIVE_TOOLS: [&str; 7] = [
    write_file::NAME,
    edit_file::NAME,
    multi_edit::NAME,
    apply_patch::NAME,
    notebook_edit::NAME,
    bash::NAME,
    run_tests::NAME,
];

/// Implementer: all tools allowed.
pub const IMPLEMENTER_TOOL_PROFILE: RoleToolProfile =
    RoleToolProfile::allow_all(RoleToolProfileKind::Implementer);

/// Researcher: read-only tools only, with mutation and shell commands blocked.
pub const RESEARCHER_TOOL_PROFILE: RoleToolProfile = RoleToolProfile::allow_deny(
    RoleToolProfileKind::Researcher,
    &READ_TOOLS,
    &[write_file::NAME, edit_file::NAME, bash::NAME],
);

/// Reviewer: read tools plus lightweight comment/note tooling.
pub const REVIEWER_TOOL_PROFILE: RoleToolProfile = RoleToolProfile::allow_deny(
    RoleToolProfileKind::Reviewer,
    &REVIEWER_TOOLS,
    &[write_file::NAME, edit_file::NAME],
);

/// Strategist: read tools plus plan-management tooling, with destructive ops denied.
pub const STRATEGIST_TOOL_PROFILE: RoleToolProfile = RoleToolProfile::allow_deny(
    RoleToolProfileKind::Strategist,
    &STRATEGIST_TOOLS,
    &DESTRUCTIVE_TOOLS,
);

/// All built-in profiles in declaration order.
pub const ROLE_TOOL_PROFILES: [RoleToolProfile; 4] = [
    IMPLEMENTER_TOOL_PROFILE,
    RESEARCHER_TOOL_PROFILE,
    REVIEWER_TOOL_PROFILE,
    STRATEGIST_TOOL_PROFILE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn researcher_profile_is_read_only_plus_blocked_tools() {
        assert_eq!(RESEARCHER_TOOL_PROFILE.allowed_tools, Some(&READ_TOOLS[..]));
        assert_eq!(
            RESEARCHER_TOOL_PROFILE.denied_tools,
            &[write_file::NAME, edit_file::NAME, bash::NAME]
        );
    }

    #[test]
    fn reviewer_profile_allows_read_and_comment_tools_only() {
        assert_eq!(
            REVIEWER_TOOL_PROFILE.allowed_tools,
            Some(&REVIEWER_TOOLS[..])
        );
        assert_eq!(
            REVIEWER_TOOL_PROFILE.denied_tools,
            &[write_file::NAME, edit_file::NAME]
        );
    }

    #[test]
    fn strategist_profile_allows_read_and_plan_tools_only() {
        assert_eq!(
            STRATEGIST_TOOL_PROFILE.allowed_tools,
            Some(&STRATEGIST_TOOLS[..])
        );
        assert_eq!(STRATEGIST_TOOL_PROFILE.denied_tools, &DESTRUCTIVE_TOOLS);
    }

    #[test]
    fn profiles_have_unique_kinds() {
        let mut seen = HashSet::new();
        for profile in ROLE_TOOL_PROFILES {
            assert!(seen.insert(profile.kind));
        }
        assert_eq!(seen.len(), ROLE_TOOL_PROFILES.len());
    }
}
