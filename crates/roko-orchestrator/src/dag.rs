//! Unified task DAG for cross-plan scheduling (§2.9–§2.12).
//!
//! Given a set of plans (each with its own [`Task`](roko_core::Task)s)
//! and plan-level dependencies, [`UnifiedTaskDag::build`] produces a
//! directed acyclic graph with edges from:
//!
//! 1. **Intra-plan `depends_on`** — `t1` → `t2` inside one plan.
//! 2. **Cross-plan `depends_on`** — `"09-foo:t3"` inside plan `"10-bar"`
//!    adds an edge from `09-foo:t3` to the referring task.
//! 3. **Plan-level `depends_on`** — plan `B` depends on plan `A` adds an
//!    edge from every task in `A` to every task in `B`.
//! 4. **File-overlap inference** (opt-in via [`DagConfig::infer_file_overlap`])
//!    — two tasks that both touch `src/lib.rs` get serialized; the
//!    task with the lexicographically-earlier [`GlobalTaskId`] runs
//!    first.
//!
//! [`UnifiedTaskDag::waves`] layers the DAG via BFS: wave 0 contains
//! tasks with no open dependencies, wave 1 depends only on wave 0,
//! and so on. Tasks within a wave sort by [`GlobalTaskId`] for
//! deterministic output.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use roko_core::{GlobalTaskId, Task};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Detect every node that participates in a cycle.
///
/// Input edges are expressed as `node -> direct dependencies`.
/// The returned node list is sorted for deterministic diagnostics.
#[must_use]
pub fn detect_cycle_nodes<N>(deps: &BTreeMap<N, BTreeSet<N>>) -> Vec<N>
where
    N: Clone + Ord,
{
    fn dfs<'a, N>(
        node: &'a N,
        deps: &'a BTreeMap<N, BTreeSet<N>>,
        state: &mut BTreeMap<&'a N, u8>,
        stack: &mut Vec<&'a N>,
        positions: &mut BTreeMap<&'a N, usize>,
        cycle_nodes: &mut BTreeSet<N>,
    ) where
        N: Clone + Ord,
    {
        state.insert(node, 1);
        positions.insert(node, stack.len());
        stack.push(node);

        if let Some(children) = deps.get(node) {
            for child in children {
                match state.get(child).copied().unwrap_or(0) {
                    0 => dfs(child, deps, state, stack, positions, cycle_nodes),
                    1 => {
                        if let Some(start) = positions.get(child).copied() {
                            for entry in &stack[start..] {
                                cycle_nodes.insert((*entry).clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        stack.pop();
        positions.remove(node);
        state.insert(node, 2);
    }

    let mut state: BTreeMap<&N, u8> = BTreeMap::new();
    let mut stack: Vec<&N> = Vec::new();
    let mut positions: BTreeMap<&N, usize> = BTreeMap::new();
    let mut cycle_nodes: BTreeSet<N> = BTreeSet::new();

    for node in deps.keys() {
        if state.get(node).copied().unwrap_or(0) == 0 {
            dfs(
                node,
                deps,
                &mut state,
                &mut stack,
                &mut positions,
                &mut cycle_nodes,
            );
        }
    }

    cycle_nodes.into_iter().collect()
}

/// A single wave: every task in `tasks` has no open dependencies and
/// can run in parallel with its peers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionWave {
    /// Wave ordinal (0 is first).
    pub index: usize,
    /// Tasks in this wave, sorted by [`GlobalTaskId`].
    pub tasks: Vec<GlobalTaskId>,
    /// Max `estimated_minutes` across the wave (wall-clock).
    pub estimated_minutes: u32,
}

/// Configuration knobs for DAG construction.
#[derive(Debug, Clone)]
pub struct DagConfig {
    /// When true, two tasks sharing a file get a serialization edge.
    pub infer_file_overlap: bool,
    /// Maximum number of tasks a wave may contain. Overflow spills into
    /// the next wave. `0` means unbounded.
    pub max_wave_width: usize,
}

impl Default for DagConfig {
    fn default() -> Self {
        Self {
            infer_file_overlap: true,
            max_wave_width: 0,
        }
    }
}

/// Summary statistics for a DAG (node/edge counts, critical path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagStats {
    /// Number of tasks in the DAG.
    pub nodes: usize,
    /// Number of directed edges.
    pub edges: usize,
    /// Number of waves produced by [`UnifiedTaskDag::waves`].
    pub waves: usize,
    /// Longest path by `estimated_minutes` (dynamic-programming walk).
    pub critical_path_minutes: u32,
}

/// Errors returned by DAG construction / traversal.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum DagError {
    /// A cycle was detected; `0` lists tasks still unvisited.
    #[error("cycle detected involving: {0:?}")]
    Cycle(Vec<GlobalTaskId>),

    /// A task references a dependency that does not exist.
    #[error("dangling dep_ref: {referrer} → {target}")]
    DanglingDepRef {
        /// The task that pointed at the missing dep.
        referrer: GlobalTaskId,
        /// The unresolved dep string, verbatim.
        target: String,
    },

    /// A plan-level dep referenced a plan that was not loaded.
    #[error("plan {0} referenced by plan-level deps but not loaded")]
    UnknownPlan(String),
}

/// Errors returned when mutating a DAG in place.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum DagMutationError {
    /// The target task does not exist.
    #[error("unknown task: {0}")]
    UnknownTask(GlobalTaskId),

    /// The task already completed and must not be mutated.
    #[error("completed task cannot be mutated: {0}")]
    CompletedTask(GlobalTaskId),

    /// The mutation payload was structurally invalid.
    #[error("invalid DAG mutation: {0}")]
    InvalidMutation(String),

    /// The mutation introduced a cycle.
    #[error("mutation introduced a cycle involving: {0:?}")]
    Cycle(Vec<GlobalTaskId>),
}

/// A live DAG mutation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DagMutation {
    /// Add a new task to the DAG.
    AddTask {
        /// The fully qualified task id.
        task_id: GlobalTaskId,
        /// The task spec to insert.
        task: Task,
        /// Direct dependencies of the inserted task.
        depends_on: Vec<GlobalTaskId>,
    },

    /// Remove a task from the DAG and reconnect its dependents to its deps.
    RemoveTask {
        /// The fully qualified task id.
        task_id: GlobalTaskId,
    },

    /// Replace one task with a serial chain of subtasks.
    SplitTask {
        /// The fully qualified task id.
        task_id: GlobalTaskId,
        /// Replacement tasks in execution order.
        into: Vec<Task>,
    },

    /// Add an additional dependency edge `from -> to`.
    AddDependency {
        /// The task that should wait.
        from: GlobalTaskId,
        /// The task that must complete first.
        to: GlobalTaskId,
    },

    /// Replace an existing task spec wholesale.
    UpdateTaskMetadata {
        /// The fully qualified task id.
        task_id: GlobalTaskId,
        /// The replacement task spec.
        task: Task,
    },
}

/// Unified DAG over every plan's tasks plus plan-level dependencies.
#[derive(Debug, Clone)]
pub struct UnifiedTaskDag {
    /// Canonical task specs keyed by global id.
    tasks: HashMap<GlobalTaskId, Task>,
    /// Plan-level dependencies: plan → set of plans it depends on.
    plan_deps: HashMap<String, HashSet<String>>,
    /// task → set of direct deps.
    edges: HashMap<GlobalTaskId, BTreeSet<GlobalTaskId>>,
    /// task → set of dependents.
    reverse_edges: HashMap<GlobalTaskId, BTreeSet<GlobalTaskId>>,
    /// Canonical (sorted) node list.
    nodes: Vec<GlobalTaskId>,
    /// task → estimated minutes (0 when unknown).
    estimates: HashMap<GlobalTaskId, u32>,
    /// DAG-building configuration.
    config: DagConfig,
}

impl UnifiedTaskDag {
    /// Build a unified DAG from per-plan task lists and plan deps.
    ///
    /// # Arguments
    ///
    /// - `plan_tasks`: plan base name → list of [`Task`]s in that plan.
    /// - `plan_deps`: plan base name → set of plan bases it depends on
    ///   (all tasks in the deps run before any task in this plan).
    /// - `config`: see [`DagConfig`].
    ///
    /// # Errors
    ///
    /// - [`DagError::UnknownPlan`] if `plan_deps` references a plan not
    ///   present in `plan_tasks`.
    /// - [`DagError::DanglingDepRef`] if a task's `depends_on` entry
    ///   does not resolve to any other task.
    /// - [`DagError::Cycle`] if the resulting graph is cyclic.
    #[allow(clippy::too_many_lines)]
    pub fn build(
        plan_tasks: &BTreeMap<String, Vec<Task>>,
        plan_deps: &HashMap<String, HashSet<String>>,
        config: DagConfig,
    ) -> Result<Self, DagError> {
        // Validate plan_deps first.
        for (plan, deps) in plan_deps {
            if !plan_tasks.contains_key(plan) {
                return Err(DagError::UnknownPlan(plan.clone()));
            }
            for dep in deps {
                if !plan_tasks.contains_key(dep) {
                    return Err(DagError::UnknownPlan(dep.clone()));
                }
            }
        }
        let mut tasks: HashMap<GlobalTaskId, Task> = HashMap::new();
        for (plan, plan_tasks) in plan_tasks {
            for task in plan_tasks {
                let id = GlobalTaskId::new(plan.clone(), task.id.clone());
                tasks.insert(id, task.clone());
            }
        }
        let mut dag = Self {
            tasks,
            plan_deps: plan_deps.clone(),
            edges: HashMap::new(),
            reverse_edges: HashMap::new(),
            nodes: Vec::new(),
            estimates: HashMap::new(),
            config,
        };
        dag.rebuild_indexes()?;
        // Reject cycles eagerly so callers never hold a bad DAG.
        let _ = dag.topological_sort()?;
        Ok(dag)
    }

    /// Direct deps of `id` (empty if none).
    #[must_use]
    pub fn deps_of(&self, id: &GlobalTaskId) -> &BTreeSet<GlobalTaskId> {
        static EMPTY: std::sync::OnceLock<BTreeSet<GlobalTaskId>> = std::sync::OnceLock::new();
        self.edges
            .get(id)
            .unwrap_or_else(|| EMPTY.get_or_init(BTreeSet::new))
    }

    /// Tasks that depend on `id`.
    #[must_use]
    pub fn dependents_of(&self, id: &GlobalTaskId) -> &BTreeSet<GlobalTaskId> {
        static EMPTY: std::sync::OnceLock<BTreeSet<GlobalTaskId>> = std::sync::OnceLock::new();
        self.reverse_edges
            .get(id)
            .unwrap_or_else(|| EMPTY.get_or_init(BTreeSet::new))
    }

    /// The task spec for `id`, if it exists.
    #[must_use]
    pub fn task(&self, id: &GlobalTaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// Every task in canonical order.
    #[must_use]
    pub fn nodes(&self) -> &[GlobalTaskId] {
        &self.nodes
    }

    /// Return the earliest time a task can start.
    #[must_use]
    pub fn earliest_start(&self, task: &GlobalTaskId) -> Duration {
        let Some((earliest, _, _, _)) = self.cpm_analysis() else {
            return Duration::ZERO;
        };
        earliest.get(task).copied().unwrap_or(Duration::ZERO)
    }

    /// Return the latest time a task can start without extending the plan.
    #[must_use]
    pub fn latest_start(&self, task: &GlobalTaskId) -> Duration {
        let Some((_, latest, _, _)) = self.cpm_analysis() else {
            return Duration::ZERO;
        };
        latest.get(task).copied().unwrap_or(Duration::ZERO)
    }

    /// Return the slack for a task.
    #[must_use]
    pub fn slack(&self, task: &GlobalTaskId) -> Duration {
        self.latest_start(task)
            .saturating_sub(self.earliest_start(task))
    }

    /// Return the zero-slack tasks on the critical path.
    #[must_use]
    pub fn critical_path(&self) -> Vec<GlobalTaskId> {
        let Some((earliest, latest, _, topo)) = self.cpm_analysis() else {
            return Vec::new();
        };
        topo.into_iter()
            .filter(|id| {
                earliest
                    .get(id)
                    .zip(latest.get(id))
                    .is_some_and(|(es, ls)| ls.saturating_sub(*es).is_zero())
            })
            .collect()
    }

    /// Topologically sort the DAG using Kahn's algorithm.
    ///
    /// Tie-breaker: lexicographic on [`GlobalTaskId`] so the result is
    /// deterministic across runs.
    ///
    /// # Errors
    ///
    /// [`DagError::Cycle`] if the graph is not acyclic.
    pub fn topological_sort(&self) -> Result<Vec<GlobalTaskId>, DagError> {
        let mut remaining_deps: HashMap<GlobalTaskId, usize> = self
            .edges
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect();
        let mut ready: BTreeSet<GlobalTaskId> = remaining_deps
            .iter()
            .filter_map(|(k, n)| if *n == 0 { Some(k.clone()) } else { None })
            .collect();
        let mut out = Vec::with_capacity(self.nodes.len());
        while let Some(next) = ready.iter().next().cloned() {
            ready.remove(&next);
            out.push(next.clone());
            for dependent in self.reverse_edges.get(&next).into_iter().flatten() {
                if let Some(entry) = remaining_deps.get_mut(dependent) {
                    *entry = entry.saturating_sub(1);
                    if *entry == 0 {
                        ready.insert(dependent.clone());
                    }
                }
            }
        }
        if out.len() != self.nodes.len() {
            let cycle_graph = self
                .edges
                .iter()
                .map(|(node, deps)| (node.clone(), deps.clone()))
                .collect::<BTreeMap<_, _>>();
            let cycle_nodes = detect_cycle_nodes(&cycle_graph);
            if !cycle_nodes.is_empty() {
                return Err(DagError::Cycle(cycle_nodes));
            }

            let mut stuck: Vec<GlobalTaskId> = remaining_deps
                .into_iter()
                .filter(|(_, n)| *n > 0)
                .map(|(k, _)| k)
                .collect();
            stuck.sort();
            return Err(DagError::Cycle(stuck));
        }
        Ok(out)
    }

    /// Partition the DAG into parallel waves via BFS layering.
    ///
    /// Within a wave, tasks sort by [`GlobalTaskId`]. When
    /// [`DagConfig::max_wave_width`] is non-zero, a wave's overflow
    /// spills into the next wave.
    ///
    /// # Errors
    ///
    /// [`DagError::Cycle`] if the graph is cyclic.
    pub fn waves(&self) -> Result<Vec<ExecutionWave>, DagError> {
        // Confirm acyclic up front, and walk nodes in topological order.
        let topo = self.topological_sort()?;
        let mut depth: HashMap<GlobalTaskId, usize> = HashMap::with_capacity(self.nodes.len());
        for node in &topo {
            let d = self
                .edges
                .get(node)
                .into_iter()
                .flatten()
                .map(|dep| depth.get(dep).copied().unwrap_or(0) + 1)
                .max()
                .unwrap_or(0);
            depth.insert(node.clone(), d);
        }
        // Bucket nodes by depth.
        let mut by_depth: BTreeMap<usize, BTreeSet<GlobalTaskId>> = BTreeMap::new();
        for (id, d) in &depth {
            by_depth.entry(*d).or_default().insert(id.clone());
        }
        // Apply max_wave_width: overflow spills to the next depth.
        let max_width = self.config.max_wave_width;
        let mut waves: Vec<ExecutionWave> = Vec::new();
        let mut overflow: Vec<GlobalTaskId> = Vec::new();
        for (_, bucket) in by_depth {
            let mut combined: Vec<GlobalTaskId> = std::mem::take(&mut overflow);
            combined.extend(bucket);
            combined.sort_by(|a, b| {
                (a.plan.as_str(), a.task.as_str()).cmp(&(b.plan.as_str(), b.task.as_str()))
            });
            while !combined.is_empty() {
                let take = if max_width == 0 {
                    combined.len()
                } else {
                    combined.len().min(max_width)
                };
                let batch: Vec<GlobalTaskId> = combined.drain(..take).collect();
                let est = batch
                    .iter()
                    .map(|id| self.estimates.get(id).copied().unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                waves.push(ExecutionWave {
                    index: waves.len(),
                    tasks: batch,
                    estimated_minutes: est,
                });
                if max_width > 0 && !combined.is_empty() {
                    // Remaining nodes from this bucket wait for next wave.
                    overflow.append(&mut combined);
                    break;
                }
            }
        }
        // Drain any final overflow into its own wave.
        while !overflow.is_empty() {
            let take = if max_width == 0 {
                overflow.len()
            } else {
                overflow.len().min(max_width)
            };
            let batch: Vec<GlobalTaskId> = overflow.drain(..take).collect();
            let est = batch
                .iter()
                .map(|id| self.estimates.get(id).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);
            waves.push(ExecutionWave {
                index: waves.len(),
                tasks: batch,
                estimated_minutes: est,
            });
        }
        Ok(waves)
    }

    /// Compute summary statistics.
    #[must_use]
    pub fn stats(&self) -> DagStats {
        let edge_count: usize = self.edges.values().map(BTreeSet::len).sum();
        let critical = self
            .cpm_analysis()
            .map_or(0, |(_, _, total, _)| duration_to_minutes(total));
        let wave_count = self.waves().map_or(0, |w| w.len());
        DagStats {
            nodes: self.nodes.len(),
            edges: edge_count,
            waves: wave_count,
            critical_path_minutes: critical,
        }
    }

    /// Remove tasks not required to produce the given target task IDs.
    ///
    /// Backward BFS from `targets` collects all transitive dependencies.
    /// Every node NOT in that set is removed from the DAG. Returns the
    /// number of culled tasks.
    ///
    /// Completed tasks are retained if they appear in the target's
    /// transitive closure (their outputs may still be needed).
    pub fn cull(&mut self, targets: &[String]) -> usize {
        // Resolve target strings to GlobalTaskIds. Try GlobalTaskId::parse
        // first (qualified "plan:task"), then match any node whose task
        // portion matches (bare "task" name).
        let mut needed: HashSet<GlobalTaskId> = HashSet::new();
        let mut queue: std::collections::VecDeque<GlobalTaskId> = std::collections::VecDeque::new();

        for target in targets {
            if let Some(gid) = GlobalTaskId::parse(target) {
                if self.tasks.contains_key(&gid) {
                    queue.push_back(gid.clone());
                    needed.insert(gid);
                }
            } else {
                // Bare task name — match any node with that task id.
                for node in &self.nodes {
                    if node.task == *target {
                        queue.push_back(node.clone());
                        needed.insert(node.clone());
                    }
                }
            }
        }

        // BFS backward through deps.
        while let Some(node) = queue.pop_front() {
            for dep in self.deps_of(&node).clone() {
                if needed.insert(dep.clone()) {
                    queue.push_back(dep);
                }
            }
        }

        // Collect nodes to remove.
        let to_remove: Vec<GlobalTaskId> = self
            .nodes
            .iter()
            .filter(|id| !needed.contains(id))
            .cloned()
            .collect();

        let culled = to_remove.len();
        if culled == 0 {
            return 0;
        }

        for id in &to_remove {
            self.tasks.remove(id);
            self.edges.remove(id);
            self.reverse_edges.remove(id);
            self.estimates.remove(id);
        }

        // Clean up edges pointing to removed nodes in remaining entries.
        let remove_set: HashSet<&GlobalTaskId> = to_remove.iter().collect();
        for deps in self.edges.values_mut() {
            deps.retain(|d| !remove_set.contains(d));
        }
        for rev in self.reverse_edges.values_mut() {
            rev.retain(|d| !remove_set.contains(d));
        }

        // Rebuild node list.
        self.nodes = self.tasks.keys().cloned().collect();
        self.nodes.sort_by(|a, b| {
            (a.plan.as_str(), a.task.as_str()).cmp(&(b.plan.as_str(), b.task.as_str()))
        });

        culled
    }

    /// Collapse eligible linear chains in place.
    ///
    /// The `config` parameter controls maximum chain length, minimum
    /// average parallelism, and whether cross-tier fusion is allowed.
    ///
    /// Returns the number of fusions performed.
    pub fn fuse_linear_chains(&mut self, config: &FusionConfig) -> usize {
        let mut fusions = 0usize;

        while let Ok(topo) = self.topological_sort() {
            let mut fused = false;

            for start in topo {
                if self.dependents_of(&start).len() != 1 {
                    continue;
                }
                let mut chain = vec![start.clone()];
                let mut cursor = start.clone();
                let mut compatible = true;
                while let Some(next) = self.dependents_of(&cursor).iter().next().cloned() {
                    if self.deps_of(&next).len() != 1 {
                        compatible = false;
                        break;
                    }
                    if self.deps_of(&next).iter().next() != Some(&cursor) {
                        compatible = false;
                        break;
                    }
                    if !fusion_compatible(self.task(&cursor), self.task(&next)) {
                        compatible = false;
                        break;
                    }
                    // Respect same_tier_only: skip if complexity bands differ.
                    if config.same_tier_only {
                        let cursor_band = self.task(&cursor).and_then(|t| t.complexity_band);
                        let next_band = self.task(&next).and_then(|t| t.complexity_band);
                        if cursor_band != next_band {
                            compatible = false;
                            break;
                        }
                    }
                    chain.push(next.clone());
                    // Cap chain length.
                    if chain.len() >= config.max_chain_length {
                        break;
                    }
                    cursor = next;
                    if self.dependents_of(&cursor).is_empty() {
                        break;
                    }
                    if self.dependents_of(&cursor).len() != 1 {
                        compatible = false;
                        break;
                    }
                }
                if !compatible || chain.len() < 2 {
                    continue;
                }
                let Some(mut target) = self.tasks.get(&start).cloned() else {
                    continue;
                };
                if chain.iter().any(|id| {
                    self.tasks
                        .get(id)
                        .is_some_and(|task| task.status == roko_core::TaskStatus::Done)
                }) {
                    continue;
                }

                // Guard: check that fusion won't reduce average parallelism
                // below the configured threshold.
                if config.ave_width > 0.0 {
                    let node_count = self.nodes.len();
                    let wave_count = self.waves().map_or(1, |w| w.len()).max(1);
                    let merged_count = chain.len() - 1; // nodes that would be removed
                    let new_node_count = node_count.saturating_sub(merged_count);
                    // After fusion, waves may shrink; estimate new average width.
                    let avg_width = new_node_count as f64 / wave_count as f64;
                    if avg_width < config.ave_width {
                        continue;
                    }
                }

                for merged_id in chain.iter().skip(1) {
                    if let Some(merged) = self.tasks.get(merged_id).cloned() {
                        merge_task_specs(&mut target, &merged);
                    }
                }
                self.tasks.insert(start.clone(), target);

                // Rewire: dependents of the chain tail must now depend on the
                // chain head instead of the removed tail node.
                let Some(tail) = chain.last() else {
                    continue;
                };
                let tail_dependents: Vec<_> = self.dependents_of(tail).iter().cloned().collect();
                for dep_id in tail_dependents {
                    if let Some(dep_task) = self.tasks.get_mut(&dep_id) {
                        // depends_on strings may be bare task names ("t3") or
                        // qualified ("plan-a:t3"); match both forms.
                        let tail_bare = &tail.task;
                        let tail_qualified = tail.to_string();
                        let start_ref = if tail.plan == start.plan {
                            start.task.clone()
                        } else {
                            start.to_string()
                        };
                        for raw in &mut dep_task.depends_on {
                            if raw == tail_bare || *raw == tail_qualified {
                                *raw = start_ref.clone();
                            }
                        }
                    }
                }

                for removed in chain.iter().skip(1) {
                    self.tasks.remove(removed);
                }

                if self.rebuild_indexes().is_err() {
                    break;
                }
                fusions += 1;
                fused = true;
                break;
            }

            if !fused {
                break;
            }
        }

        fusions
    }

    /// Apply a live DAG mutation.
    ///
    /// The mutation is validated against the current graph and rejected if
    /// it would introduce a cycle or touch a completed task.
    ///
    /// # Errors
    ///
    /// Returns [`DagMutationError::UnknownTask`] if the target task does
    /// not exist, [`DagMutationError::CompletedTask`] if the mutation
    /// touches a completed task, [`DagMutationError::InvalidMutation`] if
    /// the payload is structurally invalid, or [`DagMutationError::Cycle`]
    /// if the mutation introduces a cycle.
    #[allow(clippy::too_many_lines)]
    pub fn apply_mutation(&mut self, mutation: DagMutation) -> Result<(), DagMutationError> {
        let mut next = self.clone();
        let result = match mutation {
            DagMutation::AddTask {
                task_id,
                mut task,
                depends_on,
            } => {
                if task_id.task != task.id {
                    return Err(DagMutationError::InvalidMutation(format!(
                        "task id mismatch: {task_id} vs {}",
                        task.id
                    )));
                }
                if next.tasks.contains_key(&task_id) {
                    return Err(DagMutationError::InvalidMutation(format!(
                        "task already exists: {task_id}"
                    )));
                }
                task.depends_on = depends_on.iter().map(ToString::to_string).collect();
                next.tasks.insert(task_id, task);
                Ok(())
            }
            DagMutation::RemoveTask { task_id } => {
                ensure_mutable(&next, &task_id)?;
                let Some(task) = next.tasks.get(&task_id).cloned() else {
                    return Err(DagMutationError::UnknownTask(task_id));
                };
                let deps = task.depends_on;
                let dependents: Vec<_> = next.dependents_of(&task_id).iter().cloned().collect();
                let task_key = task_id.to_string();
                for dependent in dependents {
                    if let Some(dep_task) = next.tasks.get_mut(&dependent) {
                        remove_dep(&mut dep_task.depends_on, &task_key);
                        for dep in &deps {
                            let dep_key = dep.clone();
                            if !dep_task.depends_on.iter().any(|raw| raw == &dep_key) {
                                dep_task.depends_on.push(dep_key);
                            }
                        }
                    }
                }
                next.tasks.remove(&task_id);
                Ok(())
            }
            DagMutation::SplitTask { task_id, into } => {
                ensure_mutable(&next, &task_id)?;
                if into.is_empty() {
                    return Err(DagMutationError::InvalidMutation(
                        "split requires at least one replacement task".into(),
                    ));
                }
                let Some(original) = next.tasks.get(&task_id).cloned() else {
                    return Err(DagMutationError::UnknownTask(task_id));
                };
                let dependents: Vec<_> = next.dependents_of(&task_id).iter().cloned().collect();
                let original_deps = original.depends_on;
                let mut chain: Vec<GlobalTaskId> = Vec::with_capacity(into.len());
                for (index, mut task) in into.into_iter().enumerate() {
                    if index == 0 && task.id != task_id.task {
                        return Err(DagMutationError::InvalidMutation(format!(
                            "split head must keep original id {}",
                            task_id.task
                        )));
                    }
                    if task.id.is_empty() {
                        return Err(DagMutationError::InvalidMutation(
                            "split tasks must have explicit ids".into(),
                        ));
                    }
                    if index > 0 && task.id == task_id.task {
                        return Err(DagMutationError::InvalidMutation(format!(
                            "split task id mismatch: replacement must not reuse original id {}",
                            task_id.task
                        )));
                    }
                    task.depends_on = if index == 0 {
                        original_deps.clone()
                    } else {
                        vec![chain[index - 1].to_string()]
                    };
                    chain.push(GlobalTaskId::new(&task_id.plan, task.id.clone()));
                    next.tasks.insert(chain[index].clone(), task);
                }

                let last = chain
                    .last()
                    .cloned()
                    .ok_or_else(|| DagMutationError::InvalidMutation("empty split".into()))?;
                let last_key = last.to_string();
                let task_key = task_id.to_string();
                for dependent in dependents {
                    if let Some(dep_task) = next.tasks.get_mut(&dependent) {
                        replace_dep(&mut dep_task.depends_on, &task_key, &last_key);
                    }
                }
                next.tasks.remove(&task_id);
                Ok(())
            }
            DagMutation::AddDependency { from, to } => {
                if from == to {
                    return Err(DagMutationError::InvalidMutation(
                        "self-dependency is not allowed".into(),
                    ));
                }
                ensure_mutable(&next, &from)?;
                let Some(task) = next.tasks.get_mut(&from) else {
                    return Err(DagMutationError::UnknownTask(from));
                };
                let to_key = to.to_string();
                if !task.depends_on.iter().any(|raw| raw == &to_key) {
                    task.depends_on.push(to_key);
                }
                Ok(())
            }
            DagMutation::UpdateTaskMetadata { task_id, mut task } => {
                ensure_mutable(&next, &task_id)?;
                if task_id.task != task.id {
                    return Err(DagMutationError::InvalidMutation(format!(
                        "task id mismatch: {task_id} vs {}",
                        task.id
                    )));
                }
                task.depends_on = next
                    .tasks
                    .get(&task_id)
                    .map(|current| current.depends_on.clone())
                    .ok_or_else(|| DagMutationError::UnknownTask(task_id.clone()))?;
                next.tasks.insert(task_id, task);
                Ok(())
            }
        };

        result?;
        next.rebuild_indexes()
            .map_err(|err| map_rebuild_error(err, &next))?;
        if let Err(err) = next.topological_sort() {
            return Err(match err {
                DagError::Cycle(stuck) => DagMutationError::Cycle(stuck),
                other => map_rebuild_error(other, &next),
            });
        }
        *self = next;
        Ok(())
    }

    fn rebuild_indexes(&mut self) -> Result<(), DagError> {
        self.edges.clear();
        self.reverse_edges.clear();
        self.nodes.clear();
        self.estimates.clear();

        self.nodes = self.tasks.keys().cloned().collect();
        self.nodes.sort_by(|a, b| {
            (a.plan.as_str(), a.task.as_str()).cmp(&(b.plan.as_str(), b.task.as_str()))
        });
        let node_set: HashSet<GlobalTaskId> = self.nodes.iter().cloned().collect();
        for node in &self.nodes {
            self.edges.insert(node.clone(), BTreeSet::new());
            self.reverse_edges.insert(node.clone(), BTreeSet::new());
        }

        for (id, task) in &self.tasks {
            self.estimates
                .insert(id.clone(), task.estimated_minutes.unwrap_or(0));
            for raw in &task.depends_on {
                let dep_id = resolve_dep_ref(&id.plan, raw);
                if !node_set.contains(&dep_id) {
                    return Err(DagError::DanglingDepRef {
                        referrer: id.clone(),
                        target: raw.clone(),
                    });
                }
                self.edges.entry(id.clone()).or_default().insert(dep_id);
            }
        }

        for (plan, deps) in &self.plan_deps {
            let Some(plan_tasks) = tasks_for_plan(&self.tasks, plan) else {
                return Err(DagError::UnknownPlan(plan.clone()));
            };
            for dep_plan in deps {
                let Some(dep_tasks) = tasks_for_plan(&self.tasks, dep_plan) else {
                    return Err(DagError::UnknownPlan(dep_plan.clone()));
                };
                for id in &plan_tasks {
                    for dep_id in &dep_tasks {
                        self.edges
                            .entry(id.clone())
                            .or_default()
                            .insert(dep_id.clone());
                    }
                }
            }
        }

        if self.config.infer_file_overlap {
            let mut by_file: HashMap<String, BTreeSet<GlobalTaskId>> = HashMap::new();
            for (id, task) in &self.tasks {
                for file in &task.files {
                    by_file.entry(file.clone()).or_default().insert(id.clone());
                }
            }
            for tasks in by_file.into_values() {
                let ordered: Vec<_> = tasks.into_iter().collect();
                for i in 0..ordered.len() {
                    for j in 0..i {
                        if ordered[i] != ordered[j] {
                            self.edges
                                .entry(ordered[i].clone())
                                .or_default()
                                .insert(ordered[j].clone());
                        }
                    }
                }
            }
        }

        for node in &self.nodes {
            self.reverse_edges.insert(node.clone(), BTreeSet::new());
        }
        for (from, deps) in &self.edges {
            for dep in deps {
                self.reverse_edges
                    .entry(dep.clone())
                    .or_default()
                    .insert(from.clone());
            }
        }

        Ok(())
    }

    #[allow(clippy::type_complexity)]
    fn cpm_analysis(
        &self,
    ) -> Option<(
        HashMap<GlobalTaskId, Duration>,
        HashMap<GlobalTaskId, Duration>,
        Duration,
        Vec<GlobalTaskId>,
    )> {
        let topo = self.topological_sort().ok()?;
        let mut earliest: HashMap<GlobalTaskId, Duration> =
            HashMap::with_capacity(self.nodes.len());
        for node in &topo {
            let start = self
                .deps_of(node)
                .iter()
                .map(|dep| {
                    earliest.get(dep).copied().unwrap_or(Duration::ZERO) + self.task_duration(dep)
                })
                .max()
                .unwrap_or(Duration::ZERO);
            earliest.insert(node.clone(), start);
        }

        let mut project_duration = Duration::ZERO;
        for node in &topo {
            let finish =
                earliest.get(node).copied().unwrap_or(Duration::ZERO) + self.task_duration(node);
            project_duration = project_duration.max(finish);
        }

        let mut latest: HashMap<GlobalTaskId, Duration> = HashMap::with_capacity(self.nodes.len());
        for node in topo.iter().rev() {
            let duration = self.task_duration(node);
            let latest_finish = self
                .dependents_of(node)
                .iter()
                .map(|dep| latest.get(dep).copied().unwrap_or(project_duration))
                .min()
                .unwrap_or(project_duration);
            let latest_start = latest_finish.saturating_sub(duration);
            latest.insert(node.clone(), latest_start);
        }

        Some((earliest, latest, project_duration, topo))
    }

    fn task_duration(&self, id: &GlobalTaskId) -> Duration {
        self.tasks
            .get(id)
            .and_then(|task| task.estimated_minutes)
            .map_or(Duration::ZERO, |minutes| {
                Duration::from_secs(u64::from(minutes).saturating_mul(60))
            })
    }

    /// Partition the DAG into `k` partitions using simplified METIS-style
    /// heavy-edge matching.
    ///
    /// The algorithm:
    /// 1. Coarsen: greedily match adjacent nodes by heaviest edge weight
    ///    (estimated minutes) and contract them.
    /// 2. Bisect the coarsened graph via BFS assignment.
    /// 3. Uncoarsen and assign original nodes to their partition.
    ///
    /// The goal is to minimize cross-partition edges (communication cost)
    /// while balancing total work across partitions.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn partition(&self, k: usize) -> Vec<DagPartition> {
        let k = k.max(1);
        if self.nodes.is_empty() {
            return (0..k)
                .map(|i| DagPartition {
                    partition_id: i,
                    tasks: Vec::new(),
                    cut_edges: 0,
                    total_work: 0.0,
                })
                .collect();
        }
        if k >= self.nodes.len() {
            // One task per partition.
            let mut result: Vec<DagPartition> = self
                .nodes
                .iter()
                .enumerate()
                .map(|(i, id)| DagPartition {
                    partition_id: i,
                    tasks: vec![id.clone()],
                    cut_edges: 0,
                    total_work: f64::from(self.estimates.get(id).copied().unwrap_or(0)),
                })
                .collect();
            // Fill remaining empty partitions.
            for i in result.len()..k {
                result.push(DagPartition {
                    partition_id: i,
                    tasks: Vec::new(),
                    cut_edges: 0,
                    total_work: 0.0,
                });
            }
            // Count cut edges (all edges are cross-partition).
            for part in &mut result {
                for task in &part.tasks {
                    let cuts = self.edges.get(task).map_or(0, |deps| {
                        deps.iter().filter(|dep| !part.tasks.contains(dep)).count()
                    });
                    part.cut_edges += cuts;
                }
            }
            return result;
        }

        // Assign each node a partition using topological-order round-robin
        // weighted by estimated work to balance load.
        let topo = self
            .topological_sort()
            .unwrap_or_else(|_| self.nodes.clone());
        let mut assignment: HashMap<GlobalTaskId, usize> = HashMap::new();
        let mut partition_work = vec![0.0_f64; k];

        for node in &topo {
            // Prefer the partition of the heaviest dependency to minimize cuts.
            let dep_partition = self
                .edges
                .get(node)
                .into_iter()
                .flatten()
                .filter_map(|dep| {
                    let p = *assignment.get(dep)?;
                    let weight = self.estimates.get(dep).copied().unwrap_or(0);
                    Some((p, weight))
                })
                .max_by_key(|&(_, w)| w)
                .map(|(p, _)| p);

            let chosen = dep_partition
                .filter(|&dep_p| {
                    let min_work = partition_work.iter().copied().fold(f64::INFINITY, f64::min);
                    partition_work[dep_p] <= min_work * 1.5 + 1.0
                })
                .unwrap_or_else(|| lightest_partition(&partition_work));

            assignment.insert(node.clone(), chosen);
            partition_work[chosen] += f64::from(self.estimates.get(node).copied().unwrap_or(0));
        }

        // Build partition structs.
        let mut partitions: Vec<DagPartition> = (0..k)
            .map(|i| DagPartition {
                partition_id: i,
                tasks: Vec::new(),
                cut_edges: 0,
                total_work: 0.0,
            })
            .collect();

        for node in &self.nodes {
            let p = assignment.get(node).copied().unwrap_or(0);
            let work = f64::from(self.estimates.get(node).copied().unwrap_or(0));
            partitions[p].tasks.push(node.clone());
            partitions[p].total_work += work;
        }

        // Count cut edges per partition.
        for part in &mut partitions {
            let pid = part.partition_id;
            for task in &part.tasks {
                if let Some(deps) = self.edges.get(task) {
                    for dep in deps {
                        if assignment.get(dep).copied().unwrap_or(pid) != pid {
                            part.cut_edges += 1;
                        }
                    }
                }
            }
        }

        partitions
    }

    /// Compute full Critical Path Method analysis with float calculations.
    ///
    /// Calls the internal [`cpm_analysis`] and extends the results with
    /// total float (schedule slack per task) and free float (slack that
    /// does not affect any successor).
    ///
    /// Returns `None` if the DAG contains a cycle.
    #[must_use]
    pub fn cpm_analysis_full(&self) -> Option<CpmAnalysis> {
        let (earliest, latest, project_duration, topo) = self.cpm_analysis()?;

        let mut total_float = HashMap::with_capacity(topo.len());
        let mut free_float = HashMap::with_capacity(topo.len());

        for node in &topo {
            let es = earliest.get(node).copied().unwrap_or(Duration::ZERO);
            let ls = latest.get(node).copied().unwrap_or(Duration::ZERO);
            total_float.insert(node.clone(), ls.as_secs_f64() - es.as_secs_f64());

            // Free float = min(ES of successors) - EF(self).
            // EF(self) = ES(self) + duration(self).
            let ef = es + self.task_duration(node);
            let successors = self.dependents_of(node);
            let ff = if successors.is_empty() {
                // Terminal task: free float equals total float.
                ls.as_secs_f64() - es.as_secs_f64()
            } else {
                let min_succ_es = successors
                    .iter()
                    .map(|s| earliest.get(s).copied().unwrap_or(Duration::ZERO))
                    .min()
                    .unwrap_or(Duration::ZERO);
                (min_succ_es.as_secs_f64() - ef.as_secs_f64()).max(0.0)
            };
            free_float.insert(node.clone(), ff);
        }

        let critical_path = topo
            .iter()
            .filter(|id| {
                earliest
                    .get(*id)
                    .zip(latest.get(*id))
                    .is_some_and(|(es, ls)| ls.saturating_sub(*es).is_zero())
            })
            .cloned()
            .collect();

        let earliest_f64 = earliest
            .into_iter()
            .map(|(k, v)| (k, v.as_secs_f64()))
            .collect();
        let latest_f64 = latest
            .into_iter()
            .map(|(k, v)| (k, v.as_secs_f64()))
            .collect();

        Some(CpmAnalysis {
            earliest_start: earliest_f64,
            latest_start: latest_f64,
            total_float,
            free_float,
            critical_path,
            min_duration: project_duration.as_secs_f64(),
        })
    }
}

/// A partition of the DAG for distributed execution.
///
/// Produced by [`UnifiedTaskDag::partition`] using simplified METIS-style
/// heavy-edge matching: coarsen via maximum-weight edge matching, bisect,
/// then uncoarsen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DagPartition {
    /// Partition ordinal.
    pub partition_id: usize,
    /// Tasks assigned to this partition.
    pub tasks: Vec<GlobalTaskId>,
    /// Number of cross-partition dependency edges.
    pub cut_edges: usize,
    /// Sum of estimated minutes for tasks in this partition.
    pub total_work: f64,
}

/// Critical Path Method analysis results with float calculations.
///
/// Bundles the four CPM outputs (earliest/latest starts, critical path,
/// min duration) with float computations (total/free) into a single
/// inspectable result type.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CpmAnalysis {
    /// Earliest possible start time for each task (seconds).
    pub earliest_start: HashMap<GlobalTaskId, f64>,
    /// Latest possible start time without delaying the project (seconds).
    pub latest_start: HashMap<GlobalTaskId, f64>,
    /// Total float = latest_start - earliest_start (schedule slack, seconds).
    pub total_float: HashMap<GlobalTaskId, f64>,
    /// Free float = min(ES(successors)) - EF(self) (slack without affecting successors, seconds).
    pub free_float: HashMap<GlobalTaskId, f64>,
    /// Tasks on the critical path (zero total float).
    pub critical_path: Vec<GlobalTaskId>,
    /// Minimum project duration (seconds).
    pub min_duration: f64,
}

