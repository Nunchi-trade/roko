//! Perpetual-Agent-Jobs runtime — keeper / operator / cooperative / managed.
//!
//! Ports `Nunchi-trade/offchainservices-agent`'s `cli/jobs/` module and the
//! full spec at `contracts-core/docs/agent_cli_jobs_spec.tex`.
//!
//! What's here:
//! - [`JobCategory`] + [`TriggerType`] enums matching the Python values.
//! - [`JobDefinition`] + [`registry`] with every pre-registered job.
//! - [`KeeperStrategy`] trait + [`ChainEvent`] + [`KeeperContext`] +
//!   [`Transaction`] — the contract strategies implement.
//! - [`KeeperEngine`] — async event loop with custody guard, tx submission,
//!   status persistence, heartbeat ticks.
//! - [`JobStatusTracker`] — JSON heartbeat + reward persistence.
//! - [`EventSubscriber`] + [`ChainTxSubmitter`] traits — abstraction points
//!   so the engine runs against alloy / mocks / mirage-rs identically.
//!
//! See `plans/P08-trading-surface.md` T7.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

pub mod engine;
pub mod events;
pub mod registry;
pub mod status;
pub mod strategy;
pub mod submitter;
pub mod types;

pub use engine::{JobEngine, JobEngineError, JobStatus, KeeperEngine};
pub use events::{ChannelSubscriber, EventSubscriber};
pub use registry::{get_job, job_registry, list_jobs, list_jobs_by_category};
pub use status::{HeartbeatRecord, JobStatusTracker, RewardRecord};
pub use strategy::KeeperStrategy;
pub use submitter::{ChainTxSubmitter, MockSubmitter, TxReceipt};
pub use types::{
    ChainEvent, JobCategory, JobConfig, JobDefinition, KeeperContext, Transaction, TriggerType,
};

pub use roko_custody::{CustodyGuard, CustodyPolicy, CustodyViolation};
