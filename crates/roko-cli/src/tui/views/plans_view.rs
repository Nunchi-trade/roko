//! F2 Plans view -- Mori-style wave browser + plan detail.
//!
//! Layout: left 31% (wave list with pipeline header + collapsible plan
//! groups), right 69% (selected plan detail with tasks, gate results,
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
use crate::tui::state::TuiState;

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
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let panels =
        Layout::horizontal([Constraint::Percentage(31), Constraint::Percentage(69)]).split(area);

    render_left_panel(frame, panels[0], data, tui_state, view_state, theme);
    render_right_panel(frame, panels[1], data, tui_state, view_state, theme);
}

// ---------------------------------------------------------------------------
// Left panel: pipeline header + wave/plan tree
// ---------------------------------------------------------------------------

fn render_left_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
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

    render_pipeline_header(frame, sections[0], data, tui_state, theme);
    render_selected_plan_summary(frame, sections[1], data, tui_state, view_state, theme);
    render_wave_tree(frame, sections[2], data, tui_state, view_state, theme);
}

// ---------------------------------------------------------------------------
// Pipeline header (3 lines with overall progress)
// ---------------------------------------------------------------------------

fn render_pipeline_header(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let total_plans = data.plans.len();
    let completed = data.plans.iter().filter(|p| p.completed).count();
    let active_count = tui_state
        .plans
        .iter()
        .filter(|p| p.status == "running" || p.status == "active")
        .count();
    let pct = if total_plans > 0 {
        completed as f64 / total_plans as f64
    } else {
        0.0
    };

    let bar_w = (area.width.saturating_sub(24)) as usize;
    let bar = build_progress_bar(pct, bar_w);
    let bar_color = progress_color(pct, total_plans, completed, theme);

    // Health suffix
    let mut health_parts: Vec<Span<'_>> = Vec::new();
    if active_count > 0 {
        health_parts.push(Span::styled(
            format!(" {active_count}\u{25b8}"),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if tui_state.wave_count() > 0 {
        health_parts.push(Span::styled(
            format!(
                " wave {}/{}",
                tui_state.current_wave().saturating_add(1),
                tui_state.wave_count()
            ),
            Style::default().fg(theme.muted),
        ));
    }

    let mut title_spans = vec![Span::styled(
        format!(" {completed}/{total_plans}"),
        Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
    )];
    title_spans.extend(health_parts);

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

    let border_color = if completed == total_plans && total_plans > 0 {
        theme.success
    } else if active_count > 0 {
        theme.accent
    } else {
        theme.muted
    };

    let header = Paragraph::new(header_line).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Pipeline ",
                Style::default()
                    .fg(border_color)
                    .add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(border_color)),
    );
    frame.render_widget(header, area);
}

// ---------------------------------------------------------------------------
// Wave tree: hierarchical wave -> plan list
// ---------------------------------------------------------------------------

fn render_wave_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let focused = matches!(tui_state.focus, FocusZone::PlanTree);
    let total_plans = data.plans.len();
    let completed = data.plans.iter().filter(|p| p.completed).count();
    let failed = tui_state
        .plans
        .iter()
        .filter(|p| p.status == "failed" || p.status == "error")
        .count();

    let mut health_suffix = String::new();
    let active = tui_state
        .plans
        .iter()
        .filter(|p| p.status == "running" || p.status == "active")
        .count();
    if active > 0 {
        health_suffix.push_str(&format!(" {active}\u{25b8}"));
    }
    if failed > 0 {
        health_suffix.push_str(&format!(" {failed}\u{2717}"));
    }

    let title = format!(" Plans ({completed}/{total_plans}{health_suffix}) ");

    let border_style = if focused {
        Style::default().fg(theme.accent)
    } else {
        theme.muted()
    };
    let title_style = if focused || active > 0 {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
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

    if data.plans.is_empty() {
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
    let use_waves = data.plans.len() > 3;

    if use_waves {
        // Group into waves of ~3-5 plans each
        let wave_size = 4usize;
        let num_waves = (data.plans.len() + wave_size - 1) / wave_size;

        for wave_idx in 0..num_waves {
            let start = wave_idx * wave_size;
            let end = (start + wave_size).min(data.plans.len());
            let wave_plans = &data.plans[start..end];

            let wave_done = wave_plans.iter().filter(|p| p.completed).count();
            let wave_total = wave_plans.len();
            let all_done = wave_done == wave_total;
            let any_active = (start..end).any(|i| {
                tui_state
                    .plans
                    .get(i)
                    .map(|p| p.status == "running" || p.status == "active")
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
                        .map(|p| p.status == "failed" || p.status == "error")
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
                if let Some(plan) = data.plans.get(i) {
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
        for (i, plan) in data.plans.iter().enumerate() {
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
    let is_active = tui_plan
        .map(|p| p.status == "running" || p.status == "active")
        .unwrap_or(false);
    let is_failed = tui_plan
        .map(|p| p.status == "failed" || p.status == "error")
        .unwrap_or(false);
    let task_done = tui_plan.map(|p| p.task_done).unwrap_or(0);
    let task_total = plan.task_count;

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
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let focused = matches!(tui_state.focus, FocusZone::RightPanel);
    let border_style = if focused {
        Style::default().fg(theme.accent)
    } else {
        theme.muted()
    };
    let title_style = if focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
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

    // Show execution detail if available
    if let Some(exec) = &data.current_plan_execution {
        // Check for last_error on the matching plan summary.
        let plan_error: Option<&str> = data
            .plans
            .iter()
            .find(|p| p.id == exec.plan_id)
            .and_then(|p| p.last_error.as_deref());
        let error_height = if plan_error.is_some() { 2u16 } else { 0u16 };

        let sections = Layout::vertical([
            Constraint::Length(error_height), // Error banner (if any)
            Constraint::Length(6),            // Plan header + progress
            Constraint::Min(0),               // Task table
            Constraint::Length(6),            // Gate results
        ])
        .split(inner);

        // Render error banner at the top of the detail panel.
        if let Some(err) = plan_error {
            let err_line = Line::from(vec![
                Span::styled(
                    " \u{26a0} ERROR: ",
                    Style::default()
                        .fg(theme.danger)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(truncate(err, 70), Style::default().fg(theme.danger)),
            ]);
            frame.render_widget(Paragraph::new(vec![err_line, Line::from("")]), sections[0]);
        }

        render_execution_header(frame, sections[1], exec, theme);
        render_execution_tasks(frame, sections[2], exec, view_state, theme);
        render_gate_summary(frame, sections[3], data, theme);
        return;
    }

    // Fallback: show selected plan summary
    if let Some(plan) = data.plans.get(view_state.selected) {
        render_plan_summary(frame, inner, plan, data, tui_state, view_state, theme);
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
// Execution header (plan name + progress bar)
// ---------------------------------------------------------------------------

fn render_execution_header(
    frame: &mut Frame<'_>,
    area: Rect,
    exec: &crate::tui::dashboard::PlanExecutionSnapshot,
    theme: &Theme,
) {
    let pct = if exec.tasks_total > 0 {
        exec.tasks_done as f64 / exec.tasks_total as f64
    } else {
        0.0
    };

    let bar_w = 20usize;
    let bar = build_progress_bar(pct, bar_w);
    let bar_color = if exec.tasks_done == exec.tasks_total && exec.tasks_total > 0 {
        theme.success
    } else {
        semantic_color(pct, theme)
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(" plan: ", Style::default().fg(theme.muted)),
            Span::styled(
                &exec.plan_title,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" progress: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{}/{}", exec.tasks_done, exec.tasks_total),
                Style::default().fg(theme.foreground),
            ),
            Span::raw("  "),
            Span::styled(bar, Style::default().fg(bar_color)),
            Span::styled(
                format!(" {:.0}%", pct * 100.0),
                Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" remaining: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{}", exec.tasks_total.saturating_sub(exec.tasks_done)),
                Style::default().fg(theme.foreground),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(" current: ", Style::default().fg(theme.muted)),
            Span::styled(
                exec.current_task
                    .as_ref()
                    .map(|task| task.task_id.as_str())
                    .unwrap_or("-")
                    .to_string(),
                Style::default().fg(theme.warning),
            ),
        ]),
        Line::from(Span::styled(
            format!(
                " {}",
                "\u{2500}".repeat(area.width.saturating_sub(3) as usize)
            ),
            Style::default().fg(Color::Rgb(40, 35, 42)),
        )),
    ];
    let para = Paragraph::new(lines);
    frame.render_widget(para, area);
}

fn render_selected_plan_summary(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Selected Plan ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let selected = data.plans.get(view_state.selected);
    let selected_tui = tui_state.plans.get(view_state.selected);

    if selected.is_none() {
        let empty = Paragraph::new(Span::styled(
            " select a plan to inspect live wave and task state",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ))
        .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let plan = selected.unwrap();
    let plan_state = selected_tui
        .map(|plan| plan.status.as_str())
        .unwrap_or("pending");
    let tasks_done = selected_tui
        .map(|plan| plan.tasks_done)
        .unwrap_or_else(|| if plan.completed { plan.task_count } else { 0 });
    let tasks_total = selected_tui
        .map(|plan| plan.tasks_total)
        .unwrap_or(plan.task_count);
    let tasks_failed = selected_tui.map(|plan| plan.tasks_failed).unwrap_or(0);
    let elapsed_secs = selected_tui.map(|plan| plan.elapsed_secs).unwrap_or(0.0);
    let remaining = tasks_total.saturating_sub(tasks_done);
    let current_wave = tui_state.current_wave().saturating_add(1);
    let total_waves = tui_state.wave_count().max(1);
    let current_exec = data.current_plan_execution.as_ref();

    let lines = vec![
        Line::from(vec![
            Span::styled(" plan ", Style::default().fg(theme.muted)),
            Span::styled(
                &plan.title,
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(format!("({})", plan.id), Style::default().fg(theme.muted)),
        ]),
        Line::from(vec![
            Span::styled(" status ", Style::default().fg(theme.muted)),
            Span::styled(plan_state, Style::default().fg(theme.accent)),
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("wave {current_wave}/{total_waves}"),
                Style::default().fg(theme.muted),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("done {tasks_done}/{tasks_total} rem {remaining}"),
                Style::default().fg(theme.foreground),
            ),
        ]),
        Line::from(vec![
            Span::styled(" elapsed ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{:.1}s", elapsed_secs.max(0.0)),
                Style::default().fg(theme.warning),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(" fail ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{tasks_failed}"),
                Style::default().fg(if tasks_failed > 0 {
                    theme.danger
                } else {
                    theme.muted
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled(" current ", Style::default().fg(theme.muted)),
            Span::styled(
                current_exec
                    .and_then(|exec| exec.current_task.as_ref())
                    .map(|task| task.description.as_str())
                    .unwrap_or("no active execution")
                    .to_string(),
                Style::default().fg(theme.foreground),
            ),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// ---------------------------------------------------------------------------
// Execution tasks table
// ---------------------------------------------------------------------------

fn render_execution_tasks(
    frame: &mut Frame<'_>,
    area: Rect,
    exec: &crate::tui::dashboard::PlanExecutionSnapshot,
    view_state: &ViewState,
    theme: &Theme,
) {
    if exec.tasks.is_empty() {
        let empty = Paragraph::new(Span::styled(
            " no tasks in execution",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ))
        .wrap(Wrap { trim: false });
        frame.render_widget(empty, area);
        return;
    }

    let rows: Vec<Row<'_>> = exec
        .tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            let (icon, icon_color) = if task.is_current {
                ("\u{25b6}", theme.warning) // ▶
            } else if task.phase == "done" || task.phase == "completed" {
                ("\u{2713}", theme.success) // checkmark
            } else if task.phase == "failed" {
                ("\u{2717}", theme.danger) // X
            } else {
                ("\u{00b7}", theme.muted) // ·
            };

            let style = if task.is_current {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else if i == view_state.secondary_selected {
                theme.selection()
            } else {
                theme.text()
            };

            let short_model = if task.model.is_empty() {
                "-".to_string()
            } else {
                shorten_model(&task.model)
            };

            // Phase chip with semantic color
            let phase_color = phase_color(&task.phase, theme);

            Row::new(vec![
                Cell::from(Span::styled(
                    format!(" {icon}"),
                    Style::default().fg(icon_color),
                )),
                Cell::from(truncate(&task.task_id, 14)),
                Cell::from(truncate(&task.title, 22)),
                Cell::from(Span::styled(
                    truncate(&task.phase, 10),
                    Style::default().fg(phase_color),
                )),
                Cell::from(short_model),
                Cell::from(Span::styled(
                    task.duration.clone(),
                    Style::default().fg(theme.muted),
                )),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(10),
        Constraint::Min(14),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new([" ", "task", "title", "phase", "model", "time"]).style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .column_spacing(1);
    frame.render_widget(table, area);
}

// ---------------------------------------------------------------------------
// Gate results summary
// ---------------------------------------------------------------------------

fn render_gate_summary(frame: &mut Frame<'_>, area: Rect, data: &DashboardData, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::TOP)
        .title(Span::styled(
            " Gate Results ",
            Style::default().fg(theme.muted),
        ))
        .border_style(Style::default().fg(Color::Rgb(40, 35, 42)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.gate_results.is_empty() && data.gate_results_page.gate_rows.is_empty() {
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

    if !data.gate_results_page.gate_rows.is_empty() {
        let rows: Vec<Row<'_>> = data
            .gate_results_page
            .gate_rows
            .iter()
            .take(inner.height as usize)
            .map(|row| {
                let rate_color = if row.pass_rate >= 0.9 {
                    theme.success
                } else if row.pass_rate >= 0.5 {
                    theme.warning
                } else {
                    theme.danger
                };

                // Mini pass-rate bar
                let bar_w = 6;
                let filled = (row.pass_rate * bar_w as f64).round() as usize;
                let empty = bar_w - filled.min(bar_w);
                let mini_bar = format!(
                    "{}{}",
                    "\u{2588}".repeat(filled.min(bar_w)),
                    "\u{2500}".repeat(empty)
                );

                Row::new(vec![
                    Cell::from(Span::styled(
                        format!(" {}", truncate(&row.gate_name, 14)),
                        Style::default().fg(theme.foreground),
                    )),
                    Cell::from(format!("{}", row.total_runs)),
                    Cell::from(Span::styled(
                        format!("{:.0}%", row.pass_rate * 100.0),
                        Style::default().fg(rate_color),
                    )),
                    Cell::from(Span::styled(mini_bar, Style::default().fg(rate_color))),
                    Cell::from(Span::styled(
                        format!("{:.0}ms", row.avg_duration_ms),
                        Style::default().fg(theme.muted),
                    )),
                ])
            })
            .collect();

        let widths = [
            Constraint::Min(10),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(8),
        ];
        let table = Table::new(rows, widths)
            .header(
                Row::new([" gate", "runs", "pass%", "bar", "avg ms"]).style(
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            )
            .column_spacing(1);
        frame.render_widget(table, inner);
    } else {
        // Fall back to raw gate results
        let passed = data.gate_results.iter().filter(|g| g.passed).count();
        let total_gates = data.gate_results.len();
        let color = if passed == total_gates && total_gates > 0 {
            theme.success
        } else if passed > 0 {
            theme.warning
        } else {
            theme.danger
        };

        let pct = if total_gates > 0 {
            (passed as f64 / total_gates as f64 * 100.0).round() as u64
        } else {
            0
        };

        let bar_w = 12;
        let filled = if total_gates > 0 {
            (passed as f64 / total_gates as f64 * bar_w as f64).round() as usize
        } else {
            0
        };
        let empty = bar_w - filled.min(bar_w);

        let summary = Paragraph::new(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                format!("{passed}/{total_gates}"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" gates passed ", Style::default().fg(theme.muted)),
            Span::styled(
                format!(
                    "{}{}",
                    "\u{2588}".repeat(filled.min(bar_w)),
                    "\u{2500}".repeat(empty)
                ),
                Style::default().fg(color),
            ),
            Span::styled(format!(" {pct}%"), Style::default().fg(color)),
        ]));
        frame.render_widget(summary, inner);
    }
}

// ---------------------------------------------------------------------------
// Plan summary (when no active execution)
// ---------------------------------------------------------------------------

fn render_plan_summary(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &crate::plan::PlanSummary,
    data: &DashboardData,
    _tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let header_height = if plan.last_error.is_some() {
        8u16
    } else {
        7u16
    };
    let sections = Layout::vertical([
        Constraint::Length(header_height), // Header (extra line for error)
        Constraint::Min(0),                // Tasks
    ])
    .split(area);

    // Plan header
    let status_style = if plan.completed {
        theme.success()
    } else {
        theme.warning()
    };

    // Gate results for this plan
    let plan_gates: Vec<_> = data
        .gate_results
        .iter()
        .filter(|g| g.plan_id == plan.id)
        .collect();
    let gate_passed = plan_gates.iter().filter(|g| g.passed).count();
    let gate_total = plan_gates.len();

    let mut header_lines = vec![
        Line::from(vec![
            Span::styled(" plan:   ", Style::default().fg(theme.muted)),
            Span::styled(
                &plan.title,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" id:     ", Style::default().fg(theme.muted)),
            Span::styled(&plan.id, Style::default().fg(theme.foreground)),
        ]),
        Line::from(vec![
            Span::styled(" tasks:  ", Style::default().fg(theme.muted)),
            Span::styled(
                plan.task_count.to_string(),
                Style::default().fg(theme.foreground),
            ),
        ]),
        Line::from(vec![
            Span::styled(" status: ", Style::default().fg(theme.muted)),
            if plan.completed {
                Span::styled("\u{2713} completed", status_style)
            } else {
                Span::styled("\u{25cb} pending", status_style)
            },
        ]),
    ];

    // Show last error prominently if present.
    if let Some(err) = &plan.last_error {
        header_lines.push(Line::from(vec![
            Span::styled(" error:  ", Style::default().fg(theme.danger)),
            Span::styled(
                truncate(err, 80),
                Style::default()
                    .fg(theme.danger)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    if gate_total > 0 {
        let gate_color = if gate_passed == gate_total {
            theme.success
        } else if gate_passed > 0 {
            theme.warning
        } else {
            theme.danger
        };
        header_lines.push(Line::from(vec![
            Span::styled(" gates:  ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{gate_passed}/{gate_total} passed"),
                Style::default().fg(gate_color),
            ),
        ]));
    }

    // Separator
    header_lines.push(Line::from(Span::styled(
        format!(
            " {}",
            "\u{2500}".repeat(area.width.saturating_sub(3) as usize)
        ),
        Style::default().fg(Color::Rgb(40, 35, 42)),
    )));

    let header = Paragraph::new(header_lines);
    frame.render_widget(header, sections[0]);

    // Active tasks for this plan
    let matching_tasks: Vec<_> = data
        .active_tasks
        .iter()
        .filter(|t| t.plan_id == plan.id)
        .collect();

    if !matching_tasks.is_empty() {
        let rows: Vec<Row<'_>> = matching_tasks
            .iter()
            .enumerate()
            .map(|(i, task)| {
                let (icon, icon_color) = match task.status.as_str() {
                    "done" | "completed" => ("\u{2713}", theme.success),
                    "running" | "in_progress" => ("\u{25b6}", theme.warning),
                    "failed" => ("\u{2717}", theme.danger),
                    _ => ("\u{00b7}", theme.muted),
                };

                let style = if i == view_state.secondary_selected {
                    theme.selection()
                } else {
                    theme.text()
                };

                let iter_str = if task.iteration > 0 {
                    format!("#{}", task.iteration)
                } else {
                    "-".to_string()
                };

                let agents = if task.assigned_agents.is_empty() {
                    "-".to_string()
                } else {
                    task.assigned_agents.join(", ")
                };

                // Phase color
                let phase_color = phase_color(&task.status, theme);

                Row::new(vec![
                    Cell::from(Span::styled(
                        format!(" {icon}"),
                        Style::default().fg(icon_color),
                    )),
                    Cell::from(truncate(&task.task_id, 18)),
                    Cell::from(Span::styled(
                        task.status.clone(),
                        Style::default().fg(phase_color),
                    )),
                    Cell::from(iter_str),
                    Cell::from(truncate(&agents, 14)),
                ])
                .style(style)
            })
            .collect();

        let widths = [
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(12),
            Constraint::Length(5),
            Constraint::Min(10),
        ];
        let table = Table::new(rows, widths)
            .header(
                Row::new([" ", "task", "status", "iter", "agents"]).style(
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            )
            .column_spacing(1);
        frame.render_widget(table, sections[1]);
    } else {
        let empty = Paragraph::new(Span::styled(
            " no active tasks for this plan",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ))
        .wrap(Wrap { trim: false });
        frame.render_widget(empty, sections[1]);
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
fn phase_color(phase: &str, theme: &Theme) -> Color {
    match phase {
        p if p.contains("done") || p.contains("completed") => theme.success,
        p if p.contains("fail") || p.contains("error") => theme.danger,
        p if p.contains("running") || p.contains("active") || p.contains("implement") => {
            theme.accent
        }
        p if p.contains("gate") || p.contains("verify") || p.contains("test") => theme.warning,
        p if p.contains("compile") || p.contains("build") => theme.info,
        _ => theme.foreground,
    }
}

/// Shorten model slug for compact display.
fn shorten_model(slug: &str) -> String {
    slug.replace("claude-", "")
        .replace("gpt-", "")
        .replace("-codex", "c")
        .replace("-mini", "m")
        .replace("sonnet-", "s")
        .replace("opus-", "o")
        .replace("haiku-", "h")
}

/// Truncate with trailing `...`.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

/// Truncate in the middle with ellipsis.
fn truncate_middle(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return chars[..max].iter().collect();
    }
    let keep_left = (max - 1) / 2;
    let keep_right = max - keep_left - 1;
    let left: String = chars[..keep_left].iter().collect();
    let right: String = chars[chars.len() - keep_right..].iter().collect();
    format!("{left}\u{2026}{right}")
}
