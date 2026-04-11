//! Two-column keybinding reference modal.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::tui::dashboard::Theme;

/// Render the help modal as a centered overlay.
///
/// Caller should pass an area produced by `centered_rect(80, 70, frame.area())`.
pub fn render_help_modal(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Keybindings ")
        .border_style(theme.accent());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // Left column: general keys.
    let left_lines = vec![
        Line::from(Span::styled(
            "General",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        key_line("q / Esc", "Quit", theme),
        key_line("Tab", "Next page", theme),
        key_line("Shift+Tab", "Previous page", theme),
        key_line("1-6", "Jump to page 1-6", theme),
        key_line("j / Down", "Scroll down", theme),
        key_line("k / Up", "Scroll up", theme),
        key_line("PgDn", "Page down", theme),
        key_line("PgUp", "Page up", theme),
        key_line("Home", "Scroll to top", theme),
        key_line("r", "Refresh data", theme),
        key_line("?", "Toggle this help", theme),
    ];

    let left = Paragraph::new(left_lines)
        .style(theme.text())
        .wrap(Wrap { trim: false });
    frame.render_widget(left, cols[0]);

    // Right column: page-specific keys.
    let right_lines = vec![
        Line::from(Span::styled(
            "Page-Specific",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        key_line("Enter", "Expand selected item", theme),
        key_line("h / Left", "Previous page", theme),
        key_line("l / Right", "Next page", theme),
        Line::from(""),
        Line::from(Span::styled(
            "Signals / Gate Results",
            theme.warning().add_modifier(Modifier::BOLD),
        )),
        key_line("j / k", "Select row", theme),
        key_line("Enter", "Show detail overlay", theme),
        Line::from(""),
        Line::from(Span::styled(
            "Overlays",
            theme.warning().add_modifier(Modifier::BOLD),
        )),
        key_line("Esc", "Close overlay / quit", theme),
        key_line("j / k", "Scroll overlay content", theme),
    ];

    let right = Paragraph::new(right_lines)
        .style(theme.text())
        .wrap(Wrap { trim: false });
    frame.render_widget(right, cols[1]);
}

fn key_line<'a>(key: &'a str, desc: &'a str, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {key:<14}"), theme.warning()),
        Span::styled(desc, theme.text()),
    ])
}
