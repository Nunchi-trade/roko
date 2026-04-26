//! `roko chat` with inline ratatui UX.
//!
//! Claude Code-like experience: streaming responses in a fixed viewport at the
//! bottom, completed turns pushed into terminal scrollback. Supports multi-line
//! input, `/` commands, Ctrl+C interrupt, and cost tracking.
//!
//! Falls back to the legacy line-oriented REPL when stdout is not a TTY.

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
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
use crate::chat::{self, extract_clean_text};
use crate::inline::primitives::{CostMeter, StreamingState};
use crate::inline::styled;
use crate::inline::symbols;
use crate::inline::terminal::InlineTerminal;
use crate::tui::Theme;

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
}

impl InputState {
    fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_idx: None,
            saved_input: String::new(),
        }
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

/// Full chat session state.
struct ChatSession {
    phase: Phase,
    input: InputState,
    streaming: StreamingState,
    cost: CostMeter,
    agent_id: String,
    tick: u64,
    started_at: Instant,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

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

    let mut term = InlineTerminal::new().context("init inline terminal")?;
    let theme = *term.theme();

    // Push header
    term.push_lines(&[
        styled::section_start(&theme, "roko chat", agent_id, Some(serve_url)),
        Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(
                "Type a message. Ctrl-D to exit. /help for commands.".to_string(),
                Style::default().fg(Theme::TEXT_DIM),
            ),
        ]),
        Line::raw(""),
    ])?;

    let mut session = ChatSession {
        phase: Phase::Input,
        input: InputState::new(),
        streaming: StreamingState::new("unknown"),
        cost: CostMeter::new(),
        agent_id: agent_id.to_string(),
        tick: 0,
        started_at: Instant::now(),
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
                        if handle_input_key(key, &mut session, &mut term, &theme, &client, serve_url).await? {
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
            &theme, "session", &summary_line, None,
        )])?;
    }

    term.push_blank()?;
    drop(term); // restores terminal

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
    client: &reqwest::Client,
    serve_url: &str,
) -> Result<bool> {
    match key.code {
        // Submit
        KeyCode::Enter => {
            if session.input.is_empty() {
                return Ok(false);
            }
            let text = session.input.submit();

            // Handle / commands
            if text.starts_with('/') {
                return handle_slash_command(&text, session, term, theme);
            }

            // Push user message to scrollback
            term.push_lines(&[Line::from(vec![
                Span::styled(
                    format!("{} ", symbols::PROMPT),
                    Style::default().fg(Theme::ROSE),
                ),
                Span::styled(text.clone(), Style::default().fg(Theme::BONE)),
            ])])?;

            // Start thinking phase
            session.phase = Phase::Thinking;
            session.streaming = StreamingState::new("resolving...");

            // Send message and get response (blocking for now)
            let response = send_and_receive(client, serve_url, &session.agent_id, &text).await;

            match response {
                Ok(reply) => {
                    push_agent_response(term, theme, &reply, &session.agent_id)?;
                    // Record in cost meter (approximate — real cost tracking comes later)
                    let approx_tokens = (reply.len() as u64) / 4;
                    session.cost.record_run(0.0, approx_tokens, approx_tokens, "unknown", 0.0);
                }
                Err(err) => {
                    term.push_lines(&[styled::continuation(
                        theme,
                        "error",
                        &format!("{err}"),
                        None,
                    )])?;
                }
            }

            session.phase = Phase::Input;
            term.push_blank()?;
        }

        // Exit
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
        }

        // Editing
        KeyCode::Backspace => session.input.backspace(),
        KeyCode::Delete => session.input.delete(),
        KeyCode::Left => session.input.move_left(),
        KeyCode::Right => session.input.move_right(),
        KeyCode::Home | KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session.input.home();
        }
        KeyCode::End | KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session.input.end();
        }

        // History
        KeyCode::Up => session.input.history_up(),
        KeyCode::Down => session.input.history_down(),

        // Clear line
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session.input.clear();
        }

        // Regular character input
        KeyCode::Char(ch) => session.input.insert(ch),

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
                styled::continuation(theme, "/cost", "show session cost summary", None),
                styled::continuation(theme, "/clear", "clear scrollback", None),
                styled::section_end(theme, "/quit", "exit the chat"),
            ])?;
        }
        "/cost" => {
            let ratio = session.cost.savings_ratio();
            term.push_lines(&[
                styled::section_start(theme, "cost", "session summary", None),
                styled::continuation(
                    theme,
                    "turns",
                    &session.cost.run_count.to_string(),
                    None,
                ),
                styled::continuation(
                    theme,
                    "total",
                    &format!("${:.4}", session.cost.total_cost),
                    None,
                ),
                styled::continuation(
                    theme,
                    "tokens",
                    &format!("{} in / {} out", session.cost.input_tokens, session.cost.output_tokens),
                    None,
                ),
                styled::section_end(
                    theme,
                    "savings",
                    &format!("{ratio:.1}x vs baseline"),
                ),
            ])?;
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
            let chunks = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

            let spinner = styled::spinner_line(
                theme,
                session.tick,
                "Thinking...",
                session.streaming.elapsed_s(),
            );
            frame.render_widget(Paragraph::new(spinner), chunks[0]);
            render_status_bar(frame, chunks[1], session, theme);
        }
        Phase::Streaming => {
            session.streaming.render(frame, area, theme);
        }
        Phase::Done => {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("bye.".to_string(), theme.muted()),
                ])),
                area,
            );
        }
    }
}

