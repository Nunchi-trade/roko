# 10 — roko-mcp-github

> 17 GitHub API tools: PR operations, issue management, repository tools, CI integration.
> Full JSON Schema specifications.


> **Implementation**: Scaffold

---

## Overview

`roko-mcp-github` is an MCP server that exposes 17 GitHub API tools via the Model Context
Protocol. It provides agents with comprehensive GitHub operations — PR review, issue triage,
file management, repository search, and CI status monitoring.

**Status:** Planned (spec complete, implementation pending)

**Crate:** `crates/roko-mcp-github/`

**Protocol:** MCP (JSON-RPC 2.0 over stdio)

**Authentication:** GitHub App installation token or Personal Access Token (PAT)

**Agent templates using this:** pr-review-agent, triage-agent, auto-plan-agent,
code-implementer-agent, gate-fixer-agent, enrich-agent, prd-ingestion-agent,
review-response-agent, action-tracker-agent

---

## The 17 Tools

### Pull Request Tools (6)

#### 1. `github.get_pr`

Get pull request details including diff, comments, and review status.

```json
{
  "name": "get_pr",
  "description": "Get pull request details. Use include_diff:true to see what changed.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string", "description": "owner/repo format" },
      "number": { "type": "integer", "description": "PR number" },
      "include_diff": { "type": "boolean", "default": false, "description": "Include full diff" },
      "include_comments": { "type": "boolean", "default": true, "description": "Include review comments" }
    },
    "required": ["repo", "number"]
  }
}
```

#### 2. `github.create_pr`

Create a new pull request.

```json
{
  "name": "create_pr",
  "description": "Create a pull request. Branch must exist and have commits ahead of base.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "title": { "type": "string", "maxLength": 256 },
      "body": { "type": "string" },
      "head": { "type": "string", "description": "Branch to merge from" },
      "base": { "type": "string", "default": "main", "description": "Branch to merge into" },
      "draft": { "type": "boolean", "default": false }
    },
    "required": ["repo", "title", "head"]
  }
}
```

#### 3. `github.review_pr`

Submit a PR review with optional inline comments.

```json
{
  "name": "review_pr",
  "description": "Submit a review on a PR. Use APPROVE, COMMENT, or REQUEST_CHANGES.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "number": { "type": "integer" },
      "event": { "type": "string", "enum": ["APPROVE", "COMMENT", "REQUEST_CHANGES"] },
      "body": { "type": "string", "description": "Overall review comment" },
      "comments": {
        "type": "array",
        "items": {
          "type": "object",
          "properties": {
            "path": { "type": "string" },
            "line": { "type": "integer" },
            "body": { "type": "string" }
          },
          "required": ["path", "line", "body"]
        }
      }
    },
    "required": ["repo", "number", "event", "body"]
  }
}
```

#### 4. `github.merge_pr`

Merge a pull request.

```json
{
  "name": "merge_pr",
  "description": "Merge a PR. Checks must pass first.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "number": { "type": "integer" },
      "merge_method": { "type": "string", "enum": ["merge", "squash", "rebase"], "default": "squash" },
      "commit_title": { "type": "string" },
      "commit_message": { "type": "string" }
    },
    "required": ["repo", "number"]
  }
}
```

#### 5. `github.update_pr`

Update PR title, body, or base branch.

```json
{
  "name": "update_pr",
  "description": "Update PR metadata (title, body, base branch).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "number": { "type": "integer" },
      "title": { "type": "string" },
      "body": { "type": "string" },
      "base": { "type": "string" }
    },
    "required": ["repo", "number"]
  }
}
```

#### 6. `github.list_prs`

List pull requests with filtering.

```json
{
  "name": "list_prs",
  "description": "List PRs. Filter by state, author, label.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "state": { "type": "string", "enum": ["open", "closed", "all"], "default": "open" },
      "author": { "type": "string" },
      "label": { "type": "string" },
      "limit": { "type": "integer", "default": 30, "maximum": 100 }
    },
    "required": ["repo"]
  }
}
```

### Issue Tools (6)

#### 7. `github.get_issue`

Get issue details including comments.

```json
{
  "name": "get_issue",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "number": { "type": "integer" },
      "include_comments": { "type": "boolean", "default": true }
    },
    "required": ["repo", "number"]
  }
}
```

#### 8. `github.create_issue`

Create a new issue.

