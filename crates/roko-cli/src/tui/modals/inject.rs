//! Free-text message injection modal for steering agents.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::super::dashboard::Theme;

/// Render the inject-message modal.
///
/// Shows a text input field with a blinking cursor indicator.
/// `target_agent` is the agent role being steered.
/// `input_text` is the current buffer content.
/// `cursor_pos` is the byte offset of the cursor within `input_text`.
pub fn render_inject(
    frame: &mut Frame<'_>,
    area: Rect,
    target_agent: &str,
    input_text: &str,
    cursor_pos: usize,
    theme: &Theme,
) {
    let popup = centered_rect(70, 20, area);
    frame.render_widget(Clear, popup);

    let title = format!(" Steer: {} ", target_agent);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_alignment(Alignment::Center)
        .border_style(theme.accent());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Split inner area: hint line at top, input area, then keybinding hint at bottom.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    // Context hint
    let context = Line::from(vec![
        Span::styled("Message to ", theme.muted()),
        Span::styled(target_agent, theme.accent_bold()),
        Span::styled(":", theme.muted()),
    ]);
    frame.render_widget(Paragraph::new(context), chunks[0]);

    // Input field: show text with a cursor character.
    let (before, after) = if cursor_pos <= input_text.len() {
        (&input_text[..cursor_pos], &input_text[cursor_pos..])
    } else {
        (input_text, "")
    };
    let cursor_char = if after.is_empty() { " " } else { &after[..1] };
    let after_cursor = if after.len() > 1 { &after[1..] } else { "" };

    let input_line = Line::from(vec![
        Span::styled(before, theme.text()),
        Span::styled(cursor_char, theme.selection()),
        Span::styled(after_cursor, theme.text()),
    ]);
    frame.render_widget(Paragraph::new(input_line), chunks[1]);

    // Keybinding hints
    let hints = Line::from(vec![
        Span::styled("[Enter]", theme.accent_bold()),
        Span::styled(" send   ", theme.muted()),
        Span::styled("[Esc]", theme.accent_bold()),
        Span::styled(" cancel", theme.muted()),
    ]);
    frame.render_widget(Paragraph::new(hints).alignment(Alignment::Right), chunks[2]);
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
