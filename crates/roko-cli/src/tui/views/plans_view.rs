//! F2 Plans view -- Mori-style wave browser + plan detail.
//!
//! Layout: left 35% (wave list with pipeline header + collapsible plan
//! groups), right 65% (selected plan detail with tasks, gate results,
//! timing).
//!
//! Renders hierarchical wave groups with gradient progress bars, status
//! icons, phase indicators, and timing matching the Mori Plans screen (F2).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::input::FocusZone;
use crate::tui::state::{PlanEntry, TaskEntry, TaskStatus, TuiState};
use crate::tui::util::truncate_middle;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Fractional block characters for smooth progress bars.
const BLOCKS: &[char] = &[
    ' ', '\u{2591}', '\u{258F}', '\u{258E}', '\u{258D}', '\u{258C}', '\u{258B}', '\u{258A}',
    '\u{2589}', '\u{2588}',
];

// ---------------------------------------------------------------------------
// Public render
// ---------------------------------------------------------------------------

/// Render the full plans view.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let panels =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(area);

    render_left_panel(frame, panels[0], _data, tui_state, view_state, theme);
    render_right_panel(frame, panels[1], _data, tui_state, view_state, theme);
}

// ---------------------------------------------------------------------------
// Left panel: pipeline header + wave/plan tree
// ---------------------------------------------------------------------------

fn render_left_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections = Layout::vertical([
        Constraint::Length(3), // Pipeline header
        Constraint::Length(4), // Selected plan summary
        Constraint::Min(0),    // Wave/plan tree
    ])
    .split(area);

    let focused = matches!(
        tui_state.focus,
        FocusZone::PlanTree | FocusZone::TaskProgress
    );

    render_pipeline_header(frame, sections[0], _data, tui_state, focused, theme);
    if let Some(plan) = tui_state.plans.get(tui_state.selected_plan_idx) {
        let plan_summary = tui_state
            .plan_summaries
            .iter()
            .find(|summary| summary.id == plan.id)
            .or_else(|| tui_state.plan_summaries.get(tui_state.selected_plan_idx));
        let plan_execution = tui_state
            .current_plan_execution
            .as_ref()
            .filter(|exec| exec.plan_id == plan.id);
        render_plan_summary(
            frame,
            sections[1],
            plan,
            plan_summary,
            plan_execution,
            tui_state,
            view_state,
            theme,
        );
    }
    render_wave_tree(frame, sections[2], _data, tui_state, view_state, theme);
}

// ---------------------------------------------------------------------------
// Pipeline header (3 lines with overall progress)
// ---------------------------------------------------------------------------

fn render_pipeline_header(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    focused: bool,
    theme: &Theme,
) {
    let total_plans = tui_state.plan_summaries.len();
    let completed = tui_state
        .plan_summaries
        .iter()
        .filter(|p| p.completed)
        .count();
    let active_count = tui_state
        .plans
        .iter()
        .filter(|p| p.status.is_active())
        .count();
    let pct = if total_plans > 0 {
        completed as f64 / total_plans as f64
    } else {
        0.0
    };

    let bar_w = (area.width.saturating_sub(24)) as usize;
    let bar = build_progress_bar(pct, bar_w);
    let bar_color = progress_color(pct, total_plans, completed, theme);

    let header_line = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!("{completed}/{total_plans}"),
            Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(bar, Style::default().fg(bar_color)),
        Span::styled(
            format!(" {:.0}%", pct * 100.0),
            Style::default().fg(bar_color),
        ),
    ]);

    let border_style = if focused {
        Theme::focused_border_style()
    } else if completed == total_plans && total_plans > 0 {
        theme.success()
    } else if active_count > 0 {
        theme.accent()
    } else {
        theme.muted()
    };
    let title_style = if focused {
        Theme::focused_title_style()
    } else if completed == total_plans && total_plans > 0 {
        theme.success().add_modifier(Modifier::BOLD)
    } else if active_count > 0 {
        theme.accent().add_modifier(Modifier::BOLD)
    } else {
        theme.muted()
    };

    let header = Paragraph::new(header_line).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Pipeline ", title_style))
            .border_style(border_style),
    );
    frame.render_widget(header, area);
}

