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

/// Execution-only tools denied to the Scribe role.
///
/// Scribes can write documentation files but cannot execute shell commands
/// or run tests — they are limited to read + write tools.
pub const EXEC_TOOLS: [&str; 2] = [bash::NAME, run_tests::NAME];

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
    /// Documentation role. Can read and write files but cannot execute commands.
    Scribe,
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

/// Canonical scribe allowlist — read tools plus write tools (no exec).
pub const SCRIBE_TOOLS: [&str; 10] = [
    read_file::NAME,
    grep::NAME,
    glob::NAME,
    web_search::NAME,
    web_fetch::NAME,
    write_file::NAME,
    edit_file::NAME,
    multi_edit::NAME,
    apply_patch::NAME,
    todo_write::NAME,
];

/// Scribe: read + write tools, but no shell execution.
pub const SCRIBE_TOOL_PROFILE: RoleToolProfile = RoleToolProfile::allow_deny(
    RoleToolProfileKind::Scribe,
    &SCRIBE_TOOLS,
    &EXEC_TOOLS,
);

/// All built-in profiles in declaration order.
pub const ROLE_TOOL_PROFILES: [RoleToolProfile; 5] = [
    IMPLEMENTER_TOOL_PROFILE,
    RESEARCHER_TOOL_PROFILE,
    REVIEWER_TOOL_PROFILE,
    STRATEGIST_TOOL_PROFILE,
    SCRIBE_TOOL_PROFILE,
];

/// Look up role-based denied tools by role label string.
///
/// Maps the kebab-case role label (as it appears in `tasks.toml`) to the
/// canonical [`RoleToolProfile`] denied-tool list. Returns `None` for roles
/// that have no profile-level denied tools (e.g. `"implementer"`) or for
/// unknown role strings.
///
/// This is the single authoritative mapping from role strings to denied
/// tools, consumed by the task parser's `apply_role_tool_defaults`.
pub fn denied_tools_for_role(role: &str) -> Option<&'static [&'static str]> {
    let profile = match role.to_ascii_lowercase().as_str() {
        "researcher" => &RESEARCHER_TOOL_PROFILE,
        "reviewer" | "auditor" | "quick-reviewer" | "critic" => &REVIEWER_TOOL_PROFILE,
        "strategist" | "architect" | "pre-planner" => &STRATEGIST_TOOL_PROFILE,
        "scribe" | "doc-verifier" => &SCRIBE_TOOL_PROFILE,
        // Implementer, auto-fixer, refactorer, etc. — full access, no denials.
        _ => return None,
    };
    if profile.denied_tools.is_empty() {
        None
    } else {
        Some(profile.denied_tools)
    }
}

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
    fn scribe_profile_allows_read_and_write_but_not_exec() {
        assert_eq!(
            SCRIBE_TOOL_PROFILE.allowed_tools,
            Some(&SCRIBE_TOOLS[..])
        );
        assert_eq!(SCRIBE_TOOL_PROFILE.denied_tools, &EXEC_TOOLS);
        // Scribe can write files...
        let allowed: HashSet<&str> = SCRIBE_TOOLS.iter().copied().collect();
        assert!(allowed.contains(write_file::NAME));
        assert!(allowed.contains(edit_file::NAME));
        // ...but cannot execute commands.
        let denied: HashSet<&str> = EXEC_TOOLS.iter().copied().collect();
        assert!(denied.contains(bash::NAME));
        assert!(denied.contains(run_tests::NAME));
    }

    #[test]
    fn profiles_have_unique_kinds() {
        let mut seen = HashSet::new();
        for profile in ROLE_TOOL_PROFILES {
            assert!(seen.insert(profile.kind));
        }
        assert_eq!(seen.len(), ROLE_TOOL_PROFILES.len());
    }

    #[test]
    fn denied_tools_for_role_returns_correct_profiles() {
        // Researcher denies write + edit + bash.
        let denied = denied_tools_for_role("researcher").unwrap();
        assert!(denied.contains(&write_file::NAME));
        assert!(denied.contains(&edit_file::NAME));
        assert!(denied.contains(&bash::NAME));

        // Reviewer/auditor deny write + edit.
        let denied = denied_tools_for_role("reviewer").unwrap();
        assert!(denied.contains(&write_file::NAME));
        assert!(denied.contains(&edit_file::NAME));
        assert!(!denied.contains(&bash::NAME));

        let denied = denied_tools_for_role("auditor").unwrap();
        assert_eq!(denied, denied_tools_for_role("reviewer").unwrap());

        // Strategist denies all destructive tools.
        let denied = denied_tools_for_role("strategist").unwrap();
        assert_eq!(denied, &DESTRUCTIVE_TOOLS[..]);

        // Scribe denies exec tools.
        let denied = denied_tools_for_role("scribe").unwrap();
        assert!(denied.contains(&bash::NAME));
        assert!(denied.contains(&run_tests::NAME));
        assert!(!denied.contains(&write_file::NAME));

        // Implementer has no denials.
        assert!(denied_tools_for_role("implementer").is_none());
        // Unknown role has no denials.
        assert!(denied_tools_for_role("unknown-role").is_none());
    }

    #[test]
    fn denied_tools_for_role_is_case_insensitive() {
        assert_eq!(
            denied_tools_for_role("Researcher"),
            denied_tools_for_role("researcher")
        );
        assert_eq!(
            denied_tools_for_role("SCRIBE"),
            denied_tools_for_role("scribe")
        );
    }
}
