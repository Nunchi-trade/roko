//! JsonlLogger -- persists RuntimeEvents to a JSONL file.
//!
//! Each event is serialized as a single JSON line with a timestamp, enabling
//! replay and state reconstruction.

use roko_core::RuntimeEvent;
pub use roko_core::foundation::EventConsumer;
use roko_core::runtime_event::RuntimeEventEnvelope;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Logger that writes RuntimeEvents as JSONL (one JSON object per line).
pub struct JsonlLogger {
    path: PathBuf,
    seq: AtomicU64,
    writer: Mutex<Option<std::io::BufWriter<std::fs::File>>>,
}

impl JsonlLogger {
    /// Create a new JsonlLogger writing to the given path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            seq: AtomicU64::new(0),
            writer: Mutex::new(None),
        }
    }

    /// Create from the standard .roko directory.
    pub fn from_roko_dir(roko_dir: &Path) -> Self {
        Self::new(roko_dir.join("runtime-events.jsonl"))
    }

    /// Path to the log file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn ensure_writer(&self) -> std::io::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if writer.is_none() {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            *writer = Some(std::io::BufWriter::new(file));
        }
        drop(writer);

        Ok(())
    }

    fn write_event(&self, event: &RuntimeEvent) -> std::io::Result<()> {
        self.ensure_writer()?;

        let envelope = RuntimeEventEnvelope::new(
            event.run_id(),
            self.seq.fetch_add(1, Ordering::Relaxed),
            "jsonl_logger",
            event.clone(),
        );

        let json = serde_json::to_string(&envelope)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;

        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(ref mut w) = *writer {
            writeln!(w, "{json}")?;
            w.flush()?;
        }

        Ok(())
    }
}

impl EventConsumer for JsonlLogger {
    fn consume(&self, event: &RuntimeEvent) {
        let _ = self.write_event(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_events_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let logger = JsonlLogger::new(path.clone());

        logger.consume(&RuntimeEvent::AgentSpawned {
            run_id: "r1".into(),
            agent_id: "a1".into(),
            role: "implementer".into(),
            model: "model".into(),
        });

        logger.consume(&RuntimeEvent::GatePassed {
            run_id: "r1".into(),
            gate_name: "compile".into(),
            duration_ms: 100,
        });

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: RuntimeEventEnvelope = serde_json::from_str(lines[0]).unwrap();
        let second: RuntimeEventEnvelope = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first.payload.kind(), "agent_spawned");
        assert_eq!(second.payload.kind(), "gate_passed");
    }
}
