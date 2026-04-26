//! F5 Logs view -- scrollable log tail with level-based coloring.
//!
//! Multi-source log view combining:
//! - Signals from `.roko/engrams.jsonl`
//! - Episodes from `.roko/episodes.jsonl`
//! - Efficiency events from `.roko/learn/efficiency.jsonl`
//! - Verify results from signal data
//!
//! Each source is color-coded by type and severity.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::input::FocusZone;
use crate::tui::input::LogFilterLevel;
use crate::tui::state::{LogEntry, LogEntryLevel, TuiState};

/// Render the full logs view.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    render_with_entries(
        frame,
        area,
        tui_state.unified_log_entries(),
        _data,
        tui_state,
        view_state,
        theme,
    );
}

/// Count visible log entries after applying the active level filter.
pub(crate) fn filtered_entry_count(_data: &DashboardData, tui_state: &TuiState) -> usize {
    tui_state
        .unified_log_entries()
        .iter()
        .filter(|entry| tui_state.log_level_visible(entry.level.filter_level()))
        .count()
}

/// Render the logs view with explicit log entries (for integration layer).
fn render_with_entries(
    frame: &mut Frame<'_>,
    area: Rect,
    entries: &[LogEntry],
    _data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(area);
    let filtered_entries: Vec<&LogEntry> = entries
        .iter()
        .filter(|entry| tui_state.log_level_visible(entry.level.filter_level()))
        .collect();

    // Status bar with source counts
    let signal_count = tui_state.recent_signals.len();
    let episode_count = tui_state.episodes_cache.len();
    let eff_count = tui_state.efficiency_events.len();
    let gate_count = tui_state.gate_result_summaries.len();
    let event_count = tui_state.event_log.len();

    let tail_label = if view_state.auto_tail {
        "TAIL"
    } else {
        "SCROLL"
    };
    let focused = matches!(tui_state.focus, FocusZone::RightPanel);
    let entry_label = if filtered_entries.len() == entries.len() {
        format!(" {} entries ", entries.len())
    } else {
        format!(" {}/{} entries ", filtered_entries.len(), entries.len())
    };
    let mut status_spans = vec![
        Span::styled(entry_label, theme.muted()),
        Span::styled(format!("[{tail_label}]"), theme.accent()),
        Span::styled("  |  levels:", theme.muted()),
    ];
    for level in LogFilterLevel::all() {
        let style = if tui_state.log_level_visible(level) {
            level_filter_style(level, theme)
        } else {
            theme.muted()
        };
        status_spans.push(Span::raw(" "));
        status_spans.push(Span::styled(format!("[{}]", level.label()), style));
    }
    status_spans.extend([
        Span::styled("  |  ", theme.muted()),
        Span::styled(format!("signals:{signal_count}"), theme.info()),
        Span::styled("  ", theme.muted()),
        Span::styled(format!("episodes:{episode_count}"), theme.accent()),
        Span::styled("  ", theme.muted()),
        Span::styled(format!("efficiency:{eff_count}"), theme.muted()),
        Span::styled("  ", theme.muted()),
        Span::styled(format!("gates:{gate_count}"), theme.warning()),
        Span::styled("  ", theme.muted()),
        Span::styled(format!("events:{event_count}"), theme.text()),
    ]);
    let status_line1 = Line::from(status_spans);
    let status = Paragraph::new(vec![status_line1]).alignment(Alignment::Right);
    frame.render_widget(status, sections[0]);

    // Log content
    let border_style = if focused {
        Theme::focused_border_style()
    } else {
        theme.accent()
    };
    let title_style = if focused {
        Theme::focused_title_style()
    } else {
        theme.accent()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Logs ", title_style))
        .border_style(border_style);
    let inner = block.inner(sections[1]);
    frame.render_widget(block, sections[1]);

    if filtered_entries.is_empty() {
        let empty_text = "no log entries -- run agents to generate signals and episodes";
        let empty = Paragraph::new(empty_text)
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let row_focus_idx = if view_state.auto_tail {
        filtered_entries.len().saturating_sub(1)
    } else {
        (view_state.scroll as usize).min(filtered_entries.len().saturating_sub(1))
    };

    let lines: Vec<Line<'_>> = filtered_entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let selected = idx == row_focus_idx;
            let row_bg = if selected {
                Some(theme.selection_background)
            } else {
                None
            };
            let prefix_style = if selected {
                theme.selection()
            } else {
                theme.muted()
            };
            let entry_level_style = style_with_bg(level_style(entry.level, theme), row_bg);
            let source_style = style_with_bg(source_style(&entry.source, theme), row_bg);
            let message_style = style_with_bg(
                level_style(entry.level, theme).remove_modifier(Modifier::BOLD),
                row_bg,
            );
            let ts_style = style_with_bg(theme.muted(), row_bg);

            Line::from(vec![
                Span::styled(if selected { "▶" } else { " " }, prefix_style),
                Span::raw(" "),
                Span::styled(&entry.timestamp, ts_style),
                Span::raw(" "),
                Span::styled(format!("[{}]", entry.level.label()), entry_level_style),
                Span::raw(" "),
                Span::styled(&entry.source, source_style),
                Span::raw(": "),
                Span::styled(&entry.message, message_style),
            ])
        })
        .collect();

    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    let max_scroll = max_scroll.min(u16::MAX as usize) as u16;
    let scroll = if view_state.auto_tail {
        max_scroll
    } else {
        view_state.scroll.min(max_scroll)
    };

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, inner);
}

/// Color style for log levels.
fn level_style(level: LogEntryLevel, theme: &Theme) -> ratatui::style::Style {
    match level {
        LogEntryLevel::Debug => theme.muted(),
        LogEntryLevel::Info => theme.text(),
        LogEntryLevel::Warn => theme.warning(),
        LogEntryLevel::Error => theme.danger(),
    }
}

fn level_filter_style(level: LogFilterLevel, theme: &Theme) -> ratatui::style::Style {
    match level {
        LogFilterLevel::Debug => theme.muted(),
        LogFilterLevel::Info => theme.text(),
        LogFilterLevel::Warn => theme.warning(),
        LogFilterLevel::Error => theme.danger(),
    }
}

/// Color style for log sources.
fn source_style(source: &str, theme: &Theme) -> ratatui::style::Style {
    if source.starts_with("signal:") {
        theme.info()
    } else if source.starts_with("episode:") {
        theme.accent()
    } else if source.starts_with("gate:") {
        theme.warning()
    } else if source.starts_with("efficiency:") {
        theme.muted()
    } else if source.starts_with("event:") {
        theme.info()
    } else {
        theme.text()
    }
}

fn style_with_bg(
    style: ratatui::style::Style,
    bg: Option<ratatui::style::Color>,
) -> ratatui::style::Style {
    if let Some(bg) = bg {
        style.bg(bg)
    } else {
        style
    }
}
