//! MCP server stdio transport for `roko-mcp-github`.
//!
//! This module implements the JSON-RPC 2.0 framing layer used by MCP
//! servers: read line-delimited JSON from stdin and write line-delimited
//! JSON responses to stdout.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    id: Value,
}

#[derive(Debug, Serialize, PartialEq)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcError {
    const PARSE_ERROR: i64 = -32700;
    const INVALID_REQUEST: i64 = -32600;
    const METHOD_NOT_FOUND: i64 = -32601;

    fn parse_error(message: impl Into<String>) -> Self {
        Self {
            code: Self::PARSE_ERROR,
            message: message.into(),
            data: None,
        }
    }

    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: Self::INVALID_REQUEST,
            message: message.into(),
            data: None,
        }
    }

    fn method_not_found(method: &str) -> Self {
        Self {
            code: Self::METHOD_NOT_FOUND,
            message: format!("method not found: {method}"),
            data: None,
        }
    }
}

fn main() -> anyhow::Result<()> {
    serve_stdio(io::stdin().lock(), io::stdout().lock(), |request| {
        let _ = &request.params;
        match request.method.as_str() {
            "initialize" => Ok(handle_initialize()),
            "tools/list" => Ok(handle_tools_list()),
            _ => Err(JsonRpcError::method_not_found(&request.method)),
        }
    })
}

fn handle_initialize() -> Value {
    serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "roko-mcp-github",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn handle_tools_list() -> Value {
    serde_json::json!({
        "tools": [
            github_tool(
                "github.list_prs",
                "List pull requests in a repository.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "state": {"type": "string", "enum": ["open", "closed", "all"]},
                        "head": {"type": "string"},
                        "base": {"type": "string"},
                        "per_page": {"type": "integer", "minimum": 1}
                    },
                    "required": ["owner", "repo"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.get_pr",
                "Get a pull request, optionally including its diff.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "number": {"type": "integer", "minimum": 1},
                        "include_diff": {"type": "boolean"}
                    },
                    "required": ["owner", "repo", "number"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.create_pr",
                "Create a pull request.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "title": {"type": "string"},
                        "body": {"type": "string"},
                        "head": {"type": "string"},
                        "base": {"type": "string"},
                        "draft": {"type": "boolean"}
                    },
                    "required": ["owner", "repo", "title", "body", "head", "base"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.comment_pr",
                "Comment on a pull request.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "number": {"type": "integer", "minimum": 1},
                        "body": {"type": "string"}
                    },
                    "required": ["owner", "repo", "number", "body"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.review_pr",
                "Create a pull request review.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "number": {"type": "integer", "minimum": 1},
                        "event": {"type": "string", "enum": ["APPROVE", "REQUEST_CHANGES", "COMMENT"]},
                        "body": {"type": "string"},
                        "comments": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "line": {"type": "integer", "minimum": 1},
                                    "body": {"type": "string"}
                                },
                                "required": ["path", "line", "body"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["owner", "repo", "number", "event", "body"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.merge_pr",
                "Merge a pull request.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "number": {"type": "integer", "minimum": 1},
                        "merge_method": {"type": "string", "enum": ["merge", "squash", "rebase"]},
                        "commit_title": {"type": "string"}
                    },
                    "required": ["owner", "repo", "number"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.list_issues",
                "List issues in a repository.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "state": {"type": "string"},
                        "labels": {
                            "type": "array",
                            "items": {"type": "string"}
                        },
                        "assignee": {"type": "string"},
                        "per_page": {"type": "integer", "minimum": 1}
                    },
                    "required": ["owner", "repo"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.create_issue",
                "Create an issue.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "title": {"type": "string"},
                        "body": {"type": "string"},
                        "labels": {
                            "type": "array",
                            "items": {"type": "string"}
                        },
                        "assignees": {
                            "type": "array",
                            "items": {"type": "string"}
                        }
                    },
                    "required": ["owner", "repo", "title", "body"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.comment_issue",
                "Comment on an issue.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "number": {"type": "integer", "minimum": 1},
                        "body": {"type": "string"}
                    },
                    "required": ["owner", "repo", "number", "body"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.close_issue",
                "Close an issue.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "number": {"type": "integer", "minimum": 1},
                        "reason": {"type": "string", "enum": ["completed", "not_planned"]}
                    },
                    "required": ["owner", "repo", "number"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.add_labels",
                "Add labels to an issue.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "number": {"type": "integer", "minimum": 1},
                        "labels": {
                            "type": "array",
                            "items": {"type": "string"}
                        }
                    },
                    "required": ["owner", "repo", "number", "labels"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.create_label",
                "Create a repository label.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "name": {"type": "string"},
                        "color": {"type": "string"},
                        "description": {"type": "string"}
                    },
                    "required": ["owner", "repo", "name", "color"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.get_file",
                "Fetch a file from a repository.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "path": {"type": "string"},
                        "ref": {"type": "string"}
                    },
                    "required": ["owner", "repo", "path"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.search_code",
                "Search code in GitHub repositories.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "per_page": {"type": "integer", "minimum": 1}
                    },
                    "required": ["query"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.list_commits",
                "List commits in a repository.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "sha": {"type": "string"},
                        "path": {"type": "string"},
                        "since": {"type": "string"},
                        "until": {"type": "string"},
                        "per_page": {"type": "integer", "minimum": 1}
                    },
                    "required": ["owner", "repo"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.create_branch",
                "Create a branch from a commit SHA.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "branch": {"type": "string"},
                        "from_sha": {"type": "string"}
                    },
                    "required": ["owner", "repo", "branch", "from_sha"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.get_branch",
                "Get branch metadata.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "branch": {"type": "string"}
                    },
                    "required": ["owner", "repo", "branch"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.compare_branches",
                "Compare two branches.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "base": {"type": "string"},
                        "head": {"type": "string"}
                    },
                    "required": ["owner", "repo", "base", "head"],
                    "additionalProperties": false
                })
            ),
            github_tool(
                "github.get_actions_status",
                "Get the combined GitHub Actions status for a ref.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "ref": {"type": "string"}
                    },
                    "required": ["owner", "repo", "ref"],
                    "additionalProperties": false
                })
            ),
        ]
    })
}

