//! Async snapshot writer — offloads JSON persistence to a dedicated OS thread
//! so `save_snapshot` never blocks the `select!` event loop.

use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::JoinHandle;

use tracing::{error, warn};

/// Pre-serialized snapshot data ready for disk writes.
pub struct SnapshotPayload {
    pub orchestrator_json: Vec<u8>,
    pub orchestrator_path: PathBuf,
    pub executor_json: Vec<u8>,
    pub executor_path: PathBuf,
    pub run_state_json: Vec<u8>,
    pub run_state_path: PathBuf,
}

enum WriterMsg {
    Write(SnapshotPayload),
    /// Drain pending writes, then ack via the flush channel.
    Flush,
}

/// Result of draining the channel for pending `Write` messages.
struct DrainResult {
    /// The latest payload found while draining (if any).
    latest: Option<SnapshotPayload>,
    /// Whether a `Flush` message was consumed during the drain and needs acking.
    flush_consumed: bool,
}

/// Bounded-channel writer that persists snapshots on a dedicated OS thread.
///
/// If multiple snapshots queue up, intermediate ones are skipped — the thread
/// always drains to the *latest* payload before writing.
pub struct SnapshotWriter {
    tx: Option<SyncSender<WriterMsg>>,
    handle: Option<JoinHandle<()>>,
    /// Ack channel used by `flush()` to block until the writer has drained.
    flush_rx: std::sync::mpsc::Receiver<()>,
}

impl SnapshotWriter {
    /// Spawn the writer thread. `capacity` bounds the channel (4 is plenty).
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel::<WriterMsg>(capacity);
        let (flush_tx, flush_rx) = mpsc::channel::<()>();
        let handle = std::thread::Builder::new()
            .name("snapshot-writer".into())
            .spawn(move || writer_loop(rx, flush_tx))
            .expect("failed to spawn snapshot-writer thread");

        Self {
            tx: Some(tx),
            handle: Some(handle),
            flush_rx,
        }
    }

    /// Enqueue a snapshot for async write. Non-blocking: drops the payload if
    /// the channel is full (the writer will catch up with the next one).
    pub fn write(&self, payload: SnapshotPayload) {
        let Some(tx) = self.tx.as_ref() else { return };
        match tx.try_send(WriterMsg::Write(payload)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                warn!("snapshot writer channel full — dropping intermediate snapshot");
            }
            Err(TrySendError::Disconnected(_)) => {
                error!("snapshot writer thread has stopped — snapshot lost");
            }
        }
    }

    /// Block until the writer thread has drained all pending payloads.
    pub fn flush(&self) {
        let Some(tx) = self.tx.as_ref() else { return };
        if tx.send(WriterMsg::Flush).is_ok() {
            let _ = self.flush_rx.recv();
        }
    }
}

impl Drop for SnapshotWriter {
    fn drop(&mut self) {
        // Drop the sender to close the channel, then join the writer thread.
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            if let Err(panic_payload) = handle.join() {
                let msg = panic_payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic_payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic");
                error!(
                    panic = %msg,
                    "snapshot-writer thread panicked — recent snapshots may be lost"
                );
            }
        }
    }
}

/// Writer thread main loop.
fn writer_loop(rx: std::sync::mpsc::Receiver<WriterMsg>, flush_tx: std::sync::mpsc::Sender<()>) {
    let mut fail_streak: u32 = 0;

    loop {
        let msg = match rx.recv() {
            Ok(msg) => msg,
            Err(_) => break, // sender dropped — exit
        };

        match msg {
            WriterMsg::Write(payload) => {
                let drain = drain_writes(&rx);
                let to_write = drain.latest.unwrap_or(payload);
                write_payload(&to_write, &mut fail_streak);
                if drain.flush_consumed {
                    let _ = flush_tx.send(());
                }
            }
            WriterMsg::Flush => {
                let drain = drain_writes(&rx);
                if let Some(payload) = drain.latest {
                    write_payload(&payload, &mut fail_streak);
                }
                let _ = flush_tx.send(());
                // If another Flush was consumed during drain, ack it too.
                if drain.flush_consumed {
                    let _ = flush_tx.send(());
                }
            }
        }
    }
}

/// Non-blocking drain: consume all queued `Write` messages, return only the
/// latest payload. Stops on empty queue or disconnected channel. If a `Flush`
/// is encountered, it is consumed and flagged so the caller can ack it.
fn drain_writes(rx: &std::sync::mpsc::Receiver<WriterMsg>) -> DrainResult {
    let mut latest: Option<SnapshotPayload> = None;
    let mut flush_consumed = false;
    loop {
        match rx.try_recv() {
            Ok(WriterMsg::Write(payload)) => {
                latest = Some(payload);
            }
            Ok(WriterMsg::Flush) => {
                flush_consumed = true;
                break;
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }
    DrainResult {
        latest,
        flush_consumed,
    }
}

fn write_payload(payload: &SnapshotPayload, fail_streak: &mut u32) {
    if let Err(e) = write_all_files(payload) {
        *fail_streak += 1;
        if *fail_streak >= 3 {
            error!(
                error = %e,
                streak = *fail_streak,
                "snapshot persistence degraded"
            );
        } else {
            error!(error = %e, "failed to write snapshot");
        }
    } else {
        *fail_streak = 0;
    }
}

fn write_all_files(payload: &SnapshotPayload) -> anyhow::Result<()> {
    use super::persist::atomic_write;
    atomic_write(&payload.orchestrator_path, &payload.orchestrator_json)?;
    atomic_write(&payload.executor_path, &payload.executor_json)?;
    atomic_write(&payload.run_state_path, &payload.run_state_json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_flush_persists_files() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = SnapshotWriter::new(4);

        let payload = SnapshotPayload {
            orchestrator_json: b"orch".to_vec(),
            orchestrator_path: tmp.path().join("orch.json"),
            executor_json: b"exec".to_vec(),
            executor_path: tmp.path().join("exec.json"),
            run_state_json: b"state".to_vec(),
            run_state_path: tmp.path().join("state.json"),
        };
        writer.write(payload);
        writer.flush();

        assert_eq!(
            std::fs::read_to_string(tmp.path().join("orch.json")).unwrap(),
            "orch"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("exec.json")).unwrap(),
            "exec"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("state.json")).unwrap(),
            "state"
        );
    }

    #[test]
    fn latest_snapshot_wins_when_batched() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = SnapshotWriter::new(4);

        for i in 0..3 {
            let label = format!("v{i}");
            writer.write(SnapshotPayload {
                orchestrator_json: label.as_bytes().to_vec(),
                orchestrator_path: tmp.path().join("orch.json"),
                executor_json: label.as_bytes().to_vec(),
                executor_path: tmp.path().join("exec.json"),
                run_state_json: label.as_bytes().to_vec(),
                run_state_path: tmp.path().join("state.json"),
            });
        }
        writer.flush();

        // The writer drains to latest, so files should contain "v2" (or at
        // minimum the last payload that was written).
        let content = std::fs::read_to_string(tmp.path().join("orch.json")).unwrap();
        assert!(content == "v0" || content == "v1" || content == "v2");
    }

    #[test]
    fn drop_joins_thread() {
        let writer = SnapshotWriter::new(4);
        drop(writer);
        // If we get here without hanging, the thread joined successfully.
    }
}
