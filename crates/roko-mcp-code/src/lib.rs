//! MCP server for code-intelligence queries backed by `roko-index`.

use anyhow::Result;
use roko_index::WorkspaceIndex;
use roko_mcp_stdio::{serve_stdio, JsonRpcError, JsonRpcRequest};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
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
    let root = env::var_os("ROKO_WORKSPACE_ROOT").map_or(env::current_dir()?, PathBuf::from);
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
                &json!({
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
                &json!({
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
                &json!({
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
                &json!({
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

fn tool_spec(name: &str, description: &str, input_schema: &Value) -> Value {
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
    Value::Object(serde_json::Map::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::language::{Symbol, SymbolKind, Visibility};
    use serde_json::json;
    use std::io::Cursor;

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

    #[test]
    fn tools_call_missing_arguments_field_returns_error() {
        let index = make_index();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: json!({ "arguments": {} }),
            id: json!(1),
        };

        let err = match handle_request(request, &index) {
            Ok(value) => panic!(
                "expected invalid tools/call params error for missing arguments field, got success: {value}"
            ),
            Err(err) => err,
        };

        assert_eq!(
            err.code,
            JsonRpcError::INVALID_REQUEST,
            "missing arguments field should be reported as an invalid request"
        );
        assert!(
            err.message.contains("invalid tools/call params"),
            "error message should identify the tools/call parameter shape problem: {}",
            err.message
        );
        assert!(
            err.message.contains("name"),
            "error message should mention the missing name field: {}",
            err.message
        );
    }

    #[test]
    fn symbol_lookup_missing_required_arg_returns_error() {
        let index = make_index();
        let err = match dispatch_tool_call("symbol_lookup", json!({}), &index) {
            Ok(value) => panic!(
                "expected invalid symbol_lookup args error for missing name field, got success: {value}"
            ),
            Err(err) => err,
        };

        assert_eq!(
            err.code,
            JsonRpcError::INVALID_REQUEST,
            "missing symbol_lookup.name should be reported as an invalid request"
        );
        assert!(
            err.message.contains("invalid symbol_lookup args"),
            "error message should identify the symbol_lookup argument parsing problem: {}",
            err.message
        );
        assert!(
            err.message.contains("name"),
            "error message should mention the missing name field: {}",
            err.message
        );
    }

    #[test]
    fn imports_missing_file_returns_error() {
        let index = make_index();
        let err = match dispatch_tool_call("imports", json!({ "file": "missing.rs" }), &index) {
            Ok(value) => panic!(
                "expected invalid imports args error for nonexistent file, got success: {value}"
            ),
            Err(err) => err,
        };

        assert_eq!(
            err.code,
            JsonRpcError::INVALID_REQUEST,
            "nonexistent imports file should be reported as an invalid request"
        );
        assert!(
            err.message.contains("resolve workspace file missing.rs")
                || err.message.contains("file 'missing.rs' was not indexed"),
            "error message should mention the missing file path: {}",
            err.message
        );
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let mut output = Vec::new();

        serve_stdio(Cursor::new(b"{not json}\n"), &mut output, |_request| {
            panic!("handler should not be called for malformed JSON input");
        })
        .expect("stdio transport");

        let response: Value = serde_json::from_slice(&output)
            .expect("malformed JSON should still produce a structured response");
        assert_eq!(
            response["jsonrpc"], "2.0",
            "malformed JSON should produce a JSON-RPC 2.0 response"
        );
        assert_eq!(
            response["id"],
            Value::Null,
            "parse errors should use a null request id"
        );
        assert_eq!(
            response["error"]["code"],
            JsonRpcError::PARSE_ERROR,
            "malformed JSON should surface as a parse error"
        );
    }
}
