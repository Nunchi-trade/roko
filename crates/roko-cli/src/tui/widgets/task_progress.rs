//! Task progress widget with semantic progress bar and scrollable task list.
//!
//! Ported from Mori's task_progress.rs — uses MoriTheme, Atmosphere, TuiState.
//!
//! Layout:
//! ```text
//! ┌ Tasks · plan-001 (5/12) ────────────────────────┐
//! │ ████████░░░░░░░░░░░░░  5/12  ETA:~8m            │
//! │  RUN  2 active · 5 queued · phase implementing   │
//! │ ▲ more                                           │
//! │ ✓ t-001  Wire SystemPromptBuilder                │
//! │ ► t-002  ⏱2m  Add episode logging         [impl]│
//! │ · t-003  ⏱~5m Refactor gate pipeline             │
//! │ ✗ t-004  Fix clippy warnings                     │
//! │ ▼ more                                           │
//! └─────────────────────────────────────────────────┘
//! ```

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use super::super::mori_theme::MoriTheme;
use super::super::tui_state::{TaskRowStatus, TuiState};

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the task progress widget.
pub fn render_task_progress(frame: &mut Frame<'_>, area: Rect, state: &TuiState, focused: bool) {
    let atm = &state.atmosphere;
    let tasks = &state.current_task_checklist;

    // Count by status
    let done = tasks
        .iter()
        .filter(|t| t.status == TaskRowStatus::Done)
        .count();
    let active = tasks
        .iter()
        .filter(|t| t.status == TaskRowStatus::Active)
        .count();
    let blocked = tasks
        .iter()
        .filter(|t| t.status == TaskRowStatus::Blocked)
        .count();
    let failed = tasks
        .iter()
        .filter(|t| t.status == TaskRowStatus::Failed)
        .count();
    let total = tasks.len();
    let pending = total.saturating_sub(done + active + blocked + failed);

    // Title
    let mut title = format!("Tasks ({}/{})", done, total);

    let (border_style, ttl_style) = if focused {
        (
            MoriTheme::focused_border_style(),
            MoriTheme::focused_title_style(),
        )
    } else {
        (
            MoriTheme::unfocused_border_style(),
            MoriTheme::unfocused_title_style(),
        )
    };

    // Pre-compute how many header rows we'll have (progress bar + summary)
    let inner_width = area.width.saturating_sub(4) as usize;
    let has_bar = inner_width > 8 && total > 0;
    let header_rows: u16 = if has_bar { 2 } else { 1 };

    // Visible task slots
    let visible = area.height.saturating_sub(2 + header_rows) as usize;
    let scroll = state.task_scroll.min(tasks.len().saturating_sub(1));
    let start = scroll;
    let end = (scroll + visible).min(tasks.len());

    // Append scroll position to title
    if tasks.len() > visible && visible > 0 {
        title.push_str(&format!(" [{}-{} of {}]", start + 1, end, tasks.len()));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(MoriTheme::block_style())
        .border_style(border_style)
        .title_style(ttl_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 1 || inner.width < 8 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // ── Progress bar ─────────────────────────────────────────────────────
    if has_bar {
        let fill_pct = done as f64 / total.max(1) as f64;
        let bar_spans = semantic_bar(inner_width, fill_pct, Some(atm.heartbeat()));
        let suffix = format!("  {}/{}", done, total);
        let mut bar_line = vec![Span::styled(" ", Style::default())];
        bar_line.extend(bar_spans);
        bar_line.push(Span::styled(suffix, Style::default().fg(MoriTheme::FG_DIM)));
        lines.push(Line::from(bar_line));
    }

    // ── Summary line ─────────────────────────────────────────────────────
    let summary = build_summary_line(done, total, active, pending, blocked, failed, inner_width);
    lines.push(summary);

    // ── Scroll-up indicator ──────────────────────────────────────────────
    if start > 0 {
        lines.push(Line::from(Span::styled(
            " \u{25b2} more",
            Style::default().fg(MoriTheme::TEXT_DIM),
        )));
    }

    // ── Task rows ────────────────────────────────────────────────────────
    for (i, task) in tasks[start..end].iter().enumerate() {
        let global_idx = start + i;
        let is_selected = global_idx == scroll && focused;

        let (icon, icon_style) = match task.status {
            TaskRowStatus::Done => ("\u{2713}", Style::default().fg(MoriTheme::STATUS_OK)),
            TaskRowStatus::Active => {
                let pulse_color = pulse_rose(atm.heartbeat());
                (
                    "\u{25ba}",
                    Style::default()
                        .fg(pulse_color)
                        .add_modifier(Modifier::BOLD),
                )
            }
            TaskRowStatus::Blocked => ("\u{2717}", Style::default().fg(MoriTheme::STATUS_ERROR)),
            TaskRowStatus::Failed => ("\u{2717}", Style::default().fg(MoriTheme::EMBER)),
            TaskRowStatus::Pending => ("\u{00b7}", Style::default().fg(MoriTheme::TEXT_DIM)),
        };

        let (text_style, bg) = if is_selected {
            (
                Style::default()
                    .fg(MoriTheme::BONE)
                    .add_modifier(Modifier::BOLD)
                    .bg(MoriTheme::BG_HIGHLIGHT),
                Some(MoriTheme::BG_HIGHLIGHT),
            )
        } else {
            (Style::default().fg(MoriTheme::TEXT), None)
        };

        let effective_icon_style = if let Some(bg_color) = bg {
            icon_style.bg(bg_color)
        } else {
            icon_style
        };

        // Time tag
        let time_tag = match task.status {
            TaskRowStatus::Done => String::new(),
            TaskRowStatus::Active if task.elapsed_secs > 0.0 => {
                format!(" \u{23F1}{} ", compact_duration(task.elapsed_secs as u64))
            }
            _ => String::new(),
        };

        // Truncate title
        let time_tag_len = time_tag.chars().count();
        let prefix_len = 4 + task.id.len() + time_tag_len;
        let max_title = (inner.width as usize).saturating_sub(prefix_len + 2);
        let title_display = if task.title.chars().count() > max_title && max_title > 3 {
            let truncated: String = task
                .title
                .chars()
                .take(max_title.saturating_sub(3))
                .collect();
            format!("{truncated}...")
        } else {
            task.title.clone()
        };

        let mut task_spans = vec![
            Span::styled(format!(" {icon} "), effective_icon_style),
            Span::styled(
                format!("{}  ", &task.id),
                Style::default().fg(MoriTheme::ROSE_DIM),
            ),
        ];
        if !time_tag.is_empty() {
            task_spans.push(Span::styled(
                time_tag,
                Style::default().fg(MoriTheme::TEXT_DIM),
            ));
        }
        task_spans.push(Span::styled(title_display, text_style));

        lines.push(Line::from(task_spans));
    }

    // ── Scroll-down indicator ────────────────────────────────────────────
    if end < tasks.len() {
        lines.push(Line::from(Span::styled(
            " \u{25bc} more",
            Style::default().fg(MoriTheme::TEXT_DIM),
        )));
    }

    // ── Empty state ──────────────────────────────────────────────────────
    if tasks.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(" {} waiting for tasks...", atm.spinner()),
            Style::default().fg(MoriTheme::TEXT_DIM),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);

    // ── Scrollbar ────────────────────────────────────────────────────────
    if tasks.len() > visible && visible > 0 {
        let sb_area = Rect::new(
            inner.x,
            inner.y + header_rows,
            inner.width,
            inner.height.saturating_sub(header_rows),
        );
        let mut sb_state = ScrollbarState::new(tasks.len()).position(scroll);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(Style::default().fg(MoriTheme::ROSE))
            .track_style(Style::default().fg(MoriTheme::TEXT_PHANTOM))
            .begin_symbol(Some("\u{25b2}"))
            .end_symbol(Some("\u{25bc}"));
        frame.render_stateful_widget(scrollbar, sb_area, &mut sb_state);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a semantic progress bar with gradient coloring.
fn semantic_bar(width: usize, pct: f64, heartbeat: Option<f64>) -> Vec<Span<'static>> {
    let pct = pct.clamp(0.0, 1.0);
    let filled = ((width as f64) * pct).round() as usize;
    let empty = width.saturating_sub(filled);

    let mut spans = Vec::new();

    // Filled portion with semantic color gradient
    if filled > 0 {
        let color = MoriTheme::semantic_color(pct);
        let fill_style = if let Some(hb) = heartbeat {
            // Pulse the leading edge
            let scale = hb.clamp(0.9, 1.1);
            match color {
                Color::Rgb(r, g, b) => Style::default().fg(Color::Rgb(
                    ((r as f64) * scale).min(255.0) as u8,
                    ((g as f64) * scale).min(255.0) as u8,
                    ((b as f64) * scale).min(255.0) as u8,
                )),
                _ => Style::default().fg(color),
            }
        } else {
            Style::default().fg(color)
        };
        spans.push(Span::styled("\u{2588}".repeat(filled), fill_style));
    }

    // Empty portion
    if empty > 0 {
        spans.push(Span::styled(
            "\u{2591}".repeat(empty),
            Style::default().fg(MoriTheme::TEXT_PHANTOM),
        ));
    }

    spans
}

/// Build the summary badge line: status tag + counts.
fn build_summary_line(
    done: usize,
    total: usize,
    active: usize,
    pending: usize,
    blocked: usize,
    failed: usize,
    width: usize,
) -> Line<'static> {
    let (status_text, status_color) = if done == total && total > 0 {
        ("DONE", MoriTheme::SAGE)
    } else if failed > 0 {
        ("FAIL", MoriTheme::EMBER)
    } else if active > 0 {
        ("RUN", MoriTheme::WARNING)
    } else {
        ("WAIT", MoriTheme::ROSE_DIM)
    };

    let mut details = Vec::new();
    if done == total && total > 0 {
        details.push("all tasks clear".to_string());
    } else {
        if active > 0 {
            details.push(format!("{active} active"));
        }
        if pending > 0 {
            details.push(format!("{pending} queued"));
        }
        if blocked > 0 {
            details.push(format!("{blocked} blocked"));
        }
        if failed > 0 {
            details.push(format!("{failed} failed"));
        }
    }

    let summary_str = details.join(" \u{00b7} ");
    let max_len = width.saturating_sub(status_text.len() + 4);
    let summary = if summary_str.chars().count() > max_len && max_len > 3 {
        let truncated: String = summary_str
            .chars()
            .take(max_len.saturating_sub(3))
            .collect();
        format!("{truncated}...")
    } else {
        summary_str
    };

    Line::from(vec![
        Span::styled(
            format!(" {} ", status_text),
            Style::default()
                .fg(MoriTheme::VOID)
                .bg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {summary}"),
            Style::default().fg(MoriTheme::TEXT_GHOST),
        ),
    ])
}

/// Compact duration format: "5m", "1h05m", "45s".
fn compact_duration(total_seconds: u64) -> String {
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

/// Modulate ROSE color brightness with heartbeat oscillator.
fn pulse_rose(heartbeat: f64) -> Color {
    let base_r = 170.0;
    let base_g = 112.0;
    let base_b = 136.0;
    let scale = heartbeat.clamp(0.9, 1.1);
    Color::Rgb(
        (base_r * scale).min(255.0) as u8,
        (base_g * scale).min(255.0) as u8,
        (base_b * scale).min(255.0) as u8,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::super::tui_state::{TaskRow, TaskRowStatus, TuiState};
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_state(tasks: Vec<TaskRow>) -> TuiState {
        use super::super::super::dashboard::DashboardData;
        let data = DashboardData::default();
        let mut state = TuiState::from_dashboard_data(&data);
        state.current_task_checklist = tasks;
        state
    }

    fn sample_tasks() -> Vec<TaskRow> {
        vec![
            TaskRow {
                id: "t-001".into(),
                title: "Wire SystemPromptBuilder".into(),
                status: TaskRowStatus::Done,
                elapsed_secs: 120.0,
            },
            TaskRow {
                id: "t-002".into(),
                title: "Add episode logging".into(),
                status: TaskRowStatus::Active,
                elapsed_secs: 45.0,
            },
            TaskRow {
                id: "t-003".into(),
                title: "Refactor gate pipeline".into(),
                status: TaskRowStatus::Pending,
                elapsed_secs: 0.0,
            },
            TaskRow {
                id: "t-004".into(),
                title: "Fix clippy warnings".into(),
                status: TaskRowStatus::Failed,
                elapsed_secs: 30.0,
            },
            TaskRow {
                id: "t-005".into(),
                title: "Blocked on dependency".into(),
                status: TaskRowStatus::Blocked,
                elapsed_secs: 0.0,
            },
        ]
    }

    #[test]
    fn task_progress_renders_without_panic() {
        let state = make_state(sample_tasks());
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_task_progress(frame, area, &state, false);
            })
            .unwrap();
    }

    #[test]
    fn task_progress_empty() {
        let state = make_state(Vec::new());
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_task_progress(frame, area, &state, false);
            })
            .unwrap();
    }

    #[test]
    fn task_progress_focused() {
        let state = make_state(sample_tasks());
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_task_progress(frame, area, &state, true);
            })
            .unwrap();
    }

    #[test]
    fn task_progress_all_done() {
        let tasks = vec![
            TaskRow {
                id: "t-001".into(),
                title: "Done task".into(),
                status: TaskRowStatus::Done,
                elapsed_secs: 60.0,
            },
            TaskRow {
                id: "t-002".into(),
                title: "Also done".into(),
                status: TaskRowStatus::Done,
                elapsed_secs: 30.0,
            },
        ];
        let state = make_state(tasks);
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_task_progress(frame, area, &state, false);
            })
            .unwrap();
    }

    #[test]
    fn compact_duration_formats() {
        assert_eq!(compact_duration(0), "0s");
        assert_eq!(compact_duration(45), "45s");
        assert_eq!(compact_duration(60), "1m");
        assert_eq!(compact_duration(300), "5m");
        assert_eq!(compact_duration(3661), "1h01m");
    }
}
