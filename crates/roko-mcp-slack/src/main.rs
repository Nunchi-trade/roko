//! MCP server for `roko-mcp-slack`.
//!
//! Implements a small JSON-RPC 2.0 stdio server that exposes Slack Web API
//! tools to MCP clients.

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use roko_mcp_stdio::{JsonRpcError, JsonRpcRequest, serve_stdio};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::env;
use std::io;

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
struct SlackGetThreadArguments {
    channel: String,
    thread_ts: String,
}

#[derive(Debug, Deserialize)]
struct SlackReactArguments {
    channel: String,
    ts: String,
    emoji: String,
}

#[derive(Debug, Deserialize)]
struct SlackListChannelsArguments {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SlackLookupUserArguments {
    email_or_name: String,
}

#[derive(Debug, Deserialize)]
struct SlackDmArguments {
    user_id: String,
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

#[derive(Debug, Default, Deserialize)]
struct SlackTextField {
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackChannel {
    id: String,
    name: String,
    #[serde(default)]
    topic: SlackTextField,
    #[serde(default)]
    purpose: SlackTextField,
    #[serde(default)]
    num_members: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SlackListChannelsResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    channels: Vec<SlackChannel>,
    #[serde(default)]
    response_metadata: SlackResponseMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct SlackResponseMetadata {
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct SlackThreadResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    messages: Vec<Value>,
    #[serde(default)]
    response_metadata: SlackResponseMetadata,
}

#[derive(Debug, Deserialize)]
struct SlackMutationResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackOpenConversationResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    channel: Option<SlackConversation>,
}

#[derive(Debug, Deserialize)]
struct SlackConversation {
    id: String,
}

#[derive(Debug, Default, Deserialize)]
struct SlackUserProfile {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    real_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackUser {
    id: String,
    name: String,
    #[serde(default)]
    real_name: Option<String>,
    #[serde(default)]
    deleted: Option<bool>,
    #[serde(default)]
    is_bot: Option<bool>,
    #[serde(default)]
    profile: SlackUserProfile,
}

#[derive(Debug, Deserialize)]
struct SlackLookupUserResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    user: Option<SlackUser>,
}

#[derive(Debug, Deserialize)]
struct SlackUsersListResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    members: Vec<SlackUser>,
    #[serde(default)]
    response_metadata: SlackResponseMetadata,
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
            }),
            serde_json::json!({
                "name": "slack_get_thread",
                "description": "Get all messages in a Slack thread.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "channel": {"type": "string"},
                        "thread_ts": {"type": "string"}
                    },
                    "required": ["channel", "thread_ts"],
                    "additionalProperties": false
                }
            }),
            serde_json::json!({
                "name": "slack_react",
                "description": "Add a reaction to a Slack message.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "channel": {"type": "string"},
                        "ts": {"type": "string"},
                        "emoji": {"type": "string"}
                    },
                    "required": ["channel", "ts", "emoji"],
                    "additionalProperties": false
                }
            }),
            serde_json::json!({
                "name": "slack.list_channels",
                "description": "List Slack channels the bot can access.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": {"type": "integer"}
                    },
                    "additionalProperties": false
                }
            }),
            serde_json::json!({
                "name": "slack_lookup_user",
                "description": "Find a Slack user ID from an email address or user name.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "email_or_name": {"type": "string"}
                    },
                    "required": ["email_or_name"],
                    "additionalProperties": false
                }
            }),
            serde_json::json!({
                "name": "slack_dm",
                "description": "Send a direct message to a Slack user.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "user_id": {"type": "string"},
                        "text": {"type": "string"}
                    },
                    "required": ["user_id", "text"],
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
        "slack_get_thread" => handle_slack_get_thread(arguments),
        "slack_react" => handle_slack_react(arguments),
        "slack.list_channels" => handle_slack_list_channels(arguments),
        "slack_lookup_user" => handle_slack_lookup_user(arguments),
        "slack_dm" => handle_slack_dm(arguments),
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
    let ts = ts.ok_or_else(|| JsonRpcError::internal_error("Slack API response missing ts"))?;

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
    let args: SlackReplyArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid slack_reply args: {err}")))?;

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
    let ts = ts.ok_or_else(|| JsonRpcError::internal_error("Slack API response missing ts"))?;

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

fn handle_slack_get_thread(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: SlackGetThreadArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid slack_get_thread args: {err}"))
    })?;

    let client = SlackClient::from_env()?;
    let messages = client.get_thread(&args.channel, &args.thread_ts)?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "channel": args.channel,
                "thread_ts": args.thread_ts,
                "messages": messages
            }).to_string()
        }],
        "isError": false
    }))
}

