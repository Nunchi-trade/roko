//! Queue reordering strategies for the parallel executor.
//!
//! When a plan fails or priorities change, the executor needs to reorder
//! its internal queue. This module provides two reordering functions:
//!
//! - [`reorder_queue`] — moves a failed plan to the back of the queue
//! - [`priority_reorder`] — sorts the queue by priority (higher first)

use std::collections::HashMap;
use std::hash::BuildHasher;

/// Move a failed plan to the back of the queue.
///
/// If `failed_plan_id` is not found in `queue`, the queue is returned
/// unchanged. The relative order of all other plans is preserved.
#[must_use]
pub fn reorder_queue(queue: &[String], failed_plan_id: &str) -> Vec<String> {
    let mut result: Vec<String> = queue
        .iter()
        .filter(|id| id.as_str() != failed_plan_id)
        .cloned()
        .collect();

    if queue.iter().any(|id| id.as_str() == failed_plan_id) {
        result.push(failed_plan_id.to_string());
    }

    result
}

/// Sort the queue by priority (higher priority first).
///
/// Plans not found in `plan_priorities` are treated as priority 0.
/// Ties are broken by preserving the original order (stable sort).
#[must_use]
pub fn priority_reorder<S: BuildHasher>(queue: &[String], plan_priorities: &HashMap<String, u32, S>) -> Vec<String> {
    let mut indexed: Vec<(usize, &String)> = queue.iter().enumerate().collect();
    indexed.sort_by(|(idx_a, id_a), (idx_b, id_b)| {
        let prio_a = plan_priorities.get(id_a.as_str()).copied().unwrap_or(0);
        let prio_b = plan_priorities.get(id_b.as_str()).copied().unwrap_or(0);
        // Higher priority first, then original order as tie-breaker.
        prio_b.cmp(&prio_a).then(idx_a.cmp(idx_b))
    });
    indexed.into_iter().map(|(_, id)| id.clone()).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn queue(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    // ── reorder_queue ──

    #[test]
    fn failed_plan_moves_to_back() {
        let q = queue(&["a", "b", "c", "d"]);
        let result = reorder_queue(&q, "b");
        assert_eq!(result, queue(&["a", "c", "d", "b"]));
    }

    #[test]
    fn failed_plan_already_at_back() {
        let q = queue(&["a", "b", "c"]);
        let result = reorder_queue(&q, "c");
        assert_eq!(result, queue(&["a", "b", "c"]));
    }

    #[test]
    fn failed_plan_not_in_queue() {
        let q = queue(&["a", "b"]);
        let result = reorder_queue(&q, "z");
        assert_eq!(result, queue(&["a", "b"]));
    }

    #[test]
    fn empty_queue_stays_empty() {
        let q: Vec<String> = vec![];
        let result = reorder_queue(&q, "x");
        assert!(result.is_empty());
    }

    #[test]
    fn single_element_queue() {
        let q = queue(&["a"]);
        let result = reorder_queue(&q, "a");
        assert_eq!(result, queue(&["a"]));
    }

    #[test]
    fn reorder_preserves_relative_order() {
        let q = queue(&["x", "a", "y", "b", "z"]);
        let result = reorder_queue(&q, "a");
        assert_eq!(result, queue(&["x", "y", "b", "z", "a"]));
    }

    // ── priority_reorder ──

    #[test]
    fn sorts_by_priority_descending() {
        let q = queue(&["low", "high", "mid"]);
        let mut prios = HashMap::new();
        prios.insert("low".into(), 1);
        prios.insert("mid".into(), 5);
        prios.insert("high".into(), 10);
        let result = priority_reorder(&q, &prios);
        assert_eq!(result, queue(&["high", "mid", "low"]));
    }

    #[test]
    fn ties_preserve_original_order() {
        let q = queue(&["a", "b", "c"]);
        let mut prios = HashMap::new();
        prios.insert("a".into(), 5);
        prios.insert("b".into(), 5);
        prios.insert("c".into(), 5);
        let result = priority_reorder(&q, &prios);
        // All same priority => original order preserved.
        assert_eq!(result, queue(&["a", "b", "c"]));
    }

    #[test]
    fn missing_priorities_treated_as_zero() {
        let q = queue(&["known", "unknown"]);
        let mut prios = HashMap::new();
        prios.insert("known".into(), 10);
        let result = priority_reorder(&q, &prios);
        assert_eq!(result, queue(&["known", "unknown"]));
    }

    #[test]
    fn empty_queue_priority_reorder() {
        let q: Vec<String> = vec![];
        let prios = HashMap::new();
        let result = priority_reorder(&q, &prios);
        assert!(result.is_empty());
    }

    #[test]
    fn all_unknown_priorities_preserves_order() {
        let q = queue(&["c", "b", "a"]);
        let prios = HashMap::new();
        let result = priority_reorder(&q, &prios);
        assert_eq!(result, queue(&["c", "b", "a"]));
    }

    #[test]
    fn mixed_known_and_unknown_priorities() {
        let q = queue(&["x", "y", "z", "w"]);
        let mut prios = HashMap::new();
        prios.insert("z".into(), 100);
        prios.insert("x".into(), 50);
        // y and w have no priority -> 0
        let result = priority_reorder(&q, &prios);
        assert_eq!(result, queue(&["z", "x", "y", "w"]));
    }
}
