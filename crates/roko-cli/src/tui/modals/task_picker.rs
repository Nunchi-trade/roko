//! Scrollable, searchable task picker modal.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::super::dashboard::Theme;

/// A row in the task picker.
#[derive(Debug, Clone)]
pub struct TaskPickerRow {
    pub plan_num: u32,
    pub task_id: String,
    pub title: String,
    pub status: String,
}

/// Render the task picker modal.
///
/// Flat list of tasks with plan_num, task_id, title, status columns.
/// Cursor selection with j/k navigation, Enter to select.
/// Centered ~80x60 rectangle.
pub fn render_task_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    tasks: &[TaskPickerRow],
    selected_index: usize,
    scroll_offset: u16,
    theme: &Theme,
) {
    let popup = centered_rect(80, 60, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Task Picker ")
        .title_alignment(Alignment::Center)
        .border_style(theme.accent());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Split: header, list, keybinding hints.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header + separator
            Constraint::Min(1),    // task list
            Constraint::Length(1), // hints
        ])
        .split(inner);

    // Header
    let header = Line::from(Span::styled(
        format!(" {:<6} {:<12} {:<10} {}", "PLAN", "TASK", "STATUS", "TITLE"),
        theme.accent_bold(),
    ));
    let separator = Line::from(Span::styled(
        " ".to_string() + &"-".repeat(chunks[0].width.saturating_sub(2) as usize),
        theme.muted(),
    ));
    frame.render_widget(Paragraph::new(vec![header, separator]), chunks[0]);

    // Task list
    let mut lines: Vec<Line<'_>> = Vec::new();

    if tasks.is_empty() {
        lines.push(Line::from(Span::styled(" No tasks found.", theme.muted())));
    } else {
        for (i, task) in tasks.iter().enumerate() {
            let is_selected = i == selected_index;
            let row_style = if is_selected {
                theme.selection()
            } else {
                theme.text()
            };

            let status_style = if is_selected {
                theme.selection()
            } else {
                match task.status.as_str() {
                    "done" | "completed" => theme.success(),
                    "running" | "active" => theme.info(),
                    "failed" | "error" => theme.danger(),
                    "blocked" => theme.warning(),
                    _ => theme.muted(),
                }
            };

            let prefix = if is_selected { "> " } else { "  " };

            let title_max = chunks[1].width.saturating_sub(34).max(4) as usize;
            let title = if task.title.len() > title_max {
                format!("{}...", &task.title[..title_max.saturating_sub(3)])
            } else {
                task.title.clone()
            };

            lines.push(Line::from(vec![
                Span::styled(prefix, row_style),
                Span::styled(format!("{:<6}", task.plan_num), row_style),
                Span::styled(format!("{:<12}", task.task_id), row_style),
                Span::styled(format!("{:<10}", task.status), status_style),
                Span::styled(title, row_style),
            ]));
        }
    }

    let list_para = Paragraph::new(lines).scroll((scroll_offset, 0));
    frame.render_widget(list_para, chunks[1]);

    // Hints
    let hints = Line::from(vec![
        Span::styled("[j/k]", theme.accent_bold()),
        Span::styled(" navigate  ", theme.muted()),
        Span::styled("[Enter]", theme.accent_bold()),
        Span::styled(" select  ", theme.muted()),
        Span::styled("[Esc]", theme.accent_bold()),
        Span::styled(" close", theme.muted()),
    ]);
    frame.render_widget(Paragraph::new(hints), chunks[2]);
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
