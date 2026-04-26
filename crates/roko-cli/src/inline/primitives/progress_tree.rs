//! Primitive 16: `ProgressTree` — hierarchical plan progress.
//!
//! ```text
//! ◆ plan  deploy-audit  ·  8 tasks  ·  3 waves
//! ├ wave 1  ━━━━━━━━━━ 3/3 ✔
//! │ ├ T01 dependency-scan    ✔  $0.012  2.1s
//! │ ├ T02 secret-scan        ✔  $0.008  1.4s
//! │ └ T03 policy-check       ✔  $0.031  4.2s
//! ├ wave 2  ━━━━━━░░░░ 1/3
//! │ ├ T04 integration-test   ━━━━━━ running (6.2s)
//! │ ├ T05 diff-review        ⏳ blocked by T04
//! │ └ T06 cost-analysis      ⏳ blocked by T04
//! └ wave 3  ⏳ 0/2
//!   ├ T07 episode-log        ⏳
//!   └ T08 chain-anchor       ⏳
//! ```

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::Theme;

use super::super::symbols;

/// Status of a task in the progress tree.
#[derive(Debug, Clone)]
pub enum TaskProgress {
    /// Not yet started.
    Pending,
    /// Blocked by another task.
    Blocked { blocked_by: String },
    /// Currently running.
    Running { elapsed_s: f64 },
    /// Completed successfully.
    Done { cost_usd: f64, duration_s: f64 },
    /// Failed.
    Failed { reason: String },
    /// Skipped.
    Skipped,
}

/// A single task in the tree.
#[derive(Debug, Clone)]
pub struct TreeTask {
    /// Task ID (e.g. "T01").
    pub id: String,
    /// Task title.
    pub title: String,
    /// Current status.
    pub status: TaskProgress,
}

/// A wave (group of tasks that can run in parallel).
#[derive(Debug, Clone)]
pub struct TreeWave {
    /// Wave number (1-indexed).
    pub number: u32,
    /// Tasks in this wave.
    pub tasks: Vec<TreeTask>,
}

impl TreeWave {
    /// Count of completed tasks.
    pub fn done_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| matches!(t.status, TaskProgress::Done { .. }))
            .count()
    }

    /// Whether all tasks are done.
    pub fn is_complete(&self) -> bool {
        self.done_count() == self.tasks.len()
    }

    /// Progress fraction (0.0..=1.0).
    pub fn progress(&self) -> f64 {
        if self.tasks.is_empty() {
            return 0.0;
        }
        self.done_count() as f64 / self.tasks.len() as f64
    }
}

/// Full plan progress tree.
#[derive(Debug, Clone)]
pub struct ProgressTreeData {
    /// Plan name/title.
    pub plan_name: String,
    /// Waves in execution order.
    pub waves: Vec<TreeWave>,
}

impl ProgressTreeData {
    /// Total task count.
    pub fn total_tasks(&self) -> usize {
        self.waves.iter().map(|w| w.tasks.len()).sum()
    }

    /// Total done count.
    pub fn done_tasks(&self) -> usize {
        self.waves.iter().map(|w| w.done_count()).sum()
    }

    /// Render as styled lines.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let total = self.total_tasks();
        let wave_count = self.waves.len();

        // Header
        lines.push(Line::from(vec![
            Span::styled(symbols::START.to_string(), theme.accent()),
            Span::raw(" "),
            Span::styled(
                "plan".to_string(),
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(self.plan_name.clone(), theme.text()),
            Span::styled(format!("  {}  ", symbols::SEP), theme.muted()),
            Span::styled(
                format!("{total} tasks"),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::styled(format!("  {}  ", symbols::SEP), theme.muted()),
            Span::styled(
                format!("{wave_count} wave{}", if wave_count == 1 { "" } else { "s" }),
                Style::default().fg(Theme::TEXT_DIM),
            ),
        ]));

        for (wi, wave) in self.waves.iter().enumerate() {
            let is_last_wave = wi == self.waves.len() - 1;
            let wave_connector = if is_last_wave { symbols::END } else { symbols::BRANCH };
            let done = wave.done_count();
            let total_in_wave = wave.tasks.len();
            let progress = wave.progress();

            // Wave header
            let bar = symbols::progress_bar(progress, 10);
            let status_suffix = if wave.is_complete() {
                format!(" {}", symbols::PASS)
            } else {
                String::new()
            };

            lines.push(Line::from(vec![
                Span::styled(wave_connector.to_string(), theme.muted()),
                Span::raw(" "),
                Span::styled(
                    format!("wave {}", wave.number),
                    if wave.is_complete() {
                        theme.success()
                    } else if done > 0 {
                        theme.accent()
                    } else {
                        theme.muted()
                    },
                ),
                Span::raw("  "),
                Span::styled(bar, theme.accent()),
                Span::raw(" "),
                Span::styled(
                    format!("{done}/{total_in_wave}{status_suffix}"),
                    theme.text(),
                ),
            ]));

            // Tasks under this wave
            let indent_prefix = if is_last_wave { "  " } else { &format!("{} ", symbols::BAR) };
            for (ti, task) in wave.tasks.iter().enumerate() {
                let is_last_task = ti == wave.tasks.len() - 1;
                let task_connector = if is_last_task { symbols::END } else { symbols::BRANCH };
                lines.push(render_task(theme, indent_prefix, task_connector, task));
            }
        }

        lines
    }
}

fn render_task(
    theme: &Theme,
    indent: &str,
    connector: &str,
    task: &TreeTask,
) -> Line<'static> {
    let id_width = 4;
    let title_width = 20;

