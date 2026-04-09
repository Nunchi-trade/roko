//! MCP server stdio transport for `roko-mcp-github`.
//!
//! This module implements the JSON-RPC 2.0 framing layer used by MCP
//! servers: read line-delimited JSON from stdin and write line-delimited
//! JSON responses to stdout.

use anyhow::Context;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
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

#[derive(Debug, Deserialize)]
struct ListIssuesArguments {
    owner: String,
    repo: String,
    #[serde(default)]
    state: Option<IssueState>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    #[serde(default)]
    assignee: Option<String>,
    #[serde(default)]
    per_page: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GetPrArguments {
    owner: String,
    repo: String,
    number: u64,
}

#[derive(Debug, Deserialize)]
struct CreatePrArguments {
    owner: String,
    repo: String,
    title: String,
    body: String,
    head: String,
    base: String,
    #[serde(default)]
    draft: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CommentPrArguments {
    owner: String,
    repo: String,
    number: u64,
    body: String,
}

#[derive(Debug, Deserialize)]
struct ReviewPrArguments {
    owner: String,
    repo: String,
    number: u64,
    body: String,
    event: GithubReviewEvent,
}

#[derive(Debug, Deserialize)]
struct MergePrArguments {
    owner: String,
    repo: String,
    number: u64,
    merge_method: MergeMethod,
    #[serde(default)]
    commit_title: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PullRequestState {
    Open,
    Closed,
    All,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum IssueState {
    Open,
    Closed,
    All,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GithubReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl GithubReviewEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "APPROVE",
            Self::RequestChanges => "REQUEST_CHANGES",
            Self::Comment => "COMMENT",
        }
    }
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

impl IssueState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

impl MergeMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
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

#[derive(Debug, Deserialize)]
struct GithubIssue {
    number: u64,
    title: String,
    state: String,
    #[serde(default)]
    labels: Vec<GithubLabel>,
    #[serde(default)]
    assignee: Option<GithubUser>,
    created_at: Option<String>,
    #[serde(default)]
    pull_request: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct GithubRepositoryRef {
    full_name: String,
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubBranchRef {
    label: String,
    #[serde(rename = "ref")]
    ref_name: String,
    sha: String,
    #[serde(default)]
    user: Option<GithubUser>,
    #[serde(default)]
    repo: Option<GithubRepositoryRef>,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestDetails {
    url: String,
    html_url: Option<String>,
    diff_url: Option<String>,
    patch_url: Option<String>,
    issue_url: Option<String>,
    number: u64,
    state: String,
    title: String,
    body: Option<String>,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    merged: bool,
    merged_at: Option<String>,
    merge_commit_sha: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    closed_at: Option<String>,
    additions: u64,
    deletions: u64,
    changed_files: u64,
    commits: u64,
    comments: u64,
    review_comments: u64,
    #[serde(default)]
    maintainer_can_modify: bool,
    mergeable: Option<bool>,
    mergeable_state: Option<String>,
    #[serde(default)]
    user: Option<GithubUser>,
    #[serde(default)]
    labels: Vec<GithubLabel>,
    #[serde(default)]
    assignees: Vec<GithubUser>,
    #[serde(default)]
    requested_reviewers: Vec<GithubUser>,
    head: GithubBranchRef,
    base: GithubBranchRef,
}

#[derive(Debug, Deserialize)]
struct GithubCreatePullRequestResponse {
    number: u64,
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubIssueComment {
    id: u64,
    html_url: Option<String>,
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubPullRequestReview {
    id: u64,
    state: GithubReviewState,
    body: Option<String>,
    submitted_at: Option<String>,
    commit_id: Option<String>,
    html_url: Option<String>,
    #[serde(default)]
    user: Option<GithubUser>,
}

#[derive(Debug, Deserialize)]
struct GithubMergePullRequestResponse {
    merged: bool,
    sha: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GithubReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

impl GithubReviewState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "APPROVED",
            Self::ChangesRequested => "CHANGES_REQUESTED",
            Self::Commented => "COMMENTED",
            Self::Dismissed => "DISMISSED",
            Self::Pending => "PENDING",
        }
    }
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
                "Get a pull request with diff stats and review summary.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": {"type": "string"},
                        "repo": {"type": "string"},
                        "number": {"type": "integer", "minimum": 1}
                    },
                    "required": ["owner", "repo", "number", "merge_method"],
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
        _ => Err(JsonRpcError::invalid_params(format!(
            "unknown tool: {name}"
        ))),
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
    let args: ListPrsArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid github.list_prs args: {err}"))
    })?;
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
    let args: GetPrArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid github.get_pr args: {err}"))
    })?;
    let client = github_client()?;
    let pr = get_pull_request(&client, &args.owner, &args.repo, args.number)?;
    let reviews = list_pull_request_reviews(&client, &args.owner, &args.repo, args.number)?;
    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": summarize_pull_request(&pr, &reviews).to_string()
        }],
        "isError": false
    }))
}

