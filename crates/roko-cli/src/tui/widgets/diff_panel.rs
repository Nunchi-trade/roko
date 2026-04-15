//! Diff viewer widget with +/- syntax coloring.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::super::dashboard::Theme;

/// Render a diff panel with syntax coloring for unified diff format.
///
/// Lines starting with `+` are green, `-` are red, `@@` are cyan,
/// and context lines are default foreground.
pub fn render_diff_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    diff_text: &str,
    scroll: Option<usize>,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("diff")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if diff_text.is_empty() {
        let empty = Paragraph::new("no diff")
            .style(theme.muted())
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let lines: Vec<Line<'_>> = diff_text
        .lines()
        .map(|line| {
            let style = diff_line_style(line, theme);
            Line::from(Span::styled(line.to_string(), style))
        })
        .collect();

    let total_lines = lines.len();
    let visible = inner.height as usize;

    // Auto-scroll to end if no explicit scroll position, otherwise use pinned position.
    let scroll_offset = match scroll {
        Some(pos) => pos.min(total_lines.saturating_sub(visible)),
        None => total_lines.saturating_sub(visible),
    };

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset as u16, 0));
    frame.render_widget(paragraph, inner);
}

fn diff_line_style(line: &str, theme: &Theme) -> Style {
    if line.starts_with("diff --git") || line.starts_with("index ") {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("@@") {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("+++ ") {
        Style::default()
            .fg(theme.success)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("--- ") {
        Style::default()
            .fg(theme.danger)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with('+') {
        Style::default().fg(theme.success)
    } else if line.starts_with('-') {
        Style::default().fg(theme.danger)
    } else {
        Style::default().fg(theme.foreground)
    }
}
