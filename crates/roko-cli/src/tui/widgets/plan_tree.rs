//! Collapsible plan tree widget with inline progress indicators.

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use roko_core::dashboard_snapshot::PlanState;

use super::super::dashboard::Theme;

// ---------------------------------------------------------------------------
// Status icons
// ---------------------------------------------------------------------------

fn status_icon(plan: &PlanState, theme: &Theme) -> (&'static str, ratatui::style::Style) {
    if !plan.active && plan.phase == "completed" {
        ("\u{2714}", theme.success()) // checkmark
    } else if !plan.active && plan.phase == "failed" {
        ("\u{2718}", theme.danger()) // X mark
    } else if plan.active {
        ("\u{25b6}", theme.warning()) // play triangle
    } else {
        ("\u{25cb}", theme.muted()) // circle
    }
}

/// Compute a progress fraction for display.
fn progress_pct(plan: &PlanState) -> u8 {
    if plan.tasks_total == 0 {
        return 0;
    }
    let ratio = (plan.tasks_done as f64) / (plan.tasks_total as f64);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pct = (ratio * 100.0).round() as u8;
    pct.min(100)
}

/// Build a tiny inline bar like `[=====     ]`.
fn inline_bar(pct: u8, width: usize) -> String {
    let filled = ((pct as usize) * width) / 100;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "=".repeat(filled), " ".repeat(empty))
}

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the plan tree.
///
/// Each row looks like:
/// ```text
/// [icon] plan_id — phase — 3/5 tasks  [====      ] 60%
/// ```
///
/// The `selected` index controls which row gets the selection highlight.
pub fn render_plan_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    plans: &[PlanState],
    selected: usize,
    theme: &Theme,
) {
    if plans.is_empty() {
        let empty = Paragraph::new("No plans")
            .style(theme.muted())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.muted())
                    .title(Span::styled("Plans", theme.accent())),
            );
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem<'_>> = plans
        .iter()
        .enumerate()
        .map(|(i, plan)| {
            let (icon, icon_style) = status_icon(plan, theme);
            let pct = progress_pct(plan);
            let bar = inline_bar(pct, 10);

            let base_style = if i == selected {
                theme.selection()
            } else {
                theme.text()
            };

            let line = Line::from(vec![
                Span::styled(format!("{icon} "), icon_style),
                Span::styled(&plan.plan_id, base_style.add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" \u{2014} {} \u{2014} ", plan.phase),
                    if i == selected {
                        theme.selection()
                    } else {
                        theme.muted()
                    },
                ),
                Span::styled(
                    format!("{}/{} tasks  ", plan.tasks_done, plan.tasks_total),
                    base_style,
                ),
                Span::styled(bar, icon_style),
                Span::styled(format!(" {pct}%"), icon_style),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.muted())
            .title(Span::styled("Plans", theme.accent())),
    );

    frame.render_widget(list, area);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_plans() -> Vec<PlanState> {
        vec![
            PlanState {
                plan_id: "plan-alpha".into(),
                phase: "execute".into(),
                tasks_total: 5,
                tasks_done: 3,
                tasks_failed: 0,
                active: true,
            },
            PlanState {
                plan_id: "plan-beta".into(),
                phase: "completed".into(),
                tasks_total: 4,
                tasks_done: 4,
                tasks_failed: 0,
                active: false,
            },
            PlanState {
                plan_id: "plan-gamma".into(),
                phase: "failed".into(),
                tasks_total: 3,
                tasks_done: 1,
                tasks_failed: 2,
                active: false,
            },
        ]
    }

    #[test]
    fn plan_tree_renders_without_panic() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        let plans = sample_plans();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_plan_tree(frame, area, &plans, 0, &theme);
            })
            .unwrap();
    }

    #[test]
    fn plan_tree_empty() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_plan_tree(frame, area, &[], 0, &theme);
            })
            .unwrap();
    }

    #[test]
    fn progress_pct_edge_cases() {
        assert_eq!(
            progress_pct(&PlanState {
                tasks_total: 0,
                ..Default::default()
            }),
            0
        );
        assert_eq!(
            progress_pct(&PlanState {
                tasks_total: 10,
                tasks_done: 10,
                ..Default::default()
            }),
            100
        );
    }
}