```json
{
  "name": "create_issue",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "title": { "type": "string" },
      "body": { "type": "string" },
      "labels": { "type": "array", "items": { "type": "string" } },
      "assignees": { "type": "array", "items": { "type": "string" } }
    },
    "required": ["repo", "title"]
  }
}
```

#### 9. `github.update_issue`

Update issue state, labels, or assignees.

```json
{
  "name": "update_issue",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "number": { "type": "integer" },
      "state": { "type": "string", "enum": ["open", "closed"] },
      "labels": { "type": "array", "items": { "type": "string" } },
      "assignees": { "type": "array", "items": { "type": "string" } },
      "title": { "type": "string" },
      "body": { "type": "string" }
    },
    "required": ["repo", "number"]
  }
}
```

#### 10. `github.list_issues`

List issues with filtering.

```json
{
  "name": "list_issues",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "state": { "type": "string", "enum": ["open", "closed", "all"], "default": "open" },
      "labels": { "type": "string", "description": "Comma-separated label names" },
      "assignee": { "type": "string" },
      "limit": { "type": "integer", "default": 30 }
    },
    "required": ["repo"]
  }
}
```

#### 11. `github.comment_issue`

Add a comment to an issue or PR.

```json
{
  "name": "comment_issue",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "number": { "type": "integer" },
      "body": { "type": "string" }
    },
    "required": ["repo", "number", "body"]
  }
}
```

#### 12. `github.search_issues`

Search issues and PRs across repositories.

```json
{
  "name": "search_issues",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "GitHub search syntax" },
      "limit": { "type": "integer", "default": 30 }
    },
    "required": ["query"]
  }
}
```

### Repository Tools (4)

#### 13. `github.get_file`

Get file contents from a repository.

```json
{
  "name": "get_file",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "path": { "type": "string" },
      "ref": { "type": "string", "default": "main", "description": "Branch, tag, or commit SHA" }
    },
    "required": ["repo", "path"]
  }
}
```

#### 14. `github.search_code`

Search code across repositories.

```json
{
  "name": "search_code",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string" },
      "repo": { "type": "string", "description": "Limit to specific repo" },
      "language": { "type": "string" },
      "limit": { "type": "integer", "default": 30 }
    },
    "required": ["query"]
  }
}
```

#### 15. `github.list_branches`

List repository branches.

```json
{
  "name": "list_branches",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "limit": { "type": "integer", "default": 30 }
    },
    "required": ["repo"]
  }
}
```

#### 16. `github.get_tree`

Get repository file tree.

```json
{
  "name": "get_tree",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "path": { "type": "string", "default": "" },
      "ref": { "type": "string", "default": "main" },
      "recursive": { "type": "boolean", "default": false }
    },
    "required": ["repo"]
  }
}
```

### CI Tools (1)

#### 17. `github.get_check_status`

Get CI/CD check status for a commit or PR.

```json
{
  "name": "get_check_status",
  "inputSchema": {
    "type": "object",
    "properties": {
      "repo": { "type": "string" },
      "ref": { "type": "string", "description": "Commit SHA, branch name, or PR number" }
    },
    "required": ["repo", "ref"]
  }
}
```

---

## Server Configuration

```toml
# roko.toml
[[agent.mcp_servers]]
name = "github"
command = "roko-mcp-github"
args = ["--repo", "nunchi/roko"]
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }
```

The `--repo` argument sets the default repository. All tools accept an explicit `repo`
parameter that overrides the default, enabling cross-repository operations.

---

## Rate Limiting

GitHub API rate limits are handled transparently by the MCP server:

| Auth Method | Rate Limit | Reset Period |
|---|---|---|
| GitHub App | 5,000 requests/hour | Per installation |
| PAT | 5,000 requests/hour | Per token |
| Search API | 30 requests/minute | Per token |

The server implements exponential backoff with jitter for rate-limited responses (HTTP 429).

### Rate limit backoff strategy

