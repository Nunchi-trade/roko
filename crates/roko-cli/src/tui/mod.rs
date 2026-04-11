//! TUI/dashboard support for the CLI.
//!
//! The dashboard snapshot and scaffold types stay in place for text-mode and
//! API consumers. The interactive terminal shell lives in `app`, `event`,
//! `pages`, and `widgets`.

pub mod app;
pub mod atmosphere;
pub mod bars;
pub mod color;
pub mod dashboard;
pub mod event;
pub mod layout;
pub mod modals;
pub mod mori_atmosphere;
pub mod mori_theme;
pub mod pages;
pub mod postfx;
pub mod tabs;
pub mod theme;
pub mod tui_state;
pub mod views;
pub mod widgets;

pub use app::{App, run_async};
pub use bars::{GradientBar, SemanticBar};
pub use dashboard::{DashboardData, DashboardScaffold, DashboardSummary, Theme};
pub use event::{Event, EventHandler, TuiAction};
pub use layout::RootLayout;
pub use pages::{Page, PageId, PageRegistry, PageScaffold, WidgetScaffold};
pub use tabs::Tab;
pub use theme::{RosedustTheme, active_theme};
