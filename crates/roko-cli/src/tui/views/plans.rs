//! Full plan browser view (F2).
//!
//! Shows all plans with wave grouping, task progress, gate pass/fail icons,
//! and selection highlighting. Uses MoriTheme colors throughout.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::super::mori_theme::MoriTheme;
use super::super::tui_state::{FocusZone, PlanEntry, TuiState};

/// Render the full plan browser view with wave grouping and gate results.
pub fn render_plans_view(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let is_focused = matches!(state.focus, FocusZone::PlanTree);

    let border_style = if is_focused {
        MoriTheme::focused_border_style()
    } else {
        MoriTheme::unfocused_border_style()
    };
    let title_style = if is_focused {
        MoriTheme::focused_title_style()
    } else {
        MoriTheme::unfocused_title_style()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Plan Browser ", title_style))
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.plans.is_empty() {
        let empty = Paragraph::new("No plans discovered.")
            .style(MoriTheme::dim_style())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    if !state.execution_waves.is_empty() {
        // Wave-grouped rendering
        for wave in &state.execution_waves {
            let wave_pct = if wave.total > 0 {
                wave.done as f64 / wave.total as f64
            } else {
                0.0
            };
            let wave_color = MoriTheme::semantic_color(wave_pct);
            let wave_icon = if wave.done == wave.total && wave.total > 0 {
                "✓"
            } else {
                "~"
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {wave_icon} wave {} ", wave.index + 1),
                    Style::default().fg(wave_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}/{}", wave.done, wave.total),
                    MoriTheme::dim_style(),
                ),
            ]));

            if wave.expanded {
                for plan_id in &wave.plans {
                    if let Some((idx, plan)) = state
                        .plans
                        .iter()
                        .enumerate()
                        .find(|(_, p)| &p.id == plan_id)
                    {
                        render_plan_lines(&mut lines, plan, idx, state);
                    }
                }
            }

            lines.push(Line::from(""));
        }
    } else {
        // Flat list
        for (idx, plan) in state.plans.iter().enumerate() {
            render_plan_lines(&mut lines, plan, idx, state);
            lines.push(Line::from(""));
        }
    }

    let scroll = state.plan_scroll as u16;
    let paragraph = Paragraph::new(lines)
        .style(MoriTheme::default_style())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, inner);
}

// ── Per-plan line builder ─────────────────────────────────────────────────

fn render_plan_lines(
    lines: &mut Vec<Line>,
    plan: &PlanEntry,
    idx: usize,
    state: &TuiState,
) {
    let is_selected = idx == state.selected_plan;
    let bg = if is_selected {
        MoriTheme::BG_HIGHLIGHT
    } else {
        MoriTheme::BG
    };

    let active_marker = if plan.active { "  ACTIVE" } else { "" };
    let phase_color = MoriTheme::phase_accent(&plan.phase);

    let header_style = if is_selected {
        Style::default()
            .fg(MoriTheme::BONE)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(MoriTheme::ROSE)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    };

    lines.push(Line::from(vec![
        Span::styled(format!("  {} ", plan.name), header_style),
        Span::styled(
            format!("[{}]{active_marker}", plan.phase),
            Style::default().fg(phase_color).bg(bg),
        ),
    ]));

    // Progress bar
    let done = plan.tasks_done;
    let failed = plan.tasks_failed;
    let total = plan.tasks_total;
    let pending = total.saturating_sub(done).saturating_sub(failed);
    let pct = if total > 0 {
        done as f64 / total as f64
    } else {
        0.0
    };
    let bar = progress_bar(pct, 20);
    let bar_color = MoriTheme::semantic_color(pct);

    let right_span = if failed > 0 {
        Span::styled(
            format!("  {failed} failed"),
            Style::default().fg(MoriTheme::STATUS_ERROR).bg(bg),
        )
    } else {
        Span::styled(
            format!("  {pending} pending"),
            Style::default().fg(MoriTheme::FG_DIM).bg(bg),
        )
    };

    lines.push(Line::from(vec![
        Span::styled("    ", Style::default().bg(bg)),
        Span::styled(bar, Style::default().fg(bar_color).bg(bg)),
        Span::styled(
            format!("  {done}/{total}"),
            Style::default().fg(MoriTheme::FG_BRIGHT).bg(bg),
        ),
        right_span,
    ]));

    // Gate results
    let gates: Vec<_> = state
        .gate_results
        .iter()
        .filter(|g| g.plan_id == plan.id)
        .collect();

    if !gates.is_empty() {
        lines.push(Line::from(Span::styled(
            "    gates:",
            MoriTheme::dim_style(),
        )));
        for g in &gates {
            let (icon, style) = if g.passed {
                ("✓", Style::default().fg(MoriTheme::STATUS_OK).bg(bg))
            } else {
                ("✗", Style::default().fg(MoriTheme::STATUS_ERROR).bg(bg))
            };
            lines.push(Line::from(vec![
                Span::styled("      ", Style::default().bg(bg)),
                Span::styled(format!("[{icon}] "), style),
                Span::styled(
                    format!("{}: {}", g.task_id, g.gate),
                    Style::default().fg(MoriTheme::FG_DIM).bg(bg),
                ),
            ]));
        }
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────

fn progress_bar(pct: f64, width: usize) -> String {
    let filled = ((pct * width as f64).round() as usize).min(width);
    let empty = width - filled;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}
