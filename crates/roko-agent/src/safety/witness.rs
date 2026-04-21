//! Five-vertex reasoning DAG with BLAKE3 content-addressed commitments.
//!
//! Each agent action is modeled as a chain of vertices forming a DAG:
//!
//! - **Observation**: Raw sensory input (tool output, file content, external data)
//! - **Prediction**: Model's predicted outcome before action
//! - **Decision**: Chosen action with justification
//! - **Resolution**: Actual outcome after action
//! - **NeuroEntry**: Durable knowledge written to the neuro store
//!
//! Vertices are content-addressed via BLAKE3 and linked by parent edges.
//! The DAG can be persisted to `.roko/witness.jsonl` and walked for
//! forensic replay or cross-agent verification.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use roko_core::ContentHash;
use serde::{Deserialize, Serialize};

/// The five vertex types in the reasoning chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VertexKind {
    /// Raw sensory input: tool output, file content, external data.
    Observation,
    /// Model's predicted outcome before taking an action.
    Prediction,
    /// The chosen action with justification.
    Decision,
    /// Actual outcome after the action was executed.
    Resolution,
    /// Durable knowledge written to the neuro store.
    NeuroEntry,
}

/// A single vertex in the witness DAG.
///
/// Each vertex is identified by the BLAKE3 hash of its canonical JSON content.
/// Parent edges link to prior vertices that contributed to this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessVertex {
    /// Content-addressed identifier (BLAKE3 of `content`).
    pub id: ContentHash,
    /// Vertex type in the reasoning chain.
    pub kind: VertexKind,
    /// Agent that produced this vertex.
    pub agent_id: String,
    /// Unix-millis timestamp.
    pub timestamp_ms: u64,
    /// DAG edges to parent vertices.
    pub parents: Vec<ContentHash>,
    /// Arbitrary structured content.
    pub content: serde_json::Value,
    /// Optional cryptographic signature over `id`.
    pub signature: Option<Vec<u8>>,
}

impl WitnessVertex {
    /// Create a new vertex, computing its content-addressed ID from the content.
    #[must_use]
    pub fn new(
        kind: VertexKind,
        agent_id: impl Into<String>,
        timestamp_ms: u64,
        parents: Vec<ContentHash>,
        content: serde_json::Value,
    ) -> Self {
        let canonical = serde_json::to_vec(&content).unwrap_or_default();
        let id = ContentHash::of(&canonical);
        Self {
            id,
            kind,
            agent_id: agent_id.into(),
            timestamp_ms,
            parents,
            content,
            signature: None,
        }
    }

    /// Attach a cryptographic signature to this vertex.
    #[must_use]
    pub fn with_signature(mut self, signature: Vec<u8>) -> Self {
        self.signature = Some(signature);
        self
    }

    /// Verify that this vertex's ID matches its content.
    #[must_use]
    pub fn verify_id(&self) -> bool {
        let canonical = serde_json::to_vec(&self.content).unwrap_or_default();
        let expected = ContentHash::of(&canonical);
        self.id == expected
    }
}

/// An integrity violation found during DAG verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityViolation {
    /// The vertex that failed verification.
    pub vertex_id: ContentHash,
    /// Description of the violation.
    pub detail: String,
}

/// In-memory witness DAG.
///
/// Stores vertices indexed by their content-addressed ID and supports
/// traversal, integrity verification, and serialization.
#[derive(Debug, Clone, Default)]
pub struct WitnessDag {
    /// Vertices indexed by content hash.
    vertices: HashMap<ContentHash, WitnessVertex>,
}

