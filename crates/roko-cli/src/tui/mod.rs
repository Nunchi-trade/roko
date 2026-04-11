//! TUI/dashboard support for the CLI.
//!
//! The dashboard snapshot and scaffold types stay in place for text-mode and
//! API consumers. The interactive terminal shell lives in `app`, `event`,
//! `pages`, and `widgets`.

pub mod app;
pub mod dashboard;
pub mod event;
pub mod pages;
pub mod widgets;

pub use app::{App, run_async};
pub use dashboard::{DashboardData, DashboardScaffold, DashboardSummary, Theme};
pub use event::{Event, EventHandler, TuiAction};
pub use pages::{Page, PageId, PageRegistry, PageScaffold, WidgetScaffold};
