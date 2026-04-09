//! MCP server for `roko-mcp-slack`.
//!
//! Implements a small JSON-RPC 2.0 stdio server that exposes Slack Web API
//! tools to MCP clients.

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use std::env;
use std::io;
use roko_mcp_stdio::{serve_stdio, JsonRpcError, JsonRpcRequest};

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default = "empty_json_object")]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct SlackPostMessageArguments {
    channel: String,
    text: String,
    #[serde(default)]
    thread_ts: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackReplyArguments {
    channel: String,
    thread_ts: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct SlackPostMessageResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    message: Option<Value>,
}

#[derive(Debug, Clone)]
struct SlackClient {
    client: Client,
    token: String,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("roko_mcp_slack=info")
        .with_writer(io::stderr)
        .init();

    serve_stdio(io::stdin().lock(), io::stdout().lock(), handle_request)?;
    Ok(())
}

fn handle_request(request: JsonRpcRequest) -> Result<Value, JsonRpcError> {
    match request.method.as_str() {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(request.params),
        _ => Err(JsonRpcError::method_not_found(&request.method)),
    }
}

fn handle_initialize() -> Value {
    serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "roko-mcp-slack",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn handle_tools_list() -> Value {
    serde_json::json!({
        "tools": [
            serde_json::json!({
                "name": "slack.post_message",
                "description": "Post a message to a Slack channel.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "channel": {"type": "string"},
                        "text": {"type": "string"},
                        "thread_ts": {"type": "string"}
                    },
                    "required": ["channel", "text"],
                    "additionalProperties": false
                }
            }),
            serde_json::json!({
                "name": "slack_reply",
                "description": "Reply to a Slack thread.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "channel": {"type": "string"},
                        "thread_ts": {"type": "string"},
                        "text": {"type": "string"}
                    },
                    "required": ["channel", "thread_ts", "text"],
                    "additionalProperties": false
                }
            })
        ]
    })
}

fn handle_tools_call(params: Value) -> Result<Value, JsonRpcError> {
    let params: ToolsCallParams = serde_json::from_value(params)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid tools/call params: {err}")))?;
    dispatch_tool_call(&params.name, params.arguments)
}

fn dispatch_tool_call(name: &str, arguments: Value) -> Result<Value, JsonRpcError> {
    match name {
        "slack.post_message" => handle_slack_post_message(arguments),
        "slack_reply" => handle_slack_reply(arguments),
        _ => Err(JsonRpcError::invalid_params(format!(
            "unknown tool: {name}"
        ))),
    }
}

fn empty_json_object() -> Value {
    Value::Object(Default::default())
}

fn handle_slack_post_message(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: SlackPostMessageArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid slack.post_message args: {err}"))
    })?;

    let client = SlackClient::from_env()?;
    let response = client.post_message(&args.channel, &args.text, args.thread_ts.as_deref())?;
    let SlackPostMessageResponse {
        ok: _,
        error: _,
        channel,
        ts,
        message,
    } = response;
    let channel = channel
        .ok_or_else(|| JsonRpcError::internal_error("Slack API response missing channel"))?;
    let ts = ts
        .ok_or_else(|| JsonRpcError::internal_error("Slack API response missing ts"))?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "channel": channel,
                "ts": ts,
                "message": message
            }).to_string()
        }],
        "isError": false
    }))
}

fn handle_slack_reply(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: SlackReplyArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid slack_reply args: {err}"))
    })?;

    let client = SlackClient::from_env()?;
    let response = client.post_message(&args.channel, &args.text, Some(&args.thread_ts))?;
    let SlackPostMessageResponse {
        ok: _,
        error: _,
        channel,
        ts,
        message,
    } = response;
    let channel = channel
        .ok_or_else(|| JsonRpcError::internal_error("Slack API response missing channel"))?;
    let ts = ts
        .ok_or_else(|| JsonRpcError::internal_error("Slack API response missing ts"))?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "channel": channel,
                "ts": ts,
                "message": message
            }).to_string()
        }],
        "isError": false
    }))
}

impl SlackClient {
    fn from_env() -> Result<Self, JsonRpcError> {
        let token = env::var("SLACK_BOT_TOKEN")
            .or_else(|_| env::var("SLACK_TOKEN"))
            .map_err(|_| {
                JsonRpcError::internal_error(
                    "SLACK_BOT_TOKEN or SLACK_TOKEN env var required",
                )
            })?;

        Ok(Self {
            client: Client::new(),
            token,
        })
    }

    fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<SlackPostMessageResponse, JsonRpcError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
        let auth_value = format!("Bearer {}", self.token);
        let auth_value = HeaderValue::from_str(&auth_value)
            .map_err(|err| JsonRpcError::internal_error(format!("invalid slack auth header: {err}")))?;
        headers.insert(AUTHORIZATION, auth_value);

        let mut body = serde_json::json!({
            "channel": channel,
            "text": text,
        });
        if let Some(thread_ts) = thread_ts {
            body["thread_ts"] = Value::String(thread_ts.to_owned());
        }

        let response = self
            .client
            .post("https://slack.com/api/chat.postMessage")
            .headers(headers)
            .json(&body)
            .send()
            .map_err(|err| JsonRpcError::internal_error(format!("slack request failed: {err}")))?;

        let parsed: SlackPostMessageResponse = response
            .json()
            .map_err(|err| JsonRpcError::internal_error(format!("invalid slack response: {err}")))?;

        if !parsed.ok {
            let error = parsed.error.as_deref().unwrap_or("unknown");
            return Err(JsonRpcError::internal_error(format!(
                "Slack API error: {error}"
            )));
        }

        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_contains_post_message() {
        let tools = handle_tools_list()["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "slack.post_message");
        assert_eq!(tools[0]["inputSchema"]["required"], serde_json::json!(["channel", "text"]));
        assert_eq!(tools[1]["name"], "slack_reply");
        assert_eq!(tools[1]["inputSchema"]["required"], serde_json::json!(["channel", "thread_ts", "text"]));
    }

    #[test]
    fn dispatch_rejects_unknown_tool() {
        let err = dispatch_tool_call("slack.unknown", serde_json::json!({}))
            .expect_err("unknown tool should fail");
        assert!(err.message.contains("unknown tool"));
    }
}
