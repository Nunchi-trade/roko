//! `roko chat` with inline ratatui UX.
//!
//! Claude Code-like experience: streaming responses in a fixed viewport at the
//! bottom, completed turns pushed into terminal scrollback. Supports multi-line
//! input, `/` commands, Ctrl+C interrupt, and cost tracking.
//!
//! Falls back to the legacy line-oriented REPL when stdout is not a TTY.

use std::collections::HashMap;
use std::io::Write as _;
use std::time::{Duration, Instant};

use anyhow::{bail, Context as _, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use serde::Deserialize;
use serde_json::json;

use crate::auth;
use crate::auth_detect::AuthMethod;
use crate::chat::{self, extract_clean_text};
#[cfg(feature = "legacy-orchestrate")]
use crate::dispatch_direct;
use crate::dispatch_v2::{DispatchResult, ToolOutput};
use crate::inline::primitives::{CostMeter, StreamingState};
use crate::inline::styled;
use crate::inline::symbols;
use crate::inline::terminal::InlineTerminal;
use crate::tui::Theme;

use crate::chat_session::{ChatAgentSession, SlashResult};
use chrono;
use roko_core::agent::ProviderKind;
use roko_learn::cost_table::CostTable;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Chat session phase.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Waiting for user input.
    Input,
    /// Sent message, waiting for first token.
    Thinking,
    /// Receiving streaming tokens.
    Streaming,
    /// Dispatch error — shows [r]etry / [s]witch / [q]uit options.
    Error { prompt: String, error: String },
    /// Session complete (user pressed Ctrl-D or /quit).
    Done,
}

/// All available slash commands for tab-completion.
const SLASH_COMMANDS: &[(&str, &str)] = &[
    // Session & display
    ("/help", "show available commands"),
    ("/version", "show version info"),
    ("/stats", "detailed session statistics"),
    ("/context", "show token/context usage"),
    ("/history", "show input history"),
    ("/copy", "copy last response to clipboard"),
    ("/compact", "toggle compact output mode"),
    ("/system", "set system message for session"),
    ("/reset", "clear conversation, fresh start"),
    ("/retry", "resend the last message"),
    ("/export", "export conversation (markdown/json)"),
    ("/cost", "session cost summary"),
    ("/tools", "list available tools"),
    ("/clear", "clear scrollback"),
    ("/quit", "exit the chat"),
    ("/exit", "exit the chat"),
    // Configuration
    ("/config", "show or set configuration"),
    ("/config providers", "list configured providers"),
    ("/config models", "list available models"),
    ("/config gates", "show gate configuration"),
    ("/model", "show or change model"),
    ("/provider", "show current auth/provider"),
    ("/auth", "show current auth/provider"),
    ("/effort", "set effort level (low/med/high/max)"),
    // Workspace & git
    ("/status", "workspace status"),
    ("/doctor", "health check"),
    ("/diff", "show git diff"),
    ("/git", "git status"),
    ("/log", "recent git commits"),
    ("/branch", "show current branch"),
    ("/changes", "changed files since last commit"),
    // File operations
    ("/file", "read and display a file"),
    ("/search", "grep workspace for pattern"),
    ("/find", "find files matching pattern"),
    ("/tree", "show directory tree"),
    // Agent & workflow
    ("/agent", "show/switch agent identity"),
    ("/agent list", "list configured agents"),
    ("/run", "execute prompt through universal loop"),
    ("/plan list", "list plans"),
    ("/plan run", "execute a plan"),
    ("/plan generate", "generate plan from prompt"),
    ("/gate", "toggle gates (compile/test/clippy)"),
    // PRD & research
    ("/prd idea", "capture a work item idea"),
    ("/prd list", "list PRDs"),
    ("/research", "research a topic"),
    // Knowledge & learning
    ("/knowledge", "query knowledge store"),
    ("/learn", "show learning state"),
];

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

/// Fuzzy-match `query` against `target` using character-subsequence matching.
///
/// Returns `Some((score, matched_indices))` if all characters in `query` appear
/// in order within `target`, or `None` if not.
///
/// Scoring:
/// - +15 for a match at index 0
/// - +10 for a match at a word boundary (after `/`, `-`, `_`, or space)
/// - +5 for consecutive matches
/// - -1 per gap between matched characters
fn fuzzy_match(query: &str, target: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }

    let query_lower: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let target_chars: Vec<char> = target.chars().collect();
    let target_lower: Vec<char> = target.chars().flat_map(|c| c.to_lowercase()).collect();

    let mut matched_indices = Vec::with_capacity(query_lower.len());
    let mut score: i32 = 0;
    let mut qi = 0;
    let mut last_match: Option<usize> = None;

    for (ti, &tc) in target_lower.iter().enumerate() {
        if qi < query_lower.len() && tc == query_lower[qi] {
            // Scoring bonuses
            if ti == 0 {
                score += 15;
            } else if matches!(
                target_chars.get(ti.wrapping_sub(1)),
                Some('/' | '-' | '_' | ' ')
            ) {
                score += 10;
            }

            if let Some(prev) = last_match {
                if ti == prev + 1 {
                    score += 5; // consecutive
                } else {
                    score -= (ti - prev - 1) as i32; // gap penalty
                }
            }

            matched_indices.push(ti);
            last_match = Some(ti);
            qi += 1;
        }
    }

    if qi == query_lower.len() {
        Some((score, matched_indices))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Completion state
// ---------------------------------------------------------------------------

/// A single match in the completion dropdown.
#[derive(Debug, Clone)]
struct CompletionMatch {
    command: &'static str,
    description: &'static str,
    score: i32,
    matched_indices: Vec<usize>,
}

/// State for the slash-command completion dropdown.
#[derive(Debug)]
struct CompletionState {
    /// Whether the dropdown is currently visible.
    visible: bool,
    /// Sorted matches (best first).
    matches: Vec<CompletionMatch>,
    /// Currently selected index within `matches`.
    selected: usize,
    /// The query that produced the current matches (without leading `/`).
    query: String,
}

impl CompletionState {
    fn new() -> Self {
        Self {
            visible: false,
            matches: Vec::new(),
            selected: 0,
            query: String::new(),
        }
    }

    /// Recompute matches from the current buffer. Shows all commands when query
    /// is just `/`, fuzzy-filters otherwise.
    fn update(&mut self, buffer: &str) {
        let trimmed = buffer.trim_start();
        if !trimmed.starts_with('/') {
            self.dismiss();
            return;
        }

        self.query = trimmed.to_string();
        let filter = &trimmed[1..]; // text after the `/`

        let mut matches: Vec<CompletionMatch> = if filter.is_empty() {
            // Show all commands when user types just `/`
            SLASH_COMMANDS
                .iter()
                .map(|&(cmd, desc)| CompletionMatch {
                    command: cmd,
                    description: desc,
                    score: 0,
                    matched_indices: Vec::new(),
                })
                .collect()
        } else {
            SLASH_COMMANDS
                .iter()
                .filter_map(|&(cmd, desc)| {
                    // Match against the command without the leading `/`
                    let cmd_body = &cmd[1..];
                    fuzzy_match(filter, cmd_body).map(|(score, indices)| CompletionMatch {
                        command: cmd,
                        description: desc,
                        score,
                        // Offset indices by 1 to account for the leading `/`
                        matched_indices: indices.into_iter().map(|i| i + 1).collect(),
                    })
                })
                .collect()
        };

        matches.sort_by(|a, b| b.score.cmp(&a.score));

        self.visible = !matches.is_empty();
        self.matches = matches;
        // Keep selected in bounds
        if self.selected >= self.matches.len() {
            self.selected = 0;
        }
    }

    /// Move selection to the next match (wraps around).
    fn select_next(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
        }
    }

    /// Move selection to the previous match (wraps around).
    fn select_prev(&mut self) {
        if !self.matches.is_empty() {
            if self.selected == 0 {
                self.selected = self.matches.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    /// Accept the currently selected match. Returns the command string to populate
    /// into the buffer, or `None` if nothing is selected.
    fn accept(&mut self) -> Option<String> {
        if !self.visible || self.matches.is_empty() {
            return None;
        }
        let cmd = self.matches[self.selected].command.to_string();
        self.dismiss();
        Some(cmd)
    }

    /// Hide the dropdown and clear state.
    fn dismiss(&mut self) {
        self.visible = false;
        self.matches.clear();
        self.selected = 0;
        self.query.clear();
    }
}

/// Reverse history search state (Ctrl+R).
#[derive(Debug)]
struct HistorySearch {
    /// Whether the search is active.
    active: bool,
    /// Current search query.
    query: String,
    /// Index into history matches (0 = most recent match).
    match_idx: usize,
    /// Cached matched entries (indices into history, most-recent-first).
    matches: Vec<usize>,
}

impl HistorySearch {
    fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
            match_idx: 0,
            matches: Vec::new(),
        }
    }

    /// Update matches against the given history.
    fn update(&mut self, history: &[String]) {
        self.matches.clear();
        if self.query.is_empty() {
            return;
        }
        let q = self.query.to_lowercase();
        for (i, entry) in history.iter().enumerate().rev() {
            if entry.to_lowercase().contains(&q) {
                self.matches.push(i);
            }
        }
        // Clamp match_idx
        if !self.matches.is_empty() {
            self.match_idx = self.match_idx.min(self.matches.len() - 1);
        } else {
            self.match_idx = 0;
        }
    }

    /// Get the currently selected history entry, if any.
    fn current_match<'a>(&self, history: &'a [String]) -> Option<&'a str> {
        self.matches
            .get(self.match_idx)
            .and_then(|&idx| history.get(idx))
            .map(|s| s.as_str())
    }

    /// Move to the next (older) match.
    fn next_match(&mut self) {
        if !self.matches.is_empty() {
            self.match_idx = (self.match_idx + 1) % self.matches.len();
        }
    }

    /// Accept the current match and deactivate.
    fn accept(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let idx = self.matches[self.match_idx];
        self.deactivate();
        Some(idx)
    }

    /// Deactivate without accepting.
    fn deactivate(&mut self) {
        self.active = false;
        self.query.clear();
        self.match_idx = 0;
        self.matches.clear();
    }
}

/// Command palette state (Ctrl+K).
///
/// Fuzzy-searchable overlay that gives access to all slash commands and extra
/// actions from anywhere in the input.
#[derive(Debug)]
struct CommandPalette {
    /// Whether the palette is visible.
    active: bool,
    /// User's search query.
    query: String,
    /// Filtered matches.
    matches: Vec<CompletionMatch>,
    /// Currently selected index.
    selected: usize,
}

impl CommandPalette {
    fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
        }
    }

    /// Open the palette, showing all commands.
    fn open(&mut self) {
        self.active = true;
        self.query.clear();
        self.selected = 0;
        self.refresh();
    }

    /// Refresh matches based on current query.
    fn refresh(&mut self) {
        if self.query.is_empty() {
            // Show all commands
            self.matches = SLASH_COMMANDS
                .iter()
                .map(|&(cmd, desc)| CompletionMatch {
                    command: cmd,
                    description: desc,
                    score: 0,
                    matched_indices: Vec::new(),
                })
                .collect();
        } else {
            let mut matches: Vec<CompletionMatch> = SLASH_COMMANDS
                .iter()
                .filter_map(|&(cmd, desc)| {
                    // Search both command name and description
                    let cmd_match = fuzzy_match(&self.query, &cmd[1..]);
                    let desc_match = fuzzy_match(&self.query, desc);
                    let best = match (cmd_match, desc_match) {
                        (Some((s1, i1)), Some((s2, _))) => {
                            if s1 >= s2 {
                                Some((s1, i1))
                            } else {
                                Some((s2, Vec::new()))
                            }
                        }
                        (Some(m), None) | (None, Some(m)) => Some(m),
                        (None, None) => None,
                    };
                    best.map(|(score, indices)| CompletionMatch {
                        command: cmd,
                        description: desc,
                        score,
                        matched_indices: indices,
                    })
                })
                .collect();
            matches.sort_by(|a, b| b.score.cmp(&a.score));
            self.matches = matches;
        }
        // Clamp selected
        if !self.matches.is_empty() {
            self.selected = self.selected.min(self.matches.len() - 1);
        } else {
            self.selected = 0;
        }
    }

    /// Type a character into the search query.
    fn type_char(&mut self, ch: char) {
        self.query.push(ch);
        self.refresh();
    }

    /// Backspace in the search query.
    fn backspace(&mut self) {
        self.query.pop();
        self.refresh();
    }

    fn select_next(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
        }
    }

    fn select_prev(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
        }
    }

    /// Accept the selected command, returning it.
    fn accept(&mut self) -> Option<String> {
        if self.matches.is_empty() {
            self.dismiss();
            return None;
        }
        let cmd = self.matches[self.selected].command.to_string();
        self.dismiss();
        Some(cmd)
    }

    fn dismiss(&mut self) {
        self.active = false;
        self.query.clear();
        self.matches.clear();
        self.selected = 0;
    }
}

/// Input buffer state.
#[derive(Debug)]
struct InputState {
    /// Current input text.
    buffer: String,
    /// Cursor position (byte offset).
    cursor: usize,
    /// Command history.
    history: Vec<String>,
    /// History navigation index (None = current input).
    history_idx: Option<usize>,
    /// Saved current input when navigating history.
    saved_input: String,
    /// Slash-command completion dropdown state.
    completion: CompletionState,
    /// Reverse history search (Ctrl+R).
    search: HistorySearch,
    /// Command palette (Ctrl+K).
    palette: CommandPalette,
}

impl InputState {
    fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_idx: None,
            saved_input: String::new(),
            completion: CompletionState::new(),
            search: HistorySearch::new(),
            palette: CommandPalette::new(),
        }
    }

    /// Returns the ghost suggestion suffix if available.
    ///
    /// Active when: buffer is non-empty, cursor is at end, buffer does NOT
    /// start with `/`, and a history entry matches the current buffer as prefix.
    /// Searches most-recent-first.
    fn ghost_suggestion(&self) -> Option<&str> {
        if self.buffer.is_empty()
            || self.cursor != self.buffer.len()
            || self.buffer.starts_with('/')
            || self.completion.visible
        {
            return None;
        }

        for entry in self.history.iter().rev() {
            if entry.len() > self.buffer.len() && entry.starts_with(&self.buffer) {
                return Some(&entry[self.buffer.len()..]);
            }
        }
        None
    }

    /// Accept the current ghost suggestion, appending it to the buffer.
    /// Returns true if a suggestion was accepted.
    fn accept_ghost(&mut self) -> bool {
        if let Some(suffix) = self.ghost_suggestion().map(|s| s.to_string()) {
            self.buffer.push_str(&suffix);
            self.cursor = self.buffer.len();
            true
        } else {
            false
        }
    }

    /// Insert a newline at the cursor position.
    fn insert_newline(&mut self) {
        self.buffer.insert(self.cursor, '\n');
        self.cursor += 1;
    }

    /// Number of lines in the buffer.
    fn line_count(&self) -> usize {
        self.buffer.lines().count().max(1)
    }

    /// Get the current line index and column offset for the cursor.
    fn cursor_line_col(&self) -> (usize, usize) {
        let before = &self.buffer[..self.cursor];
        let line = before.matches('\n').count();
        let col = before
            .rfind('\n')
            .map(|i| self.cursor - i - 1)
            .unwrap_or(self.cursor);
        (line, col)
    }

    fn insert(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.buffer[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.buffer.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.buffer.len() {
            let next = self.buffer[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.buffer.len());
            self.buffer.drain(self.cursor..next);
        }
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.buffer[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.buffer.len() {
            self.cursor = self.buffer[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.buffer.len());
        }
    }

    fn home(&mut self) {
        self.cursor = 0;
    }

    fn end(&mut self) {
        self.cursor = self.buffer.len();
    }

    fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_idx {
            None => {
                self.saved_input = self.buffer.clone();
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_idx = Some(idx);
        self.buffer = self.history[idx].clone();
        self.cursor = self.buffer.len();
    }

    fn history_down(&mut self) {
        let idx = match self.history_idx {
            None => return,
            Some(i) => i + 1,
        };
        if idx >= self.history.len() {
            self.history_idx = None;
            self.buffer = std::mem::take(&mut self.saved_input);
        } else {
            self.history_idx = Some(idx);
            self.buffer = self.history[idx].clone();
        }
        self.cursor = self.buffer.len();
    }

    fn submit(&mut self) -> String {
        let text = std::mem::take(&mut self.buffer);
        self.cursor = 0;
        self.history_idx = None;
        if !text.trim().is_empty() {
            self.history.push(text.clone());
        }
        text
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    fn is_empty(&self) -> bool {
        self.buffer.trim().is_empty()
    }
}

/// How the session dispatches prompts.
#[derive(Debug, Clone)]
enum DispatchMode {
    /// HTTP backend (sidecar or serve).
    Http {
        client: reqwest::Client,
        backend_url: String,
        is_sidecar: bool,
        serve_url: String,
    },
    /// Direct in-process dispatch via [`AuthMethod`].
    Direct { auth: AuthMethod },
    /// Full agent session with system prompt, tools, MCP, safety.
    Session,
}

/// A recorded conversation message.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ConversationMessage {
    role: String, // "user" or "assistant"
    text: String,
    timestamp: String,
}

/// Serializable session snapshot for auto-save/resume.
#[derive(serde::Serialize, serde::Deserialize)]
struct SessionSnapshot {
    turn_count: u32,
    total_cost: f64,
    input_tokens: u64,
    output_tokens: u64,
    model: String,
    agent_id: String,
    messages: Vec<ConversationMessage>,
    system_message: Option<String>,
    saved_at: String,
    first_user_message: Option<String>,
}

/// Full chat session state.
struct ChatSession {
    phase: Phase,
    input: InputState,
    streaming: StreamingState,
    cost: CostMeter,
    cost_table: CostTable,
    agent_id: String,
    tick: u64,
    started_at: Instant,
    dispatch: DispatchMode,
    /// Channel for receiving async responses from background dispatch calls.
    response_rx: Option<tokio::sync::mpsc::Receiver<Result<DispatchResult, String>>>,
    /// Number of completed user→agent exchanges.
    turn_count: u32,
    /// When the current thinking phase began (for animated labels).
    thinking_started: Option<Instant>,
    /// Conversation transcript for export.
    conversation: Vec<ConversationMessage>,
    /// Last submitted prompt (for retry on error).
    last_prompt: Option<String>,
    /// Persistent system message for this session.
    system_message: Option<String>,
    /// Compact output mode.
    compact: bool,
    /// Full agent session (present when dispatch == `DispatchMode::Session`).
    agent_session: Option<ChatAgentSession>,
}

// ---------------------------------------------------------------------------
// History persistence
// ---------------------------------------------------------------------------

/// Path to the persistent chat history file.
fn history_path() -> std::path::PathBuf {
    let roko_dir = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".roko");
    roko_dir.join("chat_history")
}

/// Load history entries from disk (one per line, last 500).
fn load_history() -> Vec<String> {
    let path = history_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => content
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.replace("\\n", "\n"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Append a single history entry to disk.
fn save_history_entry(entry: &str) {
    let path = history_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let escaped = entry.replace('\n', "\\n");
        let _ = writeln!(f, "{escaped}");
    }
    // Trim to 500 entries periodically
    if let Ok(content) = std::fs::read_to_string(&path) {
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() > 600 {
            let trimmed: Vec<&str> = lines[lines.len() - 500..].to_vec();
            let _ = std::fs::write(&path, trimmed.join("\n") + "\n");
        }
    }
}

// ---------------------------------------------------------------------------
// Session auto-save
// ---------------------------------------------------------------------------

/// Directory for session snapshots.
fn sessions_dir() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".roko")
        .join("sessions")
}

