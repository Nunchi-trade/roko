//! Task detail modal.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::tui::dashboard::{GateSignalSummary, Theme};
use crate::tui::state::{TaskRow, TaskRowStatus};

/// Render the task detail modal for a task in the checklist.
pub fn render_task_detail_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    task: &TaskRow,
    assigned_agents: &[String],
    gate_results: &[GateSignalSummary],
    scroll_offset: usize,
    theme: &Theme,
) {
    let popup = centered_rect(78, 72, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Task Detail: {} ", task.id))
        .title_alignment(Alignment::Center)
        .border_style(theme.accent());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(0)])
        .split(inner);

    let status_style = match task.status {
        TaskRowStatus::Done => theme.success(),
        TaskRowStatus::Active => theme.info(),
        TaskRowStatus::Failed => theme.danger(),
        TaskRowStatus::Blocked => theme.warning(),
        TaskRowStatus::Pending => theme.muted(),
    };
    let assigned = if assigned_agents.is_empty() {
        "unassigned".to_string()
    } else {
        assigned_agents.join(", ")
    };

    let header = vec![
        Line::from(vec![
            Span::styled("Name:    ", theme.muted()),
            Span::styled(&task.title, theme.text().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Task ID: ", theme.muted()),
            Span::styled(&task.id, theme.text()),
        ]),
        Line::from(vec![
            Span::styled("Status:  ", theme.muted()),
            Span::styled(task_status_label(task.status), status_style),
        ]),
        Line::from(vec![
            Span::styled("Elapsed: ", theme.muted()),
            Span::styled(format_elapsed(task.elapsed_secs), theme.text()),
        ]),
        Line::from(vec![
            Span::styled("Agents:  ", theme.muted()),
            Span::styled(assigned, theme.text()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(header).wrap(Wrap { trim: false }),
        sections[0],
    );

    let mut lines = vec![
        Line::from(Span::styled(
            "Verify Results",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if gate_results.is_empty() {
        lines.push(Line::from(Span::styled(
            "No gate results for this task.",
            theme.muted(),
        )));
    } else {
        for gate in gate_results {
            let verdict_style = if gate.passed {
                theme.success()
            } else {
                theme.danger()
            };
            let verdict = if gate.passed { "PASS" } else { "FAIL" };
            let duration = if gate.duration_ms > 0 {
                format!("{}ms", gate.duration_ms)
            } else {
                "--".to_string()
            };

            lines.push(Line::from(vec![
                Span::styled(format!("{verdict:<4} "), verdict_style),
                Span::styled(
                    format!("{:<18}", gate.gate_name),
                    theme.text().add_modifier(Modifier::BOLD),
                ),
                Span::styled(duration, theme.muted()),
            ]));

            if !gate.excerpt.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("      ", theme.muted()),
                    Span::styled(gate.excerpt.as_str(), theme.muted()),
                ]));
            }

            lines.push(Line::from(""));
        }
    }

    let scroll = scroll_offset.min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        sections[1],
    );
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

fn task_status_label(status: TaskRowStatus) -> &'static str {
    match status {
        TaskRowStatus::Pending => "pending",
        TaskRowStatus::Active => "active",
        TaskRowStatus::Done => "done",
        TaskRowStatus::Failed => "failed",
        TaskRowStatus::Blocked => "blocked",
    }
}

fn format_elapsed(elapsed_secs: f64) -> String {
    let elapsed_secs = elapsed_secs.max(0.0).round() as u64;
    let hours = elapsed_secs / 3600;
    let minutes = (elapsed_secs % 3600) / 60;
    let seconds = elapsed_secs % 60;

    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}
