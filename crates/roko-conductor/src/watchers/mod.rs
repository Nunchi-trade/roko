//! Conductor watchers — each implements [`Policy`] and scans the signal
//! stream for a specific anomaly pattern.
//!
//! Watchers are **pure functions**: they examine `&[Engram]` and emit
//! intervention signals. They have no side effects and hold no mutable
//! state beyond their configuration thresholds.

pub mod compile_fail_repeat;
pub mod context_window_pressure;
pub mod cost_overrun;
pub mod ghost_turn;
pub mod iteration_loop;
pub mod review_loop;
pub mod spec_drift;
pub mod stuck_pattern;
pub mod test_failure_budget;
pub mod time_overrun;

pub use compile_fail_repeat::CompileFailRepeatWatcher;
pub use context_window_pressure::ContextWindowPressureWatcher;
pub use cost_overrun::CostOverrunWatcher;
pub use ghost_turn::GhostTurnWatcher;
pub use iteration_loop::IterationLoopWatcher;
pub use review_loop::ReviewLoopWatcher;
pub use spec_drift::SpecDriftWatcher;
pub use stuck_pattern::StuckPatternWatcher;
pub use test_failure_budget::TestFailureBudgetWatcher;
pub use time_overrun::TimeOverrunWatcher;
