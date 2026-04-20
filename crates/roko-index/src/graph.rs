//! Symbol dependency graph with `PageRank` scoring.
//!
//! Builds a directed graph from parsed [`SourceFile`]s where nodes are symbols
//! and edges represent relationships (imports, calls, containment). `PageRank`
//! over the graph produces importance scores for prioritising context.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::LazyLock;

use crate::parser::SourceFile;
use crate::symbol::SymbolId;
use regex::Regex;
use roko_core::language::SymbolKind;

static CALL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(").expect("call regex"));

/// Matches `PascalCase` identifiers that likely refer to types.
static TYPE_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Z][A-Za-z0-9_]*)\b").expect("type ref regex")
});

// ─── Edge kinds ─────────────────────────────────────────────────────────

/// The kind of relationship between two symbols.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
#[non_exhaustive]
pub enum EdgeKind {
    /// One symbol calls another.
    Calls,
    /// One symbol imports another.
    Imports,
    /// One symbol implements a trait/interface.
    Implements,
    /// One symbol is contained within another (e.g. method in impl block).
    Contains,
    /// One symbol references a type (struct/enum/trait) in its signature or body.
    TypeRef,
}

/// A directed edge in the symbol graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub struct SymbolEdge {
    /// Source symbol.
    pub from_id: SymbolId,
    /// Target symbol.
    pub to_id: SymbolId,
    /// Relationship kind.
    pub kind: EdgeKind,
}

// ─── SymbolGraph ────────────────────────────────────────────────────────

/// A directed graph of symbol dependencies.
///
/// Nodes are [`SymbolId`]s. Edges record how symbols depend on or relate to
/// each other. The graph supports neighbor queries and `PageRank` scoring.
#[derive(Clone, Debug)]
pub struct SymbolGraph {
    /// Set of all node ids.
    nodes: HashSet<SymbolId>,
    /// Forward edges: from -> list of (to, kind).
    forward: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
    /// Reverse edges: to -> list of (from, kind).
    reverse: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
}

