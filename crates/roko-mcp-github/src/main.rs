//! MCP server stdio transport for `roko-mcp-github`.
//!
//! This module implements the JSON-RPC 2.0 framing layer used by MCP
//! servers: read line-delimited JSON from stdin and write line-delimited
//! JSON responses to stdout.

use anyhow::Context;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
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

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default = "empty_json_object")]
    arguments: Value,
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
    const INTERNAL_ERROR: i64 = -32603;

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

    fn invalid_params(message: impl Into<String>) -> Self {
        Self::invalid_request(message)
    }

    fn internal_error(message: impl Into<String>) -> Self {
        Self {
            code: Self::INTERNAL_ERROR,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ListPrsArguments {
    owner: String,
    repo: String,
    #[serde(default)]
    state: Option<PullRequestState>,
    #[serde(default)]
    head: Option<String>,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    per_page: Option<u32>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PullRequestState {
    Open,
    Closed,
    All,
}

impl PullRequestState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Deserialize)]
struct GithubPullRequest {
    title: String,
    number: u64,
    #[serde(default)]
    user: Option<GithubUser>,
    #[serde(default)]
    labels: Vec<GithubLabel>,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GithubLabel {
    name: String,
}

fn main() -> anyhow::Result<()> {
    serve_stdio(io::stdin().lock(), io::stdout().lock(), handle_request)
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

fn handle_tools_call(params: Value) -> Result<Value, JsonRpcError> {
    let params: ToolsCallParams = serde_json::from_value(params)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid tools/call params: {err}")))?;
    dispatch_tool_call(&params.name, params.arguments)
}

fn dispatch_tool_call(name: &str, arguments: Value) -> Result<Value, JsonRpcError> {
    match name {
        "github.list_prs" => handle_list_prs(arguments),
        "github.get_pr" => handle_get_pr(arguments),
        "github.create_pr" => handle_create_pr(arguments),
        "github.comment_pr" => handle_comment_pr(arguments),
        "github.review_pr" => handle_review_pr(arguments),
        "github.merge_pr" => handle_merge_pr(arguments),
        "github.list_issues" => handle_list_issues(arguments),
        "github.create_issue" => handle_create_issue(arguments),
        "github.comment_issue" => handle_comment_issue(arguments),
        "github.close_issue" => handle_close_issue(arguments),
        "github.add_labels" => handle_add_labels(arguments),
        "github.create_label" => handle_create_label(arguments),
        "github.get_file" => handle_get_file(arguments),
        "github.search_code" => handle_search_code(arguments),
        "github.list_commits" => handle_list_commits(arguments),
        "github.create_branch" => handle_create_branch(arguments),
        "github.get_branch" => handle_get_branch(arguments),
        "github.compare_branches" => handle_compare_branches(arguments),
        "github.get_actions_status" => handle_get_actions_status(arguments),
        _ => Err(JsonRpcError::invalid_params(format!("unknown tool: {name}"))),
    }
}

fn empty_json_object() -> Value {
    Value::Object(Default::default())
}

fn unsupported_tool(name: &str) -> Result<Value, JsonRpcError> {
    Err(JsonRpcError::invalid_params(format!(
        "tool handler `{name}` is not implemented yet"
    )))
}

fn handle_list_prs(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: ListPrsArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid github.list_prs args: {err}")))?;
    let client = github_client()?;
    let prs = list_pull_requests(&client, &args)?;
    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": summarize_pull_requests(&prs).to_string()
        }],
        "isError": false
    }))
}

fn handle_get_pr(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.get_pr")
}

fn handle_create_pr(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.create_pr")
}

fn handle_comment_pr(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.comment_pr")
}

fn handle_review_pr(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.review_pr")
}

fn handle_merge_pr(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.merge_pr")
}

fn handle_list_issues(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.list_issues")
}

fn handle_create_issue(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.create_issue")
}

fn handle_comment_issue(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.comment_issue")
}

fn handle_close_issue(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.close_issue")
}

fn handle_add_labels(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.add_labels")
}

fn handle_create_label(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.create_label")
}

fn handle_get_file(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.get_file")
}

fn handle_search_code(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.search_code")
}

fn handle_list_commits(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.list_commits")
}

fn handle_create_branch(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.create_branch")
}

fn handle_get_branch(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.get_branch")
}

