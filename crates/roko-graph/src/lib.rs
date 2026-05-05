<<<<<<< HEAD
//! `roko-graph` -- Graph execution engine for the Roko toolkit.
=======
//! `roko-graph` — Graph execution engine for the Roko toolkit.
>>>>>>> worktree-agent-aa5b2a60
//!
//! This crate provides the foundation for defining and executing directed acyclic
//! graphs (DAGs) of Cells. It includes:
//!
<<<<<<< HEAD
//! - **Cell** trait (universal computation unit for graph nodes)
=======
>>>>>>> worktree-agent-aa5b2a60
//! - **Types** (`Graph`, `Node`, `Edge`, `NodeId`, `EdgeCondition`, `GraphMetadata`)
//! - **Loader** (TOML parsing into `Graph` struct)
//! - **Registry** (`CellRegistry` for mapping cell type names to factory functions)
//! - **Topo** (topological sort, cycle detection, dependency resolution)
//!
//! # Example
//!
//! ```rust
<<<<<<< HEAD
//! use roko_graph::{loader, topo};
=======
//! use roko_graph::{loader, topo, Graph, GraphMetadata, Node, Edge};
>>>>>>> worktree-agent-aa5b2a60
//!
//! let toml_str = r#"
//! [graph]
//! name = "example"
//!
//! [[nodes]]
//! id = "step1"
//! cell_type = "noop"
//!
//! [[nodes]]
//! id = "step2"
//! cell_type = "noop"
//!
//! [[edges]]
//! from = "step1"
//! to = "step2"
//! "#;
//!
//! let graph = loader::load_from_str(toml_str).unwrap();
//! let order = topo::topological_order(&graph).unwrap();
//! assert_eq!(order, vec!["step1", "step2"]);
//! ```

<<<<<<< HEAD
pub mod cell;
pub mod engine;
=======
>>>>>>> worktree-agent-aa5b2a60
pub mod loader;
pub mod registry;
pub mod topo;
pub mod types;

// Re-export primary types at crate root for convenience.
<<<<<<< HEAD
pub use cell::{Cell, CellContext, CellVersion};
pub use engine::{GraphEngine, GraphOutput, NodeResult, NodeStatus, default_registry};
pub use registry::{CellFactory, CellRegistry};
pub use types::{
    Edge, EdgeCondition, Graph, GraphError, GraphMetadata, GraphNodeIdx, Node, NodeId,
};
=======
pub use registry::{CellFactory, CellRegistry};
pub use types::{Edge, EdgeCondition, Graph, GraphError, GraphMetadata, GraphNodeIdx, Node, NodeId};
>>>>>>> worktree-agent-aa5b2a60
