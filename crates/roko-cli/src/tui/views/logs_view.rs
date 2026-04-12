//! F5 Logs view -- scrollable log tail with level-based coloring.
//!
//! Multi-source log view combining:
//! - Signals from `.roko/signals.jsonl`
//! - Episodes from `.roko/episodes.jsonl`
//! - Efficiency events from `.roko/learn/efficiency.jsonl`
//! - Gate results from signal data
//!
//! Each source is color-coded by type and severity.

use std::collections::BTreeMap;

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
    pub timestamp_ms: i64,
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
    let entries = build_unified_log(data);
    render_with_entries(frame, area, &entries, data, view_state, theme);
}

/// Render the logs view with explicit log entries (for integration layer).
pub fn render_with_entries(
    frame: &mut Frame<'_>,
    area: Rect,
    entries: &[LogEntry],
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(area);

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
    let status_line1 = Line::from(vec![
        Span::styled(
            format!(" {} entries ", entries.len()),
            theme.muted(),
        ),
        Span::styled(format!("[{tail_label}]"), theme.accent()),
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
    let status = Paragraph::new(vec![status_line1]).alignment(Alignment::Right);
    frame.render_widget(status, sections[0]);

    // Log content
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Logs ")
        .border_style(theme.accent());
    let inner = block.inner(sections[1]);
    frame.render_widget(block, sections[1]);

    if entries.is_empty() {
        let empty = Paragraph::new("no log entries -- run agents to generate signals and episodes")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let lines: Vec<Line<'_>> = entries
        .iter()
        .map(|entry| {
            let level_style = level_style(entry.level, theme);
            let source_style = source_style(&entry.source, theme);
            Line::from(vec![
                Span::styled(&entry.timestamp, theme.muted()),
                Span::raw(" "),
                Span::styled(
                    format!("[{}]", entry.level.label()),
                    level_style,
                ),
                Span::raw(" "),
                Span::styled(&entry.source, source_style),
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
            LogLevel::Error
        } else if signal.kind.contains("warn") {
            LogLevel::Warn
        } else if signal.kind.contains("gate:") {
            if signal.payload_preview.contains("passed") {
                LogLevel::Info
            } else {
                LogLevel::Warn
            }
        } else if signal.kind.contains("debug") {
            LogLevel::Debug
        } else {
            LogLevel::Info
        };

        let ts = format_timestamp_ms(signal.created_at_ms);
        let message = if signal.payload_preview.is_empty() {
            signal.kind.clone()
        } else {
            truncate(&signal.payload_preview, 120)
        };

        entries.insert(
            (signal.created_at_ms, seq),
            LogEntry {
                timestamp: ts,
                timestamp_ms: signal.created_at_ms,
                level,
                source: format!("signal:{}", truncate_kind(&signal.kind)),
                message,
            },
        );
        seq += 1;
    }

    // 2. Episodes
    for episode in data.episodes() {
        let ts_ms = episode.timestamp.timestamp_millis();
        let level = if !episode.success {
            LogLevel::Error
        } else if episode.kind == "gate" {
            LogLevel::Warn
        } else {
            LogLevel::Info
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
            LogEntry {
                timestamp: episode.timestamp.format("%H:%M:%S").to_string(),
                timestamp_ms: ts_ms,
                level,
                source: format!("episode:{}", truncate_kind(&episode.kind)),
                message,
            },
        );
        seq += 1;
    }

    // 3. Efficiency events
    for event in &data.efficiency_events {
        let ts_ms = event.wall_time_ms as i64; // Approximate -- these don't have real timestamps
        let level = if event.cost_usd > 1.0 {
            LogLevel::Warn
        } else {
            LogLevel::Debug
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
            event.wall_time_ms,
            cache_pct,
        );

        entries.insert(
            (ts_ms, seq),
            LogEntry {
                timestamp: String::from("--:--:--"),
                timestamp_ms: ts_ms,
                level,
                source: format!("efficiency:{}", truncate(&event.agent_id, 12)),
                message,
            },
        );
        seq += 1;
    }

    // 4. Gate failures (highlighted)
    for failure in &data.gate_results_page.failure_rows {
        let ts = format_timestamp_ms(failure.created_at_ms);
        entries.insert(
            (failure.created_at_ms, seq),
            LogEntry {
                timestamp: ts,
                timestamp_ms: failure.created_at_ms,
                level: LogLevel::Error,
                source: format!("gate:{}", failure.gate_name),
                message: format!(
                    "FAILED task={} {}",
                    failure.task_id,
                    truncate(&failure.error_excerpt, 80),
                ),
            },
        );
        seq += 1;
    }

    // 5. Orchestrator event log
    for event in &data.event_log {
        let ts_ms = event.timestamp_ms as i64;
        let ts = format_timestamp_ms(ts_ms);
        let level = match event.event_type.as_str() {
            "error" | "task_failed" | "gate_failed" => LogLevel::Error,
            "warning" | "retry" => LogLevel::Warn,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Info,
        };
        let detail = if event.task_id.is_empty() {
            event.message.clone()
        } else {
            format!("[{}] {}", event.task_id, event.message)
        };
        entries.insert(
            (ts_ms, seq),
            LogEntry {
                timestamp: ts,
                timestamp_ms: ts_ms,
                level,
                source: format!("event:{}", truncate(&event.event_type, 16)),
                message: detail,
            },
        );
        seq += 1;
    }

    // Collect and return sorted by time
    entries.into_values().collect()
}

/// Color style for log levels.
fn level_style(level: LogLevel, theme: &Theme) -> ratatui::style::Style {
    match level {
        LogLevel::Debug => theme.muted(),
        LogLevel::Info => theme.text(),
        LogLevel::Warn => theme.warning(),
        LogLevel::Error => theme.danger(),
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
