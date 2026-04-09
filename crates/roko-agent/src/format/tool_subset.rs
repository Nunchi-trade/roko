//! Tool subset selection respecting `max_tools_per_call` from the model
//! profile (checklist item 36.79).
//!
//! When the number of available tools exceeds the model's degradation
//! threshold ([`ToolFormatProfile::max_tools_before_degrade`]), this
//! module selects the top-N most relevant tools using a
//! [`ToolRelevanceScorer`]. If no task context is provided, tools are
//! returned in their original order, truncated to the limit.

use roko_core::tool::def::ToolDef;
use roko_core::tool::format::ToolFormatProfile;
use roko_core::tool::relevance::{KeywordOverlapScorer, ToolRelevanceScorer};

/// Default tool-count cap when the profile has no explicit limit.
const DEFAULT_MAX_TOOLS: u8 = 32;

/// Select which tools to include in a single LLM call.
///
/// If the number of `available` tools is within the profile's
/// `max_tools_before_degrade` threshold, all tools are returned.
/// Otherwise the set is pruned to the threshold count using relevance
/// scoring against `task_context`.
///
/// When `task_context` is `None` or empty, tools are kept in input order
/// (truncated to the limit). When present, the built-in keyword-overlap
/// scorer ranks tools by relevance and the top-N are returned.
#[must_use]
pub fn select_tools_for_call(
    available: &[ToolDef],
    profile: &ToolFormatProfile,
    task_context: Option<&str>,
) -> Vec<ToolDef> {
    let max = profile.max_tools_before_degrade.max(1);
    let limit = if max == 0 { DEFAULT_MAX_TOOLS } else { max } as usize;

    if available.len() <= limit {
        return available.to_vec();
    }

    match task_context {
        Some(ctx) if !ctx.trim().is_empty() => {
            let scorer = KeywordOverlapScorer;
            let ranked = scorer.top_n(ctx, available, limit);
            ranked.into_iter().cloned().collect()
        }
        _ => {
            // No context: take the first `limit` tools in input order.
            available[..limit].to_vec()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::def::{ToolCategory, ToolPermission};
    use roko_core::tool::format::{ToolFormat, ToolFormatProfile};

    fn make_profile(max_tools: u8) -> ToolFormatProfile {
        ToolFormatProfile {
            preferred: ToolFormat::OpenAiJson,
            fallback_chain: vec![],
            supports_tools: true,
            parallel_safe: true,
            max_tools_before_degrade: max_tools,
            needs_stream_disabled: false,
            tool_call_id_len: None,
            demotion_after_failures: 3,
        }
    }

    fn def(name: &str, desc: &str) -> ToolDef {
        ToolDef::new(name, desc, ToolCategory::Read, ToolPermission::read_only())
    }

    fn sample_tools() -> Vec<ToolDef> {
        vec![
            def(
                "read_file",
                "Read a UTF-8 text file with optional line range",
            ),
            def("write_file", "Write a text file, creating or overwriting"),
            def("edit_file", "Replace an exact string in a file"),
            def("grep", "Search file contents using regex pattern matching"),
            def("glob", "Find files matching a glob pattern in a directory"),
            def("bash", "Execute a shell command and capture stdout/stderr"),
            def("web_fetch", "Fetch a URL over HTTPS and return the body"),
            def("web_search", "Search the web for a query string"),
            def("run_tests", "Run the test suite for a project"),
            def("apply_patch", "Apply a unified diff patch to files"),
        ]
    }

    #[test]
    fn tool_subset_all_returned_under_limit() {
        let tools = sample_tools();
        let profile = make_profile(32);
        let result = select_tools_for_call(&tools, &profile, None);
        assert_eq!(result.len(), tools.len());
    }

    #[test]
    fn tool_subset_truncated_without_context() {
        let tools = sample_tools();
        let profile = make_profile(5);
        let result = select_tools_for_call(&tools, &profile, None);
        assert_eq!(result.len(), 5);
        // Should be the first 5 in input order.
        assert_eq!(result[0].name, "read_file");
        assert_eq!(result[4].name, "glob");
    }

    #[test]
    fn tool_subset_relevance_scored_with_context() {
        let tools = sample_tools();
        let profile = make_profile(3);
        let result = select_tools_for_call(
            &tools,
            &profile,
            Some("search for a regex pattern in files"),
        );
        assert_eq!(result.len(), 3);
        // grep should be ranked highly given the task context.
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"grep"),
            "grep should be in the top 3 for 'search for a regex pattern in files', got {names:?}"
        );
    }

    #[test]
    fn tool_subset_empty_context_falls_back_to_truncation() {
        let tools = sample_tools();
        let profile = make_profile(4);
        let result = select_tools_for_call(&tools, &profile, Some(""));
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].name, "read_file");
    }

    #[test]
    fn tool_subset_empty_tools_returns_empty() {
        let profile = make_profile(5);
        let result = select_tools_for_call(&[], &profile, Some("do something"));
        assert!(result.is_empty());
    }

    #[test]
    fn tool_subset_exact_limit_returns_all() {
        let tools = sample_tools();
        let profile = make_profile(tools.len() as u8);
        let result = select_tools_for_call(&tools, &profile, Some("anything"));
        assert_eq!(result.len(), tools.len());
    }

    #[test]
    fn tool_subset_limit_one() {
        let tools = sample_tools();
        let profile = make_profile(1);
        let result = select_tools_for_call(&tools, &profile, Some("execute a shell command"));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn tool_subset_none_context_truncates() {
        let tools = sample_tools();
        let profile = make_profile(3);
        let result = select_tools_for_call(&tools, &profile, None);
        assert_eq!(result.len(), 3);
        // First 3 in input order.
        assert_eq!(result[0].name, "read_file");
        assert_eq!(result[1].name, "write_file");
        assert_eq!(result[2].name, "edit_file");
    }

    #[test]
    fn tool_subset_web_fetch_ranked_for_network_task() {
        let tools = sample_tools();
        let profile = make_profile(2);
        let result = select_tools_for_call(
            &tools,
            &profile,
            Some("fetch data from a URL on the web over HTTPS"),
        );
        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"web_fetch"),
            "web_fetch should be ranked highly for network tasks, got {names:?}"
        );
    }

    #[test]
    fn tool_subset_preserves_tool_def_fields() {
        let tools = vec![
            ToolDef::new(
                "custom_tool",
                "A custom tool with specific settings",
                ToolCategory::Exec,
                ToolPermission::executes(),
            )
            .with_timeout_ms(120_000)
            .with_idempotent(true),
        ];
        let profile = make_profile(5);
        let result = select_tools_for_call(&tools, &profile, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].timeout_ms, 120_000);
        assert!(result[0].idempotent);
    }
}
