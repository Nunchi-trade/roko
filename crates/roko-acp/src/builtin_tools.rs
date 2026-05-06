//! ACP builtin tool definitions and execution.
//!
//! Returns [`ToolDef`] entries for the 8 core tools exposed to ACP sessions,
//! plus [`execute_acp_builtin_tool`] which dispatches tool calls to async handlers.

use std::path::{Path, PathBuf};

use tokio::sync::mpsc;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::bridge_events::CognitiveEvent;
use crate::types::{ContentBlock, ToolCallKind, ToolCallStatus};
use roko_core::tool::{ToolCategory, ToolConcurrency, ToolDef, ToolPermission, ToolSchema};

/// Returns the 8 builtin tool definitions for ACP sessions.
#[must_use]
pub fn acp_builtin_tools() -> Vec<ToolDef> {
    vec![
        read_file(),
        write_file(),
        edit_file(),
        glob(),
        grep(),
        bash(),
        ls(),
        web_fetch(),
    ]
}

fn read_file() -> ToolDef {
    ToolDef::new(
        "read_file",
        "Read the contents of a UTF-8 file.",
        ToolCategory::Read,
        ToolPermission::read_only(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Absolute or relative file path to read." },
            "lines": { "type": "integer", "description": "Maximum number of lines to return." }
        },
        "required": ["path"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(30_000)
}

fn write_file() -> ToolDef {
    ToolDef::new(
        "write_file",
        "Write content to a file, creating or overwriting it.",
        ToolCategory::Write,
        ToolPermission::writes(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Absolute or relative file path to write." },
            "content": { "type": "string", "description": "Full file content to write." }
        },
        "required": ["path", "content"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_timeout_ms(30_000)
}

fn edit_file() -> ToolDef {
    ToolDef::new(
        "edit_file",
        "Perform a surgical string replacement in a file.",
        ToolCategory::Write,
        ToolPermission::writes(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "File path to edit." },
            "old_string": { "type": "string", "description": "Exact text to find and replace." },
            "new_string": { "type": "string", "description": "Replacement text." }
        },
        "required": ["path", "old_string", "new_string"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_timeout_ms(30_000)
}

fn glob() -> ToolDef {
    ToolDef::new("glob", "Find files matching a glob pattern.", ToolCategory::Read, ToolPermission::read_only())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern to match (e.g. \"**/*.rs\")." },
                "path": { "type": "string", "description": "Directory to search in. Defaults to working directory." }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Parallel)
        .with_idempotent(true)
        .with_timeout_ms(30_000)
}

fn grep() -> ToolDef {
    ToolDef::new("grep", "Search file contents using a regex pattern.", ToolCategory::Read, ToolPermission::read_only())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regular expression pattern to search for." },
                "path": { "type": "string", "description": "File or directory to search in." },
                "type": { "type": "string", "description": "File type filter (e.g. \"rs\", \"py\", \"js\")." }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Parallel)
        .with_idempotent(true)
        .with_timeout_ms(60_000)
}

fn bash() -> ToolDef {
    ToolDef::new("bash", "Execute a shell command and return its output.", ToolCategory::Exec, ToolPermission::executes())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute." },
                "timeout": { "type": "integer", "description": "Timeout in milliseconds (default 120000)." }
            },
            "required": ["command"],
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Serial)
        .with_timeout_ms(120_000)
}

fn ls() -> ToolDef {
    ToolDef::new("ls", "List directory contents.", ToolCategory::Read, ToolPermission::read_only())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path to list. Defaults to working directory." }
            },
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Parallel)
        .with_idempotent(true)
        .with_timeout_ms(15_000)
}

fn web_fetch() -> ToolDef {
    ToolDef::new("web_fetch", "Fetch content from a URL and process it.", ToolCategory::Network, ToolPermission::networked())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to fetch content from." },
                "prompt": { "type": "string", "description": "Prompt describing what information to extract." }
            },
            "required": ["url", "prompt"],
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Parallel)
        .with_timeout_ms(60_000)
}

// ── Tool execution ──────────────────────────────────────────────────

/// Whether a tool is auto-approved (read-only / safe) or needs user permission.
fn needs_permission(name: &str) -> bool {
    matches!(name, "write_file" | "edit_file" | "bash")
}

