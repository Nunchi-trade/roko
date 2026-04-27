## Batch P1B: PromptAssemblyService

### Write Scope
- **CREATE**: `crates/roko-compose/src/prompt_assembly_service.rs`
- **MODIFY**: `crates/roko-compose/src/lib.rs` (add `pub mod prompt_assembly_service;` and re-export)

### Dependencies
- P0A (RuntimeEvent types)
- P0B (PromptAssembler trait, PromptSpec)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Create inline prompt strings (format!("You are the..."))
- Create a new crate

### Existing Code Context

`roko-compose` already has the 9-layer `SystemPromptBuilder`:
```rust
pub struct SystemPromptBuilder {
    role_identity: String,
    conventions: Option<String>,
    domain: Option<String>,
    task: Option<String>,
    gate_feedback: Vec<String>,
    tools: Option<String>,
    anti_patterns: Vec<String>,
    // ... more fields
}

impl SystemPromptBuilder {
    pub fn new(role_identity: &str) -> Self;
    pub fn with_conventions(self, text: &str) -> Self;
    pub fn with_task(self, text: &str) -> Self;
    pub fn build(self) -> String;
}
```

And role prompt sources:
```rust
pub fn role_identity_for(role: &str) -> String;
pub fn role_prompt_source_for(role: &str) -> RolePromptSource;
```

### Task

Create `PromptAssemblyService` — a concrete implementation of the `PromptAssembler` trait.
It wraps `SystemPromptBuilder` with role resolution, convention detection, and gate feedback
injection.

#### File: `crates/roko-compose/src/prompt_assembly_service.rs`

```rust
//! PromptAssemblyService — concrete implementation of `PromptAssembler`.
//!
//! Wraps the existing `SystemPromptBuilder` with role resolution,
//! convention detection, and gate feedback injection.

use anyhow::Result;
use async_trait::async_trait;
use roko_core::foundation::{PromptAssembler, PromptSpec};

use crate::system_prompt_builder::SystemPromptBuilder;
use crate::role_prompt::role_identity_for;

/// Service that assembles system prompts via the 9-layer SystemPromptBuilder.
///
/// This is the canonical way to build prompts in the workflow engine. It:
/// - Resolves role identity from role name
/// - Detects project conventions from the working directory
/// - Injects gate feedback from prior iterations
/// - Applies anti-patterns
pub struct PromptAssemblyService {
    /// Default conventions text (used when workdir detection unavailable)
    default_conventions: Option<String>,
}

impl PromptAssemblyService {
    /// Create a new PromptAssemblyService.
    pub fn new() -> Self {
        Self {
            default_conventions: None,
        }
    }

    /// Create with default conventions text.
    pub fn with_conventions(mut self, conventions: String) -> Self {
        self.default_conventions = Some(conventions);
        self
    }
}

impl Default for PromptAssemblyService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PromptAssembler for PromptAssemblyService {
    async fn assemble(&self, spec: PromptSpec) -> Result<String> {
        // Layer 1: Role identity
        let role = spec.role.as_deref().unwrap_or("implementer");
        let identity = role_identity_for(role);

        let mut builder = SystemPromptBuilder::new(&identity);

        // Layer 2: Conventions
        if let Some(ref conventions) = self.default_conventions {
            builder = builder.with_conventions(conventions);
        }

        // Layer 4: Task context
        if let Some(ref task) = spec.task {
            builder = builder.with_task(task);
        }

        // Layer 4b: Gate feedback from prior iterations
        for feedback in &spec.gate_feedback {
            builder = builder.with_gate_feedback(feedback);
        }

        // Layer 7: Anti-patterns
        if !spec.anti_patterns.is_empty() {
            builder = builder.with_anti_patterns(
                spec.anti_patterns.iter().map(|s| s.as_str()).collect(),
            );
        }

        Ok(builder.build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn basic_assembly() {
        let svc = PromptAssemblyService::new();
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix the login bug".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
    }

    #[tokio::test]
    async fn assembly_with_gate_feedback() {
        let svc = PromptAssemblyService::new();
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix the build".into()),
                gate_feedback: vec!["error[E0308]: mismatched types".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
    }

    #[tokio::test]
    async fn default_role_is_implementer() {
        let svc = PromptAssemblyService::new();
        let prompt = svc
            .assemble(PromptSpec {
                task: Some("Do something".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
    }
}
```

**Important**: The `SystemPromptBuilder` methods may have slightly different signatures than
shown above. Read the actual `system_prompt_builder.rs` file to verify method names before
implementing. Adapt the implementation to match the real API.

#### Modification: `crates/roko-compose/src/lib.rs`

Add:
```rust
pub mod prompt_assembly_service;
pub use prompt_assembly_service::PromptAssemblyService;
```

### Done Criteria
```bash
grep -q 'pub struct PromptAssemblyService' crates/roko-compose/src/prompt_assembly_service.rs
grep -q 'impl PromptAssembler for PromptAssemblyService' crates/roko-compose/src/prompt_assembly_service.rs
grep -q 'pub mod prompt_assembly_service' crates/roko-compose/src/lib.rs
! grep -rn 'format!.*You are.*the' crates/roko-compose/src/prompt_assembly_service.rs
cargo check -p roko-compose
```
