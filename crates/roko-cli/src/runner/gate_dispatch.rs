//! Verify dispatch — runs gate rungs as background tokio tasks and sends
//! results through a channel.

use std::path::PathBuf;
use std::time::Instant;

use roko_core::{Body, EngramBuilder, Kind, Provenance};
use roko_gate::rung_dispatch::{RungExecutionConfig, RungExecutionInputs, run_rung};
use tokio::sync::mpsc;
use tracing::{error, info};

use super::types::{GateCompletion, GateVerdictSummary};

/// Spawn a gate rung as a background task. Sends `GateCompletion` when done.
pub fn spawn_gate(
    plan_id: String,
    task_id: String,
    rung: u32,
    workdir: PathBuf,
    gate_tx: mpsc::Sender<GateCompletion>,
) {
    tokio::spawn(async move {
        let start = Instant::now();

        let signal = EngramBuilder::new(Kind::Task)
            .body(Body::text(format!("gate:{plan_id}:{task_id}:rung-{rung}")))
            .provenance(Provenance::trusted("runner"))
            .build();
        let ctx = roko_core::Context::now();

        let inputs = RungExecutionInputs::default();
        let config = RungExecutionConfig {
            source_roots: Some(vec![workdir]),
            ..Default::default()
        };

        let verdicts = run_rung(&signal, &ctx, rung, &inputs, &config).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let passed = verdicts.iter().all(|v| v.passed);
        let output = verdicts
            .iter()
            .map(|v| format!("{}: {}", v.gate, if v.passed { "pass" } else { "FAIL" }))
            .collect::<Vec<_>>()
            .join("; ");

        let summaries: Vec<GateVerdictSummary> = verdicts
            .iter()
            .map(|v| GateVerdictSummary {
                gate_name: v.gate.clone(),
                passed: v.passed,
                summary: v.reason.clone(),
            })
            .collect();

        info!(
            plan_id = %plan_id,
            task_id = %task_id,
            rung,
            passed,
            duration_ms,
            "gate completed"
        );

        let completion = GateCompletion {
            plan_id,
            task_id,
            rung,
            passed,
            verdicts: summaries,
            output,
            duration_ms,
        };

        if let Err(e) = gate_tx.send(completion).await {
            error!(err = %e, "failed to send gate completion — channel closed");
            return;
        }
    });
}
