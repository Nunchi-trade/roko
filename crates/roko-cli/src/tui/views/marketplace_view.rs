//! F8 Marketplace view -- job board browser.
//!
//! Layout: left 35% (job list) | right 65% (job detail).
//!
//! Data source: `.roko/jobs/*.json` files (no roko-serve required).
//! Job type tags: research = rose, coding_task = bone, other = muted.
//! Status icons: pending = open circle, active = play, done = check, failed = cross.
//!
//! Keyboard:
//!   j/k     -- navigate list (wraps at boundaries)
//!   Enter   -- focus detail panel
//!   r       -- signal refresh (next file poll picks up changes)

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap};

use super::{SubView, ViewState};
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::{JobFormField, TuiState};
use crate::tui::tabs::Tab;

type Job = roko_core::MarketplaceJob;

/// Canonical status string, preferring `status` over `state`.
fn effective_status(job: &Job) -> &str {
    if !job.status.is_empty() {
        &job.status
    } else if !job.state.is_empty() {
        &job.state
    } else {
        "unknown"
    }
}

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render the full marketplace view.
///
/// Handles terminal resize: the layout uses percentage constraints so it
/// adapts automatically to any terminal width.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let jobs = &tui_state.marketplace_jobs;

    if jobs.is_empty() {
        match view_state.active_sub_view(Tab::Marketplace) {
            SubView::CreateJob => render_create_job(frame, area, tui_state, theme),
            _ => render_empty(frame, area, theme),
        }
        return;
    }

    let selected = view_state.selected.min(jobs.len().saturating_sub(1));
    match view_state.active_sub_view(Tab::Marketplace) {
        SubView::JobDetail => {
            if let Some(job) = jobs.get(selected) {
                render_job_detail(frame, area, job, tui_state, theme);
            }
        }
        SubView::CreateJob => render_create_job(frame, area, tui_state, theme),
        _ => {
            let panels =
                Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
                    .split(area);
            render_job_list(frame, panels[0], jobs, selected, theme);
            if let Some(job) = jobs.get(selected) {
                render_job_detail(frame, panels[1], job, tui_state, theme);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

fn render_empty(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let block = Block::bordered()
        .title(Span::styled(
            " Marketplace ",
            theme.accent().add_modifier(Modifier::BOLD),
        ))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("No jobs posted.", theme.muted())),
        Line::from(""),
        Line::from(Span::styled(
            "Jobs appear when agents or operators post work items to .roko/jobs/.",
            theme.muted(),
        )),
        Line::from(Span::styled(
            "Press 'n' to create a new job manually.",
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
// Left panel: job list
// ---------------------------------------------------------------------------

fn render_job_list(
    frame: &mut Frame<'_>,
    area: Rect,
    jobs: &[Job],
    selected: usize,
    theme: &Theme,
) {
    // Count by canonical status for the header badge.
    let pending = jobs
        .iter()
        .filter(|job| matches!(effective_status(job), "open" | "pending" | "assigned"))
        .count();
    let active = jobs
        .iter()
        .filter(|job| matches!(effective_status(job), "active" | "running" | "in_progress"))
        .count();
    let done = jobs
        .iter()
        .filter(|job| matches!(effective_status(job), "done" | "completed" | "evaluated"))
        .count();

    let block = Block::bordered()
        .title(Span::styled(
            format!(" Jobs ({}) {pending}P {active}A {done}D ", jobs.len()),
            theme.accent().add_modifier(Modifier::BOLD),
        ))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let visible_height = inner.height as usize;
    // Scroll to keep `selected` visible.
    let scroll = if selected >= visible_height {
        selected - visible_height + 1
    } else {
        0
    };

    let items: Vec<ListItem<'_>> = jobs
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, job)| {
            let is_sel = i == selected;
            let status = effective_status(job);

            let (icon, status_style) = match status {
                "open" | "pending" => ("\u{25cb}", theme.muted()), // open circle
                "assigned" => ("\u{25d4}", theme.info()),          // half circle
                "in_progress" | "active" | "running" => ("\u{25b6}", theme.warning()), // play
                "submitted" => ("\u{25d1}", theme.info()),         // half circle
                "done" | "completed" | "evaluated" => ("\u{2713}", theme.success()), // check
                "failed" | "cancelled" => ("\u{2717}", theme.danger()), // cross
                _ => ("\u{00b7}", theme.muted()),                  // dot
            };

            // Job type color tag (research=rose, coding_task=bone/dim, other=muted).
            let type_style = match job.job_type.as_str() {
                "research" => Style::default().fg(Theme::ROSE),
                "coding_task" | "coding" => Style::default().fg(Theme::BONE_DIM),
                _ => theme.muted(),
            };

            let avail_width = (inner.width as usize).saturating_sub(8);
            let title = truncate(&job.title, avail_width);
            let row_style = if is_sel {
                theme.selection()
            } else {
                theme.text()
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {icon} "), status_style),
                // Small type indicator
                Span::styled(
                    format!("[{}] ", &job.job_type.chars().take(3).collect::<String>()),
                    type_style,
                ),
                Span::styled(title, row_style),
            ]))
        })
        .collect();

    frame.render_widget(List::new(items), inner);
}

// ---------------------------------------------------------------------------
// Right panel: job detail
// ---------------------------------------------------------------------------

fn render_job_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    job: &Job,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let block = Block::bordered()
        .title(Span::styled(
            format!(" {} ", truncate(&job.title, 40)),
            theme.accent().add_modifier(Modifier::BOLD),
        ))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 4 || inner.width < 20 {
        return;
    }

    let is_in_progress =
        matches!(effective_status(job), "in_progress" | "active" | "running");
    let has_progress = is_in_progress && tui_state.job_progress.contains_key(&job.id);

    let sections = Layout::vertical([
        Constraint::Length(8),                                  // Metadata table
        Constraint::Min(0),                                     // Description (word-wrapped)
        Constraint::Length(if has_progress { 4 } else { 0 }),   // Progress bar
        Constraint::Length(3),                                   // Keybinding hints + assign prompt
    ])
    .split(inner);

    let status = effective_status(job);
    let status_style = match status {
        "open" | "pending" => theme.muted(),
        "assigned" | "submitted" => theme.info(),
        "in_progress" | "active" | "running" => theme.warning(),
        "done" | "completed" | "evaluated" => theme.success(),
        "failed" | "cancelled" => theme.danger(),
        _ => theme.text(),
    };
    let priority_style = match job.priority.as_str() {
        "critical" | "p0" => theme.danger(),
        "high" | "p1" => theme.warning(),
        "medium" | "p2" | "" => theme.muted(),
        _ => theme.muted(),
    };

    // Build valid-transitions hint
    let parsed_status = roko_core::JobStatus::parse(status).unwrap_or(roko_core::JobStatus::Open);
    let transitions = parsed_status.valid_transitions();
    let transition_hint = if transitions.is_empty() {
        "(terminal)".to_string()
    } else {
        transitions
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let col_widths = [Constraint::Length(11), Constraint::Min(0)];

    let meta_rows = vec![
        Row::new([
            Cell::from(Span::styled("id:", theme.muted())),
            Cell::from(Span::styled(&job.id, theme.text())),
        ]),
        Row::new([
            Cell::from(Span::styled("status:", theme.muted())),
            Cell::from(Line::from(vec![
                Span::styled(status, status_style),
                Span::styled(format!("  \u{2192} {transition_hint}"), theme.muted()),
            ])),
        ]),
        Row::new([
            Cell::from(Span::styled("type:", theme.muted())),
            Cell::from(Span::styled(&job.job_type, theme.text())),
        ]),
        Row::new([
            Cell::from(Span::styled("priority:", theme.muted())),
            Cell::from(Span::styled(
                if job.priority.is_empty() {
                    "\u{2014}"
                } else {
                    &job.priority
                },
                priority_style,
            )),
        ]),
        Row::new([
            Cell::from(Span::styled("posted by:", theme.muted())),
            Cell::from(if job.posted_by.is_empty() {
                Span::styled("\u{2014}", theme.muted())
            } else {
                Span::styled(&job.posted_by, theme.text())
            }),
        ]),
        Row::new([
            Cell::from(Span::styled("assigned:", theme.muted())),
            Cell::from(if job.assigned_to.is_empty() {
                Span::styled("(unassigned)", theme.muted())
            } else {
                Span::styled(&job.assigned_to, theme.info())
            }),
        ]),
        Row::new([
            Cell::from(Span::styled("created:", theme.muted())),
            Cell::from(Span::styled(
                if job.created_at.is_empty() {
                    "\u{2014}"
                } else {
                    &job.created_at
                },
                theme.muted(),
            )),
        ]),
        Row::new([
            Cell::from(Span::styled("tags:", theme.muted())),
            Cell::from(Span::styled(
                if job.tags.is_empty() {
                    "(none)".to_string()
                } else {
                    job.tags.join(", ")
                },
                theme.muted(),
            )),
        ]),
    ];

    frame.render_widget(
        Table::new(meta_rows, col_widths).column_spacing(1),
        sections[0],
    );

    // Description with proper word-wrap using ratatui's Wrap widget.
    let desc_block = Block::default()
        .borders(Borders::TOP)
        .title(Span::styled(" Description ", theme.muted()))
        .border_style(theme.muted());
    let desc_inner = desc_block.inner(sections[1]);
    frame.render_widget(desc_block, sections[1]);

    let desc_text = if job.description.is_empty() {
        "No description provided.".to_string()
    } else {
        job.description.clone()
    };
    frame.render_widget(
        Paragraph::new(desc_text)
            .style(theme.text())
            .wrap(Wrap { trim: false }),
        desc_inner,
    );

    // Progress bar for in-progress jobs.
    if has_progress {
        if let Some(progress) = tui_state.job_progress.get(&job.id) {
            let prog_block = Block::default()
                .borders(Borders::TOP)
                .title(Span::styled(" Progress ", theme.muted()))
                .border_style(theme.muted());
            let prog_inner = prog_block.inner(sections[2]);
            frame.render_widget(prog_block, sections[2]);

            let bar_width = (prog_inner.width as usize).saturating_sub(10);
            let filled = (progress.percent as usize * bar_width) / 100;
            let empty = bar_width.saturating_sub(filled);
            let bar_line = Line::from(vec![
                Span::styled(" [", theme.muted()),
                Span::styled("\u{2588}".repeat(filled), theme.success()),
                Span::styled("\u{2591}".repeat(empty), theme.muted()),
                Span::styled(format!("] {}%", progress.percent), theme.muted()),
            ]);
            let agent_hint = if progress.agent_id.is_empty() {
                String::new()
            } else {
                format!("  agent: {}", progress.agent_id)
            };
            let msg_line = Line::from(vec![
                Span::styled(
                    format!(
                        " {}",
                        truncate(&progress.message, prog_inner.width as usize - 2)
                    ),
                    theme.text(),
                ),
                Span::styled(agent_hint, theme.muted()),
            ]);
            frame.render_widget(Paragraph::new(vec![bar_line, msg_line]), prog_inner);
        }
    }

    // Section index for hints/assign prompt shifts when progress is shown.
    let hints_section = if has_progress { 3 } else { 2 };

    // Bottom hints or assign prompt
    if tui_state.job_assign_editing {
        // Show the assign-agent inline prompt
        let assign_block = Block::bordered()
            .title(Span::styled(
                " Assign to agent: ",
                theme.accent().add_modifier(Modifier::BOLD),
            ))
            .border_style(theme.warning());
        let assign_inner = assign_block.inner(sections[hints_section]);
        frame.render_widget(assign_block, sections[hints_section]);
        frame.render_widget(
            Paragraph::new(format!("{}\u{2588}", tui_state.job_assign_buffer)).style(theme.text()),
            assign_inner,
        );
    } else {
        // Show keybinding hints
        let hint_line = Line::from(vec![
            Span::styled(" s", theme.accent()),
            Span::styled(":status  ", theme.muted()),
            Span::styled("a", theme.accent()),
            Span::styled(":assign  ", theme.muted()),
            Span::styled("n", theme.accent()),
            Span::styled(":new  ", theme.muted()),
            Span::styled("r", theme.accent()),
            Span::styled(":refresh", theme.muted()),
        ]);
        frame.render_widget(
            Paragraph::new(hint_line).alignment(Alignment::Center),
            sections[hints_section],
        );
    }
}

fn render_create_job(frame: &mut Frame<'_>, area: Rect, tui_state: &TuiState, theme: &Theme) {
    let block = Block::bordered()
        .title(Span::styled(
            " New Job ",
            theme.accent().add_modifier(Modifier::BOLD),
        ))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 10 || inner.width < 30 {
        frame.render_widget(
            Paragraph::new("Terminal too small for form.")
                .style(theme.muted())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let sections = Layout::vertical([
        Constraint::Length(3), // Title
        Constraint::Length(3), // Type
        Constraint::Length(3), // Priority
        Constraint::Length(5), // Description (taller)
        Constraint::Length(3), // Buttons / hints
        Constraint::Min(0),    // Padding
    ])
    .split(inner);

    let fields: &[(JobFormField, &str, &str)] = &[
        (JobFormField::Title, "Title", &tui_state.job_form_title),
        (
            JobFormField::Type,
            "Type [coding_task|research|other]",
            &tui_state.job_form_type,
        ),
        (
            JobFormField::Priority,
            "Priority [low|medium|high|critical]",
            &tui_state.job_form_priority,
        ),
        (
            JobFormField::Description,
            "Description",
            &tui_state.job_form_description,
        ),
    ];

    for (i, &(field, label, value)) in fields.iter().enumerate() {
        let is_focused = tui_state.job_form_focus == field;
        let is_editing = is_focused && tui_state.job_form_editing;

        let border_style = if is_editing {
            theme.warning()
        } else if is_focused {
            theme.accent()
        } else {
            theme.muted()
        };

        let field_block = Block::bordered()
            .title(Span::styled(
                format!(" {label} "),
                if is_focused {
                    theme.accent().add_modifier(Modifier::BOLD)
                } else {
                    theme.muted()
                },
            ))
            .border_style(border_style);
        let field_inner = field_block.inner(sections[i]);
        frame.render_widget(field_block, sections[i]);

        let display_value = if is_editing {
            format!("{value}\u{2588}") // block cursor
        } else if value.is_empty() {
            "(empty)".to_string()
        } else {
            value.to_string()
        };
        let text_style = if value.is_empty() && !is_editing {
            theme.muted()
        } else {
            theme.text()
        };

        frame.render_widget(
            Paragraph::new(display_value)
                .style(text_style)
                .wrap(Wrap { trim: false }),
            field_inner,
        );
    }

    // Hints
    let hint_line = Line::from(vec![
        Span::styled(" Tab", theme.accent()),
        Span::styled(":next  ", theme.muted()),
        Span::styled("Enter", theme.accent()),
        Span::styled(":edit  ", theme.muted()),
        Span::styled("Ctrl-S", theme.accent()),
        Span::styled(":submit  ", theme.muted()),
        Span::styled("Esc", theme.accent()),
        Span::styled(":cancel", theme.muted()),
    ]);
    frame.render_widget(
        Paragraph::new(hint_line).alignment(Alignment::Center),
        sections[4],
    );

    // Show command results feedback, or fallback instructions for backend submission.
    if let Some(result) = tui_state.command_results.last() {
        let style = if result.ok {
            theme.success()
        } else {
            theme.danger()
        };
        frame.render_widget(
            Paragraph::new(format!("{}: {}", result.label, result.message))
                .style(style)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false }),
            sections[5],
        );
    } else {
        let help_lines = vec![
            Line::from(Span::styled(
                "Create a job from the CLI:",
                theme.muted(),
            )),
            Line::from(Span::styled(
                "  roko serve                               # start the server",
                theme.muted(),
            )),
            Line::from(Span::styled(
                "  curl -X POST http://localhost:6677/api/jobs \\",
                theme.muted(),
            )),
            Line::from(Span::styled(
                "    -H \"Content-Type: application/json\" \\",
                theme.muted(),
            )),
            Line::from(Span::styled(
                "    -d '{\"title\":\"...\", \"job_type\":\"research\"}'",
                theme.muted(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Or use the roko-serve API directly.",
                theme.muted(),
            )),
            Line::from(Span::styled(
                "Jobs appear here when created via the API.",
                theme.muted(),
            )),
        ];
        frame.render_widget(
            Paragraph::new(help_lines).wrap(Wrap { trim: false }),
            sections[5],
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate(s: &str, max: usize) -> String {
    if max < 4 || s.len() <= max {
        return s.to_string();
    }
    // Find a char boundary near max-3 for the "..." suffix.
    let mut end = max - 3;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}
