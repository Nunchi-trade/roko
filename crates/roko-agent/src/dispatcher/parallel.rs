//! Parallel vs serial batch-dispatch (§36.41).
//!
//! When an LLM emits ≥2 tool calls in one turn, this module partitions
//! them by [`ToolConcurrency`]: `Parallel` tools go through
//! `futures::future::join_all` (concurrent), `Serial` tools run one at a
//! time (preserves shell-state ordering, avoids write-write races).
//!
//! Unknown tools (not registered) default to **serial** — it's the safer
//! assumption. A parallel batch of unknown calls could race each other
//! in surprising ways if a plugin later wires them up with shared state.

use roko_core::tool::{ToolCall, ToolConcurrency, ToolRegistry};

/// Partition calls into `(parallel, serial)` groups based on their
/// registered [`ToolConcurrency`]. Unknown tools fall into `serial`.
#[must_use]
pub fn partition_by_concurrency(
    calls: Vec<ToolCall>,
    registry: &dyn ToolRegistry,
) -> (Vec<ToolCall>, Vec<ToolCall>) {
    let mut parallel = Vec::new();
    let mut serial = Vec::new();
    for call in calls {
        let is_parallel = registry
            .get(&call.name)
            .is_some_and(|d| d.concurrency == ToolConcurrency::Parallel);
        if is_parallel {
            parallel.push(call);
        } else {
            serial.push(call);
        }
    }
    (parallel, serial)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolDef, ToolPermission, VecToolRegistry};

    fn read_file() -> ToolDef {
        ToolDef::new(
            "read_file",
            "r",
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
        .with_concurrency(ToolConcurrency::Parallel)
    }

    fn bash() -> ToolDef {
        ToolDef::new("bash", "x", ToolCategory::Exec, ToolPermission::executes())
            .with_concurrency(ToolConcurrency::Serial)
    }

    #[test]
    fn partition_splits_by_concurrency() {
        let registry = VecToolRegistry::from_tools(vec![read_file(), bash()]);
        let calls = vec![
            ToolCall::new("a", "read_file", serde_json::json!({})),
            ToolCall::new("b", "bash", serde_json::json!({})),
            ToolCall::new("c", "read_file", serde_json::json!({})),
        ];
        let (par, ser) = partition_by_concurrency(calls, &registry);
        assert_eq!(par.len(), 2, "expected two parallel reads");
        assert_eq!(ser.len(), 1, "expected one serial bash");
        assert!(par.iter().all(|c| c.name == "read_file"));
        assert_eq!(ser[0].name, "bash");
    }

    #[test]
    fn partition_unknown_tools_go_serial() {
        let registry = VecToolRegistry::from_tools(vec![read_file()]);
        let calls = vec![
            ToolCall::new("a", "read_file", serde_json::json!({})),
            ToolCall::new("b", "mystery_tool", serde_json::json!({})),
        ];
        let (par, ser) = partition_by_concurrency(calls, &registry);
        assert_eq!(par.len(), 1, "only the known parallel tool");
        assert_eq!(ser.len(), 1, "unknown tool defaults to serial");
        assert_eq!(ser[0].name, "mystery_tool");
    }
}
