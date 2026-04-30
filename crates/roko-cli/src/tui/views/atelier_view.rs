//! F9 Atelier view -- PRD and plan workshop.
//!
//! Layout: top 3-line stats bar + left 40% (PRD list) + right 60% (plan detail).
//!
//! Data source: `roko_core::DashboardSnapshot` via StateHub.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap};

use super::{SubView, ViewState};
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;
use crate::tui::tabs::Tab;

type PrdEntry = roko_core::PrdSummary;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum PrdStatus {
    #[default]
    Idea,
    Draft,
    Published,
    Planned,
}

impl PrdStatus {
    fn from_str(status: &str) -> Self {
        match status.trim().to_ascii_lowercase().as_str() {
            "published" | "publish" => Self::Published,
            "draft" => Self::Draft,
            "planned" | "plan" => Self::Planned,
            _ => Self::Idea,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Idea => "idea",
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Planned => "planned",
        }
    }

    /// Four-character Unicode badge shown in the PRD list.
    fn badge(self) -> &'static str {
        match self {
            Self::Idea => "IDEA",
            Self::Draft => "DRFT",
            Self::Published => "PUBL",
            Self::Planned => "PLAN",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TaskState {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
}

impl TaskState {
    fn from_str(status: &str) -> Self {
        match status.trim().to_ascii_lowercase().as_str() {
            "done" | "completed" | "passed" | "skipped" => Self::Done,
            "running" | "active" | "executing" | "in_progress" | "implementing" | "gating"
            | "verifying" | "reviewing" => Self::Running,
            "failed" | "error" | "gate_rejected" => Self::Failed,
            _ => Self::Pending,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Pending => "[ ]",
            Self::Running => "[>]",
            Self::Done => "[x]",
            Self::Failed => "[!]",
        }
    }
}

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render the full atelier view.
///
/// Handles empty state and terminal resize via percentage constraints.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let prds = &tui_state.atelier_prds;

    let rows = Layout::vertical([
        Constraint::Length(3), // Stats bar
        Constraint::Min(0),    // Main content
    ])
    .split(area);

    render_stats_bar(frame, rows[0], &prds, data, theme);

    if prds.is_empty() {
        render_empty(frame, rows[1], theme);
        return;
    }

    let selected = view_state.selected.min(prds.len().saturating_sub(1));
    match view_state.active_sub_view(Tab::Atelier) {
        SubView::PlanExplorer => {
            render_plan_detail(frame, rows[1], prds, selected, data, tui_state, theme);
        }
        _ => {
            let panels =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .split(rows[1]);
            render_prd_list(frame, panels[0], prds, selected, theme);
            render_plan_detail(frame, panels[1], prds, selected, data, tui_state, theme);
        }
    }
}

// ---------------------------------------------------------------------------
// Stats bar
// ---------------------------------------------------------------------------

fn render_stats_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    prds: &[PrdEntry],
    data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::bordered().border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let plan_count = prds.iter().filter(|p| p.plan_count > 0).count();
    let done_tasks: usize = prds.iter().map(|p| p.task_done).sum();
    let total_tasks: usize = prds.iter().map(|p| p.task_total).sum();

    let all_done = total_tasks > 0 && done_tasks == total_tasks;
    let tasks_style = if all_done {
        theme.success()
    } else {
        theme.text()
    };

    let cols = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
    ])
    .split(inner);

    let stat = |label: &str, value: String, style| {
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{label}: "), theme.muted()),
            Span::styled(value, style),
        ]))
        .alignment(Alignment::Center)
    };

    frame.render_widget(stat("PRDs", prds.len().to_string(), theme.text()), cols[0]);
    frame.render_widget(stat("Plans", plan_count.to_string(), theme.info()), cols[1]);
    frame.render_widget(
        stat("Tasks", format!("{done_tasks}/{total_tasks}"), tasks_style),
        cols[2],
    );
    frame.render_widget(
        stat("Agents", data.agents.len().to_string(), theme.text()),
        cols[3],
    );
    frame.render_widget(
        stat(
            "Episodes",
            data.efficiency.event_count.to_string(),
            theme.muted(),
        ),
        cols[4],
    );
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

