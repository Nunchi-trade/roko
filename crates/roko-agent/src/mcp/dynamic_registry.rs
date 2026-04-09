//! Dynamic tool registry composing static + MCP tools (SS36.62).
//!
//! [`DynamicToolRegistry`] wraps a static base registry (any
//! [`ToolRegistry`] implementation) and layers dynamically-discovered
//! MCP tools on top. It implements [`ToolRegistry`] itself so callers
//! need not distinguish between static and dynamic registries.

use roko_core::tool::{ToolDef, ToolRegistry};
use std::collections::{HashMap, HashSet};
use tracing::warn;

/// A registry that combines a static base set of tools with
/// dynamically-added MCP server tools.
///
/// Tool lookup (`get`, `all`) searches both sets. MCP tools from
/// different servers are namespaced by their server prefix (see
/// [`super::to_tool_def::mcp_to_tool_def`]).
pub struct DynamicToolRegistry {
    /// The base (static/built-in) tools.
    base: Vec<ToolDef>,
    /// MCP tools keyed by server name.
    mcp_servers: HashMap<String, Vec<ToolDef>>,
    /// If true, prefer MCP tools over built-ins when names collide.
    prefer_mcp: bool,
    /// Flattened view of base + all MCP tools, rebuilt on mutation.
    all_tools: Vec<ToolDef>,
}

impl DynamicToolRegistry {
    /// Create a new registry backed by the given base tools.
    ///
    /// Accepts any [`ToolRegistry`] -- copies its tools into the base
    /// set so the dynamic registry owns all data.
    pub fn new(base: &dyn ToolRegistry) -> Self {
        Self::with_preference(base, false)
    }

    /// Create a new registry with explicit collision preference.
    #[must_use]
    pub fn with_preference(base: &dyn ToolRegistry, prefer_mcp: bool) -> Self {
        let base_tools: Vec<ToolDef> = base.all().to_vec();
        let all_tools = base_tools.clone();
        Self {
            base: base_tools,
            mcp_servers: HashMap::new(),
            prefer_mcp,
            all_tools,
        }
    }

    /// Create an empty registry with no base tools.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            base: Vec::new(),
            mcp_servers: HashMap::new(),
            prefer_mcp: false,
            all_tools: Vec::new(),
        }
    }

    /// Add tools discovered from an MCP server.
    ///
    /// If tools from the same server were previously added, they are
    /// replaced.
    pub fn add_mcp_tools(&mut self, server: &str, tools: Vec<ToolDef>) {
        self.mcp_servers.insert(server.to_string(), tools);
        self.rebuild();
    }

    /// Remove all tools from a specific MCP server.
    ///
    /// Returns `true` if the server was present.
    pub fn remove_server(&mut self, name: &str) -> bool {
        let removed = self.mcp_servers.remove(name).is_some();
        if removed {
            self.rebuild();
        }
        removed
    }

    /// Number of MCP servers currently registered.
    #[must_use]
    pub fn server_count(&self) -> usize {
        self.mcp_servers.len()
    }

    /// Rebuild the flattened `all_tools` vector after a mutation.
    fn rebuild(&mut self) {
        let mut all_tools = Vec::with_capacity(
            self.base.len()
                + self
                    .mcp_servers
                    .values()
                    .map(std::vec::Vec::len)
                    .sum::<usize>(),
        );
        let mut seen = HashSet::new();

        if self.prefer_mcp {
            for tools in self.mcp_servers.values() {
                for tool in tools {
                    if seen.insert(tool.name.clone()) {
                        all_tools.push(tool.clone());
                    }
                }
            }
            for tool in &self.base {
                if seen.contains(&tool.name) {
                    warn!(
                        "builtin tool '{}' is shadowed by an MCP tool; prefer_mcp=true keeps the MCP version",
                        tool.name
                    );
                } else {
                    seen.insert(tool.name.clone());
                    all_tools.push(tool.clone());
                }
            }
        } else {
            for tool in &self.base {
                seen.insert(tool.name.clone());
                all_tools.push(tool.clone());
            }
            for tools in self.mcp_servers.values() {
                for tool in tools {
                    if seen.contains(&tool.name) {
                        warn!(
                            "MCP tool '{}' duplicates a builtin tool; keeping the builtin version",
                            tool.name
                        );
                    } else {
                        seen.insert(tool.name.clone());
                        all_tools.push(tool.clone());
                    }
                }
            }
        }
        self.all_tools = all_tools;
    }
}

impl ToolRegistry for DynamicToolRegistry {
    fn get(&self, name: &str) -> Option<&ToolDef> {
        self.all_tools.iter().find(|t| t.name == name)
    }

    fn all(&self) -> &[ToolDef] {
        &self.all_tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolPermission, VecToolRegistry};

