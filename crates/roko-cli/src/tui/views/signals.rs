//! Gate verdict signal view.
//!
//! Displays a navigable table of gate verdicts with pass/fail indicators.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};

use super::super::mori_theme::MoriTheme;
use super::super::tui_state::TuiState;

/// Render the gate verdict signal view.
pub fn render_signals_view(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Gate Signals ",
            MoriTheme::focused_title_style(),
        ))
        .border_style(MoriTheme::focused_border_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let live_gates = &state.live.gates;
    let persisted = &state.gate_results;

    if live_gates.is_empty() && persisted.is_empty() {
        let empty = Paragraph::new("No gate verdicts recorded.")
            .style(Style::default().fg(MoriTheme::FG_DIM))
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let header_style = Style::default()
        .fg(MoriTheme::ROSE_BRIGHT)
        .add_modifier(Modifier::BOLD);

    let header = Row::new(vec![
        Cell::from(" ").style(header_style),
        Cell::from("Plan").style(header_style),
        Cell::from("Task").style(header_style),
        Cell::from("Gate").style(header_style),
        Cell::from("Result").style(header_style),
    ]);

    // Use output_scroll as the scroll offset; selected_agent as row selection index.
    let scroll = state.output_scroll;
    let selected = state.selected_agent;

    let rows: Vec<Row> = if !live_gates.is_empty() {
        // Live gates: most recent first
        live_gates
            .iter()
            .rev()
            .enumerate()
            .skip(scroll)
            .map(|(i, g)| gate_row(g.passed, &g.plan_id, &g.task_id, &g.gate, i == selected))
            .collect()
    } else {
        // Persisted gate results when no live data
        persisted
            .iter()
            .rev()
            .enumerate()
            .skip(scroll)
            .map(|(i, g)| gate_row(g.passed, &g.plan_id, &g.task_id, &g.gate, i == selected))
            .collect()
    };

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

fn gate_row<'a>(
    passed: bool,
    plan_id: &'a str,
    task_id: &'a str,
    gate: &'a str,
    is_selected: bool,
) -> Row<'a> {
    let (icon, result_str) = if passed {
        ("✓", "pass")
    } else {
        ("✗", "FAIL")
    };

    let result_style = if passed {
        Style::default().fg(MoriTheme::SAGE)
    } else {
        Style::default().fg(MoriTheme::EMBER)
    };

    let row_style = if is_selected {
        MoriTheme::selected_style()
    } else {
        Style::default().fg(MoriTheme::FG)
    };

    Row::new(vec![
        Cell::from(icon).style(result_style),
        Cell::from(plan_id).style(row_style),
        Cell::from(task_id).style(row_style),
        Cell::from(gate).style(row_style),
        Cell::from(result_str).style(result_style),
    ])
}
