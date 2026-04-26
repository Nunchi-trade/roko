//! Merge dispatch wrapper around [`MergeQueue`].
//!
//! The runner used to translate `ExecutorAction::MergeBranch` into an
//! immediate `ExecutorEvent::MergeSucceeded`, which silently produced
//! broken merges whenever multiple plans ran in parallel and touched
//! overlapping files. This module routes merge actions through
//! `MergeQueue` instead and runs a real post-merge regression gate so a
//! broken integration branch can be detected and surfaced as a merge
//! failure instead of a silent success.
//!
//! The actual git plumbing (`git merge --no-ff`, conflict resolution,
//! batch branch handling) still lives outside this module. `PlanMerger`
//! reuses the existing `MergeQueue` for queue / lock semantics and adds a
//! pluggable post-merge regression gate so the runner can drive a real
//! check (for example a `cargo check --workspace`) against the merged
//! tree before flipping the executor to `MergeSucceeded`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use roko_orchestrator::{MergeQueue, MergeRequest};
use tokio::sync::mpsc;

use super::types::{GateCompletion, GateCompletionKind, GateVerdictSummary, RunnerFailureKind};

// ─── PlanMerger ─────────────────────────────────────────────────────────

/// Outcome of a merge dispatch attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeDispatch {
    /// The merge was claimed and submitted to the regression gate. The
    /// caller should expect a `GateCompletion` with the matching plan id
    /// to arrive on the gate channel.
    Reserved {
        plan_id: String,
        branch_name: String,
    },
    /// The plan was enqueued but is currently blocked by an in-progress
    /// merge holding one or more of its files.
    Blocked { plan_id: String },
}

/// Wrapper around [`MergeQueue`] that submits merge requests, runs a
/// post-merge regression gate, and emits a `GateCompletion` describing
/// the outcome.
#[derive(Debug, Clone)]
pub struct PlanMerger {
    queue: MergeQueue,
    config: PlanMergerConfig,
}

/// Configuration for [`PlanMerger`].
#[derive(Debug, Clone)]
pub struct PlanMergerConfig {
    /// Working directory used when running the regression gate.
    pub workdir: PathBuf,
    /// Wall-clock timeout for the regression gate.
    pub regression_timeout: Duration,
    /// Optional post-merge regression gate. When `None`, the merger uses
    /// [`PlanMerger::default_regression_gate`] (a `cargo check` runner).
    pub regression_gate: Option<Arc<dyn RegressionGate>>,
}

impl PlanMergerConfig {
    /// Construct a config rooted at `workdir`. The regression gate is left
    /// unset so the caller can install a custom gate (or fall back to the
    /// built-in `cargo check` runner).
    #[must_use]
    pub fn new(workdir: PathBuf, regression_timeout: Duration) -> Self {
        Self {
            workdir,
            regression_timeout,
            regression_gate: None,
        }
    }

    /// Install a custom regression gate (used by tests and integrations
    /// that want to stub out cargo).
    #[must_use]
    pub fn with_regression_gate(mut self, gate: Arc<dyn RegressionGate>) -> Self {
        self.regression_gate = Some(gate);
        self
    }
}

// ─── Regression gate ────────────────────────────────────────────────────

/// Outcome of a post-merge regression gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegressionOutcome {
    pub passed: bool,
    pub summary: String,
    pub failure_kind: Option<RunnerFailureKind>,
    pub duration_ms: u64,
}

impl RegressionOutcome {
    #[must_use]
    pub fn pass(summary: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            passed: true,
            summary: summary.into(),
            failure_kind: None,
            duration_ms,
        }
    }

    #[must_use]
    pub fn fail(
        summary: impl Into<String>,
        failure_kind: RunnerFailureKind,
        duration_ms: u64,
    ) -> Self {
        Self {
            passed: false,
            summary: summary.into(),
            failure_kind: Some(failure_kind),
            duration_ms,
        }
    }
}

/// Pluggable regression gate. Implementors run whatever workspace check
/// is appropriate (`cargo check`, custom verifier, etc.).
#[async_trait::async_trait]
pub trait RegressionGate: Send + Sync + std::fmt::Debug {
    async fn run(&self, request: &MergeRequest, config: &PlanMergerConfig) -> RegressionOutcome;
}

/// Built-in cargo-check regression gate. Spawns `cargo check --workspace`
/// in the merger's workdir and converts the exit status into a verdict.
#[derive(Debug, Default, Clone, Copy)]
pub struct CargoCheckRegressionGate;

