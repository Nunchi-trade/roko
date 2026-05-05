//! Store-backed verdict aggregation for gate observability.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use chrono::{DateTime, TimeZone, Utc};
use roko_core::{ContentHash, Context, Engram, FailureEntry, Kind, Query, Store, TrendBuckets};
use roko_fs::FileSubstrate;
use tokio::runtime::{Builder, Runtime};

const DEFAULT_BUCKET_SECS: u64 = roko_core::defaults::DEFAULT_VERDICT_BUCKET_SECS;
const DEFAULT_BUCKET_COUNT: usize = roko_core::defaults::DEFAULT_VERDICT_BUCKET_COUNT;
const FAILURE_CAP: usize = 50;

/// Incremental lower-bound cursor for verdict substrate queries.
#[derive(Debug, Clone, Default)]
pub struct SubstrateCursor {
    since_ms: Option<i64>,
    seen_ids_at_since_ms: HashSet<ContentHash>,
}

impl SubstrateCursor {
    fn query(&self) -> Query {
        self.since_ms.map_or_else(
            || Query::of_kind(Kind::GateVerdict),
            |since| Query::of_kind(Kind::GateVerdict).since(since),
        )
    }

    fn should_visit(&self, signal: &Engram) -> bool {
        match self.since_ms {
            None => true,
            Some(lower_bound) if signal.created_at_ms > lower_bound => true,
            Some(lower_bound) if signal.created_at_ms == lower_bound => {
                !self.seen_ids_at_since_ms.contains(&signal.id)
            }
            _ => false,
        }
    }

    fn mark_seen(&mut self, signal: &Engram) {
        match self.since_ms {
            None => {
                self.since_ms = Some(signal.created_at_ms);
                self.seen_ids_at_since_ms.clear();
                self.seen_ids_at_since_ms.insert(signal.id);
            }
            Some(current) if signal.created_at_ms > current => {
                self.since_ms = Some(signal.created_at_ms);
                self.seen_ids_at_since_ms.clear();
                self.seen_ids_at_since_ms.insert(signal.id);
            }
            Some(current) if signal.created_at_ms == current => {
                self.seen_ids_at_since_ms.insert(signal.id);
            }
            _ => {}
        }
    }
}

/// Rolling per-gate statistics derived from persisted verdict engrams.
#[derive(Debug, Clone, Default)]
pub struct GateStats {
    /// Verify name.
    pub name: String,
    /// Rolling 24x1h pass/fail buckets.
    pub buckets: TrendBuckets,
    /// Recent failures for this gate, newest last.
    pub recent_failures: VecDeque<FailureEntry>,
}

impl GateStats {
    fn new(name: impl Into<String>, now: DateTime<Utc>) -> Self {
        Self {
            name: name.into(),
            buckets: TrendBuckets::new(DEFAULT_BUCKET_SECS, DEFAULT_BUCKET_COUNT, now),
            recent_failures: VecDeque::new(),
        }
    }
}

/// Incremental reader and rolling aggregator for persisted gate verdicts.
pub struct VerdictsAggregator {
    runtime: Runtime,
    substrate: Arc<FileSubstrate>,
    cursor: SubstrateCursor,
    per_gate: HashMap<String, GateStats>,
    recent_failures: VecDeque<FailureEntry>,
}

impl VerdictsAggregator {
    /// Open the workspace substrate and prepare a verdict reader.
    ///
    /// When called inside a Tokio runtime, offloads the blocking substrate
    /// open to `tokio::task::spawn_blocking` (reuses the Tokio thread pool
    /// instead of creating a fresh OS thread on every call). When no runtime
    /// is active, creates a local current-thread runtime for the open.
    pub async fn open(workdir: impl AsRef<Path>) -> Result<Self> {
        let workdir = workdir.as_ref().to_path_buf();
        let load = move || -> Result<(Runtime, Arc<FileSubstrate>)> {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .context("build verdict reader runtime")?;
            let substrate = runtime
                .block_on(FileSubstrate::open(workdir.join(".roko")))
                .context("open verdict substrate")?;
            Ok((runtime, Arc::new(substrate)))
        };

        let (runtime, substrate) = if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::spawn_blocking(load)
                .await
                .map_err(|e| anyhow::anyhow!("verdict loader task panicked: {e}"))??
        } else {
            load()?
        };

