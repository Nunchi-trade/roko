//! Mori-accurate dashboard view — the primary execution view.
//!
//! Ported from Mori's `views/dashboard.rs`. Left panel (38%): plan tree +
//! phase compact + task progress. Right panel (62%): sub-tab bar + routed
//! content (agents/output/diff/errors/git).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::mori_theme::MoriTheme;
use super::super::tui_state::{DetailSubTab, FocusZone, TuiState};
use super::super::widgets;

/// Render the Mori-accurate dashboard view into the content area.
pub fn render(f: &mut Frame, area: Rect, state: &TuiState) {
    let has_waves = !state.execution_waves.is_empty();

    // Optional wave progress ribbon on top, then master-detail
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if has_waves {
            vec![Constraint::Length(1), Constraint::Min(0)]
        } else {
            vec![Constraint::Length(0), Constraint::Min(0)]
        })
        .split(area);

    if has_waves {
        widgets::wave_progress::render_wave_progress(f, root[0], state);
    }

    // Left panel 38% | 1-col spacer | Right panel flex
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(38),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(root[1]);

    render_left_panel(f, main[0], state);
    // main[1] is VOID spacer
    render_right_panel(f, main[2], state);
}

// ── Left panel: plan tree + phase compact + tasks ────────────────────────

fn render_left_panel(f: &mut Frame, area: Rect, state: &TuiState) {
    let plan_focused = state.focus == FocusZone::PlanTree;
    let task_focused = state.focus == FocusZone::TaskProgress;

    let task_count = state.current_task_checklist.len();
    let phase_height = 4u16;
    let available = area.height.saturating_sub(phase_height);

    let plan_count = state.plans.len() as u16;
    let wave_count = state.execution_waves.len() as u16;
    let plan_content = plan_count + wave_count + 4;
    let task_content = if task_count > 0 {
        task_count as u16 + 4
    } else {
        3
    };

    let total_content = plan_content + task_content;

    let (plan_height, task_height) = if available == 0 {
        (0, 0)
    } else if available == 1 {
        (1, 0)
    } else {
        let desired_min_plan = 10u16;
        let desired_min_task = if task_count > 0 {
            if task_focused { 11 } else { 8 }
        } else {
            4
        };
        let min_task = desired_min_task.min(available.saturating_sub(1));
        let min_plan = desired_min_plan
            .min(available.saturating_sub(min_task).max(1));

        if total_content <= available {
            let preferred_task = if task_focused {
                available.saturating_mul(3) / 5
            } else {
                available.saturating_mul(11) / 20
            };
            let max_task = available
                .saturating_sub(min_plan)
                .min(preferred_task.max(min_task))
                .max(min_task);
            let desired_task = if task_count > 0 {
                (task_count as u16 + 5).min(max_task)
            } else {
                min_task
            };
            let task_h = desired_task.clamp(min_task, max_task);
            let plan_h = available.saturating_sub(task_h);
            (plan_h, task_h)
        } else {
            let plan_ratio = plan_content as f64 / total_content.max(1) as f64;
            let desired_plan = (available as f64 * plan_ratio).round() as u16;
            let max_plan = available.saturating_sub(min_task);
            let min_plan_bound = min_plan.min(max_plan);
            let plan_h = desired_plan.clamp(min_plan_bound, max_plan);
            let task_h = available.saturating_sub(plan_h);
            (plan_h, task_h)
        }
    };

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(plan_height),
            Constraint::Length(phase_height),
            Constraint::Length(task_height),
        ])
        .split(area);

    widgets::plan_tree::render_plan_tree(f, left[0], state, plan_focused);
    widgets::phase_compact::render_phase_compact(f, left[1], state, state.focus == FocusZone::PhaseCompact);
    widgets::task_progress::render_task_progress(f, left[2], state, task_focused);
}

// ── Right panel: sub-tab bar + routed content ────────────────────────────

fn render_right_panel(f: &mut Frame, area: Rect, state: &TuiState) {
    let output_focused = state.focus == FocusZone::AgentOutput;

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // sub-tab bar
            Constraint::Min(0),   // content
        ])
        .split(area);

    render_sub_tab_bar(f, right[0], state);

    match state.detail_sub_tab {
        DetailSubTab::Agents => {
            render_agents_content(f, right[1], state, output_focused);
        }
        DetailSubTab::Output => {
            widgets::agent_output::render_agent_output(f, right[1], state, output_focused);
        }
        DetailSubTab::Diff => {
            // Diff view — placeholder, show gate output as fallback
            widgets::command_output::render_command_output(
                f,
                right[1],
                state,
                state.focus == FocusZone::CommandOutput,
            );
        }
        DetailSubTab::Errors => {
            let theme = super::super::dashboard::Theme::dark();
            widgets::error_digest::render_error_digest(
                f,
                right[1],
                &state.live.gates,
                &state.live.errors,
                &state.live.stats,
                &theme,
            );
        }
        DetailSubTab::Git => {
            // Git view — minimal placeholder
            let text = format!(
                " Branch: {}\n Commit: {}\n Age: {}",
                state.git_branch, state.git_commit_short, state.git_age,
            );
            let p = Paragraph::new(text).style(Style::default().fg(MoriTheme::TEXT));
            f.render_widget(p, right[1]);
        }
    }
}

