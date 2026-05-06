//! Phase 1 integration test -- verifies all v2 core abstractions work together.
//!
//! This test exercises:
//! - Cell::execute() with CellContext
//! - Signal as the canonical type name (Engram alias)
//! - TypeSchema compatibility checks
//! - Observe, Connect, Trigger protocol traits (with local test impls)
//! - Bus pub/sub pipeline connecting observation -> trigger -> cell execution
//!
//! These tests serve as the reference for how Phase 2 (Graph engine) will
//! compose these abstractions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use roko_core::error::Result;
use roko_core::traits::{Connect, Observe, Store, Substrate, Trigger};
use roko_core::{
    Body, Bus, BusErased, Cell, CellContext, CellVersion, ContentHash, Context, Engram, HdcVector,
    Kind, MemoryBus, Pulse, Query, Signal, SignalBuilder, Topic, TopicFilter, TypeSchema,
};

// ============================================================================
// Test doubles
// ============================================================================

/// Minimal no-op Store for building CellContexts in tests.
/// Substrate is auto-derived from Store via blanket impl.
#[derive(Default)]
struct TestStore {
    signals: std::sync::Mutex<Vec<Engram>>,
}

#[async_trait]
impl Store for TestStore {
    async fn put(&self, engram: Engram) -> Result<ContentHash> {
        let id = engram.id;
        self.signals.lock().unwrap().push(engram);
        Ok(id)
    }

    async fn get(&self, _id: &ContentHash) -> Result<Option<Engram>> {
        Ok(None)
    }

    async fn query(&self, _q: &Query, _ctx: &Context) -> Result<Vec<Engram>> {
        Ok(Vec::new())
    }

    async fn query_similar(
        &self,
        _fp: &HdcVector,
        _radius: f32,
        _limit: usize,
        _ctx: &Context,
    ) -> Result<Vec<(ContentHash, f32)>> {
        Ok(Vec::new())
    }

    async fn prune(&self, _threshold: f32, _ctx: &Context) -> Result<usize> {
        Ok(0)
    }

    async fn len(&self) -> Result<usize> {
        Ok(self.signals.lock().unwrap().len())
    }

    fn name(&self) -> &'static str {
        "test_store"
    }
}

fn make_context() -> CellContext {
    let bus: Arc<dyn BusErased> = Arc::new(MemoryBus::new(64));
    let store: Arc<dyn Substrate> = Arc::new(TestStore::default());
    let cancel = CancellationToken::new();
    CellContext::new(bus, store, cancel)
}

fn make_context_with_store(store: Arc<TestStore>) -> CellContext {
    let bus: Arc<dyn BusErased> = Arc::new(MemoryBus::new(64));
    let cancel = CancellationToken::new();
    CellContext::new(bus, store, cancel)
}

// ============================================================================
// Cell test doubles
// ============================================================================

/// A Cell that doubles its input signals (returns input ++ input).
struct DoubleCell;

#[async_trait]
impl Cell for DoubleCell {
    fn cell_id(&self) -> &str {
        "double"
    }
    fn cell_name(&self) -> &str {
        "Double Cell"
    }
    fn protocols(&self) -> &[&str] {
        &["Transform"]
    }
    fn cell_version(&self) -> CellVersion {
        (1, 0, 0)
    }
    fn input_schema(&self) -> Option<&TypeSchema> {
        None // accepts anything
    }

    async fn execute(&self, input: Vec<Engram>, _ctx: &CellContext) -> Result<Vec<Engram>> {
        let mut output = input.clone();
        output.extend(input);
        Ok(output)
    }
}

/// A Cell that does NOT override execute(), using the default error path.
struct StubCell;

#[async_trait]
impl Cell for StubCell {
    fn cell_id(&self) -> &str {
        "stub"
    }
    fn cell_name(&self) -> &str {
        "Stub"
    }
}

/// A Cell that passes input through unchanged.
struct PassthroughCell;

#[async_trait]
impl Cell for PassthroughCell {
    fn cell_id(&self) -> &str {
        "passthrough"
    }
    fn cell_name(&self) -> &str {
        "Passthrough Cell"
    }
    fn protocols(&self) -> &[&str] {
        &["Observe", "Transform"]
    }

    async fn execute(&self, input: Vec<Engram>, _ctx: &CellContext) -> Result<Vec<Engram>> {
        Ok(input)
    }
}

// ============================================================================
// Observe protocol test double
// ============================================================================

/// An observer cell that reports how many signals are in the store.
struct StoreCountObserver {
    count: std::sync::Mutex<usize>,
}

