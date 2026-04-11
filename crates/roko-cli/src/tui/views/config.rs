//! Config display view (read-only).
//!
//! Shows the current snapshot stats as a summary panel.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use roko_core::dashboard_snapshot::DashboardSnapshot;

use crate::tui::dashboard::Theme;

/// Render the config / stats view.
pub fn render_config_view(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Stats & Config ")
        .border_style(theme.accent());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(0)])
        .split(inner);

    let stats = &snapshot.stats;

    let lines = vec![
        Line::from(Span::styled(
            "Snapshot Statistics",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        stat_line("Plans active", stats.plans_active, theme),
        stat_line("Plans completed", stats.plans_completed, theme),
        stat_line("Plans failed", stats.plans_failed, theme),
        Line::from(""),
        stat_line("Tasks active", stats.tasks_active, theme),
        stat_line("Tasks completed", stats.tasks_completed, theme),
        stat_line("Tasks failed", stats.tasks_failed, theme),
    ];

    let left = Paragraph::new(lines)
        .style(theme.text())
        .wrap(Wrap { trim: false });
    frame.render_widget(left, chunks[0]);

    let lines2 = vec![
        stat_line("Agents active", stats.agents_active, theme),
        Line::from(""),
        stat_line("Gates passed", stats.gates_passed, theme),
        stat_line("Gates failed", stats.gates_failed, theme),
        Line::from(""),
        stat_line("Total errors", stats.errors_total, theme),
    ];

    let right = Paragraph::new(lines2)
        .style(theme.text())
        .wrap(Wrap { trim: false });
    frame.render_widget(right, chunks[1]);
}

fn stat_line<'a>(label: &'a str, value: usize, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {label}: "), theme.muted()),
        Span::styled(value.to_string(), theme.text().add_modifier(Modifier::BOLD)),
    ])
}