/// Save current session state to disk.
fn save_session(session: &ChatSession) {
    if session.conversation.is_empty() {
        return;
    }
    let dir = sessions_dir();
    let _ = std::fs::create_dir_all(&dir);
    let first_user = session
        .conversation
        .iter()
        .find(|m| m.role == "user")
        .map(|m| {
            let s = m.text.replace('\n', " ");
            if s.len() > 80 {
                format!("{}...", &s[..77])
            } else {
                s
            }
        });
    let snapshot = SessionSnapshot {
        turn_count: session.turn_count,
        total_cost: session.cost.total_cost,
        input_tokens: session.cost.input_tokens,
        output_tokens: session.cost.output_tokens,
        model: active_model_name(session),
        agent_id: session.agent_id.clone(),
        messages: session.conversation.clone(),
        system_message: active_system_prompt(session),
        saved_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        first_user_message: first_user,
    };
    let path = dir.join("last.json");
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&snapshot).unwrap_or_default(),
    );
}

/// Load the last session summary (not full restore — just for display).
fn load_last_session_summary() -> Option<(String, u32, f64, String)> {
    let path = sessions_dir().join("last.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let snap: SessionSnapshot = serde_json::from_str(&content).ok()?;
    let saved_at = snap.saved_at;
    let topic = snap
        .first_user_message
        .unwrap_or_else(|| "unknown".to_string());
    Some((saved_at, snap.turn_count, snap.total_cost, topic))
}

/// Get model name without needing mutable access.
fn current_model_name_static(session: &ChatSession) -> String {
    active_model_name(session)
}

fn active_model_name(session: &ChatSession) -> String {
    if let Some(agent_session) = session.agent_session.as_ref() {
        return agent_session.model.clone();
    }

    match &session.dispatch {
        DispatchMode::Direct { auth } => match auth {
            AuthMethod::AnthropicApi { model, .. } => {
                model.as_deref().unwrap_or("claude-sonnet-4-6").to_string()
            }
            AuthMethod::ClaudeCli => "claude CLI".to_string(),
            AuthMethod::OpenAiCompat { model, .. } => {
                model.as_deref().unwrap_or("unknown").to_string()
            }
            AuthMethod::NeedsSetup => "none".to_string(),
        },
        DispatchMode::Http { .. } => "HTTP backend".to_string(),
        DispatchMode::Session => "session".to_string(),
    }
}

fn active_system_prompt(session: &ChatSession) -> Option<String> {
    session
        .agent_session
        .as_ref()
        .map(|agent_session| agent_session.system_prompt.clone())
        .or_else(|| session.system_message.clone())
}

fn clone_chat_agent_session(session: &ChatAgentSession) -> ChatAgentSession {
    ChatAgentSession {
        workdir: session.workdir.clone(),
        model: session.model.clone(),
        model_selection: session.model_selection.clone(),
        effort: session.effort.clone(),
        system_prompt: session.system_prompt.clone(),
        allowed_tools_csv: session.allowed_tools_csv.clone(),
        mcp_config: session.mcp_config.clone(),
        session_id: session.session_id.clone(),
        api_history: session.api_history.clone(),
        http_client: session.http_client.clone(),
        settings_json: session.settings_json.clone(),
        timeout: session.timeout,
    }
}

fn turn_result_to_dispatch_result(
    turn: crate::chat_session::TurnResult,
    model: String,
) -> DispatchResult {
    DispatchResult {
        text: turn.text,
        model,
        input_tokens: turn.input_tokens,
        output_tokens: turn.output_tokens,
        tool_outputs: turn
            .tool_calls
            .into_iter()
            .map(|tool_call| ToolOutput {
                tool_name: Some(tool_call.name),
                content: if tool_call.input_abbrev.is_empty() {
                    if tool_call.success {
                        "done".to_string()
                    } else {
                        "failed".to_string()
                    }
                } else {
                    tool_call.input_abbrev
                },
            })
            .collect(),
        session_id: turn.session_id,
    }
}

/// Thinking phase label based on elapsed time.
fn thinking_label(elapsed_s: f64) -> &'static str {
    if elapsed_s < 2.0 {
        "Connecting..."
    } else if elapsed_s < 8.0 {
        "Thinking..."
    } else if elapsed_s < 15.0 {
        "Still thinking..."
    } else {
        "Deep in thought..."
    }
}

/// Format a wall-clock time as `h:mm PM`.
/// Truncate a string to fit within `max` columns, adding "..." if needed.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max > 3 {
        format!("{}...", &s[..max - 3])
    } else {
        s[..max].to_string()
    }
}

fn format_time(instant: Instant) -> String {
    // Use system time offset from session start
    let now = chrono::Local::now();
    let _ = instant; // we use wall clock, not the instant
    now.format("%-I:%M %p").to_string()
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Resolve the best backend URL for an agent.
///
/// Priority: sidecar (from `.roko/runtime/agents.json`) → serve URL.
fn resolve_chat_backend(agent_id: &str, serve_url: &str) -> (String, bool) {
    let workdir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    // Try sidecar first
    if let Some(sidecar_url) = chat::lookup_sidecar_url(agent_id, &workdir) {
        return (sidecar_url, true);
    }
    (serve_url.to_string(), false)
}

/// Run the inline chat REPL.
///
/// If stdout is not a TTY, falls back to the legacy line-oriented REPL.
pub async fn run_chat_inline(agent_id: &str, serve_url: &str) -> Result<()> {
    if !crate::inline::should_use_inline() {
        return chat::run_chat_repl(agent_id, serve_url).await;
    }

    let api_key =
        auth::resolve_api_key(&roko_core::config::ServeAuthConfig::default(), None).map(|r| r.key);

    let mut client_builder = reqwest::Client::builder();
    if let Some(ref key) = api_key {
        client_builder = client_builder.default_headers(auth::auth_headers(key));
    }
    let client = client_builder.build().context("build HTTP client")?;

    // Discover sidecar or fall back to serve
    let (backend_url, is_sidecar) = resolve_chat_backend(agent_id, serve_url);
    let backend_label = if is_sidecar {
        format!("sidecar @ {backend_url}")
    } else {
        backend_url.clone()
    };

    let mut term = InlineTerminal::new().context("init inline terminal")?;
    let theme = *term.theme();

    // Push welcome banner
    let version = env!("CARGO_PKG_VERSION");
    term.push_lines(&[
        styled::section_start(
            &theme,
            "roko chat",
            &format!("v{version}  {}  {agent_id}", symbols::SEP),
            Some(&backend_label),
        ),
        Line::from(vec![
            Span::styled(symbols::END.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(
                "Type a message. Ctrl-D to exit. /help for commands.".to_string(),
                Style::default().fg(Theme::TEXT_DIM),
            ),
        ]),
    ])?;
    if let Some((saved_at, turns, cost, topic)) = load_last_session_summary() {
        term.push_lines(&[
            Line::from(vec![
                Span::styled(format!("{} ", symbols::BAR), theme.muted()),
                Span::styled(
                    format!(
                        "Last: {saved_at}  {}  {turns} turns  {}  ${cost:.4}",
                        symbols::SEP,
                        symbols::SEP
                    ),
                    Style::default().fg(Theme::TEXT_GHOST),
                ),
            ]),
            Line::from(vec![
                Span::styled(format!("{} ", symbols::BAR), theme.muted()),
                Span::styled(
                    format!("\"{topic}\""),
                    Style::default().fg(Theme::TEXT_GHOST),
                ),
            ]),
        ])?;
    }
    term.push_lines(&[Line::raw("")])?;

    let cost_table = CostTable {
        models: HashMap::new(),
    }
    .with_defaults();

    let mut input = InputState::new();
    input.history = load_history();

    let mut session = ChatSession {
        phase: Phase::Input,
        input,
        streaming: StreamingState::new("unknown"),
        cost: CostMeter::new(),
        cost_table,
        agent_id: agent_id.to_string(),
        tick: 0,
        started_at: Instant::now(),
        dispatch: DispatchMode::Http {
            client,
            backend_url,
            is_sidecar,
            serve_url: serve_url.to_string(),
        },
        response_rx: None,
        turn_count: 0,
        thinking_started: None,
        conversation: Vec::new(),
        last_prompt: None,
        system_message: None,
        compact: false,
        agent_session: None,
    };

    // Main event loop
    loop {
        // Draw the viewport
        term.draw(|frame| {
            render_viewport(frame, &session, &theme);
        })?;

        // Poll for events (30fps = ~33ms)
        if event::poll(Duration::from_millis(33)).context("poll events")? {
            if let Event::Key(key) = event::read().context("read event")? {
                match session.phase {
                    Phase::Input => {
                        if handle_input_key(key, &mut session, &mut term, &theme).await? {
                            break; // exit signal
                        }
                    }
                    Phase::Thinking | Phase::Streaming => {
                        // Ctrl+C interrupts generation
                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            // Push partial output to scrollback
                            if !session.streaming.is_empty() {
                                let text = session.streaming.take_buffer();
                                push_agent_response(&mut term, &theme, &text, &session.agent_id)?;
                            }
                            term.push_lines(&[styled::continuation(
                                &theme,
                                "",
                                "[interrupted]",
                                None,
                            )])?;
                            session.phase = Phase::Input;
                        }
                    }
                    Phase::Error { ref prompt, .. } => {
                        match key.code {
                            KeyCode::Char('r') => {
                                // Retry: resend same prompt
                                let retry_prompt = prompt.clone();
                                session.phase = Phase::Thinking;
                                session.thinking_started = Some(Instant::now());
                                dispatch_prompt(&mut session, &retry_prompt);
                            }
                            KeyCode::Char('q') | KeyCode::Esc => {
                                // Cancel: return to input
                                term.push_blank()?;
                                session.phase = Phase::Input;
                            }
                            _ => {}
                        }
                    }
                    Phase::Done => break,
                }
            }
        }

        // Check for async response from background HTTP call
        if let Some(ref mut rx) = session.response_rx {
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    let latency = session
                        .thinking_started
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(0.0);
                    push_tool_outputs(&mut term, &theme, &result.tool_outputs)?;
                    push_agent_response(&mut term, &theme, &result.text, &session.agent_id)?;
                    let ts = format_time(Instant::now());
                    term.push_lines(&[Line::from(vec![Span::styled(
                        format!("  {ts} ({latency:.1}s)"),
                        Style::default().fg(Theme::TEXT_GHOST),
                    )])])?;
                    session.conversation.push(ConversationMessage {
                        role: "assistant".into(),
                        text: result.text.clone(),
                        timestamp: format!("{ts} ({latency:.1}s)"),
                    });
                    let cost = cost_from_result(&session.cost_table, &result);
                    let naive = naive_opus_cost(result.input_tokens, result.output_tokens);
                    session.cost.record_run(
                        cost,
                        result.input_tokens,
                        result.output_tokens,
                        &result.model,
                        naive,
                    );
                    if matches!(&session.dispatch, DispatchMode::Session) {
                        if let Some(agent_session) = session.agent_session.as_mut() {
                            if let Some(session_id) = result.session_id.clone() {
                                agent_session.session_id = Some(session_id);
                            }
                        }
                    }
                    session.turn_count += 1;
                    session.thinking_started = None;
                    if latency > 10.0 {
                        print!("\x07");
                    }
                    // Auto-save every 5 turns
                    if session.turn_count % 5 == 0 {
                        save_session(&session);
                    }
                    session.phase = Phase::Input;
                    session.response_rx = None;
                    term.push_blank()?;
                }
                Ok(Err(err)) => {
                    if err == "__cancelled__" {
                        session.thinking_started = None;
                        session.phase = Phase::Input;
                        session.response_rx = None;
                    } else {
                        push_error_with_suggestions(&mut term, &theme, &err)?;
                        let prompt = session.last_prompt.clone().unwrap_or_default();
                        session.thinking_started = None;
                        session.phase = Phase::Error { prompt, error: err };
                        session.response_rx = None;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    // Still waiting — spinner continues
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    term.push_lines(&[styled::continuation(
                        &theme,
                        "error",
                        "response channel closed",
                        None,
                    )])?;
                    session.phase = Phase::Input;
                    session.response_rx = None;
                }
            }
        }

        session.tick += 1;
        session.streaming.tick();
    }

    // Push session summary before exit
    if session.cost.run_count > 0 {
        term.push_blank()?;
        let ratio = session.cost.savings_ratio();
        let summary_line = if ratio > 1.5 {
            format!(
                "{} turns  {}  ${:.4} total  {}  {:.1}x savings vs baseline",
                session.cost.run_count,
                symbols::SEP,
                session.cost.total_cost,
                symbols::SEP,
                ratio,
            )
        } else {
            format!(
                "{} turns  {}  ${:.4} total",
                session.cost.run_count,
                symbols::SEP,
                session.cost.total_cost,
            )
        };
        term.push_lines(&[styled::section_start(
            &theme,
            "session",
            &summary_line,
            None,
        )])?;
    }

    term.push_blank()?;
    save_session(&session);
    drop(term); // restores terminal

    Ok(())
}