fn render_input(frame: &mut Frame<'_>, area: Rect, session: &ChatSession, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Min(1),    // spacer
        Constraint::Length(1), // input line
        Constraint::Length(1), // status bar
    ])
    .split(area);

    // Input prompt
    let before_cursor = &session.input.buffer[..session.input.cursor];
    let after_cursor = &session.input.buffer[session.input.cursor..];

    let input_line = Line::from(vec![
        Span::styled(
            format!("{} ", symbols::PROMPT),
            Style::default().fg(Theme::ROSE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(before_cursor.to_string(), theme.text()),
        Span::styled(
            if after_cursor.is_empty() {
                symbols::CURSOR.to_string()
            } else {
                after_cursor.chars().next().unwrap_or(' ').to_string()
            },
            Style::default()
                .fg(Theme::BONE)
                .add_modifier(Modifier::REVERSED),
        ),
        Span::styled(
            if after_cursor.len() > 1 {
                after_cursor[after_cursor.chars().next().map(|c| c.len_utf8()).unwrap_or(1)..].to_string()
            } else {
                String::new()
            },
            theme.text(),
        ),
    ]);
    frame.render_widget(Paragraph::new(input_line), chunks[1]);

    render_status_bar(frame, chunks[2], session, theme);
}

fn render_status_bar(frame: &mut Frame<'_>, area: Rect, session: &ChatSession, theme: &Theme) {
    let model = session
        .cost
        .primary_model()
        .unwrap_or("—")
        .to_string();

    let bar = styled::status_bar(
        theme,
        session.cost.total_cost,
        session.cost.input_tokens,
        session.cost.output_tokens,
        &model,
        None,
    );
    frame.render_widget(Paragraph::new(bar), area);
}

// ---------------------------------------------------------------------------
// Backend communication
// ---------------------------------------------------------------------------

/// Send a message and receive the response. Currently blocking (no streaming).
async fn send_and_receive(
    client: &reqwest::Client,
    serve_url: &str,
    agent_id: &str,
    message: &str,
) -> Result<String> {
    let url = format!(
        "{}/api/agents/{agent_id}/message",
        serve_url.trim_end_matches('/'),
    );
    let body = json!({ "message": message });

    let response = client
        .post(&url)
        .json(&body)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .context("send message")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("request failed: {status} {body}");
    }

    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        response: Option<String>,
        #[serde(default)]
        run_id: Option<String>,
    }

    let resp: Resp = response.json().await.context("decode response")?;

    if let Some(text) = resp.response.filter(|s| !s.trim().is_empty()) {
        return Ok(extract_clean_text(&text));
    }

    if let Some(run_id) = resp.run_id.filter(|s| !s.trim().is_empty()) {
        // Poll for completion
        let status_url = format!(
            "{}/api/run/{run_id}/status",
            serve_url.trim_end_matches('/'),
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
            }

            let status: StatusResp = status_resp.json().await.context("decode status")?;
            if status.finished {
                if let Some(err) = status.error.filter(|s| !s.trim().is_empty()) {
                    bail!("agent error: {err}");
                }
                return Ok(status.output_text.unwrap_or_default());
            }
        }
    }

    bail!("no response or run_id in reply");
}

/// Push a completed agent response into scrollback with markdown rendering.
fn push_agent_response(
    term: &mut InlineTerminal,
    theme: &Theme,
    text: &str,
    agent_id: &str,
) -> std::io::Result<()> {
    let mut lines = Vec::new();

    // Agent header
    lines.push(Line::from(vec![
        Span::styled(symbols::START.to_string(), theme.info()),
        Span::raw(" "),
        Span::styled(
            agent_id.to_string(),
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        ),
    ]));

    // Response body — rendered as markdown with bar prefix
    let md_lines = crate::inline::markdown::render_markdown_with_bar(text, theme);
    lines.extend(md_lines);

    // Close
    lines.push(Line::from(vec![
        Span::styled(symbols::END.to_string(), theme.muted()),
    ]));

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
}
