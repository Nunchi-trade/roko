//! TUI/dashboard scaffold types.
//!
//! This module is intentionally scaffold-only. It defines placeholder data
//! structures for the future interactive dashboard without adding rendering
//! dependencies or command routing.

pub mod dashboard;
pub mod pages;

pub use dashboard::{DashboardScaffold, DashboardSummary};
pub use pages::{PageId, PageScaffold, WidgetScaffold};
