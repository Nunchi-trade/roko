//! Agent command approval modal.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::super::dashboard::Theme;

/// Render the approval modal for an agent command.
///
/// Shows the agent role, the command text (word-wrapped), and keybinding hints.
/// Centered ~60x20 rectangle.
pub fn render_approval(
    frame: &mut Frame<'_>,
    area: Rect,
    role: &str,
    command: &str,
    theme: &Theme,
) {
    let popup = centered_rect(60, 40, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Approval Required ")
        .title_alignment(Alignment::Center)
        .border_style(theme.danger());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Agent: ", theme.muted()),
            Span::styled(role, theme.accent_bold()),
        ]),
        Line::from(""),
        Line::from(Span::styled("Command:", theme.muted())),
    ];

    // Wrap the command text manually into lines for display.
    for line in command.lines() {
        lines.push(Line::from(Span::styled(
            format!("  {line}"),
            theme.warning(),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("[y]", theme.success()),
        Span::styled(" approve   ", theme.text()),
        Span::styled("[n]", theme.danger()),
        Span::styled(" reject", theme.text()),
    ]));

    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
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
