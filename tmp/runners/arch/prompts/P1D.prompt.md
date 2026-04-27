## Batch P1D: GateService

### Write Scope
- **CREATE**: `crates/roko-gate/src/gate_service.rs`
- **MODIFY**: `crates/roko-gate/src/lib.rs` (add `pub mod gate_service;` and re-export)

### Dependencies
- P0A (RuntimeEvent types)
- P0B (GateRunner trait, GateConfig, GateReport, GateVerdict)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Create a new crate
- Duplicate existing gate implementations

### Existing Code Context

`roko-gate` already has concrete gate implementations:
```rust
pub struct CompileGate;   // rung 0
pub struct ClippyGate;    // rung 1
pub struct TestGate;      // rung 2

#[async_trait]
pub trait Verify: Send + Sync {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Result<Verdict>;
    fn name(&self) -> &str;
}
```

And adaptive thresholds:
```rust
pub struct AdaptiveThresholds { /* ... */ }
```

And a GatePayload:
```rust
impl GatePayload {
    pub fn in_dir(workdir: &Path) -> Self;
}
```

### Task

Create `GateService` — a concrete implementation of the `GateRunner` trait. It wraps
existing gate implementations (CompileGate, TestGate, ClippyGate) and runs them based
on the `GateConfig`.

#### File: `crates/roko-gate/src/gate_service.rs`

```rust
//! GateService — concrete implementation of `GateRunner`.
//!
//! Wraps existing gate implementations (CompileGate, TestGate, ClippyGate)
//! and runs them according to GateConfig.

use anyhow::Result;
use async_trait::async_trait;
use roko_core::foundation::{GateConfig, GateReport, GateRunner, GateVerdict};
use roko_core::{Context, Engram};
use std::time::Instant;

use crate::compile::CompileGate;
use crate::clippy::ClippyGate;
use crate::test_gate::TestGate;
use crate::Verify;
use crate::payload::GatePayload;

/// Service that runs verification gates via the existing gate infrastructure.
///
/// This is the canonical way to run gates in the workflow engine. It:
/// - Selects which gates to run based on GateConfig
/// - Runs gates sequentially (compile first, fail-fast)
/// - Tracks duration per gate
/// - Returns a unified GateReport
pub struct GateService;

impl GateService {
    pub fn new() -> Self {
        Self
    }

    /// Map a gate name to a concrete gate implementation.
    fn gate_for_name(&self, name: &str) -> Option<Box<dyn Verify>> {
        match name {
            "compile" => Some(Box::new(CompileGate)),
            "clippy" => Some(Box::new(ClippyGate)),
            "test" => Some(Box::new(TestGate)),
            _ => None,
        }
    }
}

impl Default for GateService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GateRunner for GateService {
    async fn run_gates(&self, config: GateConfig) -> Result<GateReport> {
        let mut verdicts = Vec::new();

        // Build signal and context from workdir
        let payload = GatePayload::in_dir(&config.workdir);
        let signal = Engram::default();
        let ctx = Context::default();

        for gate_name in &config.enabled_gates {
            let gate = match self.gate_for_name(gate_name) {
                Some(g) => g,
                None => {
                    verdicts.push(GateVerdict {
                        gate_name: gate_name.clone(),
                        passed: false,
                        output: format!("Unknown gate: {}", gate_name),
                        duration_ms: 0,
                    });
                    continue;
                }
            };

            let start = Instant::now();
            let result = gate.verify(&signal, &ctx).await;
            let duration_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(verdict) => {
                    verdicts.push(GateVerdict {
                        gate_name: gate_name.clone(),
                        passed: verdict.passed(),
                        output: verdict.details().unwrap_or_default().to_string(),
                        duration_ms,
                    });
                }
                Err(e) => {
                    verdicts.push(GateVerdict {
                        gate_name: gate_name.clone(),
                        passed: false,
                        output: format!("Gate error: {}", e),
                        duration_ms,
                    });
                }
            }
        }

        Ok(GateReport { verdicts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_gate_produces_failure() {
        let svc = GateService::new();
        assert!(svc.gate_for_name("nonexistent").is_none());
        assert!(svc.gate_for_name("compile").is_some());
    }
}
```

**Important**: The actual gate constructors and `Verify` trait may differ from what's shown.
Read the actual source files to verify:
- How `CompileGate`, `TestGate`, `ClippyGate` are constructed
- What `Verdict` looks like (`passed()` method, `details()` method)
- How `GatePayload::in_dir()` works
- What `Context::default()` provides (you may need to set `workdir` on it)

Adapt the implementation to match the real APIs. The key contract is: implement `GateRunner`
using existing gate infrastructure.

#### Modification: `crates/roko-gate/src/lib.rs`

Add:
```rust
pub mod gate_service;
pub use gate_service::GateService;
```

### Done Criteria
```bash
grep -q 'pub struct GateService' crates/roko-gate/src/gate_service.rs
grep -q 'impl GateRunner for GateService' crates/roko-gate/src/gate_service.rs
grep -q 'pub mod gate_service' crates/roko-gate/src/lib.rs
cargo check -p roko-gate
```
