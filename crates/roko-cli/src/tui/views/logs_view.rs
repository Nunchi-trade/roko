//! F5 Logs view -- scrollable log tail with level-based coloring.
//!
//! Multi-source log view combining:
//! - Signals from `.roko/engrams.jsonl`
//! - Episodes from `.roko/episodes.jsonl`
//! - Efficiency events from `.roko/learn/efficiency.jsonl`
//! - Gate results from signal data
//!
//! Each source is color-coded by type and severity.

use std::collections::BTreeMap;

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
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let entries = build_unified_log(data);
    render_with_entries(frame, area, &entries, data, tui_state, view_state, theme);
}

/// Count visible log entries after applying the active level filter.
pub(crate) fn filtered_entry_count(data: &DashboardData, tui_state: &TuiState) -> usize {
    build_unified_log(data)
        .into_iter()
        .filter(|entry| tui_state.log_level_visible(entry.level.filter_level()))
        .count()
}

/// Render the logs view with explicit log entries (for integration layer).
fn render_with_entries(
    frame: &mut Frame<'_>,
    area: Rect,
    entries: &[LogEntry],
    data: &DashboardData,
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
    let signal_count = data.recent_signals.len();
    let episode_count = data.episodes().len();
    let eff_count = data.efficiency_events.len();
    let gate_count = data.gate_results.len();
    let event_count = data.event_log.len();

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

// ---------------------------------------------------------------------------
// Log construction
// ---------------------------------------------------------------------------

/// Build a unified, time-sorted log from all available data sources.
fn build_unified_log(data: &DashboardData) -> Vec<LogEntry> {
    // Use a BTreeMap keyed by (timestamp_ms, sequence) for stable ordering
    let mut entries: BTreeMap<(i64, usize), LogEntry> = BTreeMap::new();
    let mut seq = 0usize;

    // 1. Signals
    for signal in &data.recent_signals {
        let level = if signal.kind.contains("error") || signal.kind.contains("fail") {
            LogEntryLevel::Error
        } else if signal.kind.contains("warn") {
            LogEntryLevel::Warn
        } else if signal.kind.contains("gate:") {
            if signal.payload_preview.contains("passed") {
                LogEntryLevel::Info
            } else {
                LogEntryLevel::Warn
            }
        } else if signal.kind.contains("debug") {
            LogEntryLevel::Debug
        } else {
            LogEntryLevel::Info
        };

        let ts = format_timestamp_ms(signal.created_at_ms);
        let message = if signal.payload_preview.is_empty() {
            signal.kind.clone()
        } else {
            truncate(&signal.payload_preview, 120)
        };

        entries.insert(
            (signal.created_at_ms, seq),
            LogEntry::new(
                ts,
                level,
                format!("signal:{}", truncate_kind(&signal.kind)),
                message,
            ),
        );
        seq += 1;
    }

    // 2. Episodes
    for episode in data.episodes() {
        let ts_ms = episode.timestamp.timestamp_millis();
        let level = if !episode.success {
            LogEntryLevel::Error
        } else if episode.kind == "gate" {
            LogEntryLevel::Warn
        } else {
            LogEntryLevel::Info
        };

        let duration_str = if episode.duration_secs > 0.0 {
            format!(" ({:.1}s)", episode.duration_secs)
        } else {
            String::new()
        };

        let gate_summary = if !episode.gate_verdicts.is_empty() {
            let passed = episode.gate_verdicts.iter().filter(|g| g.passed).count();
            let total = episode.gate_verdicts.len();
            format!(" gates:{passed}/{total}")
        } else {
            String::new()
        };

        let message = format!(
            "{} [{}] task={}{}{} {}",
            episode.kind,
            if episode.success { "ok" } else { "FAIL" },
            truncate(&episode.task_id, 30),
            duration_str,
            gate_summary,
            if !episode.model.is_empty() {
                format!("model={}", episode.model)
            } else {
                String::new()
            },
        );

        entries.insert(
            (ts_ms, seq),
            LogEntry::new(
                episode.timestamp.format("%H:%M:%S").to_string(),
                level,
                format!("episode:{}", truncate_kind(&episode.kind)),
                message,
            ),
        );
        seq += 1;
    }

    // 3. Efficiency events
    for event in &data.efficiency_events {
        let ts_ms = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
            .map(|dt| dt.timestamp_millis())
            .unwrap_or_else(|_| chrono::Utc::now().timestamp_millis());
        let ts = format_timestamp_ms(ts_ms);
        let duration_ms = event.duration_ms;
        let level = if event.cost_usd > 1.0 {
            LogEntryLevel::Warn
        } else {
            LogEntryLevel::Debug
        };

        let cache_pct = if event.input_tokens > 0 {
            format!(
                " cache:{:.0}%",
                event.cache_read_tokens as f64 / event.input_tokens as f64 * 100.0
            )
        } else {
            String::new()
        };

        let message = format!(
            "{} model={} in={} out={} ${:.4} {}ms{}",
            event.role,
            truncate(&event.model, 20),
            format_count(event.input_tokens),
            format_count(event.output_tokens),
            event.cost_usd,
            duration_ms,
            cache_pct,
        );

        entries.insert(
            (ts_ms, seq),
            LogEntry::new(
                ts,
                level,
                format!("efficiency:{}", truncate(&event.agent_id, 12)),
                message,
            ),
        );
        seq += 1;
    }

    // 4. Gate failures (highlighted)
    for failure in &data.gate_results_page.failure_rows {
        let ts = format_timestamp_ms(failure.created_at_ms);
        entries.insert(
            (failure.created_at_ms, seq),
            LogEntry::new(
                ts,
                LogEntryLevel::Error,
                format!("gate:{}", failure.gate_name),
                format!(
                    "FAILED task={} {}",
                    failure.task_id,
                    truncate(&failure.error_excerpt, 80),
                ),
            ),
        );
        seq += 1;
    }

    // 5. Orchestrator event log
    for event in &data.event_log {
        let ts_ms = event.timestamp_ms as i64;
        let ts = format_timestamp_ms(ts_ms);
        let level = match event.event_type.as_str() {
            "error" | "task_failed" | "gate_failed" => LogEntryLevel::Error,
            "warning" | "retry" => LogEntryLevel::Warn,
            "debug" => LogEntryLevel::Debug,
            _ => LogEntryLevel::Info,
        };
        let detail = if event.task_id.is_empty() {
            event.message.clone()
        } else {
            format!("[{}] {}", event.task_id, event.message)
        };
        entries.insert(
            (ts_ms, seq),
            LogEntry::new(
                ts,
                level,
                format!("event:{}", truncate(&event.event_type, 16)),
                detail,
            ),
        );
        seq += 1;
    }

    // Collect and return sorted by time
    entries.into_values().collect()
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

fn format_timestamp_ms(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| String::from("??:??:??"))
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

/// Truncate a signal kind to the last two segments for readability.
fn truncate_kind(kind: &str) -> String {
    let parts: Vec<&str> = kind.split(':').collect();
    if parts.len() <= 2 {
        kind.to_string()
    } else {
        parts[parts.len() - 2..].join(":")
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
