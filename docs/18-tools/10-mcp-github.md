# 10 — roko-mcp-github

> 17 GitHub API tools: PR operations, issue management, repository tools, CI integration.
> Full JSON Schema specifications.

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