#[async_trait::async_trait]
impl RegressionGate for CargoCheckRegressionGate {
    async fn run(&self, request: &MergeRequest, config: &PlanMergerConfig) -> RegressionOutcome {
        use std::time::Instant;
        let start = Instant::now();
        let workdir = config.workdir.clone();
        let plan_id = request.plan_id.clone();
        let branch = request.branch_name.clone();
        let timeout = config.regression_timeout;

        let join = tokio::task::spawn_blocking(move || {
            std::process::Command::new("cargo")
                .args(["check", "--workspace", "--quiet"])
                .current_dir(&workdir)
                .output()
        });

        let result = tokio::time::timeout(timeout, join).await;
        let duration_ms = start.elapsed().as_millis() as u64;
        match result {
            Ok(Ok(Ok(output))) => {
                if output.status.success() {
                    RegressionOutcome::pass(
                        format!("post-merge cargo check passed for {plan_id}@{branch}"),
                        duration_ms,
                    )
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let summary = format!(
                        "post-merge cargo check failed for {plan_id}@{branch}: {}",
                        stderr.lines().take(3).collect::<Vec<_>>().join(" | ")
                    );
                    RegressionOutcome::fail(summary, RunnerFailureKind::Permanent, duration_ms)
                }
            }
            Ok(Ok(Err(err))) => RegressionOutcome::fail(
                format!("post-merge cargo check failed to spawn: {err}"),
                RunnerFailureKind::Resource,
                duration_ms,
            ),
            Ok(Err(join_err)) => RegressionOutcome::fail(
                format!("post-merge cargo check task aborted: {join_err}"),
                RunnerFailureKind::Resource,
                duration_ms,
            ),
            Err(_) => RegressionOutcome::fail(
                format!(
                    "post-merge cargo check timed out after {}s",
                    config.regression_timeout.as_secs()
                ),
                RunnerFailureKind::Transient,
                duration_ms,
            ),
        }
    }
}

impl PlanMerger {
    /// Construct a new merger. The merger borrows the existing
    /// `MergeQueue` from the runtime so resume snapshots remain coherent.
    #[must_use]
    pub fn new(queue: MergeQueue, config: PlanMergerConfig) -> Self {
        Self { queue, config }
    }

    /// Default regression gate (cargo check workspace) used when none has
    /// been explicitly installed.
    #[must_use]
    pub fn default_regression_gate() -> Arc<dyn RegressionGate> {
        Arc::new(CargoCheckRegressionGate)
    }

    /// Submit a `MergeRequest` to the queue and (if the queue grants the
    /// slot immediately) spawn the regression gate. Returns:
    ///
    /// - `MergeDispatch::Reserved` when this plan got the merge slot. The
    ///   caller should expect a `GateCompletion` with `kind == Gate` for
    ///   the merge plan id to land on `gate_tx` shortly.
    /// - `MergeDispatch::Blocked` when the plan is queued but waiting for
    ///   another plan's file locks to release.
    pub fn submit(
        &self,
        request: MergeRequest,
        gate_tx: mpsc::Sender<GateCompletion>,
    ) -> MergeDispatch {
        let plan_id = request.plan_id.clone();
        self.queue.enqueue(request);

        let Some(reserved) = self.queue.reserve_next_mergeable() else {
            return MergeDispatch::Blocked { plan_id };
        };
        if reserved.plan_id != plan_id {
            // A higher-priority plan was reserved instead; spawn its
            // regression gate so it can complete, and report the original
            // request as still blocked.
            self.spawn_regression(reserved, gate_tx);
            return MergeDispatch::Blocked { plan_id };
        }

        let branch_name = reserved.branch_name.clone();
        self.spawn_regression(reserved, gate_tx);
        MergeDispatch::Reserved {
            plan_id,
            branch_name,
        }
    }

    /// Try to drain the queue further. Useful after a merge completes to
    /// kick off the next non-conflicting plan.
    pub fn drain_next(&self, gate_tx: mpsc::Sender<GateCompletion>) -> Option<String> {
        let reserved = self.queue.reserve_next_mergeable()?;
        let plan_id = reserved.plan_id.clone();
        self.spawn_regression(reserved, gate_tx);
        Some(plan_id)
    }

