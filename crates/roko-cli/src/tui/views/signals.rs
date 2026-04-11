//! Gate verdict signal view.
//!
//! Displays a navigable table of gate verdicts with pass/fail indicators.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Modifier;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};

use roko_core::dashboard_snapshot::DashboardSnapshot;

use crate::tui::dashboard::Theme;

/// Render the gate verdict signal view.
pub fn render_signals_view(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    selected: usize,
    scroll: u16,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Gate Signals ")
        .border_style(theme.accent());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snapshot.gates.is_empty() {
        let empty = Paragraph::new("No gate verdicts recorded.")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let header = Row::new(vec![
        Cell::from(" ").style(theme.accent()),
        Cell::from("Plan").style(theme.accent().add_modifier(Modifier::BOLD)),
        Cell::from("Task").style(theme.accent().add_modifier(Modifier::BOLD)),
        Cell::from("Gate").style(theme.accent().add_modifier(Modifier::BOLD)),
        Cell::from("Result").style(theme.accent().add_modifier(Modifier::BOLD)),
    ]);

    // Show most recent first.
    let gates: Vec<_> = snapshot.gates.iter().rev().collect();

    let rows: Vec<Row> = gates
        .iter()
        .enumerate()
        .skip(scroll as usize)
        .map(|(i, g)| {
            let is_selected = i == selected;
            let icon = if g.passed { "+" } else { "x" };
            let result_str = if g.passed { "pass" } else { "FAIL" };
            let result_style = if g.passed {
                theme.success()
            } else {
                theme.danger()
            };

            let row_style = if is_selected {
                theme.selection()
            } else {
                theme.text()
            };

            Row::new(vec![
                Cell::from(icon).style(result_style),
                Cell::from(g.plan_id.as_str()).style(row_style),
                Cell::from(g.task_id.as_str()).style(row_style),
                Cell::from(g.gate.as_str()).style(row_style),
                Cell::from(result_str).style(result_style),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(15),
        ],
    )
    .header(header)
    .column_spacing(1);

    frame.render_widget(table, inner);
}