/// Run the unified inline chat, preferring `ChatAgentSession` and falling
/// back to direct dispatch when session setup fails.
///
/// This is the primary entry point for `roko` with no subcommand.
/// Uses [`AuthMethod`] to dispatch prompts directly via Claude CLI or API.
pub async fn run_unified_inline(auth: &AuthMethod) -> Result<()> {
    if !crate::inline::should_use_inline() {
        // Fallback: non-TTY one-shot via dispatch_direct
        eprintln!("hint: stdin is not a TTY — use `roko \"prompt\"` for one-shot mode");
        return Ok(());
    }

    let mut term = InlineTerminal::new().context("init inline terminal")?;
    let theme = *term.theme();

    // Push welcome banner
    let version = env!("CARGO_PKG_VERSION");
    let workspace = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let roko_initialized = std::path::Path::new(".roko").exists();
    let init_status = if roko_initialized {
        ".roko/ initialized"
    } else {
        ".roko/ not found"
    };
    term.push_lines(&[
        styled::section_start(
            &theme,
            "roko",
            &format!("v{version}  {}  {}", symbols::SEP, auth.label()),
            None,
        ),
        styled::continuation(&theme, "workspace", &workspace, Some(init_status)),
        Line::from(vec![
            Span::styled(symbols::END.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(
                "Type a message. Ctrl-D to exit. /help for commands.".to_string(),
                Style::default().fg(Theme::TEXT_DIM),
            ),
        ]),
    ])?;
    if let Some((saved_at, turns, cost, topic)) = load_last_session_summary() {
        term.push_lines(&[
            Line::from(vec![
                Span::styled(format!("{} ", symbols::BAR), theme.muted()),
                Span::styled(
                    format!(
                        "Last: {saved_at}  {}  {turns} turns  {}  ${cost:.4}",
                        symbols::SEP,
                        symbols::SEP
                    ),
                    Style::default().fg(Theme::TEXT_GHOST),
                ),
            ]),
            Line::from(vec![
                Span::styled(format!("{} ", symbols::BAR), theme.muted()),
                Span::styled(
                    format!("\"{topic}\""),
                    Style::default().fg(Theme::TEXT_GHOST),
                ),
            ]),
        ])?;
    }
    term.push_lines(&[Line::raw("")])?;

    let cost_table = CostTable {
        models: HashMap::new(),
    }
    .with_defaults();

    let mut input = InputState::new();
    input.history = load_history();

    let workdir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let (dispatch, agent_session, system_message) = match crate::config::load_layered(&workdir) {
        Ok(resolved) => {
            let config = resolved.config;
            let mut model_config = roko_core::config::schema::RokoConfig::default();
            model_config.providers.extend(config.providers.clone());
            model_config.models.extend(config.models.clone());
            if let Some(model) = config.agent.model.clone() {
                model_config.agent.default_model = model;
            }
            model_config.agent.default_effort = config.agent.effort.clone();
            model_config.agent.bare_mode = config.agent.bare_mode;
            model_config.agent.timeout_ms = Some(config.agent.timeout_ms);
            model_config.agent.fallback_model = config.agent.fallback_model.clone();
            model_config.agent.tier_models = config.agent.tier_models.clone();
            model_config.agent.env = Some(config.agent.env.clone());

            let role = {
                let role = config.prompt.role.trim();
                (!role.is_empty()).then(|| role.to_string())
            };

            match crate::model_selection::resolve_effective_model(
                None,
                None,
                role,
                None,
                &model_config,
            ) {
                Ok(selection) if selection.provider_kind == ProviderKind::ClaudeCli.label() => {
                    match ChatAgentSession::new(&config, workdir.clone(), selection) {
                        Ok(agent_session) => {
                            let system_message = Some(agent_session.system_prompt.clone());
                            (DispatchMode::Session, Some(agent_session), system_message)
                        }
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                "ChatAgentSession init failed; using direct dispatch"
                            );
                            (DispatchMode::Direct { auth: auth.clone() }, None, None)
                        }
                    }
                }
                Ok(selection) => {
                    tracing::warn!(
                        provider = %selection.provider_kind,
                        model = %selection.effective_model_key,
                        "interactive chat resolved to unsupported provider; using direct dispatch"
                    );
                    (DispatchMode::Direct { auth: auth.clone() }, None, None)
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to resolve interactive chat model; using direct dispatch"
                    );
                    (DispatchMode::Direct { auth: auth.clone() }, None, None)
                }
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to load interactive chat config; using direct dispatch"
            );
            (DispatchMode::Direct { auth: auth.clone() }, None, None)
        }
    };

    let mut session = ChatSession {
        phase: Phase::Input,
        input,
        streaming: StreamingState::new("unknown"),
        cost: CostMeter::new(),
        cost_table,
        agent_id: "roko".to_string(),
        tick: 0,
        started_at: Instant::now(),
        dispatch,
        response_rx: None,
        turn_count: 0,
        thinking_started: None,
        conversation: Vec::new(),
        last_prompt: None,
        system_message,
        compact: false,
        agent_session,
    };

    // Main event loop (identical structure to run_chat_inline)
    loop {
        term.draw(|frame| {
            render_viewport(frame, &session, &theme);
        })?;

        if event::poll(Duration::from_millis(33)).context("poll events")? {
            if let Event::Key(key) = event::read().context("read event")? {
                match session.phase {
                    Phase::Input => {
                        if handle_input_key(key, &mut session, &mut term, &theme).await? {
                            break;
                        }
                    }
                    Phase::Thinking | Phase::Streaming => {
                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            if !session.streaming.is_empty() {
                                let text = session.streaming.take_buffer();
                                push_agent_response(&mut term, &theme, &text, &session.agent_id)?;
                            }
                            term.push_lines(&[styled::continuation(
                                &theme,
                                "",
                                "[interrupted]",
                                None,
                            )])?;
                            session.phase = Phase::Input;
                        }
                    }
                    Phase::Error { ref prompt, .. } => match key.code {
                        KeyCode::Char('r') => {
                            let retry_prompt = prompt.clone();
                            session.phase = Phase::Thinking;
                            session.thinking_started = Some(Instant::now());
                            dispatch_prompt(&mut session, &retry_prompt);
                        }
                        KeyCode::Char('q') | KeyCode::Esc => {
                            term.push_blank()?;
                            session.phase = Phase::Input;
                        }
                        _ => {}
                    },
                    Phase::Done => break,
                }
            }
        }

        // Check for async response
        if let Some(ref mut rx) = session.response_rx {
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    let latency = session
                        .thinking_started
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(0.0);
                    push_tool_outputs(&mut term, &theme, &result.tool_outputs)?;
                    push_agent_response(&mut term, &theme, &result.text, &session.agent_id)?;
                    let ts = format_time(Instant::now());
                    term.push_lines(&[Line::from(vec![Span::styled(
                        format!("  {ts} ({latency:.1}s)"),
                        Style::default().fg(Theme::TEXT_GHOST),
                    )])])?;
                    session.conversation.push(ConversationMessage {
                        role: "assistant".into(),
                        text: result.text.clone(),
                        timestamp: format!("{ts} ({latency:.1}s)"),
                    });
                    let cost = cost_from_result(&session.cost_table, &result);
                    let naive = naive_opus_cost(result.input_tokens, result.output_tokens);
                    session.cost.record_run(
                        cost,
                        result.input_tokens,
                        result.output_tokens,
                        &result.model,
                        naive,
                    );
                    if matches!(&session.dispatch, DispatchMode::Session) {
                        if let Some(agent_session) = session.agent_session.as_mut() {
                            if let Some(session_id) = result.session_id.clone() {
                                agent_session.session_id = Some(session_id);
                            }
                        }
                    }
                    session.turn_count += 1;
                    session.thinking_started = None;
                    if latency > 10.0 {
                        print!("\x07");
                    }
                    // Auto-save every 5 turns
                    if session.turn_count % 5 == 0 {
                        save_session(&session);
                    }
                    session.phase = Phase::Input;
                    session.response_rx = None;
                    term.push_blank()?;
                }
                Ok(Err(err)) => {
                    if err == "__cancelled__" {
                        session.thinking_started = None;
                        session.phase = Phase::Input;
                        session.response_rx = None;
                    } else {
                        push_error_with_suggestions(&mut term, &theme, &err)?;
                        let prompt = session.last_prompt.clone().unwrap_or_default();
                        session.thinking_started = None;
                        session.phase = Phase::Error { prompt, error: err };
                        session.response_rx = None;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    term.push_lines(&[styled::continuation(
                        &theme,
                        "error",
                        "response channel closed",
                        None,
                    )])?;
                    session.thinking_started = None;
                    session.phase = Phase::Input;
                    session.response_rx = None;
                }
            }
        }

        session.tick += 1;
        session.streaming.tick();
    }

    // Session summary
    if session.cost.run_count > 0 {
        term.push_blank()?;
        let ratio = session.cost.savings_ratio();
        let summary_line = if ratio > 1.5 {
            format!(
                "{} turns  {}  ${:.4} total  {}  {:.1}x savings vs baseline",
                session.cost.run_count,
                symbols::SEP,
                session.cost.total_cost,
                symbols::SEP,
                ratio,
            )
        } else {
            format!(
                "{} turns  {}  ${:.4} total",
                session.cost.run_count,
                symbols::SEP,
                session.cost.total_cost,
            )
        };
        term.push_lines(&[styled::section_start(
            &theme,
            "session",
            &summary_line,
            None,
        )])?;
    }

    term.push_blank()?;
    save_session(&session);
    drop(term);

    Ok(())
}

// ---------------------------------------------------------------------------
// Input handling
// ---------------------------------------------------------------------------

/// Spawn async dispatch for a prompt, setting up the response channel.
fn dispatch_prompt(session: &mut ChatSession, prompt: &str) {
    session.streaming = StreamingState::new("resolving...");
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    session.response_rx = Some(rx);
    let text = prompt.to_string();

    match &session.dispatch {
        DispatchMode::Http {
            client,
            backend_url,
            is_sidecar,
            ..
        } => {
            let client_clone = client.clone();
            let url_owned = backend_url.clone();
            let agent_id_owned = session.agent_id.clone();
            let sidecar = *is_sidecar;
            tokio::spawn(async move {
                let result =
                    send_and_receive(&client_clone, &url_owned, &agent_id_owned, &text, sidecar)
                        .await;
                let _ = tx
                    .send(result.map(|r| r.into()).map_err(|e| e.to_string()))
                    .await;
            });
        }
        DispatchMode::Direct { auth } => {
            let auth_clone = auth.clone();
            tokio::spawn(async move {
                #[cfg(feature = "legacy-orchestrate")]
                let result = dispatch_direct::dispatch_prompt(&auth_clone, &text).await;
                #[cfg(not(feature = "legacy-orchestrate"))]
                let result = crate::dispatch_v2::dispatch_via_model_call_service(&text).await;
                let _ = tx.send(result.map_err(|e| e.to_string())).await;
            });
        }
        DispatchMode::Session => {
            let Some(agent_session) = session.agent_session.as_ref() else {
                let _ = tx.try_send(Err("agent session unavailable".to_string()));
                return;
            };

            let mut agent_session = clone_chat_agent_session(agent_session);
            let model = agent_session.model.clone();
            tokio::spawn(async move {
                let (event_tx, mut event_rx) =
                    tokio::sync::mpsc::channel::<roko_agent::AgentRuntimeEvent>(256);
                let drain_handle =
                    tokio::spawn(async move { while let Some(_event) = event_rx.recv().await {} });

                let result = agent_session.send_turn_streaming(&text, event_tx).await;
                let _ = drain_handle.await;

                let mapped = match result {
                    Ok(turn) if turn.cancelled => Err("__cancelled__".to_string()),
                    Ok(turn) => Ok(turn_result_to_dispatch_result(turn, model)),
                    Err(error) => Err(error.to_string()),
                };
                let _ = tx.send(mapped).await;
            });
        }
    }
}

/// Returns true if the session should exit.
async fn handle_input_key(
    key: KeyEvent,
    session: &mut ChatSession,
    term: &mut InlineTerminal,
    theme: &Theme,
) -> Result<bool> {
    // --- Reverse history search mode (Ctrl+R) ---
    if session.input.search.active {
        match key.code {
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+R again: cycle to next match
                session.input.search.next_match();
                if let Some(entry) = session.input.search.current_match(&session.input.history) {
                    session.input.buffer = entry.to_string();
                    session.input.cursor = session.input.buffer.len();
                }
            }
            KeyCode::Enter => {
                // Accept match and populate buffer
                if let Some(idx) = session.input.search.accept() {
                    if let Some(entry) = session.input.history.get(idx) {
                        session.input.buffer = entry.clone();
                        session.input.cursor = session.input.buffer.len();
                    }
                } else {
                    session.input.search.deactivate();
                }
            }
            KeyCode::Esc | KeyCode::Char('c')
                if key.code == KeyCode::Esc || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                // Cancel search, restore original buffer
                session.input.search.deactivate();
                session.input.buffer.clear();
                session.input.cursor = 0;
            }
            KeyCode::Backspace => {
                session.input.search.query.pop();
                session.input.search.match_idx = 0;
                session.input.search.update(&session.input.history);
                if let Some(entry) = session.input.search.current_match(&session.input.history) {
                    session.input.buffer = entry.to_string();
                    session.input.cursor = session.input.buffer.len();
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                session.input.search.query.push(ch);
                session.input.search.match_idx = 0;
                session.input.search.update(&session.input.history);
                if let Some(entry) = session.input.search.current_match(&session.input.history) {
                    session.input.buffer = entry.to_string();
                    session.input.cursor = session.input.buffer.len();
                }
            }
            _ => {
                // Any other key: accept current match and pass through
                if let Some(idx) = session.input.search.accept() {
                    if let Some(entry) = session.input.history.get(idx) {
                        session.input.buffer = entry.clone();
                        session.input.cursor = session.input.buffer.len();
                    }
                } else {
                    session.input.search.deactivate();
                }
            }
        }
        return Ok(false);
    }

    // --- Command palette mode (Ctrl+K) ---
    if session.input.palette.active {
        match key.code {
            KeyCode::Esc => {
                session.input.palette.dismiss();
            }
            KeyCode::Enter => {
                if let Some(cmd) = session.input.palette.accept() {
                    // Execute the command directly
                    return handle_slash_command(&cmd, session, term, theme);
                }
            }
            KeyCode::Up => session.input.palette.select_prev(),
            KeyCode::Down => session.input.palette.select_next(),
            KeyCode::Backspace => {
                if session.input.palette.query.is_empty() {
                    session.input.palette.dismiss();
                } else {
                    session.input.palette.backspace();
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                session.input.palette.type_char(ch);
            }
            _ => {}
        }
        return Ok(false);
    }

    let dropdown_visible = session.input.completion.visible;

    match key.code {
        // --- Shift+Enter: insert newline ---
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
            session.input.insert_newline();
            session.input.completion.dismiss();
        }

        // --- Enter ---
        KeyCode::Enter => {
            if dropdown_visible {
                // Accept the selected completion into buffer (don't submit)
                if let Some(cmd) = session.input.completion.accept() {
                    session.input.buffer = cmd;
                    session.input.cursor = session.input.buffer.len();
                }
                return Ok(false);
            }
            if session.input.is_empty() {
                return Ok(false);
            }
            session.input.completion.dismiss();
            let text = session.input.submit();

            // Persist to history file
            save_history_entry(&text);

            if let Some(handled) = handle_agent_session_slash_command(&text, session, term, theme)?
            {
                return Ok(handled);
            }

            // Handle / commands
            if text.starts_with('/') {
                return handle_slash_command(&text, session, term, theme);
            }

            // Push user message to scrollback with timestamp
            let timestamp = format_time(Instant::now());
            if text.contains('\n') {
                // Multi-line: first line with prompt, rest with bar prefix
                let msg_lines: Vec<&str> = text.split('\n').collect();
                let mut scroll_lines = vec![Line::from(vec![
                    Span::styled(
                        format!("{} ", symbols::PROMPT),
                        Style::default().fg(Theme::ROSE),
                    ),
                    Span::styled(msg_lines[0].to_string(), Style::default().fg(Theme::BONE)),
                    Span::styled(
                        format!("  {timestamp}"),
                        Style::default().fg(Theme::TEXT_GHOST),
                    ),
                ])];
                for line in &msg_lines[1..] {
                    scroll_lines.push(Line::from(vec![
                        Span::styled(
                            format!("{} ", symbols::BAR),
                            Style::default().fg(Theme::TEXT_DIM),
                        ),
                        Span::styled(line.to_string(), Style::default().fg(Theme::BONE)),
                    ]));
                }
                term.push_lines(&scroll_lines)?;
            } else {
                term.push_lines(&[Line::from(vec![
                    Span::styled(
                        format!("{} ", symbols::PROMPT),
                        Style::default().fg(Theme::ROSE),
                    ),
                    Span::styled(text.clone(), Style::default().fg(Theme::BONE)),
                    Span::styled(
                        format!("  {timestamp}"),
                        Style::default().fg(Theme::TEXT_GHOST),
                    ),
                ])])?;
            }

            // Record user message in conversation
            session.conversation.push(ConversationMessage {
                role: "user".into(),
                text: text.clone(),
                timestamp: timestamp.clone(),
            });

            // Store for potential retry
            session.last_prompt = Some(text.clone());

            // Start thinking phase
            session.phase = Phase::Thinking;
            session.thinking_started = Some(Instant::now());
            dispatch_prompt(session, &text);
        }

        // --- Escape: dismiss dropdown ---
        KeyCode::Esc => {
            session.input.completion.dismiss();
        }

        // --- Exit ---
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session.phase = Phase::Done;
            return Ok(true);
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if session.input.is_empty() {
                session.phase = Phase::Done;
                return Ok(true);
            }
            session.input.clear();
            session.input.completion.dismiss();
        }

        // --- Editing ---
        KeyCode::Backspace => {
            session.input.backspace();
            session.input.completion.update(&session.input.buffer);
        }
        KeyCode::Delete => session.input.delete(),
        KeyCode::Left => {
            session.input.move_left();
            session.input.completion.dismiss();
        }
        KeyCode::Right => {
            if !dropdown_visible
                && session.input.cursor == session.input.buffer.len()
                && session.input.accept_ghost()
            {
                // Accepted ghost suggestion
            } else {
                session.input.move_right();
                session.input.completion.dismiss();
            }
        }
        KeyCode::Home | KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session.input.home();
            session.input.completion.dismiss();
        }
        KeyCode::End | KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if !dropdown_visible
                && session.input.cursor == session.input.buffer.len()
                && session.input.accept_ghost()
            {
                // Accepted ghost suggestion
            } else {
                session.input.end();
                session.input.completion.dismiss();
            }
        }

        // --- Tab / Shift+Tab: completion navigation ---
        KeyCode::Tab => {
            if dropdown_visible {
                session.input.completion.select_next();
            } else {
                // Open dropdown if buffer starts with /
                session.input.completion.update(&session.input.buffer);
            }
        }
        KeyCode::BackTab => {
            if dropdown_visible {
                session.input.completion.select_prev();
            }
        }

        // --- Up / Down: dropdown nav or history ---
        KeyCode::Up => {
            if dropdown_visible {
                session.input.completion.select_prev();
            } else {
                session.input.history_up();
            }
        }
        KeyCode::Down => {
            if dropdown_visible {
                session.input.completion.select_next();
            } else {
                session.input.history_down();
            }
        }

        // --- Clear line ---
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session.input.clear();
            session.input.completion.dismiss();
        }

        // --- Clear screen ---
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            for _ in 0..term.viewport_height() {
                term.push_blank()?;
            }
        }

        // --- Command palette ---
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session.input.completion.dismiss();
            session.input.palette.open();
        }

        // --- Reverse history search ---
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session.input.search.active = true;
            session.input.search.query.clear();
            session.input.search.match_idx = 0;
            session.input.search.matches.clear();
            session.input.completion.dismiss();
        }

        // --- Regular character input ---
        KeyCode::Char(ch) => {
            session.input.insert(ch);
            // Update completion if the buffer starts with /
            if session.input.buffer.trim_start().starts_with('/') {
                session.input.completion.update(&session.input.buffer);
            } else {
                session.input.completion.dismiss();
            }
        }

        _ => {}
    }

    Ok(false)
}

