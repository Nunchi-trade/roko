//! Multi-server tool deduplication (SS36.60).
//!
//! When multiple MCP servers are active, their tool lists may overlap.
//! [`dedup_tools`] merges them using a prefix strategy (`server__tool`)
//! and a last-writer-wins conflict policy.

use roko_core::tool::ToolDef;
use std::collections::HashMap;

/// Merge tool lists from multiple MCP servers into a single deduplicated
/// list.
///
/// Each entry in `tools` is `(server_name, tool_defs)`. Tools are
/// expected to already be prefixed with `server_name__` (by
/// [`super::to_tool_def::mcp_to_tool_def`]).
///
/// When two servers provide a tool with the same canonical name (after
/// prefixing), the **last** server in the input order wins — its
/// definition replaces the earlier one.
#[must_use]
pub fn dedup_tools(tools: Vec<(String, Vec<ToolDef>)>) -> Vec<ToolDef> {
    // Insertion-order map: name -> (insertion_index, def).
    let mut seen: HashMap<String, (usize, ToolDef)> = HashMap::new();
    let mut counter: usize = 0;

    for (_server, defs) in tools {
        for def in defs {
            let idx = seen.get(&def.name).map_or_else(
                || {
                    let i = counter;
                    counter += 1;
                    i
                },
                |&(existing_idx, _)| existing_idx,
            );
            seen.insert(def.name.clone(), (idx, def));
        }
    }

    let mut entries: Vec<(usize, ToolDef)> = seen.into_values().collect();
    entries.sort_by_key(|(idx, _)| *idx);
    entries.into_iter().map(|(_, def)| def).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolPermission};

    fn tool(name: &str, desc: &str) -> ToolDef {
        ToolDef::new(name, desc, ToolCategory::Mcp, ToolPermission::read_only())
    }

    #[test]
    fn mcp_dedup_empty_input() {
        let result = dedup_tools(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn mcp_dedup_single_server() {
        let tools = vec![(
            "fs".to_string(),
            vec![tool("fs__read", "read"), tool("fs__write", "write")],
        )];
        let result = dedup_tools(tools);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "fs__read");
        assert_eq!(result[1].name, "fs__write");
    }

    #[test]
    fn mcp_dedup_multiple_servers_no_overlap() {
        let tools = vec![
            ("fs".to_string(), vec![tool("fs__read", "read")]),
            ("git".to_string(), vec![tool("git__status", "status")]),
        ];
        let result = dedup_tools(tools);
        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"fs__read"));
        assert!(names.contains(&"git__status"));
    }

    #[test]
    fn mcp_dedup_last_writer_wins() {
        let tools = vec![
            (
                "server_a".to_string(),
                vec![tool("shared__search", "search v1")],
            ),
            (
                "server_b".to_string(),
                vec![tool("shared__search", "search v2")],
            ),
        ];
        let result = dedup_tools(tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].description, "search v2");
    }

    #[test]
    fn mcp_dedup_preserves_insertion_order() {
        let tools = vec![
            ("a".to_string(), vec![tool("a__x", "x"), tool("a__y", "y")]),
            ("b".to_string(), vec![tool("b__z", "z"), tool("b__w", "w")]),
        ];
        let result = dedup_tools(tools);
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["a__x", "a__y", "b__z", "b__w"]);
    }

    #[test]
    fn mcp_dedup_mixed_overlap_and_unique() {
        let tools = vec![
            (
                "a".to_string(),
                vec![tool("shared__read", "v1"), tool("a__only", "only a")],
            ),
            (
                "b".to_string(),
                vec![tool("shared__read", "v2"), tool("b__only", "only b")],
            ),
        ];
        let result = dedup_tools(tools);
        assert_eq!(result.len(), 3);
        // shared__read should be the v2 version (last writer wins), in original position
        assert_eq!(result[0].name, "shared__read");
        assert_eq!(result[0].description, "v2");
        assert_eq!(result[1].name, "a__only");
        assert_eq!(result[2].name, "b__only");
    }
}