        Ok(Self {
            runtime,
            substrate,
            cursor: SubstrateCursor::default(),
            per_gate: HashMap::new(),
            recent_failures: VecDeque::new(),
        })
    }

    /// Advance the reader and fold any newly observed verdict signals.
    ///
    /// When called inside a Tokio runtime, offloads the blocking substrate
    /// query to `tokio::task::spawn_blocking` (reuses the Tokio thread pool
    /// instead of creating a fresh OS thread on every tick). When no runtime
    /// is active, uses the embedded current-thread runtime directly.
    pub async fn tick(&mut self) -> Result<()> {
        let now = Utc::now();
        let ctx = Context::now();
        let query = self.cursor.query();
        let substrate = Arc::clone(&self.substrate);
        let mut verdicts = if tokio::runtime::Handle::try_current().is_ok() {
            // We're inside a tokio runtime — cannot call block_on on our
            // current-thread runtime here, so offload to a blocking task.
            let rt_handle = self.runtime.handle().clone();
            tokio::task::spawn_blocking(move || rt_handle.block_on(substrate.query(&query, &ctx)))
                .await
                .map_err(|e| anyhow::anyhow!("verdict tick task panicked: {e}"))?
                .context("query verdict substrate")?
        } else {
            self.runtime
                .block_on(substrate.query(&query, &ctx))
                .context("query verdict substrate")?
        };
        verdicts.sort_by(|lhs, rhs| {
            lhs.created_at_ms
                .cmp(&rhs.created_at_ms)
                .then_with(|| lhs.id.to_hex().cmp(&rhs.id.to_hex()))
        });

        for verdict in verdicts {
            if !self.cursor.should_visit(&verdict) {
                continue;
            }
            self.ingest(verdict, now);
        }

        for stats in self.per_gate.values_mut() {
            stats.buckets.align_to(now);
        }

        Ok(())
    }

    /// Blocking wrapper for [`Self::open`] — for use in sync contexts where
    /// no Tokio runtime is active (e.g. the standalone `main_loop`).
    ///
    /// Creates a current-thread runtime internally and blocks on substrate
    /// open. Panics if called from inside a Tokio runtime (use the async
    /// `open()` instead).
    pub fn open_blocking(workdir: impl AsRef<Path>) -> Result<Self> {
        let workdir = workdir.as_ref().to_path_buf();
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build verdict reader runtime")?;
        let substrate = runtime
            .block_on(FileSubstrate::open(workdir.join(".roko")))
            .context("open verdict substrate")?;
        Ok(Self {
            runtime,
            substrate: Arc::new(substrate),
            cursor: SubstrateCursor::default(),
            per_gate: HashMap::new(),
            recent_failures: VecDeque::new(),
        })
    }

    /// Blocking wrapper for [`Self::tick`] — for use in sync contexts where
    /// no Tokio runtime is active (e.g. the standalone `main_loop`).
    ///
    /// Uses the embedded current-thread runtime directly. Panics if called
    /// from inside a Tokio runtime (use the async `tick()` instead).
    pub fn tick_blocking(&mut self) -> Result<()> {
        let now = Utc::now();
        let ctx = Context::now();
        let query = self.cursor.query();
        let substrate = Arc::clone(&self.substrate);
        let mut verdicts = self
            .runtime
            .block_on(substrate.query(&query, &ctx))
            .context("query verdict substrate")?;
        verdicts.sort_by(|lhs, rhs| {
            lhs.created_at_ms
                .cmp(&rhs.created_at_ms)
                .then_with(|| lhs.id.to_hex().cmp(&rhs.id.to_hex()))
        });

        for verdict in verdicts {
            if !self.cursor.should_visit(&verdict) {
                continue;
            }
            self.ingest(verdict, now);
        }

        for stats in self.per_gate.values_mut() {
            stats.buckets.align_to(now);
        }

        Ok(())
    }

    /// Snapshot of current per-gate trends keyed by gate name.
    #[must_use]
    pub fn gate_trends(&self) -> HashMap<String, TrendBuckets> {
        self.per_gate
            .iter()
            .map(|(gate, stats)| (gate.clone(), stats.buckets.clone()))
            .collect()
    }

    /// Snapshot of recent failures across all gates, oldest to newest.
    #[must_use]
    pub fn recent_failures(&self) -> Vec<FailureEntry> {
        self.recent_failures.iter().cloned().collect()
    }

    fn ingest(&mut self, signal: Engram, now: DateTime<Utc>) {
        let Some(gate) = extract_gate_name(&signal) else {
            self.cursor.mark_seen(&signal);
            return;
        };
        let Some(passed) = extract_gate_passed(&signal) else {
            self.cursor.mark_seen(&signal);
            return;
        };

        let stats = self
            .per_gate
            .entry(gate.clone())
            .or_insert_with(|| GateStats::new(gate.clone(), now));
        stats
            .buckets
            .record_gate_result(timestamp_from_millis(signal.created_at_ms), passed);

        if !passed {
            let failure = FailureEntry {
                ts: timestamp_from_millis(signal.created_at_ms),
                plan_id: extract_plan_id(&signal).unwrap_or_default(),
                task_id: extract_task_id(&signal).unwrap_or_default(),
                gate: gate.clone(),
                summary: extract_summary(&signal).unwrap_or_default(),
                artifacts: extract_artifact_path(&signal),
            };
            push_bounded(&mut stats.recent_failures, failure.clone(), FAILURE_CAP);
            push_bounded(&mut self.recent_failures, failure, FAILURE_CAP);
        }

        self.cursor.mark_seen(&signal);
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, item: T, cap: usize) {
    if cap == 0 {
        return;
    }
    if queue.len() == cap {
        queue.pop_front();
    }
    queue.push_back(item);
}