```rust
use std::time::Duration;

/// Exponential backoff with full jitter for GitHub API rate limits.
pub struct RateLimitBackoff {
    /// Base delay for first retry.
    base_delay: Duration,
    /// Maximum delay cap.
    max_delay: Duration,
    /// Maximum number of retries before giving up.
    max_retries: u32,
    /// Current retry count.
    current_retry: u32,
}

impl RateLimitBackoff {
    /// Default: 1s base, 60s max, 5 retries.
    pub fn new() -> Self {
        Self {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            max_retries: 5,
            current_retry: 0,
        }
    }

    /// Compute the next backoff delay with full jitter.
    /// Full jitter (AWS recommendation): uniform random in [0, min(cap, base * 2^attempt)].
    /// This distributes retry storms across time.
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.current_retry >= self.max_retries {
            return None; // Give up.
        }
        let exp_delay = self.base_delay.as_millis() as u64
            * 2u64.saturating_pow(self.current_retry);
        let capped = exp_delay.min(self.max_delay.as_millis() as u64);
        let jittered = rand::random::<u64>() % (capped + 1);
        self.current_retry += 1;
        Some(Duration::from_millis(jittered))
    }

    /// Reset after a successful request.
    pub fn reset(&mut self) {
        self.current_retry = 0;
    }
}
```

**Configuration:**

```toml
[mcp.github.rate_limit]
base_delay_ms = 1000       # Base delay for first retry. Range: 100..5000.
max_delay_ms = 60000       # Maximum delay cap. Range: 5000..300000.
max_retries = 5            # Maximum retries before failure. Range: 1..10.
respect_retry_after = true # Use GitHub's Retry-After header when present.
```

### Error response differentiation

The server distinguishes GitHub API error codes to provide actionable feedback to agents:

```rust
/// GitHub API error classification.
pub enum GitHubApiError {
    /// 401 Unauthorized: token expired or invalid.
    /// Action: re-authenticate or report to operator.
    Unauthorized { message: String },

    /// 403 Forbidden: permission denied or secondary rate limit.
    /// Two sub-cases:
    /// - Rate limit exceeded (check X-RateLimit-Remaining header)
    /// - Insufficient permissions (token lacks required scope)
    Forbidden {
        is_rate_limit: bool,
        rate_limit_reset: Option<u64>, // Unix timestamp when limit resets.
        required_scope: Option<String>,
        message: String,
    },

    /// 404 Not Found: resource doesn't exist or is private.
    /// Agent should not retry.
    NotFound { resource_type: String, identifier: String },

    /// 422 Unprocessable Entity: validation error.
    /// Agent should fix the request parameters.
    ValidationError { errors: Vec<ValidationDetail> },

    /// 429 Too Many Requests: primary rate limit hit.
    /// Agent should back off.
    RateLimited {
        retry_after_secs: Option<u64>,
    },

    /// 5xx Server Error: GitHub is having issues.
    /// Agent should retry with backoff.
    ServerError { status: u16, message: String },
}

pub struct ValidationDetail {
    pub field: String,
    pub code: String,  // "missing", "invalid", "already_exists"
    pub message: String,
}
```

```rust
/// Classify a GitHub API response into an actionable error.
pub fn classify_error(status: u16, headers: &Headers, body: &str) -> GitHubApiError {
    match status {
        401 => GitHubApiError::Unauthorized {
            message: parse_message(body),
        },
        403 => {
            let remaining = headers.get("x-ratelimit-remaining")
                .and_then(|v| v.parse::<u64>().ok());
            let reset = headers.get("x-ratelimit-reset")
                .and_then(|v| v.parse::<u64>().ok());
            GitHubApiError::Forbidden {
                is_rate_limit: remaining == Some(0),
                rate_limit_reset: reset,
                required_scope: extract_required_scope(body),
                message: parse_message(body),
            }
        }
        404 => GitHubApiError::NotFound {
            resource_type: extract_resource_type(body),
            identifier: extract_identifier(body),
        },
        422 => GitHubApiError::ValidationError {
            errors: parse_validation_errors(body),
        },
        429 => GitHubApiError::RateLimited {
            retry_after_secs: headers.get("retry-after")
                .and_then(|v| v.parse::<u64>().ok()),
        },
        500..=599 => GitHubApiError::ServerError {
            status,
            message: parse_message(body),
        },
        _ => GitHubApiError::ServerError {
            status,
            message: format!("Unexpected status {}: {}", status, body),
        },
    }
}
```

### Rust handler signatures and octocrab API mapping

Each tool maps to one or more `octocrab` API calls:

