//! Registry mapping canonical tool names to [`ToolHandler`] instances.
//!
//! [`StaticToolRegistry`](super::StaticToolRegistry) describes **what**
//! the built-in tools look like; [`HandlerRegistry`] tells the
//! dispatcher **how** to execute them. All 16 canonical built-ins
//! (§36.b) are shipped, though a few are day-one stubs that return a
//! structured "not yet wired" error until their downstream
//! dependencies (HTTP client, sub-agent supervisor) land.

use std::sync::Arc;

use roko_core::tool::ToolHandler;

use super::builtin::{
    apply_patch, bash, edit_file, exit_plan_mode, glob, grep, ls, multi_edit, notebook_edit,
    read_file, run_tests, task_agent, todo_write, web_fetch, web_search, write_file,
};

/// Look up the concrete handler for a canonical tool `name`.
///
/// Every built-in from [`BUILTIN_TOOL_NAMES`](super::builtin::BUILTIN_TOOL_NAMES)
/// resolves to a handler; a few are day-one stubs that fail with
/// [`roko_core::tool::ToolError::Other`] describing the missing dep.
/// Returns `None` only for genuinely unknown names.
#[must_use]
pub fn handler_for(name: &str) -> Option<Arc<dyn ToolHandler>> {
    match name {
        n if n == read_file::NAME => Some(Arc::new(read_file::Handler)),
        n if n == write_file::NAME => Some(Arc::new(write_file::Handler)),
        n if n == edit_file::NAME => Some(Arc::new(edit_file::Handler)),
        n if n == multi_edit::NAME => Some(Arc::new(multi_edit::Handler)),
        n if n == glob::NAME => Some(Arc::new(glob::Handler)),
        n if n == ls::NAME => Some(Arc::new(ls::Handler)),
        n if n == grep::NAME => Some(Arc::new(grep::Handler)),
        n if n == bash::NAME => Some(Arc::new(bash::Handler)),
        n if n == run_tests::NAME => Some(Arc::new(run_tests::Handler)),
        n if n == apply_patch::NAME => Some(Arc::new(apply_patch::Handler)),
        n if n == notebook_edit::NAME => Some(Arc::new(notebook_edit::Handler)),
        n if n == todo_write::NAME => Some(Arc::new(todo_write::Handler)),
        n if n == exit_plan_mode::NAME => Some(Arc::new(exit_plan_mode::Handler)),
        n if n == web_fetch::NAME => Some(Arc::new(web_fetch::Handler)),
        n if n == web_search::NAME => Some(Arc::new(web_search::Handler)),
        n if n == task_agent::NAME => Some(Arc::new(task_agent::Handler)),
        _ => None,
    }
}

/// Thin wrapper around [`handler_for`] for trait-object dispatch sites.
#[derive(Debug, Clone, Copy, Default)]
pub struct HandlerRegistry;

impl HandlerRegistry {
    /// Construct a new registry (zero-sized).
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Look up a handler by canonical name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        handler_for(name)
    }

    /// Canonical names of every shipped handler (all 16 built-ins).
    #[must_use]
    pub const fn shipped_names(&self) -> &'static [&'static str] {
        const NAMES: &[&str] = &[
            read_file::NAME,
            write_file::NAME,
            edit_file::NAME,
            multi_edit::NAME,
            glob::NAME,
            grep::NAME,
            ls::NAME,
            bash::NAME,
            run_tests::NAME,
            apply_patch::NAME,
            notebook_edit::NAME,
            todo_write::NAME,
            exit_plan_mode::NAME,
            web_fetch::NAME,
            web_search::NAME,
            task_agent::NAME,
        ];
        NAMES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::builtin::BUILTIN_TOOL_NAMES;

    #[test]
    fn shipped_handlers_are_findable() {
        let reg = HandlerRegistry::new();
        for name in reg.shipped_names() {
            assert!(reg.get(name).is_some(), "expected handler for {name}");
        }
    }

    #[test]
    fn unknown_tool_returns_none() {
        let reg = HandlerRegistry::new();
        assert!(reg.get("not_a_real_tool").is_none());
        assert!(reg.get("").is_none());
    }

    #[test]
    fn handler_names_match_tool_defs() {
        let reg = HandlerRegistry::new();
        for name in reg.shipped_names() {
            let handler = reg.get(name).expect("handler present");
            assert_eq!(handler.name(), *name);
        }
    }

    #[test]
    fn every_std_builtin_tool_has_a_handler() {
        let reg = HandlerRegistry::new();
        // Only the 16 std handlers ship in HandlerRegistry; chain tool
        // handlers ship from roko-cli.
        for name in reg.shipped_names() {
            assert!(
                reg.get(name).is_some(),
                "no handler registered for shipped tool `{name}`"
            );
        }
    }

    #[test]
    fn shipped_names_are_subset_of_builtin_names() {
        use std::collections::HashSet;
        let reg = HandlerRegistry::new();
        let shipped: HashSet<&str> = reg.shipped_names().iter().copied().collect();
        let builtin: HashSet<&str> = BUILTIN_TOOL_NAMES.iter().copied().collect();
        assert!(
            shipped.is_subset(&builtin),
            "shipped_names must be a subset of BUILTIN_TOOL_NAMES"
        );
    }

    #[test]
    fn all_16_std_builtins_ship_handlers() {
        assert_eq!(HandlerRegistry::new().shipped_names().len(), 16);
    }
}