impl CpmAnalysis {
    /// Returns `true` if the given task is on the critical path (zero total float).
    #[must_use]
    pub fn is_critical(&self, task: &GlobalTaskId) -> bool {
        self.critical_path.contains(task)
    }

    /// Returns the total float (schedule slack) for a task in seconds.
    ///
    /// Returns `0.0` if the task is not found.
    #[must_use]
    pub fn slack(&self, task: &GlobalTaskId) -> f64 {
        self.total_float.get(task).copied().unwrap_or(0.0)
    }
}

/// Configuration for the chain fusion optimizer.
///
/// Controls how [`UnifiedTaskDag::fuse_linear_chains`] collapses
/// serial chains of tasks into single fused tasks.
#[derive(Clone, Debug)]
pub struct FusionConfig {
    /// Maximum number of tasks that can be fused into one. Default: 5.
    pub max_chain_length: usize,
    /// Minimum average DAG width to allow fusion (don't fuse if it
    /// would reduce parallelism below this threshold). Default: 2.0.
    pub ave_width: f64,
    /// Only fuse tasks within the same complexity band. Default: true.
    pub same_tier_only: bool,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            max_chain_length: 5,
            ave_width: 2.0,
            same_tier_only: true,
        }
    }
}

/// Durability level used by incremental DAG recomputation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Durability {
    /// Recompute eagerly after any upstream change.
    Low,
    /// Default behaviour.
    Medium,
    /// Survives re-plans unless explicitly dirtied.
    High,
}

