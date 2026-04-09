//! TUI/dashboard support for the CLI.
//!
//! The dashboard snapshot and scaffold types stay in place for text-mode and
//! API consumers. The interactive terminal shell lives in `app`, `event`,
//! `pages`, and `widgets`.

pub mod app;
pub mod event;
pub mod pages;
pub mod widgets;
pub mod dashboard;

pub use app::App;
pub use event::{Event, EventHandler};
pub use dashboard::{DashboardData, DashboardScaffold, DashboardSummary};
pub use pages::{Page, PageId, PageRegistry, PageScaffold, WidgetScaffold};