/// Run a shell command and return stdout (or stderr if stdout is empty).
fn shell_output(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .current_dir(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")))
        .output()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            if stdout.trim().is_empty() {
                stderr
            } else {
                stdout
            }
        })
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Push shell output as scrollback lines (truncated to max_lines).
fn push_shell_output(
    term: &mut InlineTerminal,
    theme: &Theme,
    label: &str,
    output: &str,
    max_lines: usize,
) -> std::io::Result<()> {
    let lines: Vec<&str> = output.lines().collect();
    let truncated = lines.len() > max_lines;
    let mut styled_lines = vec![styled::section_start(theme, label, "", None)];
    for line in lines.iter().take(max_lines) {
        styled_lines.push(Line::from(vec![
            Span::styled(format!("{} ", symbols::BAR), theme.muted()),
            Span::styled(line.to_string(), theme.text()),
        ]));
    }
    if truncated {
        styled_lines.push(styled::continuation(
            theme,
            "",
            &format!("... ({} more lines)", lines.len() - max_lines),
            None,
        ));
    }
    styled_lines.push(Line::from(vec![Span::styled(
        symbols::END.to_string(),
        theme.muted(),
    )]));
    term.push_lines(&styled_lines)
}

/// Read roko.toml and return its content.
fn read_roko_toml() -> Option<String> {
    let path = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("roko.toml");
    std::fs::read_to_string(path).ok()
}

/// Let `ChatAgentSession` handle slash commands that mutate its own state.
fn handle_agent_session_slash_command(
    text: &str,
    session: &mut ChatSession,
    term: &mut InlineTerminal,
    theme: &Theme,
) -> Result<Option<bool>> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }

    let result = {
        session
            .agent_session
            .as_mut()
            .map(|agent_session| agent_session.handle_slash_command(trimmed))
    };

    let Some(result) = result else {
        return Ok(None);
    };

    match result {
        SlashResult::Updated(msg) => {
            if trimmed.starts_with("/system") || trimmed.starts_with("/reset") {
                session.system_message = active_system_prompt(session);
            }
            if trimmed.starts_with("/reset") {
                session.conversation.clear();
                session.turn_count = 0;
                session.cost = CostMeter::new();
                session.last_prompt = None;
            }
            term.push_lines(&[styled::continuation(theme, "config", &msg, None)])?;
            Ok(Some(false))
        }
        SlashResult::Error(msg) => {
            term.push_lines(&[styled::continuation(theme, "error", &msg, None)])?;
            Ok(Some(false))
        }
        SlashResult::Unknown(_) | SlashResult::NotACommand => Ok(None),
    }
}

