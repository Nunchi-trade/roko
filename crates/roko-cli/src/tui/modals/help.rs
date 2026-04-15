//! Two-column keybinding reference modal.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::tui::mori_theme::MoriTheme;

/// Render the help modal as a centered overlay.
///
/// Caller should pass an area produced by `centered_rect(80, 70, frame.area())`.
pub fn render_help_modal(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Keybindings ")
        .border_style(Style::default().fg(MoriTheme::ROSE))
        .style(Style::default().bg(MoriTheme::VOID));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // Left column: global keys.
    let left_lines = vec![
        Line::from(Span::styled(
            "Global",
            Style::default()
                .fg(MoriTheme::BONE_DIM)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        key_line("q / Esc", "Quit"),
        key_line("F1-F6", "Jump to page 1-6"),
        key_line("Tab", "Next page"),
        key_line("Shift+Tab", "Previous page"),
        key_line("j / k", "Scroll down / up"),
        key_line("PgDn / PgUp", "Page down / up"),
        key_line("Home", "Scroll to top"),
        key_line("r", "Refresh data"),
        key_line("?", "Toggle this help"),
    ];

    let left = Paragraph::new(left_lines)
        .style(Style::default().fg(MoriTheme::TEXT))
        .wrap(Wrap { trim: false });
    frame.render_widget(left, cols[0]);

    // Right column: dashboard-specific keys.
    let right_lines = vec![
        Line::from(Span::styled(
            "Dashboard",
            Style::default()
                .fg(MoriTheme::BONE_DIM)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        key_line("a", "Agents sub-tab"),
        key_line("o", "Output sub-tab"),
        key_line("d", "Diff sub-tab"),
        key_line("e", "Errors sub-tab"),
        key_line("g", "Git sub-tab"),
        Line::from(""),
        key_line("Tab", "Cycle focus between panels"),
        key_line("Enter", "Expand selected item"),
    ];

    let right = Paragraph::new(right_lines)
        .style(Style::default().fg(MoriTheme::TEXT))
        .wrap(Wrap { trim: false });
    frame.render_widget(right, cols[1]);
}

fn key_line<'a>(key: &'a str, desc: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("  {key:<16}"),
            Style::default().fg(MoriTheme::WARNING),
        ),
        Span::styled(desc, Style::default().fg(MoriTheme::TEXT)),
    ])
}
