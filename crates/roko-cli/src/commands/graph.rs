//! `roko graph` subcommand handlers: run, validate, and show graph definitions.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use clap::Subcommand;

use roko_graph::{CellContext, GraphEngine, default_registry, loader};

/// Exit code for success.
const EXIT_SUCCESS: i32 = 0;
/// Exit code for failure.
const EXIT_FAILURE: i32 = 1;

/// Subcommands for `roko graph`.
#[derive(Debug, Subcommand)]
pub enum GraphCmd {
    /// Execute a graph definition file.
    #[command(after_help = "\
Examples:
  roko graph run examples/graphs/linear-gates.toml
  roko graph run my-pipeline.toml")]
    Run {
        /// Path to a graph TOML file.
        path: PathBuf,
    },
    /// Validate a graph definition (check for cycles, unknown cell types, unresolved refs).
    #[command(after_help = "\
Examples:
  roko graph validate examples/graphs/linear-gates.toml
  roko graph validate my-pipeline.toml")]
    Validate {
        /// Path to a graph TOML file.
        path: PathBuf,
    },
    /// Show a summary of nodes and edges in a graph definition.
    #[command(after_help = "\
Examples:
  roko graph show examples/graphs/linear-gates.toml")]
    Show {
        /// Path to a graph TOML file.
        path: PathBuf,
    },
}

/// Dispatch a `roko graph` subcommand.
pub async fn cmd_graph(cmd: GraphCmd) -> Result<i32> {
    match cmd {
        GraphCmd::Run { path } => cmd_graph_run(&path).await,
        GraphCmd::Validate { path } => cmd_graph_validate(&path),
        GraphCmd::Show { path } => cmd_graph_show(&path),
    }
}

/// Execute a graph: load the TOML, build the engine with the default registry,
/// run all nodes sequentially, and print the results.
async fn cmd_graph_run(path: &Path) -> Result<i32> {
    let graph = loader::load_from_file(path)
        .map_err(|e| anyhow!("failed to load graph '{}': {}", path.display(), e))?;

    let registry = default_registry();
    let engine = GraphEngine::new(graph, registry);
    let ctx = CellContext::new();

    // Validate before running
    let issues = engine.validate();
    if !issues.is_empty() {
        eprintln!("validation errors:");
        for issue in &issues {
            eprintln!("  - {issue}");
        }
        return Ok(EXIT_FAILURE);
    }

    let output = engine
        .execute(&ctx)
        .await
        .map_err(|e| anyhow!("graph execution error: {e}"))?;

    // Print the summary
    println!("{}", output.summary());

    if output.success {
        Ok(EXIT_SUCCESS)
    } else {
        Ok(EXIT_FAILURE)
    }
}

/// Validate a graph definition without executing it.
fn cmd_graph_validate(path: &Path) -> Result<i32> {
    let graph = loader::load_from_file(path)
        .map_err(|e| anyhow!("failed to load graph '{}': {}", path.display(), e))?;

    let registry = default_registry();
    let engine = GraphEngine::new(graph, registry);
    let issues = engine.validate();

    if issues.is_empty() {
        println!("graph '{}' is valid", path.display());
        Ok(EXIT_SUCCESS)
    } else {
        println!("graph '{}' has {} issue(s):", path.display(), issues.len());
        for issue in &issues {
            println!("  - {issue}");
        }
        Ok(EXIT_FAILURE)
    }
}

/// Show a summary of nodes and edges in a graph.
fn cmd_graph_show(path: &Path) -> Result<i32> {
    let graph = loader::load_from_file(path)
        .map_err(|e| anyhow!("failed to load graph '{}': {}", path.display(), e))?;

    println!("Graph: {}", graph.metadata.name);
    if let Some(desc) = &graph.metadata.description {
        println!("Description: {desc}");
    }
    if let Some(ver) = &graph.metadata.version {
        println!("Version: {ver}");
    }
    println!();

    // Print nodes
    println!("Nodes ({}):", graph.node_count());
    for (node_id, idx) in &graph.node_map {
        let node = &graph.inner[*idx];
        println!("  {node_id}  [cell_type: {}]", node.cell_type);
        if !node.inputs.is_empty() {
            println!("    inputs: {}", node.inputs.join(", "));
        }
        if !node.outputs.is_empty() {
            println!("    outputs: {}", node.outputs.join(", "));
        }
    }
    println!();

    // Print edges using the raw edge indices from petgraph
    println!("Edges ({}):", graph.edge_count());
    for edge_idx in graph.inner.edge_indices() {
        let edge = &graph.inner[edge_idx];
        let cond = match &edge.condition {
            Some(c) => format!(" [condition: {c:?}]"),
            None => String::new(),
        };
        println!("  {} -> {}{}", edge.from, edge.to, cond);
    }

    // Print topological order if DAG
    if let Ok(order) = roko_graph::topo::topological_order(&graph) {
        println!();
        println!("Execution order: {}", order.join(" -> "));
    }

    Ok(EXIT_SUCCESS)
}
