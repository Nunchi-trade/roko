//! F5 Logs view -- scrollable log tail with level-based coloring.
//!
//! Single-panel scrollable view. Log levels are color-coded:
//! Info=white, Warn=yellow, Error=red, Debug=gray.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

/// A parsed log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
}

/// Log severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Parse a level string (case-insensitive).
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "debug" | "trace" => Self::Debug,
            "warn" | "warning" => Self::Warn,
            "error" | "err" | "fatal" => Self::Error,
            _ => Self::Info,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Debug => "DBG",
            Self::Info => "INF",
            Self::Warn => "WRN",
            Self::Error => "ERR",
        }
    }
}

/// Render the full logs view.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    _tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    // Logs are not yet stored in DashboardData; render with
    // placeholder entries. The integration layer will supply
    // LogEntry data via an extended render function once wired.
    let entries = build_log_entries_from_signals(data);
    render_with_entries(frame, area, &entries, view_state, theme);
}

/// Render the logs view with explicit log entries (for integration layer).
pub fn render_with_entries(
    frame: &mut Frame<'_>,
    area: Rect,
    entries: &[LogEntry],
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // Status bar
    let tail_label = if view_state.auto_tail {
        "TAIL"
    } else {
        "SCROLL"
    };
    let status = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} entries ", entries.len()),
            theme.muted(),
        ),
        Span::styled(format!("[{tail_label}]"), theme.accent()),
    ]))
    .alignment(Alignment::Right);
    frame.render_widget(status, sections[0]);

    // Log content
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Logs ")
        .border_style(theme.accent());
    let inner = block.inner(sections[1]);
    frame.render_widget(block, sections[1]);

    if entries.is_empty() {
        let empty = Paragraph::new("no log entries")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let lines: Vec<Line<'_>> = entries
        .iter()
        .map(|entry| {
            let level_style = level_style(entry.level, theme);
            Line::from(vec![
                Span::styled(&entry.timestamp, theme.muted()),
                Span::raw(" "),
                Span::styled(
                    format!("[{}]", entry.level.label()),
                    level_style,
                ),
                Span::raw(" "),
                Span::styled(&entry.source, theme.accent()),
                Span::raw(": "),
                Span::styled(&entry.message, level_style.remove_modifier(Modifier::BOLD)),
            ])
        })
        .collect();

    let scroll = if view_state.auto_tail {
        lines.len().saturating_sub(inner.height as usize) as u16
    } else {
        view_state.scroll
    };

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, inner);
}

/// Build log entries from signal data as a fallback.
fn build_log_entries_from_signals(data: &DashboardData) -> Vec<LogEntry> {
    data.recent_signals
        .iter()
        .map(|signal| {
            let level = if signal.kind.contains("error") || signal.kind.contains("fail") {
                LogLevel::Error
            } else if signal.kind.contains("warn") {
                LogLevel::Warn
            } else if signal.kind.contains("debug") {
                LogLevel::Debug
            } else {
                LogLevel::Info
            };

            let ts = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(signal.created_at_ms)
                .map(|dt| dt.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| String::from("??:??:??"));

            LogEntry {
                timestamp: ts,
                level,
                source: signal.kind.clone(),
                message: truncate(&signal.payload_preview, 120),
            }
        })
        .collect()
}

fn level_style(level: LogLevel, theme: &Theme) -> ratatui::style::Style {
    match level {
        LogLevel::Debug => theme.muted(),
        LogLevel::Info => theme.text(),
        LogLevel::Warn => theme.warning(),
        LogLevel::Error => theme.danger(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
