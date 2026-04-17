//! Shared stdio JSON-RPC transport for Roko MCP servers.
//!
//! The transport is intentionally line-delimited JSON-RPC 2.0 over stdin/stdout
//! so standalone MCP servers can share the exact same wire behavior.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, Write};

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    /// Must be `"2.0"`.
    pub jsonrpc: String,
    /// Method name.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: Value,
    /// Request identifier.
    #[serde(default)]
    pub id: Value,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsonRpcError {
    /// Numeric error code.
    pub code: i64,
    /// Human-readable message.
    pub message: String,
    /// Optional structured data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    /// Parse error code from the JSON-RPC spec.
    pub const PARSE_ERROR: i64 = -32700;
    /// Invalid request code from the JSON-RPC spec.
    pub const INVALID_REQUEST: i64 = -32600;
    /// Method not found code from the JSON-RPC spec.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Internal error code from the JSON-RPC spec.
    pub const INTERNAL_ERROR: i64 = -32603;

    /// Build a parse error response.
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self {
            code: Self::PARSE_ERROR,
            message: message.into(),
            data: None,
        }
    }

    /// Build an invalid request response.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: Self::INVALID_REQUEST,
            message: message.into(),
            data: None,
        }
    }

    /// Build an invalid-parameters response.
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::invalid_request(message)
    }

    /// Build a method-not-found response.
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: Self::METHOD_NOT_FOUND,
            message: format!("method not found: {method}"),
            data: None,
        }
    }

    /// Build an internal-error response.
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self {
            code: Self::INTERNAL_ERROR,
            message: message.into(),
            data: None,
        }
    }
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

/// Serve an MCP-style JSON-RPC loop over stdin/stdout.
///
/// Requests and responses are serialized as single newline-delimited JSON
/// objects. Notifications are handled by calling the handler and discarding
/// any returned value.
pub fn serve_stdio<R, W, F>(reader: R, mut writer: W, mut handler: F) -> anyhow::Result<()>
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
                    &JsonRpcResponse {
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
                    &JsonRpcResponse {
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
                &JsonRpcResponse {
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

        write_response(&mut writer, &response)?;
    }

    writer.flush().context("flush JSON-RPC output")?;
    Ok(())
}

fn write_response<W: Write>(writer: &mut W, response: &JsonRpcResponse) -> anyhow::Result<()> {
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

        let response: Value = serde_json::from_slice(&output).expect("parse error response json");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], JsonRpcError::PARSE_ERROR);
    }
}
