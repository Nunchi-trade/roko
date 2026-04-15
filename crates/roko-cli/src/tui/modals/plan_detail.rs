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
    let popup = centered_rect(80, 70, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Plan: {} ", plan.id))
        .title_alignment(Alignment::Center)
        .border_style(theme.accent());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

    // -- Header -----------------------------------------------------------
    render_header(frame, chunks[0], plan, theme);

    // -- Task list (scrollable) -------------------------------------------
    render_tasks(frame, chunks[1], plan, scroll, theme);

    // -- Footer -----------------------------------------------------------
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Esc]", theme.accent_bold()),
            Span::styled(" close  ", theme.muted()),
            Span::styled("[Up/Down]", theme.accent_bold()),
            Span::styled(" scroll", theme.muted()),
        ])),
        chunks[2],
    );
}

// -------------------------------------------------------------------------
// Header
// -------------------------------------------------------------------------

fn render_header(frame: &mut Frame<'_>, area: Rect, plan: &PlanEntry, theme: &Theme) {
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

    // Simple text progress bar.
    let bar_width: usize = 20;
    let filled = (pct * bar_width as f64).round() as usize;
    let progress_bar = format!(
        "[{}{}]",
        "#".repeat(filled),
        "-".repeat(bar_width.saturating_sub(filled)),
    );

    let header_lines = vec![
        Line::from(vec![
            Span::styled("Name:     ", theme.muted()),
            Span::styled(
                plan.name.as_str(),
                theme.text().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Status:   ", theme.muted()),
            Span::styled(status_str, status_style),
            Span::styled(format!("  Phase: {}", plan.phase), theme.text()),
        ]),
        Line::from(vec![
            Span::styled("Progress: ", theme.muted()),
            Span::styled(
                format!("{}/{}", plan.tasks_done, plan.tasks_total),
                theme.text(),
            ),
            Span::styled(
                if plan.tasks_failed > 0 {
                    format!("  ({} failed)", plan.tasks_failed)
                } else {
                    String::new()
                },
                theme.danger(),
            ),
        ]),
        Line::from(vec![
            Span::styled("          ", theme.muted()),
            Span::styled(&progress_bar, status_style),
            Span::styled(
                format!(" {:.0}%", pct * 100.0),
                theme.text().add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(header_lines)
            .style(theme.text())
            .wrap(Wrap { trim: false }),
        area,
    );
}

// -------------------------------------------------------------------------
// Task list
// -------------------------------------------------------------------------

fn render_tasks(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &PlanEntry,
    scroll: u16,
    theme: &Theme,
) {
    if plan.tasks.is_empty() {
        let empty = Paragraph::new("No tasks recorded for this plan.")
            .style(theme.muted())
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        frame.render_widget(empty, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        "Tasks",
        theme.accent().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for task in &plan.tasks {
        let task_status = TaskStatus::from(task.status.as_str());
        let status_style = match task_status {
            TaskStatus::Done => theme.success(),
            TaskStatus::Failed | TaskStatus::Blocked => theme.danger(),
            TaskStatus::Active => theme.warning(),
            TaskStatus::Pending => theme.muted(),
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<24}", task.id),
                theme.text().add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:<14}", task.status), status_style),
            Span::styled(task.name.as_str(), theme.text()),
        ]));
    }

    let paragraph = Paragraph::new(lines)
        .style(theme.text())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, area);
}

// -------------------------------------------------------------------------
// Layout helper
// -------------------------------------------------------------------------

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
