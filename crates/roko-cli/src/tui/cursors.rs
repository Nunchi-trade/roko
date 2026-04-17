use std::io;
use std::path::{Path, PathBuf};

use roko_learn::episode_logger::Episode;
use serde_json::Value;

use super::dashboard::{
    EventLogEntry, GateResultSummary, GateSignalSummary, SignalSummary, load_event_log,
    signal_gate_result_from_value,
};
use super::jsonl_cursor::JsonlCursor;

#[derive(Debug, Clone, Default)]
pub struct SignalCursor {
    path: PathBuf,
    cursor: JsonlCursor,
    recent_signals: Vec<SignalSummary>,
    gate_signal_summaries: Vec<GateSignalSummary>,
    signal_gate_results: Vec<GateResultSummary>,
}

impl SignalCursor {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            cursor: JsonlCursor::new(&path),
            path,
            recent_signals: Vec::new(),
            gate_signal_summaries: Vec::new(),
            signal_gate_results: Vec::new(),
        }
    }

    pub fn tick(&mut self) -> io::Result<bool> {
        let should_reset = jsonl_cursor_should_reset(&self.path, self.cursor.offset());
        let lines = self.cursor.read_new_lines()?;
        if should_reset {
            self.recent_signals.clear();
            self.gate_signal_summaries.clear();
            self.signal_gate_results.clear();
        }

        if lines.is_empty() {
            return Ok(should_reset);
        }

        for line in lines {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(signal) = SignalSummary::from_value(&value) {
                self.recent_signals.push(signal);
            }
            if let Some(gate_signal) = GateSignalSummary::from_value(&value) {
                self.gate_signal_summaries.push(gate_signal);
            }
            if let Some(gate_result) = signal_gate_result_from_value(&value) {
                self.signal_gate_results.push(gate_result);
            }
        }

        if self.recent_signals.len() > 100 {
            let keep_from = self.recent_signals.len() - 100;
            self.recent_signals.drain(0..keep_from);
        }

        Ok(true)
    }

    pub fn recent_signals(&self) -> &[SignalSummary] {
        &self.recent_signals
    }

    pub fn gate_signal_summaries(&self) -> &[GateSignalSummary] {
        &self.gate_signal_summaries
    }

    pub fn signal_gate_results(&self) -> &[GateResultSummary] {
        &self.signal_gate_results
    }
}

#[derive(Debug, Clone, Default)]
pub struct EpisodeCursor {
    path: PathBuf,
    cursor: JsonlCursor,
    episodes: Vec<Episode>,
}

impl EpisodeCursor {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            cursor: JsonlCursor::new(&path),
            path,
            episodes: Vec::new(),
        }
    }

    pub fn tick(&mut self) -> io::Result<bool> {
        let should_reset = jsonl_cursor_should_reset(&self.path, self.cursor.offset());
        let lines = self.cursor.read_new_lines()?;
        if should_reset {
            self.episodes.clear();
        }

        if lines.is_empty() {
            return Ok(should_reset);
        }

        for line in lines {
            if let Ok(episode) = serde_json::from_str::<Episode>(&line) {
                self.episodes.push(episode);
            }
        }

        Ok(true)
    }

    pub fn episodes(&self) -> &[Episode] {
        &self.episodes
    }
}

#[derive(Debug, Clone, Default)]
pub struct EventLogCursor {
    path: PathBuf,
    event_log: Vec<EventLogEntry>,
}

impl EventLogCursor {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            event_log: Vec::new(),
        }
    }

    pub fn tick(&mut self) -> io::Result<bool> {
        let next = load_event_log(&self.path);
        if next == self.event_log {
            return Ok(false);
        }
        self.event_log = next;
        Ok(true)
    }

    pub fn event_log(&self) -> &[EventLogEntry] {
        &self.event_log
    }
}

fn jsonl_cursor_should_reset(path: &Path, offset: u64) -> bool {
    if offset == 0 {
        return false;
    }

    match std::fs::metadata(path) {
        Ok(meta) => meta.len() < offset,
        Err(err) if err.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}