/// Handle `/` commands. Returns true if the session should exit.
fn handle_slash_command(
    text: &str,
    session: &mut ChatSession,
    term: &mut InlineTerminal,
    theme: &Theme,
) -> Result<bool> {
    let cmd = text.trim();
    if let Some(exit) = handle_agent_session_slash_command(cmd, session, term, theme)? {
        return Ok(exit);
    }
    match cmd {
        // =================================================================
        // Session & display
        // =================================================================
        "/quit" | "/exit" | "/q" => {
            session.phase = Phase::Done;
            return Ok(true);
        }
        "/help" | "/h" => {
            term.push_lines(&[
                styled::section_start(theme, "help", "session & display", None),
                styled::continuation(theme, "/model <name>", "show or change model", None),
                styled::continuation(theme, "/provider", "show auth/provider info", None),
                styled::continuation(theme, "/cost", "session cost summary", None),
                styled::continuation(theme, "/stats", "detailed session statistics", None),
                styled::continuation(theme, "/context", "token/context usage", None),
                styled::continuation(theme, "/tools", "list available tools", None),
                styled::continuation(theme, "/version", "version info", None),
                styled::continuation(theme, "/history", "show input history", None),
                styled::continuation(theme, "/copy", "copy last response to clipboard", None),
                styled::continuation(theme, "/compact", "toggle compact output", None),
                styled::continuation(theme, "/system <text>", "set system message", None),
                styled::continuation(theme, "/reset", "clear conversation", None),
                styled::continuation(theme, "/retry", "resend last message", None),
                styled::continuation(theme, "/export [md|json]", "export conversation", None),
                styled::continuation(theme, "/clear", "clear scrollback", None),
            ])?;
            term.push_lines(&[
                styled::section_start(theme, "", "configuration", None),
                styled::continuation(theme, "/config", "show config summary", None),
                styled::continuation(theme, "/config providers", "list providers", None),
                styled::continuation(theme, "/config models", "list all models", None),
                styled::continuation(theme, "/config gates", "gate configuration", None),
                styled::continuation(theme, "/config set <k> <v>", "set config value", None),
                styled::continuation(
                    theme,
                    "/effort <level>",
                    "set effort (low/med/high/max)",
                    None,
                ),
                styled::continuation(theme, "/gate <name> on|off", "toggle gate", None),
            ])?;
            term.push_lines(&[
                styled::section_start(theme, "", "workspace & git", None),
                styled::continuation(theme, "/status", "workspace status", None),
                styled::continuation(theme, "/doctor", "health check", None),
                styled::continuation(theme, "/diff", "git diff", None),
                styled::continuation(theme, "/git", "git status", None),
                styled::continuation(theme, "/log [n]", "recent commits", None),
                styled::continuation(theme, "/branch", "current branch", None),
                styled::continuation(theme, "/changes", "changed files", None),
            ])?;
            term.push_lines(&[
                styled::section_start(theme, "", "files & search", None),
                styled::continuation(theme, "/file <path>", "read a file", None),
                styled::continuation(theme, "/search <pattern>", "grep workspace", None),
                styled::continuation(theme, "/find <pattern>", "find files", None),
                styled::continuation(theme, "/tree [path]", "directory tree", None),
            ])?;
            term.push_lines(&[
                styled::section_start(theme, "", "agents & workflows", None),
                styled::continuation(theme, "/agent [name]", "show/switch agent", None),
                styled::continuation(theme, "/run <prompt>", "universal loop", None),
                styled::continuation(theme, "/plan list|run|generate", "plan management", None),
                styled::continuation(theme, "/prd idea|list", "PRD management", None),
                styled::continuation(theme, "/research <query>", "research a topic", None),
                styled::continuation(theme, "/knowledge <query>", "query knowledge", None),
                styled::continuation(theme, "/learn", "learning state", None),
                styled::section_end(theme, "/quit", "exit the chat"),
            ])?;
        }
        "/version" | "/v" => {
            let version = env!("CARGO_PKG_VERSION");
            let rustc = shell_output("rustc", &["--version"]);
            term.push_lines(&[
                styled::section_start(theme, "version", "", None),
                styled::continuation(theme, "roko", &format!("v{version}"), None),
                styled::continuation(theme, "rustc", rustc.trim(), None),
                styled::continuation(theme, "platform", std::env::consts::OS, None),
                styled::section_end(theme, "arch", std::env::consts::ARCH),
            ])?;
        }
        "/stats" => {
            let elapsed = session.started_at.elapsed();
            let mins = elapsed.as_secs() / 60;
            let secs = elapsed.as_secs() % 60;
            let avg_cost = if session.turn_count > 0 {
                session.cost.total_cost / session.turn_count as f64
            } else {
                0.0
            };
            let avg_tokens = if session.turn_count > 0 {
                (session.cost.input_tokens + session.cost.output_tokens) / session.turn_count as u64
            } else {
                0
            };
            term.push_lines(&[
                styled::section_start(theme, "stats", "session details", None),
                styled::continuation(theme, "elapsed", &format!("{mins}m {secs}s"), None),
                styled::continuation(theme, "turns", &session.turn_count.to_string(), None),
                styled::continuation(
                    theme,
                    "total cost",
                    &format!("${:.4}", session.cost.total_cost),
                    None,
                ),
                styled::continuation(
                    theme,
                    "avg/turn",
                    &format!("${avg_cost:.4} cost, {avg_tokens} tokens"),
                    None,
                ),
                styled::continuation(
                    theme,
                    "tokens in",
                    &session.cost.input_tokens.to_string(),
                    None,
                ),
                styled::continuation(
                    theme,
                    "tokens out",
                    &session.cost.output_tokens.to_string(),
                    None,
                ),
                styled::continuation(
                    theme,
                    "messages",
                    &session.conversation.len().to_string(),
                    None,
                ),
                styled::section_end(
                    theme,
                    "savings",
                    &format!("{:.1}x vs baseline", session.cost.savings_ratio()),
                ),
            ])?;
        }
        "/context" => {
            let total = session.cost.input_tokens + session.cost.output_tokens;
            let limit: u64 = 200_000; // from roko.toml context_limit_k
            let pct = (total as f64 / limit as f64 * 100.0).min(100.0);
            let bar_width = 20;
            let filled = ((pct / 100.0) * bar_width as f64) as usize;
            let bar: String = format!("{}{}", "━".repeat(filled), "░".repeat(bar_width - filled));
            term.push_lines(&[
                styled::section_start(theme, "context", "", None),
                styled::continuation(theme, "used", &format!("{total} / {limit} tokens"), None),
                styled::continuation(theme, "usage", &format!("{bar}  {pct:.0}%"), None),
                styled::section_end(theme, "turns", &session.turn_count.to_string()),
            ])?;
        }
        "/history" => {
            let history = &session.input.history;
            if history.is_empty() {
                term.push_lines(&[styled::continuation(theme, "history", "no history", None)])?;
            } else {
                let start = history.len().saturating_sub(20);
                let mut lines = vec![styled::section_start(
                    theme,
                    "history",
                    &format!(
                        "{} entries (showing last {})",
                        history.len(),
                        history.len() - start
                    ),
                    None,
                )];
                for (i, entry) in history[start..].iter().enumerate() {
                    let display = if entry.len() > 60 {
                        format!("{}...", &entry[..57])
                    } else {
                        entry.clone()
                    };
                    lines.push(styled::continuation(
                        theme,
                        &format!("{}", start + i + 1),
                        &display,
                        None,
                    ));
                }
                lines.push(Line::from(vec![Span::styled(
                    symbols::END.to_string(),
                    theme.muted(),
                )]));
                term.push_lines(&lines)?;
            }
        }
        "/copy" => {
            if let Some(last) = session
                .conversation
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
            {
                // Try pbcopy (macOS), xclip (Linux), or xsel
                let result = std::process::Command::new("pbcopy")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        if let Some(ref mut stdin) = child.stdin {
                            use std::io::Write;
                            stdin.write_all(last.text.as_bytes())?;
                        }
                        child.wait()
                    });
                match result {
                    Ok(status) if status.success() => {
                        let preview = if last.text.len() > 50 {
                            format!("{}...", &last.text[..47])
                        } else {
                            last.text.clone()
                        };
                        term.push_lines(&[styled::continuation(
                            theme,
                            "copy",
                            "copied to clipboard",
                            Some(&preview),
                        )])?;
                    }
                    _ => {
                        term.push_lines(&[styled::continuation(
                            theme,
                            "copy",
                            "clipboard not available",
                            Some("pbcopy/xclip not found"),
                        )])?;
                    }
                }
            } else {
                term.push_lines(&[styled::continuation(
                    theme,
                    "copy",
                    "no response to copy",
                    None,
                )])?;
            }
        }
        "/compact" => {
            session.compact = !session.compact;
            let state = if session.compact { "on" } else { "off" };
            term.push_lines(&[styled::continuation(theme, "compact", state, None)])?;
        }
        _ if cmd.starts_with("/system") => {
            let msg = cmd.strip_prefix("/system").unwrap().trim();
            if msg.is_empty() {
                if let Some(sys) = active_system_prompt(session) {
                    term.push_lines(&[styled::continuation(theme, "system", &sys, None)])?;
                } else {
                    term.push_lines(&[styled::continuation(
                        theme,
                        "system",
                        "no system message set",
                        Some("/system <text>"),
                    )])?;
                }
            } else {
                session.system_message = Some(msg.to_string());
                if let Some(agent_session) = session.agent_session.as_mut() {
                    agent_session.system_prompt = msg.to_string();
                }
                term.push_lines(&[styled::continuation(theme, "system", "set", Some(msg))])?;
            }
        }
        "/reset" => {
            session.conversation.clear();
            session.turn_count = 0;
            session.cost = CostMeter::new();
            session.last_prompt = None;
            session.system_message = if session.agent_session.is_some() {
                active_system_prompt(session)
            } else {
                None
            };
            term.push_lines(&[styled::continuation(
                theme,
                "reset",
                "conversation cleared",
                None,
            )])?;
        }
        "/cost" => {
            let ratio = session.cost.savings_ratio();
            term.push_lines(&[
                styled::section_start(theme, "cost", "session summary", None),
                styled::continuation(theme, "turns", &session.cost.run_count.to_string(), None),
                styled::continuation(
                    theme,
                    "total",
                    &format!("${:.4}", session.cost.total_cost),
                    None,
                ),
                styled::continuation(
                    theme,
                    "tokens",
                    &format!(
                        "{} in / {} out",
                        session.cost.input_tokens, session.cost.output_tokens
                    ),
                    None,
                ),
                styled::section_end(theme, "savings", &format!("{ratio:.1}x vs baseline")),
            ])?;
        }
        "/provider" | "/auth" => {
            let info = match &session.dispatch {
                DispatchMode::Direct { auth } => auth.label(),
                DispatchMode::Http {
                    backend_url,
                    is_sidecar,
                    ..
                } => {
                    format!(
                        "HTTP {} ({})",
                        backend_url,
                        if *is_sidecar { "sidecar" } else { "serve" }
                    )
                }
                DispatchMode::Session => session
                    .agent_session
                    .as_ref()
                    .map(|agent_session| {
                        format!(
                            "{} ({})",
                            agent_session.model_selection.provider_key,
                            agent_session.model_selection.provider_kind
                        )
                    })
                    .unwrap_or_else(|| "ChatAgentSession".to_string()),
            };
            term.push_lines(&[styled::continuation(theme, "provider", &info, None)])?;
        }
        _ if cmd.starts_with("/model") => {
            let arg = cmd.strip_prefix("/model").unwrap().trim();
            if arg.is_empty() {
                let current = match &session.dispatch {
                    DispatchMode::Direct { auth } => match auth {
                        AuthMethod::ClaudeCli => "claude CLI (auto)".to_string(),
                        AuthMethod::AnthropicApi { model, .. } => {
                            let m = model.as_deref().unwrap_or("claude-sonnet-4-6");
                            format!("{m} (Anthropic API)")
                        }
                        AuthMethod::OpenAiCompat {
                            model, base_url, ..
                        } => {
                            let m = model.as_deref().unwrap_or("gpt-5.4-mini");
                            format!("{m} ({base_url})")
                        }
                        AuthMethod::NeedsSetup => "none".to_string(),
                    },
                    DispatchMode::Http { .. } => "HTTP backend (model set server-side)".to_string(),
                    DispatchMode::Session => active_model_name(session),
                };
                term.push_lines(&[styled::continuation(theme, "model", &current, None)])?;
            } else {
                match &mut session.dispatch {
                    DispatchMode::Direct { auth } => match auth {
                        AuthMethod::AnthropicApi { model, .. }
                        | AuthMethod::OpenAiCompat { model, .. } => {
                            *model = Some(arg.to_string());
                            term.push_lines(&[styled::continuation(
                                theme,
                                "model",
                                &format!("switched to {arg}"),
                                None,
                            )])?;
                        }
                        _ => {
                            term.push_lines(&[styled::continuation(
                                theme,
                                "model",
                                "can only switch with API providers",
                                Some("set ZAI_API_KEY, OPENAI_API_KEY, or ANTHROPIC_API_KEY"),
                            )])?;
                        }
                    },
                    DispatchMode::Http { .. } => {
                        term.push_lines(&[styled::continuation(
                            theme,
                            "model",
                            "model switching not supported in HTTP mode",
                            None,
                        )])?;
                    }
                    DispatchMode::Session => {
                        if let Some(agent_session) = session.agent_session.as_mut() {
                            agent_session.model = arg.to_string();
                            term.push_lines(&[styled::continuation(
                                theme,
                                "model",
                                &format!("switched to {arg}"),
                                None,
                            )])?;
                        } else {
                            term.push_lines(&[styled::continuation(
                                theme,
                                "model",
                                "ChatAgentSession unavailable",
                                None,
                            )])?;
                        }
                    }
                }
            }
        }
        _ if cmd.starts_with("/effort") => {
            let arg = cmd.strip_prefix("/effort").unwrap().trim();
            if arg.is_empty() {
                let current = session
                    .agent_session
                    .as_ref()
                    .map(|agent_session| agent_session.effort.as_str())
                    .unwrap_or("medium");
                term.push_lines(&[styled::continuation(
                    theme,
                    "effort",
                    &format!("current: {current}"),
                    Some("use: low, medium, high, max"),
                )])?;
            } else {
                match arg {
                    "low" | "medium" | "med" | "high" | "max" => {
                        if let Some(agent_session) = session.agent_session.as_mut() {
                            agent_session.effort = arg.to_string();
                        }
                        term.push_lines(&[styled::continuation(
                            theme,
                            "effort",
                            &format!("set to {arg}"),
                            None,
                        )])?;
                    }
                    _ => {
                        term.push_lines(&[styled::continuation(
                            theme,
                            "effort",
                            &format!("unknown level: {arg}"),
                            Some("use: low, medium, high, max"),
                        )])?;
                    }
                }
            }
        }
        _ if cmd.starts_with("/gate") => {
            let arg = cmd.strip_prefix("/gate").unwrap().trim();
            if arg.is_empty() {
                if let Some(toml) = read_roko_toml() {
                    let clippy = if toml.contains("clippy_enabled = true") {
                        "on"
                    } else {
                        "off"
                    };
                    let tests = if toml.contains("skip_tests = false") {
                        "on"
                    } else {
                        "off"
                    };
                    term.push_lines(&[
                        styled::section_start(theme, "gates", "", None),
                        styled::continuation(theme, "compile", "on (always)", None),
                        styled::continuation(theme, "test", tests, None),
                        styled::continuation(theme, "clippy", clippy, None),
                        styled::section_end(theme, "max iter", "3"),
                    ])?;
                } else {
                    term.push_lines(&[styled::continuation(
                        theme,
                        "gates",
                        "no roko.toml found",
                        None,
                    )])?;
                }
            } else {
                term.push_lines(&[styled::continuation(
                    theme,
                    "gate",
                    &format!("gate toggle: {arg}"),
                    Some("update roko.toml"),
                )])?;
            }
        }

        // =================================================================
        // Configuration
        // =================================================================
        "/config" => {
            if let Some(toml) = read_roko_toml() {
                // Extract key config values
                let mut lines = vec![styled::section_start(theme, "config", "roko.toml", None)];
                for section in &[
                    "[project]",
                    "[agent]",
                    "[routing]",
                    "[gates]",
                    "[budget]",
                    "[conductor]",
                ] {
                    if let Some(pos) = toml.find(section) {
                        let chunk: String =
                            toml[pos..].lines().take(6).collect::<Vec<_>>().join("\n");
                        for line in chunk.lines().take(5) {
                            if !line.trim().is_empty() {
                                lines.push(Line::from(vec![
                                    Span::styled(format!("{} ", symbols::BAR), theme.muted()),
                                    Span::styled(line.to_string(), theme.text()),
                                ]));
                            }
                        }
                    }
                }
                lines.push(Line::from(vec![Span::styled(
                    symbols::END.to_string(),
                    theme.muted(),
                )]));
                term.push_lines(&lines)?;
            } else {
                term.push_lines(&[styled::continuation(
                    theme,
                    "config",
                    "no roko.toml found",
                    Some("run: roko init"),
                )])?;
            }
        }
        "/config providers" => {
            if let Some(toml) = read_roko_toml() {
                let mut lines = vec![styled::section_start(theme, "providers", "", None)];
                for line in toml.lines() {
                    if line.starts_with("[providers.") {
                        let name = line.trim_start_matches("[providers.").trim_end_matches(']');
                        // Check if the env var is set
                        let next_lines: Vec<&str> = toml[toml.find(line).unwrap()..]
                            .lines()
                            .skip(1)
                            .take(4)
                            .collect();
                        let env_key = next_lines
                            .iter()
                            .find(|l| l.contains("api_key_env"))
                            .and_then(|l| l.split('"').nth(1))
                            .unwrap_or("");
                        let has_key = if env_key.is_empty() {
                            "no key needed"
                        } else if std::env::var(env_key).is_ok() {
                            "key set"
                        } else {
                            "key missing"
                        };
                        let default_model = next_lines
                            .iter()
                            .find(|l| l.contains("default_model"))
                            .and_then(|l| l.split('"').nth(1))
                            .unwrap_or("?");
                        lines.push(styled::continuation(
                            theme,
                            name,
                            default_model,
                            Some(has_key),
                        ));
                    }
                }
                lines.push(Line::from(vec![Span::styled(
                    symbols::END.to_string(),
                    theme.muted(),
                )]));
                term.push_lines(&lines)?;
            } else {
                term.push_lines(&[styled::continuation(
                    theme,
                    "providers",
                    "no roko.toml",
                    None,
                )])?;
            }
        }
        "/config models" => {
            if let Some(toml) = read_roko_toml() {
                let mut lines = vec![styled::section_start(
                    theme,
                    "models",
                    "all configured models",
                    None,
                )];
                for line in toml.lines() {
                    if line.starts_with("[models.") && !line.starts_with("[models]") {
                        let alias = line.trim_start_matches("[models.").trim_end_matches(']');
                        let next_lines: Vec<&str> = toml[toml.find(line).unwrap()..]
                            .lines()
                            .skip(1)
                            .take(3)
                            .collect();
                        let provider = next_lines
                            .iter()
                            .find(|l| l.contains("provider"))
                            .and_then(|l| l.split('"').nth(1))
                            .unwrap_or("?");
                        let slug = next_lines
                            .iter()
                            .find(|l| l.contains("slug"))
                            .and_then(|l| l.split('"').nth(1))
                            .unwrap_or("?");
                        lines.push(styled::continuation(theme, alias, slug, Some(provider)));
                    }
                }
                lines.push(Line::from(vec![Span::styled(
                    symbols::END.to_string(),
                    theme.muted(),
                )]));
                term.push_lines(&lines)?;
            } else {
                term.push_lines(&[styled::continuation(theme, "models", "no roko.toml", None)])?;
            }
        }
        "/config gates" => {
            // Delegate to /gate
            return handle_slash_command("/gate", session, term, theme);
        }
        _ if cmd.starts_with("/config set ") => {
            let rest = cmd.strip_prefix("/config set ").unwrap().trim();
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() < 2 {
                term.push_lines(&[styled::continuation(
                    theme,
                    "config",
                    "usage: /config set <key> <value>",
                    None,
                )])?;
            } else {
                let (key, value) = (parts[0], parts[1]);
                term.push_lines(&[styled::continuation(
                    theme,
                    "config",
                    &format!("set {key} = {value}"),
                    Some("edit roko.toml to persist"),
                )])?;
            }
        }

        // =================================================================
        // Workspace & git
        // =================================================================
        "/status" => {
            let roko_dir = std::path::Path::new(".roko");
            let has_roko = roko_dir.exists();
            let signal_count = std::fs::read_to_string(".roko/signals.jsonl")
                .map(|s| s.lines().count())
                .unwrap_or(0);
            let episode_count = std::fs::read_to_string(".roko/episodes.jsonl")
                .map(|s| s.lines().count())
                .unwrap_or(0);
            let plan_count = std::fs::read_dir(".roko/plans")
                .map(|d| d.count())
                .unwrap_or(0);
            let branch = shell_output("git", &["branch", "--show-current"]);
            term.push_lines(&[
                styled::section_start(theme, "status", "", None),
                styled::continuation(
                    theme,
                    ".roko",
                    if has_roko { "initialized" } else { "not found" },
                    None,
                ),
                styled::continuation(theme, "branch", branch.trim(), None),
                styled::continuation(theme, "signals", &signal_count.to_string(), None),
                styled::continuation(theme, "episodes", &episode_count.to_string(), None),
                styled::section_end(theme, "plans", &plan_count.to_string()),
            ])?;
        }
        "/doctor" => {
            let checks = [
                ("roko.toml", std::path::Path::new("roko.toml").exists()),
                (".roko/", std::path::Path::new(".roko").exists()),
                ("git", std::path::Path::new(".git").exists()),
                ("ZAI_API_KEY", std::env::var("ZAI_API_KEY").is_ok()),
                (
                    "ANTHROPIC_API_KEY",
                    std::env::var("ANTHROPIC_API_KEY").is_ok(),
                ),
                ("OPENAI_API_KEY", std::env::var("OPENAI_API_KEY").is_ok()),
                ("GEMINI_API_KEY", std::env::var("GEMINI_API_KEY").is_ok()),
                (
                    "MOONSHOT_API_KEY",
                    std::env::var("MOONSHOT_API_KEY").is_ok(),
                ),
                (
                    "PERPLEXITY_API_KEY",
                    std::env::var("PERPLEXITY_API_KEY").is_ok(),
                ),
            ];
            let mut lines = vec![styled::section_start(
                theme,
                "doctor",
                "workspace health",
                None,
            )];
            for (name, ok) in &checks {
                let icon = if *ok { symbols::PASS } else { symbols::FAIL };
                lines.push(styled::continuation(theme, icon, name, None));
            }
            lines.push(Line::from(vec![Span::styled(
                symbols::END.to_string(),
                theme.muted(),
            )]));
            term.push_lines(&lines)?;
        }
        "/diff" => {
            let output = shell_output("git", &["diff", "--stat", "--color=never"]);
            if output.trim().is_empty() {
                term.push_lines(&[styled::continuation(theme, "diff", "no changes", None)])?;
            } else {
                push_shell_output(term, theme, "diff", &output, 30)?;
            }
        }
        "/git" => {
            let output = shell_output("git", &["status", "--short"]);
            if output.trim().is_empty() {
                term.push_lines(&[styled::continuation(
                    theme,
                    "git",
                    "clean working tree",
                    None,
                )])?;
            } else {
                push_shell_output(term, theme, "git", &output, 25)?;
            }
        }
        _ if cmd.starts_with("/log") => {
            let arg = cmd.strip_prefix("/log").unwrap().trim();
            let n = arg.parse::<usize>().unwrap_or(5);
            let output = shell_output("git", &["log", &format!("-{n}"), "--oneline", "--decorate"]);
            push_shell_output(term, theme, "log", &output, n + 1)?;
        }
        "/branch" => {
            let output = shell_output("git", &["branch", "-v", "--color=never"]);
            push_shell_output(term, theme, "branch", &output, 15)?;
        }
        "/changes" => {
            let output = shell_output("git", &["diff", "--name-status", "HEAD"]);
            if output.trim().is_empty() {
                term.push_lines(&[styled::continuation(
                    theme,
                    "changes",
                    "no changes since last commit",
                    None,
                )])?;
            } else {
                push_shell_output(term, theme, "changes", &output, 30)?;
            }
        }

        // =================================================================
        // File operations
        // =================================================================
        _ if cmd.starts_with("/file ") => {
            let path = cmd.strip_prefix("/file ").unwrap().trim();
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    push_shell_output(term, theme, &format!("file: {path}"), &content, 40)?;
                }
                Err(e) => {
                    term.push_lines(&[styled::continuation(
                        theme,
                        "error",
                        &format!("{path}: {e}"),
                        None,
                    )])?;
                }
            }
        }
        _ if cmd.starts_with("/search ") => {
            let pattern = cmd.strip_prefix("/search ").unwrap().trim();
            let output = shell_output(
                "grep",
                &[
                    "-rn",
                    "--include=*.rs",
                    "--include=*.toml",
                    "--include=*.md",
                    pattern,
                    ".",
                ],
            );
            if output.trim().is_empty() {
                term.push_lines(&[styled::continuation(
                    theme,
                    "search",
                    &format!("no matches for '{pattern}'"),
                    None,
                )])?;
            } else {
                push_shell_output(term, theme, &format!("search: {pattern}"), &output, 25)?;
            }
        }
        _ if cmd.starts_with("/find ") => {
            let pattern = cmd.strip_prefix("/find ").unwrap().trim();
            let output = shell_output(
                "find",
                &[
                    ".",
                    "-name",
                    pattern,
                    "-not",
                    "-path",
                    "*/target/*",
                    "-not",
                    "-path",
                    "*/.git/*",
                ],
            );
            if output.trim().is_empty() {
                term.push_lines(&[styled::continuation(
                    theme,
                    "find",
                    &format!("no files matching '{pattern}'"),
                    None,
                )])?;
            } else {
                push_shell_output(term, theme, &format!("find: {pattern}"), &output, 25)?;
            }
        }
        _ if cmd.starts_with("/tree") => {
            let arg = cmd.strip_prefix("/tree").unwrap().trim();
            let path = if arg.is_empty() { "." } else { arg };
            let output = shell_output(
                "find",
                &[
                    path,
                    "-maxdepth",
                    "3",
                    "-not",
                    "-path",
                    "*/target/*",
                    "-not",
                    "-path",
                    "*/.git/*",
                ],
            );
            push_shell_output(term, theme, &format!("tree: {path}"), &output, 30)?;
        }

        // =================================================================
        // Agent & workflow
        // =================================================================
        "/agent" | "/agent list" => {
            let agent_id = &session.agent_id;
            term.push_lines(&[
                styled::section_start(theme, "agent", "", None),
                styled::continuation(theme, "current", agent_id, None),
                styled::continuation(
                    theme,
                    "roles",
                    "implementer, strategist, architect, auditor, researcher, scribe, critic",
                    None,
                ),
                styled::section_end(theme, "switch", "/agent <name>"),
            ])?;
        }
        _ if cmd.starts_with("/agent ") => {
            let name = cmd.strip_prefix("/agent ").unwrap().trim();
            if name != "list" {
                session.agent_id = name.to_string();
                let color = Theme::role_accent(name);
                let color_name = if color == Theme::ROSE {
                    "rose"
                } else if color == Theme::DREAM {
                    "dream"
                } else if color == Theme::BONE {
                    "bone"
                } else if color == Theme::SAGE {
                    "sage"
                } else if color == Theme::EMBER {
                    "ember"
                } else {
                    "default"
                };
                term.push_lines(&[styled::continuation(
                    theme,
                    "agent",
                    &format!("switched to {name}"),
                    Some(color_name),
                )])?;
            }
        }
        _ if cmd.starts_with("/run ") => {
            let prompt = cmd.strip_prefix("/run ").unwrap().trim();
            term.push_lines(&[
                styled::section_start(theme, "run", "universal loop", None),
                styled::continuation(theme, "prompt", prompt, None),
                styled::continuation(theme, "command", &format!("roko run \"{prompt}\""), None),
                styled::section_end(theme, "tip", "run this in a terminal for full output"),
            ])?;
        }
        "/plan list" => {
            let output = shell_output(
                "find",
                &[
                    ".roko/plans",
                    "-name",
                    "*.toml",
                    "-not",
                    "-path",
                    "*/target/*",
                ],
            );
            if output.trim().is_empty() {
                let output2 = shell_output("find", &["plans", "-name", "*.toml"]);
                if output2.trim().is_empty() {
                    term.push_lines(&[styled::continuation(
                        theme,
                        "plans",
                        "no plans found",
                        None,
                    )])?;
                } else {
                    push_shell_output(term, theme, "plans", &output2, 20)?;
                }
            } else {
                push_shell_output(term, theme, "plans", &output, 20)?;
            }
        }
        _ if cmd.starts_with("/plan run ") => {
            let dir = cmd.strip_prefix("/plan run ").unwrap().trim();
            term.push_lines(&[styled::continuation(
                theme,
                "plan",
                &format!("roko plan run {dir}"),
                Some("run in terminal"),
            )])?;
        }
        _ if cmd.starts_with("/plan generate ") => {
            let prompt = cmd.strip_prefix("/plan generate ").unwrap().trim();
            term.push_lines(&[styled::continuation(
                theme,
                "plan",
                &format!("roko plan generate \"{prompt}\""),
                Some("run in terminal"),
            )])?;
        }
        "/plan" => {
            term.push_lines(&[
                styled::section_start(theme, "plan", "subcommands", None),
                styled::continuation(theme, "/plan list", "list plans", None),
                styled::continuation(theme, "/plan run <dir>", "execute a plan", None),
                styled::section_end(theme, "/plan generate <prompt>", "generate plan"),
            ])?;
        }

        // =================================================================
        // PRD & research
        // =================================================================
        _ if cmd.starts_with("/prd idea ") => {
            let idea = cmd.strip_prefix("/prd idea ").unwrap().trim();
            term.push_lines(&[styled::continuation(
                theme,
                "prd",
                &format!("roko prd idea \"{idea}\""),
                Some("run in terminal"),
            )])?;
        }
        "/prd list" => {
            let prd_dir = std::path::Path::new(".roko/prd");
            if prd_dir.exists() {
                let output = shell_output("ls", &["-la", ".roko/prd/"]);
                push_shell_output(term, theme, "PRDs", &output, 20)?;
            } else {
                term.push_lines(&[styled::continuation(
                    theme,
                    "prd",
                    "no PRDs found",
                    Some(".roko/prd/ not found"),
                )])?;
            }
        }
        "/prd" => {
            term.push_lines(&[
                styled::section_start(theme, "prd", "subcommands", None),
                styled::continuation(theme, "/prd idea <text>", "capture idea", None),
                styled::continuation(theme, "/prd list", "list PRDs", None),
                styled::section_end(theme, "cli", "roko prd draft|plan|status"),
            ])?;
        }
        _ if cmd.starts_with("/research ") => {
            let query = cmd.strip_prefix("/research ").unwrap().trim();
            term.push_lines(&[styled::continuation(
                theme,
                "research",
                &format!("roko research topic \"{query}\""),
                Some("run in terminal"),
            )])?;
        }

        // =================================================================
        // Knowledge & learning
        // =================================================================
        _ if cmd.starts_with("/knowledge ") => {
            let query = cmd.strip_prefix("/knowledge ").unwrap().trim();
            if query == "stats" {
                let neuro_dir = std::path::Path::new(".roko/neuro");
                if neuro_dir.exists() {
                    let output = shell_output("ls", &["-la", ".roko/neuro/"]);
                    push_shell_output(term, theme, "knowledge", &output, 15)?;
                } else {
                    term.push_lines(&[styled::continuation(
                        theme,
                        "knowledge",
                        "store not initialized",
                        None,
                    )])?;
                }
            } else {
                term.push_lines(&[styled::continuation(
                    theme,
                    "knowledge",
                    &format!("roko knowledge query \"{query}\""),
                    Some("run in terminal"),
                )])?;
            }
        }
        "/knowledge" => {
            term.push_lines(&[
                styled::section_start(theme, "knowledge", "subcommands", None),
                styled::continuation(theme, "/knowledge <query>", "query store", None),
                styled::continuation(theme, "/knowledge stats", "store stats", None),
                styled::section_end(theme, "cli", "roko knowledge query|stats|gc"),
            ])?;
        }
        "/learn" => {
            let learn_dir = std::path::Path::new(".roko/learn");
            if learn_dir.exists() {
                let files: Vec<String> = std::fs::read_dir(learn_dir)
                    .map(|d| {
                        d.filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                let mut lines = vec![styled::section_start(
                    theme,
                    "learn",
                    "learning state",
                    None,
                )];
                for f in &files {
                    lines.push(styled::continuation(theme, "", f, None));
                }
                lines.push(Line::from(vec![Span::styled(
                    symbols::END.to_string(),
                    theme.muted(),
                )]));
                term.push_lines(&lines)?;
            } else {
                term.push_lines(&[styled::continuation(
                    theme,
                    "learn",
                    "no learning data",
                    Some(".roko/learn/ not found"),
                )])?;
            }
        }

        // =================================================================
        // Export & retry (kept from before)
        // =================================================================
        _ if cmd.starts_with("/export") => {
            let arg = cmd.strip_prefix("/export").unwrap().trim();
            let format = if arg.is_empty() { "markdown" } else { arg };
            if session.conversation.is_empty() {
                term.push_lines(&[styled::continuation(
                    theme,
                    "export",
                    "no messages to export",
                    None,
                )])?;
            } else {
                let exports_dir = std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .join(".roko")
                    .join("exports");
                let _ = std::fs::create_dir_all(&exports_dir);
                let ts = chrono::Local::now().format("%Y-%m-%d-%H%M");
                match format {
                    "markdown" | "md" => {
                        let path = exports_dir.join(format!("chat-{ts}.md"));
                        let model_name = current_model_name(session);
                        let mut md = format!(
                            "# Roko Chat — {}\n\n**Model**: {} | **Turns**: {} | **Cost**: ${:.4}\n\n---\n\n",
                            chrono::Local::now().format("%Y-%m-%d %H:%M"),
                            model_name,
                            session.turn_count,
                            session.cost.total_cost,
                        );
                        for msg in &session.conversation {
                            let role = if msg.role == "user" { "User" } else { "Roko" };
                            md.push_str(&format!("## {role}\n\n{}\n\n---\n\n", msg.text));
                        }
                        match std::fs::write(&path, &md) {
                            Ok(()) => term.push_lines(&[styled::continuation(
                                theme,
                                "export",
                                &format!("saved to {}", path.display()),
                                None,
                            )])?,
                            Err(e) => term.push_lines(&[styled::continuation(
                                theme,
                                "error",
                                &format!("export failed: {e}"),
                                None,
                            )])?,
                        }
                    }
                    "json" => {
                        let path = exports_dir.join(format!("chat-{ts}.json"));
                        let messages: Vec<serde_json::Value> = session.conversation.iter()
                            .map(|m| json!({ "role": m.role, "text": m.text, "timestamp": m.timestamp }))
                            .collect();
                        let export = json!({
                            "turns": session.turn_count, "cost": session.cost.total_cost,
                            "tokens_in": session.cost.input_tokens, "tokens_out": session.cost.output_tokens,
                            "messages": messages,
                        });
                        match std::fs::write(
                            &path,
                            serde_json::to_string_pretty(&export).unwrap_or_default(),
                        ) {
                            Ok(()) => term.push_lines(&[styled::continuation(
                                theme,
                                "export",
                                &format!("saved to {}", path.display()),
                                None,
                            )])?,
                            Err(e) => term.push_lines(&[styled::continuation(
                                theme,
                                "error",
                                &format!("export failed: {e}"),
                                None,
                            )])?,
                        }
                    }
                    _ => {
                        term.push_lines(&[styled::continuation(
                            theme,
                            "export",
                            &format!("unknown format: {format}"),
                            Some("use: markdown, json"),
                        )])?;
                    }
                }
            }
        }
        "/retry" => {
            if let Some(ref prompt) = session.last_prompt {
                session.input.buffer = prompt.clone();
                session.input.cursor = session.input.buffer.len();
                term.push_lines(&[styled::continuation(
                    theme,
                    "retry",
                    "loaded last message — press Enter to send",
                    None,
                )])?;
            } else {
                term.push_lines(&[styled::continuation(
                    theme,
                    "retry",
                    "no previous message to retry",
                    None,
                )])?;
            }
        }
        "/clear" => {
            for _ in 0..term.viewport_height() {
                term.push_blank()?;
            }
        }

        "/tools" => {
            use roko_std::tool::builtin::BUILTIN_TOOL_NAMES;

            let names = BUILTIN_TOOL_NAMES.as_slice();
            let mut lines = vec![styled::section_start(
                theme,
                "tools",
                &format!("{} builtin tools", names.len()),
                None,
            )];

            for chunk in names.chunks(4) {
                lines.push(styled::continuation(theme, "tool", &chunk.join("  "), None));
            }

            lines.push(styled::section_end(
                theme,
                "tip",
                "resolved from roko-std builtins",
            ));
            term.push_lines(&lines)?;
        }

        _ => {
            term.push_lines(&[styled::continuation(
                theme,
                "unknown",
                &format!("command: {cmd}"),
                Some("try /help"),
            )])?;
        }
    }
    Ok(false)
}