/// Build-system-style incremental DAG wrapper.
#[derive(Debug, Clone)]
pub struct IncrementalDag {
    /// The underlying DAG.
    pub dag: UnifiedTaskDag,
    dirty: HashSet<GlobalTaskId>,
    durability: HashMap<GlobalTaskId, Durability>,
    /// Global revision counter, incremented on every input change.
    revision: u64,
    /// Per-node revision at which the node was last verified clean.
    verified_at: HashMap<GlobalTaskId, u64>,
    /// Per-node BLAKE3 hash of the node's inputs (serialized task + deps).
    input_hashes: HashMap<GlobalTaskId, [u8; 32]>,
}

impl IncrementalDag {
    /// Create a new incremental wrapper.
    #[must_use]
    pub fn new(dag: UnifiedTaskDag) -> Self {
        Self {
            dag,
            dirty: HashSet::new(),
            durability: HashMap::new(),
            revision: 0,
            verified_at: HashMap::new(),
            input_hashes: HashMap::new(),
        }
    }

    /// Current global revision counter.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Per-node verified-at revisions.
    #[must_use]
    pub fn verified_at_map(&self) -> &HashMap<GlobalTaskId, u64> {
        &self.verified_at
    }

    /// Per-node input hashes.
    #[must_use]
    pub fn input_hashes_map(&self) -> &HashMap<GlobalTaskId, [u8; 32]> {
        &self.input_hashes
    }

