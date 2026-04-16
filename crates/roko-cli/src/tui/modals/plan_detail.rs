//! Scrollable plan detail modal.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::tui::dashboard::Theme;
use crate::tui::state::{PlanEntry, TaskStatus};

/// Render the plan detail modal overlay.
pub fn render_plan_detail_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &PlanEntry,
    scroll: u16,
    theme: &Theme,
) {
    let popup = centered_rect(86, 84, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Plan: {} ", plan.id))
        .title_alignment(Alignment::Center)
        .border_style(theme.warning());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

    // Header
    let pct = if plan.tasks_total > 0 {
        plan.tasks_done as f64 / plan.tasks_total as f64
    } else {
        0.0
    };

    let (status_str, status_style) = if plan.status.is_done() {
        ("COMPLETE", theme.success())
    } else if plan.status.is_failed() {
        ("FAILED", theme.danger())
    } else if plan.status.is_active() || plan.active {
        ("ACTIVE", theme.info())
    } else {
        ("PENDING", theme.muted())
    };

    let header_lines = vec![
        Line::from(vec![
            Span::styled("Name:   ", theme.muted()),
            Span::styled(
                plan.name.as_str(),
                theme.text().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Status: ", theme.muted()),
            Span::styled(status_str, status_style),
            Span::styled(format!("  Phase: {}", plan.phase), theme.text()),
        ]),
        Line::from(vec![
            Span::styled("Progress: ", theme.muted()),
            Span::styled(
                format!(
                    "{}/{} ({:.0}%)",
                    plan.tasks_done,
                    plan.tasks_total,
                    pct * 100.0
                ),
                theme.text(),
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(header_lines)
            .style(theme.text())
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    // Footer
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Esc]", theme.accent_bold()),
            Span::styled(" close  ", theme.muted()),
            Span::styled("[Up/Down]", theme.accent_bold()),
            Span::styled(" scroll", theme.muted()),
        ])),
        chunks[2],
    );

    // Task list (scrollable).
    if plan.tasks.is_empty() {
        let empty = Paragraph::new("No tasks recorded for this plan.")
            .style(theme.muted())
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        frame.render_widget(empty, chunks[1]);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        "Tasks",
        theme.accent().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for task in &plan.tasks {
        let task_status = task.status;
        let phase_style = match task_status {
            TaskStatus::Done => theme.success(),
            TaskStatus::Failed | TaskStatus::Blocked => theme.danger(),
            TaskStatus::Active => theme.warning(),
            TaskStatus::Pending => theme.muted(),
        };
        let outcome_style = match task_status {
            TaskStatus::Failed | TaskStatus::Blocked => theme.danger(),
            TaskStatus::Done => theme.success(),
            TaskStatus::Active | TaskStatus::Pending => theme.muted(),
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<24}", task.id),
                theme.text().add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:<14}", task_status), phase_style),
            Span::styled(task.name.as_str(), outcome_style),
        ]));
    }

    let paragraph = Paragraph::new(lines)
        .style(theme.text())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, chunks[1]);
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
