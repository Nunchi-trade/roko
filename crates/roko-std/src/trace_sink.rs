//! [`InMemoryTraceSink`] — test / TUI-inspection [`TraceSink`] (§36.100).
//!
//! Stores trace events in a bounded-capacity buffer and remembers the
//! last finished [`ToolTrace`] per [`TraceId`]. Intended for:
//!
//! - **Unit tests** — assertions on event streams without touching disk.
//! - **TUI "live trace" view** — recent events displayed as they arrive.
//!
//! Not intended for production persistence — use
//! `roko_fs::JsonlTraceSink` for that.

use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use roko_core::tool::trace::{ToolTrace, ToolTraceEvent, TraceId, TraceSink};

/// One entry in the event buffer.
#[derive(Debug, Clone)]
struct EntryRecord {
    trace_id: TraceId,
    event: ToolTraceEvent,
}

/// In-memory [`TraceSink`] with a bounded event buffer.
///
/// Cheap to clone — all state lives behind an [`Arc`]/[`Mutex`] so
/// multiple handles share the same buffer.
#[derive(Clone, Default)]
pub struct InMemoryTraceSink {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// Event buffer, oldest-first; drop-front when capacity is exceeded.
    events: VecDeque<EntryRecord>,
    /// Finished traces, keyed by trace id.
    finished: HashMap<TraceId, ToolTrace>,
    /// Optional capacity cap (None = unbounded).
    cap: Option<usize>,
}

impl InMemoryTraceSink {
    /// Construct an unbounded sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a sink with a maximum retained event count.
    ///
    /// When the buffer reaches `cap`, the oldest event is dropped on
    /// each new append (FIFO). Finished traces are *not* capped.
    #[must_use]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                events: VecDeque::with_capacity(cap),
                finished: HashMap::new(),
                cap: Some(cap),
            })),
        }
    }

    /// Snapshot of all retained events (cloned).
    #[must_use]
    pub fn events(&self) -> Vec<ToolTraceEvent> {
        self.inner
            .lock()
            .events
            .iter()
            .map(|r| r.event.clone())
            .collect()
    }

    /// Snapshot of retained events for one trace id (cloned).
    #[must_use]
    pub fn events_for(&self, trace_id: TraceId) -> Vec<ToolTraceEvent> {
        self.inner
            .lock()
            .events
            .iter()
            .filter(|r| r.trace_id == trace_id)
            .map(|r| r.event.clone())
            .collect()
    }

    /// Retrieve a finished trace by id (cloned).
    #[must_use]
    pub fn finished(&self, trace_id: TraceId) -> Option<ToolTrace> {
        self.inner.lock().finished.get(&trace_id).cloned()
    }

    /// Number of events currently retained in the buffer.
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.inner.lock().events.len()
    }

    /// Number of distinct finished traces retained.
    #[must_use]
    pub fn finished_count(&self) -> usize {
        self.inner.lock().finished.len()
    }

    /// Clear all retained events and finished traces (test helper).
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.events.clear();
        inner.finished.clear();
    }
}

impl std::fmt::Debug for InMemoryTraceSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock();
        f.debug_struct("InMemoryTraceSink")
            .field("events", &inner.events.len())
            .field("finished", &inner.finished.len())
            .field("cap", &inner.cap)
            .finish()
    }
}

impl TraceSink for InMemoryTraceSink {
    fn append(&self, trace_id: TraceId, event: ToolTraceEvent) {
        let mut inner = self.inner.lock();
        if let Some(cap) = inner.cap {
            // cap == 0 means "keep nothing"; skip the append entirely.
            if cap == 0 {
                return;
            }
            // Drop oldest events until `push_back` will leave us within cap.
            while inner.events.len() >= cap {
                if inner.events.pop_front().is_none() {
                    break;
                }
            }
        }
        inner.events.push_back(EntryRecord { trace_id, event });
    }