    fn spawn_regression(&self, request: MergeRequest, gate_tx: mpsc::Sender<GateCompletion>) {
        let queue = self.queue.clone();
        let config = self.config.clone();
        let gate = self
            .config
            .regression_gate
            .clone()
            .unwrap_or_else(Self::default_regression_gate);

        tokio::spawn(async move {
            let outcome = gate.run(&request, &config).await;
            let plan_id = request.plan_id.clone();
            let passed = outcome.passed;

            if passed {
                queue.mark_complete(&plan_id);
            } else {
                queue.mark_failed(&plan_id, &outcome.summary);
            }

            let summary = GateVerdictSummary {
                gate_name: "post-merge-regression".to_string(),
                passed,
                summary: outcome.summary.clone(),
                error_digest: None,
                failure_kind: outcome.failure_kind,
            };

            let completion = GateCompletion {
                kind: GateCompletionKind::Gate,
                plan_id,
                task_id: format!("merge:{}", request.branch_name),
                rung: u32::MAX - 1,
                passed,
                failure_kind: outcome.failure_kind,
                verdicts: vec![summary],
                output: outcome.summary,
                duration_ms: outcome.duration_ms,
            };

            // Channel may be closed if the runner shut down — log only.
            if gate_tx.send(completion).await.is_err() {
                tracing::warn!("merge regression completion dropped — gate channel closed");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct StubGate {
        calls: Mutex<Vec<MergeRequest>>,
        outcome: Mutex<Option<RegressionOutcome>>,
    }

    impl StubGate {
        fn new(outcome: RegressionOutcome) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                outcome: Mutex::new(Some(outcome)),
            }
        }
    }

    #[async_trait::async_trait]
    impl RegressionGate for StubGate {
        async fn run(
            &self,
            request: &MergeRequest,
            _config: &PlanMergerConfig,
        ) -> RegressionOutcome {
            self.calls.lock().unwrap().push(request.clone());
            self.outcome
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| RegressionOutcome::pass("ok", 1))
        }
    }

    fn merger_with_gate(gate: Arc<dyn RegressionGate>) -> PlanMerger {
        let cfg = PlanMergerConfig::new(PathBuf::from("/tmp"), Duration::from_secs(5))
            .with_regression_gate(gate);
        PlanMerger::new(MergeQueue::new(), cfg)
    }

    #[tokio::test]
    async fn submit_reserves_first_plan_and_runs_regression() {
        let gate: Arc<StubGate> = Arc::new(StubGate::new(RegressionOutcome::pass("ok", 10)));
        let merger = merger_with_gate(gate.clone());
        let (tx, mut rx) = mpsc::channel(4);
        let request = MergeRequest::new("plan-a", "roko/plan-a", vec!["src/lib.rs".to_string()], 0);

        let result = merger.submit(request, tx);
        assert!(matches!(
            result,
            MergeDispatch::Reserved { ref plan_id, .. } if plan_id == "plan-a"
        ));

        let completion = rx.recv().await.expect("regression gate completion");
        assert_eq!(completion.plan_id, "plan-a");
        assert!(completion.passed);
        assert_eq!(gate.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn post_merge_failure_marks_failed_and_emits_completion() {
        let gate: Arc<StubGate> = Arc::new(StubGate::new(RegressionOutcome::fail(
            "regression: trait bound failed",
            RunnerFailureKind::Permanent,
            42,
        )));
        let merger = merger_with_gate(gate.clone());
        let (tx, mut rx) = mpsc::channel(4);
        let request = MergeRequest::new("plan-a", "roko/plan-a", vec!["src/lib.rs".into()], 0);

        let dispatch = merger.submit(request, tx);
        assert!(matches!(dispatch, MergeDispatch::Reserved { .. }));

        let completion = rx.recv().await.expect("expected gate completion");
        assert_eq!(completion.plan_id, "plan-a");
        assert!(!completion.passed);
        assert_eq!(completion.failure_kind, Some(RunnerFailureKind::Permanent));
        assert_eq!(completion.verdicts.len(), 1);
        assert!(completion.verdicts[0].summary.contains("regression"));
    }

    #[tokio::test]
    async fn second_conflicting_plan_blocks_until_first_clears() {
        let gate = Arc::new(StubGate::new(RegressionOutcome::pass("ok", 1)));
        let merger = merger_with_gate(gate.clone());
        let (tx1, mut rx1) = mpsc::channel(4);

        let req_a = MergeRequest::new("plan-a", "roko/plan-a", vec!["src/lib.rs".into()], 0);
        let req_b = MergeRequest::new("plan-b", "roko/plan-b", vec!["src/lib.rs".into()], 0);

        let dispatch_a = merger.submit(req_a, tx1.clone());
        assert!(matches!(dispatch_a, MergeDispatch::Reserved { .. }));

        // Wait for plan-a's regression gate to finish before B is submitted —
        // otherwise we cannot guarantee plan-a holds the lock.
        let completion_a = rx1.recv().await.expect("plan-a completion");
        assert_eq!(completion_a.plan_id, "plan-a");

        // Now submit B. It should not be blocked because plan-a is already
        // released.
        let (tx2, mut rx2) = mpsc::channel(4);
        let dispatch_b = merger.submit(req_b, tx2);
        assert!(matches!(dispatch_b, MergeDispatch::Reserved { .. }));
        let completion_b = rx2.recv().await.expect("plan-b completion");
        assert_eq!(completion_b.plan_id, "plan-b");
    }

    #[tokio::test]
    async fn submit_returns_blocked_when_lock_held() {
        // Manually craft a queue with plan-a already merging so plan-b is
        // blocked when it submits.
        let gate = Arc::new(StubGate::new(RegressionOutcome::pass("ok", 1)));
        let cfg = PlanMergerConfig::new(PathBuf::from("/tmp"), Duration::from_secs(5))
            .with_regression_gate(gate.clone());
        let queue = MergeQueue::new();
        queue.enqueue(MergeRequest::new(
            "plan-a",
            "roko/plan-a",
            vec!["src/lib.rs".into()],
            10,
        ));
        assert!(queue.mark_merging("plan-a"));
        let merger = PlanMerger::new(queue.clone(), cfg);

        let (tx, _rx) = mpsc::channel(4);
        let dispatch = merger.submit(
            MergeRequest::new("plan-b", "roko/plan-b", vec!["src/lib.rs".into()], 0),
            tx,
        );
        assert!(matches!(dispatch, MergeDispatch::Blocked { ref plan_id } if plan_id == "plan-b"));
    }
}