fn handle_create_pr(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: CreatePrArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid github.create_pr args: {err}"))
    })?;
    let client = github_client()?;
    let pr = create_pull_request(&client, &args, "https://api.github.com")?;
    let html_url = pr
        .html_url
        .ok_or_else(|| JsonRpcError::internal_error("GitHub API response missing html_url"))?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "number": pr.number,
                "html_url": html_url
            }).to_string()
        }],
        "isError": false
    }))
}

fn handle_comment_pr(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: CommentPrArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid github.comment_pr args: {err}"))
    })?;
    let client = github_client()?;
    let comment = create_pull_request_comment(&client, &args, "https://api.github.com")?;
    let html_url = comment
        .html_url
        .ok_or_else(|| JsonRpcError::internal_error("GitHub API response missing html_url"))?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "id": comment.id,
                "html_url": html_url,
                "body": comment.body
            }).to_string()
        }],
        "isError": false
    }))
}

fn handle_review_pr(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: ReviewPrArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid github.review_pr args: {err}"))
    })?;
    let client = github_client()?;
    let review = submit_pull_request_review(&client, &args, "https://api.github.com")?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "id": review.id,
                "state": review.state.as_str(),
                "body": review.body,
                "submitted_at": review.submitted_at,
                "html_url": review.html_url,
                "commit_id": review.commit_id,
                "author": review.user.as_ref().map(|user| user.login.clone())
            }).to_string()
        }],
        "isError": false
    }))
}

fn handle_merge_pr(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: MergePrArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid github.merge_pr args: {err}"))
    })?;
    let client = github_client()?;
    let merge = merge_pull_request(&client, &args, "https://api.github.com")?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::json!({
                "merged": merge.merged,
                "sha": merge.sha,
                "message": merge.message
            }).to_string()
        }],
        "isError": false
    }))
}

fn handle_list_issues(arguments: Value) -> Result<Value, JsonRpcError> {
    let args: ListIssuesArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid github.list_issues args: {err}"))
    })?;
    let client = github_client()?;
    let issues = list_issues(&client, &args, "https://api.github.com")?;
    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": summarize_issues(&issues).to_string()
        }],
        "isError": false
    }))
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
    headers.insert(USER_AGENT, HeaderValue::from_static("roko-mcp-github/0.1"));

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
    query.push((
        "state",
        args.state
            .unwrap_or(PullRequestState::Open)
            .as_str()
            .to_string(),
    ));
    if let Some(head) = &args.head {
        query.push(("head", head.clone()));
    }
    if let Some(base) = &args.base {
        query.push(("base", base.clone()));
    }
    query.push((
        "per_page",
        args.per_page.unwrap_or(30).clamp(1, 100).to_string(),
    ));

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

fn list_issues(
    client: &Client,
    args: &ListIssuesArguments,
    api_base_url: &str,
) -> Result<Vec<GithubIssue>, JsonRpcError> {
    let url = format!("{api_base_url}/repos/{}/{}/issues", args.owner, args.repo);
    let mut request = client.get(url);
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }

    let mut query: Vec<(&str, String)> = Vec::with_capacity(5);
    query.push((
        "state",
        args.state.unwrap_or(IssueState::Open).as_str().to_string(),
    ));
    if let Some(labels) = &args.labels
        && !labels.is_empty()
    {
        query.push(("labels", labels.join(",")));
    }
    if let Some(assignee) = &args.assignee
        && !assignee.is_empty()
    {
        query.push(("assignee", assignee.clone()));
    }
    query.push((
        "per_page",
        args.per_page.unwrap_or(30).clamp(1, 100).to_string(),
    ));

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

    let issues: Vec<GithubIssue> = serde_json::from_str(&body)
        .map_err(|err| JsonRpcError::internal_error(format!("parse GitHub issues: {err}")))?;
    Ok(issues
        .into_iter()
        .filter(|issue| issue.pull_request.is_none())
        .collect())
}

