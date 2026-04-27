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

use anyhow::{Context as _, Result, bail};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use serde::Deserialize;
use serde_json::json;

use crate::auth;
use crate::auth_detect::AuthMethod;
use crate::chat::{self, extract_clean_text};
use crate::dispatch_direct::{self, DispatchResult};
use crate::inline::primitives::{CostMeter, StreamingState};
use crate::inline::styled;
use crate::inline::symbols;
use crate::inline::terminal::InlineTerminal;
use crate::tui::Theme;

use chrono;
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
    /// Session complete (user pressed Ctrl-D or /quit).
    Done,
}

/// All available slash commands for tab-completion.
const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "show available commands"),
    ("/model", "show or change model"),
    ("/provider", "show current auth/provider"),
    ("/cost", "session cost summary"),
    ("/clear", "clear scrollback"),
    ("/quit", "exit the chat"),
    ("/exit", "exit the chat"),
    ("/auth", "show current auth/provider"),
    ("/export", "export conversation to markdown"),
    ("/retry", "resend the last message"),
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
            } else if matches!(target_chars.get(ti.wrapping_sub(1)), Some('/' | '-' | '_' | ' '))
            {
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
        let col = before.rfind('\n').map(|i| self.cursor - i - 1).unwrap_or(self.cursor);
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
    Direct {
        auth: AuthMethod,
    },
}

/// A recorded conversation message.
#[derive(Debug, Clone)]
struct ConversationMessage {
    role: &'static str, // "user" or "assistant"
    text: String,
    timestamp: String,
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
        Line::raw(""),
    ])?;

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
                    push_agent_response(&mut term, &theme, &result.text, &session.agent_id)?;
                    let ts = format_time(Instant::now());
                    term.push_lines(&[Line::from(vec![Span::styled(
                        format!("  {ts} ({latency:.1}s)"),
                        Style::default().fg(Theme::TEXT_GHOST),
                    )])])?;
                    session.conversation.push(ConversationMessage {
                        role: "assistant",
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
                    session.turn_count += 1;
                    session.thinking_started = None;
                    if latency > 10.0 {
                        print!("\x07");
                    }
                    session.phase = Phase::Input;
                    session.response_rx = None;
                    term.push_blank()?;
                }
                Ok(Err(err)) => {
                    push_error_with_suggestions(&mut term, &theme, &err)?;
                    session.thinking_started = None;
                    session.phase = Phase::Input;
                    session.response_rx = None;
                    term.push_blank()?;
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
    drop(term); // restores terminal

    Ok(())
}

/// Run the unified inline chat with direct dispatch (no HTTP intermediary).
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
    let init_status = if roko_initialized { ".roko/ initialized" } else { ".roko/ not found" };
    term.push_lines(&[
        styled::section_start(
            &theme,
            "roko",
            &format!("v{version}  {}  {}", symbols::SEP, auth.label()),
            None,
        ),
        styled::continuation(
            &theme,
            "workspace",
            &workspace,
            Some(init_status),
        ),
        Line::from(vec![
            Span::styled(symbols::END.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(
                "Type a message. Ctrl-D to exit. /help for commands.".to_string(),
                Style::default().fg(Theme::TEXT_DIM),
            ),
        ]),
        Line::raw(""),
    ])?;

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
        agent_id: "roko".to_string(),
        tick: 0,
        started_at: Instant::now(),
        dispatch: DispatchMode::Direct {
            auth: auth.clone(),
        },
        response_rx: None,
        turn_count: 0,
        thinking_started: None,
        conversation: Vec::new(),
        last_prompt: None,
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
                    push_agent_response(&mut term, &theme, &result.text, &session.agent_id)?;
                    let ts = format_time(Instant::now());
                    term.push_lines(&[Line::from(vec![Span::styled(
                        format!("  {ts} ({latency:.1}s)"),
                        Style::default().fg(Theme::TEXT_GHOST),
                    )])])?;
                    session.conversation.push(ConversationMessage {
                        role: "assistant",
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
                    session.turn_count += 1;
                    session.thinking_started = None;
                    if latency > 10.0 {
                        print!("\x07");
                    }
                    session.phase = Phase::Input;
                    session.response_rx = None;
                    term.push_blank()?;
                }
                Ok(Err(err)) => {
                    push_error_with_suggestions(&mut term, &theme, &err)?;
                    session.thinking_started = None;
                    session.phase = Phase::Input;
                    session.response_rx = None;
                    term.push_blank()?;
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
    drop(term);

    Ok(())
}

// ---------------------------------------------------------------------------
// Input handling
// ---------------------------------------------------------------------------

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
            KeyCode::Esc | KeyCode::Char('c') if key.code == KeyCode::Esc || key.modifiers.contains(KeyModifiers::CONTROL) => {
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
                role: "user",
                text: text.clone(),
                timestamp: timestamp.clone(),
            });

            // Store for potential retry
            session.last_prompt = Some(text.clone());

            // Start thinking phase
            session.phase = Phase::Thinking;
            session.thinking_started = Some(Instant::now());
            session.streaming = StreamingState::new("resolving...");

            // Spawn async dispatch — non-blocking so the spinner animates
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            session.response_rx = Some(rx);

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
                        let result = send_and_receive(
                            &client_clone,
                            &url_owned,
                            &agent_id_owned,
                            &text,
                            sidecar,
                        )
                        .await;
                        let _ = tx
                            .send(result.map(|r| r.into()).map_err(|e| e.to_string()))
                            .await;
                    });
                }
                DispatchMode::Direct { auth } => {
                    let auth_clone = auth.clone();
                    tokio::spawn(async move {
                        let result = dispatch_direct::dispatch_prompt(&auth_clone, &text).await;
                        let _ = tx.send(result.map_err(|e| e.to_string())).await;
                    });
                }
            }
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

