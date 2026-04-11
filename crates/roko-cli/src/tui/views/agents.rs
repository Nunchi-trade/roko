//! Agent roster and activity view.
//!
//! Displays a table of all agents with role, active status, and output size.

use ratatui::layout::{Constraint, Rect};
use ratatui::style::Modifier;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use roko_core::dashboard_snapshot::DashboardSnapshot;

use crate::tui::dashboard::Theme;

/// Render the agent roster and activity view.
pub fn render_agents_view(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Agent Activity ")
        .border_style(theme.accent());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut agents: Vec<_> = snapshot.agents.values().collect();
    agents.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));

    if agents.is_empty() {
        let empty = Paragraph::new("No agents spawned.")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let header = Row::new(vec![
        Cell::from("Agent ID").style(theme.accent().add_modifier(Modifier::BOLD)),
        Cell::from("Role").style(theme.accent().add_modifier(Modifier::BOLD)),
        Cell::from("Active").style(theme.accent().add_modifier(Modifier::BOLD)),
        Cell::from("Output").style(theme.accent().add_modifier(Modifier::BOLD)),
    ]);

    let rows: Vec<Row> = agents
        .iter()
        .map(|agent| {
            let active_style = if agent.active {
                theme.success()
            } else {
                theme.muted()
            };
            let active_str = if agent.active { "yes" } else { "no" };
            let output = format_bytes(agent.output_bytes);

            Row::new(vec![
                Cell::from(agent.agent_id.as_str()).style(theme.text()),
                Cell::from(agent.role.as_str()).style(theme.warning()),
                Cell::from(active_str).style(active_style),
                Cell::from(output).style(theme.text()),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(25),
            Constraint::Percentage(15),
            Constraint::Percentage(30),
        ],
    )
    .header(header)
    .column_spacing(1);

    frame.render_widget(table, inner);
}

/// Format a byte count into a human-readable string.
fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
