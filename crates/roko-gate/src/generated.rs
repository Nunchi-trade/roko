//! Types for gates generated from acceptance criteria rather than hand-written.
//!
//! These checks are consumed by gate orchestration only. The implementer agent
//! never sees the generated symbols or test bodies.

/// Gate-generation errors reuse the crate's canonical core error type.
pub type GateError = roko_core::RokoError;

/// Produces verifier artifacts from acceptance criteria and task context.
pub trait GateGenerator: Send + Sync {
    /// Generate verification artifacts from acceptance criteria.
    fn generate(
        &self,
        acceptance_criteria: &str,
        task_context: &str,
    ) -> Result<Vec<GeneratedCheck>, GateError>;
}

/// A gate artifact synthesized from plan acceptance criteria.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedCheck {
    /// Assert that a symbol exists with the expected kind, visibility, and module.
    SymbolExists {
        /// Symbol identifier, e.g. `RateLimiter`.
        name: String,
        /// Rust item kind, e.g. `struct`, `fn`, `trait`, `enum`.
        kind: String,
        /// Expected visibility, e.g. `pub`, `pub(crate)`, or empty for private.
        visibility: String,
        /// Canonical Rust module path where the symbol should live.
        module_path: String,
    },
    /// A complete generated test case to run at a specific verification rung.
    TestCase {
        /// Stable test function name.
        name: String,
        /// Complete test source, including `#[test]`.
        code: String,
        /// Verification rung for this test, e.g. 3 for behavioral, 4 for property.
        rung: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticGenerator;

    impl GateGenerator for StaticGenerator {
        fn generate(
            &self,
            acceptance_criteria: &str,
            task_context: &str,
        ) -> Result<Vec<GeneratedCheck>, GateError> {
            let _ = (acceptance_criteria, task_context);
            Ok(vec![
                GeneratedCheck::SymbolExists {
                    name: "RateLimiter".into(),
                    kind: "struct".into(),
                    visibility: "pub".into(),
                    module_path: "roko_gate::rate_limit".into(),
                },
                GeneratedCheck::TestCase {
                    name: "gen_rate_limiter_allows_first_call".into(),
                    code: "#[test]\nfn gen_rate_limiter_allows_first_call() {}".into(),
                    rung: 3,
                },
            ])
        }
    }

    #[test]
    fn generated_check_types() {
        let generator: &dyn GateGenerator = &StaticGenerator;
        let checks = generator.generate("must expose a rate limiter", "task 2K.31").unwrap();

        assert_eq!(checks.len(), 2);

        match &checks[0] {
            GeneratedCheck::SymbolExists {
                name,
                kind,
                visibility,
                module_path,
            } => {
                assert_eq!(name, "RateLimiter");
                assert_eq!(kind, "struct");
                assert_eq!(visibility, "pub");
                assert_eq!(module_path, "roko_gate::rate_limit");
            }
            GeneratedCheck::TestCase { .. } => panic!("expected symbol check"),
        }

        match &checks[1] {
            GeneratedCheck::TestCase { name, code, rung } => {
                assert_eq!(name, "gen_rate_limiter_allows_first_call");
                assert!(code.starts_with("#[test]"));
                assert_eq!(*rung, 3);
            }
            GeneratedCheck::SymbolExists { .. } => panic!("expected test case"),
        }
    }
}