fn handle_compare_branches(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.compare_branches")
}

fn handle_get_actions_status(arguments: Value) -> Result<Value, JsonRpcError> {
    let _ = arguments;
    unsupported_tool("github.get_actions_status")
}

fn github_tool(name: &str, description: &str, input_schema: Value) -> Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

fn github_client() -> Result<Client, JsonRpcError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("roko-mcp-github/0.1"),
    );

    Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|err| JsonRpcError::internal_error(format!("build GitHub client: {err}")))
}

fn github_token() -> Option<String> {
    env::var("GITHUB_TOKEN")
        .ok()
        .filter(|token| !token.is_empty())
        .or_else(|| env::var("GH_TOKEN").ok().filter(|token| !token.is_empty()))
}

fn list_pull_requests(
    client: &Client,
    args: &ListPrsArguments,
) -> Result<Vec<GithubPullRequest>, JsonRpcError> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/pulls",
        args.owner, args.repo
    );
    let mut request = client.get(url);
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }

    let mut query: Vec<(&str, String)> = Vec::with_capacity(4);
    query.push(("state", args.state.unwrap_or(PullRequestState::Open).as_str().to_string()));
    if let Some(head) = &args.head {
        query.push(("head", head.clone()));
    }
    if let Some(base) = &args.base {
        query.push(("base", base.clone()));
    }
    query.push(("per_page", args.per_page.unwrap_or(30).clamp(1, 100).to_string()));

    let response = request
        .query(&query)
        .send()
        .map_err(|err| JsonRpcError::internal_error(format!("call GitHub API: {err}")))?;

    let status = response.status();
    let body = response
        .text()
        .map_err(|err| JsonRpcError::internal_error(format!("read GitHub response: {err}")))?;
    if !status.is_success() {
        return Err(JsonRpcError::internal_error(format!(
            "GitHub API returned {status}: {}",
            body.trim()
        )));
    }

    serde_json::from_str(&body)
        .map_err(|err| JsonRpcError::internal_error(format!("parse GitHub pull requests: {err}")))
}

fn summarize_pull_requests(prs: &[GithubPullRequest]) -> Value {
    serde_json::json!({
        "pull_requests": prs.iter().map(|pr| {
            serde_json::json!({
                "title": pr.title.clone(),
                "number": pr.number,
                "author": pr.user.as_ref().map(|user| user.login.clone()),
                "labels": pr.labels.iter().map(|label| label.name.clone()).collect::<Vec<_>>()
            })
        }).collect::<Vec<_>>()
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

    #[test]
    fn tools_call_dispatches_known_tool_names() {
        let err = handle_tools_call(json!({
            "name": "github.get_pr",
            "arguments": {
                "owner": "octo",
                "repo": "hello-world",
                "number": 1
            }
        }))
        .expect_err("tool is not wired yet");

        assert!(
            err.message.contains("github.get_pr"),
            "expected error to mention dispatched tool name"
        );
    }

    #[test]
    fn tools_call_rejects_unknown_tool_names() {
        let err = handle_tools_call(json!({
            "name": "github.not_real",
            "arguments": {}
        }))
        .expect_err("unknown tool should fail");

        assert_eq!(err.code, JsonRpcError::INVALID_REQUEST);
        assert!(err.message.contains("unknown tool"));
    }

    #[test]
    fn summarize_pull_requests_extracts_expected_fields() {
        let prs = vec![
            GithubPullRequest {
                title: "Fix login flow".to_string(),
                number: 17,
                user: Some(GithubUser {
                    login: "octocat".to_string(),
                }),
                labels: vec![
                    GithubLabel {
                        name: "bug".to_string(),
                    },
                    GithubLabel {
                        name: "priority".to_string(),
                    },
                ],
            },
            GithubPullRequest {
                title: "Update docs".to_string(),
                number: 18,
                user: None,
                labels: vec![],
            },
        ];

        let summary = summarize_pull_requests(&prs);

        assert_eq!(
            summary,
            json!({
                "pull_requests": [
                    {
                        "title": "Fix login flow",
                        "number": 17,
                        "author": "octocat",
                        "labels": ["bug", "priority"]
                    },
                    {
                        "title": "Update docs",
                        "number": 18,
                        "author": null,
                        "labels": []
                    }
                ]
            })
        );
    }
}