    fn base_tool(name: &str) -> ToolDef {
        ToolDef::new(
            name,
            "base tool",
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
    }

    fn mcp_tool(name: &str) -> ToolDef {
        ToolDef::new(
            name,
            "mcp tool",
            ToolCategory::Mcp,
            ToolPermission::read_only(),
        )
    }

    #[test]
    fn mcp_dynamic_registry_base_only() {
        let base = VecToolRegistry::from_tools(vec![base_tool("read_file"), base_tool("grep")]);
        let reg = DynamicToolRegistry::new(&base);
        assert_eq!(reg.all().len(), 2);
        assert!(reg.get("read_file").is_some());
        assert!(reg.get("grep").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn mcp_dynamic_registry_add_mcp_tools() {
        let base = VecToolRegistry::from_tools(vec![base_tool("read_file")]);
        let mut reg = DynamicToolRegistry::new(&base);
        reg.add_mcp_tools("fs", vec![mcp_tool("fs__list"), mcp_tool("fs__stat")]);
        assert_eq!(reg.all().len(), 3);
        assert!(reg.get("read_file").is_some());
        assert!(reg.get("fs__list").is_some());
        assert!(reg.get("fs__stat").is_some());
        assert_eq!(reg.server_count(), 1);
    }

    #[test]
    fn mcp_dynamic_registry_remove_server() {
        let base = VecToolRegistry::from_tools(vec![base_tool("read_file")]);
        let mut reg = DynamicToolRegistry::new(&base);
        reg.add_mcp_tools("fs", vec![mcp_tool("fs__list")]);
        assert_eq!(reg.all().len(), 2);

        let removed = reg.remove_server("fs");
        assert!(removed);
        assert_eq!(reg.all().len(), 1);
        assert!(reg.get("fs__list").is_none());
        assert!(reg.get("read_file").is_some());
        assert_eq!(reg.server_count(), 0);
    }

    #[test]
    fn mcp_dynamic_registry_remove_nonexistent_server() {
        let base = VecToolRegistry::from_tools(vec![base_tool("read_file")]);
        let mut reg = DynamicToolRegistry::new(&base);
        let removed = reg.remove_server("does_not_exist");
        assert!(!removed);
        assert_eq!(reg.all().len(), 1);
    }

    #[test]
    fn mcp_dynamic_registry_replace_server_tools() {
        let base = VecToolRegistry::from_tools(vec![base_tool("read_file")]);
        let mut reg = DynamicToolRegistry::new(&base);
        reg.add_mcp_tools("fs", vec![mcp_tool("fs__v1")]);
        assert_eq!(reg.all().len(), 2);

        // Replace with different tools.
        reg.add_mcp_tools("fs", vec![mcp_tool("fs__v2"), mcp_tool("fs__v3")]);
        assert_eq!(reg.all().len(), 3);
        assert!(reg.get("fs__v1").is_none());
        assert!(reg.get("fs__v2").is_some());
        assert!(reg.get("fs__v3").is_some());
    }

    #[test]
    fn mcp_dynamic_registry_multiple_servers() {
        let base = VecToolRegistry::from_tools(vec![base_tool("read_file")]);
        let mut reg = DynamicToolRegistry::new(&base);
        reg.add_mcp_tools("fs", vec![mcp_tool("fs__list")]);
        reg.add_mcp_tools("git", vec![mcp_tool("git__status")]);
        assert_eq!(reg.all().len(), 3);
        assert_eq!(reg.server_count(), 2);
        assert!(reg.get("fs__list").is_some());
        assert!(reg.get("git__status").is_some());

        reg.remove_server("fs");
        assert_eq!(reg.all().len(), 2);
        assert!(reg.get("fs__list").is_none());
        assert!(reg.get("git__status").is_some());
    }

    #[test]
    fn mcp_dynamic_registry_empty() {
        let reg = DynamicToolRegistry::empty();
        assert!(reg.all().is_empty());
        assert!(reg.get("anything").is_none());
    }

    #[test]
    fn mcp_dynamic_registry_validate_args_unknown_tool_errs() {
        let reg = DynamicToolRegistry::empty();
        let result = reg.validate_args("nope", &serde_json::json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn mcp_dynamic_registry_validate_args_known_tool_ok() {
        let base = VecToolRegistry::from_tools(vec![base_tool("read_file")]);
        let reg = DynamicToolRegistry::new(&base);
        let result = reg.validate_args("read_file", &serde_json::json!({}));
        assert!(result.is_ok());
    }

    #[test]
    fn mcp_dynamic_registry_prefers_builtin_by_default() {
        let base = VecToolRegistry::from_tools(vec![base_tool("read_file")]);
        let mut reg = DynamicToolRegistry::new(&base);
        reg.add_mcp_tools("fs", vec![mcp_tool("read_file"), mcp_tool("fs__list")]);

        assert_eq!(reg.all().len(), 2);
        let def = reg.get("read_file").expect("builtin tool should win");
        assert_eq!(def.category, ToolCategory::Read);
    }

    #[test]
    fn mcp_dynamic_registry_can_prefer_mcp_tools() {
        let base = VecToolRegistry::from_tools(vec![base_tool("read_file")]);
        let mut reg = DynamicToolRegistry::with_preference(&base, true);
        reg.add_mcp_tools("fs", vec![mcp_tool("read_file"), mcp_tool("fs__list")]);

        assert_eq!(reg.all().len(), 2);
        let def = reg.get("read_file").expect("mcp tool should win");
        assert_eq!(def.category, ToolCategory::Mcp);
    }
}
