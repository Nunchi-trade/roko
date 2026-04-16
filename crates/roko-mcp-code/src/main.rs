//! Binary entrypoint for the `roko-mcp-code` MCP server.

fn main() -> anyhow::Result<()> {
    roko_mcp_code::run()
}