fn get_pull_request(
    client: &Client,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<GithubPullRequestDetails, JsonRpcError> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
    let mut request = client.get(url);
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }

    let response = request
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
        .map_err(|err| JsonRpcError::internal_error(format!("parse GitHub pull request: {err}")))
}

fn list_pull_request_reviews(
    client: &Client,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<Vec<GithubPullRequestReview>, JsonRpcError> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/reviews");
    let mut request = client.get(url);
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }

    let response = request
        .query(&[("per_page", "100")])
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

    serde_json::from_str(&body).map_err(|err| {
        JsonRpcError::internal_error(format!("parse GitHub pull request reviews: {err}"))
    })
}

fn create_pull_request(
    client: &Client,
    args: &CreatePrArguments,
    api_base_url: &str,
) -> Result<GithubCreatePullRequestResponse, JsonRpcError> {
    let url = format!("{api_base_url}/repos/{}/{}/pulls", args.owner, args.repo);
    let mut request = client.post(url);
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }

    let mut payload = serde_json::json!({
        "title": args.title,
        "body": args.body,
        "head": args.head,
        "base": args.base,
    });
    if let Some(draft) = args.draft {
        payload["draft"] = Value::Bool(draft);
    }

    let response = request
        .json(&payload)
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

    serde_json::from_str(&body).map_err(|err| {
        JsonRpcError::internal_error(format!(
            "parse GitHub pull request creation response: {err}"
        ))
    })
}

fn create_pull_request_comment(
    client: &Client,
    args: &CommentPrArguments,
    api_base_url: &str,
) -> Result<GithubIssueComment, JsonRpcError> {
    let url = format!("{api_base_url}/repos/{}/{}/issues/{}/comments", args.owner, args.repo, args.number);
    let mut request = client.post(url);
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }

    let payload = serde_json::json!({
        "body": args.body,
    });

    let response = request
        .json(&payload)
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

    serde_json::from_str(&body).map_err(|err| {
        JsonRpcError::internal_error(format!("parse GitHub issue comment response: {err}"))
    })
}

fn submit_pull_request_review(
    client: &Client,
    args: &ReviewPrArguments,
    api_base_url: &str,
) -> Result<GithubPullRequestReview, JsonRpcError> {
    let url = format!(
        "{api_base_url}/repos/{}/{}/pulls/{}/reviews",
        args.owner, args.repo, args.number
    );
    let mut request = client.post(url);
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }

    let payload = serde_json::json!({
        "body": args.body,
        "event": args.event.as_str(),
    });

    let response = request
        .json(&payload)
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

    serde_json::from_str(&body).map_err(|err| {
        JsonRpcError::internal_error(format!("parse GitHub pull request review response: {err}"))
    })
}

fn merge_pull_request(
    client: &Client,
    args: &MergePrArguments,
    api_base_url: &str,
) -> Result<GithubMergePullRequestResponse, JsonRpcError> {
    let url = format!(
        "{api_base_url}/repos/{}/{}/pulls/{}/merge",
        args.owner, args.repo, args.number
    );
    let mut request = client.put(url);
    if let Some(token) = github_token() {
        request = request.bearer_auth(token);
    }

    let mut payload = serde_json::json!({
        "merge_method": args.merge_method.as_str(),
    });
    if let Some(commit_title) = &args.commit_title {
        payload["commit_title"] = Value::String(commit_title.clone());
    }

    let response = request
        .json(&payload)
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

    serde_json::from_str(&body).map_err(|err| {
        JsonRpcError::internal_error(format!("parse GitHub pull request merge response: {err}"))
    })
}

