//! TUI/dashboard support for the CLI.
//!
//! The dashboard snapshot and scaffold types stay in place for text-mode and
//! API consumers. The interactive terminal shell lives in `app`, `event`,
//! `pages`, and `widgets`. The Mori-style modal+focus+tab system is in
//! `input`, `state`, and `tabs`.

pub mod ansi;
pub mod app;
pub mod approval_ipc;
pub mod atmosphere;
pub mod config_meta;
pub mod dashboard;
pub mod effects_config;
pub mod event;
pub mod hit_test;
pub mod input;
pub mod layout;
pub mod modals;
pub mod pages;
pub mod postfx;
pub mod postfx_pipeline;
pub mod scroll;
pub mod segment;
pub mod state;
pub mod tabs;
pub mod util;
pub mod views;
pub mod widgets;

pub use app::App;
pub use approval_ipc::{ApprovalChannel, ApprovalRequest};
pub use atmosphere::Atmosphere;
pub use dashboard::{DashboardData, DashboardScaffold, DashboardSummary, Theme};
pub use effects_config::EffectsConfig;
pub use event::{Event, EventHandler};
pub use hit_test::HitZones;
pub use input::{ConfirmAction, FocusZone, InputMode, TuiAction};
pub use layout::{centered_rect, responsive_outer_margin};
pub use modals::{ModalState, Notification, NotificationLevel, render_modals};
pub use pages::{Page, PageId, PageRegistry, PageScaffold, WidgetScaffold};
pub use scroll::ScrollAccel;
pub use state::TuiState;
pub use tabs::Tab;
