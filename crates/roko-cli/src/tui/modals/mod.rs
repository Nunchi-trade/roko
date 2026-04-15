//! Modal dialogs for the interactive TUI.
//!
//! Each modal is self-contained: it takes a `Frame`, an `area` (usually
//! `frame.area()`), the relevant state slice, and the `Theme`.
//!
//! The [`render_modal`] dispatch function matches on [`ModalState`] and routes
//! to the appropriate modal renderer.

pub mod agent_pool_modal;
pub mod approval;
pub mod batch_review;
pub mod confirm;
pub mod help;
pub mod inject;
pub mod notification;
pub mod plan_detail;
pub mod queue_overview;
pub mod quit;
pub mod task_detail;
pub mod task_picker;
pub mod wave_overview;

pub use agent_pool_modal::{AgentPoolRow, render_agent_pool};
pub use approval::render_approval;
pub use batch_review::{BatchTaskResult, render_batch_review};
pub use confirm::{ConfirmAction, render_confirm};
pub use help::render_help_modal;
pub use inject::render_inject;
pub use notification::{Notification, NotificationLevel, render_notifications};
pub use plan_detail::render_plan_detail_modal;
pub use queue_overview::{Milestone, QueueTask, render_queue_overview};
pub use quit::render_quit;
pub use task_detail::render_task_detail_modal;
pub use task_picker::{TaskPickerRow, render_task_picker};
pub use wave_overview::{WaveInfo, WavePlanEntry, render_wave_overview};

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use super::dashboard::{DashboardData, Theme};
use super::postfx;
use super::state::TuiState;

/// Which modal is currently active, if any.
///
/// The integration layer (app.rs / TuiState) stores one of these to indicate
/// the active modal. [`render_modal`] dispatches rendering based on the variant.
#[derive(Debug, Clone)]
pub enum ModalState {
    /// Global help.
    Help,

    /// Plan detail browser.
    PlanDetail { plan_id: String },

    /// Quit confirmation.
    Quit,

    /// Agent command approval.
    Approval { role: String, command: String },

    /// Destructive action confirmation.
    Confirm { action: ConfirmAction },

    /// Free-text injection to an agent.
    Inject {
        target_agent: String,
        input_text: String,
        cursor_pos: usize,
    },

    /// Wave progress overview.
    WaveOverview {
        waves: Vec<WaveInfo>,
        scroll_offset: u16,
    },

    /// Milestone queue browser.
    QueueOverview {
        milestones: Vec<Milestone>,
        selected_index: usize,
        scroll_offset: u16,
    },

    /// Full agent roster.
    AgentPool {
        agents: Vec<AgentPoolRow>,
        scroll_offset: u16,
    },

    /// Task picker.
    TaskPicker {
        tasks: Vec<TaskPickerRow>,
        selected_index: usize,
        scroll_offset: u16,
    },

    /// Task detail browser.
    TaskDetail {
        task_idx: usize,
        scroll_offset: usize,
    },

    /// Batch-pause review.
    BatchReview {
        batch_name: String,
        results: Vec<BatchTaskResult>,
        scroll_offset: u16,
    },
}