impl StoreCountObserver {
    fn new() -> Self {
        Self {
            count: std::sync::Mutex::new(0),
        }
    }

    fn set_count(&self, n: usize) {
        *self.count.lock().unwrap() = n;
    }
}

#[async_trait]
impl Cell for StoreCountObserver {
    fn cell_id(&self) -> &str {
        "store-count-observer"
    }
    fn cell_name(&self) -> &str {
        "Store Count Observer"
    }
    fn protocols(&self) -> &[&str] {
        &["Observe"]
    }
}

impl Observe for StoreCountObserver {
    fn observe(&self) -> Vec<Engram> {
        let count = *self.count.lock().unwrap();
        vec![Engram::builder(Kind::Metric)
            .body(Body::text(format!("signal_count={count}")))
            .tag("source", "store_observer")
            .tag("count", count.to_string())
            .build()]
    }
}

// ============================================================================
// Connect protocol test double
// ============================================================================

/// A connection cell with controllable health status.
struct MockConnection {
    connected: AtomicBool,
    healthy: AtomicBool,
}

impl MockConnection {
    fn new() -> Self {
        Self {
            connected: AtomicBool::new(false),
            healthy: AtomicBool::new(true),
        }
    }

    fn set_healthy(&self, v: bool) {
        self.healthy.store(v, Ordering::Relaxed);
    }
}

#[async_trait]
impl Cell for MockConnection {
    fn cell_id(&self) -> &str {
        "mock-connection"
    }
    fn cell_name(&self) -> &str {
        "Mock Connection"
    }
    fn protocols(&self) -> &[&str] {
        &["Connect"]
    }
}

impl Connect for MockConnection {
    fn connect(&self) -> Result<()> {
        self.connected.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn health(&self) -> bool {
        self.connected.load(Ordering::Relaxed) && self.healthy.load(Ordering::Relaxed)
    }

    fn disconnect(&self) -> Result<()> {
        self.connected.store(false, Ordering::Relaxed);
        Ok(())
    }
}

// ============================================================================
// Trigger protocol test double
// ============================================================================

/// A trigger cell that watches for a specific topic prefix on the bus.
struct TopicTrigger {
    armed: AtomicBool,
    topic_prefix: String,
}

impl TopicTrigger {
    fn new(prefix: impl Into<String>) -> Self {
        Self {
            armed: AtomicBool::new(false),
            topic_prefix: prefix.into(),
        }
    }

    fn is_armed(&self) -> bool {
        self.armed.load(Ordering::Relaxed)
    }

    /// Check if a pulse should fire this trigger.
    fn matches_pulse(&self, pulse: &Pulse) -> bool {
        self.is_armed() && pulse.topic.starts_with(&self.topic_prefix)
    }
}

#[async_trait]
impl Cell for TopicTrigger {
    fn cell_id(&self) -> &str {
        "topic-trigger"
    }
    fn cell_name(&self) -> &str {
        "Topic Trigger"
    }
    fn protocols(&self) -> &[&str] {
        &["Trigger"]
    }
}

impl Trigger for TopicTrigger {
    fn arm(&self) -> Result<()> {
        self.armed.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn disarm(&self) -> Result<()> {
        self.armed.store(false, Ordering::Relaxed);
        Ok(())
    }
}

// ============================================================================
// Tests: Cell::execute
// ============================================================================

#[tokio::test]
async fn cell_execute_produces_signals() {
    let ctx = make_context();
    let cell = DoubleCell;
    let input = vec![Engram::builder(Kind::Metric).build()];

    let output = cell.execute(input, &ctx).await.unwrap();
    assert_eq!(output.len(), 2, "DoubleCell should produce 2x signals");
}

#[tokio::test]
async fn default_execute_returns_error() {
    let ctx = make_context();
    let result = StubCell.execute(vec![], &ctx).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not implemented"),
        "expected 'not implemented' in error, got: {err_msg}"
    );
}

#[tokio::test]
async fn cell_execute_preserves_signal_identity() {
    let ctx = make_context();
    let signal = Engram::builder(Kind::Task)
        .body(Body::text("test payload"))
        .build();
    let original_id = signal.id;

    let output = PassthroughCell
        .execute(vec![signal], &ctx)
        .await
        .unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].id, original_id);
}

// ============================================================================
// Tests: TypeSchema compatibility
// ============================================================================

#[test]
fn type_schema_any_compatible_with_any() {
    assert!(TypeSchema::Any.is_compatible_with(&TypeSchema::Any));
}