    fn finish(&self, trace: ToolTrace) {
        self.inner.lock().finished.insert(trace.trace_id, trace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::AgentRole;
    use roko_core::tool::ToolFormat;
    use roko_core::tool::trace::{ToolOutcome, TraceId};

    fn trace_id(byte: u8) -> TraceId {
        TraceId::from_bytes([byte; 16])
    }

    fn make_trace(id: TraceId) -> ToolTrace {
        ToolTrace {
            trace_id: id,
            call_id: "call-1".to_string(),
            role: AgentRole::Implementer,
            model: "mock".to_string(),
            format_used: ToolFormat::OpenAiJson,
            started_at_ms: 0,
            ended_at_ms: 10,
            events: Vec::new(),
            outcome: ToolOutcome::success(10, 0.0),
        }
    }

    #[test]
    fn unbounded_retains_all_events() {
        let sink = InMemoryTraceSink::new();
        let id = trace_id(0x01);
        for i in 0..10 {
            sink.append(id, ToolTraceEvent::StreamCoerced { at_ms: i });
        }
        assert_eq!(sink.event_count(), 10);
    }

    #[test]
    fn capacity_drops_oldest_events() {
        let sink = InMemoryTraceSink::with_capacity(3);
        let id = trace_id(0x02);
        for i in 0..5 {
            sink.append(id, ToolTraceEvent::StreamCoerced { at_ms: i });
        }
        assert_eq!(sink.event_count(), 3);
        // Should have kept the last 3 (at_ms = 2, 3, 4).
        let events = sink.events();
        let at_ms_values: Vec<i64> = events.iter().map(ToolTraceEvent::at_ms).collect();
        assert_eq!(at_ms_values, vec![2, 3, 4]);
    }

    #[test]
    fn finish_stores_trace() {
        let sink = InMemoryTraceSink::new();
        let id = trace_id(0x03);
        sink.finish(make_trace(id));
        assert_eq!(sink.finished_count(), 1);
        let retrieved = sink.finished(id).expect("trace must be present");
        assert_eq!(retrieved.trace_id, id);
    }

    #[test]
    fn events_for_filters_by_trace_id() {
        let sink = InMemoryTraceSink::new();
        let id_a = trace_id(0xAA);
        let id_b = trace_id(0xBB);
        sink.append(id_a, ToolTraceEvent::StreamCoerced { at_ms: 1 });
        sink.append(id_b, ToolTraceEvent::StreamCoerced { at_ms: 2 });
        sink.append(id_a, ToolTraceEvent::StreamCoerced { at_ms: 3 });
        assert_eq!(sink.events_for(id_a).len(), 2);
        assert_eq!(sink.events_for(id_b).len(), 1);
    }

    #[test]
    fn clear_empties_buffer() {
        let sink = InMemoryTraceSink::new();
        let id = trace_id(0x04);
        sink.append(id, ToolTraceEvent::StreamCoerced { at_ms: 1 });
        sink.finish(make_trace(id));
        sink.clear();
        assert_eq!(sink.event_count(), 0);
        assert_eq!(sink.finished_count(), 0);
    }

    #[test]
    fn shared_handles_see_same_buffer() {
        let a = InMemoryTraceSink::new();
        let b = a.clone();
        let id = trace_id(0x05);
        a.append(id, ToolTraceEvent::StreamCoerced { at_ms: 1 });
        assert_eq!(b.event_count(), 1);
    }

    #[test]
    fn capacity_zero_drops_all() {
        let sink = InMemoryTraceSink::with_capacity(0);
        let id = trace_id(0x06);
        sink.append(id, ToolTraceEvent::StreamCoerced { at_ms: 1 });
        assert_eq!(sink.event_count(), 0);
    }

    #[test]
    fn uses_as_trace_sink_trait_object() {
        let sink: Arc<dyn TraceSink> = Arc::new(InMemoryTraceSink::new());
        let id = trace_id(0x07);
        sink.append(id, ToolTraceEvent::StreamCoerced { at_ms: 1 });
        sink.finish(make_trace(id));
    }
}
