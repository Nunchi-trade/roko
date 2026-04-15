//! Batch-pause review modal for reviewing completed batch results.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::super::dashboard::Theme;

/// A completed task in the batch review.
#[derive(Debug, Clone)]
pub struct BatchTaskResult {
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub gate_passed: bool,
    pub summary: String,
}

/// Render the batch review modal.
///
/// Shows the completed batch results with approve/reject/skip actions.
/// Centered ~75x65 rectangle.
pub fn render_batch_review(
    frame: &mut Frame<'_>,
    area: Rect,
    batch_name: &str,
    results: &[BatchTaskResult],
    scroll_offset: u16,
    theme: &Theme,
) {
    let popup = centered_rect(75, 65, area);
    frame.render_widget(Clear, popup);

    let title = format!(" Batch Review: {} ", batch_name);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_alignment(Alignment::Center)
        .border_style(theme.accent());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Split: summary, results list, action buttons.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // summary
            Constraint::Min(1),    // results
            Constraint::Length(2), // actions
        ])
        .split(inner);

    // Summary line
    let passed = results.iter().filter(|r| r.gate_passed).count();
    let failed = results.len() - passed;
    let summary_lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} tasks completed  ", results.len()),
                theme.text(),
            ),
            Span::styled(format!("{passed} passed"), theme.success()),
            Span::styled("  ", theme.text()),
            Span::styled(
                format!("{failed} failed"),
                if failed > 0 {
                    theme.danger()
                } else {
                    theme.muted()
                },
            ),
        ]),
        Line::from(""),
    ];
    frame.render_widget(Paragraph::new(summary_lines), chunks[0]);

    // Results list
    let mut lines: Vec<Line<'_>> = Vec::new();
    for result in results {
        let gate_badge = if result.gate_passed {
            Span::styled(" PASS ", theme.success())
        } else {
            Span::styled(" FAIL ", theme.danger())
        };

        let status_style = match result.status.as_str() {
            "done" | "completed" => theme.success(),
            "failed" | "error" => theme.danger(),
            _ => theme.muted(),
        };

        lines.push(Line::from(vec![
            Span::styled(" ", theme.text()),
            gate_badge,
            Span::styled(format!(" {:<10}", result.task_id), theme.muted()),
            Span::styled(format!("{:<10}", result.status), status_style),
            Span::styled(&result.title, theme.text()),
        ]));

        if !result.summary.is_empty() {
            let summary_max = inner.width.saturating_sub(8) as usize;
            let summary = if result.summary.len() > summary_max {
                format!("{}...", &result.summary[..summary_max.saturating_sub(3)])
            } else {
                result.summary.clone()
            };
            lines.push(Line::from(vec![
                Span::styled("        ", theme.text()),
                Span::styled(summary, theme.muted()),
            ]));
        }
    }

    if results.is_empty() {
        lines.push(Line::from(Span::styled(
            " No results in this batch.",
            theme.muted(),
        )));
    }

    let results_para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));
    frame.render_widget(results_para, chunks[1]);

    // Action buttons
    let actions = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(" [a]", theme.success()),
            Span::styled(" approve   ", theme.text()),
            Span::styled("[r]", theme.danger()),
            Span::styled(" reject   ", theme.text()),
            Span::styled("[s]", theme.warning()),
            Span::styled(" skip   ", theme.text()),
            Span::styled("[Esc]", theme.accent_bold()),
            Span::styled(" close", theme.muted()),
        ]),
    ];
    frame.render_widget(Paragraph::new(actions), chunks[2]);
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