// ---------------------------------------------------------------------------
// Wave tree: hierarchical wave -> plan list
// ---------------------------------------------------------------------------

fn render_wave_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let focused = matches!(tui_state.focus, FocusZone::PlanTree);
    let total_plans = tui_state.plan_summaries.len();
    let completed = tui_state
        .plan_summaries
        .iter()
        .filter(|p| p.completed)
        .count();
    let failed = tui_state
        .plans
        .iter()
        .filter(|p| p.status.is_failed())
        .count();

    let mut health_suffix = String::new();
    let active = tui_state
        .plans
        .iter()
        .filter(|p| p.status.is_active())
        .count();
    if active > 0 {
        health_suffix.push_str(&format!(" {active}\u{25b8}"));
    }
    if failed > 0 {
        health_suffix.push_str(&format!(" {failed}\u{2717}"));
    }

    let title = format!(" Plans ({completed}/{total_plans}{health_suffix}) ");

    let border_style = if focused {
        Theme::focused_border_style()
    } else {
        theme.muted()
    };
    let title_style = if focused || active > 0 {
        if focused {
            Theme::focused_title_style()
        } else {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        }
    } else {
        theme.muted()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, title_style))
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if tui_state.plan_summaries.is_empty() {
        let v_pad = inner.height / 2;
        let mut empty_lines: Vec<Line<'_>> = Vec::new();
        for _ in 0..v_pad.saturating_sub(1) {
            empty_lines.push(Line::from(""));
        }
        empty_lines.push(Line::from(Span::styled(
            "no plans found",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        )));
        empty_lines.push(Line::from(""));
        empty_lines.push(Line::from(Span::styled(
            "run `roko plan run <dir>` to begin",
            Style::default().fg(theme.muted),
        )));
        let empty = Paragraph::new(empty_lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let content_width = inner.width as usize;
    let mut lines: Vec<Line<'_>> = Vec::new();

    // Column header
    if inner.height > 4 && content_width >= 30 {
        let name_w = content_width.saturating_sub(20);
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<name_w$}", "plan"),
                Style::default().fg(Color::Rgb(60, 50, 60)),
            ),
            Span::styled(
                format!("{:>6}", "prog"),
                Style::default().fg(Color::Rgb(60, 50, 60)),
            ),
            Span::styled(
                format!("{:>8}", "bar"),
                Style::default().fg(Color::Rgb(60, 50, 60)),
            ),
        ]));
    }

    // Build wave groups from the ordered plan list
    // Each group of plans gets a synthetic wave header if we have > 3 plans
    let use_waves = tui_state.plan_summaries.len() > 3;

    if use_waves {
        // Group into waves of ~3-5 plans each
        let wave_size = 4usize;
        let num_waves = (tui_state.plan_summaries.len() + wave_size - 1) / wave_size;

        for wave_idx in 0..num_waves {
            let start = wave_idx * wave_size;
            let end = (start + wave_size).min(tui_state.plan_summaries.len());
            let wave_plans = &tui_state.plan_summaries[start..end];

            let wave_done = wave_plans.iter().filter(|p| p.completed).count();
            let wave_total = wave_plans.len();
            let all_done = wave_done == wave_total;
            let any_active = (start..end).any(|i| {
                tui_state
                    .plans
                    .get(i)
                    .map(|p| p.status.is_active())
                    .unwrap_or(false)
            });

            // Is this wave selected (contains selected plan)?
            let wave_selected = view_state.selected >= start && view_state.selected < end;
            // Default: expand selected wave and completed waves, collapse others
            let expanded = wave_selected || all_done || any_active;

            // Wave header
            let (wave_icon, wave_style) = if all_done {
                (
                    "\u{2713}", // checkmark
                    Style::default().fg(theme.success),
                )
            } else if any_active {
                (
                    "\u{25b6}", // ▶
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    "\u{00b7}", // ·
                    Style::default().fg(theme.muted),
                )
            };

            let collapse_icon = if expanded {
                "\u{25be}" // ▾
            } else {
                "\u{25b8}" // ▸
            };

            // Wave progress bar (8-char)
            let wave_fill = wave_done as f64 / wave_total.max(1) as f64;
            let wave_bar = build_mini_bar(8, wave_fill, all_done, any_active, theme);

            // Count failed in wave
            let wave_failed = (start..end)
                .filter(|&i| {
                    tui_state
                        .plans
                        .get(i)
                        .map(|p| p.status.is_failed())
                        .unwrap_or(false)
                })
                .count();

            let mut wave_spans = vec![
                Span::styled(
                    format!(" {collapse_icon} "),
                    Style::default().fg(theme.muted),
                ),
                Span::styled(format!("{wave_icon} "), wave_style),
                Span::styled(
                    format!("Wave {} ", wave_idx),
                    Style::default()
                        .fg(theme.foreground)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("({wave_done}/{wave_total}) "),
                    Style::default().fg(theme.muted),
                ),
                Span::styled(
                    wave_bar,
                    Style::default().fg(if all_done {
                        theme.success
                    } else if any_active {
                        theme.accent
                    } else {
                        theme.muted
                    }),
                ),
            ];

            if wave_failed > 0 {
                wave_spans.push(Span::styled(
                    format!(" \u{2717}{wave_failed}"),
                    Style::default().fg(theme.danger),
                ));
            }

            // Fill remaining width with horizontal line
            let used: usize = wave_spans.iter().map(|s| s.content.chars().count()).sum();
            let avail = content_width;
            if avail > used + 1 {
                wave_spans.push(Span::styled(
                    format!(" {}", "\u{2500}".repeat(avail - used - 1)),
                    Style::default().fg(Color::Rgb(40, 35, 42)),
                ));
            }
            lines.push(Line::from(wave_spans));

            if !expanded {
                continue;
            }

            // Plans within wave
            for i in start..end {
                if let Some(plan) = tui_state.plan_summaries.get(i) {
                    render_plan_line(
                        &mut lines,
                        plan,
                        i,
                        view_state,
                        tui_state,
                        theme,
                        content_width,
                        true,
                    );
                }
            }
        }
    } else {
        // Flat list
        for (i, plan) in tui_state.plan_summaries.iter().enumerate() {
            render_plan_line(
                &mut lines,
                plan,
                i,
                view_state,
                tui_state,
                theme,
                content_width,
                false,
            );
        }
    }

    // Scroll
    let visible_height = inner.height as usize;
    let total_lines = lines.len();
    let scroll_offset =
        (view_state.scroll as usize).min(total_lines.saturating_sub(visible_height));
    let visible: Vec<Line<'_>> = lines
        .into_iter()
        .skip(scroll_offset)
        .take(visible_height)
        .collect();

    let paragraph = Paragraph::new(visible);
    frame.render_widget(paragraph, inner);

    // Scrollbar
    if total_lines > visible_height {
        render_scrollbar(
            frame,
            inner,
            total_lines,
            visible_height,
            scroll_offset,
            theme,
        );
    }
}

