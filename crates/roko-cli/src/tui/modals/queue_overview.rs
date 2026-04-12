//! Milestone progress browser modal with two-column layout.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::super::dashboard::Theme;

/// A task within a milestone queue.
#[derive(Debug, Clone)]
pub struct QueueTask {
    pub id: String,
    pub title: String,
    pub status: String,
}

/// A milestone in the execution queue.
#[derive(Debug, Clone)]
pub struct Milestone {
    pub name: String,
    pub tasks: Vec<QueueTask>,
    pub completed: usize,
    pub total: usize,
}

/// Render the queue overview modal.
///
/// Two-column layout: left column shows milestone list with cursor,
/// right column shows task details for the selected milestone.
/// Centered ~85x75 rectangle.
pub fn render_queue_overview(
    frame: &mut Frame<'_>,
    area: Rect,
    milestones: &[Milestone],
    selected_index: usize,
    scroll_offset: u16,
    theme: &Theme,
) {
    let popup = centered_rect(85, 75, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Milestone Queue ")
        .title_alignment(Alignment::Center)
        .border_style(theme.accent());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if milestones.is_empty() {
        let empty = Paragraph::new(Span::styled("No milestones.", theme.muted()));
        frame.render_widget(empty, inner);
        return;
    }

    // Split into left (milestone list) and right (detail) columns.
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(inner);

    // -- Left column: milestone list --
    let left_block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(theme.muted());
    let left_inner = left_block.inner(columns[0]);
    frame.render_widget(left_block, columns[0]);

    let mut left_lines: Vec<Line<'_>> = Vec::new();
    for (i, ms) in milestones.iter().enumerate() {
        let progress = if ms.total > 0 {
            format!("{}/{}", ms.completed, ms.total)
        } else {
            "0/0".into()
        };

        let (prefix, style) = if i == selected_index {
            ("> ", theme.selection())
        } else {
            ("  ", theme.text())
        };

        left_lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(format!("{:<16}", ms.name), style),
            Span::styled(progress, theme.muted()),
        ]));
    }

    let left_para = Paragraph::new(left_lines).scroll((scroll_offset, 0));
    frame.render_widget(left_para, left_inner);

    // -- Right column: selected milestone detail --
    let sel = selected_index.min(milestones.len().saturating_sub(1));
    let milestone = &milestones[sel];

    let mut right_lines: Vec<Line<'_>> = Vec::new();
    right_lines.push(Line::from(Span::styled(
        format!(" {}", milestone.name),
        theme.accent_bold(),
    )));
    right_lines.push(Line::from(""));

    for task in &milestone.tasks {
        let status_style = match task.status.as_str() {
            "done" | "completed" => theme.success(),
            "running" | "active" => theme.info(),
            "failed" | "error" => theme.danger(),
            "blocked" => theme.warning(),
            _ => theme.muted(),
        };

        right_lines.push(Line::from(vec![
            Span::styled(format!(" {:<8}", task.id), theme.muted()),
            Span::styled(format!("{:<10}", task.status), status_style),
            Span::styled(&task.title, theme.text()),
        ]));
    }

    if milestone.tasks.is_empty() {
        right_lines.push(Line::from(Span::styled(
            " No tasks in this milestone.",
            theme.muted(),
        )));
    }

    right_lines.push(Line::from(""));
    right_lines.push(Line::from(vec![
        Span::styled(" [Esc]", theme.accent_bold()),
        Span::styled(" close  ", theme.muted()),
        Span::styled("[Up/Down]", theme.accent_bold()),
        Span::styled(" navigate", theme.muted()),
    ]));

    let right_para = Paragraph::new(right_lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));
    frame.render_widget(right_para, columns[1]);
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
