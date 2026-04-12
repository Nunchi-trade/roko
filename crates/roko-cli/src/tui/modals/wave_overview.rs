//! Wave progress popup showing execution wave details.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::super::dashboard::Theme;

/// A single plan entry within a wave.
#[derive(Debug, Clone)]
pub struct WavePlanEntry {
    pub plan_id: String,
    pub status: String,
    pub duration_secs: Option<u64>,
}

/// A wave in the execution pipeline.
#[derive(Debug, Clone)]
pub struct WaveInfo {
    pub wave_index: usize,
    pub plans: Vec<WavePlanEntry>,
    pub total_duration_secs: Option<u64>,
    pub eta_secs: Option<u64>,
}

/// Render the wave overview popup.
///
/// Shows all execution waves with their plan statuses, durations, and ETA.
/// Centered ~80x70 rectangle. Scrollable via `scroll_offset`.
pub fn render_wave_overview(
    frame: &mut Frame<'_>,
    area: Rect,
    waves: &[WaveInfo],
    scroll_offset: u16,
    theme: &Theme,
) {
    let popup = centered_rect(80, 70, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Wave Overview ")
        .title_alignment(Alignment::Center)
        .border_style(theme.accent());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines: Vec<Line<'_>> = Vec::new();

    if waves.is_empty() {
        lines.push(Line::from(Span::styled(
            "No execution waves.",
            theme.muted(),
        )));
    } else {
        for wave in waves {
            // Wave header
            let duration_str = wave
                .total_duration_secs
                .map(format_duration)
                .unwrap_or_else(|| "...".into());
            let eta_str = wave
                .eta_secs
                .map(|s| format!("ETA: {}", format_duration(s)))
                .unwrap_or_default();

            lines.push(Line::from(vec![
                Span::styled(
                    format!("Wave {} ", wave.wave_index),
                    theme.accent_bold(),
                ),
                Span::styled(format!("[{duration_str}]"), theme.muted()),
                Span::styled(format!("  {eta_str}"), theme.info()),
            ]));

            // Plan entries
            for plan in &wave.plans {
                let status_style = match plan.status.as_str() {
                    "done" | "completed" => theme.success(),
                    "running" | "active" => theme.info(),
                    "failed" | "error" => theme.danger(),
                    "pending" | "queued" => theme.muted(),
                    _ => theme.text(),
                };

                let dur = plan
                    .duration_secs
                    .map(format_duration)
                    .unwrap_or_else(|| "-".into());

                lines.push(Line::from(vec![
                    Span::styled("  ", theme.text()),
                    Span::styled(
                        format!("{:<20}", plan.plan_id),
                        theme.text(),
                    ),
                    Span::styled(format!("{:<10}", plan.status), status_style),
                    Span::styled(dur, theme.muted()),
                ]));
            }

            lines.push(Line::from(""));
        }
    }

    // Keybinding hint at the bottom.
    lines.push(Line::from(vec![
        Span::styled("[Esc]", theme.accent_bold()),
        Span::styled(" close   ", theme.muted()),
        Span::styled("[j/k]", theme.accent_bold()),
        Span::styled(" scroll", theme.muted()),
    ]));

    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));
    frame.render_widget(paragraph, inner);
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