/// Render the currently active modal, if any.
///
/// Call this after rendering the main dashboard content so the modal
/// draws on top. Notifications are rendered independently since they can
/// coexist with other modals.
pub fn render_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    modal: &ModalState,
    tui_state: &TuiState,
    data: &DashboardData,
    theme: &Theme,
) {
    match modal {
        ModalState::Help => {
            render_help_modal(frame, area, theme);
        }
        ModalState::PlanDetail { plan_id } => {
            if let Some(plan) = tui_state.plans.iter().find(|plan| &plan.id == plan_id) {
                plan_detail::render_plan_detail_modal(
                    frame,
                    area,
                    plan,
                    tui_state.plan_detail_scroll as u16,
                    theme,
                );
            }
        }
        ModalState::Quit => {
            render_quit(frame, area, theme);
        }
        ModalState::Approval { role, command } => {
            render_approval(frame, area, role, command, theme);
        }
        ModalState::Confirm { action } => {
            render_confirm(frame, area, action, theme);
        }
        ModalState::Inject {
            target_agent,
            input_text,
            cursor_pos,
        } => {
            render_inject(frame, area, target_agent, input_text, *cursor_pos, theme);
        }
        ModalState::WaveOverview {
            waves,
            scroll_offset,
        } => {
            render_wave_overview(frame, area, waves, *scroll_offset, theme);
        }
        ModalState::QueueOverview {
            milestones,
            selected_index,
            scroll_offset,
        } => {
            render_queue_overview(
                frame,
                area,
                milestones,
                *selected_index,
                *scroll_offset,
                theme,
            );
        }
        ModalState::AgentPool {
            agents,
            scroll_offset,
        } => {
            render_agent_pool(frame, area, agents, *scroll_offset, theme);
        }
        ModalState::TaskPicker {
            tasks,
            selected_index,
            scroll_offset,
        } => {
            render_task_picker(frame, area, tasks, *selected_index, *scroll_offset, theme);
        }
        ModalState::TaskDetail {
            task_idx,
            scroll_offset,
        } => {
            if let Some(task) = tui_state.current_task_checklist.get(*task_idx) {
                let mut assigned_agents = data
                    .active_tasks
                    .iter()
                    .find(|active_task| active_task.task_id == task.id)
                    .map(|active_task| active_task.assigned_agents.clone())
                    .unwrap_or_default();

                if assigned_agents.is_empty() {
                    assigned_agents.extend(
                        tui_state
                            .agents
                            .iter()
                            .filter(|agent| agent.current_task == task.id)
                            .map(|agent| agent.id.clone()),
                    );
                    assigned_agents.sort();
                    assigned_agents.dedup();
                }

                let gate_results = data.gate_signals_for_task(&task.id);
                render_task_detail_modal(
                    frame,
                    area,
                    task,
                    &assigned_agents,
                    &gate_results,
                    *scroll_offset,
                    theme,
                );
            }
        }
        ModalState::BatchReview {
            batch_name,
            results,
            scroll_offset,
        } => {
            render_batch_review(frame, area, batch_name, results, *scroll_offset, theme);
        }
    }
}

/// Render all active modals and notifications.
///
/// This is the top-level entry point called from the draw loop.
/// It renders the active modal (if any) and then overlays notifications.
pub fn render_modals(
    frame: &mut Frame<'_>,
    area: Rect,
    active_modal: Option<&ModalState>,
    tui_state: &TuiState,
    data: &DashboardData,
    notifications: &[Notification],
    theme: &Theme,
    screen_postfx: bool,
) {
    if let Some(modal) = active_modal {
        if screen_postfx {
            let popup = modal_area(modal, area);
            postfx::modal_glow(popup, frame.buffer_mut(), area, theme.accent, 0.18);
        }
        render_modal(frame, area, modal, tui_state, data, theme);
    }

    // Notifications always render (they stack in the bottom-right corner).
    if !notifications.is_empty() {
        render_notifications(frame, area, notifications, theme);
    }
}

fn modal_area(modal: &ModalState, area: Rect) -> Rect {
    match modal {
        ModalState::Help => centered_rect(86, 84, area),
        ModalState::PlanDetail { .. } => centered_rect(86, 84, area),
        ModalState::Quit => centered_rect_fixed(42, 8, area),
        ModalState::Approval { .. } => centered_rect(60, 40, area),
        ModalState::Confirm { .. } => centered_rect(50, 30, area),
        ModalState::Inject { .. } => centered_rect(70, 20, area),
        ModalState::WaveOverview { .. } => centered_rect(80, 70, area),
        ModalState::QueueOverview { .. } => centered_rect(85, 75, area),
        ModalState::AgentPool { .. } => centered_rect(90, 70, area),
        ModalState::TaskPicker { .. } => centered_rect(80, 60, area),
        ModalState::TaskDetail { .. } => centered_rect(78, 72, area),
        ModalState::BatchReview { .. } => centered_rect(75, 65, area),
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn centered_rect_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