impl WitnessDag {
    /// Create an empty DAG.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a vertex to the DAG.
    ///
    /// Returns `true` if the vertex was newly inserted, `false` if it
    /// already existed (content-addressed dedup).
    pub fn add_vertex(&mut self, vertex: WitnessVertex) -> bool {
        use std::collections::hash_map::Entry;
        match self.vertices.entry(vertex.id) {
            Entry::Vacant(e) => {
                e.insert(vertex);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    /// Look up a vertex by its ID.
    #[must_use]
    pub fn get(&self, id: &ContentHash) -> Option<&WitnessVertex> {
        self.vertices.get(id)
    }

    /// Return the number of vertices in the DAG.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vertices.len()
    }

    /// Return whether the DAG is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Walk the DAG from a starting vertex, following parent edges (BFS).
    ///
    /// Returns vertices in breadth-first order starting from `start_id`.
    /// If `start_id` is not in the DAG, returns an empty vec.
    #[must_use]
    pub fn walk_from(&self, start_id: &ContentHash) -> Vec<&WitnessVertex> {
        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        let mut result = Vec::new();

        if let Some(start) = self.vertices.get(start_id) {
            queue.push_back(start);
            visited.insert(start.id);
        }

        while let Some(vertex) = queue.pop_front() {
            result.push(vertex);
            for parent_id in &vertex.parents {
                if visited.insert(*parent_id) {
                    if let Some(parent) = self.vertices.get(parent_id) {
                        queue.push_back(parent);
                    }
                }
            }
        }

        result
    }

    /// Verify integrity of all vertices in the DAG.
    ///
    /// Checks:
    /// 1. Each vertex's ID matches its content hash.
    /// 2. All parent references point to vertices that exist in the DAG.
    #[must_use]
    pub fn verify_integrity(&self) -> Vec<IntegrityViolation> {
        let mut violations = Vec::new();

        for (id, vertex) in &self.vertices {
            // Check content hash.
            if !vertex.verify_id() {
                violations.push(IntegrityViolation {
                    vertex_id: *id,
                    detail: "content hash mismatch: id does not match content".into(),
                });
            }

            // Check parent references.
            for parent_id in &vertex.parents {
                if !self.vertices.contains_key(parent_id) {
                    violations.push(IntegrityViolation {
                        vertex_id: *id,
                        detail: format!("dangling parent reference: {}", parent_id.short()),
                    });
                }
            }
        }

        violations
    }

    /// Return all vertices of a given kind.
    #[must_use]
    pub fn vertices_of_kind(&self, kind: VertexKind) -> Vec<&WitnessVertex> {
        self.vertices.values().filter(|v| v.kind == kind).collect()
    }

    /// Return all vertices for a given agent.
    #[must_use]
    pub fn vertices_for_agent(&self, agent_id: &str) -> Vec<&WitnessVertex> {
        self.vertices
            .values()
            .filter(|v| v.agent_id == agent_id)
            .collect()
    }
}

// ─── Persistence ────────────────────────────────────────────────────────

/// Append-only JSONL logger for witness vertices.
///
/// Each call to [`WitnessLogger::log`] serializes a [`WitnessVertex`] as a
/// single JSON line and appends it to the witness log file.
#[derive(Debug, Clone)]
pub struct WitnessLogger {
    path: PathBuf,
}

impl WitnessLogger {
    /// Create a logger that writes to the given path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Append a vertex to the log file.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be opened/written.
    pub fn log(&self, vertex: &WitnessVertex) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = serde_json::to_string(vertex)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(file, "{line}")
    }

    /// Read all vertices from the log file into a DAG.
    ///
    /// Returns an empty DAG if the file does not exist. Lines that fail
    /// to parse are silently skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read.
    pub fn read_all(&self) -> std::io::Result<WitnessDag> {
        let mut dag = WitnessDag::new();
        if !self.path.exists() {
            return Ok(dag);
        }
        let content = fs::read_to_string(&self.path)?;
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(vertex) = serde_json::from_str::<WitnessVertex>(line) {
                dag.add_vertex(vertex);
            }
        }
        Ok(dag)
    }

    /// Return the path to the witness log file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[test]
    fn vertex_id_is_content_addressed() {
        let v1 = WitnessVertex::new(
            VertexKind::Observation,
            "agent-1",
            now_ms(),
            vec![],
            serde_json::json!({"data": "hello"}),
        );
        let v2 = WitnessVertex::new(
            VertexKind::Observation,
            "agent-2",
            now_ms() + 1,
            vec![],
            serde_json::json!({"data": "hello"}),
        );
        // Same content -> same ID (content-addressed).
        assert_eq!(v1.id, v2.id);
    }

    #[test]
    fn different_content_different_id() {
        let v1 = WitnessVertex::new(
            VertexKind::Decision,
            "agent-1",
            now_ms(),
            vec![],
            serde_json::json!({"action": "write_file"}),
        );
        let v2 = WitnessVertex::new(
            VertexKind::Decision,
            "agent-1",
            now_ms(),
            vec![],
            serde_json::json!({"action": "read_file"}),
        );
        assert_ne!(v1.id, v2.id);
    }

    #[test]
    fn verify_id_passes_for_valid_vertex() {
        let v = WitnessVertex::new(
            VertexKind::Resolution,
            "agent-1",
            now_ms(),
            vec![],
            serde_json::json!({"result": "ok"}),
        );
        assert!(v.verify_id());
    }

    #[test]
    fn verify_id_fails_for_tampered_vertex() {
        let mut v = WitnessVertex::new(
            VertexKind::Resolution,
            "agent-1",
            now_ms(),
            vec![],
            serde_json::json!({"result": "ok"}),
        );
        // Tamper with the content after construction.
        v.content = serde_json::json!({"result": "tampered"});
        assert!(!v.verify_id());
    }

    #[test]
    fn dag_add_and_get() {
        let mut dag = WitnessDag::new();
        let v = WitnessVertex::new(
            VertexKind::Observation,
            "agent-1",
            now_ms(),
            vec![],
            serde_json::json!({"input": "data"}),
        );
        let id = v.id;
        assert!(dag.add_vertex(v));
        assert_eq!(dag.len(), 1);
        assert!(dag.get(&id).is_some());
    }

    #[test]
    fn dag_dedup_on_same_content() {
        let mut dag = WitnessDag::new();
        let v1 = WitnessVertex::new(
            VertexKind::Observation,
            "a",
            1,
            vec![],
            serde_json::json!({"x": 1}),
        );
        let v2 = WitnessVertex::new(
            VertexKind::Observation,
            "b",
            2,
            vec![],
            serde_json::json!({"x": 1}),
        );
        assert!(dag.add_vertex(v1));
        assert!(!dag.add_vertex(v2)); // duplicate content
        assert_eq!(dag.len(), 1);
    }