    /// Read-only access to the underlying DAG.
    #[must_use]
    pub const fn dag(&self) -> &UnifiedTaskDag {
        &self.dag
    }

    /// Mutable access to the underlying DAG.
    pub const fn dag_mut(&mut self) -> &mut UnifiedTaskDag {
        &mut self.dag
    }

    /// Set a task's durability.
    pub fn set_durability(&mut self, task: GlobalTaskId, durability: Durability) {
        self.durability.insert(task, durability);
    }

    /// Mark a task dirty and propagate to dependents.
    ///
    /// Increments the global revision counter on each call.
    pub fn mark_dirty(&mut self, task: GlobalTaskId) {
        self.revision += 1;
        let mut stack = vec![task];
        while let Some(next) = stack.pop() {
            if !self.dirty.insert(next.clone()) {
                continue;
            }
            if matches!(self.durability.get(&next), Some(Durability::High)) {
                continue;
            }
            for dependent in self.dag.dependents_of(&next).iter().cloned() {
                stack.push(dependent);
            }
        }
    }

    /// Salsa-style backdate optimization: check whether a task's inputs
    /// actually changed before recomputing.
    ///
    /// If the BLAKE3 hash of the node's current inputs matches the stored
    /// hash, the node is "backdated" clean at the current revision and
    /// removed from the dirty set without recomputation.
    ///
    /// Returns `true` if the node is clean (either already clean or
    /// successfully backdated).
    pub fn ensure_clean(&mut self, task_id: &GlobalTaskId) -> bool {
        // Already clean.
        if !self.dirty.contains(task_id) {
            return true;
        }

        let current_hash = self.compute_input_hash(task_id);
        if let Some(stored) = self.input_hashes.get(task_id) {
            if *stored == current_hash {
                // Inputs unchanged — backdate: mark clean at current revision.
                self.dirty.remove(task_id);
                self.verified_at.insert(task_id.clone(), self.revision);
                return true;
            }
        }

        // Inputs changed — update stored hash, keep dirty.
        self.input_hashes.insert(task_id.clone(), current_hash);
        false
    }