```rust
use octocrab::Octocrab;

pub struct GitHubMcpServer {
    client: Octocrab,
    default_repo: Option<(String, String)>, // (owner, repo)
    backoff: RateLimitBackoff,
}

impl GitHubMcpServer {
    // PR tools
    pub async fn get_pr(&self, repo: &str, number: u64, include_diff: bool, include_comments: bool)
        -> Result<serde_json::Value>;
        // octocrab: client.pulls(owner, repo).get(number)
        //           + client.pulls(owner, repo).get_diff(number) if include_diff
        //           + client.pulls(owner, repo).list_comments(number) if include_comments

    pub async fn create_pr(&self, repo: &str, title: &str, head: &str, base: &str, body: &str, draft: bool)
        -> Result<serde_json::Value>;
        // octocrab: client.pulls(owner, repo).create(title, head, base).body(body).draft(draft)

    pub async fn review_pr(&self, repo: &str, number: u64, event: &str, body: &str, comments: Vec<ReviewComment>)
        -> Result<serde_json::Value>;
        // octocrab: client.pulls(owner, repo).create_review(number, event, body, comments)

    pub async fn merge_pr(&self, repo: &str, number: u64, method: &str, title: Option<&str>, message: Option<&str>)
        -> Result<serde_json::Value>;
        // octocrab: client.pulls(owner, repo).merge(number).method(method)

    pub async fn update_pr(&self, repo: &str, number: u64, title: Option<&str>, body: Option<&str>, base: Option<&str>)
        -> Result<serde_json::Value>;
        // octocrab: client.pulls(owner, repo).update(number).title(title).body(body)

    pub async fn list_prs(&self, repo: &str, state: &str, author: Option<&str>, label: Option<&str>, limit: u64)
        -> Result<serde_json::Value>;
        // octocrab: client.pulls(owner, repo).list().state(state).per_page(limit)

    // Issue tools
    pub async fn get_issue(&self, repo: &str, number: u64, include_comments: bool)
        -> Result<serde_json::Value>;
        // octocrab: client.issues(owner, repo).get(number)

    pub async fn create_issue(&self, repo: &str, title: &str, body: &str, labels: Vec<String>, assignees: Vec<String>)
        -> Result<serde_json::Value>;
        // octocrab: client.issues(owner, repo).create(title).body(body).labels(labels)

    pub async fn update_issue(&self, repo: &str, number: u64, state: Option<&str>, labels: Option<Vec<String>>,
                              assignees: Option<Vec<String>>, title: Option<&str>, body: Option<&str>)
        -> Result<serde_json::Value>;
        // octocrab: client.issues(owner, repo).update(number)

    pub async fn list_issues(&self, repo: &str, state: &str, labels: Option<&str>, assignee: Option<&str>, limit: u64)
        -> Result<serde_json::Value>;
        // octocrab: client.issues(owner, repo).list().state(state).per_page(limit)

    pub async fn comment_issue(&self, repo: &str, number: u64, body: &str)
        -> Result<serde_json::Value>;
        // octocrab: client.issues(owner, repo).create_comment(number, body)

    pub async fn search_issues(&self, query: &str, limit: u64)
        -> Result<serde_json::Value>;
        // octocrab: client.search().issues_and_pull_requests(query).per_page(limit)

    // Repository tools
    pub async fn get_file(&self, repo: &str, path: &str, git_ref: &str)
        -> Result<serde_json::Value>;
        // octocrab: client.repos(owner, repo).get_content().path(path).r#ref(git_ref)

    pub async fn search_code(&self, query: &str, repo: Option<&str>, language: Option<&str>, limit: u64)
        -> Result<serde_json::Value>;
        // octocrab: client.search().code(query).per_page(limit)

    pub async fn list_branches(&self, repo: &str, limit: u64)
        -> Result<serde_json::Value>;
        // octocrab: client.repos(owner, repo).list_branches().per_page(limit)

    pub async fn get_tree(&self, repo: &str, path: &str, git_ref: &str, recursive: bool)
        -> Result<serde_json::Value>;
        // octocrab: client.repos(owner, repo).get_content().path(path).r#ref(git_ref)

    // CI tools
    pub async fn get_check_status(&self, repo: &str, git_ref: &str)
        -> Result<serde_json::Value>;
        // octocrab: client.checks(owner, repo).list_for_ref(git_ref)
}
```

### Test criteria

- Each tool validates required input fields and returns a clear error for missing fields
- `classify_error()` correctly distinguishes 401/403/404/422/429/5xx responses
- Rate limit backoff delay increases exponentially and respects the max_delay cap
- Backoff resets to zero after a successful request
- 403 with `X-RateLimit-Remaining: 0` is classified as rate limit, not permission error
- 404 for a private repo is not retried
- All 17 tools return valid JSON matching their output schema
- `merge_pr` fails with a clear message when checks have not passed
