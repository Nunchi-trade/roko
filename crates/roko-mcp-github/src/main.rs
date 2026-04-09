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
}