fn render_empty(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let block = Block::bordered()
        .title(Span::styled(
            " Atelier ",
            theme.accent().add_modifier(Modifier::BOLD),
        ))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("No PRDs found.", theme.muted())),
        Line::from(""),
        Line::from(Span::styled(
            "Create one with: roko prd idea \"your idea\"",
            theme.muted(),
        )),
        Line::from(Span::styled(
            "Then draft: roko prd draft new \"your-slug\"",
            theme.muted(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        inner,
    );
}

// ---------------------------------------------------------------------------
// Left panel: PRD list
// ---------------------------------------------------------------------------

fn render_prd_list(
    frame: &mut Frame<'_>,
    area: Rect,
    prds: &[PrdEntry],
    selected: usize,
    theme: &Theme,
) {
    let block = Block::bordered()
        .title(Span::styled(
            format!(" PRDs ({}) ", prds.len()),
            theme.accent().add_modifier(Modifier::BOLD),
        ))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let visible_height = inner.height as usize;
    let scroll = if selected >= visible_height {
        selected - visible_height + 1
    } else {
        0
    };

    let items: Vec<ListItem<'_>> = prds
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, prd)| {
            let is_sel = i == selected;
            let status = PrdStatus::from_str(&prd.status);
            let badge_style = match status {
                PrdStatus::Idea => theme.muted(),
                PrdStatus::Draft => theme.warning(),
                PrdStatus::Published => theme.success(),
                PrdStatus::Planned => theme.info(),
            };

            let progress = if prd.task_total > 0 {
                format!(" {}/{}", prd.task_done, prd.task_total)
            } else {
                String::new()
            };

            let title_max = (inner.width as usize).saturating_sub(12 + progress.len());
            let row_style = if is_sel {
                theme.selection()
            } else {
                theme.text()
            };

            let mut spans = vec![
                Span::styled(format!(" {} ", status.badge()), badge_style),
                Span::styled(truncate(&prd.title, title_max), row_style),
            ];

            if !progress.is_empty() {
                let progress_style = if prd.task_failed > 0 {
                    theme.danger()
                } else if prd.task_done == prd.task_total {
                    theme.success()
                } else {
                    theme.muted()
                };
                spans.push(Span::styled(progress, progress_style));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    frame.render_widget(List::new(items), inner);
}

// ---------------------------------------------------------------------------
// Right panel: plan detail with task list
// ---------------------------------------------------------------------------

fn render_plan_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    prds: &[PrdEntry],
    selected: usize,
    _data: &DashboardData,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let Some(prd) = prds.get(selected) else {
        return;
    };

    let block = Block::bordered()
        .title(Span::styled(
            format!(" {} ", truncate(&prd.title, 40)),
            theme.accent().add_modifier(Modifier::BOLD),
        ))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 4 || inner.width < 20 {
        return;
    }

    // Compute how many lines the actions section needs.
    let status = PrdStatus::from_str(&prd.status);
    let actions_height: u16 = match status {
        PrdStatus::Idea | PrdStatus::Draft => 4,
        PrdStatus::Published => 4,
        PrdStatus::Planned => 3,
    };

    let sections = Layout::vertical([
        Constraint::Length(5),              // PRD metadata
        Constraint::Length(actions_height), // CLI actions
        Constraint::Min(0),                 // Task list
        Constraint::Length(1),              // Keybinding hints
    ])
    .split(inner);

    // PRD metadata
    let status_style = match status {
        PrdStatus::Idea => theme.muted(),
        PrdStatus::Draft => theme.warning(),
        PrdStatus::Published => theme.success(),
        PrdStatus::Planned => theme.info(),
    };
    let completion = if prd.task_total > 0 {
        format!(
            "{:.0}%",
            prd.task_done as f64 / prd.task_total as f64 * 100.0
        )
    } else {
        "\u{2014}".to_string() // em dash
    };

    // Build available-actions hint based on current status.
    let action_hint = match status {
        PrdStatus::Idea | PrdStatus::Draft => "p:publish",
        PrdStatus::Published => "g:generate plan",
        PrdStatus::Planned => "(complete)",
    };

    let meta_lines = vec![
        Line::from(vec![
            Span::styled("slug:       ", theme.muted()),
            Span::styled(&prd.slug, theme.text()),
        ]),
        Line::from(vec![
            Span::styled("status:     ", theme.muted()),
            Span::styled(status.label(), status_style),
            Span::styled(format!("  [{action_hint}]"), theme.accent()),
        ]),
        Line::from(vec![
            Span::styled("tasks:      ", theme.muted()),
            Span::styled(
                format!("{}/{}", prd.task_done, prd.task_total),
                theme.text(),
            ),
            Span::styled(format!("  ({completion})"), theme.muted()),
        ]),
        Line::from(vec![
            Span::styled("failed:     ", theme.muted()),
            Span::styled(
                prd.task_failed.to_string(),
                if prd.task_failed > 0 {
                    theme.danger()
                } else {
                    theme.muted()
                },
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(meta_lines).wrap(Wrap { trim: false }),
        sections[0],
    );

    // CLI actions block: show actionable commands based on PRD status.
    let action_lines: Vec<Line<'_>> = match status {
        PrdStatus::Idea | PrdStatus::Draft => vec![
            Line::from(Span::styled("Actions:", theme.accent())),
            Line::from(Span::styled(
                "  roko prd draft promote    # publish this draft".to_string(),
                theme.muted(),
            )),
            Line::from(Span::styled(
                format!(
                    "  roko prd plan {:<12}# generate implementation plan",
                    &prd.slug
                ),
                theme.muted(),
            )),
        ],
        PrdStatus::Published => vec![
            Line::from(Span::styled("Actions:", theme.accent())),
            Line::from(Span::styled(
                format!(
                    "  roko prd plan {:<12}# generate implementation plan",
                    &prd.slug
                ),
                theme.muted(),
            )),
            Line::from(Span::styled(
                "  roko plan run plans/      # execute generated plan".to_string(),
                theme.muted(),
            )),
        ],
        PrdStatus::Planned => vec![
            Line::from(Span::styled("Actions:", theme.accent())),
            Line::from(Span::styled(
                "  roko plan run plans/      # execute the plan".to_string(),
                theme.muted(),
            )),
        ],
    };
    frame.render_widget(
        Paragraph::new(action_lines).wrap(Wrap { trim: false }),
        sections[1],
    );

    // Task list: read from cached atelier tasks in TuiState.
    let empty_tasks = Vec::new();
    let tasks = tui_state
        .atelier_tasks_by_slug
        .get(&prd.slug)
        .unwrap_or(&empty_tasks);

    let task_block = Block::default()
        .borders(Borders::TOP)
        .title(Span::styled(
            format!(" Tasks ({}) ", tasks.len()),
            theme.muted(),
        ))
        .border_style(theme.muted());
    let task_inner = task_block.inner(sections[2]);
    frame.render_widget(task_block, sections[2]);

    if tasks.is_empty() {
        frame.render_widget(
            Paragraph::new("no tasks -- run 'roko prd plan <slug>' to generate")
                .style(theme.muted())
                .wrap(Wrap { trim: false }),
            task_inner,
        );
        return;
    }

    let title_max = (task_inner.width as usize).saturating_sub(24);
    let rows: Vec<Row<'_>> = tasks
        .iter()
        .map(|task| {
            let status = TaskState::from_str(&task.status);
            let icon_style = match status {
                TaskState::Pending => theme.muted(),
                TaskState::Running => theme.warning(),
                TaskState::Done => theme.success(),
                TaskState::Failed => theme.danger(),
            };
            Row::new(vec![
                Cell::from(Span::styled(status.icon(), icon_style)),
                Cell::from(Span::styled(truncate(&task.id, 8), theme.muted())),
                Cell::from(Span::styled(truncate(&task.title, title_max), theme.text())),
                Cell::from(Span::styled(truncate(&task.agent, 12), theme.muted())),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(8),
        Constraint::Min(10),
        Constraint::Length(12),
    ];
    frame.render_widget(
        Table::new(rows, widths)
            .header(
                Row::new(["", "id", "title", "agent"])
                    .style(theme.accent().add_modifier(Modifier::BOLD)),
            )
            .column_spacing(1),
        task_inner,
    );

    // Bottom keybinding hints
    let hint_line = Line::from(vec![
        Span::styled(" p", theme.accent()),
        Span::styled(":publish  ", theme.muted()),
        Span::styled("g", theme.accent()),
        Span::styled(":gen plan  ", theme.muted()),
        Span::styled("r", theme.accent()),
        Span::styled(":refresh", theme.muted()),
    ]);
    frame.render_widget(
        Paragraph::new(hint_line).alignment(Alignment::Center),
        sections[3],
    );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

use crate::tui::display_utils::truncate;