/// Map tool name to the ACP [`ToolCallKind`] for UI display.
fn tool_call_kind(name: &str) -> ToolCallKind {
    match name {
        "read_file" => ToolCallKind::Read,
        "write_file" => ToolCallKind::Create,
        "edit_file" => ToolCallKind::Edit,
        "glob" | "grep" => ToolCallKind::Search,
        "bash" => ToolCallKind::Terminal,
        "ls" => ToolCallKind::Read,
        "web_fetch" => ToolCallKind::Fetch,
        _ => ToolCallKind::Other,
    }
}

/// Emit a [`CognitiveEvent`] without blocking the caller.
async fn emit(tx: &mpsc::Sender<CognitiveEvent>, event: CognitiveEvent) {
    if tx.send(event).await.is_err() {
        debug!("builtin_tools: event receiver dropped");
    }
}

/// Extract a required string field from a JSON args object.
fn require_str(args: &serde_json::Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| format!("missing required parameter: {key}"))
}

/// Extract an optional string field.
fn opt_str(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Extract an optional integer field.
fn opt_u64(args: &serde_json::Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

/// Resolve a path argument relative to `workdir`, rejecting traversal outside it.
fn resolve_path(workdir: &Path, raw: &str) -> Result<PathBuf, String> {
    let candidate = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        workdir.join(raw)
    };
    let resolved = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.clone());
    // Allow the exact workdir or any child.
    let workdir_canon = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    if !resolved.starts_with(&workdir_canon) {
        return Err(format!("path '{}' escapes the working directory", raw));
    }
    Ok(candidate)
}

/// Like [`resolve_path`] but allows the path to not yet exist (for write_file).
fn resolve_path_for_write(workdir: &Path, raw: &str) -> Result<PathBuf, String> {
    let candidate = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        workdir.join(raw)
    };
    // Check the parent exists and is inside workdir.
    let parent = candidate.parent().unwrap_or(workdir);
    let parent_canon = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    let workdir_canon = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    if !parent_canon.starts_with(&workdir_canon) {
        return Err(format!("path '{}' escapes the working directory", raw));
    }
    Ok(candidate)
}

/// Execute an ACP builtin tool by name.
///
/// Emits [`CognitiveEvent::ToolCallStart`] before execution and
/// [`CognitiveEvent::ToolCallComplete`] after. Returns the tool result
/// as a `String` (content or error message).
///
/// # Safety classification
///
/// Auto-approve: `read_file`, `glob`, `grep`, `ls`, `web_fetch`.
/// Needs permission: `write_file`, `edit_file`, `bash` — the emitted
/// `ToolCallStart` carries `needs_permission` info via the [`ToolCallKind`]
/// so the ACP layer can gate on it.
pub async fn execute_acp_builtin_tool(
    name: &str,
    args: &serde_json::Value,
    workdir: &Path,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> String {
    let tool_call_id = Uuid::new_v4().to_string();
    let kind = tool_call_kind(name);
    let title = format_tool_title(name, args);

    // Emit start event.
    emit(
        event_sender,
        CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: title.clone(),
            kind,
            locations: None,
        },
    )
    .await;

    let result = match name {
        "read_file" => exec_read_file(args, workdir).await,
        "write_file" => exec_write_file(args, workdir).await,
        "edit_file" => exec_edit_file(args, workdir).await,
        "glob" => exec_glob(args, workdir).await,
        "grep" => exec_grep(args, workdir).await,
        "bash" => exec_bash(args, workdir).await,
        "ls" => exec_ls(args, workdir).await,
        "web_fetch" => exec_web_fetch(args).await,
        _ => Err(format!("unknown builtin tool: {name}")),
    };

    let (status, output) = match result {
        Ok(text) => (ToolCallStatus::Completed, text),
        Err(msg) => (ToolCallStatus::Failed, msg),
    };

    // Emit completion event.
    emit(
        event_sender,
        CognitiveEvent::ToolCallComplete {
            tool_call_id,
            status,
            content: vec![ContentBlock::Text {
                text: output.clone(),
            }],
        },
    )
    .await;

    output
}

/// Returns `true` if the named tool requires user permission before execution.
#[must_use]
pub fn tool_needs_permission(name: &str) -> bool {
    needs_permission(name)
}