impl SymbolGraph {
    /// Number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.forward.values().map(Vec::len).sum()
    }

    /// Number of edges in the graph for one edge kind.
    pub fn edge_count_by_kind(&self, kind: EdgeKind) -> usize {
        self.forward
            .values()
            .map(|edges| {
                edges
                    .iter()
                    .filter(|(_, edge_kind)| *edge_kind == kind)
                    .count()
            })
            .sum()
    }

    /// Return the forward neighbors (dependencies) of a node.
    pub fn neighbors(&self, id: &SymbolId) -> Vec<&SymbolId> {
        self.forward
            .get(id)
            .map(|edges| edges.iter().map(|(target, _)| target).collect())
            .unwrap_or_default()
    }

    /// Return the reverse neighbors (dependents/callers) of a node.
    pub fn reverse_neighbors(&self, id: &SymbolId) -> Vec<&SymbolId> {
        self.reverse
            .get(id)
            .map(|edges| edges.iter().map(|(source, _)| source).collect())
            .unwrap_or_default()
    }

    /// Return forward neighbors filtered by edge kind.
    pub fn neighbors_by_kind(&self, id: &SymbolId, kind: EdgeKind) -> Vec<&SymbolId> {
        let mut seen = HashSet::new();
        self.forward
            .get(id)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|(target, edge_kind)| {
                        if *edge_kind == kind.clone() && seen.insert((*target).clone()) {
                            Some(target)
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return reverse neighbors filtered by edge kind.
    pub fn reverse_neighbors_by_kind(&self, id: &SymbolId, kind: EdgeKind) -> Vec<&SymbolId> {
        let mut seen = HashSet::new();
        self.reverse
            .get(id)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|(source, edge_kind)| {
                        if *edge_kind == kind.clone() && seen.insert((*source).clone()) {
                            Some(source)
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// BFS from `start` up to `max_depth` hops following forward edges.
    pub fn transitive(&self, start: &SymbolId, max_depth: usize) -> Vec<(SymbolId, usize)> {
        if max_depth == 0 || !self.nodes.contains(start) {
            return Vec::new();
        }

        let mut visited = HashSet::new();
        visited.insert(start.clone());
        let mut queue = VecDeque::new();
        queue.push_back((start.clone(), 0usize));
        let mut results = Vec::new();

        while let Some((node, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for neighbor in self.neighbors(&node) {
                if visited.insert(neighbor.clone()) {
                    let d = depth + 1;
                    results.push((neighbor.clone(), d));
                    queue.push_back((neighbor.clone(), d));
                }
            }
        }

        results
    }
}

// ─── rkyv zero-copy snapshots ────────────────────────────────────────────

/// Flat snapshot of a graph's edges for rkyv serialization.
///
/// The graph's internal `HashMap`/`HashSet` structures do not derive rkyv
/// directly, so we serialize a flat edge list instead and rebuild on load.
#[cfg(feature = "rkyv")]
#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SymbolGraphSnapshot {
    /// All edges as `(from, to, kind)` triples.
    pub edges: Vec<SymbolEdge>,
}

#[cfg(feature = "rkyv")]
impl SymbolGraph {
    /// Create a flat snapshot suitable for rkyv serialization.
    #[must_use]
    pub fn snapshot(&self) -> SymbolGraphSnapshot {
        let mut edges = Vec::new();
        for (from_id, targets) in &self.forward {
            for (to_id, kind) in targets {
                edges.push(SymbolEdge {
                    from_id: from_id.clone(),
                    to_id: to_id.clone(),
                    kind: kind.clone(),
                });
            }
        }
        SymbolGraphSnapshot { edges }
    }

    /// Rebuild a graph from a snapshot.
    #[must_use]
    pub fn from_snapshot(snapshot: &SymbolGraphSnapshot) -> Self {
        let mut nodes = HashSet::new();
        let mut forward: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>> = HashMap::new();
        let mut reverse: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>> = HashMap::new();
        for edge in &snapshot.edges {
            nodes.insert(edge.from_id.clone());
            nodes.insert(edge.to_id.clone());
            forward
                .entry(edge.from_id.clone())
                .or_default()
                .push((edge.to_id.clone(), edge.kind.clone()));
            reverse
                .entry(edge.to_id.clone())
                .or_default()
                .push((edge.from_id.clone(), edge.kind.clone()));
        }
        Self { nodes, forward, reverse }
    }

    /// Serialize the graph to an rkyv archive and write it to `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the graph cannot be serialized or the file cannot
    /// be written.
    pub fn save_rkyv(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = self.snapshot();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&snapshot)
            .map_err(|e| format!("rkyv ser: {e}"))?;
        std::fs::write(path, &bytes)?;
        Ok(())
    }

    /// Load a graph from an rkyv archive at `path`.
    ///
    /// Deserializes the snapshot and rebuilds the graph indices.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or the archive is invalid.
    pub fn load_rkyv(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let bytes = std::fs::read(path)?;
        let archived = rkyv::access::<ArchivedSymbolGraphSnapshot, rkyv::rancor::Error>(&bytes)
            .map_err(|e| format!("rkyv access: {e}"))?;
        let snapshot: SymbolGraphSnapshot =
            rkyv::deserialize::<SymbolGraphSnapshot, rkyv::rancor::Error>(archived)
                .map_err(|e| format!("rkyv deser: {e}"))?;
        Ok(Self::from_snapshot(&snapshot))
    }
}

// ─── Graph construction ─────────────────────────────────────────────────

/// Build a [`SymbolGraph`] from a set of parsed source files.
///
/// Edges are inferred by matching import paths against defined symbol names:
/// if file A imports a path ending in `Foo` and file B defines a symbol named
/// `Foo`, an `Imports` edge is created from A's importing context to B's `Foo`.
pub fn build_graph(files: &[SourceFile]) -> SymbolGraph {
    let mut nodes = HashSet::new();
    let mut forward: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>> = HashMap::new();
    let mut reverse: HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>> = HashMap::new();
    let mut seen_edges: HashSet<(SymbolId, SymbolId, EdgeKind)> = HashSet::new();

    // Phase 1: register all symbols as nodes.
    for file in files {
        for sym in &file.symbols {
            nodes.insert(SymbolId::from_symbol(sym, &file.path));
        }
    }

    // Phase 2: build name -> SymbolId lookups for import and call resolution.
    let mut name_to_ids: HashMap<&str, Vec<SymbolId>> = HashMap::new();
    let mut function_name_to_ids: HashMap<&str, Vec<SymbolId>> = HashMap::new();
    let mut type_name_to_ids: HashMap<&str, Vec<SymbolId>> = HashMap::new();
    for file in files {
        for sym in &file.symbols {
            let id = SymbolId::from_symbol(sym, &file.path);
            name_to_ids.entry(&sym.name).or_default().push(id.clone());
            if sym.kind == SymbolKind::Function {
                function_name_to_ids.entry(&sym.name).or_default().push(id);
            } else if matches!(
                sym.kind,
                SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait | SymbolKind::Type
            ) {
                type_name_to_ids.entry(&sym.name).or_default().push(id);
            }
        }
    }

    // Phase 3: create import edges.
    // For each file, look at its imports. The last segment of an import path
    // is matched against known symbol names to create edges.
    for file in files {
        // If the file has symbols, pick the first one as the "file-level" node
        // that will be the source of import edges. If there are no symbols we
        // skip because there is nothing to link from.
        let source_id = match file.symbols.first() {
            Some(sym) => SymbolId::from_symbol(sym, &file.path),
            None => continue,
        };

        for import in &file.imports {
            // Extract the last segment of the import path.
            let target_name = import
                .path
                .rsplit("::")
                .next()
                .or_else(|| import.path.rsplit('/').next())
                .or_else(|| import.path.rsplit('.').next())
                .unwrap_or(&import.path);

            if let Some(targets) = name_to_ids.get(target_name) {
                for target in targets {
                    if target.file_path == file.path {
                        continue;
                    }

                    add_edge(
                        &mut forward,
                        &mut reverse,
                        &mut seen_edges,
                        source_id.clone(),
                        target.clone(),
                        EdgeKind::Imports,
                    );
                }
            }
        }
    }

    // Phase 4: infer call edges from function bodies.
    for file in files {
        let total_lines = file.content.lines().count();
        if total_lines == 0 {
            continue;
        }

        let mut symbols = file.symbols.iter().collect::<Vec<_>>();
        symbols.sort_by_key(|sym| sym.line);

        for (index, sym) in symbols.iter().enumerate() {
            if sym.kind != SymbolKind::Function {
                continue;
            }

            let start_line = sym.line;
            if start_line == 0 || start_line > total_lines {
                continue;
            }

            let end_line = symbols
                .get(index + 1)
                .map(|next| next.line.saturating_sub(1))
                .unwrap_or(total_lines);

            if end_line < start_line {
                continue;
            }

            let source_id = SymbolId::from_symbol(sym, &file.path);
            for (line_idx, line) in file
                .content
                .lines()
                .enumerate()
                .skip(start_line.saturating_sub(1))
                .take(end_line.saturating_sub(start_line).saturating_add(1))
            {
                let line_number = line_idx + 1;
                for capture in CALL_RE.captures_iter(line) {
                    let candidate = capture.get(1).map(|m| m.as_str()).unwrap_or_default();
                    if candidate.is_empty() {
                        continue;
                    }
                    if line_number == start_line && candidate == sym.name {
                        continue;
                    }

                    if let Some(targets) = function_name_to_ids.get(candidate) {
                        for target in targets {
                            add_edge(
                                &mut forward,
                                &mut reverse,
                                &mut seen_edges,
                                source_id.clone(),
                                target.clone(),
                                EdgeKind::Calls,
                            );
                        }
                    }
                }
            }
        }
    }

    // Phase 5: infer TypeRef edges from function signatures and bodies.
    // When a PascalCase identifier in a function matches a known type name, add
    // a TypeRef edge.  This is heuristic — no full type resolution needed.
    if !type_name_to_ids.is_empty() {
        for file in files {
            let total_lines = file.content.lines().count();
            if total_lines == 0 {
                continue;
            }

            let mut symbols = file.symbols.iter().collect::<Vec<_>>();
            symbols.sort_by_key(|sym| sym.line);

            for (index, sym) in symbols.iter().enumerate() {
                if sym.kind != SymbolKind::Function {
                    continue;
                }

                let start_line = sym.line;
                if start_line == 0 || start_line > total_lines {
                    continue;
                }

                let end_line = symbols
                    .get(index + 1)
                    .map(|next| next.line.saturating_sub(1))
                    .unwrap_or(total_lines);

                if end_line < start_line {
                    continue;
                }

                let source_id = SymbolId::from_symbol(sym, &file.path);
                for (_line_idx, line) in file
                    .content
                    .lines()
                    .enumerate()
                    .skip(start_line.saturating_sub(1))
                    .take(end_line.saturating_sub(start_line).saturating_add(1))
                {
                    for capture in TYPE_REF_RE.captures_iter(line) {
                        let candidate =
                            capture.get(1).map(|m| m.as_str()).unwrap_or_default();
                        if candidate.is_empty() {
                            continue;
                        }

                        if let Some(targets) = type_name_to_ids.get(candidate) {
                            for target in targets {
                                add_edge(
                                    &mut forward,
                                    &mut reverse,
                                    &mut seen_edges,
                                    source_id.clone(),
                                    target.clone(),
                                    EdgeKind::TypeRef,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Phase 6: infer Contains edges from scope nesting.
    // Functions/methods that appear between an impl/mod/trait symbol and the
    // next symbol at the same scope level are considered "contained" by that
    // parent. Heuristic: symbols sorted by line number; impl/mod/trait symbols
    // "contain" subsequent function symbols until the next non-function symbol
    // of the same or higher kind.
    for file in files {
        let mut symbols = file.symbols.iter().collect::<Vec<_>>();
        symbols.sort_by_key(|sym| sym.line);

        let mut current_parent: Option<SymbolId> = None;
        for sym in &symbols {
            if matches!(
                sym.kind,
                SymbolKind::Impl | SymbolKind::Module | SymbolKind::Trait
            ) {
                // This is a container — set as current parent.
                current_parent = Some(SymbolId::from_symbol(sym, &file.path));
            } else if matches!(
                sym.kind,
                SymbolKind::Struct | SymbolKind::Enum
            ) {
                // Top-level type resets the containment scope.
                current_parent = None;
            } else if sym.kind == SymbolKind::Function {
                // Function after a container: emit Contains edge.
                if let Some(ref parent_id) = current_parent {
                    let child_id = SymbolId::from_symbol(sym, &file.path);
                    if *parent_id != child_id {
                        add_edge(
                            &mut forward,
                            &mut reverse,
                            &mut seen_edges,
                            parent_id.clone(),
                            child_id,
                            EdgeKind::Contains,
                        );
                    }
                }
            }
        }
    }

    // Phase 7: infer Implements edges from `impl Trait for Type` symbols.
    // The Rust parser produces Impl symbols named "Trait for Type". We split
    // on " for " to resolve both the trait and the type, then emit edges:
    //   Type --Implements--> Trait
    for file in files {
        for sym in &file.symbols {
            if sym.kind != SymbolKind::Impl {
                continue;
            }
            // Check for "Trait for Type" pattern.
            if let Some(for_pos) = sym.name.find(" for ") {
                let trait_name = &sym.name[..for_pos];
                let type_name = &sym.name[for_pos + 5..];

                // Resolve the trait and type to existing SymbolIds.
                let trait_ids = name_to_ids.get(trait_name).cloned().unwrap_or_default();
                let type_ids = name_to_ids.get(type_name).cloned().unwrap_or_default();

                for type_id in &type_ids {
                    for trait_id in &trait_ids {
                        // Type implements Trait: edge from type to trait.
                        add_edge(
                            &mut forward,
                            &mut reverse,
                            &mut seen_edges,
                            type_id.clone(),
                            trait_id.clone(),
                            EdgeKind::Implements,
                        );
                    }
                }
            }
        }
    }

    SymbolGraph {
        nodes,
        forward,
        reverse,
    }
}

fn add_edge(
    forward: &mut HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
    reverse: &mut HashMap<SymbolId, Vec<(SymbolId, EdgeKind)>>,
    seen: &mut HashSet<(SymbolId, SymbolId, EdgeKind)>,
    from: SymbolId,
    to: SymbolId,
    kind: EdgeKind,
) {
    if !seen.insert((from.clone(), to.clone(), kind.clone())) {
        return;
    }

    forward
        .entry(from.clone())
        .or_default()
        .push((to.clone(), kind.clone()));
    reverse.entry(to).or_default().push((from, kind));
}

// ─── PageRank ───────────────────────────────────────────────────────────

/// Compute `PageRank` scores over a [`SymbolGraph`].
///
/// Returns a map from `SymbolId` to its rank score. `iterations` controls
/// convergence depth; `damping` is the standard `PageRank` damping factor
/// (typically 0.85).
#[allow(clippy::cast_precision_loss)]
pub fn pagerank(graph: &SymbolGraph, iterations: u32, damping: f64) -> HashMap<SymbolId, f64> {
    let all_nodes: Vec<&SymbolId> = graph.nodes.iter().collect();
    let n = all_nodes.len();
    if n == 0 {
        return HashMap::new();
    }

    let n_f = n as f64;
    let mut rank: HashMap<SymbolId, f64> = all_nodes
        .iter()
        .map(|id| ((*id).clone(), 1.0 / n_f))
        .collect();

    for _ in 0..iterations {
        let mut new_rank: HashMap<SymbolId, f64> = HashMap::with_capacity(n);
        let base = (1.0 - damping) / n_f;

        for &node in &all_nodes {
            let mut incoming_sum = 0.0_f64;
            if let Some(inbound) = graph.reverse.get(node) {
                for (src, _) in inbound {
                    let src_rank = rank.get(src).copied().unwrap_or(0.0);
                    let out_degree = graph.forward.get(src).map_or(1, Vec::len).max(1) as f64;
                    incoming_sum += src_rank / out_degree;
                }
            }
            new_rank.insert(node.clone(), damping.mul_add(incoming_sum, base));
        }

        rank = new_rank;
    }

    rank
}

/// Weight assigned to each edge kind for weighted PageRank.
fn edge_weight(kind: &EdgeKind) -> f64 {
    match kind {
        EdgeKind::Imports => 1.0,
        EdgeKind::Calls => 0.8,
        EdgeKind::Implements => 0.9,
        EdgeKind::Contains => 0.6,
        EdgeKind::TypeRef => 0.5,
    }
}

/// Compute weighted `PageRank` scores over a [`SymbolGraph`].
///
/// Like [`pagerank`], but edge weights based on [`EdgeKind`] importance
/// influence the transfer matrix. Import edges carry full weight (1.0),
/// call edges 0.8, implements 0.9, contains 0.6, and type-ref edges 0.5.
#[allow(clippy::cast_precision_loss)]
pub fn weighted_pagerank(
    graph: &SymbolGraph,
    damping: f64,
    iterations: u32,
) -> HashMap<SymbolId, f64> {
    let all_nodes: Vec<&SymbolId> = graph.nodes.iter().collect();
    let n = all_nodes.len();
    if n == 0 {
        return HashMap::new();
    }

    // Pre-compute weighted out-degree for each node.
    let weighted_out: HashMap<&SymbolId, f64> = all_nodes
        .iter()
        .map(|&id| {
            let w = graph
                .forward
                .get(id)
                .map(|edges| edges.iter().map(|(_, kind)| edge_weight(kind)).sum::<f64>())
                .unwrap_or(0.0)
                .max(f64::MIN_POSITIVE);
            (id, w)
        })
        .collect();

    let n_f = n as f64;
    let mut rank: HashMap<SymbolId, f64> = all_nodes
        .iter()
        .map(|id| ((*id).clone(), 1.0 / n_f))
        .collect();

    for _ in 0..iterations {
        let mut new_rank: HashMap<SymbolId, f64> = HashMap::with_capacity(n);
        let base = (1.0 - damping) / n_f;

        for &node in &all_nodes {
            let mut incoming_sum = 0.0_f64;
            if let Some(inbound) = graph.reverse.get(node) {
                for (src, kind) in inbound {
                    let src_rank = rank.get(src).copied().unwrap_or(0.0);
                    let w = edge_weight(kind);
                    let out_w = weighted_out.get(src).copied().unwrap_or(1.0);
                    incoming_sum += src_rank * w / out_w;
                }
            }
            new_rank.insert(node.clone(), damping.mul_add(incoming_sum, base));
        }

        rank = new_rank;
    }

    rank
}

/// Compute Personalized PageRank (PPR) biased toward a set of seed nodes.
///
/// Instead of uniform teleport (`(1-d)/N` to all nodes), teleport probability
/// is concentrated on the seed nodes:
/// ```text
/// rank_new[j] = (1-d) * seed_weight[j]
///             + d * sum(rank[i] * edge_weight(i,j) / out_weight[i])
/// ```
/// where `seed_weight[j] = 1.0 / |seeds|` if `j` is a seed, else `0.0`.
///
/// Uses edge weights from [`edge_weight`] (same as [`weighted_pagerank`]).
/// Surfaces task-relevant symbols that might have low global importance but are
/// critical for the current context window.
#[allow(clippy::cast_precision_loss)]
pub fn personalized_pagerank(
    graph: &SymbolGraph,
    seed_nodes: &[SymbolId],
    damping: f64,
    iterations: u32,
) -> HashMap<SymbolId, f64> {
    let all_nodes: Vec<&SymbolId> = graph.nodes.iter().collect();
    let n = all_nodes.len();
    if n == 0 {
        return HashMap::new();
    }

    // Build the seed set for O(1) lookup.
    let seed_set: HashSet<&SymbolId> = seed_nodes.iter().collect();
    let seed_count = seed_set.len().max(1) as f64;

    // Pre-compute weighted out-degree for each node.
    let weighted_out: HashMap<&SymbolId, f64> = all_nodes
        .iter()
        .map(|&id| {
            let w = graph
                .forward
                .get(id)
                .map(|edges| edges.iter().map(|(_, kind)| edge_weight(kind)).sum::<f64>())
                .unwrap_or(0.0)
                .max(f64::MIN_POSITIVE);
            (id, w)
        })
        .collect();

    let n_f = n as f64;
    let mut rank: HashMap<SymbolId, f64> = all_nodes
        .iter()
        .map(|id| ((*id).clone(), 1.0 / n_f))
        .collect();

    for _ in 0..iterations {
        let mut new_rank: HashMap<SymbolId, f64> = HashMap::with_capacity(n);

        for &node in &all_nodes {
            // Personalized teleport: only seeds get teleport probability.
            let teleport = if seed_set.contains(node) {
                (1.0 - damping) / seed_count
            } else {
                0.0
            };

            let mut incoming_sum = 0.0_f64;
            if let Some(inbound) = graph.reverse.get(node) {
                for (src, kind) in inbound {
                    let src_rank = rank.get(src).copied().unwrap_or(0.0);
                    let w = edge_weight(kind);
                    let out_w = weighted_out.get(src).copied().unwrap_or(1.0);
                    incoming_sum += src_rank * w / out_w;
                }
            }
            new_rank.insert(node.clone(), damping.mul_add(incoming_sum, teleport));
        }

        rank = new_rank;
    }

    rank
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::SourceFile;
    use roko_core::language::{Import, ImportKind, Symbol, SymbolKind, Visibility};

    fn make_file(path: &str, symbols: Vec<Symbol>, imports: Vec<Import>) -> SourceFile {
        SourceFile {
            path: path.into(),
            language: "test".into(),
            content: String::new(),
            symbols,
            imports,
        }
    }

    fn sym(name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            name: name.into(),
            kind,
            visibility: Visibility::Public,
            line: 1,
        }
    }

    fn imp(path: &str) -> Import {
        Import {
            path: path.into(),
            alias: None,
            kind: ImportKind::Use,
        }
    }

    #[test]
    fn empty_graph() {
        let graph = build_graph(&[]);
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn single_file_no_edges() {
        let files = vec![make_file(
            "a.rs",
            vec![sym("foo", SymbolKind::Function)],
            vec![],
        )];
        let graph = build_graph(&files);
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn import_creates_edge() {
        let files = vec![
            make_file("a.rs", vec![sym("Config", SymbolKind::Struct)], vec![]),
            make_file(
                "b.rs",
                vec![sym("main", SymbolKind::Function)],
                vec![imp("a::Config")],
            ),
        ];
        let graph = build_graph(&files);
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);

        // b::main -> a::Config
        let main_id = SymbolId::new("b.rs", "main", SymbolKind::Function);
        let neighbors = graph.neighbors(&main_id);
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].symbol_name, "Config");
    }

    #[test]
    fn no_self_file_edges() {
        // If a file imports its own symbol, no edge should be created.
        let files = vec![make_file(
            "a.rs",
            vec![
                sym("Foo", SymbolKind::Struct),
                sym("bar", SymbolKind::Function),
            ],
            vec![imp("crate::Foo")],
        )];
        let graph = build_graph(&files);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn call_edges_are_created_from_function_bodies() {
        let files = vec![make_file(
            "a.rs",
            vec![
                sym("helper", SymbolKind::Function),
                sym("main", SymbolKind::Function),
            ],
            vec![],
        )];
        let mut files = files;
        files[0].content = "fn helper() {}\nfn main() { helper(); }\n".to_string();

        let graph = build_graph(&files);
        let main_id = SymbolId::new("a.rs", "main", SymbolKind::Function);
        let callees = graph.neighbors_by_kind(&main_id, EdgeKind::Calls);
        assert!(!callees.is_empty());
        assert!(callees.iter().any(|callee| callee.symbol_name == "helper"));
    }

    #[test]
    fn contains_edges_for_impl_block() {
        // impl Foo contains methods bar and baz.
        let files = vec![make_file(
            "a.rs",
            vec![
                Symbol {
                    name: "Foo".into(),
                    kind: SymbolKind::Impl,
                    visibility: Visibility::Private,
                    line: 1,
                },
                Symbol {
                    name: "bar".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    line: 2,
                },
                Symbol {
                    name: "baz".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    line: 5,
                },
            ],
            vec![],
        )];
        let graph = build_graph(&files);
        let impl_id = SymbolId::new("a.rs", "Foo", SymbolKind::Impl);
        let contained = graph.neighbors_by_kind(&impl_id, EdgeKind::Contains);
        assert_eq!(
            contained.len(),
            2,
            "impl Foo should contain 2 functions, got {:?}",
            contained
        );
        assert!(contained.iter().any(|c| c.symbol_name == "bar"));
        assert!(contained.iter().any(|c| c.symbol_name == "baz"));
    }

    #[test]
    fn contains_edges_for_module() {
        // mod tests contains a function.
        let files = vec![make_file(
            "a.rs",
            vec![
                Symbol {
                    name: "tests".into(),
                    kind: SymbolKind::Module,
                    visibility: Visibility::Private,
                    line: 1,
                },
                sym("it_works", SymbolKind::Function),
            ],
            vec![],
        )];
        let graph = build_graph(&files);
        let mod_id = SymbolId::new("a.rs", "tests", SymbolKind::Module);
        let contained = graph.neighbors_by_kind(&mod_id, EdgeKind::Contains);
        assert!(
            contained.iter().any(|c| c.symbol_name == "it_works"),
            "module should contain its function"
        );
    }

    #[test]
    fn implements_edges_for_trait_impl() {
        // `impl Display for Foo` should create Foo --Implements--> Display.
        let files = vec![make_file(
            "a.rs",
            vec![
                Symbol {
                    name: "Display".into(),
                    kind: SymbolKind::Trait,
                    visibility: Visibility::Public,
                    line: 1,
                },
                Symbol {
                    name: "Foo".into(),
                    kind: SymbolKind::Struct,
                    visibility: Visibility::Public,
                    line: 5,
                },
                Symbol {
                    name: "Display for Foo".into(),
                    kind: SymbolKind::Impl,
                    visibility: Visibility::Private,
                    line: 10,
                },
            ],
            vec![],
        )];
        let graph = build_graph(&files);

        // Foo should have an Implements edge to Display.
        let foo_id = SymbolId::new("a.rs", "Foo", SymbolKind::Struct);
        let impls = graph.neighbors_by_kind(&foo_id, EdgeKind::Implements);
        assert!(
            impls.iter().any(|i| i.symbol_name == "Display"),
            "Foo should implement Display, but got: {:?}",
            impls
        );

        // Reverse: Display should have Foo as a reverse Implements neighbor.
        let display_id = SymbolId::new("a.rs", "Display", SymbolKind::Trait);
        let rev = graph.reverse_neighbors_by_kind(&display_id, EdgeKind::Implements);
        assert!(
            rev.iter().any(|r| r.symbol_name == "Foo"),
            "Display should be implemented by Foo"
        );
    }

    #[test]
    fn implements_edges_across_files() {
        // Trait in one file, struct + impl in another.
        let files = vec![
            make_file(
                "trait.rs",
                vec![Symbol {
                    name: "Serializable".into(),
                    kind: SymbolKind::Trait,
                    visibility: Visibility::Public,
                    line: 1,
                }],
                vec![],
            ),
            make_file(
                "model.rs",
                vec![
                    Symbol {
                        name: "Config".into(),
                        kind: SymbolKind::Struct,
                        visibility: Visibility::Public,
                        line: 1,
                    },
                    Symbol {
                        name: "Serializable for Config".into(),
                        kind: SymbolKind::Impl,
                        visibility: Visibility::Private,
                        line: 5,
                    },
                ],
                vec![],
            ),
        ];
        let graph = build_graph(&files);

        let config_id = SymbolId::new("model.rs", "Config", SymbolKind::Struct);
        let impls = graph.neighbors_by_kind(&config_id, EdgeKind::Implements);
        assert!(
            impls.iter().any(|i| i.symbol_name == "Serializable"),
            "Config should implement Serializable across files"
        );
    }

    #[test]
    fn edge_count_by_kind_works() {
        let files = vec![make_file(
            "a.rs",
            vec![
                Symbol {
                    name: "Foo".into(),
                    kind: SymbolKind::Impl,
                    visibility: Visibility::Private,
                    line: 1,
                },
                Symbol {
                    name: "bar".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    line: 2,
                },
            ],
            vec![],
        )];
        let graph = build_graph(&files);
        assert!(
            graph.edge_count_by_kind(EdgeKind::Contains) > 0,
            "should have at least one Contains edge"
        );
    }

    #[test]
    fn reverse_neighbors() {
        let files = vec![
            make_file("a.rs", vec![sym("Server", SymbolKind::Struct)], vec![]),
            make_file(
                "b.rs",
                vec![sym("handler", SymbolKind::Function)],
                vec![imp("crate::Server")],
            ),
        ];
        let graph = build_graph(&files);
        let server_id = SymbolId::new("a.rs", "Server", SymbolKind::Struct);
        let rev = graph.reverse_neighbors(&server_id);
        assert_eq!(rev.len(), 1);
        assert_eq!(rev[0].symbol_name, "handler");
    }

    #[test]
    fn transitive_deps() {
        // a -> b -> c
        let files = vec![
            make_file("c.rs", vec![sym("Core", SymbolKind::Struct)], vec![]),
            make_file(
                "b.rs",
                vec![sym("Mid", SymbolKind::Struct)],
                vec![imp("c::Core")],
            ),
            make_file(
                "a.rs",
                vec![sym("Top", SymbolKind::Function)],
                vec![imp("b::Mid")],
            ),
        ];
        let graph = build_graph(&files);
        let top_id = SymbolId::new("a.rs", "Top", SymbolKind::Function);

        // Depth 1 should only reach Mid.
        let depth1 = graph.transitive(&top_id, 1);
        assert_eq!(depth1.len(), 1);
        assert_eq!(depth1[0].0.symbol_name, "Mid");

        // Depth 2 should reach Mid and Core.
        let depth2 = graph.transitive(&top_id, 2);
        assert_eq!(depth2.len(), 2);
    }

    #[test]
    fn pagerank_empty() {
        let graph = build_graph(&[]);
        let ranks = pagerank(&graph, 10, 0.85);
        assert!(ranks.is_empty());
    }

    #[test]
    fn pagerank_star_hub_highest() {
        // Three files all import Hub from hub.rs.
        let files = vec![
            make_file("hub.rs", vec![sym("Hub", SymbolKind::Struct)], vec![]),
            make_file(
                "a.rs",
                vec![sym("A", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "b.rs",
                vec![sym("B", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "c.rs",
                vec![sym("C", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
        ];
        let graph = build_graph(&files);
        let ranks = pagerank(&graph, 30, 0.85);

        let hub_id = SymbolId::new("hub.rs", "Hub", SymbolKind::Struct);
        let hub_rank = ranks.get(&hub_id).copied().unwrap_or(0.0);

        for (id, rank) in &ranks {
            if *id != hub_id {
                assert!(
                    hub_rank > *rank,
                    "Hub rank {hub_rank} should exceed {id} rank {rank}"
                );
            }
        }
    }

    #[test]
    fn weighted_pagerank_hub_highest() {
        // Same star topology as the unweighted test.
        let files = vec![
            make_file("hub.rs", vec![sym("Hub", SymbolKind::Struct)], vec![]),
            make_file(
                "a.rs",
                vec![sym("A", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "b.rs",
                vec![sym("B", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "c.rs",
                vec![sym("C", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
        ];
        let graph = build_graph(&files);
        let ranks = weighted_pagerank(&graph, 0.85, 30);
        let hub_id = SymbolId::new("hub.rs", "Hub", SymbolKind::Struct);
        let hub_rank = ranks.get(&hub_id).copied().unwrap_or(0.0);

        for (id, rank) in &ranks {
            if *id != hub_id {
                assert!(
                    hub_rank > *rank,
                    "Hub rank {hub_rank} should exceed {id} rank {rank}"
                );
            }
        }
    }

    #[test]
    fn weighted_pagerank_converges_like_unweighted_on_cycle() {
        // A cycle with all the same edge kind should converge to roughly equal
        // values, matching the unweighted result.
        let files = vec![
            make_file(
                "a.rs",
                vec![sym("A", SymbolKind::Function)],
                vec![imp("b::B")],
            ),
            make_file(
                "b.rs",
                vec![sym("B", SymbolKind::Function)],
                vec![imp("c::C")],
            ),
            make_file(
                "c.rs",
                vec![sym("C", SymbolKind::Function)],
                vec![imp("a::A")],
            ),
        ];
        let graph = build_graph(&files);
        let ranks = weighted_pagerank(&graph, 0.85, 50);
        let vals: Vec<f64> = ranks.values().copied().collect();
        assert_eq!(vals.len(), 3);
        let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(
            (max - min).abs() < 0.01,
            "Cycle nodes should have near-equal weighted ranks, max={max} min={min}"
        );
    }

    #[test]
    fn pagerank_cycle_roughly_equal() {
        // a -> b -> c -> a
        let files = vec![
            make_file(
                "a.rs",
                vec![sym("A", SymbolKind::Function)],
                vec![imp("b::B")],
            ),
            make_file(
                "b.rs",
                vec![sym("B", SymbolKind::Function)],
                vec![imp("c::C")],
            ),
            make_file(
                "c.rs",
                vec![sym("C", SymbolKind::Function)],
                vec![imp("a::A")],
            ),
        ];
        let graph = build_graph(&files);
        let ranks = pagerank(&graph, 50, 0.85);

        let vals: Vec<f64> = ranks.values().copied().collect();
        assert_eq!(vals.len(), 3);
        let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(
            (max - min).abs() < 0.01,
            "Cycle nodes should have near-equal ranks, max={max} min={min}"
        );
    }

    // ── Personalized PageRank tests (CODE-07) ───────────────────────

    #[test]
    fn personalized_pagerank_empty() {
        let graph = build_graph(&[]);
        let ranks = personalized_pagerank(&graph, &[], 0.85, 10);
        assert!(ranks.is_empty());
    }

    #[test]
    fn personalized_pagerank_seed_gets_higher_rank() {
        // Star topology: A, B, C all import Hub.
        // PPR seeded on A should give A higher rank than B or C.
        let files = vec![
            make_file("hub.rs", vec![sym("Hub", SymbolKind::Struct)], vec![]),
            make_file(
                "a.rs",
                vec![sym("A", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "b.rs",
                vec![sym("B", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "c.rs",
                vec![sym("C", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
        ];
        let graph = build_graph(&files);
        let seed = SymbolId::new("a.rs", "A", SymbolKind::Function);
        let ranks = personalized_pagerank(&graph, &[seed.clone()], 0.85, 30);

        let a_rank = ranks.get(&seed).copied().unwrap_or(0.0);
        let b_id = SymbolId::new("b.rs", "B", SymbolKind::Function);
        let b_rank = ranks.get(&b_id).copied().unwrap_or(0.0);
        let c_id = SymbolId::new("c.rs", "C", SymbolKind::Function);
        let c_rank = ranks.get(&c_id).copied().unwrap_or(0.0);

        assert!(
            a_rank > b_rank,
            "Seed A ({a_rank}) should rank higher than non-seed B ({b_rank})"
        );
        assert!(
            a_rank > c_rank,
            "Seed A ({a_rank}) should rank higher than non-seed C ({c_rank})"
        );
    }

    #[test]
    fn personalized_pagerank_hub_still_high_when_seeded() {
        // Even with PPR seeded on A, Hub (with 3 inbound edges) should still
        // rank highly because it receives rank from all nodes.
        let files = vec![
            make_file("hub.rs", vec![sym("Hub", SymbolKind::Struct)], vec![]),
            make_file(
                "a.rs",
                vec![sym("A", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "b.rs",
                vec![sym("B", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "c.rs",
                vec![sym("C", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
        ];
        let graph = build_graph(&files);
        let seed = SymbolId::new("a.rs", "A", SymbolKind::Function);
        let ranks = personalized_pagerank(&graph, &[seed], 0.85, 30);

        let hub_id = SymbolId::new("hub.rs", "Hub", SymbolKind::Struct);
        let hub_rank = ranks.get(&hub_id).copied().unwrap_or(0.0);
        let b_id = SymbolId::new("b.rs", "B", SymbolKind::Function);
        let b_rank = ranks.get(&b_id).copied().unwrap_or(0.0);

        // Hub should still outrank non-seed leaf B because of link structure.
        assert!(
            hub_rank > b_rank,
            "Hub ({hub_rank}) should outrank non-seed B ({b_rank})"
        );
    }

    #[test]
    fn personalized_pagerank_multiple_seeds() {
        // Seeds A and B should both rank higher than non-seed C.
        let files = vec![
            make_file("hub.rs", vec![sym("Hub", SymbolKind::Struct)], vec![]),
            make_file(
                "a.rs",
                vec![sym("A", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "b.rs",
                vec![sym("B", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
            make_file(
                "c.rs",
                vec![sym("C", SymbolKind::Function)],
                vec![imp("hub::Hub")],
            ),
        ];
        let graph = build_graph(&files);
        let seed_a = SymbolId::new("a.rs", "A", SymbolKind::Function);
        let seed_b = SymbolId::new("b.rs", "B", SymbolKind::Function);
        let ranks =
            personalized_pagerank(&graph, &[seed_a.clone(), seed_b.clone()], 0.85, 30);

        let a_rank = ranks.get(&seed_a).copied().unwrap_or(0.0);
        let b_rank = ranks.get(&seed_b).copied().unwrap_or(0.0);
        let c_id = SymbolId::new("c.rs", "C", SymbolKind::Function);
        let c_rank = ranks.get(&c_id).copied().unwrap_or(0.0);

        assert!(
            a_rank > c_rank,
            "Seed A ({a_rank}) should rank higher than C ({c_rank})"
        );
        assert!(
            b_rank > c_rank,
            "Seed B ({b_rank}) should rank higher than C ({c_rank})"
        );
    }

    #[test]
    fn personalized_pagerank_empty_seeds_still_runs() {
        // Empty seeds = no teleport to any node. All nodes should get 0 or
        // near-zero rank except from random walk itself.
        let files = vec![
            make_file("a.rs", vec![sym("A", SymbolKind::Function)], vec![]),
        ];
        let graph = build_graph(&files);
        let ranks = personalized_pagerank(&graph, &[], 0.85, 10);
        // Should still return results (all near-zero since no teleport).
        assert_eq!(ranks.len(), 1);
    }
}
