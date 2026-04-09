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

use roko_core::{GlobalTaskId, Task};
use thiserror::Error;

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

/// Unified DAG over every plan's tasks plus plan-level dependencies.
#[derive(Debug, Clone)]
pub struct UnifiedTaskDag {
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
        // Collect all nodes (sorted by GlobalTaskId).
        let mut nodes: Vec<GlobalTaskId> = plan_tasks
            .iter()
            .flat_map(|(plan, tasks)| {
                tasks
                    .iter()
                    .map(move |t| GlobalTaskId::new(plan.clone(), t.id.clone()))
            })
            .collect();
        nodes.sort_by(|a, b| {
            (a.plan.as_str(), a.task.as_str()).cmp(&(b.plan.as_str(), b.task.as_str()))
        });
        let node_set: HashSet<GlobalTaskId> = nodes.iter().cloned().collect();
        let mut edges: HashMap<GlobalTaskId, BTreeSet<GlobalTaskId>> = HashMap::new();
        let mut estimates: HashMap<GlobalTaskId, u32> = HashMap::new();
        let mut files: HashMap<GlobalTaskId, Vec<String>> = HashMap::new();
        for n in &nodes {
            edges.insert(n.clone(), BTreeSet::new());
        }
        // Intra- and cross-plan task deps + estimates + files.
        for (plan, tasks) in plan_tasks {
            for task in tasks {
                let id = GlobalTaskId::new(plan.clone(), task.id.clone());
                estimates.insert(id.clone(), task.estimated_minutes.unwrap_or(0));
                files.insert(id.clone(), task.files.clone());
                for raw in &task.depends_on {
                    let dep_id = resolve_dep_ref(plan, raw);
                    if !node_set.contains(&dep_id) {
                        return Err(DagError::DanglingDepRef {
                            referrer: id,
                            target: raw.clone(),
                        });
                    }
                    edges.entry(id.clone()).or_default().insert(dep_id);
                }
            }
        }
        // Plan-level deps: every task in `plan` depends on every task in each `dep` plan.
        for (plan, deps) in plan_deps {
            let plan_ids: Vec<GlobalTaskId> = plan_tasks[plan]
                .iter()
                .map(|t| GlobalTaskId::new(plan.clone(), t.id.clone()))
                .collect();
            for dep_plan in deps {
                for dep_task in &plan_tasks[dep_plan] {
                    let dep_id = GlobalTaskId::new(dep_plan.clone(), dep_task.id.clone());
                    for id in &plan_ids {
                        edges.entry(id.clone()).or_default().insert(dep_id.clone());
                    }
                }
            }
        }
        // File-overlap inference (deterministic: earlier GlobalTaskId runs first).
        if config.infer_file_overlap {
            let mut by_file: HashMap<String, BTreeSet<GlobalTaskId>> = HashMap::new();
            for (id, fs) in &files {
                for f in fs {
                    by_file.entry(f.clone()).or_default().insert(id.clone());
                }
            }
            for (_, tasks) in by_file {
                // Iterate every pair; the later node depends on all earlier ones.
                let ordered: Vec<_> = tasks.into_iter().collect();
                for i in 0..ordered.len() {
                    for j in 0..i {
                        // earlier wins; edges[i] gains a dep on ordered[j]
                        // (avoid inserting a self-loop).
                        if ordered[i] != ordered[j] {
                            edges
                                .entry(ordered[i].clone())
                                .or_default()
                                .insert(ordered[j].clone());
                        }
                    }
                }
            }
        }
        // Reverse edges.
        let mut reverse_edges: HashMap<GlobalTaskId, BTreeSet<GlobalTaskId>> = HashMap::new();
        for n in &nodes {
            reverse_edges.insert(n.clone(), BTreeSet::new());
        }
        for (from, deps) in &edges {
            for dep in deps {
                reverse_edges
                    .entry(dep.clone())
                    .or_default()
                    .insert(from.clone());
            }
        }
        let dag = Self {
            edges,
            reverse_edges,
            nodes,
            estimates,
            config,
        };
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

    /// Every task in canonical order.
    #[must_use]
    pub fn nodes(&self) -> &[GlobalTaskId] {
        &self.nodes
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
            let stuck: Vec<GlobalTaskId> = remaining_deps
                .into_iter()
                .filter(|(_, n)| *n > 0)
                .map(|(k, _)| k)
                .collect();
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
        // Critical path: longest path by estimated_minutes (DP on DAG).
        let mut longest: HashMap<GlobalTaskId, u32> = HashMap::new();
        // Process nodes in topological order so every dep has been seen
        // before the node itself.
        if let Ok(topo) = self.topological_sort() {
            for id in &topo {
                let est = self.estimates.get(id).copied().unwrap_or(0);
                let max_dep = self
                    .edges
                    .get(id)
                    .into_iter()
                    .flatten()
                    .map(|d| longest.get(d).copied().unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                longest.insert(id.clone(), max_dep + est);
            }
        }
        let critical = longest.values().copied().max().unwrap_or(0);
        let wave_count = self.waves().map(|w| w.len()).unwrap_or(0);
        DagStats {
            nodes: self.nodes.len(),
            edges: edge_count,
            waves: wave_count,
            critical_path_minutes: critical,
        }
    }
}

/// Resolve a `depends_on` string against the referring plan:
///
/// - `"t3"` → `GlobalTaskId { plan: referrer_plan, task: "t3" }`.
/// - `"09-foo:t3"` → `GlobalTaskId { plan: "09-foo", task: "t3" }`.
fn resolve_dep_ref(referrer_plan: &str, raw: &str) -> GlobalTaskId {
    GlobalTaskId::parse(raw).unwrap_or_else(|| GlobalTaskId::new(referrer_plan, raw))
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
}
