//! Primitive 2: `StreamingBlock` — live agent output rendered in the viewport.
//!
//! This is the live content area that updates on every token delta. It renders
//! in the inline viewport (not scrollback) and supports auto-scroll.
//!
//! When the agent finishes, the streaming content is finalized and pushed
//! into scrollback as part of a `RunBlock`.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::tui::Theme;

use super::super::symbols;

/// State for a live streaming block.
#[derive(Debug, Clone)]
pub struct StreamingState {
    /// Accumulated text buffer (raw markdown from LLM).
    buffer: String,
    /// Vertical scroll offset (lines from top).
    scroll_offset: u16,
    /// Whether auto-scroll is active (follows new content).
    auto_scroll: bool,
    /// Tick counter for spinner animation.
    tick: u64,
    /// Current phase label (e.g. "Thinking", "Writing", "Analyzing").
    phase_label: String,
    /// When streaming started (for elapsed time display).
    started_at: std::time::Instant,
    /// Token count so far.
    token_count: u64,
    /// Running cost USD.
    cost_usd: f64,
    /// Active model name.
    model: String,
}

impl StreamingState {
    /// Create a new streaming state.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            buffer: String::new(),
            scroll_offset: 0,
            auto_scroll: true,
            tick: 0,
            phase_label: "Thinking".into(),
            started_at: std::time::Instant::now(),
            token_count: 0,
            cost_usd: 0.0,
            model: model.into(),
        }
    }

    /// Append a token delta to the buffer.
    pub fn append(&mut self, text: &str) {
        self.buffer.push_str(text);
        self.phase_label = "Streaming".into();
    }

    /// Set the phase label (e.g. when a tool call starts).
    pub fn set_phase(&mut self, label: impl Into<String>) {
        self.phase_label = label.into();
    }

    /// Update token count and cost.
    pub fn update_usage(&mut self, tokens: u64, cost: f64) {
        self.token_count = tokens;
        self.cost_usd = cost;
    }

    /// Advance the tick counter (call on each frame).
    pub fn tick(&mut self) {
        self.tick += 1;
    }

    /// Scroll up by N lines, disabling auto-scroll.
    pub fn scroll_up(&mut self, lines: u16) {
        self.auto_scroll = false;
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }

    /// Scroll down by N lines. Re-enables auto-scroll if at bottom.
    pub fn scroll_down(&mut self, lines: u16, visible_height: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        let total_lines = self.buffer.lines().count() as u16;
        if self.scroll_offset + visible_height >= total_lines {
            self.auto_scroll = true;
        }
    }

    /// Get the accumulated buffer text.
    #[must_use]
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Take the buffer content, leaving it empty.
    pub fn take_buffer(&mut self) -> String {
        std::mem::take(&mut self.buffer)
    }

    /// Whether the buffer has any content.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Token count.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.token_count
    }

    /// Running cost.
    #[must_use]
    pub fn cost(&self) -> f64 {
        self.cost_usd
    }

    /// Elapsed seconds since streaming started.
    #[must_use]
    pub fn elapsed_s(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }

    /// Render the streaming viewport.
    ///
    /// Layout:
    /// - Top area: streaming text (scrollable)
    /// - Bottom 1 line: status bar
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let chunks = Layout::vertical([
            Constraint::Min(1),    // streaming content
            Constraint::Length(1), // status bar
        ])
        .split(area);

        self.render_content(frame, chunks[0], theme);
        self.render_status(frame, chunks[1], theme);
    }

    fn render_content(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        if self.buffer.is_empty() {
            // Show spinner when thinking (no content yet)
            let elapsed = self.elapsed_s();
            let line = super::super::styled::spinner_line(
                theme,
                self.tick,
                &format!("{}...", self.phase_label),
                elapsed,
            );
            frame.render_widget(Paragraph::new(line), area);
            return;
        }

        // Render buffer as wrapped paragraph with auto-scroll
        let mut lines: Vec<Line<'_>> = Vec::new();

        // Header line
        lines.push(Line::from(vec![
            Span::styled(symbols::BAR, theme.muted()),
            Span::raw(" "),
        ]));

        // Content lines with bar prefix
        for text_line in self.buffer.lines() {
            lines.push(Line::from(vec![
                Span::styled(symbols::BAR, theme.muted()),
                Span::raw(" "),
                Span::styled(text_line.to_string(), theme.text()),
            ]));
        }

        // Add cursor at end of last line if still streaming
        if let Some(last) = lines.last_mut() {
            last.spans.push(Span::styled(
                symbols::CURSOR,
                Style::default()
                    .fg(Theme::ROSE)
                    .add_modifier(Modifier::SLOW_BLINK),
            ));
        }

        let total_lines = lines.len() as u16;
        let visible = area.height;
        let scroll = if self.auto_scroll {
            total_lines.saturating_sub(visible)
        } else {
            self.scroll_offset
        };

        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));

        frame.render_widget(paragraph, area);
    }

    fn render_status(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let elapsed = self.elapsed_s();
        let status = super::super::styled::status_bar(
            theme,
            self.cost_usd,
            self.token_count,
            0, // output tokens not tracked separately during stream
            &self.model,
            None, // no progress bar during streaming
        );

        // Prepend elapsed time
        let mut spans = vec![
            Span::styled(
                format!("{elapsed:.1}s"),
                Style::default().fg(Theme::TEXT_GHOST),
            ),
            Span::styled(format!("  {}  ", symbols::SEP), theme.muted()),
        ];
        spans.extend(status.spans);

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_state_new() {
        let state = StreamingState::new("haiku");
        assert!(state.is_empty());
        assert_eq!(state.tokens(), 0);
    }

    #[test]
    fn streaming_state_append() {
        let mut state = StreamingState::new("haiku");
        state.append("Hello ");
        state.append("world");
        assert_eq!(state.buffer(), "Hello world");
        assert!(!state.is_empty());
    }

    #[test]
    fn streaming_state_take() {
        let mut state = StreamingState::new("haiku");
        state.append("test");
        let taken = state.take_buffer();
        assert_eq!(taken, "test");
        assert!(state.is_empty());
    }

    #[test]
    fn streaming_state_scroll() {
        let mut state = StreamingState::new("haiku");
        assert!(state.auto_scroll);
        state.scroll_up(5);
        assert!(!state.auto_scroll);
        state.scroll_down(10, 20);
        assert!(state.auto_scroll); // should re-enable at bottom
    }
}