fn github_tool(name: &str, description: &str, input_schema: Value) -> Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

fn serve_stdio<R, W, F>(reader: R, mut writer: W, mut handler: F) -> anyhow::Result<()>
where
    R: BufRead,
    W: Write,
    F: FnMut(JsonRpcRequest) -> Result<Value, JsonRpcError>,
{
    for line in reader.lines() {
        let line = line.context("read JSON-RPC line")?;
        if line.trim().is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                write_response(
                    &mut writer,
                    JsonRpcResponse {
                        jsonrpc: "2.0",
                        result: None,
                        error: Some(JsonRpcError::parse_error(err.to_string())),
                        id: Value::Null,
                    },
                )?;
                continue;
            }
        };

        let has_id = parsed.get("id").is_some();
        let request_id = parsed.get("id").cloned().unwrap_or(Value::Null);
        let request: JsonRpcRequest = match serde_json::from_value(parsed) {
            Ok(request) => request,
            Err(err) => {
                write_response(
                    &mut writer,
                    JsonRpcResponse {
                        jsonrpc: "2.0",
                        result: None,
                        error: Some(JsonRpcError::invalid_request(err.to_string())),
                        id: request_id,
                    },
                )?;
                continue;
            }
        };

        if request.jsonrpc != "2.0" {
            write_response(
                &mut writer,
                JsonRpcResponse {
                    jsonrpc: "2.0",
                    result: None,
                    error: Some(JsonRpcError::invalid_request(
                        "jsonrpc field must be \"2.0\"",
                    )),
                    id: request.id,
                },
            )?;
            continue;
        }

        if !has_id {
            let _ = handler(request);
            continue;
        }

        let response = match handler(request) {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0",
                result: Some(result),
                error: None,
                id: request_id,
            },
            Err(error) => JsonRpcResponse {
                jsonrpc: "2.0",
                result: None,
                error: Some(error),
                id: request_id,
            },
        };

        write_response(&mut writer, response)?;
    }

    writer.flush().context("flush JSON-RPC output")?;
    Ok(())
}

fn write_response<W: Write>(writer: &mut W, response: JsonRpcResponse) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *writer, &response).context("serialize JSON-RPC response")?;
    writer.write_all(b"\n").context("write JSON-RPC newline")?;
    writer.flush().context("flush JSON-RPC response")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn serve_stdio_writes_responses_for_requests() {
        let input = b"{\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"id\":7}\n";
        let mut output = Vec::new();

        serve_stdio(Cursor::new(&input[..]), &mut output, |request| {
            assert_eq!(request.method, "tools/list");
            assert_eq!(request.params, Value::Null);
            Ok(json!({ "tools": [] }))
        })
        .expect("stdio transport");

        let lines: Vec<&str> = std::str::from_utf8(&output)
            .expect("utf8")
            .lines()
            .collect();
        assert_eq!(lines.len(), 1);

        let response: Value = serde_json::from_str(lines[0]).expect("response json");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 7);
        assert_eq!(response["result"]["tools"], json!([]));
        assert!(response.get("error").is_none());
    }

    #[test]
    fn serve_stdio_reports_parse_errors() {
        let input = b"{not json}\n";
        let mut output = Vec::new();

        serve_stdio(Cursor::new(&input[..]), &mut output, |_request| {
            panic!("handler should not be called for invalid json");
        })
        .expect("stdio transport");

        let response: Value =
            serde_json::from_slice(&output).expect("parse error response json");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], JsonRpcError::PARSE_ERROR);
    }

    #[test]
    fn initialize_returns_server_capabilities() {
        let result = handle_initialize();

        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["capabilities"]["tools"], json!({}));
        assert_eq!(result["serverInfo"]["name"], "roko-mcp-github");
        assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn tools_list_returns_all_tool_definitions() {
        let result = handle_tools_list();
        let tools = result["tools"].as_array().expect("tools array");

        assert_eq!(tools.len(), 19);
        assert_eq!(tools[0]["name"], "github.list_prs");
        assert_eq!(tools[18]["name"], "github.get_actions_status");

        let get_pr = tools
            .iter()
            .find(|tool| tool["name"] == "github.get_pr")
            .expect("github.get_pr tool");
        assert_eq!(get_pr["description"], "Get a pull request, optionally including its diff.");
        assert_eq!(get_pr["inputSchema"]["required"], json!(["owner", "repo", "number"]));
    }
}
