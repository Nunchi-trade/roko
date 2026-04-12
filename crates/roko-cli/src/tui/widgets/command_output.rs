//! Command / gate output widget — shows gate pipeline output with PASS/FAIL
//! badges, per-line coloring, and scrollbar.
//!
//! Ported from Mori's command_output.rs.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::rosedust::MoriTheme;
use super::super::state::TuiState;

// ---------------------------------------------------------------------------
// Public render
// ---------------------------------------------------------------------------

/// Render the gate/command output panel.
pub fn render_command_output(frame: &mut Frame<'_>, area: Rect, state: &TuiState, focused: bool) {
    // Collect all gate output text, most recent first
    let output: String = state
        .gate_results
        .iter()
        .rev()
        .map(|g| {
            let badge = if g.passed {
                "\u{2713} PASS"
            } else {
                "\u{2717} FAIL"
            };
            format!(
                "[{}] {} \u{00b7} {}\n{}\n",
                badge, g.gate, g.plan_id, g.output
            )
        })
        .collect();

    let check_src = &output;
    let passed = check_src.contains("PASS") || check_src.contains("test result: ok");
    let failed = check_src.contains("FAIL")
        || check_src.contains("error[E")
        || check_src.contains("error: could not compile");

    let running_count = state
        .gate_results
        .iter()
        .filter(|g| !g.passed && g.output.is_empty())
        .count();

    let title_text = if running_count > 0 {
        format!(
            "{} Gate Output ({} running)",
            state.atmosphere.spinner(),
            running_count
        )
    } else {
        "Gate Output".to_string()
    };

    let title: Line = if failed {
        Line::from(vec![
            Span::raw(title_text),
            Span::styled(" \u{2717} FAIL", Style::default().fg(MoriTheme::EMBER)),
        ])
    } else if passed {
        Line::from(vec![
            Span::raw(title_text),
            Span::styled(" \u{2713} PASS", Style::default().fg(MoriTheme::SAGE)),
        ])
    } else {
        Line::from(title_text)
    };

    let visible_height = area.height.saturating_sub(2) as usize;
    let all_lines: Vec<&str> = output.lines().collect();
    let total = all_lines.len();

    // Auto-scroll to bottom unless user scrolled
    let start = if state.output_scroll == 0 {
        total.saturating_sub(visible_height)
    } else {
        state
            .output_scroll
            .min(total.saturating_sub(visible_height.min(total)))
    };
    let end = (start + visible_height).min(total);

    let lines: Vec<Line> = if total == 0 {
        vec![Line::from(vec![
            Span::raw(" "),
            Span::styled("No gate output", Style::default().fg(MoriTheme::TEXT_DIM)),
        ])]
    } else {
        all_lines[start..end]
            .iter()
            .map(|&line| {
                let lower = line.to_lowercase();
                let style =
                    if lower.contains("pass") || lower.contains(" ok ") || lower.contains("passed")
                    {
                        Style::default().fg(MoriTheme::SAGE)
                    } else if lower.contains("fail") || lower.contains("error") {
                        Style::default().fg(MoriTheme::EMBER)
                    } else {
                        Style::default().fg(MoriTheme::FG_DIM)
                    };
                Line::from(vec![Span::raw(" "), Span::styled(line, style)])
            })
            .collect()
    };

    let is_running = running_count > 0;
    let (border_s, title_s) = if focused {
        (
            MoriTheme::focused_border_style(),
            MoriTheme::focused_title_style(),
        )
    } else if is_running {
        (
            Style::default().fg(MoriTheme::WARNING),
            Style::default()
                .fg(MoriTheme::WARNING)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            MoriTheme::unfocused_border_style(),
            MoriTheme::unfocused_title_style(),
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(MoriTheme::block_style())
        .border_style(border_s)
        .title_style(title_s);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}