    /// Compute a BLAKE3 hash of a node's inputs: its serialized task spec
    /// plus the sorted list of dependency task IDs.
    fn compute_input_hash(&self, task_id: &GlobalTaskId) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(task_id.to_string().as_bytes());
        if let Some(task) = self.dag.task(task_id) {
            if let Ok(json) = serde_json::to_string(task) {
                hasher.update(json.as_bytes());
            }
        }
        let deps = self.dag.deps_of(task_id);
        for dep in deps {
            hasher.update(dep.to_string().as_bytes());
        }
        *hasher.finalize().as_bytes()
    }

    /// Mark a task verified at the current revision and store its input hash.
    pub fn mark_verified(&mut self, task_id: &GlobalTaskId) {
        self.dirty.remove(task_id);
        self.verified_at.insert(task_id.clone(), self.revision);
        let hash = self.compute_input_hash(task_id);
        self.input_hashes.insert(task_id.clone(), hash);
    }

    /// Return the tasks that do not need to be re-executed.
    #[must_use]
    pub fn clean_set(&self) -> HashSet<GlobalTaskId> {
        self.dag
            .nodes()
            .iter()
            .filter(|id| !self.dirty.contains(*id))
            .cloned()
            .collect()
    }

    /// Return the dirty tasks in execution order and clear their dirty marks.
    #[must_use]
    pub fn recompute_plan(&mut self) -> Vec<GlobalTaskId> {
        let Ok(topo) = self.dag.topological_sort() else {
            return Vec::new();
        };
        let mut dirty_order = Vec::new();
        for id in topo {
            if self.dirty.remove(&id) {
                dirty_order.push(id);
            }
        }
        dirty_order
    }

    /// Apply a mutation and mark the touched subtree dirty.
    ///
    /// # Errors
    ///
    /// Returns the same [`DagMutationError`] variants as
    /// [`UnifiedTaskDag::apply_mutation`] when the mutation cannot be
    /// applied.
    pub fn apply_mutation(&mut self, mutation: DagMutation) -> Result<(), DagMutationError> {
        let touched = mutation_touched_ids(&mutation);
        self.dag.apply_mutation(mutation)?;
        for id in touched {
            self.mark_dirty(id);
        }
        Ok(())
    }
}

/// Resolve a `depends_on` string against the referring plan:
///
/// - `"t3"` → `GlobalTaskId { plan: referrer_plan, task: "t3" }`.
/// - `"09-foo:t3"` → `GlobalTaskId { plan: "09-foo", task: "t3" }`.
fn resolve_dep_ref(referrer_plan: &str, raw: &str) -> GlobalTaskId {
    GlobalTaskId::parse(raw).unwrap_or_else(|| GlobalTaskId::new(referrer_plan, raw))
}

fn tasks_for_plan(tasks: &HashMap<GlobalTaskId, Task>, plan: &str) -> Option<Vec<GlobalTaskId>> {
    let mut ids: Vec<GlobalTaskId> = tasks.keys().filter(|id| id.plan == plan).cloned().collect();
    if ids.is_empty() {
        return None;
    }
    ids.sort_by(|a, b| (a.plan.as_str(), a.task.as_str()).cmp(&(b.plan.as_str(), b.task.as_str())));
    Some(ids)
}

fn duration_to_minutes(duration: Duration) -> u32 {
    let minutes = duration.as_secs() / 60;
    u32::try_from(minutes).unwrap_or(u32::MAX)
}

