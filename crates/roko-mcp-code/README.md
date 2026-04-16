# roko-mcp-code

MCP (Model Context Protocol) server that exposes Roko's code-intelligence
primitives — symbol lookup, dependency graph walks, HDC-based similarity —
to any MCP-speaking agent (Claude Desktop, Cursor, a Roko agent, etc.).

## What MCP gives you

MCP is Anthropic's JSON-RPC–over-stdio protocol for agents to call external
tools. An agent sees a menu of tools; `roko-mcp-code` publishes a set of
code-search + graph-walk tools backed by the same indexer Roko uses
internally (`roko-index`).

## Tools exposed

| Tool | What it does |
|------|-------------|
| `find_symbol` | Look up a symbol by name; returns file, line, definition, and usage sites |
| `get_symbol_context` | Given a file:line, return the enclosing symbol + doc comments |
| `walk_dependencies` | Walk the dependency graph from a symbol (forward or reverse) |
| `find_similar` | HDC-similarity search over the codebase for a given symbol shape |
| `list_files_touching` | Given a topic or symbol, list files that reference it |
| `get_module_tree` | Render the module tree for a crate |

Exact tool names, arguments, and return schemas are declared in the
`tools/list` MCP response — introspect with any MCP client.

## Running it

### As a stdio MCP server (normal use)

```bash
roko-mcp-code --workspace /path/to/project
```

The binary reads JSON-RPC frames on stdin and writes responses on stdout.
That's what any MCP-speaking agent will spawn.

### Configured in a Roko agent's `mcp_config`

```toml
# roko.toml
[agent]
command = "claude"
mcp_config = ".roko/mcp-servers.json"
```

With `.roko/mcp-servers.json`:

```json
{
  "mcpServers": {
    "roko-code": {
      "command": "roko-mcp-code",
      "args": ["--workspace", "."]
    }
  }
}
```

Roko passes this file through to the configured LLM CLI (`claude
--mcp-config` or equivalent), so your agent gets access to the tools
without any other wiring.

### Configured in Claude Desktop

```json
// ~/Library/Application Support/Claude/claude_desktop_config.json
{
  "mcpServers": {
    "roko-code": {
      "command": "/path/to/target/release/roko-mcp-code",
      "args": ["--workspace", "/path/to/project"]
    }
  }
}
```

Restart Claude Desktop; `roko-code` tools appear in the tool menu.

## Building

```bash
cargo build -p roko-mcp-code --release
# binary: target/release/roko-mcp-code
```

## Testing

```bash
cargo test -p roko-mcp-code
cargo clippy -p roko-mcp-code --no-deps -- -D warnings
```

Integration tests use a fixture workspace with known symbols, call every
tool, and assert return shapes.

## Sibling MCP crates

Roko ships a few adjacent MCP servers for integrations other than code:

- `roko-mcp-github` — repo + issue + PR access
- `roko-mcp-slack` — channel + DM + search
- `roko-mcp-scripts` — run whitelisted shell scripts
- `roko-mcp-stdio` — generic stdio adapter

See each crate's own source; they follow the same launch pattern.

## Related

- Top-level `README.md` — multi-provider agent configuration
- `crates/roko-index/` — the underlying symbol graph + HDC fingerprints
- Anthropic's MCP spec: https://modelcontextprotocol.io