/// Build a short human-readable title for the tool call.
fn format_tool_title(name: &str, args: &serde_json::Value) -> String {
    match name {
        "read_file" => {
            let path = opt_str(args, "path").unwrap_or_default();
            format!("Read {path}")
        }
        "write_file" => {
            let path = opt_str(args, "path").unwrap_or_default();
            format!("Write {path}")
        }
        "edit_file" => {
            let path = opt_str(args, "path").unwrap_or_default();
            format!("Edit {path}")
        }
        "glob" => {
            let pattern = opt_str(args, "pattern").unwrap_or_default();
            format!("Glob {pattern}")
        }
        "grep" => {
            let pattern = opt_str(args, "pattern").unwrap_or_default();
            format!("Grep {pattern}")
        }
        "bash" => {
            let cmd = opt_str(args, "command").unwrap_or_default();
            let short = if cmd.len() > 60 { &cmd[..60] } else { &cmd };
            format!("$ {short}")
        }
        "ls" => {
            let path = opt_str(args, "path").unwrap_or_else(|| ".".into());
            format!("List {path}")
        }
        "web_fetch" => {
            let url = opt_str(args, "url").unwrap_or_default();
            format!("Fetch {url}")
        }
        _ => name.to_string(),
    }
}

// ── Individual tool handlers ────────────────────────────────────────

async fn exec_read_file(args: &serde_json::Value, workdir: &Path) -> Result<String, String> {
    let raw_path = require_str(args, "path")?;
    let path = resolve_path(workdir, &raw_path)?;
    let lines_limit = opt_u64(args, "lines");

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("read_file: {e}"))?;

    match lines_limit {
        Some(n) => {
            let taken: Vec<&str> = content.lines().take(n as usize).collect();
            Ok(taken.join("\n"))
        }
        None => Ok(content),
    }
}

async fn exec_write_file(args: &serde_json::Value, workdir: &Path) -> Result<String, String> {
    let raw_path = require_str(args, "path")?;
    let content = require_str(args, "content")?;
    let path = resolve_path_for_write(workdir, &raw_path)?;

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("write_file: mkdir: {e}"))?;
    }

    tokio::fs::write(&path, &content)
        .await
        .map_err(|e| format!("write_file: {e}"))?;

    Ok(format!("Wrote {} bytes to {}", content.len(), raw_path))
}

async fn exec_edit_file(args: &serde_json::Value, workdir: &Path) -> Result<String, String> {
    let raw_path = require_str(args, "path")?;
    let old_string = require_str(args, "old_string")?;
    let new_string = require_str(args, "new_string")?;
    let path = resolve_path(workdir, &raw_path)?;

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("edit_file: read: {e}"))?;

    let count = content.matches(&old_string).count();
    if count == 0 {
        return Err(format!("edit_file: old_string not found in {raw_path}"));
    }
    if count > 1 {
        return Err(format!(
            "edit_file: old_string found {count} times in {raw_path} (must be unique)"
        ));
    }

    let updated = content.replacen(&old_string, &new_string, 1);
    tokio::fs::write(&path, &updated)
        .await
        .map_err(|e| format!("edit_file: write: {e}"))?;

    Ok(format!("Edited {raw_path}"))
}

async fn exec_glob(args: &serde_json::Value, workdir: &Path) -> Result<String, String> {
    let pattern = require_str(args, "pattern")?;
    let search_dir = match opt_str(args, "path") {
        Some(p) => resolve_path(workdir, &p)?,
        None => workdir.to_path_buf(),
    };

    let root = search_dir.clone();
    let pat = pattern.clone();
    let matches = tokio::task::spawn_blocking(move || walk_and_glob(&root, &pat))
        .await
        .map_err(|e| format!("glob: join: {e}"))?
        .map_err(|e| format!("glob: {e}"))?;

    if matches.is_empty() {
        Ok("No matches found".into())
    } else {
        Ok(matches.join("\n"))
    }
}