fn render_sub_tab_bar(f: &mut Frame, area: Rect, state: &TuiState) {
    let tabs: [(DetailSubTab, &str, ratatui::style::Color); 5] = [
        (DetailSubTab::Agents, "a:Agents", MoriTheme::ROSE),
        (DetailSubTab::Output, "o:Output", MoriTheme::BONE_DIM),
        (DetailSubTab::Diff, "d:Diff", MoriTheme::SAGE),
        (DetailSubTab::Errors, "e:Errors", MoriTheme::EMBER),
        (DetailSubTab::Git, "g:Git", MoriTheme::DREAM),
    ];

    let bg = MoriTheme::BG_RAISED;

    let mut spans = vec![Span::styled(" ", Style::default().bg(bg))];

    for (tab, label, accent) in &tabs {
        let style = if *tab == state.detail_sub_tab {
            Style::default()
                .fg(MoriTheme::VOID)
                .bg(*accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(MoriTheme::FG_DIM).bg(bg)
        };
        spans.push(Span::styled(format!(" {label} "), style));

        // Content-aware badges on inactive tabs
        if *tab != state.detail_sub_tab {
            let badge = match tab {
                DetailSubTab::Agents => {
                    let active = state.active_agent_count();
                    if active > 0 {
                        Some((format!("{active}\u{25B8}"), MoriTheme::ROSE_DIM))
                    } else {
                        None
                    }
                }
                DetailSubTab::Errors => {
                    let count = state.live.errors.len();
                    if count > 0 {
                        Some((format!("{count}\u{2717}"), MoriTheme::EMBER))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some((text, color)) = badge {
                spans.push(Span::styled(
                    text,
                    Style::default()
                        .fg(color)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        }

        spans.push(Span::styled(" ", Style::default().bg(bg)));
    }

    // Right-aligned hint
    let hint = " F2:plans F3:agents ";
    let used: usize = spans.iter().map(|s| s.content.len()).sum();
    let hint_len = hint.len();
    let gap = (area.width as usize).saturating_sub(used + hint_len);
    spans.push(Span::styled(" ".repeat(gap), Style::default().bg(bg)));
    spans.push(Span::styled(
        hint,
        Style::default().fg(MoriTheme::TEXT_GHOST).bg(bg),
    ));

    let line = Line::from(spans);
    f.render_widget(Paragraph::new(line), area);
}

/// Agents content: agent pool + agent output + bottom strip (token burn | sys metrics).
fn render_agents_content(
    f: &mut Frame,
    area: Rect,
    state: &TuiState,
    output_focused: bool,
) {
    let active_agent_count = state
        .agents
        .iter()
        .filter(|a| a.active || a.input_tokens > 0)
        .count();
    let agent_pool_height = (active_agent_count.max(1) as u16 + 2).min(6);

    let has_burn_data = !state.token_history.is_empty();
    let active_burn_agents = state.token_history.len() as u16;
    let sparkline_height = if has_burn_data {
        (4 + active_burn_agents).min(8)
    } else {
        5
    };
    let bottom_height = sparkline_height.max(9);

    let show_cmd_output = !state.gate_results.is_empty();

    if show_cmd_output {
        let gate_height = 5u16.min(area.height * 2 / 5).max(5);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(agent_pool_height),
                Constraint::Min(8),
                Constraint::Length(gate_height),
                Constraint::Length(bottom_height),
            ])
            .split(area);

        widgets::agent_pool::render_agent_pool(f, layout[0], state, false);
        widgets::agent_output::render_agent_output(f, layout[1], state, output_focused);
        let cmd_focused = state.focus == FocusZone::CommandOutput;
        widgets::command_output::render_command_output(f, layout[2], state, cmd_focused);
        render_bottom_strip(f, layout[3], state, has_burn_data);
    } else {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(agent_pool_height),
                Constraint::Min(0),
                Constraint::Length(bottom_height),
            ])
            .split(area);

        widgets::agent_pool::render_agent_pool(f, layout[0], state, false);
        widgets::agent_output::render_agent_output(f, layout[1], state, output_focused);
        render_bottom_strip(f, layout[2], state, has_burn_data);
    }
}

/// Token burn (left 50%) | System metrics (right 50%) side by side.
fn render_bottom_strip(
    f: &mut Frame,
    area: Rect,
    state: &TuiState,
    has_burn_data: bool,
) {
    if has_burn_data {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        widgets::token_sparkline::render_token_sparkline(f, cols[0], state);
        widgets::sys_metrics::render_sys_metrics(f, cols[1], state);
    } else {
        widgets::sys_metrics::render_sys_metrics(f, area, state);
    }
}