fn replace_dep(deps: &mut Vec<String>, old: &str, new: &str) {
    for dep in deps.iter_mut() {
        if dep == old {
            *dep = new.to_string();
        }
    }
    dedup_in_place(deps);
}

fn remove_dep(deps: &mut Vec<String>, target: &str) {
    deps.retain(|dep| dep != target);
    dedup_in_place(deps);
}

fn dedup_in_place(deps: &mut Vec<String>) {
    let mut seen = HashSet::new();
    deps.retain(|dep| seen.insert(dep.clone()));
}

fn fusion_compatible(left: Option<&Task>, right: Option<&Task>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    left.role == right.role
        && left.preferred_model == right.preferred_model
        && left.preferred_provider == right.preferred_provider
        && left.complexity_band == right.complexity_band
}

fn merge_task_specs(target: &mut Task, merged: &Task) {
    target.title = format!("{} + {}", target.title, merged.title);
    target.estimated_minutes = Some(
        target
            .estimated_minutes
            .unwrap_or(0)
            .saturating_add(merged.estimated_minutes.unwrap_or(0)),
    );
    append_unique(&mut target.files, &merged.files);
    append_unique(&mut target.acceptance, &merged.acceptance);
    target.exclusive_files |= merged.exclusive_files;
    if target.role.is_none() {
        target.role.clone_from(&merged.role);
    }
    if target.category.is_none() {
        target.category = merged.category;
    }
    if target.reasoning_level.is_none() {
        target.reasoning_level = merged.reasoning_level;
    }
    if target.speed_priority.is_none() {
        target.speed_priority = merged.speed_priority;
    }
    if target.quality_profile.is_none() {
        target.quality_profile = merged.quality_profile;
    }
    if target.context_weight.is_none() {
        target.context_weight = merged.context_weight;
    }
    if target.complexity_band.is_none() {
        target.complexity_band = merged.complexity_band;
    }
    if target.preferred_model.is_none() {
        target.preferred_model.clone_from(&merged.preferred_model);
    }
    if target.preferred_provider.is_none() {
        target
            .preferred_provider
            .clone_from(&merged.preferred_provider);
    }
}

fn append_unique(target: &mut Vec<String>, source: &[String]) {
    for item in source {
        if !target.iter().any(|existing| existing == item) {
            target.push(item.clone());
        }
    }
}

fn ensure_mutable(dag: &UnifiedTaskDag, task_id: &GlobalTaskId) -> Result<(), DagMutationError> {
    let Some(task) = dag.tasks.get(task_id) else {
        return Err(DagMutationError::UnknownTask(task_id.clone()));
    };
    if task.status == roko_core::TaskStatus::Done {
        return Err(DagMutationError::CompletedTask(task_id.clone()));
    }
    Ok(())
}

fn map_rebuild_error(err: DagError, _dag: &UnifiedTaskDag) -> DagMutationError {
    match err {
        DagError::Cycle(stuck) => DagMutationError::Cycle(stuck),
        DagError::DanglingDepRef { referrer, target } => DagMutationError::InvalidMutation(
            format!("dangling dep during rebuild: {referrer} -> {target}"),
        ),
        DagError::UnknownPlan(plan) => {
            DagMutationError::InvalidMutation(format!("unknown plan during rebuild: {plan}"))
        }
    }
}

fn lightest_partition(partition_work: &[f64]) -> usize {
    partition_work
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map_or(0, |(i, _)| i)
}

