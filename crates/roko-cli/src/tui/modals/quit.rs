//! Quit confirmation modal.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::super::dashboard::Theme;

/// Render the quit confirmation dialog.
///
/// Small centered rectangle asking "Are you sure you want to quit?"
/// with `[y]` yes / `[n]` no keybindings.
pub fn render_quit(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let popup = centered_rect_fixed(42, 8, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Quit ")
        .title_alignment(Alignment::Center)
        .border_style(theme.warning());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("Are you sure you want to quit?", theme.text())),
        Line::from(""),
        Line::from(vec![
            Span::styled("[y]", theme.accent_bold()),
            Span::styled(" yes   ", theme.text()),
            Span::styled("[n]", theme.accent_bold()),
            Span::styled(" no", theme.text()),
        ]),
    ];

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(paragraph, inner);
}

fn centered_rect_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