#[test]
fn type_schema_of_kind_compatible_with_any() {
    assert!(TypeSchema::OfKind(Kind::Metric).is_compatible_with(&TypeSchema::Any));
    assert!(TypeSchema::Any.is_compatible_with(&TypeSchema::OfKind(Kind::Metric)));
}

#[test]
fn type_schema_same_kind_compatible() {
    assert!(
        TypeSchema::OfKind(Kind::Metric).is_compatible_with(&TypeSchema::OfKind(Kind::Metric))
    );
}

#[test]
fn type_schema_different_kind_incompatible() {
    assert!(!TypeSchema::OfKind(Kind::Metric).is_compatible_with(&TypeSchema::OfKind(Kind::Task)));
    assert!(!TypeSchema::OfKind(Kind::Task).is_compatible_with(&TypeSchema::OfKind(Kind::Metric)));
}

// ============================================================================
// Tests: Signal is the canonical type name
// ============================================================================

#[test]
fn signal_is_canonical_alias() {
    // Signal is a type alias for Engram -- verify both work and are identical.
    let signal: Signal = Signal::builder(Kind::Metric)
        .body(Body::text("from Signal"))
        .created_at_ms(1000)
        .build();
    let engram: Engram = Engram::builder(Kind::Metric)
        .body(Body::text("from Signal"))
        .created_at_ms(1000)
        .build();
    // Same content produces the same hash.
    assert_eq!(signal.id, engram.id);
}

#[test]
fn signal_builder_is_canonical_alias() {
    // SignalBuilder is a type alias for EngramBuilder.
    let _builder: SignalBuilder = Signal::builder(Kind::Task);
}

// ============================================================================
// Tests: Observe protocol
// ============================================================================

#[test]
fn observer_reports_signal_count() {
    let observer = StoreCountObserver::new();
    observer.set_count(5);

    let observations = observer.observe();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].kind, Kind::Metric);
    assert_eq!(observations[0].tag("count"), Some("5"));
    assert_eq!(observations[0].tag("source"), Some("store_observer"));
}

#[test]
fn observer_has_cell_identity() {
    let observer = StoreCountObserver::new();
    assert_eq!(observer.cell_id(), "store-count-observer");
    assert_eq!(observer.cell_name(), "Store Count Observer");
    assert!(observer.protocols().contains(&"Observe"));
}

// ============================================================================
// Tests: Connect protocol
// ============================================================================

#[test]
fn connect_lifecycle() {
    let conn = MockConnection::new();
    // Before connecting: not healthy (not connected yet).
    assert!(!conn.health());

    // Connect.
    conn.connect().unwrap();
    assert!(conn.health());

    // Degrade health.
    conn.set_healthy(false);
    assert!(!conn.health());

    // Restore health.
    conn.set_healthy(true);
    assert!(conn.health());

    // Disconnect.
    conn.disconnect().unwrap();
    assert!(!conn.health());
}

#[test]
fn connect_has_cell_identity() {
    let conn = MockConnection::new();
    assert_eq!(conn.cell_id(), "mock-connection");
    assert_eq!(conn.cell_name(), "Mock Connection");
    assert!(conn.protocols().contains(&"Connect"));
}

// ============================================================================
// Tests: Trigger protocol
// ============================================================================

#[test]
fn trigger_arm_disarm_lifecycle() {
    let trigger = TopicTrigger::new("observe.");
    assert!(!trigger.is_armed());

    // Arm.
    trigger.arm().unwrap();
    assert!(trigger.is_armed());

    // Check matching pulse fires.
    let matching_pulse = Pulse::new(
        1,
        Topic::new("observe.store.stats"),
        Kind::Metric,
        Body::text("count=5"),
    );
    assert!(trigger.matches_pulse(&matching_pulse));

    // Check non-matching pulse does not fire.
    let non_matching = Pulse::new(
        2,
        Topic::new("gate.compile"),
        Kind::GateVerdict,
        Body::text("pass"),
    );
    assert!(!trigger.matches_pulse(&non_matching));

    // Disarm.
    trigger.disarm().unwrap();
    assert!(!trigger.is_armed());

    // After disarm, matching pulse no longer fires.
    assert!(!trigger.matches_pulse(&matching_pulse));
}

#[test]
fn trigger_has_cell_identity() {
    let trigger = TopicTrigger::new("test.");
    assert_eq!(trigger.cell_id(), "topic-trigger");
    assert_eq!(trigger.cell_name(), "Topic Trigger");
    assert!(trigger.protocols().contains(&"Trigger"));
}

