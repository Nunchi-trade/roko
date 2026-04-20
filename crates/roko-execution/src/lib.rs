//! Execution helpers — ports of `execution/` from
//! `Nunchi-trade/offchainservices-agent`.
//!
//! See `plans/P08-trading-surface.md` T5.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

mod order_book;
mod order_types;
mod parent_order;
mod portfolio_risk;
mod routing;
mod twap;

pub use order_book::ManagedOrderBook;
pub use order_types::{BracketOrder, ConditionalOrder, ManagedOrder, PeggedOrder};
pub use parent_order::{ExecutionAlgo, ParentOrder};
pub use portfolio_risk::{
    PortfolioRiskConfig, PortfolioRiskManager, PortfolioRiskState, PositionSummary,
    CORRELATION_GROUPS,
};
pub use routing::{AloStats, OrderRouter};
pub use twap::{ChildSlice, TwapExecutor};
