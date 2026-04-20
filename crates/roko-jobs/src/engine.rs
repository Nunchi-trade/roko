//! Keeper / Operator engine. Ports `KeeperEngine` from `cli/jobs/engines.py`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use roko_custody::CustodyGuard;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time;
use tracing::{info, warn};

use crate::events::EventSubscriber;
use crate::status::JobStatusTracker;
use crate::strategy::KeeperStrategy;
use crate::submitter::ChainTxSubmitter;
use crate::types::{ChainEvent, JobConfig, JobDefinition, KeeperContext};

#[derive(Debug, Error)]
pub enum JobEngineError {
    #[error("missing dependency: {0}")]
    Missing(&'static str),
    #[error("submitter error: {0}")]
    Submitter(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Snapshot of a running engine's operational state. Mirrors the Python
/// `JobStatus` pydantic model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobStatus {
    pub job_id: String,
    pub agent_id: String,
    pub running: bool,
    pub category: String,
    pub ticks_processed: u64,
    pub last_heartbeat_block: u64,
    pub accumulated_reward_eth: f64,
    pub events_received: u64,
    pub txs_submitted: u64,
    pub txs_succeeded: u64,
    pub txs_failed: u64,
    pub uptime_seconds: f64,
    pub error: Option<String>,
}

#[async_trait]
pub trait JobEngine: Send + Sync {
    async fn run(&mut self) -> Result<(), JobEngineError>;
    fn status(&self) -> JobStatus;
}

/// Engine for KEEPER / OPERATOR jobs — event-driven loop with custody guard,
/// tx submission, status persistence, and a periodic heartbeat tick.
pub struct KeeperEngine {
    job_def: JobDefinition,
    config: JobConfig,
    strategy: Box<dyn KeeperStrategy>,
    subscriber: Box<dyn EventSubscriber>,
    submitter: Arc<dyn ChainTxSubmitter>,
    custody: CustodyGuard,
    tracker: JobStatusTracker,
    status: JobStatus,
    started_at: Option<Instant>,
}

impl KeeperEngine {
    #[must_use]
    pub fn new(
        job_def: JobDefinition,
        config: JobConfig,
        strategy: Box<dyn KeeperStrategy>,
        subscriber: Box<dyn EventSubscriber>,
        submitter: Arc<dyn ChainTxSubmitter>,
    ) -> Self {
        let tracker = JobStatusTracker::new(&job_def.job_id, &config.agent_id, &config.data_dir);
        let status = JobStatus {
            job_id: job_def.job_id.clone(),
            agent_id: config.agent_id.clone(),
            running: false,
            category: job_def.category.as_str().into(),
            ..JobStatus::default()
        };
        let custody = CustodyGuard::new(job_def.custody.clone());
        Self {
            job_def,
            config,
            strategy,
            subscriber,
            submitter,
            custody,
            tracker,
            status,
            started_at: None,
        }
    }

    async fn process_event(&mut self, event: ChainEvent) {
        self.status.events_received += 1;
        self.status.last_heartbeat_block = event.block_number;
        self.custody.reset_rate_limit();

        let context = KeeperContext {
            event: event.clone(),
            chain_state: event.data.clone(),
            gas_price_gwei: 0.0,
            agent_balance_eth: 0.0,
        };

        let txs = self.strategy.should_execute(&event, &context);
        self.status.ticks_processed += 1;

        for tx in txs {
            let custody_tx: roko_custody::Transaction = (&tx).into();
            match self.custody.validate(&custody_tx) {
                Ok(()) => {}
                Err(err) => {
                    warn!(error = ?err, "custody violation");
                    self.status.txs_failed += 1;
                    continue;
                }
            }
            self.status.txs_submitted += 1;

            if self.config.dry_run {
                info!(to = %tx.to, value = tx.value_wei, "[DRY RUN] submitted tx");
                self.status.txs_succeeded += 1;
                continue;
            }

            match self.submitter.submit(&tx).await {
                Ok(receipt) => {
                    if receipt.status == 1 {
                        self.status.txs_succeeded += 1;
                        self.strategy.on_execution_result(&receipt.tx_hash, true, receipt.gas_used);
                    } else {
                        self.status.txs_failed += 1;
                        self.strategy.on_execution_result(&receipt.tx_hash, false, receipt.gas_used);
                    }
                }
                Err(err) => {
                    warn!(error = %err, "submission error");
                    self.status.txs_failed += 1;
                    self.strategy.on_execution_result("submission_failed", false, 0);
                }
            }
        }
    }

    fn heartbeat(&mut self) {
        self.tracker.record_heartbeat(self.status.last_heartbeat_block, true, None);
        if let Err(err) = self.tracker.save() {
            warn!(error = %err, "heartbeat persistence failed");
        }
    }

    fn update_uptime(&mut self) {
        if let Some(start) = self.started_at {
            self.status.uptime_seconds = start.elapsed().as_secs_f64();
        }
    }

    /// Ignore custody violations — used by tests that want to see the engine
    /// dispatch transactions regardless of policy.
    #[cfg(test)]
    pub(crate) fn disable_custody(&mut self) {
        let permissive = roko_custody::CustodyPolicy {
            destinations: vec![],
            selectors: vec![],
            value_cap_eth: 0.0,
            rate_limit_per_block: u32::MAX,
        };
        self.custody = CustodyGuard::new(permissive);
    }
}

#[async_trait]
impl JobEngine for KeeperEngine {
    async fn run(&mut self) -> Result<(), JobEngineError> {
        self.started_at = Some(Instant::now());
        self.status.running = true;
        self.tracker.save()?;

        let interval = Duration::from_secs(self.config.heartbeat_interval_s.max(1));
        let mut heartbeat = time::interval(interval);
        heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        // Skip the immediate tick so tests don't double-heartbeat on startup.
        heartbeat.tick().await;

        info!(job = %self.job_def.job_id, "KeeperEngine starting");

        loop {
            tokio::select! {
                maybe_event = self.subscriber.next() => {
                    match maybe_event {
                        Some(event) => self.process_event(event).await,
                        None => break,
                    }
                }
                _ = heartbeat.tick() => {
                    self.heartbeat();
                }
            }
        }

        self.update_uptime();
        self.status.running = false;
        self.tracker.save()?;
        info!(job = %self.job_def.job_id, ticks = self.status.ticks_processed,
              txs_ok = self.status.txs_succeeded, txs_fail = self.status.txs_failed,
              uptime_s = %self.status.uptime_seconds, "KeeperEngine stopped");
        Ok(())
    }

    fn status(&self) -> JobStatus {
        let mut snap = self.status.clone();
        if let Some(start) = self.started_at {
            snap.uptime_seconds = start.elapsed().as_secs_f64();
        }
        snap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ChannelSubscriber;
    use crate::registry::get_job;
    use crate::submitter::MockSubmitter;
    use crate::types::{ChainEvent, Transaction};

    struct DummyStrategy;

    impl KeeperStrategy for DummyStrategy {
        fn should_execute(&self, event: &ChainEvent, _context: &KeeperContext) -> Vec<Transaction> {
            if event.event_type == "NewBlock" {
                vec![Transaction {
                    to: "0x0000000000000000000000000000000000001111".into(),
                    data: "0x9045975b".into(),
                    value_wei: 0,
                    gas_limit: 0,
                    chain_id: 0,
                }]
            } else {
                vec![]
            }
        }
    }

    #[tokio::test]
    async fn keeper_engine_processes_events_and_submits() {
        let dir = tempfile::tempdir().unwrap();
        let job_def = get_job("oracle_updater").unwrap().clone();
        let config = JobConfig {
            job_id: "oracle_updater".into(),
            agent_id: "agent-test".into(),
            data_dir: dir.path().to_string_lossy().into_owned(),
            heartbeat_interval_s: 3600,
            ..JobConfig::default()
        };
        let (mut sub, tx) = ChannelSubscriber::pair(vec!["NewBlock".into()], 4);
        let submitter = Arc::new(MockSubmitter::new());
        let mut engine = KeeperEngine::new(
            job_def,
            config,
            Box::new(DummyStrategy),
            Box::new(sub),
            submitter.clone(),
        );
        engine.disable_custody();

        let handle = tokio::spawn(async move {
            let _ = engine.run().await;
            engine.status()
        });

        tx.send(ChainEvent {
            event_type: "NewBlock".into(),
            block_number: 42,
            ..ChainEvent::default_zero()
        })
        .await
        .unwrap();
        drop(tx);

        let status = handle.await.unwrap();
        assert_eq!(status.events_received, 1);
        assert_eq!(status.txs_submitted, 1);
        assert_eq!(status.txs_succeeded, 1);
        assert_eq!(submitter.recorded().len(), 1);
    }
}

impl ChainEvent {
    /// Convenience for tests — default doesn't exist on the serde-derived
    /// struct because `event_type` is required.
    #[cfg(test)]
    pub(crate) fn default_zero() -> Self {
        Self {
            event_type: String::new(),
            block_number: 0,
            tx_hash: None,
            data: Default::default(),
            timestamp_ms: 0,
        }
    }
}
