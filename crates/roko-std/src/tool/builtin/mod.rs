//! Built-in tool definitions (§36.b) plus chain-domain tools.
//!
//! Each `builtin/<name>.rs` module exposes:
//!
//! - `pub const NAME: &str` — canonical `snake_case` name
//! - `pub const DESCRIPTION: &str` — LLM-facing help text
//! - `pub fn tool_def() -> ToolDef` — constructs the full definition
//!
//! [`ROKO_BUILTIN_TOOLS`] exposes the 16 std tools plus 14 chain-domain
//! tools as a single `Vec`, materialized once via [`std::sync::LazyLock`]
//! on first access.
//!
//! Std tool order follows `roko_core::tool::aliases::ALIASES`:
//! `read_file` → `write_file` → `edit_file` → `multi_edit` → `glob` →
//! `grep` → `bash` → `ls` → `web_fetch` → `web_search` →
//! `notebook_edit` → `todo_write` → `task` (from `task_agent`) →
//! `exit_plan_mode` → `apply_patch` → `run_tests`.

use std::sync::LazyLock;

use roko_chain::tools::{CHAIN_DOMAIN_TOOLS, CHAIN_TOOL_NAMES};
use roko_core::tool::ToolDef;

pub mod apply_patch;
pub mod bash;
pub mod edit_file;
pub mod exit_plan_mode;
pub mod glob;
pub mod grep;
pub mod ls;
pub mod multi_edit;
pub mod notebook_edit;
pub mod read_file;
pub mod run_tests;
pub mod sandbox;
pub mod task_agent;
pub mod todo_write;
pub mod web_fetch;
pub mod web_search;
pub mod write_file;

/// Number of std built-in tools shipped in §36.b (excludes chain tools).
pub const TOOL_COUNT: usize = 30;

/// All built-in tool definitions: 16 std tools + 14 chain-domain tools.
///
/// Materialized on first access via [`std::sync::LazyLock`]; every
/// subsequent read is lock-free.
pub static ROKO_BUILTIN_TOOLS: LazyLock<Vec<ToolDef>> = LazyLock::new(|| {
    let mut tools = vec![
        read_file::tool_def(),
        write_file::tool_def(),
        edit_file::tool_def(),
        multi_edit::tool_def(),
        glob::tool_def(),
        grep::tool_def(),
        bash::tool_def(),
        ls::tool_def(),
        web_fetch::tool_def(),
        web_search::tool_def(),
        notebook_edit::tool_def(),
        todo_write::tool_def(),
        task_agent::tool_def(),
        exit_plan_mode::tool_def(),
        apply_patch::tool_def(),
        run_tests::tool_def(),
    ];
    tools.extend(CHAIN_DOMAIN_TOOLS.iter().cloned());
    tools
});

/// Canonical names of all built-in tools, in [`ROKO_BUILTIN_TOOLS`] order.
pub static BUILTIN_TOOL_NAMES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut names = vec![
        read_file::NAME,
        write_file::NAME,
        edit_file::NAME,
        multi_edit::NAME,
        glob::NAME,
        grep::NAME,
        bash::NAME,
        ls::NAME,
        web_fetch::NAME,
        web_search::NAME,
        notebook_edit::NAME,
        todo_write::NAME,
        task_agent::NAME,
        exit_plan_mode::NAME,
        apply_patch::NAME,
        run_tests::NAME,
    ];
    names.extend_from_slice(&CHAIN_TOOL_NAMES);
    names
});
