//! JsonlLogger -- persists RuntimeEvents to a JSONL file.
//!
//! Each event is serialized as a single JSON line with a timestamp, enabling
//! replay and state reconstruction.

use crate::effect_driver::RuntimeEvent;
use chrono::Utc;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// TODO(arch): Replace this local compatibility trait with
// `roko_core::foundation::EventConsumer` once the manifest dependency direction
// permits `roko-runtime -> roko-core`. This checkout currently has
// `roko-core -> roko-runtime`, and this batch cannot modify Cargo.toml.
/// Consume RuntimeEvents for side effects such as logging or UI updates.
pub trait EventConsumer: Send + Sync {
    /// Called for each event emitted by the workflow engine.
    fn consume(&self, event: &RuntimeEvent);
}

/// Logger that writes RuntimeEvents as JSONL (one JSON object per line).
pub struct JsonlLogger {
    path: PathBuf,
    writer: Mutex<Option<std::io::BufWriter<std::fs::File>>>,
}

impl JsonlLogger {
    /// Create a new JsonlLogger writing to the given path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
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
        let mut writer = self.writer.lock().expect("jsonl logger writer lock poisoned");
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

        Ok(())
    }

    fn write_event(&self, event: &RuntimeEvent) {
        if self.ensure_writer().is_err() {
            return;
        }

        let json = serde_json::json!({
            "ts": Utc::now().to_rfc3339(),
            "kind": event_kind(event),
            "run_id": event_run_id(event),
            "event": format!("{event:?}"),
        });

        let mut writer = self.writer.lock().expect("jsonl logger writer lock poisoned");
        if let Some(ref mut w) = *writer {
            let _ = writeln!(w, "{json}");
            let _ = w.flush();
        }
    }
}

impl EventConsumer for JsonlLogger {
    fn consume(&self, event: &RuntimeEvent) {
        self.write_event(event);
    }
}

fn event_kind(event: &RuntimeEvent) -> &'static str {
    event.kind()
}

fn event_run_id(event: &RuntimeEvent) -> &str {
    event.run_id()
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
        assert!(lines[0].contains("agent_spawned"));
        assert!(lines[1].contains("gate_passed"));
    }
}
