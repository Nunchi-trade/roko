//! Task checklist widget with overall progress bar.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph};
use ratatui::Frame;

use roko_core::dashboard_snapshot::{SnapshotStats, TaskState};

use super::super::dashboard::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Determine the display icon and style for a task based on its outcome/phase.
fn task_decoration(task: &TaskState, theme: &Theme) -> (&'static str, ratatui::style::Style) {
    match task.outcome.as_deref() {
        Some(o) if o.contains("fail") || o.contains("error") => ("\u{2718}", theme.danger()),
        Some(_) => ("\u{2714}", theme.success()),
        None if task.phase == "completed" => ("\u{2714}", theme.success()),
        None => ("\u{25b6}", theme.warning()),
    }
}

/// Compute overall progress ratio from stats.
fn overall_ratio(stats: &SnapshotStats) -> f64 {
    let total = stats.tasks_completed + stats.tasks_active + stats.tasks_failed;
    if total == 0 {
        return 0.0;
    }
    (stats.tasks_completed as f64) / (total as f64)
}

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render a task list with a progress bar at the top.
///
/// ```text
/// ┌ Tasks ─────────────────────────────────────────┐
/// │ [========          ] 40% (4/10 done)            │
/// │                                                 │
/// │ [x] task-001  compose                           │
/// │ [x] task-002  completed                         │
/// │ [>] task-003  gate                              │
/// │ [ ] task-004  pending                           │
/// └─────────────────────────────────────────────────┘
/// ```
pub fn render_task_progress(
    frame: &mut Frame<'_>,
    area: Rect,
    tasks: &[TaskState],
    stats: &SnapshotStats,
    theme: &Theme,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.muted())
        .title(Span::styled("Tasks", theme.accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if inner.height < 2 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    // --- Progress gauge ---
    let ratio = overall_ratio(stats);
    let total = stats.tasks_completed + stats.tasks_active + stats.tasks_failed;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pct = (ratio * 100.0).round() as u16;
    let label = format!("{pct}% ({}/{} done)", stats.tasks_completed, total);
    let gauge = Gauge::default()
        .ratio(ratio.min(1.0))
        .label(label)
        .gauge_style(theme.success())
        .use_unicode(true);
    frame.render_widget(gauge, rows[0]);

    // --- Task list ---
    if tasks.is_empty() {
        let empty = Paragraph::new("No tasks").style(theme.muted());
        frame.render_widget(empty, rows[1]);
        return;
    }

    let items: Vec<ListItem<'_>> = tasks
        .iter()
        .map(|task| {
            let (icon, style) = task_decoration(task, theme);
            let line = Line::from(vec![
                Span::styled(format!("{icon} "), style),
                Span::styled(&task.task_id, theme.text()),
                Span::styled(format!("  {}", task.phase), theme.muted()),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, rows[1]);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_tasks() -> Vec<TaskState> {
        vec![
            TaskState {
                task_id: "t-001".into(),
                plan_id: "p1".into(),
                phase: "completed".into(),
                outcome: Some("success".into()),
            },
            TaskState {
                task_id: "t-002".into(),
                plan_id: "p1".into(),
                phase: "gate".into(),
                outcome: None,
            },
            TaskState {
                task_id: "t-003".into(),
                plan_id: "p1".into(),
                phase: "completed".into(),
                outcome: Some("failed: compile error".into()),
            },
        ]
    }

    fn sample_stats() -> SnapshotStats {
        SnapshotStats {
            tasks_completed: 1,
            tasks_active: 1,
            tasks_failed: 1,
            ..Default::default()
        }
    }

    #[test]
    fn task_progress_renders_without_panic() {
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        let tasks = sample_tasks();
        let stats = sample_stats();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_task_progress(frame, area, &tasks, &stats, &theme);
            })
            .unwrap();
    }

    #[test]
    fn task_progress_empty() {
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        let stats = SnapshotStats::default();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_task_progress(frame, area, &[], &stats, &theme);
            })
            .unwrap();
    }
}