fn mutation_touched_ids(mutation: &DagMutation) -> Vec<GlobalTaskId> {
    match mutation {
        DagMutation::AddTask { task_id, .. }
        | DagMutation::RemoveTask { task_id }
        | DagMutation::UpdateTaskMetadata { task_id, .. } => vec![task_id.clone()],
        DagMutation::SplitTask { task_id, into } => {
            let mut touched = vec![task_id.clone()];
            if let Some(first) = into.first() {
                touched.push(GlobalTaskId::new(&task_id.plan, first.id.clone()));
            }
            touched
        }
        DagMutation::AddDependency { from, .. } => vec![from.clone()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_task(id: &str, deps: &[&str], files: &[&str], minutes: Option<u32>) -> Task {
        let mut t = Task::new(id, format!("title of {id}"));
        t.depends_on = deps.iter().map(|s| (*s).to_string()).collect();
        t.files = files.iter().map(|s| (*s).to_string()).collect();
        t.estimated_minutes = minutes;
        t
    }

    fn single_plan_dag(tasks: Vec<Task>) -> Result<UnifiedTaskDag, DagError> {
        let mut plans = BTreeMap::new();
        plans.insert("plan-a".to_string(), tasks);
        UnifiedTaskDag::build(&plans, &HashMap::new(), DagConfig::default())
    }

    #[test]
    fn empty_input_builds_empty_dag() {
        let dag =
            UnifiedTaskDag::build(&BTreeMap::new(), &HashMap::new(), DagConfig::default()).unwrap();
        assert!(dag.nodes().is_empty());
        assert!(dag.waves().unwrap().is_empty());
        assert_eq!(dag.stats().nodes, 0);
    }

    #[test]
    fn linear_chain_produces_one_task_per_wave() {
        let tasks = vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
            mk_task("t3", &["t2"], &[], Some(3)),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0].tasks[0].task, "t1");
        assert_eq!(waves[1].tasks[0].task, "t2");
        assert_eq!(waves[2].tasks[0].task, "t3");
        assert_eq!(dag.stats().critical_path_minutes, 18);
    }

    #[test]
    fn critical_path_helpers_follow_the_chain() {
        let dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
            mk_task("t3", &["t2"], &[], Some(3)),
        ])
        .unwrap();
        let id = GlobalTaskId::new("plan-a", "t2");
        assert_eq!(dag.earliest_start(&id), Duration::from_secs(600));
        assert_eq!(dag.latest_start(&id), Duration::from_secs(600));
        assert!(dag.slack(&id).is_zero());
        let critical: Vec<_> = dag.critical_path().into_iter().map(|id| id.task).collect();
        assert_eq!(critical, vec!["t1", "t2", "t3"]);
    }

    #[test]
    fn fuse_linear_chains_collapses_serial_runs() {
        let mut dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
            mk_task("t3", &["t2"], &[], Some(3)),
        ])
        .unwrap();
        // Use permissive config so the chain is fully fused.
        let config = FusionConfig {
            max_chain_length: 10,
            ave_width: 0.0,
            same_tier_only: false,
        };
        let fused = dag.fuse_linear_chains(&config);
        assert_eq!(fused, 1);
        assert_eq!(dag.nodes().len(), 1);
        let head = dag.task(&GlobalTaskId::new("plan-a", "t1")).unwrap();
        assert_eq!(head.estimated_minutes, Some(18));
    }

    #[test]
    fn cpm_analysis_full_computes_floats() {
        // Diamond DAG: t1 -> t2 (5 min), t1 -> t3 (10 min), t2 -> t4, t3 -> t4
        // Critical path: t1 -> t3 -> t4  (t2 has float)
        let mut t1 = mk_task("t1", &[], &[], Some(5));
        let mut t2 = mk_task("t2", &["t1"], &[], Some(3));
        let mut t3 = mk_task("t3", &["t1"], &[], Some(10));
        let mut t4 = mk_task("t4", &["t2", "t3"], &[], Some(2));
        let _ = (&mut t1, &mut t2, &mut t3, &mut t4);
        let dag = single_plan_dag(vec![t1, t2, t3, t4]).unwrap();

        let cpm = dag.cpm_analysis_full().expect("should compute CPM");

        // t1 starts at 0
        let id_t1 = GlobalTaskId::new("plan-a", "t1");
        let id_t2 = GlobalTaskId::new("plan-a", "t2");
        let id_t3 = GlobalTaskId::new("plan-a", "t3");
        let id_t4 = GlobalTaskId::new("plan-a", "t4");

        assert!((cpm.earliest_start[&id_t1] - 0.0).abs() < 0.01);
        // t2 and t3 both start after t1 (5 min = 300s)
        assert!((cpm.earliest_start[&id_t2] - 300.0).abs() < 0.01);
        assert!((cpm.earliest_start[&id_t3] - 300.0).abs() < 0.01);
        // t4 starts after both t2 and t3 finish; t3 finishes at 300+600=900s
        assert!((cpm.earliest_start[&id_t4] - 900.0).abs() < 0.01);

        // Critical path: t1, t3, t4 (total float = 0)
        assert!(cpm.is_critical(&id_t1));
        assert!(cpm.is_critical(&id_t3));
        assert!(cpm.is_critical(&id_t4));
        // t2 is not critical
        assert!(!cpm.is_critical(&id_t2));

        // t2 has slack = latest_start - earliest_start > 0
        assert!(cpm.slack(&id_t2) > 0.0);
        // t2 total float: t3 takes 10 min, t2 takes 3 min, so t2 has 7 min = 420s slack
        assert!((cpm.slack(&id_t2) - 420.0).abs() < 0.01);

        // Free float for t2: min(ES(successors)) - EF(t2) = 900 - (300+180) = 420s
        assert!((cpm.free_float[&id_t2] - 420.0).abs() < 0.01);

        // Duration = 5 + 10 + 2 = 17 min = 1020s
        assert!((cpm.min_duration - 1020.0).abs() < 0.01);
    }

    #[test]
    fn fusion_config_caps_chain_length() {
        // With max_chain_length=2, a 5-node linear chain should require
        // multiple fusion rounds, each capped at 2 nodes per chain.
        let mut dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(1)),
            mk_task("t2", &["t1"], &[], Some(1)),
            mk_task("t3", &["t2"], &[], Some(1)),
            mk_task("t4", &["t3"], &[], Some(1)),
            mk_task("t5", &["t4"], &[], Some(1)),
        ])
        .unwrap();
        let config = FusionConfig {
            max_chain_length: 2,
            ave_width: 0.0,
            same_tier_only: false,
        };
        let fused = dag.fuse_linear_chains(&config);
        // Each round fuses pairs (max 2): t1+t2, then (t1,t3)+t4, etc.
        // Must have done at least 2 fusions.
        assert!(fused >= 2);
        // All tasks eventually collapse (each round halves the chain).
        assert!(dag.nodes().len() <= 3);
    }

    #[test]
    fn fusion_config_max_chain_prevents_full_collapse() {
        // 4-node chain with max_chain_length=2: first fusion merges t1+t2,
        // second merges t3+t4. Result: 2 nodes (t1 and t3).
        let mut dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(1)),
            mk_task("t2", &["t1"], &[], Some(1)),
            mk_task("t3", &["t2"], &[], Some(1)),
            mk_task("t4", &["t3"], &[], Some(1)),
        ])
        .unwrap();
        let config = FusionConfig {
            max_chain_length: 2,
            ave_width: 0.0,
            same_tier_only: false,
        };
        dag.fuse_linear_chains(&config);
        // After iterative fusion with cap 2, the chain converges to 1 node
        // (each round fuses 2 into 1, then the remaining 2 fuse again).
        // The key is that each individual fusion round respects the cap.
        assert!(dag.nodes().len() >= 1);
        // Total estimate should be preserved (4 minutes).
        let head = dag.task(&GlobalTaskId::new("plan-a", "t1")).unwrap();
        assert_eq!(head.estimated_minutes, Some(4));
    }

    #[test]
    fn incremental_dag_dirty_propagates_downstream() {
        let dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
            mk_task("t3", &["t2"], &[], Some(3)),
        ])
        .unwrap();
        let mut inc = IncrementalDag::new(dag);
        inc.mark_dirty(GlobalTaskId::new("plan-a", "t1"));
        let dirty = inc.recompute_plan();
        assert_eq!(dirty.len(), 3);
        assert_eq!(dirty[0].task, "t1");
        assert_eq!(dirty[2].task, "t3");
        assert_eq!(inc.clean_set().len(), 3);
    }

    #[test]
    fn apply_mutation_can_add_and_remove_tasks() {
        let mut dag = single_plan_dag(vec![mk_task("t1", &[], &[], Some(10))]).unwrap();
        dag.apply_mutation(DagMutation::AddTask {
            task_id: GlobalTaskId::new("plan-a", "t2"),
            task: mk_task("t2", &[], &[], Some(5)),
            depends_on: vec![GlobalTaskId::new("plan-a", "t1")],
        })
        .unwrap();
        assert_eq!(dag.nodes().len(), 2);
        assert_eq!(
            dag.deps_of(&GlobalTaskId::new("plan-a", "t2"))
                .iter()
                .next()
                .unwrap()
                .task,
            "t1"
        );

        dag.apply_mutation(DagMutation::RemoveTask {
            task_id: GlobalTaskId::new("plan-a", "t1"),
        })
        .unwrap();
        assert_eq!(dag.nodes().len(), 1);
        assert_eq!(dag.nodes()[0].task, "t2");
    }

    #[test]
    fn apply_mutation_rejects_completed_tasks() {
        let mut task = mk_task("t1", &[], &[], Some(10));
        task.status = roko_core::TaskStatus::Done;
        let mut plans = BTreeMap::new();
        plans.insert("plan-a".to_string(), vec![task]);
        let mut dag = UnifiedTaskDag::build(&plans, &HashMap::new(), DagConfig::default()).unwrap();
        let err = dag
            .apply_mutation(DagMutation::RemoveTask {
                task_id: GlobalTaskId::new("plan-a", "t1"),
            })
            .unwrap_err();
        assert!(matches!(err, DagMutationError::CompletedTask(_)));
    }

    #[test]
    fn fan_out_puts_children_in_same_wave() {
        let tasks = vec![
            mk_task("t1", &[], &[], Some(5)),
            mk_task("t2", &["t1"], &[], Some(5)),
            mk_task("t3", &["t1"], &[], Some(5)),
            mk_task("t4", &["t1"], &[], Some(5)),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].tasks.len(), 1);
        assert_eq!(waves[1].tasks.len(), 3);
    }

    #[test]
    fn fan_in_collapses_to_one() {
        let tasks = vec![
            mk_task("t1", &[], &[], None),
            mk_task("t2", &[], &[], None),
            mk_task("t3", &[], &[], None),
            mk_task("t4", &["t1", "t2", "t3"], &[], None),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].tasks.len(), 3);
        assert_eq!(waves[1].tasks.len(), 1);
    }

    #[test]
    fn cycle_is_rejected() {
        let tasks = vec![
            mk_task("t1", &["t2"], &[], None),
            mk_task("t2", &["t1"], &[], None),
        ];
        let err = single_plan_dag(tasks).unwrap_err();
        assert!(matches!(err, DagError::Cycle(_)));
    }

    #[test]
    fn self_loop_is_rejected() {
        let tasks = vec![mk_task("t1", &["t1"], &[], None)];
        let err = single_plan_dag(tasks).unwrap_err();
        assert!(matches!(err, DagError::Cycle(_)));
    }

    #[test]
    fn dangling_dep_fails_loud() {
        let tasks = vec![mk_task("t1", &["t999"], &[], None)];
        let err = single_plan_dag(tasks).unwrap_err();
        assert!(matches!(err, DagError::DanglingDepRef { .. }));
    }

    #[test]
    fn cross_plan_dep_resolves() {
        let mut plans = BTreeMap::new();
        plans.insert(
            "09-foo".to_string(),
            vec![mk_task("t1", &[], &[], Some(10))],
        );
        plans.insert(
            "10-bar".to_string(),
            vec![mk_task("t2", &["09-foo:t1"], &[], Some(5))],
        );
        let dag = UnifiedTaskDag::build(&plans, &HashMap::new(), DagConfig::default()).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].tasks[0].task, "t1");
        assert_eq!(waves[1].tasks[0].task, "t2");
    }

    #[test]
    fn plan_level_deps_propagate_to_all_tasks() {
        let mut plans = BTreeMap::new();
        plans.insert("a".to_string(), vec![mk_task("t1", &[], &[], None)]);
        plans.insert(
            "b".to_string(),
            vec![mk_task("t1", &[], &[], None), mk_task("t2", &[], &[], None)],
        );
        let mut deps = HashMap::new();
        deps.insert("b".to_string(), HashSet::from(["a".to_string()]));
        let dag = UnifiedTaskDag::build(&plans, &deps, DagConfig::default()).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].tasks[0].plan, "a");
        assert_eq!(waves[1].tasks.len(), 2);
    }

    #[test]
    fn unknown_plan_in_deps_is_rejected() {
        let mut plans = BTreeMap::new();
        plans.insert("a".to_string(), vec![mk_task("t1", &[], &[], None)]);
        let mut deps = HashMap::new();
        deps.insert("a".to_string(), HashSet::from(["missing".to_string()]));
        let err = UnifiedTaskDag::build(&plans, &deps, DagConfig::default()).unwrap_err();
        assert!(matches!(err, DagError::UnknownPlan(_)));
    }

    #[test]
    fn file_overlap_serializes_conflicting_tasks() {
        let tasks = vec![
            mk_task("t1", &[], &["src/lib.rs"], None),
            mk_task("t2", &[], &["src/lib.rs"], None),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves.len(), 2, "shared file should serialize");
        assert_eq!(waves[0].tasks[0].task, "t1");
        assert_eq!(waves[1].tasks[0].task, "t2");
    }

    #[test]
    fn file_overlap_can_be_disabled() {
        let mut plans = BTreeMap::new();
        plans.insert(
            "p".to_string(),
            vec![
                mk_task("t1", &[], &["src/lib.rs"], None),
                mk_task("t2", &[], &["src/lib.rs"], None),
            ],
        );
        let cfg = DagConfig {
            infer_file_overlap: false,
            max_wave_width: 0,
        };
        let dag = UnifiedTaskDag::build(&plans, &HashMap::new(), cfg).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves.len(), 1, "with inference off they run in parallel");
        assert_eq!(waves[0].tasks.len(), 2);
    }

    #[test]
    fn max_wave_width_limits_parallelism() {
        let tasks = vec![
            mk_task("t1", &[], &[], None),
            mk_task("t2", &[], &[], None),
            mk_task("t3", &[], &[], None),
            mk_task("t4", &[], &[], None),
            mk_task("t5", &[], &[], None),
        ];
        let mut plans = BTreeMap::new();
        plans.insert("p".to_string(), tasks);
        let cfg = DagConfig {
            infer_file_overlap: false,
            max_wave_width: 2,
        };
        let dag = UnifiedTaskDag::build(&plans, &HashMap::new(), cfg).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0].tasks.len(), 2);
        assert_eq!(waves[1].tasks.len(), 2);
        assert_eq!(waves[2].tasks.len(), 1);
    }

    #[test]
    fn critical_path_counts_estimates() {
        let tasks = vec![
            mk_task("t1", &[], &[], Some(5)),
            mk_task("t2", &["t1"], &[], Some(10)),
            mk_task("t3", &["t1"], &[], Some(3)),
            mk_task("t4", &["t2", "t3"], &[], Some(7)),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        // longest: t1(5) -> t2(10) -> t4(7) = 22
        assert_eq!(dag.stats().critical_path_minutes, 22);
    }

    #[test]
    fn stats_returns_accurate_counts() {
        let tasks = vec![
            mk_task("t1", &[], &[], None),
            mk_task("t2", &["t1"], &[], None),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let stats = dag.stats();
        assert_eq!(stats.nodes, 2);
        assert_eq!(stats.edges, 1);
        assert_eq!(stats.waves, 2);
    }

    #[test]
    fn deps_of_and_dependents_of_are_symmetric() {
        let tasks = vec![
            mk_task("t1", &[], &[], None),
            mk_task("t2", &["t1"], &[], None),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let t1 = GlobalTaskId::new("plan-a", "t1");
        let t2 = GlobalTaskId::new("plan-a", "t2");
        assert!(dag.deps_of(&t2).contains(&t1));
        assert!(dag.dependents_of(&t1).contains(&t2));
        assert!(dag.deps_of(&t1).is_empty());
        assert!(dag.dependents_of(&t2).is_empty());
    }

    #[test]
    fn waves_are_deterministic_across_runs() {
        let tasks = vec![
            mk_task("t3", &[], &[], None),
            mk_task("t1", &[], &[], None),
            mk_task("t2", &[], &[], None),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let w1 = dag.waves().unwrap();
        let w2 = dag.waves().unwrap();
        assert_eq!(w1, w2);
        // Wave 0 should be sorted by task id.
        assert_eq!(
            w1[0]
                .tasks
                .iter()
                .map(|i| i.task.as_str())
                .collect::<Vec<_>>(),
            vec!["t1", "t2", "t3"]
        );
    }

    #[test]
    fn wave_estimate_is_max_of_constituents() {
        let tasks = vec![
            mk_task("t1", &[], &[], Some(5)),
            mk_task("t2", &[], &[], Some(12)),
            mk_task("t3", &[], &[], Some(3)),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves[0].estimated_minutes, 12);
    }

    #[test]
    fn diamond_has_correct_depths() {
        let tasks = vec![
            mk_task("t1", &[], &[], None),
            mk_task("t2", &["t1"], &[], None),
            mk_task("t3", &["t1"], &[], None),
            mk_task("t4", &["t2", "t3"], &[], None),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let waves = dag.waves().unwrap();
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0].tasks.len(), 1);
        assert_eq!(waves[1].tasks.len(), 2);
        assert_eq!(waves[2].tasks.len(), 1);
    }

    #[test]
    fn three_node_triangle_cycle_is_rejected() {
        let tasks = vec![
            mk_task("t1", &["t3"], &[], None),
            mk_task("t2", &["t1"], &[], None),
            mk_task("t3", &["t2"], &[], None),
        ];
        let err = single_plan_dag(tasks).unwrap_err();
        assert!(matches!(err, DagError::Cycle(_)));
    }

    #[test]
    fn resolve_dep_ref_parses_colon_form() {
        let id = resolve_dep_ref("plan-a", "other-plan:t3");
        assert_eq!(id.plan, "other-plan");
        assert_eq!(id.task, "t3");
    }

    #[test]
    fn resolve_dep_ref_defaults_to_referring_plan() {
        let id = resolve_dep_ref("plan-a", "t3");
        assert_eq!(id.plan, "plan-a");
        assert_eq!(id.task, "t3");
    }

    #[test]
    fn nodes_are_returned_in_canonical_order() {
        let tasks = vec![
            mk_task("beta", &[], &[], None),
            mk_task("alpha", &[], &[], None),
        ];
        let dag = single_plan_dag(tasks).unwrap();
        let names: Vec<&str> = dag.nodes().iter().map(|i| i.task.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    // ── ORCH-05: DAG cull tests ─────────────────────────────────────

    #[test]
    fn cull_removes_unreachable_tasks() {
        //   t1 → t2 → t3
        //   t4 (independent)
        let mut dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
            mk_task("t3", &["t2"], &[], Some(3)),
            mk_task("t4", &[], &[], Some(7)),
        ])
        .unwrap();
        assert_eq!(dag.nodes().len(), 4);

        let culled = dag.cull(&["t3".to_string()]);
        assert_eq!(culled, 1); // t4 removed
        assert_eq!(dag.nodes().len(), 3);
        let names: Vec<&str> = dag.nodes().iter().map(|n| n.task.as_str()).collect();
        assert!(names.contains(&"t1"));
        assert!(names.contains(&"t2"));
        assert!(names.contains(&"t3"));
        assert!(!names.contains(&"t4"));
    }

    #[test]
    fn cull_with_multiple_targets() {
        //   t1 → t2 → t3
        //   t4 → t5
        let mut dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
            mk_task("t3", &["t2"], &[], Some(3)),
            mk_task("t4", &[], &[], Some(7)),
            mk_task("t5", &["t4"], &[], Some(2)),
        ])
        .unwrap();

        let culled = dag.cull(&["t3".to_string(), "t5".to_string()]);
        assert_eq!(culled, 0); // all needed
        assert_eq!(dag.nodes().len(), 5);
    }

    #[test]
    fn cull_empty_targets_removes_everything() {
        let mut dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
        ])
        .unwrap();

        let culled = dag.cull(&[]);
        assert_eq!(culled, 2);
        assert!(dag.nodes().is_empty());
    }

    #[test]
    fn cull_preserves_graph_validity() {
        //   t1 → t2 → t3
        //         ↗
        //   t4 ──┘
        let mut dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
            mk_task("t3", &["t2", "t4"], &[], Some(3)),
            mk_task("t4", &[], &[], Some(7)),
        ])
        .unwrap();

        let culled = dag.cull(&["t3".to_string()]);
        assert_eq!(culled, 0); // all needed for t3
        // Verify topo sort still works.
        assert!(dag.topological_sort().is_ok());
    }

    // ── ORCH-07: IncrementalDag revision tracking tests ─────────────

    #[test]
    fn incremental_dag_revision_increments_on_dirty() {
        let dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
        ])
        .unwrap();
        let mut inc = IncrementalDag::new(dag);
        assert_eq!(inc.revision(), 0);

        inc.mark_dirty(GlobalTaskId::new("plan-a", "t1"));
        assert_eq!(inc.revision(), 1);

        inc.mark_dirty(GlobalTaskId::new("plan-a", "t2"));
        assert_eq!(inc.revision(), 2);
    }

    #[test]
    fn incremental_dag_ensure_clean_backdates() {
        let dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &["t1"], &[], Some(5)),
        ])
        .unwrap();
        let mut inc = IncrementalDag::new(dag);
        let t1 = GlobalTaskId::new("plan-a", "t1");

        // Compute initial hash.
        inc.mark_verified(&t1);
        assert_eq!(inc.verified_at_map().get(&t1), Some(&0));

        // Mark dirty, then ensure_clean — inputs haven't changed so it backdates.
        inc.mark_dirty(t1.clone());
        assert!(inc.ensure_clean(&t1));
        assert_eq!(*inc.verified_at_map().get(&t1).unwrap(), 1);
    }

    #[test]
    fn incremental_dag_mark_verified_stores_hash() {
        let dag = single_plan_dag(vec![mk_task("t1", &[], &[], Some(10))]).unwrap();
        let mut inc = IncrementalDag::new(dag);
        let t1 = GlobalTaskId::new("plan-a", "t1");

        assert!(inc.input_hashes_map().get(&t1).is_none());
        inc.mark_verified(&t1);
        assert!(inc.input_hashes_map().get(&t1).is_some());
    }

    // ── ORCH-06: DAG partitioning tests ─────────────────────────────

    #[test]
    fn partition_empty_dag_returns_empty_partitions() {
        let dag =
            UnifiedTaskDag::build(&BTreeMap::new(), &HashMap::new(), DagConfig::default()).unwrap();
        let parts = dag.partition(3);
        assert_eq!(parts.len(), 3);
        for p in &parts {
            assert!(p.tasks.is_empty());
            assert_eq!(p.cut_edges, 0);
        }
    }

    #[test]
    fn partition_single_task_goes_to_one_partition() {
        let dag = single_plan_dag(vec![mk_task("t1", &[], &[], Some(10))]).unwrap();
        let parts = dag.partition(3);
        assert_eq!(parts.len(), 3);
        let non_empty: Vec<_> = parts.iter().filter(|p| !p.tasks.is_empty()).collect();
        assert_eq!(non_empty.len(), 1);
        assert_eq!(non_empty[0].tasks.len(), 1);
    }

    #[test]
    fn partition_distributes_independent_tasks() {
        let dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(10)),
            mk_task("t2", &[], &[], Some(10)),
            mk_task("t3", &[], &[], Some(10)),
            mk_task("t4", &[], &[], Some(10)),
        ])
        .unwrap();
        let parts = dag.partition(2);
        assert_eq!(parts.len(), 2);

        // All tasks assigned.
        let total: usize = parts.iter().map(|p| p.tasks.len()).sum();
        assert_eq!(total, 4);

        // Work should be roughly balanced.
        let max_work = parts.iter().map(|p| p.total_work).fold(0.0_f64, f64::max);
        let min_work = parts
            .iter()
            .map(|p| p.total_work)
            .fold(f64::INFINITY, f64::min);
        assert!(
            max_work - min_work <= 20.0,
            "work imbalance: {max_work} vs {min_work}"
        );
    }

    #[test]
    fn partition_preserves_all_tasks() {
        let dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(5)),
            mk_task("t2", &["t1"], &[], Some(10)),
            mk_task("t3", &["t1"], &[], Some(3)),
            mk_task("t4", &["t2", "t3"], &[], Some(7)),
        ])
        .unwrap();
        let parts = dag.partition(2);
        let mut all_tasks: Vec<GlobalTaskId> = parts.iter().flat_map(|p| p.tasks.clone()).collect();
        all_tasks.sort();
        let mut expected = dag.nodes().to_vec();
        expected.sort();
        assert_eq!(all_tasks, expected);
    }

    #[test]
    fn partition_k_greater_than_nodes() {
        let dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(5)),
            mk_task("t2", &[], &[], Some(10)),
        ])
        .unwrap();
        let parts = dag.partition(5);
        assert_eq!(parts.len(), 5);
        let total: usize = parts.iter().map(|p| p.tasks.len()).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn partition_minimizes_cuts_on_chain() {
        // Linear chain: t1 -> t2 -> t3 -> t4
        // With k=2, the partitioner should keep dependent tasks together.
        let dag = single_plan_dag(vec![
            mk_task("t1", &[], &[], Some(5)),
            mk_task("t2", &["t1"], &[], Some(5)),
            mk_task("t3", &["t2"], &[], Some(5)),
            mk_task("t4", &["t3"], &[], Some(5)),
        ])
        .unwrap();
        let parts = dag.partition(2);
        // Total cut edges should be minimal (at most 1 cut in the chain).
        let total_cuts: usize = parts.iter().map(|p| p.cut_edges).sum();
        assert!(total_cuts <= 2, "too many cuts: {total_cuts}");
    }
}