/// Recursive directory walk with shell-style glob matching.
/// Reuses the same algorithm as roko-std's glob handler.
fn walk_and_glob(root: &Path, pattern: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd =
            std::fs::read_dir(&dir).map_err(|e| format!("read_dir({}): {e}", dir.display()))?;
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if glob_matches(pattern, &rel_str) {
                out.push(rel_str);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Minimal shell-style glob matcher supporting `*`, `?`, `[...]`, `**`.
fn glob_matches(pattern: &str, path: &str) -> bool {
    let pat_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();
    match_segments(&pat_parts, &path_parts)
}

fn match_segments(pat: &[&str], path: &[&str]) -> bool {
    match (pat.first(), path.first()) {
        (None, None) => true,
        (Some(&"**"), None) => match_segments(&pat[1..], path),
        (Some(&"**"), Some(_)) => {
            match_segments(&pat[1..], path) || match_segments(pat, &path[1..])
        }
        (None, Some(_)) | (Some(_), None) => false,
        (Some(p), Some(t)) => {
            segment_match(p.as_bytes(), t.as_bytes()) && match_segments(&pat[1..], &path[1..])
        }
    }
}

fn segment_match(pat: &[u8], text: &[u8]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star: Option<(usize, usize)> = None;
    while ti < text.len() {
        if pi < pat.len() {
            if pat[pi] == b'*' {
                star = Some((pi + 1, ti));
                pi += 1;
                continue;
            }
            if pat[pi] == b'?' {
                pi += 1;
                ti += 1;
                continue;
            }
            if pat[pi] == b'[' {
                if let Some((end, matched)) = class_match(&pat[pi..], text[ti])
                    && matched
                {
                    pi += end;
                    ti += 1;
                    continue;
                }
            } else if pat[pi] == text[ti] {
                pi += 1;
                ti += 1;
                continue;
            }
        }
        if let Some((sp, st)) = star {
            pi = sp;
            ti = st + 1;
            star = Some((sp, ti));
            continue;
        }
        return false;
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

fn class_match(pat: &[u8], c: u8) -> Option<(usize, bool)> {
    let mut i = 1;
    let negate = pat.get(1) == Some(&b'!') || pat.get(1) == Some(&b'^');
    if negate {
        i += 1;
    }
    let mut matched = false;
    while i < pat.len() && pat[i] != b']' {
        if i + 2 < pat.len() && pat[i + 1] == b'-' && pat[i + 2] != b']' {
            if c >= pat[i] && c <= pat[i + 2] {
                matched = true;
            }
            i += 3;
        } else {
            if pat[i] == c {
                matched = true;
            }
            i += 1;
        }
    }
    if i >= pat.len() {
        return None;
    }
    Some((i + 1, matched ^ negate))
}

async fn exec_grep(args: &serde_json::Value, workdir: &Path) -> Result<String, String> {
    let pattern = require_str(args, "pattern")?;
    let search_path = match opt_str(args, "path") {
        Some(p) => resolve_path(workdir, &p)?,
        None => workdir.to_path_buf(),
    };
    let file_type = opt_str(args, "type");

    // Try ripgrep first, fall back to manual search.
    let mut cmd = tokio::process::Command::new("rg");
    cmd.arg("--no-heading")
        .arg("--line-number")
        .arg("--color=never")
        .arg("--max-count=200")
        .arg(&pattern)
        .arg(&search_path)
        .current_dir(workdir);

    if let Some(ref ft) = file_type {
        cmd.arg("--type").arg(ft);
    }

    match cmd.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.success() || !stdout.is_empty() {
                if stdout.is_empty() {
                    Ok("No matches found".into())
                } else {
                    Ok(stdout.into_owned())
                }
            } else if stderr.contains("not found") || stderr.contains("No such file") {
                // rg not installed — fall back to manual search.
                warn!("rg not found, falling back to manual grep");
                manual_grep(&search_path, &pattern).await
            } else {
                // rg ran but found nothing (exit code 1) or had an error.
                if output.status.code() == Some(1) {
                    Ok("No matches found".into())
                } else {
                    Err(format!("grep: rg error: {}", stderr.trim()))
                }
            }
        }
        Err(_) => {
            // rg binary not available.
            manual_grep(&search_path, &pattern).await
        }
    }
}

/// Fallback grep: walk files and do line-by-line search.
async fn manual_grep(path: &Path, pattern: &str) -> Result<String, String> {
    let path = path.to_path_buf();
    let pattern = pattern.to_string();
    tokio::task::spawn_blocking(move || {
        let re = regex::Regex::new(&pattern).map_err(|e| format!("invalid regex: {e}"))?;
        let mut results = Vec::new();
        let mut stack = vec![path.clone()];
        while let Some(p) = stack.pop() {
            if p.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&p) {
                    for entry in rd.flatten() {
                        let ep = entry.path();
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if ep.is_dir() {
                            if !name.starts_with('.') && name != "target" && name != "node_modules"
                            {
                                stack.push(ep);
                            }
                        } else {
                            stack.push(ep);
                        }
                    }
                }
            } else if let Ok(content) = std::fs::read_to_string(&p) {
                for (i, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        results.push(format!("{}:{}:{}", p.display(), i + 1, line));
                        if results.len() >= 200 {
                            return Ok(results.join("\n"));
                        }
                    }
                }
            }
        }
        if results.is_empty() {
            Ok("No matches found".into())
        } else {
            Ok(results.join("\n"))
        }
    })
    .await
    .map_err(|e| format!("grep: join: {e}"))?
}

