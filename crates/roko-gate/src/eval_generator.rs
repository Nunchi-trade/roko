//! Autonomous evaluation generation pipeline (doc 10 -- Autonomous Eval Generation).
//!
//! Before an implementation agent starts, this module generates targeted test
//! cases from task specs. Three strategies are supported:
//!
//! - **Example-based**: concrete input/output pairs
//! - **Property-based**: invariants (proptest-style)
//! - **Mutation-based**: mutant detection
//!
//! Generated evaluations are validated against the current codebase and
//! registered with the `GeneratedTestGate` artifact store.

use serde::{Deserialize, Serialize};

/// Strategy for generating evaluation test cases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EvalStrategy {
    /// Concrete input/output pairs derived from the task spec.
    ExampleBased,
    /// Invariant assertions (proptest-style properties).
    PropertyBased,
    /// Mutation-based: ensure the implementation detects seeded faults.
    MutationBased,
}

/// A single evaluation template that can generate test cases for a gate type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalTemplate {
    /// Human-readable template name (e.g. "compile-gate-basic").
    pub name: String,
    /// Gate type this template targets (e.g. "compile", "test", "clippy").
    pub gate_type: String,
    /// Strategy used for test generation.
    pub strategy: EvalStrategy,
    /// Description of expected behavior to validate.
    pub expected_behavior: String,
    /// Template body with placeholders for task-specific values.
    /// Placeholders: `{task_title}`, `{crate_name}`, `{files}`.
    pub template_body: String,
}

/// A generated evaluation case ready for registration with the artifact store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evaluation {
    /// Evaluation name derived from template + task.
    pub name: String,
    /// Gate type being evaluated.
    pub gate_type: String,
    /// Strategy used.
    pub strategy: EvalStrategy,
    /// Generated test source code.
    pub test_source: String,
    /// Whether this test is expected to fail before implementation (new feature test).
    pub expect_pre_failure: bool,
}

/// Generator that produces evaluation cases from task descriptions.
#[derive(Clone, Debug)]
pub struct EvalGenerator {
    /// Available templates for generating evaluations.
    pub templates: Vec<EvalTemplate>,
}

impl Default for EvalGenerator {
    fn default() -> Self {
        Self {
            templates: builtin_templates(),
        }
    }
}

impl EvalGenerator {
    /// Create a generator with the builtin template set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a generator with custom templates.
    #[must_use]
    pub fn with_templates(templates: Vec<EvalTemplate>) -> Self {
        Self { templates }
    }

    /// Generate evaluations for a task targeting a specific gate type.
    ///
    /// Returns all evaluations matching the gate type, with placeholders
    /// filled from `task_title`, `crate_name`, and `files`.
    #[must_use]
    pub fn generate(
        &self,
        task_title: &str,
        gate_type: &str,
        crate_name: &str,
        files: &[String],
    ) -> Vec<Evaluation> {
        self.templates
            .iter()
            .filter(|template| template.gate_type == gate_type)
            .map(|template| {
                let files_str = files.join(", ");
                let test_source = template
                    .template_body
                    .replace("{task_title}", task_title)
                    .replace("{crate_name}", crate_name)
                    .replace("{files}", &files_str);

                Evaluation {
                    name: format!("gen_{}_{}", template.name.replace('-', "_"), sanitize(task_title)),
                    gate_type: template.gate_type.clone(),
                    strategy: template.strategy.clone(),
                    test_source,
                    expect_pre_failure: true,
                }
            })
            .collect()
    }

