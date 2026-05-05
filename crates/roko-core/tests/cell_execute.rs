//! Integration tests for Cell::execute(), CellContext, and TypeSchema.

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use roko_core::error::{Result, RokoError};
use roko_core::{
    Body, BusErased, Cell, CellContext, CellVersion, ContentHash, Context, Engram, HdcVector,
    Kind, MemoryBus, Query, Substrate, TypeSchema,
};

// ─── TestStore ──────────────────────────────────────────────────────────────

/// A minimal no-op Substrate implementation for testing.
#[derive(Default)]
struct TestStore;

#[async_trait]
impl Substrate for TestStore {
    async fn put(&self, engram: Engram) -> Result<ContentHash> {
        Ok(engram.id)
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

    fn name(&self) -> &'static str {
        "test_store"
    }
}

// ─── EchoCell ───────────────────────────────────────────────────────────────

/// A Cell that echoes its input back as output.
struct EchoCell;

#[async_trait]
impl Cell for EchoCell {
    fn cell_id(&self) -> &str {
        "echo-cell-001"
    }

    fn cell_name(&self) -> &str {
        "EchoCell"
    }

    fn cell_version(&self) -> CellVersion {
        (1, 0, 0)
    }

    fn protocols(&self) -> &[&str] {
        &["Echo"]
    }

    async fn execute(&self, input: Vec<Engram>, _ctx: &CellContext) -> Result<Vec<Engram>> {
        Ok(input)
    }
}

// ─── DefaultOnlyCell ────────────────────────────────────────────────────────

/// A Cell that does NOT override execute(), relying on the default error impl.
struct DefaultOnlyCell;

#[async_trait]
impl Cell for DefaultOnlyCell {
    fn cell_id(&self) -> &str {
        "default-cell-001"
    }

    fn cell_name(&self) -> &str {
        "DefaultOnlyCell"
    }
}

// ─── Helper ─────────────────────────────────────────────────────────────────

fn make_context() -> CellContext {
    let bus: Arc<dyn BusErased> = Arc::new(MemoryBus::new(16));
    let store: Arc<dyn Substrate> = Arc::new(TestStore::default());
    let cancel = CancellationToken::new();
    CellContext::new(bus, store, cancel)
}

fn test_engram() -> Engram {
    Engram::builder(Kind::Task).body(Body::text("hello")).build()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cell_execute_echo_returns_input() {
    let ctx = make_context();
    let input = vec![test_engram()];
    let input_clone = input.clone();

    let cell = EchoCell;
    let result = cell.execute(input, &ctx).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].id, input_clone[0].id);
}

#[tokio::test]
async fn cell_execute_default_returns_error() {
    let ctx = make_context();
    let cell = DefaultOnlyCell;

    let result = cell.execute(Vec::new(), &ctx).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    match &err {
        RokoError::Invalid(msg) => {
            assert!(
                msg.contains("execute() not implemented"),
                "error message should mention execute() not implemented, got: {msg}"
            );
            assert!(
                msg.contains("DefaultOnlyCell"),
                "error message should contain cell name, got: {msg}"
            );
        }
        other => panic!("expected RokoError::Invalid, got: {other:?}"),
    }
}

#[tokio::test]
async fn cell_execute_with_empty_input() {
    let ctx = make_context();
    let cell = EchoCell;

    let result = cell.execute(Vec::new(), &ctx).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 0);
}

// ─── TypeSchema tests ───────────────────────────────────────────────────────

#[test]
fn type_schema_any_is_compatible_with_anything() {
    let any = TypeSchema::Any;
    let task = TypeSchema::OfKind(Kind::Task);
    let json = TypeSchema::JsonSchema("{}".to_string());

    assert!(any.is_compatible_with(&any));
    assert!(any.is_compatible_with(&task));
    assert!(any.is_compatible_with(&json));
    assert!(task.is_compatible_with(&any));
    assert!(json.is_compatible_with(&any));
}

#[test]
fn type_schema_matching_kinds_are_compatible() {
    let task1 = TypeSchema::OfKind(Kind::Task);
    let task2 = TypeSchema::OfKind(Kind::Task);

    assert!(task1.is_compatible_with(&task2));
}

#[test]
fn type_schema_mismatched_kinds_are_incompatible() {
    let task = TypeSchema::OfKind(Kind::Task);
    let metric = TypeSchema::OfKind(Kind::Metric);

    assert!(!task.is_compatible_with(&metric));
    assert!(!metric.is_compatible_with(&task));
}

#[test]
fn type_schema_json_schemas_are_incompatible_with_each_other() {
    // JsonSchema vs JsonSchema: not yet supported, returns false
    let a = TypeSchema::JsonSchema(r#"{"type":"object"}"#.to_string());
    let b = TypeSchema::JsonSchema(r#"{"type":"object"}"#.to_string());

    assert!(!a.is_compatible_with(&b));
}

#[test]
fn type_schema_json_schema_incompatible_with_of_kind() {
    let json = TypeSchema::JsonSchema("{}".to_string());
    let kind = TypeSchema::OfKind(Kind::Task);

    assert!(!json.is_compatible_with(&kind));
    assert!(!kind.is_compatible_with(&json));
}

// ─── Cell metadata tests ────────────────────────────────────────────────────

#[test]
fn cell_metadata_accessors() {
    let cell = EchoCell;
    assert_eq!(cell.cell_id(), "echo-cell-001");
    assert_eq!(cell.cell_name(), "EchoCell");
    assert_eq!(cell.cell_version(), (1, 0, 0));
    assert_eq!(cell.protocols(), &["Echo"]);
    assert!(cell.estimated_cost().is_none());
    assert!(cell.estimated_duration().is_none());
    assert!(cell.input_schema().is_none());
    assert!(cell.output_schema().is_none());
}

#[test]
fn cell_default_metadata() {
    let cell = DefaultOnlyCell;
    assert_eq!(cell.cell_version(), (0, 1, 0));
    assert_eq!(cell.protocols(), &[] as &[&str]);
    assert!(cell.estimated_cost().is_none());
    assert!(cell.estimated_duration().is_none());
}
