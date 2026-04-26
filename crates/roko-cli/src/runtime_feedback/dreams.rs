//! Dream trigger sink — fires consolidation cycles when conditions are
//! met.
//!
//! ## Triggers
//!
//! Two natural triggers:
//!
//! 1. **Plan completion**. When a plan finishes (success or failure), the
//!    runner has accumulated a coherent batch of observations. That is
//!    the right moment to consolidate them.
//! 2. **Idle ticks**. When the runner has been idle for *N* consecutive
//!    ticks, the dream subsystem can run a lightweight reinforcement
//!    pass without competing for resources.
//!
//! ## How a "trigger" looks today
//!
//! `roko-dreams` exposes `DreamCycle::run` which is heavy and
//! synchronous. We don't want to block the runner's event loop on a
//! dream cycle. The sink writes a `DreamTrigger` record to
//! `.roko/learn/dream_triggers.jsonl`; a separate worker consumes those.
//! The runner is still observably "trying" to dream — the trigger is
//! durable — but it doesn't pay the dream cost on the hot path.
//!
//! When `dream_runner` is supplied the sink calls into it directly,
//! suitable for tests and for hosts that want eager consolidation.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::{FeedbackEvent, FeedbackSink};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamTrigger {
    pub kind: DreamTriggerKind,
    pub plan_id: Option<String>,
    pub timestamp_ms: u64,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamTriggerKind {
    PlanCompleted,
    Idle,
}

#[async_trait]
pub trait DreamRunner: Send + Sync + std::fmt::Debug {
    async fn run(&self, trigger: &DreamTrigger) -> Result<(), anyhow::Error>;
}

#[derive(Debug)]
pub struct DreamTriggerSink {
    path: PathBuf,
    file: Mutex<Option<tokio::fs::File>>,
    runner: Option<Arc<dyn DreamRunner>>,
    /// Idle ticks threshold before an idle trigger fires.
    idle_threshold: u32,
}

impl DreamTriggerSink {
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            file: Mutex::new(None),
            runner: None,
            idle_threshold: 12,
        }
    }

    #[must_use]
    pub fn with_runner(mut self, runner: Arc<dyn DreamRunner>) -> Self {
        self.runner = Some(runner);
        self
    }

    #[must_use]
    pub fn with_idle_threshold(mut self, threshold: u32) -> Self {
        self.idle_threshold = threshold;
        self
    }

    async fn append(&self, trigger: &DreamTrigger) -> Result<(), anyhow::Error> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let mut guard = self.file.lock().await;
        if guard.is_none() {
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .await?;
            *guard = Some(file);
        }
        let line = serde_json::to_string(trigger)?;
        let bytes = format!("{line}\n");
        let file = guard.as_mut().unwrap();
        file.write_all(bytes.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }
}

#[async_trait]
impl FeedbackSink for DreamTriggerSink {
    fn name(&self) -> &'static str {
        "dreams"
    }

    fn interested(&self, event: &FeedbackEvent) -> bool {
        match event {
            FeedbackEvent::PlanCompleted { .. } => true,
            FeedbackEvent::IdleTick {
                ticks_since_last_work,
            } => *ticks_since_last_work >= self.idle_threshold,
            _ => false,
        }
    }

    async fn on_event(&self, event: &FeedbackEvent) -> Result<(), anyhow::Error> {
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let trigger = match event {
            FeedbackEvent::PlanCompleted {
                plan_id,
                succeeded,
                tasks_completed,
                tasks_failed,
                total_cost_usd,
            } => DreamTrigger {
                kind: DreamTriggerKind::PlanCompleted,
                plan_id: Some(plan_id.clone()),
                timestamp_ms: now,
                detail: serde_json::json!({
                    "succeeded": succeeded,
                    "tasks_completed": tasks_completed,
                    "tasks_failed": tasks_failed,
                    "total_cost_usd": total_cost_usd,
                }),
            },
            FeedbackEvent::IdleTick {
                ticks_since_last_work,
            } if *ticks_since_last_work >= self.idle_threshold => DreamTrigger {
                kind: DreamTriggerKind::Idle,
                plan_id: None,
                timestamp_ms: now,
                detail: serde_json::json!({
                    "ticks_since_last_work": ticks_since_last_work,
                }),
            },
            _ => return Ok(()),
        };

        self.append(&trigger).await?;
        if let Some(runner) = &self.runner {
            runner.run(&trigger).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn plan_completed_writes_trigger() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dreams.jsonl");
        let sink = DreamTriggerSink::at(&path);
        sink.on_event(&FeedbackEvent::PlanCompleted {
            plan_id: "p".into(),
            succeeded: true,
            tasks_completed: 3,
            tasks_failed: 0,
            total_cost_usd: 0.05,
        })
        .await
        .unwrap();
        let txt = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(txt.contains("\"kind\":\"plan_completed\""));
        assert!(txt.contains("\"tasks_completed\":3"));
    }

    #[tokio::test]
    async fn idle_under_threshold_does_not_trigger() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dreams.jsonl");
        let sink = DreamTriggerSink::at(&path).with_idle_threshold(10);
        let event = FeedbackEvent::IdleTick {
            ticks_since_last_work: 5,
        };
        assert!(!sink.interested(&event));
        sink.on_event(&event).await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn idle_at_threshold_triggers() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dreams.jsonl");
        let sink = DreamTriggerSink::at(&path).with_idle_threshold(3);
        let event = FeedbackEvent::IdleTick {
            ticks_since_last_work: 3,
        };
        assert!(sink.interested(&event));
        sink.on_event(&event).await.unwrap();
        let txt = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(txt.contains("\"kind\":\"idle\""));
    }

    #[derive(Debug, Default)]
    struct CountingRunner {
        runs: tokio::sync::Mutex<u32>,
    }

    #[async_trait]
    impl DreamRunner for CountingRunner {
        async fn run(&self, _t: &DreamTrigger) -> Result<(), anyhow::Error> {
            let mut g = self.runs.lock().await;
            *g += 1;
            Ok(())
        }
    }

    #[tokio::test]
    async fn runner_attached_runs_immediately_on_trigger() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dreams.jsonl");
        let runner = Arc::new(CountingRunner::default());
        let sink = DreamTriggerSink::at(&path).with_runner(runner.clone());
        sink.on_event(&FeedbackEvent::PlanCompleted {
            plan_id: "p".into(),
            succeeded: true,
            tasks_completed: 1,
            tasks_failed: 0,
            total_cost_usd: 0.001,
        })
        .await
        .unwrap();
        assert_eq!(*runner.runs.lock().await, 1);
    }
}
