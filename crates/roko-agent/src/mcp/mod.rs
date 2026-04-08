//! MCP (Model Context Protocol) integration (SS36.58-36.62).
//!
//! Provides a JSON-RPC stdio client for MCP servers, tool conversion from
//! MCP definitions to [`roko_core::tool::ToolDef`], multi-server dedup,
//! `.mcp.json` config discovery, and a dynamic registry that composes
//! static built-in tools with MCP-discovered tools.

pub mod client;
pub mod config;
pub mod dedup;
pub mod dynamic_registry;
pub mod to_tool_def;

pub use client::{McpClient, McpRequest, McpResponse, McpToolDef, McpToolResult, StdioTransport, Transport};
pub use config::{McpConfig, McpServerConfig, find_mcp_config};
pub use dedup::dedup_tools;
pub use dynamic_registry::DynamicToolRegistry;
pub use to_tool_def::mcp_to_tool_def;
