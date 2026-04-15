//! MCP server for code-intelligence queries backed by `roko-index`.

use anyhow::Result;
use roko_index::WorkspaceIndex;
use roko_mcp_stdio::{JsonRpcError, JsonRpcRequest, serve_stdio};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use std::env;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default = "empty_json_object")]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct SymbolLookupArguments {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CallGraphArguments {
    function: String,
    depth: u32,
}

#[derive(Debug, Deserialize)]
struct ImportsArguments {
    file: String,
}

#[derive(Debug, Deserialize)]
struct SemanticSearchArguments {
    query: String,
    limit: u32,
}

/// Run the MCP server over stdio.
pub fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("roko_mcp_code=info")
        .with_writer(io::stderr)
        .init();

    let workspace = Arc::new(load_workspace_index()?);
    serve_stdio(io::stdin().lock(), io::stdout().lock(), move |request| {
        handle_request(request, workspace.as_ref())
    })?;
    Ok(())
}

fn load_workspace_index() -> Result<WorkspaceIndex> {
    let root = env::var_os("ROKO_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    WorkspaceIndex::load(root)
}

fn handle_request(request: JsonRpcRequest, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    match request.method.as_str() {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(request.params, index),
        _ => Err(JsonRpcError::method_not_found(&request.method)),
    }
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "roko-mcp-code",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [
            tool_spec(
                "symbol_lookup",
                "Look up symbol definitions by exact name.",
                json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Symbol name to resolve."}
                    },
                    "required": ["name"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "call_graph",
                "Return callers and callees around a function name.",
                json!({
                    "type": "object",
                    "properties": {
                        "function": {"type": "string", "description": "Function name to inspect."},
                        "depth": {"type": "integer", "minimum": 0, "description": "Traversal depth."}
                    },
                    "required": ["function", "depth"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "imports",
                "Return a file's parsed import list.",
                json!({
                    "type": "object",
                    "properties": {
                        "file": {"type": "string", "description": "Workspace-relative file path."}
                    },
                    "required": ["file"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "semantic_search",
                "Search code semantically using HDC fingerprints.",
                json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query text."},
                        "limit": {"type": "integer", "minimum": 1, "description": "Maximum number of results."}
                    },
                    "required": ["query", "limit"],
                    "additionalProperties": false
                })
            ),
        ]
    })
}

fn handle_tools_call(params: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let params: ToolsCallParams = serde_json::from_value(params)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid tools/call params: {err}")))?;
    dispatch_tool_call(&params.name, params.arguments, index)
}

fn dispatch_tool_call(
    name: &str,
    arguments: Value,
    index: &WorkspaceIndex,
) -> Result<Value, JsonRpcError> {
    match name {
        "symbol_lookup" => handle_symbol_lookup(arguments, index),
        "call_graph" => handle_call_graph(arguments, index),
        "imports" => handle_imports(arguments, index),
        "semantic_search" => handle_semantic_search(arguments, index),
        _ => Err(JsonRpcError::invalid_params(format!(
            "unknown tool: {name}"
        ))),
    }
}

fn handle_symbol_lookup(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: SymbolLookupArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid symbol_lookup args: {err}"))
    })?;
    let results = index.symbol_lookup(&args.name);
    tool_result(json!({
        "query": args.name,
        "results": results,
    }))
}

fn handle_call_graph(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: CallGraphArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid call_graph args: {err}")))?;
    let graph = index.call_graph(&args.function, args.depth);
    tool_result(graph)
}

fn handle_imports(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: ImportsArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid imports args: {err}")))?;
    let imports = index
        .imports_for_file(&args.file)
        .map_err(|err| JsonRpcError::invalid_params(err.to_string()))?;
    tool_result(json!({
        "file": args.file,
        "imports": imports,
    }))
}

fn handle_semantic_search(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: SemanticSearchArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid semantic_search args: {err}"))
    })?;
    let results = index.semantic_search(&args.query, args.limit);
    tool_result(json!({
        "query": args.query,
        "results": results,
    }))
}

fn tool_spec(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

fn tool_result<T: Serialize>(payload: T) -> Result<Value, JsonRpcError> {
    let text = serde_json::to_string(&payload)
        .map_err(|err| JsonRpcError::internal_error(err.to_string()))?;
    Ok(json!({
        "content": [{
            "type": "text",
            "text": text
        }],
        "isError": false
    }))
}

fn empty_json_object() -> Value {
    Value::Object(Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::language::{Symbol, SymbolKind, Visibility};

    fn make_index() -> WorkspaceIndex {
        WorkspaceIndex::from_source_files(vec![roko_index::SourceFile {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            content: "fn helper() {}\nfn main() { helper(); }\n".to_string(),
            symbols: vec![
                Symbol {
                    name: "helper".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    line: 1,
                },
                Symbol {
                    name: "main".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    line: 2,
                },
            ],
            imports: Vec::new(),
        }])
    }

    #[test]
    fn tools_list_includes_expected_tools() {
        let tools = handle_tools_list();
        let names = tools["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["symbol_lookup", "call_graph", "imports", "semantic_search",]
        );
    }

    #[test]
    fn symbol_lookup_tool_returns_matches() {
        let index = make_index();
        let result = dispatch_tool_call("symbol_lookup", json!({ "name": "helper" }), &index)
            .expect("tool result");

        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert_eq!(payload["query"], "helper");
        assert_eq!(payload["results"].as_array().expect("results").len(), 1);
    }

    #[test]
    fn call_graph_tool_returns_callers_and_callees() {
        let index = make_index();
        let result = dispatch_tool_call(
            "call_graph",
            json!({ "function": "main", "depth": 2 }),
            &index,
        )
        .expect("tool result");
        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert_eq!(payload["function"], "main");
        assert_eq!(payload["callees"].as_array().expect("callees").len(), 1);
    }
}
