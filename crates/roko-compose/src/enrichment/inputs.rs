//! Per-step input dependency graph.
//!
//! Ported from `apps/mori/src/support_enrich/mod.rs` lines 697-740.
//! Each step declares which plan-directory files it depends on. This is used
//! for staleness checks (skip if all inputs are older than the output).

use std::path::{Path, PathBuf};

use super::step::EnrichStep;

/// Return the list of file paths that a given step depends on.
///
/// All paths are relative to the plan directory. The plan file (`plan.md`) is
/// always included as a dependency for every step.
///
/// Ported from Mori `step_dependency_paths` (lines 697-740).
pub fn step_dependency_paths(plan_dir: &Path, step: EnrichStep) -> Vec<PathBuf> {
    let mut deps = vec![plan_dir.join("plan.md")];

    match step {
        // These only depend on plan.md.
        EnrichStep::Prd | EnrichStep::Briefs | EnrichStep::Invariants => {}

        // Depend on tasks.toml.
        EnrichStep::Tasks
        | EnrichStep::Verify
        | EnrichStep::Reviews
        | EnrichStep::Tests
        | EnrichStep::Scribe => {
            deps.push(plan_dir.join("tasks.toml"));
        }

        // Decompose depends on brief.
        EnrichStep::Decompose => {
            deps.push(plan_dir.join("brief.md"));
        }

        // Research depends on many upstream artifacts.
        EnrichStep::Research => {
            deps.push(plan_dir.join("tasks.toml"));
            deps.push(plan_dir.join("brief.md"));
            deps.push(plan_dir.join("decomposition.md"));
            deps.push(plan_dir.join("verify-tasks.toml"));
            deps.push(plan_dir.join("review-tasks.toml"));
        }

        // Dependencies depends on tasks + brief + research.
        EnrichStep::Dependencies => {
            deps.push(plan_dir.join("tasks.toml"));
            deps.push(plan_dir.join("brief.md"));
            deps.push(plan_dir.join("research.md"));
        }

        // Fixtures depends on tasks + brief + research + dependency manifest.
        EnrichStep::Fixtures => {
            deps.push(plan_dir.join("tasks.toml"));
            deps.push(plan_dir.join("brief.md"));
            deps.push(plan_dir.join("research.md"));
            deps.push(plan_dir.join("dependency-manifest.toml"));
        }

        // Integration depends on many artifacts.
        EnrichStep::Integration => {
            deps.push(plan_dir.join("tasks.toml"));
            deps.push(plan_dir.join("verify-tasks.toml"));
            deps.push(plan_dir.join("review-tasks.toml"));
            deps.push(plan_dir.join("research.md"));
            deps.push(plan_dir.join("dependency-manifest.toml"));
            deps.push(plan_dir.join("fixture-manifest.toml"));
        }
    }

    deps.sort();
    deps.dedup();
    deps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn prd_depends_only_on_plan() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Prd);
        assert_eq!(deps, vec![dir.join("plan.md")]);
    }

    #[test]
    fn briefs_depends_only_on_plan() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Briefs);
        assert_eq!(deps, vec![dir.join("plan.md")]);
    }

    #[test]
    fn tasks_depends_on_plan_and_tasks() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Tasks);
        assert!(deps.contains(&dir.join("plan.md")));
        assert!(deps.contains(&dir.join("tasks.toml")));
    }

    #[test]
    fn decompose_depends_on_plan_and_brief() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Decompose);
        assert!(deps.contains(&dir.join("plan.md")));
        assert!(deps.contains(&dir.join("brief.md")));
    }

    #[test]
    fn research_depends_on_five_files() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Research);
        assert!(deps.contains(&dir.join("plan.md")));
        assert!(deps.contains(&dir.join("tasks.toml")));
        assert!(deps.contains(&dir.join("brief.md")));
        assert!(deps.contains(&dir.join("decomposition.md")));
        assert!(deps.contains(&dir.join("verify-tasks.toml")));
        assert!(deps.contains(&dir.join("review-tasks.toml")));
        assert_eq!(deps.len(), 6);
    }

    #[test]
    fn dependencies_depends_on_three_files() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Dependencies);
        assert!(deps.contains(&dir.join("plan.md")));
        assert!(deps.contains(&dir.join("tasks.toml")));
        assert!(deps.contains(&dir.join("brief.md")));
        assert!(deps.contains(&dir.join("research.md")));
        assert_eq!(deps.len(), 4);
    }

    #[test]
    fn fixtures_depends_on_four_files() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Fixtures);
        assert!(deps.contains(&dir.join("plan.md")));
        assert!(deps.contains(&dir.join("tasks.toml")));
        assert!(deps.contains(&dir.join("brief.md")));
        assert!(deps.contains(&dir.join("research.md")));
        assert!(deps.contains(&dir.join("dependency-manifest.toml")));
        assert_eq!(deps.len(), 5);
    }

    #[test]
    fn integration_depends_on_six_files() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Integration);
        assert!(deps.contains(&dir.join("plan.md")));
        assert!(deps.contains(&dir.join("tasks.toml")));
        assert!(deps.contains(&dir.join("verify-tasks.toml")));
        assert!(deps.contains(&dir.join("review-tasks.toml")));
        assert!(deps.contains(&dir.join("research.md")));
        assert!(deps.contains(&dir.join("dependency-manifest.toml")));
        assert!(deps.contains(&dir.join("fixture-manifest.toml")));
        assert_eq!(deps.len(), 7);
    }

    #[test]
    fn deps_are_sorted_and_deduped() {
        let dir = Path::new("/plans/test");
        for step in super::super::step::ALL_ORDERED {
            let deps = step_dependency_paths(dir, *step);
            let mut sorted = deps.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(deps, sorted, "deps for {step} should be sorted and deduped");
        }
    }

    #[test]
    fn verify_depends_on_tasks() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Verify);
        assert!(deps.contains(&dir.join("tasks.toml")));
    }

    #[test]
    fn reviews_depends_on_tasks() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Reviews);
        assert!(deps.contains(&dir.join("tasks.toml")));
    }

    #[test]
    fn scribe_depends_on_tasks() {
        let dir = Path::new("/plans/test");
        let deps = step_dependency_paths(dir, EnrichStep::Scribe);
        assert!(deps.contains(&dir.join("tasks.toml")));
    }
}