    #[test]
    fn dag_walk_from_follows_parents() {
        let mut dag = WitnessDag::new();
        let root = WitnessVertex::new(
            VertexKind::Observation,
            "a",
            1,
            vec![],
            serde_json::json!({"root": true}),
        );
        let root_id = root.id;

        let child = WitnessVertex::new(
            VertexKind::Decision,
            "a",
            2,
            vec![root_id],
            serde_json::json!({"child": true}),
        );
        let child_id = child.id;

        let grandchild = WitnessVertex::new(
            VertexKind::Resolution,
            "a",
            3,
            vec![child_id],
            serde_json::json!({"grandchild": true}),
        );
        let gc_id = grandchild.id;

        dag.add_vertex(root);
        dag.add_vertex(child);
        dag.add_vertex(grandchild);

        let walked = dag.walk_from(&gc_id);
        assert_eq!(walked.len(), 3);
        assert_eq!(walked[0].id, gc_id);
        assert_eq!(walked[1].id, child_id);
        assert_eq!(walked[2].id, root_id);
    }

    #[test]
    fn dag_verify_integrity_clean() {
        let mut dag = WitnessDag::new();
        let root = WitnessVertex::new(
            VertexKind::Observation,
            "a",
            1,
            vec![],
            serde_json::json!({"ok": true}),
        );
        let child = WitnessVertex::new(
            VertexKind::Decision,
            "a",
            2,
            vec![root.id],
            serde_json::json!({"decision": "go"}),
        );
        dag.add_vertex(root);
        dag.add_vertex(child);

        let violations = dag.verify_integrity();
        assert!(violations.is_empty());
    }

    #[test]
    fn dag_verify_integrity_detects_dangling_parent() {
        let mut dag = WitnessDag::new();
        let dangling_id = ContentHash::of(b"nonexistent");
        let v = WitnessVertex::new(
            VertexKind::Decision,
            "a",
            1,
            vec![dangling_id],
            serde_json::json!({"action": "x"}),
        );
        dag.add_vertex(v);

        let violations = dag.verify_integrity();
        assert_eq!(violations.len(), 1);
        assert!(violations[0].detail.contains("dangling parent"));
    }

    #[test]
    fn dag_vertices_of_kind() {
        let mut dag = WitnessDag::new();
        dag.add_vertex(WitnessVertex::new(
            VertexKind::Observation,
            "a",
            1,
            vec![],
            serde_json::json!({"obs": 1}),
        ));
        dag.add_vertex(WitnessVertex::new(
            VertexKind::Decision,
            "a",
            2,
            vec![],
            serde_json::json!({"dec": 1}),
        ));
        dag.add_vertex(WitnessVertex::new(
            VertexKind::Observation,
            "a",
            3,
            vec![],
            serde_json::json!({"obs": 2}),
        ));

        assert_eq!(dag.vertices_of_kind(VertexKind::Observation).len(), 2);
        assert_eq!(dag.vertices_of_kind(VertexKind::Decision).len(), 1);
        assert_eq!(dag.vertices_of_kind(VertexKind::Resolution).len(), 0);
    }

    #[test]
    fn logger_writes_and_reads() {
        let tmp = tempfile::tempdir().unwrap();
        let logger = WitnessLogger::new(tmp.path().join("witness.jsonl"));

        let v1 = WitnessVertex::new(
            VertexKind::Observation,
            "a",
            100,
            vec![],
            serde_json::json!({"step": 1}),
        );
        let v2 = WitnessVertex::new(
            VertexKind::Decision,
            "a",
            200,
            vec![v1.id],
            serde_json::json!({"step": 2}),
        );

        logger.log(&v1).unwrap();
        logger.log(&v2).unwrap();

        let dag = logger.read_all().unwrap();
        assert_eq!(dag.len(), 2);
        assert!(dag.get(&v1.id).is_some());
        assert!(dag.get(&v2.id).is_some());
    }

    #[test]
    fn logger_empty_file_returns_empty_dag() {
        let tmp = tempfile::tempdir().unwrap();
        let logger = WitnessLogger::new(tmp.path().join("empty.jsonl"));
        let dag = logger.read_all().unwrap();
        assert!(dag.is_empty());
    }

    #[test]
    fn vertex_round_trips_through_serde() {
        let v = WitnessVertex::new(
            VertexKind::NeuroEntry,
            "agent-x",
            999,
            vec![ContentHash::of(b"parent")],
            serde_json::json!({"knowledge": "distilled"}),
        )
        .with_signature(vec![1, 2, 3]);

        let json = serde_json::to_string(&v).unwrap();
        let decoded: WitnessVertex = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, v.id);
        assert_eq!(decoded.kind, VertexKind::NeuroEntry);
        assert_eq!(decoded.agent_id, "agent-x");
        assert_eq!(decoded.signature, Some(vec![1, 2, 3]));
    }
}