/// Get the current model name for display.
fn current_model_name(session: &ChatSession) -> String {
    active_model_name(session)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_viewport(frame: &mut Frame<'_>, session: &ChatSession, theme: &Theme) {
    let area = frame.area();

    match session.phase {
        Phase::Input => render_input(frame, area, session, theme),
        Phase::Thinking => {
            let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);

            let elapsed = session
                .thinking_started
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or(0.0);
            let label = thinking_label(elapsed);
            let spinner = styled::spinner_line(theme, session.tick, label, elapsed);
            frame.render_widget(Paragraph::new(spinner), chunks[0]);
            render_status_bar(frame, chunks[1], session, theme);
        }
        Phase::Streaming => {
            session.streaming.render(frame, area, theme);
        }
        Phase::Error { ref error, .. } => {
            let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
            let hint = Line::from(vec![
                Span::styled(
                    format!("  {} ", symbols::WARN),
                    Style::default().fg(Theme::EMBER),
                ),
                Span::styled(
                    truncate_str(error, area.width as usize - 6),
                    Style::default().fg(Theme::EMBER),
                ),
                Span::raw("  "),
                Span::styled(
                    "[r]",
                    Style::default()
                        .fg(Theme::BONE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("etry  ", theme.muted()),
                Span::styled(
                    "[q]",
                    Style::default()
                        .fg(Theme::BONE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("uit", theme.muted()),
            ]);
            frame.render_widget(Paragraph::new(hint), chunks[0]);
            render_status_bar(frame, chunks[1], session, theme);
        }
        Phase::Done => {
            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    "bye.".to_string(),
                    theme.muted(),
                )])),
                area,
            );
        }
    }
}

