//! MCP discovery bridge for HTTP-backed tool loops.
//!
//! Claude CLI forwards MCP config directly to the subprocess via
//! `--mcp-config`. HTTP backends cannot do that, so they must discover MCP
//! tools up front, convert them into canonical [`ToolDef`] values, and let the
//! normal translator render them into backend-specific function definitions.

use roko_core::tool::ToolDef;
use tokio::time::{Duration, timeout};

use super::{
    McpClient, McpConfig, McpTransportConfig, StdioTransport, dedup_tools, mcp_to_tool_def,
};
use crate::mcp::client::McpError;

const MCP_DISCOVERY_TIMEOUT: Duration =
    Duration::from_secs(roko_core::defaults::DEFAULT_MCP_DISCOVERY_TIMEOUT_SECS);

/// Errors raised while discovering MCP tools for HTTP backends.
#[derive(Debug, thiserror::Error)]
pub enum McpBridgeError {
    #[error("failed to spawn MCP server '{server}': {source}")]
    Spawn { server: String, source: McpError },
    #[error("MCP server '{server}' uses unsupported transport '{transport}'")]
    UnsupportedTransport { server: String, transport: String },
    #[error("MCP server '{server}' initialize timed out after {timeout_secs}s")]
    InitializeTimeout { server: String, timeout_secs: u64 },
    #[error("MCP server '{server}' initialize failed: {source}")]
    Initialize { server: String, source: McpError },
    #[error("MCP server '{server}' tools/list timed out after {timeout_secs}s")]
    ListToolsTimeout { server: String, timeout_secs: u64 },
    #[error("MCP server '{server}' tools/list failed: {source}")]
    ListTools { server: String, source: McpError },
}

/// Discover and convert MCP tools so HTTP backends can expose them as normal
/// function definitions.
pub async fn discover_mcp_tools(config: &McpConfig) -> Result<Vec<ToolDef>, McpBridgeError> {
    let mut all_server_tools = Vec::new();

    for server in &config.servers {
        if server.transport != McpTransportConfig::Stdio {
            return Err(McpBridgeError::UnsupportedTransport {
                server: server.name.clone(),
                transport: format!("{:?}", server.transport).to_ascii_lowercase(),
            });
        }

        let transport = StdioTransport::spawn_with_env(&server.command, &server.args, &server.env)
            .map_err(|source| McpBridgeError::Spawn {
                server: server.name.clone(),
                source,
            })?;
        let client = McpClient::new(transport);

        match timeout(MCP_DISCOVERY_TIMEOUT, client.initialize()).await {
            Ok(Ok(_)) => {}
            Ok(Err(source)) => {
                return Err(McpBridgeError::Initialize {
                    server: server.name.clone(),
                    source,
                });
            }
            Err(_) => {
                return Err(McpBridgeError::InitializeTimeout {
                    server: server.name.clone(),
                    timeout_secs: MCP_DISCOVERY_TIMEOUT.as_secs(),
                });
            }
        }

        let mcp_tools = match timeout(MCP_DISCOVERY_TIMEOUT, client.list_tools()).await {
            Ok(Ok(tools)) => tools,
            Ok(Err(source)) => {
                return Err(McpBridgeError::ListTools {
                    server: server.name.clone(),
                    source,
                });
            }
            Err(_) => {
                return Err(McpBridgeError::ListToolsTimeout {
                    server: server.name.clone(),
                    timeout_secs: MCP_DISCOVERY_TIMEOUT.as_secs(),
                });
            }
        };

        let defs = mcp_tools
            .iter()
            .map(|tool| mcp_to_tool_def(tool, &server.name))
            .collect();
        all_server_tools.push((server.name.clone(), defs));
    }

    Ok(dedup_tools(all_server_tools))
}