// ============================================================================
// Tests: Full pipeline -- observe -> trigger -> execute
// ============================================================================

#[tokio::test]
async fn observe_trigger_execute_pipeline() {
    // 1. Observe: collect observations from the store.
    let observer = StoreCountObserver::new();
    observer.set_count(3);
    let observations = observer.observe();
    assert!(!observations.is_empty(), "observer should produce signals");

    // 2. Trigger: arm a trigger for observation events and check it.
    let trigger = TopicTrigger::new("observe.");
    trigger.arm().unwrap();

    // Convert observation to a pulse on the bus (simulating the bus relay).
    let obs_signal = &observations[0];
    let pulse = Pulse::new(
        1,
        Topic::new("observe.store.stats"),
        obs_signal.kind.clone(),
        obs_signal.body.clone(),
    );

    assert!(
        trigger.matches_pulse(&pulse),
        "trigger should fire on matching observation pulse"
    );

    // 3. Execute: feed the observation signals into a cell.
    let cell = PassthroughCell;
    let ctx = make_context();
    let result = cell.execute(observations, &ctx).await.unwrap();
    assert_eq!(
        result.len(),
        1,
        "passthrough cell should return all input signals"
    );
    assert_eq!(result[0].kind, Kind::Metric);
}

#[tokio::test]
async fn bus_connects_observer_to_trigger_to_cell() {
    // End-to-end: publish an observation pulse on the bus, verify it arrives
    // at a subscriber, and then execute a cell with the observation data.

    let bus = MemoryBus::new(64);

    // Subscribe before publishing.
    let mut rx = bus.subscribe(TopicFilter::Prefix("observe.".into())).unwrap();

    // Observer produces observations.
    let observer = StoreCountObserver::new();
    observer.set_count(7);
    let observations = observer.observe();

    // Publish observation as a pulse on the bus.
    let obs = &observations[0];
    let pulse = Pulse::new(
        0,
        Topic::new("observe.store.stats"),
        obs.kind.clone(),
        obs.body.clone(),
    );
    let seq = bus.publish(pulse).unwrap();
    assert_eq!(seq, 0);

    // Subscriber receives the pulse.
    let received = rx.recv().await.unwrap();
    assert_eq!(received.topic, Topic::new("observe.store.stats"));

    // Trigger checks the received pulse.
    let trigger = TopicTrigger::new("observe.");
    trigger.arm().unwrap();
    assert!(trigger.matches_pulse(&received));

    // Execute a cell with the original observation signals.
    let cell = DoubleCell;
    let ctx = make_context();
    let result = cell.execute(observations, &ctx).await.unwrap();
    assert_eq!(result.len(), 2);
}

// ============================================================================
// Tests: Cell with CellContext store interaction
// ============================================================================

#[tokio::test]
async fn cell_can_interact_with_store_via_context() {
    let store = Arc::new(TestStore::default());
    let ctx = make_context_with_store(store.clone());

    // Write a signal to the store through the context.
    let signal = Engram::builder(Kind::Episode)
        .body(Body::text("test episode"))
        .build();
    let hash = ctx.store.put(signal).await.unwrap();
    assert_ne!(hash, ContentHash([0; 32]));

    // Verify the store received it.
    let count = store.signals.lock().unwrap().len();
    assert_eq!(count, 1);
}

// ============================================================================
// Tests: CellContext construction and fields
// ============================================================================

#[test]
fn cell_context_has_optional_fields() {
    let mut ctx = make_context();

    // Defaults are None.
    assert!(ctx.trace_id.is_none());
    assert!(ctx.run_id.is_none());
    assert!(ctx.budget_remaining.is_none());

    // Can be set.
    ctx.trace_id = Some("trace-001".to_string());
    ctx.run_id = Some("run-42".to_string());
    ctx.budget_remaining = Some(1.50);

    assert_eq!(ctx.trace_id.as_deref(), Some("trace-001"));
    assert_eq!(ctx.run_id.as_deref(), Some("run-42"));
    assert_eq!(ctx.budget_remaining, Some(1.50));
}

// ============================================================================
// Tests: Protocol composition -- multiple protocols on one Cell
// ============================================================================

#[test]
fn cell_can_declare_multiple_protocols() {
    let cell = PassthroughCell;
    let protos = cell.protocols();
    assert!(protos.contains(&"Observe"));
    assert!(protos.contains(&"Transform"));
    assert_eq!(protos.len(), 2);
}