fn handle_slack_react(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: SlackReactArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid slack_react args: {err}")))?;

    let client = SlackClient::from_env()?;
    client.add_reaction(&args.channel, &args.ts, &args.emoji)?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "channel": args.channel,
                "ts": args.ts,
                "emoji": args.emoji
            }).to_string()
        }],
        "isError": false
    }))
}

fn handle_slack_list_channels(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: SlackListChannelsArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid slack.list_channels args: {err}"))
    })?;

    let client = SlackClient::from_env()?;
    let response = client.list_channels(args.limit)?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "channels": response.channels.into_iter().map(|channel| serde_json::json!({
                    "id": channel.id,
                    "name": channel.name,
                    "topic": channel.topic.value.unwrap_or_default(),
                    "purpose": channel.purpose.value.unwrap_or_default(),
                    "num_members": channel.num_members
                })).collect::<Vec<_>>(),
                "next_cursor": response.response_metadata.next_cursor
            }).to_string()
        }],
        "isError": false
    }))
}

fn handle_slack_lookup_user(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: SlackLookupUserArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid slack_lookup_user args: {err}"))
    })?;

    let client = SlackClient::from_env()?;
    let result = client.lookup_user(&args.email_or_name)?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "user_id": result.id,
                "name": result.name,
                "real_name": result.real_name,
                "email": result.email,
                "matched_by": result.matched_by
            }).to_string()
        }],
        "isError": false
    }))
}

fn handle_slack_dm(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: SlackDmArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid slack_dm args: {err}")))?;

    let client = SlackClient::from_env()?;
    let channel_id = client.open_dm_channel(&args.user_id)?;
    let response = client.post_message(&channel_id, &args.text, None)?;
    let SlackPostMessageResponse {
        ok: _,
        error: _,
        channel,
        ts,
        message,
    } = response;
    let channel = channel
        .ok_or_else(|| JsonRpcError::internal_error("Slack API response missing channel"))?;
    let ts = ts.ok_or_else(|| JsonRpcError::internal_error("Slack API response missing ts"))?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "user_id": args.user_id,
                "channel": channel,
                "ts": ts,
                "message": message
            }).to_string()
        }],
        "isError": false
    }))
}

#[derive(Debug)]
struct SlackLookupUserResult {
    id: String,
    name: String,
    real_name: Option<String>,
    email: Option<String>,
    matched_by: String,
}

impl SlackClient {
    fn from_env() -> Result<Self, JsonRpcError> {
        let token = env::var("SLACK_BOT_TOKEN")
            .map_err(|_| JsonRpcError::internal_error("SLACK_BOT_TOKEN env var required"))?;

        Ok(Self {
            client: Client::new(),
            token,
        })
    }

    fn auth_headers(&self) -> Result<HeaderMap, JsonRpcError> {
        let mut headers = HeaderMap::new();
        let auth_value = format!("Bearer {}", self.token);
        let auth_value = HeaderValue::from_str(&auth_value).map_err(|err| {
            JsonRpcError::internal_error(format!("invalid slack auth header: {err}"))
        })?;
        headers.insert(AUTHORIZATION, auth_value);
        Ok(headers)
    }

    fn open_dm_channel(&self, user_id: &str) -> Result<String, JsonRpcError> {
        let headers = self.auth_headers()?;

        let response = self
            .client
            .post("https://slack.com/api/conversations.open")
            .headers(headers)
            .form(&[("users", user_id)])
            .send()
            .map_err(|err| JsonRpcError::internal_error(format!("slack request failed: {err}")))?;

        let parsed: SlackOpenConversationResponse = response.json().map_err(|err| {
            JsonRpcError::internal_error(format!("invalid slack response: {err}"))
        })?;

        if !parsed.ok {
            let error = parsed.error.as_deref().unwrap_or("unknown");
            return Err(JsonRpcError::internal_error(format!(
                "Slack API error: {error}"
            )));
        }

        parsed
            .channel
            .map(|channel| channel.id)
            .ok_or_else(|| JsonRpcError::internal_error("Slack API response missing channel"))
    }

    fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<SlackPostMessageResponse, JsonRpcError> {
        let mut headers = self.auth_headers()?;
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );

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

        let parsed: SlackPostMessageResponse = response.json().map_err(|err| {
            JsonRpcError::internal_error(format!("invalid slack response: {err}"))
        })?;