async fn exec_bash(args: &serde_json::Value, workdir: &Path) -> Result<String, String> {
    let command = require_str(args, "command")?;
    let timeout_ms = opt_u64(args, "timeout").unwrap_or(120_000);

    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(&command)
        .current_dir(workdir)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("bash: spawn: {e}"))?;

    // Read stdout/stderr concurrently with wait to avoid pipe deadlocks.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let read_stdout = async {
        if let Some(pipe) = stdout_pipe {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            let mut reader = tokio::io::BufReader::new(pipe);
            let _ = reader.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        } else {
            String::new()
        }
    };
    let read_stderr = async {
        if let Some(pipe) = stderr_pipe {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            let mut reader = tokio::io::BufReader::new(pipe);
            let _ = reader.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        } else {
            String::new()
        }
    };

    let wait_result = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), async {
        let (status, stdout, stderr) = tokio::join!(child.wait(), read_stdout, read_stderr);
        (status, stdout, stderr)
    })
    .await;

    match wait_result {
        Ok((Ok(status), stdout, stderr)) => {
            let code = status.code().unwrap_or(-1);
            if status.success() {
                if stderr.is_empty() {
                    Ok(stdout)
                } else {
                    Ok(format!("{stdout}\n[stderr]\n{stderr}"))
                }
            } else {
                Err(format!(
                    "bash: exit code {code}\n[stdout]\n{stdout}\n[stderr]\n{stderr}"
                ))
            }
        }
        Ok((Err(e), _, _)) => Err(format!("bash: {e}")),
        Err(_) => {
            // kill_on_drop ensures the child is killed when dropped.
            drop(child);
            Err(format!("bash: command timed out after {timeout_ms}ms"))
        }
    }
}

async fn exec_ls(args: &serde_json::Value, workdir: &Path) -> Result<String, String> {
    let dir = match opt_str(args, "path") {
        Some(p) => resolve_path(workdir, &p)?,
        None => workdir.to_path_buf(),
    };

    let mut rd = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| format!("ls: {e}"))?;

    let mut entries = Vec::new();
    while let Some(entry) = rd.next_entry().await.map_err(|e| format!("ls: {e}"))? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = entry.metadata().await;
        let suffix = match meta {
            Ok(m) if m.is_dir() => "/",
            _ => "",
        };
        entries.push(format!("{name}{suffix}"));
    }
    entries.sort();
    Ok(entries.join("\n"))
}

async fn exec_web_fetch(args: &serde_json::Value) -> Result<String, String> {
    let url = require_str(args, "url")?;
    let _prompt = require_str(args, "prompt")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("web_fetch: client: {e}"))?;

    let resp = client
        .get(&url)
        .header("User-Agent", "roko-acp/0.1")
        .send()
        .await
        .map_err(|e| format!("web_fetch: request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("web_fetch: HTTP {status}"));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("web_fetch: body: {e}"))?;

    // Truncate very large responses.
    const MAX_BYTES: usize = 100_000;
    if body.len() > MAX_BYTES {
        Ok(format!(
            "{}\n\n[truncated at {} bytes]",
            &body[..MAX_BYTES],
            MAX_BYTES
        ))
    } else {
        Ok(body)
    }
}