/// Handle `/` commands. Returns true if the session should exit.
fn handle_slash_command(
    text: &str,
    session: &mut ChatSession,
    term: &mut InlineTerminal,
    theme: &Theme,
) -> Result<bool> {
    let cmd = text.trim();
    match cmd {
        "/quit" | "/exit" | "/q" => {
            session.phase = Phase::Done;
            return Ok(true);
        }
        "/help" | "/h" => {
            term.push_lines(&[
                styled::section_start(theme, "help", "available commands", None),
                styled::continuation(theme, "/help", "show this help", None),
                styled::continuation(theme, "/model", "show or change model (e.g. /model glm-5.1)", None),
                styled::continuation(theme, "/provider", "show current auth/provider info", None),
                styled::continuation(theme, "/cost", "show session cost summary", None),
                styled::continuation(theme, "/clear", "clear scrollback", None),
                styled::continuation(theme, "/export", "export chat (markdown, json)", None),
                styled::continuation(theme, "/retry", "resend last message", None),
                styled::section_end(theme, "/quit", "exit the chat"),
            ])?;
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
                DispatchMode::Http { backend_url, is_sidecar, .. } => {
                    format!("HTTP {} ({})", backend_url, if *is_sidecar { "sidecar" } else { "serve" })
                }
            };
            term.push_lines(&[styled::continuation(theme, "provider", &info, None)])?;
        }
        _ if cmd.starts_with("/model") => {
            let arg = cmd.strip_prefix("/model").unwrap().trim();
            if arg.is_empty() {
                // Show current model
                let current = match &session.dispatch {
                    DispatchMode::Direct { auth } => match auth {
                        AuthMethod::ClaudeCli => "claude CLI (auto)".to_string(),
                        AuthMethod::AnthropicApi { model, .. } => {
                            let m = model.as_deref().unwrap_or("claude-sonnet-4-6");
                            format!("{m} (Anthropic API)")
                        }
                        AuthMethod::OpenAiCompat { model, base_url, .. } => {
                            let m = model.as_deref().unwrap_or("gpt-4o");
                            format!("{m} ({base_url})")
                        }
                        AuthMethod::NeedsSetup => "none".to_string(),
                    },
                    DispatchMode::Http { .. } => "HTTP backend (model set server-side)".to_string(),
                };
                term.push_lines(&[
                    styled::continuation(theme, "model", &current, None),
                ])?;
            } else {
                // Change model
                match &mut session.dispatch {
                    DispatchMode::Direct { auth } => match auth {
                        AuthMethod::AnthropicApi { model, .. } => {
                            *model = Some(arg.to_string());
                            term.push_lines(&[styled::continuation(
                                theme, "model", &format!("switched to {arg}"), None,
                            )])?;
                        }
                        AuthMethod::OpenAiCompat { model, .. } => {
                            *model = Some(arg.to_string());
                            term.push_lines(&[styled::continuation(
                                theme, "model", &format!("switched to {arg}"), None,
                            )])?;
                        }
                        _ => {
                            term.push_lines(&[styled::continuation(
                                theme, "model", "can only switch models with API providers", Some("set ANTHROPIC_API_KEY, ZAI_API_KEY, or OPENAI_API_KEY"),
                            )])?;
                        }
                    },
                    DispatchMode::Http { .. } => {
                        term.push_lines(&[styled::continuation(
                            theme, "model", "model switching not supported in HTTP mode", None,
                        )])?;
                    }
                }
            }
        }
        _ if cmd.starts_with("/export") => {
            let arg = cmd.strip_prefix("/export").unwrap().trim();
            let format = if arg.is_empty() { "markdown" } else { arg };
            if session.conversation.is_empty() {
                term.push_lines(&[styled::continuation(
                    theme, "export", "no messages to export", None,
                )])?;
            } else {
                match format {
                    "markdown" | "md" => {
                        let exports_dir = std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."))
                            .join(".roko")
                            .join("exports");
                        let _ = std::fs::create_dir_all(&exports_dir);
                        let ts = chrono::Local::now().format("%Y-%m-%d-%H%M");
                        let path = exports_dir.join(format!("chat-{ts}.md"));
                        let model = match &session.dispatch {
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
                        };
                        let mut md = format!(
                            "# Roko Chat — {}\n\n**Model**: {} | **Turns**: {} | **Cost**: ${:.4}\n\n---\n\n",
                            chrono::Local::now().format("%Y-%m-%d %H:%M"),
                            model,
                            session.turn_count,
                            session.cost.total_cost,
                        );
                        for msg in &session.conversation {
                            let role = if msg.role == "user" { "User" } else { "Roko" };
                            md.push_str(&format!("## {role}\n\n{}\n\n---\n\n", msg.text));
                        }
                        match std::fs::write(&path, &md) {
                            Ok(()) => {
                                term.push_lines(&[styled::continuation(
                                    theme, "export", &format!("saved to {}", path.display()), None,
                                )])?;
                            }
                            Err(e) => {
                                term.push_lines(&[styled::continuation(
                                    theme, "error", &format!("export failed: {e}"), None,
                                )])?;
                            }
                        }
                    }
                    "json" => {
                        let exports_dir = std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."))
                            .join(".roko")
                            .join("exports");
                        let _ = std::fs::create_dir_all(&exports_dir);
                        let ts = chrono::Local::now().format("%Y-%m-%d-%H%M");
                        let path = exports_dir.join(format!("chat-{ts}.json"));
                        let messages: Vec<serde_json::Value> = session.conversation.iter().map(|m| {
                            json!({ "role": m.role, "text": m.text, "timestamp": m.timestamp })
                        }).collect();
                        let export = json!({
                            "turns": session.turn_count,
                            "cost": session.cost.total_cost,
                            "tokens_in": session.cost.input_tokens,
                            "tokens_out": session.cost.output_tokens,
                            "messages": messages,
                        });
                        match std::fs::write(&path, serde_json::to_string_pretty(&export).unwrap_or_default()) {
                            Ok(()) => {
                                term.push_lines(&[styled::continuation(
                                    theme, "export", &format!("saved to {}", path.display()), None,
                                )])?;
                            }
                            Err(e) => {
                                term.push_lines(&[styled::continuation(
                                    theme, "error", &format!("export failed: {e}"), None,
                                )])?;
                            }
                        }
                    }
                    _ => {
                        term.push_lines(&[styled::continuation(
                            theme, "export", &format!("unknown format: {format}"), Some("use: markdown, json"),
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
                    theme, "retry", "resending last message", None,
                )])?;
                // Re-submit by populating buffer — user presses Enter to confirm
            } else {
                term.push_lines(&[styled::continuation(
                    theme, "retry", "no previous message to retry", None,
                )])?;
            }
        }
        "/clear" => {
            // Can't truly clear scrollback, but push blank lines
            for _ in 0..term.viewport_height() {
                term.push_blank()?;
            }
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
            Constraint::Min(1),                   // upper spacer
            Constraint::Length(dropdown_height),   // dropdown
            Constraint::Length(input_height),      // input area
            Constraint::Length(1),                 // status bar
        ])
        .split(area)
    } else {
        // No dropdown — 3-zone layout (pad to 4 for uniform indexing)
        let base = Layout::vertical([
            Constraint::Min(1),               // spacer
            Constraint::Length(input_height),  // input area
            Constraint::Length(1),             // status bar
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
            let mut spans: Vec<Span<'static>> = vec![Span::styled(
                prefix.to_string(),
                base_style,
            )];

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
        let matched_text = session.input.search
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
                Style::default().fg(Theme::ROSE).add_modifier(Modifier::BOLD),
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
                    line_text[cursor_col..].chars().next().unwrap_or(' ').to_string()
                };
                let after = if at_end || cursor_col + 1 >= line_text.len() {
                    String::new()
                } else {
                    line_text[cursor_col + line_text[cursor_col..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)..].to_string()
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
                let mut spans = vec![
                    prefix,
                    Span::styled(line_text.to_string(), theme.text()),
                ];
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
            format!("{}/api/agents/{agent_id}/message", base_url.trim_end_matches('/')),
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
                let input_tokens = status.input_tokens.unwrap_or_else(|| (message.len() as u64) / 4);
                let output_tokens = status.output_tokens.unwrap_or_else(|| (text.len() as u64) / 4);
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

fn push_agent_response(
    term: &mut InlineTerminal,
    theme: &Theme,
    text: &str,
    agent_id: &str,
) -> std::io::Result<()> {
    let mut lines = Vec::new();

    // Agent header (with role-specific color and optional reading time)
    let agent_color = Theme::role_accent(agent_id);
    let agent_color = if agent_color == Theme::TEXT_DIM { theme.info } else { agent_color };
    let mut header_spans = vec![
        Span::styled(symbols::START.to_string(), Style::default().fg(agent_color)),
        Span::raw(" "),
        Span::styled(
            agent_id.to_string(),
            Style::default().fg(agent_color).add_modifier(Modifier::BOLD),
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
    } else if err_lower.contains("unauthorized") || err_lower.contains("401")
        || err_lower.contains("invalid api key") || err_lower.contains("authentication")
    {
        suggestions.push(("check key", "echo $ANTHROPIC_API_KEY | head -c 10"));
        suggestions.push(("set key", "export ANTHROPIC_API_KEY=sk-ant-..."));
    } else if err_lower.contains("rate limit") || err_lower.contains("429")
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
    } else if err_lower.contains("context") && (err_lower.contains("length") || err_lower.contains("too long")) {
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
        lines.push(Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
        ]));
        for (label, cmd) in &suggestions {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", symbols::BAR),
                    theme.muted(),
                ),
                Span::styled(
                    format!("  {label}: "),
                    Style::default().fg(Theme::WARNING),
                ),
                Span::styled(
                    cmd.to_string(),
                    Style::default().fg(Theme::BONE),
                ),
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
        assert_eq!(search.current_match(&history), Some("cargo test --workspace"));
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
}
