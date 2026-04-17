//! Resume-time reconciliation between an executor snapshot and discovered plans.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use roko_orchestrator::{ExecutorSnapshot, PlanInfo};

/// Errors returned when a resume snapshot no longer matches the discovered plan set.
#[derive(Debug)]
pub enum SnapshotReconcileError {
    /// One or more plan ids from the snapshot are absent from the discovered plans directory.
    PlanIdsMissing {
        /// Path to the snapshot file the operator attempted to resume from.
        snapshot_path: PathBuf,
        /// Root directory used for plan discovery during resume.
        plans_root: PathBuf,
        /// Plan ids referenced by the snapshot but missing from the discovered plan set.
        missing: Vec<String>,
        /// Plan ids discovered on disk at resume time.
        discovered: Vec<String>,
    },
}

/// Validate that every plan id referenced by the snapshot still exists in the discovered plan set.
///
/// Extra plans on disk are allowed. Missing snapshot plan ids are a hard error because resume would
/// otherwise run against a partial plan set.
pub fn reconcile_snapshot_vs_plans(
    snapshot: &ExecutorSnapshot,
    discovered: &[PlanInfo],
    snapshot_path: &Path,
    plans_root: &Path,
) -> Result<(), SnapshotReconcileError> {
    let discovered_ids = discovered_plan_ids(discovered);
    let missing: Vec<String> = snapshot_plan_ids(snapshot)
        .into_iter()
        .filter(|id| !discovered_ids.contains(id))
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    Err(SnapshotReconcileError::PlanIdsMissing {
        snapshot_path: snapshot_path.to_path_buf(),
        plans_root: plans_root.to_path_buf(),
        missing,
        discovered: discovered_ids.into_iter().collect(),
    })
}

impl fmt::Display for SnapshotReconcileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlanIdsMissing {
                snapshot_path,
                plans_root,
                missing,
                discovered,
            } => write!(
                f,
                "resume snapshot references plans [{}] but plans/ at {} has [{}]. Rename or prune the snapshot before resuming.\n  - Snapshot path: {}\n  - Plans root: {}\n  - Missing: {}",
                format_plan_list(missing),
                plans_root.display(),
                format_plan_list(discovered),
                snapshot_path.display(),
                plans_root.display(),
                format_plan_list(missing),
            ),
        }
    }
}

impl std::error::Error for SnapshotReconcileError {}

fn snapshot_plan_ids(snapshot: &ExecutorSnapshot) -> Vec<String> {
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();

    for plan_id in &snapshot.queue_order {
        if seen.insert(plan_id.clone()) {
            ordered.push(plan_id.clone());
        }
    }

    let mut remaining: Vec<String> = snapshot
        .plan_states
        .keys()
        .filter(|plan_id| seen.insert((*plan_id).clone()))
        .cloned()
        .collect();
    remaining.sort();
    ordered.extend(remaining);

    ordered
}

fn discovered_plan_ids(discovered: &[PlanInfo]) -> BTreeSet<String> {
    discovered
        .iter()
        .map(|plan| {
            plan.frontmatter
                .as_ref()
                .and_then(|fm| fm.plan.clone())
                .unwrap_or_else(|| plan.base.clone())
        })
        .collect()
}

fn format_plan_list(values: &[String]) -> String {
    values.join(", ")
}
