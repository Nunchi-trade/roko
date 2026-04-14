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
pub use inject::render_inject;
pub use notification::{Notification, NotificationLevel, render_notifications};
pub use queue_overview::{Milestone, QueueTask, render_queue_overview};
pub use quit::render_quit;
pub use task_picker::{TaskPickerRow, render_task_picker};
pub use wave_overview::{WaveInfo, WavePlanEntry, render_wave_overview};

use ratatui::Frame;
use ratatui::layout::Rect;

use super::dashboard::Theme;

/// Which modal is currently active, if any.
///
/// The integration layer (app.rs / TuiState) stores one of these to indicate
/// the active modal. [`render_modal`] dispatches rendering based on the variant.
#[derive(Debug, Clone)]
pub enum ModalState {
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
pub fn render_modal(frame: &mut Frame<'_>, area: Rect, modal: &ModalState, theme: &Theme) {
    match modal {
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
    notifications: &[Notification],
    theme: &Theme,
) {
    if let Some(modal) = active_modal {
        render_modal(frame, area, modal, theme);
    }

    // Notifications always render (they stack in the bottom-right corner).
    if !notifications.is_empty() {
        render_notifications(frame, area, notifications, theme);
    }
}