    let mut spans = vec![
        Span::styled(indent.to_string(), theme.muted()),
        Span::styled(connector.to_string(), theme.muted()),
        Span::raw(" "),
        Span::styled(
            format!("{:<w$}", task.id, w = id_width),
            Style::default().fg(Theme::TEXT_DIM),
        ),
        Span::styled(
            format!("{:<w$}", task.title, w = title_width),
            theme.text(),
        ),
    ];

    match &task.status {
        TaskProgress::Pending => {
            spans.push(Span::styled(
                format!("{} pending", symbols::PENDING),
                theme.muted(),
            ));
        }
        TaskProgress::Blocked { blocked_by } => {
            spans.push(Span::styled(
                format!("{} blocked by {blocked_by}", symbols::PENDING),
                theme.muted(),
            ));
        }
        TaskProgress::Running { elapsed_s } => {
            spans.push(Span::styled(
                format!("{} running ({elapsed_s:.1}s)", symbols::progress_bar(0.5, 6)),
                theme.accent(),
            ));
        }
        TaskProgress::Done { cost_usd, duration_s } => {
            spans.push(Span::styled(symbols::PASS.to_string(), theme.success()));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("${cost_usd:.3}  {duration_s:.1}s"),
                Style::default().fg(Theme::TEXT_DIM),
            ));
        }
        TaskProgress::Failed { reason } => {
            spans.push(Span::styled(symbols::FAIL.to_string(), theme.danger()));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(reason.clone(), theme.danger()));
        }
        TaskProgress::Skipped => {
            spans.push(Span::styled("skipped".to_string(), theme.muted()));
        }
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_tree_renders() {
        let theme = Theme::dark();
        let data = ProgressTreeData {
            plan_name: "deploy-audit".into(),
            waves: vec![
                TreeWave {
                    number: 1,
                    tasks: vec![
                        TreeTask { id: "T01".into(), title: "dep-scan".into(), status: TaskProgress::Done { cost_usd: 0.012, duration_s: 2.1 } },
                        TreeTask { id: "T02".into(), title: "secret-scan".into(), status: TaskProgress::Done { cost_usd: 0.008, duration_s: 1.4 } },
                    ],
                },
                TreeWave {
                    number: 2,
                    tasks: vec![
                        TreeTask { id: "T03".into(), title: "integration".into(), status: TaskProgress::Running { elapsed_s: 6.2 } },
                        TreeTask { id: "T04".into(), title: "diff-review".into(), status: TaskProgress::Blocked { blocked_by: "T03".into() } },
                    ],
                },
                TreeWave {
                    number: 3,
                    tasks: vec![
                        TreeTask { id: "T05".into(), title: "chain-anchor".into(), status: TaskProgress::Pending },
                    ],
                },
            ],
        };
        let lines = data.to_lines(&theme);
        // header + 3 wave headers + 5 tasks = 9
        assert_eq!(lines.len(), 9);
        assert_eq!(data.total_tasks(), 5);
        assert_eq!(data.done_tasks(), 2);
    }

    #[test]
    fn wave_progress() {
        let wave = TreeWave {
            number: 1,
            tasks: vec![
                TreeTask { id: "T01".into(), title: "a".into(), status: TaskProgress::Done { cost_usd: 0.01, duration_s: 1.0 } },
                TreeTask { id: "T02".into(), title: "b".into(), status: TaskProgress::Pending },
            ],
        };
        assert_eq!(wave.done_count(), 1);
        assert!((wave.progress() - 0.5).abs() < f64::EPSILON);
        assert!(!wave.is_complete());
    }
}