fn extract_gate_name(signal: &Engram) -> Option<String> {
    signal
        .tag("gate")
        .map(ToOwned::to_owned)
        .or_else(|| extract_json_string(signal, "gate"))
}

fn extract_gate_passed(signal: &Engram) -> Option<bool> {
    signal
        .tag("passed")
        .and_then(parse_bool)
        .or_else(|| extract_json_bool(signal, "passed"))
}

fn extract_plan_id(signal: &Engram) -> Option<String> {
    signal
        .tag("plan_id")
        .map(ToOwned::to_owned)
        .or_else(|| extract_json_string(signal, "plan_id"))
}

fn extract_task_id(signal: &Engram) -> Option<String> {
    signal
        .tag("task_id")
        .map(ToOwned::to_owned)
        .or_else(|| extract_json_string(signal, "task_id"))
}

fn extract_summary(signal: &Engram) -> Option<String> {
    extract_json_string(signal, "error_digest")
        .or_else(|| extract_json_string(signal, "reason"))
        .or_else(|| extract_json_string(signal, "detail"))
}

fn extract_artifact_path(signal: &Engram) -> Option<PathBuf> {
    signal
        .tag("artifact")
        .map(PathBuf::from)
        .or_else(|| signal.tag("artifacts").map(PathBuf::from))
        .or_else(|| extract_json_string(signal, "artifact").map(PathBuf::from))
        .or_else(|| extract_json_string(signal, "artifacts").map(PathBuf::from))
}

fn extract_json_string(signal: &Engram, key: &str) -> Option<String> {
    let roko_core::Body::Json(value) = &signal.body else {
        return None;
    };

    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn extract_json_bool(signal: &Engram, key: &str) -> Option<bool> {
    let roko_core::Body::Json(value) = &signal.body else {
        return None;
    };
    value.get(key).and_then(serde_json::Value::as_bool)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn timestamp_from_millis(timestamp_ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .unwrap_or_else(default_bucket_start)
}

fn default_bucket_start() -> DateTime<Utc> {
    Utc.timestamp_opt(0, 0).single().unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Engram, Verdict};
    use tempfile::tempdir;

    fn verdict_signal(ts_ms: i64, gate: &str, passed: bool, task_id: Option<&str>) -> Engram {
        let verdict = if passed {
            Verdict::pass(gate)
        } else {
            Verdict::fail(gate, "boom").with_error_digest("assertion failed")
        };
        let mut builder = Engram::builder(Kind::GateVerdict)
            .body(Body::from_json(&verdict).unwrap())
            .created_at_ms(ts_ms)
            .tag("gate", gate)
            .tag("passed", passed.to_string())
            .tag("plan_id", "plan-a");
        if let Some(task_id) = task_id {
            builder = builder
                .tag("task_id", task_id)
                .tag("artifact", "/tmp/out.txt");
        }
        builder.build()
    }

    #[tokio::test]
    async fn aggregates_new_verdicts_once() {
        let dir = tempdir().unwrap();
        let substrate_root = dir.path().join(".roko");
        let substrate = FileSubstrate::open(&substrate_root).await.unwrap();
        let now_ms = Utc::now().timestamp_millis();
        substrate
            .put(verdict_signal(now_ms - 1_000, "compile", true, None))
            .await
            .unwrap();
        substrate
            .put(verdict_signal(now_ms, "compile", false, Some("task-1")))
            .await
            .unwrap();
        drop(substrate);

        let mut aggregator = VerdictsAggregator::open(dir.path()).await.unwrap();
        aggregator.tick().await.unwrap();

        let trend = aggregator
            .gate_trends()
            .remove("compile")
            .expect("compile trend");
        let total_pass = trend.slots.iter().map(|bucket| bucket.pass).sum::<u32>();
        let total_fail = trend.slots.iter().map(|bucket| bucket.fail).sum::<u32>();
        assert_eq!((total_pass, total_fail), (1, 1));

        let failures = aggregator.recent_failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].task_id, "task-1");
        assert_eq!(failures[0].gate, "compile");
        assert_eq!(
            failures[0]
                .artifacts
                .as_ref()
                .map(|path| path.display().to_string()),
            Some("/tmp/out.txt".to_string())
        );

        aggregator.tick().await.unwrap();
        let trend = aggregator
            .gate_trends()
            .remove("compile")
            .expect("compile trend");
        let total_pass = trend.slots.iter().map(|bucket| bucket.pass).sum::<u32>();
        let total_fail = trend.slots.iter().map(|bucket| bucket.fail).sum::<u32>();
        assert_eq!((total_pass, total_fail), (1, 1));
        assert_eq!(aggregator.recent_failures().len(), 1);
    }
}