// ---------------------------------------------------------------------------
// Single plan line
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_plan_line(
    lines: &mut Vec<Line<'_>>,
    plan: &crate::plan::PlanSummary,
    idx: usize,
    view_state: &ViewState,
    tui_state: &TuiState,
    theme: &Theme,
    content_width: usize,
    indented: bool,
) {
    let is_selected = idx == view_state.selected;
    let tui_plan = tui_state.plans.get(idx);
    let is_active = tui_plan.map(|p| p.status.is_active()).unwrap_or(false);
    let is_failed = tui_plan.map(|p| p.status.is_failed()).unwrap_or(false);
    let task_total = tui_plan.map(|p| p.tasks_total).unwrap_or(plan.task_count);
    let task_done =
        tui_plan
            .map(|p| p.tasks_done)
            .unwrap_or(if plan.completed { task_total } else { 0 });

    // Status icon
    let (icon, icon_style) = if plan.completed {
        (
            "\u{2713}", // checkmark
            Style::default().fg(theme.success),
        )
    } else if is_failed {
        (
            "\u{2717}", // X
            Style::default()
                .fg(theme.danger)
                .add_modifier(Modifier::BOLD),
        )
    } else if is_active {
        (
            "\u{25b6}", // ▶
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            "\u{25cb}", // ○
            Style::default().fg(theme.muted),
        )
    };

    // Text styling
    let text_style = if plan.completed {
        Style::default().fg(theme.success)
    } else if is_active {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else if is_failed {
        Style::default().fg(theme.danger)
    } else {
        Style::default().fg(theme.foreground)
    };

    let bg = if is_selected {
        theme.selection_background
    } else {
        Color::Reset
    };

    let indent = if indented { "   " } else { " " };

    // Progress fraction
    let fill_pct = if task_total > 0 {
        task_done as f64 / task_total as f64
    } else {
        if plan.completed { 1.0 } else { 0.0 }
    };

    // Progress cell
    let progress_str = if task_total > 0 {
        format!("{}/{}", task_done.min(99), task_total.min(99))
    } else {
        "\u{00b7}".to_string()
    };
    let progress_color = if plan.completed {
        theme.success
    } else if is_active {
        semantic_color(fill_pct, theme)
    } else if is_failed {
        theme.danger
    } else {
        theme.muted
    };

    // Bar cell (8 chars)
    let bar_w = 8usize;
    let filled = (fill_pct.clamp(0.0, 1.0) * bar_w as f64).round() as usize;
    let empty = bar_w.saturating_sub(filled);
    let bar_color = if plan.completed {
        theme.success
    } else if is_failed {
        theme.danger
    } else if is_active {
        semantic_color(fill_pct, theme)
    } else if task_done == 0 {
        Color::Rgb(40, 35, 42)
    } else {
        semantic_color(fill_pct, theme)
    };

    // Name column budget
    let reserved = 20usize; // progress + bar + separators
    let name_budget = content_width
        .saturating_sub(indent.len() + 2 + reserved)
        .max(8);
    let plan_name = truncate_middle(&plan.title, name_budget);

    let sep_style = Style::default().fg(Color::Rgb(40, 35, 42));

    let mut spans = vec![
        Span::styled(indent.to_string(), Style::default().bg(bg)),
        Span::styled(format!("{icon} "), icon_style.bg(bg)),
        Span::styled(
            format!("{:<width$}", plan_name, width = name_budget),
            text_style.bg(bg),
        ),
        Span::styled("\u{2502}", sep_style.bg(bg)),
        Span::styled(
            format!("{:>6}", progress_str),
            Style::default().fg(progress_color).bg(bg),
        ),
        Span::styled("\u{2502}", sep_style.bg(bg)),
        Span::styled(
            format!(
                "{}{}",
                "\u{2588}".repeat(filled.min(bar_w)),
                "\u{2500}".repeat(empty)
            ),
            Style::default().fg(bar_color).bg(bg),
        ),
    ];

    // Task count
    if content_width > 45 {
        spans.push(Span::styled(
            format!(" {}t", task_total),
            Style::default().fg(theme.muted).bg(bg),
        ));
    }

    lines.push(Line::from(spans));

    // Selected plan detail row
    if is_selected {
        let mut detail_spans = vec![Span::styled(format!("{indent}  "), Style::default().bg(bg))];

        // Mini progress bar
        if task_total > 0 {
            let mini_filled = (fill_pct * 8.0).round() as usize;
            let mini_empty = 8usize.saturating_sub(mini_filled);
            detail_spans.push(Span::styled(
                format!(
                    " {}{}",
                    "\u{2588}".repeat(mini_filled.min(8)),
                    "\u{2500}".repeat(mini_empty)
                ),
                Style::default().fg(semantic_color(fill_pct, theme)).bg(bg),
            ));
            detail_spans.push(Span::styled("  ", Style::default().bg(bg)));
        }

        // Status label
        let status_label = if plan.completed {
            "done"
        } else if is_active {
            "running"
        } else if is_failed {
            "failed"
        } else {
            "pending"
        };
        detail_spans.push(Span::styled(
            status_label.to_string(),
            Style::default()
                .fg(if plan.completed {
                    theme.success
                } else if is_active {
                    theme.accent
                } else if is_failed {
                    theme.danger
                } else {
                    theme.muted
                })
                .bg(bg),
        ));

        if task_total > 0 {
            detail_spans.push(Span::styled(
                format!(" \u{00b7} {task_total} tasks"),
                Style::default().fg(theme.muted).bg(bg),
            ));
        }

        if plan.old_format {
            detail_spans.push(Span::styled(
                " \u{00b7} old format",
                Style::default().fg(theme.warning).bg(bg),
            ));
        }

        lines.push(Line::from(detail_spans));

        // Show last error as an additional detail line.
        if let Some(err) = &plan.last_error {
            let err_budget = content_width.saturating_sub(indent.len() + 6);
            lines.push(Line::from(vec![
                Span::styled(format!("{indent}  "), Style::default().bg(bg)),
                Span::styled(
                    "\u{26a0} ",
                    Style::default()
                        .fg(theme.danger)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate(err, err_budget),
                    Style::default().fg(theme.danger).bg(bg),
                ),
            ]));
        }
    }
}

// ---------------------------------------------------------------------------
// Right panel: plan detail
// ---------------------------------------------------------------------------

fn render_right_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let focused = matches!(tui_state.focus, FocusZone::RightPanel);
    let border_style = if focused {
        Theme::focused_border_style()
    } else {
        theme.muted()
    };
    let title_style = if focused {
        Theme::focused_title_style()
    } else {
        theme.muted()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Plan Detail ", title_style))
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if let Some(plan) = tui_state.plans.get(tui_state.selected_plan_idx) {
        let plan_summary = tui_state
            .plan_summaries
            .iter()
            .find(|summary| summary.id == plan.id)
            .or_else(|| tui_state.plan_summaries.get(tui_state.selected_plan_idx));
        let plan_execution = tui_state
            .current_plan_execution
            .as_ref()
            .filter(|exec| exec.plan_id == plan.id);
        render_plan_summary(
            frame,
            inner,
            plan,
            plan_summary,
            plan_execution,
            tui_state,
            view_state,
            theme,
        );
    } else {
        let v_pad = inner.height / 2;
        let mut empty_lines: Vec<Line<'_>> = Vec::new();
        for _ in 0..v_pad.saturating_sub(1) {
            empty_lines.push(Line::from(""));
        }
        empty_lines.push(Line::from(Span::styled(
            "select a plan from the left panel",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        )));
        let empty = Paragraph::new(empty_lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
    }
}

// ---------------------------------------------------------------------------
// Selected plan detail
// ---------------------------------------------------------------------------

fn render_plan_summary(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &PlanEntry,
    plan_summary: Option<&crate::plan::PlanSummary>,
    plan_execution: Option<&crate::tui::dashboard::PlanExecutionSnapshot>,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let plan_name = if plan.name.is_empty() {
        plan_summary
            .map(|summary| summary.title.as_str())
            .unwrap_or(plan.id.as_str())
    } else {
        plan.name.as_str()
    };
    let summary_completed = plan_summary
        .map(|summary| summary.completed)
        .unwrap_or(false);
    let tasks_total = plan
        .tasks_total
        .max(plan_summary.map_or(0, |summary| summary.task_count));
    let tasks_done = plan.tasks_done.min(tasks_total);
    let pct = if tasks_total > 0 {
        tasks_done as f64 / tasks_total as f64
    } else if summary_completed {
        1.0
    } else {
        0.0
    };
    let raw_status = if plan.status.is_active() && !plan.phase.is_empty() {
        plan.phase.as_str()
    } else if summary_completed {
        "completed"
    } else {
        plan.status.label()
    };
    let (status_icon, status_color, status_label) = if summary_completed || plan.status.is_done() {
        ("\u{2713}", theme.success, "completed")
    } else if plan.status.is_failed() {
        ("\u{2717}", theme.danger, "failed")
    } else if plan.status.is_active() {
        ("\u{25b6}", theme.warning, raw_status)
    } else {
        ("\u{25cb}", theme.muted, raw_status)
    };
    let plan_gates: Vec<_> = tui_state
        .gate_result_summaries
        .iter()
        .filter(|gate| gate.plan_id == plan.id)
        .collect();
    let gate_passed = plan_gates.iter().filter(|gate| gate.passed).count();
    let last_error = plan_summary.and_then(|summary| summary.last_error.as_deref());
    let bar_w = area.width.saturating_sub(28).clamp(10, 32) as usize;
    let bar = build_progress_bar(pct, bar_w);
    let bar_color = if tasks_done == tasks_total && tasks_total > 0 {
        theme.success
    } else if plan.tasks_failed > 0 {
        theme.danger
    } else {
        semantic_color(pct, theme)
    };

    let mut header_lines = vec![
        Line::from(vec![
            Span::styled(" plan: ", Style::default().fg(theme.muted)),
            Span::styled(
                plan_name,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" status: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{status_icon} {status_label}"),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("id ", Style::default().fg(theme.muted)),
            Span::styled(
                truncate(&plan.id, 24),
                Style::default().fg(theme.foreground),
            ),
        ]),
        Line::from(vec![
            Span::styled(" tasks: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{tasks_done}/{tasks_total} done"),
                Style::default().fg(theme.foreground),
            ),
            Span::styled(
                format!("  {} failed", plan.tasks_failed),
                Style::default().fg(if plan.tasks_failed > 0 {
                    theme.danger
                } else {
                    theme.muted
                }),
            ),
            Span::styled(
                format!("  {gate_passed}/{} gates", plan_gates.len()),
                Style::default().fg(if plan_gates.is_empty() {
                    theme.muted
                } else if gate_passed == plan_gates.len() {
                    theme.success
                } else {
                    theme.warning
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled(" progress: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{tasks_done}/{tasks_total}"),
                Style::default().fg(theme.foreground),
            ),
            Span::raw("  "),
            Span::styled(bar, Style::default().fg(bar_color)),
            Span::styled(
                format!(" {:.0}%", pct * 100.0),
                Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    if let Some(err) = last_error {
        header_lines.push(Line::from(vec![
            Span::styled(" error: ", Style::default().fg(theme.danger)),
            Span::styled(
                truncate(err, area.width.saturating_sub(10) as usize),
                Style::default()
                    .fg(theme.danger)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    header_lines.push(Line::from(Span::styled(
        format!(
            " {}",
            "\u{2500}".repeat(area.width.saturating_sub(3) as usize)
        ),
        Style::default().fg(Color::Rgb(40, 35, 42)),
    )));

    let header_height = header_lines.len() as u16;
    let sections = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(0),
        Constraint::Length(6),
        Constraint::Length(4),
    ])
    .split(area);

    frame.render_widget(Paragraph::new(header_lines), sections[0]);
    render_plan_tasks(frame, sections[1], &plan.tasks, view_state, theme);
    render_plan_gates(frame, sections[2], &plan_gates, theme);
    render_plan_timing(frame, sections[3], plan, plan_execution, &plan_gates, theme);
}

fn render_plan_tasks(
    frame: &mut Frame<'_>,
    area: Rect,
    tasks: &[TaskEntry],
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .title(Span::styled(" Tasks ", Style::default().fg(theme.muted)))
        .border_style(Style::default().fg(Color::Rgb(40, 35, 42)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if tasks.is_empty() {
        let empty = Paragraph::new(Span::styled(
            " no tasks recorded for this plan",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ))
        .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let rows: Vec<Row<'_>> = tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            let status = task.status;
            let (icon, icon_color) = task_status_icon(task, theme);
            let task_title = if task.name.is_empty() {
                task.id.as_str()
            } else {
                task.name.as_str()
            };
            let style = if i == view_state.secondary_selected {
                theme.selection()
            } else {
                theme.text()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!(" {icon}"),
                    Style::default().fg(icon_color),
                )),
                Cell::from(truncate(task_title, 32)),
                Cell::from(Span::styled(
                    truncate(status.label(), 12),
                    Style::default().fg(phase_color(status, theme)),
                )),
                Cell::from(
                    task.agent_id
                        .as_deref()
                        .map(|agent| truncate(agent, 14))
                        .unwrap_or_else(|| "-".to_string()),
                ),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(16),
        Constraint::Length(12),
        Constraint::Min(8),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new([" ", "task", "status", "agent"]).style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

fn render_plan_gates(
    frame: &mut Frame<'_>,
    area: Rect,
    plan_gates: &[&crate::tui::dashboard::GateResultSummary],
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .title(Span::styled(
            " Gate Results ",
            Style::default().fg(theme.muted),
        ))
        .border_style(Style::default().fg(Color::Rgb(40, 35, 42)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if plan_gates.is_empty() {
        let empty = Paragraph::new(Span::styled(
            " no gate results yet",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ))
        .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let rows: Vec<Row<'_>> = plan_gates
        .iter()
        .rev()
        .map(|gate| {
            let (icon, color) = if gate.passed {
                ("\u{2713}", theme.success)
            } else {
                ("\u{2717}", theme.danger)
            };

            Row::new(vec![
                Cell::from(Span::styled(format!(" {icon}"), Style::default().fg(color))),
                Cell::from(truncate(&gate.gate_name, 12)),
                Cell::from(Span::styled(
                    truncate(&gate.summary, 36),
                    Style::default().fg(theme.foreground),
                )),
                Cell::from(Span::styled(
                    format_duration_ms(gate.duration_ms),
                    Style::default().fg(theme.muted),
                )),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(12),
        Constraint::Min(12),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new([" ", "gate", "summary", "time"]).style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

fn render_plan_timing(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &PlanEntry,
    plan_execution: Option<&crate::tui::dashboard::PlanExecutionSnapshot>,
    plan_gates: &[&crate::tui::dashboard::GateResultSummary],
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .title(Span::styled(" Timing ", Style::default().fg(theme.muted)))
        .border_style(Style::default().fg(Color::Rgb(40, 35, 42)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let timing_lines = build_timing_lines(plan, plan_execution, plan_gates, theme);
    if timing_lines.is_empty() {
        let empty = Paragraph::new(Span::styled(
            " timing not available",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ))
        .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    frame.render_widget(
        Paragraph::new(timing_lines).wrap(Wrap { trim: false }),
        inner,
    );
}

fn build_timing_lines(
    plan: &PlanEntry,
    plan_execution: Option<&crate::tui::dashboard::PlanExecutionSnapshot>,
    plan_gates: &[&crate::tui::dashboard::GateResultSummary],
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let tasks_done = plan.tasks_done;

    if plan.elapsed_secs > 0.0 {
        lines.push(Line::from(vec![
            Span::styled(" total ", Style::default().fg(theme.muted)),
            Span::styled(
                format_duration_secs(plan.elapsed_secs),
                Style::default().fg(theme.foreground),
            ),
        ]));
    }

    if plan.elapsed_secs > 0.0 && tasks_done > 0 {
        lines.push(Line::from(vec![
            Span::styled(" avg/done ", Style::default().fg(theme.muted)),
            Span::styled(
                format_duration_secs(plan.elapsed_secs / tasks_done as f64),
                Style::default().fg(theme.foreground),
            ),
        ]));
    }

    if let Some(exec) = plan_execution {
        if let Some(current_task) = exec.tasks.iter().find(|task| task.is_current) {
            if let Some(current_secs) = parse_duration_secs(&current_task.duration) {
                lines.push(Line::from(vec![
                    Span::styled(" current ", Style::default().fg(theme.muted)),
                    Span::styled(
                        format_duration_secs(current_secs),
                        Style::default().fg(theme.warning),
                    ),
                ]));
            }
        }
    }

    if !plan_gates.is_empty() {
        let gate_secs = plan_gates
            .iter()
            .map(|gate| gate.duration_ms as f64 / 1000.0)
            .sum::<f64>();
        lines.push(Line::from(vec![
            Span::styled(" gates ", Style::default().fg(theme.muted)),
            Span::styled(
                format_duration_secs(gate_secs),
                Style::default().fg(theme.foreground),
            ),
        ]));
    }

    lines
}

fn task_status_icon(task: &TaskEntry, theme: &Theme) -> (&'static str, Color) {
    match task.status {
        TaskStatus::Done => ("\u{2713}", theme.success),
        TaskStatus::Active => ("\u{25b6}", theme.warning),
        TaskStatus::Failed | TaskStatus::Blocked => ("\u{2717}", theme.danger),
        TaskStatus::Pending => ("\u{00b7}", theme.muted),
    }
}

// ---------------------------------------------------------------------------
// Scrollbar (buffer-direct rendering)
// ---------------------------------------------------------------------------

fn render_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    total: usize,
    visible: usize,
    offset: usize,
    theme: &Theme,
) {
    if total <= visible || area.height == 0 {
        return;
    }

    let track_height = area.height as usize;
    let thumb_height = ((visible as f64 / total as f64) * track_height as f64)
        .ceil()
        .max(1.0) as usize;
    let thumb_top = if total > visible {
        ((offset as f64 / (total - visible) as f64) * (track_height - thumb_height) as f64).round()
            as usize
    } else {
        0
    };

    let x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();

    for i in 0..track_height {
        let y = area.y + i as u16;
        let in_thumb = i >= thumb_top && i < thumb_top + thumb_height;
        let (ch, color) = if in_thumb {
            ('\u{2588}', theme.accent) // filled block
        } else {
            ('\u{2502}', Color::Rgb(40, 35, 42)) // thin line
        };
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(ch);
            cell.set_fg(color);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a smooth fractional progress bar string.
fn build_progress_bar(progress: f64, width: usize) -> String {
    let progress = progress.clamp(0.0, 1.0);
    let filled_exact = progress * width as f64;
    let full_blocks = filled_exact.floor() as usize;
    let fractional = filled_exact - full_blocks as f64;
    let fractional_idx = (fractional * (BLOCKS.len() - 1) as f64).round() as usize;

    let mut bar = String::with_capacity(width);
    for _ in 0..full_blocks.min(width) {
        bar.push('\u{2588}');
    }
    if full_blocks < width && fractional_idx > 0 {
        bar.push(BLOCKS[fractional_idx.min(BLOCKS.len() - 1)]);
    }
    let remaining = width.saturating_sub(bar.chars().count());
    for _ in 0..remaining {
        bar.push('\u{2591}');
    }
    bar
}

/// Build a compact mini-bar for wave headers.
fn build_mini_bar(
    width: usize,
    fill_pct: f64,
    done: bool,
    _active: bool,
    _theme: &Theme,
) -> String {
    let filled = (fill_pct.clamp(0.0, 1.0) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!(
        "[{}{}]",
        "\u{2588}".repeat(filled.min(width)),
        if done { "\u{2588}" } else { "\u{2500}" }
            .repeat(empty)
            .chars()
            .take(empty)
            .collect::<String>()
    )
}

/// Progress bar gradient color.
fn progress_color(pct: f64, total: usize, completed: usize, theme: &Theme) -> Color {
    if completed == total && total > 0 {
        theme.success
    } else if pct >= 0.5 {
        theme.accent
    } else if pct >= 0.2 {
        theme.warning
    } else {
        theme.muted
    }
}

/// Semantic color based on completion fraction.
fn semantic_color(pct: f64, theme: &Theme) -> Color {
    if pct >= 0.9 {
        theme.success
    } else if pct >= 0.5 {
        theme.accent
    } else if pct >= 0.2 {
        theme.warning
    } else {
        theme.muted
    }
}

/// Phase-specific color.
fn phase_color(phase: TaskStatus, theme: &Theme) -> Color {
    match phase {
        TaskStatus::Done => theme.success,
        TaskStatus::Failed | TaskStatus::Blocked => theme.danger,
        TaskStatus::Active => theme.accent,
        TaskStatus::Pending => theme.foreground,
    }
}

fn format_duration_ms(duration_ms: u64) -> String {
    if duration_ms >= 1000 {
        format_duration_secs(duration_ms as f64 / 1000.0)
    } else {
        format!("{duration_ms}ms")
    }
}

fn format_duration_secs(seconds: f64) -> String {
    let total_seconds = seconds.max(0.0).round() as u64;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let secs = total_seconds % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {secs}s")
    } else {
        format!("{secs}s")
    }
}

fn parse_duration_secs(duration: &str) -> Option<f64> {
    if duration.is_empty() || duration == "--" {
        return None;
    }

    let mut total = 0.0;
    let mut matched = false;
    for part in duration.split_whitespace() {
        if let Some(ms) = part.strip_suffix("ms") {
            total += ms.parse::<f64>().ok()? / 1000.0;
            matched = true;
        } else if let Some(hours) = part.strip_suffix('h') {
            total += hours.parse::<f64>().ok()? * 3600.0;
            matched = true;
        } else if let Some(minutes) = part.strip_suffix('m') {
            total += minutes.parse::<f64>().ok()? * 60.0;
            matched = true;
        } else if let Some(seconds) = part.strip_suffix('s') {
            total += seconds.parse::<f64>().ok()?;
            matched = true;
        }
    }

    matched.then_some(total)
}

/// Truncate with trailing `...`.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