    /// Generate evaluations for all gate types relevant to a task.
    #[must_use]
    pub fn generate_all(
        &self,
        task_title: &str,
        crate_name: &str,
        files: &[String],
    ) -> Vec<Evaluation> {
        let gate_types: Vec<String> = self
            .templates
            .iter()
            .map(|t| t.gate_type.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        gate_types
            .iter()
            .flat_map(|gate_type| self.generate(task_title, gate_type, crate_name, files))
            .collect()
    }
}

/// Sanitize a task title into a valid Rust identifier fragment.
fn sanitize(title: &str) -> String {
    title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Built-in evaluation templates for the standard gate types.
fn builtin_templates() -> Vec<EvalTemplate> {
    vec![
        EvalTemplate {
            name: "compile-check".into(),
            gate_type: "compile".into(),
            strategy: EvalStrategy::ExampleBased,
            expected_behavior: "Code compiles with no errors after implementation".into(),
            template_body: concat!(
                "// Generated eval: {task_title}\n",
                "// Verify: `cargo check -p {crate_name}` succeeds\n",
                "// Files: {files}\n",
                "#[test]\n",
                "fn gen_compiles() {\n",
                "    // This test validates that the crate compiles.\n",
                "    // The compile gate itself handles verification;\n",
                "    // this is a placeholder for the artifact store.\n",
                "}\n",
            )
            .into(),
        },
        EvalTemplate {
            name: "clippy-clean".into(),
            gate_type: "clippy".into(),
            strategy: EvalStrategy::ExampleBased,
            expected_behavior: "No new clippy warnings introduced".into(),
            template_body: concat!(
                "// Generated eval: {task_title}\n",
                "// Verify: `cargo clippy -p {crate_name}` produces no warnings\n",
                "// Files: {files}\n",
                "#[test]\n",
                "fn gen_clippy_clean() {\n",
                "    // Clippy cleanliness verified by ClippyGate.\n",
                "}\n",
            )
            .into(),
        },
        EvalTemplate {
            name: "test-pass".into(),
            gate_type: "test".into(),
            strategy: EvalStrategy::ExampleBased,
            expected_behavior: "All existing tests continue to pass".into(),
            template_body: concat!(
                "// Generated eval: {task_title}\n",
                "// Verify: `cargo test -p {crate_name}` passes\n",
                "// Files: {files}\n",
                "#[test]\n",
                "fn gen_tests_pass() {\n",
                "    // Test suite integrity verified by TestGate.\n",
                "}\n",
            )
            .into(),
        },
        EvalTemplate {
            name: "property-invariant".into(),
            gate_type: "test".into(),
            strategy: EvalStrategy::PropertyBased,
            expected_behavior: "Implementation satisfies domain invariants".into(),
            template_body: concat!(
                "// Generated property eval: {task_title}\n",
                "// Crate: {crate_name}, Files: {files}\n",
                "// Strategy: property-based (verify invariants hold)\n",
                "#[test]\n",
                "fn gen_property_invariant() {\n",
                "    // TODO: LLM-generated property assertions go here.\n",
                "    // For now, this is a template placeholder.\n",
                "}\n",
            )
            .into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_generator_has_builtin_templates() {
        let generator = EvalGenerator::new();
        assert!(!generator.templates.is_empty());
    }

    #[test]
    fn generate_for_compile_gate() {
        let generator = EvalGenerator::new();
        let evals = generator.generate("Add Demurrage trait", "compile", "roko-core", &[]);
        assert_eq!(evals.len(), 1);
        assert!(evals[0].name.starts_with("gen_compile_check"));
        assert!(evals[0].test_source.contains("Demurrage"));
        assert!(evals[0].expect_pre_failure);
    }

    #[test]
    fn generate_all_covers_multiple_gate_types() {
        let generator = EvalGenerator::new();
        let evals =
            generator.generate_all("Wire foraging", "roko-compose", &["foraging.rs".into()]);
        assert!(evals.len() >= 3);
        let gate_types: Vec<&str> = evals.iter().map(|e| e.gate_type.as_str()).collect();
        assert!(gate_types.contains(&"compile"));
        assert!(gate_types.contains(&"test"));
    }

    #[test]
    fn custom_templates() {
        let generator = EvalGenerator::with_templates(vec![EvalTemplate {
            name: "custom".into(),
            gate_type: "security".into(),
            strategy: EvalStrategy::MutationBased,
            expected_behavior: "No SQL injection".into(),
            template_body: "// {task_title} in {crate_name}".into(),
        }]);
        let evals = generator.generate("Sanitize input", "security", "my-crate", &[]);
        assert_eq!(evals.len(), 1);
        assert!(evals[0].test_source.contains("Sanitize input"));
    }

    #[test]
    fn sanitize_title() {
        assert_eq!(sanitize("Add Demurrage trait"), "add_demurrage_trait");
        assert_eq!(sanitize("fix: bug #123"), "fix__bug__123");
    }
}
