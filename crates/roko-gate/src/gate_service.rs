//! GateService -- concrete implementation of `GateRunner`.
//!
//! Wraps existing gate implementations (`CompileGate`, `TestGate`, `ClippyGate`)
//! and runs them according to `GateConfig`.

use async_trait::async_trait;
use roko_core::foundation::{GateConfig, GateReport, GateRunner, GateVerdict};
use roko_core::{Body, Context, Engram, Kind, Result, Verdict, Verify};

use crate::clippy_gate::ClippyGate;
use crate::compile::CompileGate;
use crate::payload::{BuildSystem, GatePayload};
use crate::test_gate::TestGate;

/// Service that runs verification gates via the existing gate infrastructure.
///
/// This is the canonical way to run gates in the workflow engine. It:
/// - Selects gates from `GateConfig`
/// - Runs supported gates in rung order: compile, clippy, test
/// - Stops at the first failing gate
/// - Returns a unified `GateReport`
pub struct GateService;

impl GateService {
    /// Construct a `GateService`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Map a gate name to its rung index.
    fn rung_for_name(name: &str) -> Option<u8> {
        match name {
            "compile" | "compile:cargo" => Some(0),
            "clippy" | "clippy:cargo" => Some(1),
            "test" | "test:cargo" => Some(2),
            _ => None,
        }
    }

    /// Map a gate name to a concrete gate implementation.
    fn gate_for_name(&self, name: &str, build_system: BuildSystem) -> Option<Box<dyn Verify>> {
        match name {
            "compile" | "compile:cargo" => Some(Box::new(CompileGate::new(build_system))),
            "clippy" | "clippy:cargo" => Some(Box::new(ClippyGate::new(build_system))),
            "test" | "test:cargo" => Some(Box::new(TestGate::new(build_system))),
            _ => None,
        }
    }

    fn ordered_gate_names(config: &GateConfig) -> Vec<String> {
        let mut indexed_names = config
            .enabled_gates
            .iter()
            .enumerate()
            .filter(|(_, name)| {
                Self::rung_for_name(name)
                    .is_none_or(|rung| config.max_rung.is_none_or(|max| rung <= max))
            })
            .collect::<Vec<_>>();

        indexed_names.sort_by_key(|(index, name)| {
            let rung = Self::rung_for_name(name).unwrap_or(u8::MAX);
            (rung, *index)
        });

        indexed_names
            .into_iter()
            .map(|(_, name)| name.clone())
            .collect()
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
        let payload = GatePayload::in_dir(config.workdir.clone());
        let signal = Engram::builder(Kind::Task)
            .body(Body::from_json(&payload)?)
            .build();
        let ctx = Context::now().with_attr("workdir", config.workdir.to_string_lossy());
        let build_system = BuildSystem::detect(&config.workdir);

        let mut verdicts = Vec::new();

        for gate_name in Self::ordered_gate_names(&config) {
            let Some(gate) = self.gate_for_name(&gate_name, build_system) else {
                verdicts.push(GateVerdict {
                    gate_name: gate_name.clone(),
                    passed: false,
                    output: format!("Unknown gate: {gate_name}"),
                    duration_ms: 0,
                });
                break;
            };

            let verdict = gate.verify(&signal, &ctx).await;
            let passed = verdict.passed;
            verdicts.push(to_gate_verdict(gate_name, verdict));

            if !passed {
                break;
            }
        }

        Ok(GateReport { verdicts })
    }
}

fn to_gate_verdict(gate_name: String, verdict: Verdict) -> GateVerdict {
    let output = verdict
        .error_digest
        .filter(|output| !output.is_empty())
        .or_else(|| verdict.detail.filter(|output| !output.is_empty()))
        .unwrap_or(verdict.reason);

    GateVerdict {
        gate_name,
        passed: verdict.passed,
        output,
        duration_ms: verdict.duration_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_gate_produces_failure_mapping() {
        let svc = GateService::new();
        assert!(svc.gate_for_name("nonexistent", BuildSystem::Cargo).is_none());
        assert!(svc.gate_for_name("compile", BuildSystem::Cargo).is_some());
    }

    #[test]
    fn orders_supported_gates_by_rung() {
        let config = GateConfig {
            workdir: ".".into(),
            enabled_gates: vec!["test".into(), "compile".into(), "clippy".into()],
            max_rung: None,
        };

        assert_eq!(
            GateService::ordered_gate_names(&config),
            vec![
                "compile".to_string(),
                "clippy".to_string(),
                "test".to_string()
            ]
        );
    }

    #[test]
    fn max_rung_filters_supported_gates() {
        let config = GateConfig {
            workdir: ".".into(),
            enabled_gates: vec!["compile".into(), "clippy".into(), "test".into()],
            max_rung: Some(1),
        };

        assert_eq!(
            GateService::ordered_gate_names(&config),
            vec!["compile".to_string(), "clippy".to_string()]
        );
    }
}