        if !parsed.ok {
            let error = parsed.error.as_deref().unwrap_or("unknown");
            return Err(JsonRpcError::internal_error(format!(
                "Slack API error: {error}"
            )));
        }

        Ok(parsed)
    }

    fn add_reaction(
        &self,
        channel: &str,
        ts: &str,
        emoji: &str,
    ) -> Result<SlackMutationResponse, JsonRpcError> {
        let headers = self.auth_headers()?;

        let response = self
            .client
            .post("https://slack.com/api/reactions.add")
            .headers(headers)
            .form(&[("channel", channel), ("timestamp", ts), ("name", emoji)])
            .send()
            .map_err(|err| JsonRpcError::internal_error(format!("slack request failed: {err}")))?;

        let parsed: SlackMutationResponse = response.json().map_err(|err| {
            JsonRpcError::internal_error(format!("invalid slack response: {err}"))
        })?;

        if !parsed.ok {
            let error = parsed.error.as_deref().unwrap_or("unknown");
            return Err(JsonRpcError::internal_error(format!(
                "Slack API error: {error}"
            )));
        }

        Ok(parsed)
    }

    fn list_channels(&self, limit: Option<u32>) -> Result<SlackListChannelsResponse, JsonRpcError> {
        let headers = self.auth_headers()?;

        let mut request = self
            .client
            .get("https://slack.com/api/conversations.list")
            .headers(headers);
        if let Some(limit) = limit {
            request = request.query(&[("limit", limit)]);
        }

        let response = request
            .send()
            .map_err(|err| JsonRpcError::internal_error(format!("slack request failed: {err}")))?;

        let parsed: SlackListChannelsResponse = response.json().map_err(|err| {
            JsonRpcError::internal_error(format!("invalid slack response: {err}"))
        })?;

        if !parsed.ok {
            let error = parsed.error.as_deref().unwrap_or("unknown");
            return Err(JsonRpcError::internal_error(format!(
                "Slack API error: {error}"
            )));
        }

        Ok(parsed)
    }

    fn lookup_user(&self, email_or_name: &str) -> Result<SlackLookupUserResult, JsonRpcError> {
        if email_or_name.contains('@') {
            let user = self.lookup_user_by_email(email_or_name)?;
            return Ok(SlackLookupUserResult {
                id: user.id,
                name: user.name,
                real_name: user
                    .real_name
                    .or(user.profile.real_name)
                    .unwrap_or_default()
                    .into(),
                email: user.profile.email,
                matched_by: "email".to_string(),
            });
        }

        self.lookup_user_by_name(email_or_name)
    }

    fn lookup_user_by_email(&self, email: &str) -> Result<SlackUser, JsonRpcError> {
        let headers = self.auth_headers()?;

        let response = self
            .client
            .get("https://slack.com/api/users.lookupByEmail")
            .headers(headers)
            .query(&[("email", email)])
            .send()
            .map_err(|err| JsonRpcError::internal_error(format!("slack request failed: {err}")))?;

        let parsed: SlackLookupUserResponse = response.json().map_err(|err| {
            JsonRpcError::internal_error(format!("invalid slack response: {err}"))
        })?;

        if !parsed.ok {
            let error = parsed.error.as_deref().unwrap_or("unknown");
            return Err(JsonRpcError::internal_error(format!(
                "Slack API error: {error}"
            )));
        }

        parsed
            .user
            .ok_or_else(|| JsonRpcError::internal_error("Slack API response missing user"))
    }

    fn lookup_user_by_name(&self, name: &str) -> Result<SlackLookupUserResult, JsonRpcError> {
        let headers = self.auth_headers()?;
        let needle = name.trim();
        let lowered = needle.to_lowercase();
        let mut cursor: Option<String> = None;
        let mut matches = Vec::new();

        loop {
            let mut request = self
                .client
                .get("https://slack.com/api/users.list")
                .headers(headers.clone())
                .query(&[("limit", "200")]);
            if let Some(cursor) = cursor.as_deref() {
                request = request.query(&[("cursor", cursor)]);
            }

            let response = request.send().map_err(|err| {
                JsonRpcError::internal_error(format!("slack request failed: {err}"))
            })?;

            let parsed: SlackUsersListResponse = response.json().map_err(|err| {
                JsonRpcError::internal_error(format!("invalid slack response: {err}"))
            })?;

            if !parsed.ok {
                let error = parsed.error.as_deref().unwrap_or("unknown");
                return Err(JsonRpcError::internal_error(format!(
                    "Slack API error: {error}"
                )));
            }

            for user in parsed.members {
                if user.deleted.unwrap_or(false) || user.is_bot.unwrap_or(false) {
                    continue;
                }

                let display_name = user.profile.display_name.as_deref().unwrap_or("");
                let profile_real_name = user.profile.real_name.as_deref().unwrap_or("");
                let real_name = user.real_name.as_deref().unwrap_or("");
                let candidates = [
                    user.name.as_str(),
                    real_name,
                    profile_real_name,
                    display_name,
                ];
                let matched = candidates.iter().any(|candidate| {
                    let candidate = candidate.trim();
                    !candidate.is_empty() && candidate.to_lowercase() == lowered
                });
                if matched {
                    matches.push(user);
                }
            }

            cursor = parsed
                .response_metadata
                .next_cursor
                .filter(|cursor| !cursor.is_empty());
            if cursor.is_none() {
                break;
            }
        }

        let user = matches.into_iter().next().ok_or_else(|| {
            JsonRpcError::internal_error(format!("no Slack user found for {name}"))
        })?;

        Ok(SlackLookupUserResult {
            id: user.id,
            name: user.name,
            real_name: user.real_name.or(user.profile.real_name),
            email: user.profile.email,
            matched_by: "name".to_string(),
        })
    }

    fn get_thread(&self, channel: &str, thread_ts: &str) -> Result<Vec<Value>, JsonRpcError> {
        let headers = self.auth_headers()?;

        let mut all_messages = Vec::new();
        let mut seen_ts = HashSet::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut request = self
                .client
                .get("https://slack.com/api/conversations.replies")
                .headers(headers.clone())
                .query(&[("channel", channel), ("ts", thread_ts), ("limit", "1000")]);
            if let Some(cursor) = cursor.as_deref() {
                request = request.query(&[("cursor", cursor)]);
            }

            let response = request.send().map_err(|err| {
                JsonRpcError::internal_error(format!("slack request failed: {err}"))
            })?;

            let parsed: SlackThreadResponse = response.json().map_err(|err| {
                JsonRpcError::internal_error(format!("invalid slack response: {err}"))
            })?;

            if !parsed.ok {
                let error = parsed.error.as_deref().unwrap_or("unknown");
                return Err(JsonRpcError::internal_error(format!(
                    "Slack API error: {error}"
                )));
            }

            for message in parsed.messages {
                let Some(ts) = message.get("ts").and_then(Value::as_str) else {
                    all_messages.push(message);
                    continue;
                };
                if seen_ts.insert(ts.to_string()) {
                    all_messages.push(message);
                }
            }

            cursor = parsed
                .response_metadata
                .next_cursor
                .filter(|cursor| !cursor.is_empty());
            if cursor.is_none() {
                break;
            }
        }

        Ok(all_messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_contains_post_message() {
        let tools = handle_tools_list()["tools"]
            .as_array()
            .expect("tools array");
        assert_eq!(tools.len(), 7);
        assert_eq!(tools[0]["name"], "slack.post_message");
        assert_eq!(
            tools[0]["inputSchema"]["required"],
            serde_json::json!(["channel", "text"])
        );
        assert_eq!(tools[1]["name"], "slack_reply");
        assert_eq!(
            tools[1]["inputSchema"]["required"],
            serde_json::json!(["channel", "thread_ts", "text"])
        );
        assert_eq!(tools[2]["name"], "slack_get_thread");
        assert_eq!(
            tools[2]["inputSchema"]["required"],
            serde_json::json!(["channel", "thread_ts"])
        );
        assert_eq!(tools[3]["name"], "slack_react");
        assert_eq!(
            tools[3]["inputSchema"]["required"],
            serde_json::json!(["channel", "ts", "emoji"])
        );
        assert_eq!(tools[4]["name"], "slack.list_channels");
        assert_eq!(tools[5]["name"], "slack_lookup_user");
        assert_eq!(
            tools[5]["inputSchema"]["required"],
            serde_json::json!(["email_or_name"])
        );
        assert_eq!(tools[6]["name"], "slack_dm");
        assert_eq!(
            tools[6]["inputSchema"]["required"],
            serde_json::json!(["user_id", "text"])
        );
    }

    #[test]
    fn dispatch_rejects_unknown_tool() {
        let err = dispatch_tool_call("slack.unknown", serde_json::json!({}))
            .expect_err("unknown tool should fail");
        assert!(err.message.contains("unknown tool"));
    }
}