fn summarize_pull_request(
    pr: &GithubPullRequestDetails,
    reviews: &[GithubPullRequestReview],
) -> Value {
    let latest_review_state = reviews
        .iter()
        .filter_map(|review| {
            review
                .submitted_at
                .as_ref()
                .map(|submitted_at| (submitted_at.as_str(), review.state.as_str()))
        })
        .max_by_key(|(submitted_at, _)| *submitted_at)
        .map(|(_, state)| state);

    let mut review_counts = serde_json::Map::new();
    for state in [
        GithubReviewState::Approved,
        GithubReviewState::ChangesRequested,
        GithubReviewState::Commented,
        GithubReviewState::Dismissed,
        GithubReviewState::Pending,
    ] {
        let count = reviews
            .iter()
            .filter(|review| review.state == state)
            .count();
        review_counts.insert(state.as_str().to_string(), Value::from(count as u64));
    }

    serde_json::json!({
        "pull_request": {
            "number": pr.number,
            "title": pr.title.clone(),
            "body": pr.body.clone(),
            "state": pr.state.clone(),
            "draft": pr.draft,
            "locked": pr.locked,
            "merged": pr.merged,
            "merged_at": pr.merged_at.clone(),
            "merge_commit_sha": pr.merge_commit_sha.clone(),
            "created_at": pr.created_at.clone(),
            "updated_at": pr.updated_at.clone(),
            "closed_at": pr.closed_at.clone(),
            "url": pr.url.clone(),
            "html_url": pr.html_url.clone(),
            "diff_url": pr.diff_url.clone(),
            "patch_url": pr.patch_url.clone(),
            "issue_url": pr.issue_url.clone(),
            "author": pr.user.as_ref().map(|user| user.login.clone()),
            "labels": pr.labels.iter().map(|label| label.name.clone()).collect::<Vec<_>>(),
            "assignees": pr.assignees.iter().map(|user| user.login.clone()).collect::<Vec<_>>(),
            "requested_reviewers": pr.requested_reviewers.iter().map(|user| user.login.clone()).collect::<Vec<_>>(),
            "head": {
                "label": pr.head.label.clone(),
                "ref": pr.head.ref_name.clone(),
                "sha": pr.head.sha.clone(),
                "repo": pr.head.repo.as_ref().map(|repo| serde_json::json!({
                    "full_name": repo.full_name.clone(),
                    "html_url": repo.html_url.clone()
                })),
                "author": pr.head.user.as_ref().map(|user| user.login.clone())
            },
            "base": {
                "label": pr.base.label.clone(),
                "ref": pr.base.ref_name.clone(),
                "sha": pr.base.sha.clone(),
                "repo": pr.base.repo.as_ref().map(|repo| serde_json::json!({
                    "full_name": repo.full_name.clone(),
                    "html_url": repo.html_url.clone()
                })),
                "author": pr.base.user.as_ref().map(|user| user.login.clone())
            },
            "diff_stats": {
                "additions": pr.additions,
                "deletions": pr.deletions,
                "changed_files": pr.changed_files,
                "commits": pr.commits,
                "comments": pr.comments,
                "review_comments": pr.review_comments
            },
            "mergeability": {
                "mergeable": pr.mergeable,
                "mergeable_state": pr.mergeable_state,
                "maintainer_can_modify": pr.maintainer_can_modify
            },
            "review_state": {
                "latest": latest_review_state,
                "counts": review_counts,
                "reviews": reviews.iter().map(|review| serde_json::json!({
                    "id": review.id,
                    "state": review.state.as_str(),
                    "body": review.body,
                    "submitted_at": review.submitted_at,
                    "commit_id": review.commit_id,
                    "html_url": review.html_url,
                    "author": review.user.as_ref().map(|user| user.login.clone())
                })).collect::<Vec<_>>()
            }
        }
    })
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

fn summarize_issues(issues: &[GithubIssue]) -> Value {
    serde_json::json!({
        "issues": issues.iter().map(|issue| {
            serde_json::json!({
                "number": issue.number,
                "title": issue.title.clone(),
                "state": issue.state.clone(),
                "labels": issue.labels.iter().map(|label| label.name.clone()).collect::<Vec<_>>(),
                "assignee": issue.assignee.as_ref().map(|user| user.login.clone()),
                "created_at": issue.created_at.clone()
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
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

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
        assert_eq!(
            get_pr["description"],
            "Get a pull request with diff stats and review summary."
        );
        assert_eq!(
            get_pr["inputSchema"]["required"],
            json!(["owner", "repo", "number"])
        );
        assert!(get_pr["inputSchema"]["properties"]
            .get("include_diff")
            .is_none());
    }

    #[test]
    fn summarize_pull_request_includes_diff_stats_and_review_state() {
        let pr = GithubPullRequestDetails {
            url: "https://api.github.com/repos/octo/hello-world/pulls/17".to_string(),
            html_url: Some("https://github.com/octo/hello-world/pull/17".to_string()),
            diff_url: Some("https://github.com/octo/hello-world/pull/17.diff".to_string()),
            patch_url: Some("https://github.com/octo/hello-world/pull/17.patch".to_string()),
            issue_url: Some("https://api.github.com/repos/octo/hello-world/issues/17".to_string()),
            number: 17,
            state: "open".to_string(),
            title: "Fix login flow".to_string(),
            body: Some("This fixes the login redirect.".to_string()),
            locked: false,
            draft: false,
            merged: false,
            merged_at: None,
            merge_commit_sha: None,
            created_at: Some("2026-04-08T10:00:00Z".to_string()),
            updated_at: Some("2026-04-08T12:00:00Z".to_string()),
            closed_at: None,
            additions: 12,
            deletions: 3,
            changed_files: 2,
            commits: 4,
            comments: 1,
            review_comments: 5,
            maintainer_can_modify: true,
            mergeable: Some(true),
            mergeable_state: Some("clean".to_string()),
            user: Some(GithubUser {
                login: "octocat".to_string(),
            }),
            labels: vec![GithubLabel {
                name: "bug".to_string(),
            }],
            assignees: vec![GithubUser {
                login: "maintainer".to_string(),
            }],
            requested_reviewers: vec![GithubUser {
                login: "reviewer".to_string(),
            }],
            head: GithubBranchRef {
                label: "octo:feature/login-fix".to_string(),
                ref_name: "feature/login-fix".to_string(),
                sha: "abc123".to_string(),
                user: Some(GithubUser {
                    login: "octo".to_string(),
                }),
                repo: Some(GithubRepositoryRef {
                    full_name: "octo/hello-world".to_string(),
                    html_url: Some("https://github.com/octo/hello-world".to_string()),
                }),
            },
            base: GithubBranchRef {
                label: "octo:main".to_string(),
                ref_name: "main".to_string(),
                sha: "def456".to_string(),
                user: Some(GithubUser {
                    login: "octo".to_string(),
                }),
                repo: Some(GithubRepositoryRef {
                    full_name: "octo/hello-world".to_string(),
                    html_url: Some("https://github.com/octo/hello-world".to_string()),
                }),
            },
        };
        let reviews = vec![
            GithubPullRequestReview {
                id: 1,
                state: GithubReviewState::Approved,
                body: Some("Looks good".to_string()),
                submitted_at: Some("2026-04-08T11:00:00Z".to_string()),
                commit_id: Some("abc123".to_string()),
                html_url: Some("https://github.com/octo/hello-world/pull/17#review-1".to_string()),
                user: Some(GithubUser {
                    login: "reviewer".to_string(),
                }),
            },
            GithubPullRequestReview {
                id: 2,
                state: GithubReviewState::Commented,
                body: Some("One note".to_string()),
                submitted_at: Some("2026-04-08T13:00:00Z".to_string()),
                commit_id: Some("abc123".to_string()),
                html_url: Some("https://github.com/octo/hello-world/pull/17#review-2".to_string()),
                user: Some(GithubUser {
                    login: "reviewer".to_string(),
                }),
            },
        ];

        let summary = summarize_pull_request(&pr, &reviews);

        assert_eq!(summary["pull_request"]["number"], 17);
        assert_eq!(summary["pull_request"]["diff_stats"]["additions"], 12);
        assert_eq!(summary["pull_request"]["diff_stats"]["changed_files"], 2);
        assert_eq!(
            summary["pull_request"]["review_state"]["latest"],
            "COMMENTED"
        );
        assert_eq!(
            summary["pull_request"]["review_state"]["counts"]["APPROVED"],
            1
        );
        assert_eq!(
            summary["pull_request"]["review_state"]["counts"]["COMMENTED"],
            1
        );
        assert_eq!(
            summary["pull_request"]["head"]["repo"]["full_name"],
            "octo/hello-world"
        );
    }

    #[test]
    fn tools_call_rejects_missing_get_pr_args() {
        let err = handle_tools_call(json!({
            "name": "github.get_pr",
            "arguments": {
                "owner": "octo",
                "repo": "hello-world"
            }
        }))
        .expect_err("missing number should fail");

        assert_eq!(err.code, JsonRpcError::INVALID_REQUEST);
        assert!(err.message.contains("github.get_pr"));
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

    #[test]
    fn summarize_issues_extracts_expected_fields() {
        let issues = vec![
            GithubIssue {
                number: 101,
                title: "Bug: login redirect".to_string(),
                state: "open".to_string(),
                labels: vec![
                    GithubLabel {
                        name: "bug".to_string(),
                    },
                    GithubLabel {
                        name: "urgent".to_string(),
                    },
                ],
                assignee: Some(GithubUser {
                    login: "octocat".to_string(),
                }),
                created_at: Some("2026-04-08T09:00:00Z".to_string()),
                pull_request: None,
            },
            GithubIssue {
                number: 102,
                title: "Add docs".to_string(),
                state: "closed".to_string(),
                labels: vec![],
                assignee: None,
                created_at: None,
                pull_request: None,
            },
        ];

        let summary = summarize_issues(&issues);

        assert_eq!(
            summary,
            json!({
                "issues": [
                    {
                        "number": 101,
                        "title": "Bug: login redirect",
                        "state": "open",
                        "labels": ["bug", "urgent"],
                        "assignee": "octocat",
                        "created_at": "2026-04-08T09:00:00Z"
                    },
                    {
                        "number": 102,
                        "title": "Add docs",
                        "state": "closed",
                        "labels": [],
                        "assignee": null,
                        "created_at": null
                    }
                ]
            })
        );
    }

    #[test]
    fn list_issues_queries_expected_params_and_filters_pull_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept request");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .expect("read request line");
            assert!(request_line.starts_with("GET /repos/octo/hello-world/issues?"));

            let mut saw_headers = false;
            loop {
                let mut header_line = String::new();
                reader.read_line(&mut header_line).expect("read header");
                let header = header_line.trim_end();
                if header.is_empty() {
                    saw_headers = true;
                    break;
                }
            }
            assert!(saw_headers);

            let mut writer = stream;
            let response_body = json!([
                {
                    "number": 101,
                    "title": "Bug: login redirect",
                    "state": "open",
                    "labels": [
                        {"name": "bug"},
                        {"name": "urgent"}
                    ],
                    "assignee": {"login": "octocat"},
                    "created_at": "2026-04-08T09:00:00Z"
                },
                {
                    "number": 202,
                    "title": "Actually a pull request",
                    "state": "open",
                    "labels": [],
                    "created_at": "2026-04-08T10:00:00Z",
                    "pull_request": {}
                }
            ])
            .to_string();
            write!(
                writer,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .expect("write response");
        });

        let client = github_client().expect("client");
        let args = ListIssuesArguments {
            owner: "octo".to_string(),
            repo: "hello-world".to_string(),
            state: Some(IssueState::Open),
            labels: Some(vec!["bug".to_string(), "urgent".to_string()]),
            assignee: Some("octocat".to_string()),
            per_page: Some(50),
        };

        let issues = list_issues(&client, &args, &format!("http://{}", addr)).expect("list issues");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number, 101);
        assert_eq!(issues[0].title, "Bug: login redirect");
        assert_eq!(issues[0].assignee.as_ref().map(|user| user.login.as_str()), Some("octocat"));

        server.join().expect("server thread");
    }

    #[test]
    fn create_pull_request_posts_expected_payload_and_returns_url() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept request");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .expect("read request line");
            assert!(request_line.starts_with("POST /repos/octo/hello-world/pulls HTTP/1.1"));

            let mut content_length = 0usize;
            loop {
                let mut header_line = String::new();
                reader.read_line(&mut header_line).expect("read header");
                let header = header_line.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length: ") {
                    content_length = value.parse().expect("parse content length");
                }
            }

            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).expect("read request body");
            let body_json: Value = serde_json::from_slice(&body).expect("parse request body");
            assert_eq!(
                body_json,
                json!({
                    "title": "Fix login flow",
                    "body": "This fixes the login redirect.",
                    "head": "feature/login-fix",
                    "base": "main",
                    "draft": true
                })
            );

            let mut writer = stream;
            let response_body = json!({
                "number": 17,
                "html_url": "https://github.com/octo/hello-world/pull/17"
            })
            .to_string();
            write!(
                writer,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .expect("write response");
        });

        let client = github_client().expect("client");
        let args = CreatePrArguments {
            owner: "octo".to_string(),
            repo: "hello-world".to_string(),
            title: "Fix login flow".to_string(),
            body: "This fixes the login redirect.".to_string(),
            head: "feature/login-fix".to_string(),
            base: "main".to_string(),
            draft: Some(true),
        };

        let pr =
            create_pull_request(&client, &args, &format!("http://{}", addr)).expect("create pr");
        assert_eq!(pr.number, 17);
        assert_eq!(
            pr.html_url.as_deref(),
            Some("https://github.com/octo/hello-world/pull/17")
        );

        server.join().expect("server thread");
    }

    #[test]
    fn submit_pull_request_review_posts_expected_payload_and_returns_review() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept request");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .expect("read request line");
            assert!(
                request_line.starts_with("POST /repos/octo/hello-world/pulls/17/reviews HTTP/1.1")
            );

            let mut content_length = 0usize;
            loop {
                let mut header_line = String::new();
                reader.read_line(&mut header_line).expect("read header");
                let header = header_line.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length: ") {
                    content_length = value.parse().expect("parse content length");
                }
            }

            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).expect("read request body");
            let body_json: Value = serde_json::from_slice(&body).expect("parse request body");
            assert_eq!(
                body_json,
                json!({
                    "body": "Looks good to me.",
                    "event": "APPROVE"
                })
            );

            let mut writer = stream;
            let response_body = json!({
                "id": 88,
                "state": "APPROVED",
                "body": "Looks good to me.",
                "submitted_at": "2026-04-08T15:30:00Z",
                "commit_id": "abc123",
                "html_url": "https://github.com/octo/hello-world/pull/17#pullrequestreview-88",
                "user": {
                    "login": "octocat"
                }
            })
            .to_string();
            write!(
                writer,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .expect("write response");
        });

        let client = github_client().expect("client");
        let args = ReviewPrArguments {
            owner: "octo".to_string(),
            repo: "hello-world".to_string(),
            number: 17,
            body: "Looks good to me.".to_string(),
            event: GithubReviewEvent::Approve,
        };

        let review = submit_pull_request_review(&client, &args, &format!("http://{}", addr))
            .expect("submit review");
        assert_eq!(review.id, 88);
        assert_eq!(review.state, GithubReviewState::Approved);
        assert_eq!(review.body.as_deref(), Some("Looks good to me."));
        assert_eq!(
            review.html_url.as_deref(),
            Some("https://github.com/octo/hello-world/pull/17#pullrequestreview-88")
        );

        server.join().expect("server thread");
    }

    #[test]
    fn merge_pull_request_puts_expected_payload_and_returns_merge_result() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept request");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .expect("read request line");
            assert!(
                request_line.starts_with("PUT /repos/octo/hello-world/pulls/17/merge HTTP/1.1")
            );

            let mut content_length = 0usize;
            loop {
                let mut header_line = String::new();
                reader.read_line(&mut header_line).expect("read header");
                let header = header_line.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length: ") {
                    content_length = value.parse().expect("parse content length");
                }
            }

            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).expect("read request body");
            let body_json: Value = serde_json::from_slice(&body).expect("parse request body");
            assert_eq!(
                body_json,
                json!({
                    "merge_method": "squash",
                    "commit_title": "Release v1.2.3"
                })
            );

            let mut writer = stream;
            let response_body = json!({
                "sha": "abc123",
                "merged": true,
                "message": "Pull Request successfully merged"
            })
            .to_string();
            write!(
                writer,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .expect("write response");
        });

        let client = github_client().expect("client");
        let args = MergePrArguments {
            owner: "octo".to_string(),
            repo: "hello-world".to_string(),
            number: 17,
            merge_method: MergeMethod::Squash,
            commit_title: Some("Release v1.2.3".to_string()),
        };

        let merge =
            merge_pull_request(&client, &args, &format!("http://{}", addr)).expect("merge pr");
        assert!(merge.merged);
        assert_eq!(merge.sha.as_deref(), Some("abc123"));
        assert_eq!(
            merge.message.as_deref(),
            Some("Pull Request successfully merged")
        );

        server.join().expect("server thread");
    }
}