fn render_input(frame: &mut Frame<'_>, area: Rect, session: &ChatSession, theme: &Theme) {
    // --- Command palette overlay ---
    if session.input.palette.active {
        render_palette(frame, area, session, theme);
        return;
    }

    let dropdown_visible = session.input.completion.visible;
    let match_count = session.input.completion.matches.len();

    // Multi-line input: height = min(line_count, 6), clamped to available space
    let input_lines = session.input.line_count();
    let max_input_height = (area.height as usize).saturating_sub(2); // leave room for spacer + status
    let input_height = input_lines.min(6).min(max_input_height).max(1) as u16;

    // Compute dropdown height: min(match_count, 8, viewport - input - 2)
    let dropdown_height = if dropdown_visible {
        let max_rows = (area.height as usize).saturating_sub(input_height as usize + 2);
        match_count.min(8).min(max_rows) as u16
    } else {
        0
    };

    let chunks = if dropdown_height > 0 {
        Layout::vertical([
            Constraint::Min(1),                  // upper spacer
            Constraint::Length(dropdown_height), // dropdown
            Constraint::Length(input_height),    // input area
            Constraint::Length(1),               // status bar
        ])
        .split(area)
    } else {
        // No dropdown — 3-zone layout (pad to 4 for uniform indexing)
        let base = Layout::vertical([
            Constraint::Min(1),               // spacer
            Constraint::Length(input_height), // input area
            Constraint::Length(1),            // status bar
        ])
        .split(area);
        // Map base[0..3] → chunks[0, skip, 1, 2] so chunks[2] = input, chunks[3] = status
        vec![base[0], Rect::default(), base[1], base[2]].into()
    };

    // Dropdown rendering
    if dropdown_height > 0 {
        let dropdown_area = chunks[1];
        let selected = session.input.completion.selected;
        let mut lines: Vec<Line<'static>> = Vec::with_capacity(dropdown_height as usize);

        for (i, m) in session
            .input
            .completion
            .matches
            .iter()
            .take(dropdown_height as usize)
            .enumerate()
        {
            let is_selected = i == selected;
            let base_style = if is_selected {
                theme.selection()
            } else {
                theme.text()
            };
            let dim_style = if is_selected {
                theme.selection()
            } else {
                theme.muted()
            };

            // Build command span with highlighted matched chars
            let prefix = if is_selected { "> " } else { "  " };
            let mut spans: Vec<Span<'static>> = vec![Span::styled(prefix.to_string(), base_style)];

            // Render command with fuzzy-match highlights
            let cmd_chars: Vec<char> = m.command.chars().collect();
            let highlight_style = Style::default()
                .fg(Theme::BONE)
                .add_modifier(Modifier::BOLD);
            let mut ci = 0;
            let mut run_start = 0;
            let matched_set: std::collections::HashSet<usize> =
                m.matched_indices.iter().copied().collect();

            while ci < cmd_chars.len() {
                if matched_set.contains(&ci) {
                    // Flush non-highlighted run
                    if run_start < ci {
                        let s: String = cmd_chars[run_start..ci].iter().collect();
                        spans.push(Span::styled(s, base_style));
                    }
                    // Highlighted char
                    spans.push(Span::styled(
                        cmd_chars[ci].to_string(),
                        if is_selected {
                            highlight_style.bg(theme.selection_background)
                        } else {
                            highlight_style
                        },
                    ));
                    run_start = ci + 1;
                }
                ci += 1;
            }
            if run_start < cmd_chars.len() {
                let s: String = cmd_chars[run_start..].iter().collect();
                spans.push(Span::styled(s, base_style));
            }

            // Pad and add description
            let cmd_width = m.command.len() + 2; // prefix + command
            let pad = if cmd_width < 14 { 14 - cmd_width } else { 2 };
            spans.push(Span::styled(" ".repeat(pad), dim_style));
            spans.push(Span::styled(m.description.to_string(), dim_style));

            lines.push(Line::from(spans));
        }

        frame.render_widget(Paragraph::new(lines), dropdown_area);
    }

    // Input prompt — supports multi-line buffers and search mode
    let input_area = chunks[2];
    let prompt_style = Style::default()
        .fg(Theme::ROSE)
        .add_modifier(Modifier::BOLD);
    let cont_style = Style::default().fg(Theme::TEXT_DIM);
    let cursor_style = Style::default()
        .fg(Theme::BONE)
        .add_modifier(Modifier::REVERSED);

    if session.input.search.active {
        // Reverse search prompt: (reverse-i-search) 'query': matched_text
        let query = &session.input.search.query;
        let match_info = if session.input.search.matches.is_empty() && !query.is_empty() {
            "no match"
        } else {
            ""
        };
        let matched_text = session
            .input
            .search
            .current_match(&session.input.history)
            .unwrap_or("");
        let mut spans = vec![
            Span::styled(
                "(reverse-i-search) ".to_string(),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::styled("'".to_string(), Style::default().fg(Theme::TEXT_GHOST)),
            Span::styled(
                query.to_string(),
                Style::default()
                    .fg(Theme::ROSE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("'".to_string(), Style::default().fg(Theme::TEXT_GHOST)),
            Span::styled(": ".to_string(), Style::default().fg(Theme::TEXT_DIM)),
        ];
        if match_info.is_empty() {
            spans.push(Span::styled(matched_text.to_string(), theme.text()));
        } else {
            spans.push(Span::styled(
                match_info.to_string(),
                Style::default().fg(Theme::EMBER),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), input_area);
    } else if !session.input.buffer.contains('\n') {
        // Single-line: original compact rendering
        let before_cursor = &session.input.buffer[..session.input.cursor];
        let after_cursor = &session.input.buffer[session.input.cursor..];

        let mut input_spans = vec![
            Span::styled(format!("{} ", symbols::PROMPT), prompt_style),
            Span::styled(before_cursor.to_string(), theme.text()),
            Span::styled(
                if after_cursor.is_empty() {
                    symbols::CURSOR.to_string()
                } else {
                    after_cursor.chars().next().unwrap_or(' ').to_string()
                },
                cursor_style,
            ),
            Span::styled(
                if after_cursor.len() > 1 {
                    after_cursor[after_cursor
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(1)..]
                        .to_string()
                } else {
                    String::new()
                },
                theme.text(),
            ),
        ];

        // Ghost text — only when dropdown is NOT visible and cursor is at end
        if !dropdown_visible {
            if let Some(ghost) = session.input.ghost_suggestion() {
                input_spans.push(Span::styled(
                    ghost.to_string(),
                    Style::default().fg(Theme::TEXT_GHOST),
                ));
            }
        }

        frame.render_widget(Paragraph::new(Line::from(input_spans)), input_area);
    } else {
        // Multi-line rendering
        let (cursor_line, cursor_col) = session.input.cursor_line_col();
        let buf_lines: Vec<&str> = session.input.buffer.split('\n').collect();
        let line_count_label = format!("[{} lines]", buf_lines.len());

        let mut rendered_lines: Vec<Line<'static>> = Vec::new();
        for (i, line_text) in buf_lines.iter().enumerate() {
            let prefix = if i == 0 {
                Span::styled(format!("{} ", symbols::PROMPT), prompt_style)
            } else {
                Span::styled(format!("{} ", symbols::BAR), cont_style)
            };

            if i == cursor_line {
                // This line has the cursor
                let before = &line_text[..cursor_col.min(line_text.len())];
                let at_end = cursor_col >= line_text.len();
                let cursor_char = if at_end {
                    symbols::CURSOR.to_string()
                } else {
                    line_text[cursor_col..]
                        .chars()
                        .next()
                        .unwrap_or(' ')
                        .to_string()
                };
                let after = if at_end || cursor_col + 1 >= line_text.len() {
                    String::new()
                } else {
                    line_text[cursor_col
                        + line_text[cursor_col..]
                            .chars()
                            .next()
                            .map(|c| c.len_utf8())
                            .unwrap_or(1)..]
                        .to_string()
                };

                let mut spans = vec![
                    prefix,
                    Span::styled(before.to_string(), theme.text()),
                    Span::styled(cursor_char, cursor_style),
                    Span::styled(after, theme.text()),
                ];
                // Line count badge on first line
                if i == 0 {
                    spans.push(Span::styled(
                        format!("  {line_count_label}"),
                        Style::default().fg(Theme::TEXT_GHOST),
                    ));
                }
                rendered_lines.push(Line::from(spans));
            } else {
                let mut spans = vec![prefix, Span::styled(line_text.to_string(), theme.text())];
                if i == 0 {
                    spans.push(Span::styled(
                        format!("  {line_count_label}"),
                        Style::default().fg(Theme::TEXT_GHOST),
                    ));
                }
                rendered_lines.push(Line::from(spans));
            }
        }

        frame.render_widget(Paragraph::new(rendered_lines), input_area);
    }

    render_status_bar(frame, chunks[3], session, theme);
}

/// Render the command palette overlay (Ctrl+K).
fn render_palette(frame: &mut Frame<'_>, area: Rect, session: &ChatSession, theme: &Theme) {
    let palette = &session.input.palette;
    let max_visible = 10.min(area.height.saturating_sub(3) as usize);
    let visible_matches = palette.matches.len().min(max_visible);
    let palette_height = (visible_matches as u16 + 2).min(area.height.saturating_sub(1)); // +1 search, +1 border hint

    let chunks = Layout::vertical([
        Constraint::Min(1),                 // spacer
        Constraint::Length(palette_height), // palette body
        Constraint::Length(1),              // status bar
    ])
    .split(area);

    let palette_area = chunks[1];

    // Search bar
    let search_line = Line::from(vec![
        Span::styled(
            format!("{} ", symbols::PROMPT),
            Style::default()
                .fg(Theme::ROSE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if palette.query.is_empty() {
                "type to filter...".to_string()
            } else {
                palette.query.clone()
            },
            if palette.query.is_empty() {
                Style::default().fg(Theme::TEXT_GHOST)
            } else {
                Style::default().fg(Theme::BONE)
            },
        ),
        Span::styled(
            symbols::CURSOR.to_string(),
            Style::default()
                .fg(Theme::BONE)
                .add_modifier(Modifier::REVERSED),
        ),
    ]);

    let mut lines: Vec<Line<'static>> = vec![search_line];

    // Scrolling: center selected item in view
    let scroll_offset = if palette.selected >= max_visible {
        palette.selected - max_visible + 1
    } else {
        0
    };

    for (i, m) in palette
        .matches
        .iter()
        .skip(scroll_offset)
        .take(max_visible)
        .enumerate()
    {
        let actual_idx = scroll_offset + i;
        let is_selected = actual_idx == palette.selected;
        let base_style = if is_selected {
            theme.selection()
        } else {
            theme.text()
        };
        let dim_style = if is_selected {
            theme.selection()
        } else {
            theme.muted()
        };

        let prefix = if is_selected { "> " } else { "  " };
        let cmd_width = m.command.len() + 2;
        let pad = if cmd_width < 20 { 20 - cmd_width } else { 2 };

        lines.push(Line::from(vec![
            Span::styled(prefix.to_string(), base_style),
            Span::styled(m.command.to_string(), base_style),
            Span::styled(" ".repeat(pad), dim_style),
            Span::styled(m.description.to_string(), dim_style),
        ]));
    }

    // Hint line if there are more matches
    if palette.matches.len() > max_visible {
        let remaining = palette.matches.len() - max_visible;
        lines.push(Line::from(vec![Span::styled(
            format!("  ... {remaining} more"),
            Style::default().fg(Theme::TEXT_GHOST),
        )]));
    }

    frame.render_widget(Paragraph::new(lines), palette_area);
    render_status_bar(frame, chunks[2], session, theme);
}

fn render_status_bar(frame: &mut Frame<'_>, area: Rect, session: &ChatSession, theme: &Theme) {
    let model = session.cost.primary_model().unwrap_or("—").to_string();

    // Build the base status bar
    let mut spans = vec![
        Span::styled(
            format!("${:.4}", session.cost.total_cost),
            Style::default().fg(Theme::SAGE),
        ),
        Span::styled(format!("  {}  ", symbols::SEP), theme.muted()),
        Span::styled(
            format!(
                "{} in / {} out",
                session.cost.input_tokens, session.cost.output_tokens
            ),
            Style::default().fg(Theme::TEXT_DIM),
        ),
        Span::styled(format!("  {}  ", symbols::SEP), theme.muted()),
        Span::styled(model, theme.info()),
    ];

    // Turn counter
    if session.turn_count > 0 {
        spans.push(Span::styled(format!("  {}  ", symbols::SEP), theme.muted()));
        spans.push(Span::styled(
            format!("turn {}", session.turn_count),
            Style::default().fg(Theme::TEXT_DIM),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------------------
// Backend communication
// ---------------------------------------------------------------------------

/// Compute naive Opus-rate baseline cost for savings comparison.
fn naive_opus_cost(input_tokens: u64, output_tokens: u64) -> f64 {
    (input_tokens as f64 * 15.0 / 1_000_000.0) + (output_tokens as f64 * 75.0 / 1_000_000.0)
}

/// Calculate cost from a dispatch result using the cost table.
fn cost_from_result(table: &CostTable, result: &DispatchResult) -> f64 {
    table.calculate(
        &result.model,
        &roko_agent::Usage {
            input_tokens: result.input_tokens as u32,
            output_tokens: result.output_tokens as u32,
            ..Default::default()
        },
    )
}

/// HTTP response wrapper that can convert into [`DispatchResult`].
struct HttpResponse {
    text: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
}

impl From<HttpResponse> for DispatchResult {
    fn from(r: HttpResponse) -> Self {
        Self {
            text: r.text,
            model: r.model,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            tool_outputs: Vec::new(),
            session_id: None,
        }
    }
}

/// Send a message and receive the response.
///
/// When `is_sidecar` is true, sends to `{url}/message` with `{"prompt": message}`.
/// When false, sends to `{url}/api/agents/{agent_id}/message` with `{"message": message}`.
///
/// For run-id polling, `base_url` is used as the serve URL to query status.
async fn send_and_receive(
    client: &reqwest::Client,
    base_url: &str,
    agent_id: &str,
    message: &str,
    is_sidecar: bool,
) -> Result<HttpResponse> {
    // For polling, we use base_url (which is the serve URL in the non-sidecar case).
    let poll_base = base_url;
    let (url, body) = if is_sidecar {
        (
            format!("{}/message", base_url.trim_end_matches('/')),
            json!({ "prompt": message }),
        )
    } else {
        (
            format!(
                "{}/api/agents/{agent_id}/message",
                base_url.trim_end_matches('/')
            ),
            json!({ "message": message }),
        )
    };

    let response = client
        .post(&url)
        .json(&body)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .with_context(|| format!("POST {url} — is `roko serve` running?"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();
        // Try to extract a meaningful error message from JSON response
        let detail = serde_json::from_str::<serde_json::Value>(&body_text)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
            .unwrap_or(body_text);
        bail!("{status}: {detail}");
    }

    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        response: Option<String>,
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        input_tokens: Option<u64>,
        #[serde(default)]
        output_tokens: Option<u64>,
    }

    let resp: Resp = response.json().await.context("decode response")?;

    let resp_model = resp.model.unwrap_or_default();
    let resp_input = resp.input_tokens.unwrap_or(0);
    let resp_output = resp.output_tokens.unwrap_or(0);

    if let Some(text) = resp.response.filter(|s| !s.trim().is_empty()) {
        let clean = extract_clean_text(&text);
        // Use actual tokens if provided, otherwise approximate from text
        let input_tokens = if resp_input > 0 {
            resp_input
        } else {
            (message.len() as u64) / 4
        };
        let output_tokens = if resp_output > 0 {
            resp_output
        } else {
            (clean.len() as u64) / 4
        };
        let model = if resp_model.is_empty() {
            "http".to_string()
        } else {
            resp_model
        };
        return Ok(HttpResponse {
            text: clean,
            model,
            input_tokens,
            output_tokens,
        });
    }

    if let Some(run_id) = resp.run_id.filter(|s| !s.trim().is_empty()) {
        // Poll for completion
        let status_url = format!(
            "{}/api/run/{run_id}/status",
            poll_base.trim_end_matches('/'),
        );
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let status_resp = client
                .get(&status_url)
                .send()
                .await
                .context("poll status")?;

            #[derive(Deserialize)]
            struct StatusResp {
                #[serde(default)]
                finished: bool,
                #[serde(default)]
                output_text: Option<String>,
                #[serde(default)]
                error: Option<String>,
                #[serde(default)]
                model: Option<String>,
                #[serde(default)]
                input_tokens: Option<u64>,
                #[serde(default)]
                output_tokens: Option<u64>,
            }

            let status: StatusResp = status_resp.json().await.context("decode status")?;
            if status.finished {
                if let Some(err) = status.error.filter(|s| !s.trim().is_empty()) {
                    bail!("agent error: {err}");
                }
                let text = status.output_text.unwrap_or_default();
                let input_tokens = status
                    .input_tokens
                    .unwrap_or_else(|| (message.len() as u64) / 4);
                let output_tokens = status
                    .output_tokens
                    .unwrap_or_else(|| (text.len() as u64) / 4);
                let model = status.model.unwrap_or_else(|| "http".to_string());
                return Ok(HttpResponse {
                    text,
                    model,
                    input_tokens,
                    output_tokens,
                });
            }
        }
    }

    bail!("no response or run_id in reply");
}

/// Push a completed agent response into scrollback with markdown rendering.
/// Estimate reading time for a text response.
fn reading_time(text: &str) -> Option<String> {
    let words = text.split_whitespace().count();
    if words < 100 {
        return None;
    }
    let minutes = words as f64 / 200.0; // avg reading speed
    if minutes < 1.0 {
        Some(format!("~{words} words"))
    } else {
        Some(format!("~{} min read", minutes.ceil() as u32))
    }
}

/// Render tool execution outputs above the agent response.
/// Each tool output is shown as a collapsed summary with the tool name and
/// a preview of the output content, similar to mori's CommandOutput panel.
fn push_tool_outputs(
    term: &mut InlineTerminal,
    theme: &Theme,
    tool_outputs: &[ToolOutput],
) -> std::io::Result<()> {
    if tool_outputs.is_empty() {
        return Ok(());
    }
    for output in tool_outputs {
        let tool_label = output.tool_name.as_deref().unwrap_or("tool");
        // Show first line of output as preview, truncated
        let preview = output
            .content
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(80)
            .collect::<String>();
        let line_count = output.content.lines().count();
        let suffix = if line_count > 1 {
            format!(" (+{} lines)", line_count - 1)
        } else {
            String::new()
        };

        let tool_sym = symbols::TOOL;
        term.push_lines(&[Line::from(vec![
            Span::styled(
                format!("  {tool_sym} "),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::styled(
                tool_label.to_string(),
                Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {preview}{suffix}"),
                Style::default().fg(Theme::TEXT_DIM),
            ),
        ])])?;
    }
    term.push_lines(&[Line::raw("")])?;
    Ok(())
}

fn push_agent_response(
    term: &mut InlineTerminal,
    theme: &Theme,
    text: &str,
    agent_id: &str,
) -> std::io::Result<()> {
    let mut lines = Vec::new();

    // Agent header (with role-specific color and optional reading time)
    let agent_color = Theme::role_accent(agent_id);
    let agent_color = if agent_color == Theme::TEXT_DIM {
        theme.info
    } else {
        agent_color
    };
    let mut header_spans = vec![
        Span::styled(symbols::START.to_string(), Style::default().fg(agent_color)),
        Span::raw(" "),
        Span::styled(
            agent_id.to_string(),
            Style::default()
                .fg(agent_color)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(rt) = reading_time(text) {
        header_spans.push(Span::styled(
            format!("  {}", rt),
            Style::default().fg(Theme::TEXT_GHOST),
        ));
    }
    lines.push(Line::from(header_spans));

    // Response body — rendered as markdown with bar prefix
    let md_lines = crate::inline::markdown::render_markdown_with_bar(text, theme);
    lines.extend(md_lines);

    // Close
    lines.push(Line::from(vec![Span::styled(
        symbols::END.to_string(),
        theme.muted(),
    )]));

    term.push_lines(&lines)
}

/// Suggest recovery actions for common error patterns.
fn error_suggestions(err: &str) -> Vec<(&'static str, &'static str)> {
    let err_lower = err.to_lowercase();
    let mut suggestions = Vec::new();

    if err_lower.contains("connection refused") || err_lower.contains("connect error") {
        suggestions.push(("start server", "roko serve"));
        suggestions.push(("check port", "lsof -i :6677"));
        suggestions.push(("use direct mode", "roko (no subcommand)"));
    } else if err_lower.contains("unauthorized")
        || err_lower.contains("401")
        || err_lower.contains("invalid api key")
        || err_lower.contains("authentication")
    {
        suggestions.push(("check key", "echo $ANTHROPIC_API_KEY | head -c 10"));
        suggestions.push(("set key", "export ANTHROPIC_API_KEY=sk-ant-..."));
    } else if err_lower.contains("rate limit")
        || err_lower.contains("429")
        || err_lower.contains("too many requests")
    {
        suggestions.push(("wait & retry", "try again in 30 seconds"));
        suggestions.push(("switch model", "/model claude-haiku-4-5-20251001"));
    } else if err_lower.contains("timeout") || err_lower.contains("timed out") {
        suggestions.push(("retry", "press Enter to resend"));
        suggestions.push(("switch model", "/model claude-haiku-4-5-20251001"));
    } else if err_lower.contains("model") && err_lower.contains("not found") {
        suggestions.push(("list models", "/model"));
        suggestions.push(("use default", "/model claude-sonnet-4-6"));
    } else if err_lower.contains("context")
        && (err_lower.contains("length") || err_lower.contains("too long"))
    {
        suggestions.push(("clear context", "/clear"));
        suggestions.push(("start fresh", "exit and restart"));
    }

    suggestions
}

/// Push an error with contextual recovery suggestions.
fn push_error_with_suggestions(
    term: &mut InlineTerminal,
    theme: &Theme,
    err: &str,
) -> std::io::Result<()> {
    let mut lines = vec![styled::continuation(theme, "error", err, None)];

    let suggestions = error_suggestions(err);
    if !suggestions.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            symbols::BAR.to_string(),
            theme.muted(),
        )]));
        for (label, cmd) in &suggestions {
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", symbols::BAR), theme.muted()),
                Span::styled(format!("  {label}: "), Style::default().fg(Theme::WARNING)),
                Span::styled(cmd.to_string(), Style::default().fg(Theme::BONE)),
            ]));
        }
    }

    term.push_lines(&lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_state_basic() {
        let mut input = InputState::new();
        input.insert('h');
        input.insert('i');
        assert_eq!(input.buffer, "hi");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn input_state_backspace() {
        let mut input = InputState::new();
        input.insert('a');
        input.insert('b');
        input.backspace();
        assert_eq!(input.buffer, "a");
        assert_eq!(input.cursor, 1);
    }

    #[test]
    fn input_state_history() {
        let mut input = InputState::new();
        input.buffer = "first".into();
        input.submit();
        input.buffer = "second".into();
        input.submit();
        assert!(input.is_empty());

        input.history_up();
        assert_eq!(input.buffer, "second");
        input.history_up();
        assert_eq!(input.buffer, "first");
        input.history_down();
        assert_eq!(input.buffer, "second");
        input.history_down();
        assert!(input.is_empty());
    }

    #[test]
    fn input_state_submit() {
        let mut input = InputState::new();
        input.insert('t');
        input.insert('e');
        input.insert('s');
        input.insert('t');
        let text = input.submit();
        assert_eq!(text, "test");
        assert!(input.is_empty());
        assert_eq!(input.history.len(), 1);
    }

    #[test]
    fn turn_result_to_dispatch_result_keeps_model_and_tool_preview() {
        let turn = crate::chat_session::TurnResult {
            text: "hello".to_string(),
            model: "stale-backend".to_string(),
            input_tokens: 12,
            output_tokens: 34,
            tool_calls: vec![crate::chat_session::ToolCallSummary {
                name: "Read".to_string(),
                input_abbrev: "file contents here".to_string(),
                success: true,
            }],
            session_id: Some("sess-123".to_string()),
            duration: Duration::from_millis(42),
            cancelled: false,
        };

        let result = turn_result_to_dispatch_result(turn, "claude-sonnet-4-6".to_string());
        assert_eq!(result.model, "claude-sonnet-4-6");
        assert_eq!(result.session_id.as_deref(), Some("sess-123"));
        assert_eq!(result.tool_outputs.len(), 1);
        assert_eq!(result.tool_outputs[0].tool_name.as_deref(), Some("Read"));
        assert_eq!(result.tool_outputs[0].content, "file contents here");
    }

    #[test]
    fn input_state_navigation() {
        let mut input = InputState::new();
        for ch in "hello".chars() {
            input.insert(ch);
        }
        assert_eq!(input.cursor, 5);
        input.home();
        assert_eq!(input.cursor, 0);
        input.end();
        assert_eq!(input.cursor, 5);
        input.move_left();
        assert_eq!(input.cursor, 4);
        input.move_right();
        assert_eq!(input.cursor, 5);
    }

    // -----------------------------------------------------------------------
    // fuzzy_match tests
    // -----------------------------------------------------------------------

    #[test]
    fn fuzzy_match_exact() {
        let (score, indices) = fuzzy_match("model", "model").unwrap();
        assert!(score > 0);
        assert_eq!(indices, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn fuzzy_match_prefix() {
        let (score, indices) = fuzzy_match("mo", "model").unwrap();
        assert!(score > 0);
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn fuzzy_match_subsequence() {
        let result = fuzzy_match("mdl", "model");
        assert!(result.is_some());
        let (_, indices) = result.unwrap();
        assert_eq!(indices.len(), 3);
        // m=0, d=2 (not present literally, but o at 1 doesn't match d)
        // Actually 'm' at 0, 'd' at 2, 'l' at 4
        assert_eq!(indices[0], 0); // m
    }

    #[test]
    fn fuzzy_match_no_match() {
        assert!(fuzzy_match("xyz", "model").is_none());
    }

    #[test]
    fn fuzzy_match_empty_query() {
        let (score, indices) = fuzzy_match("", "model").unwrap();
        assert_eq!(score, 0);
        assert!(indices.is_empty());
    }

    #[test]
    fn fuzzy_match_case_insensitive() {
        let result = fuzzy_match("MO", "model");
        assert!(result.is_some());
        let (_, indices) = result.unwrap();
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn fuzzy_match_word_boundary_bonus() {
        // 'h' at a word boundary (after '/') should score higher
        let (boundary_score, _) = fuzzy_match("h", "/help").unwrap();
        let (mid_score, _) = fuzzy_match("e", "/help").unwrap();
        assert!(boundary_score > mid_score);
    }

    // -----------------------------------------------------------------------
    // CompletionState tests
    // -----------------------------------------------------------------------

    #[test]
    fn completion_state_slash_trigger() {
        let mut cs = CompletionState::new();
        cs.update("/");
        assert!(cs.visible);
        assert_eq!(cs.matches.len(), SLASH_COMMANDS.len());
    }

    #[test]
    fn completion_state_filtering() {
        let mut cs = CompletionState::new();
        cs.update("/he");
        assert!(cs.visible);
        // Should match /help at minimum
        assert!(cs.matches.iter().any(|m| m.command == "/help"));
    }

    #[test]
    fn completion_state_no_match() {
        let mut cs = CompletionState::new();
        cs.update("/zzzzz");
        assert!(!cs.visible);
        assert!(cs.matches.is_empty());
    }

    #[test]
    fn completion_state_navigation_wrap() {
        let mut cs = CompletionState::new();
        cs.update("/");
        let count = cs.matches.len();
        assert!(count > 1);

        // Wraps forward
        for _ in 0..count {
            cs.select_next();
        }
        assert_eq!(cs.selected, 0);

        // Wraps backward from 0
        cs.select_prev();
        assert_eq!(cs.selected, count - 1);
    }

    #[test]
    fn completion_state_accept() {
        let mut cs = CompletionState::new();
        cs.update("/he");
        assert!(cs.visible);
        let accepted = cs.accept();
        assert!(accepted.is_some());
        assert!(accepted.unwrap().starts_with('/'));
        assert!(!cs.visible); // dismissed after accept
    }

    #[test]
    fn completion_state_dismiss() {
        let mut cs = CompletionState::new();
        cs.update("/");
        assert!(cs.visible);
        cs.dismiss();
        assert!(!cs.visible);
        assert!(cs.matches.is_empty());
    }

    #[test]
    fn completion_state_non_slash_dismissed() {
        let mut cs = CompletionState::new();
        cs.update("hello");
        assert!(!cs.visible);
    }

    // -----------------------------------------------------------------------
    // ghost_suggestion tests
    // -----------------------------------------------------------------------

    #[test]
    fn ghost_suggestion_from_history() {
        let mut input = InputState::new();
        input.buffer = "hello world".into();
        input.submit();
        // Now type "hel" — should suggest "lo world"
        input.buffer = "hel".into();
        input.cursor = 3;
        let ghost = input.ghost_suggestion();
        assert_eq!(ghost, Some("lo world"));
    }

    #[test]
    fn ghost_suggestion_none_when_empty() {
        let input = InputState::new();
        assert!(input.ghost_suggestion().is_none());
    }

    #[test]
    fn ghost_suggestion_none_for_slash() {
        let mut input = InputState::new();
        input.buffer = "/mo".into();
        input.cursor = 3;
        assert!(input.ghost_suggestion().is_none());
    }

    #[test]
    fn ghost_suggestion_prefers_recent() {
        let mut input = InputState::new();
        input.buffer = "test alpha".into();
        input.submit();
        input.buffer = "test beta".into();
        input.submit();
        // "test" should match "test beta" (most recent)
        input.buffer = "test".into();
        input.cursor = 4;
        assert_eq!(input.ghost_suggestion(), Some(" beta"));
    }

    #[test]
    fn ghost_suggestion_none_cursor_not_at_end() {
        let mut input = InputState::new();
        input.buffer = "hello world".into();
        input.submit();
        input.buffer = "hello".into();
        input.cursor = 3; // not at end
        assert!(input.ghost_suggestion().is_none());
    }

    #[test]
    fn ghost_suggestion_accept() {
        let mut input = InputState::new();
        input.buffer = "hello world".into();
        input.submit();
        input.buffer = "hel".into();
        input.cursor = 3;
        assert!(input.accept_ghost());
        assert_eq!(input.buffer, "hello world");
        assert_eq!(input.cursor, 11);
    }

    // -----------------------------------------------------------------------
    // thinking_label tests
    // -----------------------------------------------------------------------

    #[test]
    fn thinking_label_phases() {
        assert_eq!(thinking_label(0.5), "Connecting...");
        assert_eq!(thinking_label(1.9), "Connecting...");
        assert_eq!(thinking_label(2.0), "Thinking...");
        assert_eq!(thinking_label(5.0), "Thinking...");
        assert_eq!(thinking_label(8.0), "Still thinking...");
        assert_eq!(thinking_label(12.0), "Still thinking...");
        assert_eq!(thinking_label(15.0), "Deep in thought...");
        assert_eq!(thinking_label(30.0), "Deep in thought...");
    }

    // -----------------------------------------------------------------------
    // history persistence tests
    // -----------------------------------------------------------------------

    #[test]
    fn history_loaded_into_input() {
        // Verify that InputState can accept pre-loaded history
        let mut input = InputState::new();
        input.history = vec!["first".to_string(), "second".to_string()];
        input.history_up();
        assert_eq!(input.buffer, "second");
        input.history_up();
        assert_eq!(input.buffer, "first");
    }

    #[test]
    fn ghost_works_with_loaded_history() {
        let mut input = InputState::new();
        input.history = vec![
            "cargo test --workspace".to_string(),
            "cargo build --release".to_string(),
        ];
        input.buffer = "cargo".into();
        input.cursor = 5;
        // Should suggest " build --release" (most recent match)
        assert_eq!(input.ghost_suggestion(), Some(" build --release"));
    }

    // -----------------------------------------------------------------------
    // multi-line input tests
    // -----------------------------------------------------------------------

    #[test]
    fn insert_newline_basic() {
        let mut input = InputState::new();
        input.insert('a');
        input.insert_newline();
        input.insert('b');
        assert_eq!(input.buffer, "a\nb");
        assert_eq!(input.cursor, 3);
        assert_eq!(input.line_count(), 2);
    }

    #[test]
    fn line_count_single() {
        let mut input = InputState::new();
        input.buffer = "hello world".into();
        assert_eq!(input.line_count(), 1);
    }

    #[test]
    fn line_count_multiple() {
        let mut input = InputState::new();
        input.buffer = "line1\nline2\nline3".into();
        assert_eq!(input.line_count(), 3);
    }

    #[test]
    fn cursor_line_col_first_line() {
        let mut input = InputState::new();
        input.buffer = "hello\nworld".into();
        input.cursor = 3;
        assert_eq!(input.cursor_line_col(), (0, 3));
    }

    #[test]
    fn cursor_line_col_second_line() {
        let mut input = InputState::new();
        input.buffer = "hello\nworld".into();
        input.cursor = 8; // "hello\nwo" -> line 1, col 2
        assert_eq!(input.cursor_line_col(), (1, 2));
    }

    #[test]
    fn cursor_line_col_at_newline() {
        let mut input = InputState::new();
        input.buffer = "hello\nworld".into();
        input.cursor = 6; // right after \n -> line 1, col 0
        assert_eq!(input.cursor_line_col(), (1, 0));
    }

    // -----------------------------------------------------------------------
    // completion with /export
    // -----------------------------------------------------------------------

    #[test]
    fn completion_includes_export() {
        let mut cs = CompletionState::new();
        cs.update("/exp");
        assert!(cs.visible);
        assert!(cs.matches.iter().any(|m| m.command == "/export"));
    }

    // -----------------------------------------------------------------------
    // reading_time tests
    // -----------------------------------------------------------------------

    #[test]
    fn reading_time_short() {
        assert!(reading_time("hello world").is_none());
    }

    #[test]
    fn reading_time_medium() {
        let text = "word ".repeat(150);
        let rt = reading_time(&text);
        assert!(rt.is_some());
        assert!(rt.unwrap().contains("words"));
    }

    #[test]
    fn reading_time_long() {
        let text = "word ".repeat(500);
        let rt = reading_time(&text);
        assert!(rt.is_some());
        assert!(rt.unwrap().contains("min read"));
    }

    // -----------------------------------------------------------------------
    // error_suggestions tests
    // -----------------------------------------------------------------------

    #[test]
    fn error_suggestions_connection() {
        let s = error_suggestions("connection refused");
        assert!(!s.is_empty());
        assert!(s.iter().any(|(_, cmd)| cmd.contains("serve")));
    }

    #[test]
    fn error_suggestions_auth() {
        let s = error_suggestions("401 unauthorized");
        assert!(!s.is_empty());
        assert!(s.iter().any(|(label, _)| label.contains("key")));
    }

    #[test]
    fn error_suggestions_rate_limit() {
        let s = error_suggestions("429 too many requests");
        assert!(!s.is_empty());
    }

    #[test]
    fn error_suggestions_unknown() {
        let s = error_suggestions("something weird happened");
        assert!(s.is_empty());
    }

    // -----------------------------------------------------------------------
    // history search (Ctrl+R) tests
    // -----------------------------------------------------------------------

    #[test]
    fn history_search_basic() {
        let history = vec![
            "cargo build".to_string(),
            "cargo test --workspace".to_string(),
            "fix the login bug".to_string(),
        ];
        let mut search = HistorySearch::new();
        search.active = true;
        search.query = "cargo".to_string();
        search.update(&history);
        assert_eq!(search.matches.len(), 2);
        // Most recent first
        assert_eq!(
            search.current_match(&history),
            Some("cargo test --workspace")
        );
    }

    #[test]
    fn history_search_cycle() {
        let history = vec![
            "alpha one".to_string(),
            "alpha two".to_string(),
            "alpha three".to_string(),
        ];
        let mut search = HistorySearch::new();
        search.query = "alpha".to_string();
        search.update(&history);
        assert_eq!(search.matches.len(), 3);
        assert_eq!(search.current_match(&history), Some("alpha three"));
        search.next_match();
        assert_eq!(search.current_match(&history), Some("alpha two"));
        search.next_match();
        assert_eq!(search.current_match(&history), Some("alpha one"));
        search.next_match(); // wraps
        assert_eq!(search.current_match(&history), Some("alpha three"));
    }

    #[test]
    fn history_search_no_match() {
        let history = vec!["hello".to_string()];
        let mut search = HistorySearch::new();
        search.query = "xyz".to_string();
        search.update(&history);
        assert!(search.matches.is_empty());
        assert!(search.current_match(&history).is_none());
    }

    #[test]
    fn history_search_accept() {
        let history = vec!["cargo build".to_string(), "cargo test".to_string()];
        let mut search = HistorySearch::new();
        search.active = true;
        search.query = "test".to_string();
        search.update(&history);
        let idx = search.accept();
        assert_eq!(idx, Some(1)); // "cargo test" is at index 1
        assert!(!search.active);
    }

    #[test]
    fn history_search_case_insensitive() {
        let history = vec!["Fix THE bug".to_string()];
        let mut search = HistorySearch::new();
        search.query = "fix the".to_string();
        search.update(&history);
        assert_eq!(search.matches.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Command palette tests
    // -----------------------------------------------------------------------

    #[test]
    fn palette_open_shows_all() {
        let mut p = CommandPalette::new();
        p.open();
        assert!(p.active);
        assert_eq!(p.matches.len(), SLASH_COMMANDS.len());
    }

    #[test]
    fn palette_filter() {
        let mut p = CommandPalette::new();
        p.open();
        p.type_char('h');
        p.type_char('e');
        p.type_char('l');
        // Should find /help
        assert!(p.matches.iter().any(|m| m.command == "/help"));
        assert!(p.matches.len() < SLASH_COMMANDS.len());
    }

    #[test]
    fn palette_navigation() {
        let mut p = CommandPalette::new();
        p.open();
        assert_eq!(p.selected, 0);
        p.select_next();
        assert_eq!(p.selected, 1);
        p.select_prev();
        assert_eq!(p.selected, 0);
        // Wrap backward
        p.select_prev();
        assert_eq!(p.selected, p.matches.len() - 1);
    }

    #[test]
    fn palette_accept() {
        let mut p = CommandPalette::new();
        p.open();
        let cmd = p.accept();
        assert!(cmd.is_some());
        assert!(cmd.unwrap().starts_with('/'));
        assert!(!p.active);
    }

    #[test]
    fn palette_dismiss() {
        let mut p = CommandPalette::new();
        p.open();
        p.type_char('x');
        p.dismiss();
        assert!(!p.active);
        assert!(p.query.is_empty());
    }

    #[test]
    fn palette_backspace() {
        let mut p = CommandPalette::new();
        p.open();
        p.type_char('h');
        p.type_char('e');
        let after_he = p.matches.len();
        p.backspace();
        // After removing 'e', should have more matches (just 'h')
        assert!(p.matches.len() >= after_he);
    }

    #[test]
    fn palette_searches_description() {
        let mut p = CommandPalette::new();
        p.open();
        // Search for "version" — should match /version's description "show version info"
        p.type_char('v');
        p.type_char('e');
        p.type_char('r');
        assert!(p.matches.iter().any(|m| m.command == "/version"));
    }

    // -----------------------------------------------------------------------
    // truncate_str tests
    // -----------------------------------------------------------------------

    #[test]
    fn truncate_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_long() {
        assert_eq!(truncate_str("hello world", 8), "hello...");
    }
}
