//! Error and event log view.
//!
//! Displays recent errors and gate verdicts in chronological order.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::super::mori_theme::MoriTheme;
use super::super::tui_state::TuiState;

/// Render the error and event log view.
pub fn render_logs_view(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" F4: Logs ")
        .border_style(Style::default().fg(MoriTheme::TEXT_PHANTOM));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Errors section — reverse chronological (most recent first).
    if !state.live.errors.is_empty() {
        lines.push(Line::from(Span::styled(
            "Errors",
            Style::default().fg(MoriTheme::EMBER),
        )));
        lines.push(Line::from(""));

        for err in state.live.errors.iter().rev() {
            lines.push(Line::from(vec![
                Span::styled(
                    format_ts(err.ts_millis),
                    Style::default().fg(MoriTheme::TEXT_GHOST),
                ),
                Span::styled("  ✗ ", Style::default().fg(MoriTheme::EMBER)),
                Span::styled(
                    err.message.clone(),
                    Style::default().fg(MoriTheme::EMBER),
                ),
            ]));
        }

        lines.push(Line::from(""));
    }

    // Gate verdicts — prefer live snapshot, fall back to disk results.
    let live_gates = &state.live.gates;
    let disk_gates = &state.gate_results;
    let has_live_gates = !live_gates.is_empty();
    let has_disk_gates = !disk_gates.is_empty();

    if has_live_gates || has_disk_gates {
        lines.push(Line::from(Span::styled(
            "Gate Verdicts",
            Style::default().fg(MoriTheme::BONE_DIM),
        )));
        lines.push(Line::from(""));

        if has_live_gates {
            for g in live_gates.iter().rev().take(50) {
                let (icon, color) = gate_style(g.passed);
                lines.push(Line::from(vec![
                    Span::styled(
                        format_ts(g.ts_millis),
                        Style::default().fg(MoriTheme::TEXT_GHOST),
                    ),
                    Span::styled(
                        format!("  [{icon}] "),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        format!("{}/{}: {}", g.plan_id, g.task_id, g.gate),
                        Style::default().fg(MoriTheme::TEXT),
                    ),
                ]));
            }
        } else {
            for g in disk_gates.iter().rev().take(50) {
                let (icon, color) = gate_style(g.passed);
                lines.push(Line::from(vec![
                    Span::styled(
                        format_ts(g.ts_millis),
                        Style::default().fg(MoriTheme::TEXT_GHOST),
                    ),
                    Span::styled(
                        format!("  [{icon}] "),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        format!("{}/{}: {}", g.plan_id, g.task_id, g.gate),
                        Style::default().fg(MoriTheme::TEXT),
                    ),
                ]));
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No log entries.",
            Style::default().fg(MoriTheme::TEXT_GHOST),
        )));
    }

    let paragraph = Paragraph::new(lines)
        .style(Style::default().fg(MoriTheme::TEXT))
        .wrap(Wrap { trim: false })
        .scroll((state.output_scroll as u16, 0));

    frame.render_widget(paragraph, inner);
}

/// Returns (icon, color) for a gate verdict.
fn gate_style(passed: bool) -> (&'static str, Color) {
    if passed {
        ("✓", MoriTheme::SAGE)
    } else {
        ("✗", MoriTheme::EMBER)
    }
}

/// Format a Unix-millis timestamp as a short time string.
fn format_ts(ts_millis: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let dt = UNIX_EPOCH + Duration::from_millis(ts_millis);
    let secs = dt.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

    // HH:MM:SS UTC.
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    let seconds = secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}
