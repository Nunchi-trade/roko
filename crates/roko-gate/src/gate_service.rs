//! GateService -- concrete implementation of `GateRunner`.
//!
//! Wraps existing gate implementations (`CompileGate`, `TestGate`, `ClippyGate`)
//! and runs them according to `GateConfig`.

use async_trait::async_trait;
use roko_core::foundation::{GateConfig, GateReport, GateRunner, GateVerdict, ShellGateCommand};
use roko_core::{Body, Context, Engram, Kind, Result, RokoError, Verdict, Verify};
use std::sync::{Arc, Mutex};

use crate::adaptive_threshold::AdaptiveThresholds;
use crate::clippy_gate::ClippyGate;
use crate::compile::CompileGate;
use crate::payload::{BuildSystem, GatePayload};
use crate::shell::ShellGate;
use crate::test_gate::TestGate;

/// Service that runs verification gates via the existing gate infrastructure.
///
/// This is the canonical way to run gates in the workflow engine. It:
/// - Selects gates from `GateConfig`
/// - Runs supported gates in rung order: compile, clippy, test, diff, fmt, shell, judge
/// - Records skipped gates without counting them as passes
/// - Stops at the first real failing gate
/// - Returns a unified `GateReport`
pub struct GateService {
    adaptive: Option<Arc<Mutex<AdaptiveThresholds>>>,
}

impl GateService {
    /// Construct a `GateService`.
    #[must_use]
    pub const fn new() -> Self {
        Self { adaptive: None }
    }

    /// Attach adaptive thresholds for gate skip/observe decisions.
    ///
    /// When thresholds are attached, the service will:
    /// - Skip gates whose rung has a long consecutive-pass streak
    /// - Record pass/fail outcomes back to the thresholds after each gate
    ///
    /// Rung 0 (compile) is never skipped regardless of thresholds.
    #[must_use]
    pub fn with_adaptive_thresholds(mut self, thresholds: AdaptiveThresholds) -> Self {
        self.adaptive = Some(Arc::new(Mutex::new(thresholds)));
        self
    }

    /// Map a gate name to its rung index.
    fn rung_for_name(name: &str) -> Option<u8> {
        match name {
            "compile" | "compile:cargo" => Some(0),
            "clippy" | "clippy:cargo" => Some(1),
            "test" | "test:cargo" => Some(2),
            "diff" | "diff:git" => Some(3),
            "fmt" | "fmt:cargo" | "format" => Some(4),
            "custom" | "custom:shell" | "shell" => Some(5),
            "judge" | "llm-judge" => Some(6),
            _ => None,
        }
    }

    /// Map a gate name to a concrete gate implementation.
    #[allow(clippy::unused_self)]
    fn gate_for_name(&self, name: &str, build_system: BuildSystem) -> Option<Box<dyn Verify>> {
        match name {
            "compile" | "compile:cargo" => Some(Box::new(CompileGate::new(build_system))),
            "clippy" | "clippy:cargo" => Some(Box::new(ClippyGate::new(build_system))),
            "test" | "test:cargo" => Some(Box::new(TestGate::new(build_system))),
            "diff" | "diff:git" => Some(Box::new(
                ShellGate::new("git", vec!["diff".into(), "--stat".into()]).with_timeout_ms(30_000),
            )),
            "fmt" | "fmt:cargo" | "format" => Some(Box::new(FormatCheckGate::cargo())),
            // "shell" / "custom:shell" / "custom" are intentionally absent here.
            // They require a `ShellGateCommand` from `GateConfig.shell_gates` to know which
            // program to run.  Returning a stub that runs `true` would silently pass when the
            // caller intended a real check.  Shell gates must go through `run_gates()`, which
            // pops the next `ShellGateCommand` from config or records a skipped verdict.
            "judge" | "llm-judge" => Some(Box::new(StubJudgeGate)),
            _ => None,
        }
    }

    fn shell_gate_for_config(command: &ShellGateCommand) -> Result<Box<dyn Verify>> {
        if command.program.trim().is_empty() {
            return Err(RokoError::Invalid(
                "shell gate requires a non-empty program".to_string(),
            ));
        }

        Ok(Box::new(
            ShellGate::new(command.program.clone(), command.args.clone())
                .with_timeout_ms(command.timeout_ms),
        ))
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

    fn should_skip_rung_adaptively(&self, rung: Option<u8>) -> Result<bool> {
        let Some(r) = rung else {
            return Ok(false);
        };
        let Some(adaptive) = &self.adaptive else {
            return Ok(false);
        };

        if r > 0 {
            let thresholds = adaptive
                .lock()
                .map_err(|e| RokoError::Invalid(format!("threshold lock: {e}")))?;
            return Ok(thresholds.should_skip_rung(u32::from(r)));
        }

        Ok(false)
    }
}

fn skipped_gate_verdict(
    gate_name: String,
    output: impl Into<String>,
    skip_reason: impl Into<String>,
) -> GateVerdict {
    GateVerdict {
        gate_name,
        passed: false,
        skipped: true,
        skip_reason: Some(skip_reason.into()),
        output: output.into(),
        duration_ms: 0,
    }
}

/// Format-check gate adapter for the GateService rung pipeline.
struct FormatCheckGate {
    inner: ShellGate,
}

impl FormatCheckGate {
    fn cargo() -> Self {
        Self {
            inner: ShellGate::new("cargo", vec!["fmt".into(), "--check".into()])
                .with_name("format_check:cargo"),
        }
    }
}

impl roko_core::Cell for FormatCheckGate {
    fn cell_id(&self) -> &str {
        "format-check-gate"
    }

    fn cell_name(&self) -> &str {
        "FormatCheckGate"
    }

    fn protocols(&self) -> &[&str] {
        &["Verify"]
    }
}

#[async_trait]
impl Verify for FormatCheckGate {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Verdict {
        self.inner.verify(signal, ctx).await
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

/// Stub LLM judge gate that fails with a clear message if called directly.
///
/// The `GateService` runner intercepts `judge`/`llm-judge` and records a skipped
/// verdict instead of executing this placeholder.
struct StubJudgeGate;

impl roko_core::Cell for StubJudgeGate {
    fn cell_id(&self) -> &str {
        "stub-llm-judge"
    }

    fn cell_name(&self) -> &str {
        "StubJudgeGate"
    }

    fn protocols(&self) -> &[&str] {
        &["Verify"]
    }
}

#[async_trait]
impl Verify for StubJudgeGate {
    async fn verify(&self, _signal: &Engram, _ctx: &Context) -> Verdict {
        Verdict::fail(
            "stub-llm-judge",
            "LLM judge gate not yet implemented — enable a real judge or remove from enabled_gates",
        )
    }

    fn name(&self) -> &str {
        "stub-llm-judge"
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
        let mut shell_gates = config.shell_gates.iter();

        for gate_name in Self::ordered_gate_names(&config) {
            let rung = Self::rung_for_name(&gate_name);

            if matches!(gate_name.as_str(), "judge" | "llm-judge") {
                verdicts.push(skipped_gate_verdict(
                    gate_name,
                    "Skipped: LLM judge gate not yet implemented — enable a real judge or remove from enabled_gates",
                    "not implemented",
                ));
                continue;
            }

            let gate = match gate_name.as_str() {
                "custom" => {
                    let Some(command) = shell_gates.next() else {
                        verdicts.push(skipped_gate_verdict(
                            gate_name.clone(),
                            "Skipped: custom gate requires explicit command configuration",
                            "not wired",
                        ));
                        continue;
                    };

                    if command.program.trim().is_empty() {
                        verdicts.push(skipped_gate_verdict(
                            gate_name.clone(),
                            "Skipped: custom gate requires a non-empty program",
                            "missing program",
                        ));
                        continue;
                    }

                    match Self::shell_gate_for_config(command) {
                        Ok(gate) => gate,
                        Err(err) => {
                            verdicts.push(skipped_gate_verdict(
                                gate_name.clone(),
                                format!("Skipped: {err}"),
                                "missing program",
                            ));
                            continue;
                        }
                    }
                }
                "shell" | "custom:shell" => {
                    let Some(command) = shell_gates.next() else {
                        verdicts.push(skipped_gate_verdict(
                            gate_name.clone(),
                            format!(
                                "Skipped: {gate_name} gate requires explicit command configuration"
                            ),
                            "not wired",
                        ));
                        continue;
                    };

                    if command.program.trim().is_empty() {
                        verdicts.push(skipped_gate_verdict(
                            gate_name.clone(),
                            format!("Skipped: {gate_name} gate requires a non-empty program"),
                            "missing program",
                        ));
                        continue;
                    }

                    match Self::shell_gate_for_config(command) {
                        Ok(gate) => gate,
                        Err(err) => {
                            verdicts.push(skipped_gate_verdict(
                                gate_name.clone(),
                                format!("Skipped: {err}"),
                                "missing program",
                            ));
                            continue;
                        }
                    }
                }
                _ => {
                    let Some(gate) = self.gate_for_name(&gate_name, build_system) else {
                        verdicts.push(skipped_gate_verdict(
                            gate_name.clone(),
                            format!("Skipped: Unknown gate: {gate_name}"),
                            "not wired",
                        ));
                        continue;
                    };
                    gate
                }
            };

            if let Some(r) = rung
                && self.should_skip_rung_adaptively(Some(r))?
            {
                verdicts.push(skipped_gate_verdict(
                    gate_name.clone(),
                    format!("Skipped (adaptive: high pass rate for rung {r})"),
                    format!("adaptive: high pass rate for rung {r}"),
                ));
                continue;
            }

            let verdict = gate.verify(&signal, &ctx).await;
            let passed = verdict.passed;
            let gate_verdict = to_gate_verdict(gate_name, verdict);
            let was_skipped = gate_verdict.skipped;
            verdicts.push(gate_verdict);

            if let (Some(r), Some(adaptive)) = (rung, &self.adaptive)
                && !was_skipped
                && let Ok(mut thresholds) = adaptive.lock()
            {
                thresholds.observe(u32::from(r), passed);
            }

            if !passed && !was_skipped {
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
        skipped: false,
        skip_reason: None,
        output,
        duration_ms: verdict.duration_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rung_mapping_covers_all_seven_rungs() {
        assert_eq!(GateService::rung_for_name("compile"), Some(0));
        assert_eq!(GateService::rung_for_name("clippy"), Some(1));
        assert_eq!(GateService::rung_for_name("test"), Some(2));
        assert_eq!(GateService::rung_for_name("diff"), Some(3));
        assert_eq!(GateService::rung_for_name("fmt"), Some(4));
        assert_eq!(GateService::rung_for_name("custom"), Some(5));
        assert_eq!(GateService::rung_for_name("shell"), Some(5));
        assert_eq!(GateService::rung_for_name("custom:shell"), Some(5));
        assert_eq!(GateService::rung_for_name("judge"), Some(6));
        assert_eq!(GateService::rung_for_name("nonexistent"), None);
    }

    #[test]
    fn gate_for_name_returns_supported_gates() {
        let svc = GateService::new();
        let bs = BuildSystem::Cargo;
        assert!(svc.gate_for_name("compile", bs).is_some());
        assert!(svc.gate_for_name("clippy", bs).is_some());
        assert!(svc.gate_for_name("test", bs).is_some());
        assert!(svc.gate_for_name("diff", bs).is_some());
        assert!(svc.gate_for_name("fmt", bs).is_some());
        // "shell", "custom:shell", and "custom" require a ShellGateCommand from config.
        // gate_for_name() returns None for them — callers must use run_gates() instead.
        assert!(
            svc.gate_for_name("shell", bs).is_none(),
            "shell gate must not return a stub; use run_gates() with a ShellGateCommand"
        );
        assert!(
            svc.gate_for_name("custom:shell", bs).is_none(),
            "custom:shell gate must not return a stub; use run_gates() with a ShellGateCommand"
        );
        assert!(
            svc.gate_for_name("custom", bs).is_none(),
            "custom gate must not return a stub; use run_gates() with a ShellGateCommand"
        );
        assert!(svc.gate_for_name("judge", bs).is_some());
        assert!(svc.gate_for_name("nonexistent", bs).is_none());
    }

    #[tokio::test]
    async fn shell_gate_for_config_runs_true_false_and_captures_stderr() {
        let signal = Engram::builder(Kind::Task).body(Body::empty()).build();
        let ctx = Context::at(0);

        let true_gate = GateService::shell_gate_for_config(&ShellGateCommand {
            program: "true".into(),
            args: vec![],
            timeout_ms: 1_000,
        })
        .expect("true shell gate config should be valid");
        assert!(true_gate.verify(&signal, &ctx).await.passed);

        let false_gate = GateService::shell_gate_for_config(&ShellGateCommand {
            program: "false".into(),
            args: vec![],
            timeout_ms: 1_000,
        })
        .expect("false shell gate config should be valid");
        let false_verdict = false_gate.verify(&signal, &ctx).await;
        assert!(!false_verdict.passed);
        assert!(false_verdict.reason.contains("exit code"));

        let stderr_gate = GateService::shell_gate_for_config(&ShellGateCommand {
            program: "sh".into(),
            args: vec!["-c".into(), "echo shell stderr >&2; exit 1".into()],
            timeout_ms: 1_000,
        })
        .expect("stderr shell gate config should be valid");
        let stderr_verdict = stderr_gate.verify(&signal, &ctx).await;
        assert!(!stderr_verdict.passed);
        let detail = stderr_verdict.detail.as_deref().unwrap_or("");
        assert!(detail.contains("---stderr---"));
        assert!(detail.contains("shell stderr"));
    }

    #[test]
    fn orders_all_seven_rungs_correctly() {
        let config = GateConfig {
            workdir: ".".into(),
            enabled_gates: vec![
                "judge".into(),
                "fmt".into(),
                "test".into(),
                "diff".into(),
                "compile".into(),
                "custom".into(),
                "clippy".into(),
            ],
            shell_gates: Vec::new(),
            max_rung: None,
        };
        assert_eq!(
            GateService::ordered_gate_names(&config),
            vec![
                "compile", "clippy", "test", "diff", "fmt", "custom", "judge"
            ]
        );
    }

    #[test]
    fn max_rung_filters_higher_rungs() {
        let config = GateConfig {
            workdir: ".".into(),
            enabled_gates: vec![
                "compile".into(),
                "clippy".into(),
                "test".into(),
                "diff".into(),
                "fmt".into(),
                "custom".into(),
                "judge".into(),
            ],
            shell_gates: Vec::new(),
            max_rung: Some(3),
        };
        assert_eq!(
            GateService::ordered_gate_names(&config),
            vec!["compile", "clippy", "test", "diff"]
        );
    }

    #[test]
    fn unknown_gate_produces_failure_mapping() {
        let svc = GateService::new();
        assert!(
            svc.gate_for_name("nonexistent", BuildSystem::Cargo)
                .is_none()
        );
        assert!(svc.gate_for_name("compile", BuildSystem::Cargo).is_some());
        assert!(svc.gate_for_name("custom", BuildSystem::Cargo).is_none());
    }

    #[test]
    fn skipped_gate_verdict_marks_skip_metadata() {
        let verdict = skipped_gate_verdict(
            "custom".to_string(),
            "Skipped: custom gate requires explicit command configuration",
            "not wired",
        );

        assert!(!verdict.passed);
        assert!(verdict.skipped);
        assert_eq!(verdict.skip_reason.as_deref(), Some("not wired"));
        assert!(verdict.output.starts_with("Skipped:"));
    }

    #[test]
    fn real_gate_verdicts_default_to_not_skipped() {
        let verdict = Verdict::pass("compile").with_detail("ok").with_duration(7);
        let gate_verdict = to_gate_verdict("compile".to_string(), verdict);

        assert!(gate_verdict.passed);
        assert!(!gate_verdict.skipped);
        assert_eq!(gate_verdict.skip_reason, None);
        assert_eq!(gate_verdict.output, "ok");
        assert_eq!(gate_verdict.duration_ms, 7);
    }

    #[test]
    fn orders_supported_gates_by_rung() {
        let config = GateConfig {
            workdir: ".".into(),
            enabled_gates: vec!["test".into(), "compile".into(), "clippy".into()],
            shell_gates: Vec::new(),
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
            shell_gates: Vec::new(),
            max_rung: Some(1),
        };

        assert_eq!(
            GateService::ordered_gate_names(&config),
            vec!["compile".to_string(), "clippy".to_string()]
        );
    }

    #[test]
    fn adaptive_skips_high_pass_rate_rung() {
        let mut thresholds = AdaptiveThresholds::new();
        for _ in 0..25 {
            thresholds.observe(1, true);
        }

        assert!(thresholds.should_skip_rung(1));

        let svc = GateService::new().with_adaptive_thresholds(thresholds);
        let config = GateConfig {
            workdir: ".".into(),
            enabled_gates: vec!["compile".into(), "clippy".into()],
            shell_gates: Vec::new(),
            max_rung: None,
        };

        assert!(svc.adaptive.is_some());
        assert_eq!(
            GateService::ordered_gate_names(&config),
            vec!["compile".to_string(), "clippy".to_string()]
        );
        assert!(
            svc.should_skip_rung_adaptively(Some(1))
                .expect("adaptive lock should not be poisoned")
        );
    }

    #[test]
    fn adaptive_never_skips_compile() {
        let mut thresholds = AdaptiveThresholds::new();
        for _ in 0..30 {
            thresholds.observe(0, true);
        }

        let compile_would_be_skipped_by_threshold = thresholds.should_skip_rung(0);
        let svc = GateService::new().with_adaptive_thresholds(thresholds);

        assert!(compile_would_be_skipped_by_threshold);
        assert!(
            !svc.should_skip_rung_adaptively(Some(0))
                .expect("adaptive lock should not be poisoned")
        );
    }

    #[test]
    fn adaptive_records_outcomes() {
        let svc = GateService::new().with_adaptive_thresholds(AdaptiveThresholds::new());
        let adaptive = svc
            .adaptive
            .as_ref()
            .expect("adaptive thresholds should be attached");

        {
            let mut thresholds = adaptive
                .lock()
                .expect("adaptive lock should not be poisoned");
            thresholds.observe(0, true);
        }

        let thresholds = adaptive
            .lock()
            .expect("adaptive lock should not be poisoned");
        let stats = thresholds
            .rung_stats(0)
            .expect("rung 0 should have one observation");
        assert_eq!(stats.total_observations, 1);
    }

    #[test]
    fn no_adaptive_runs_all_gates() {
        let svc = GateService::new();
        let config = GateConfig {
            workdir: ".".into(),
            enabled_gates: vec![
                "compile".into(),
                "clippy".into(),
                "test".into(),
                "diff".into(),
                "fmt".into(),
                "custom".into(),
                "judge".into(),
            ],
            shell_gates: Vec::new(),
            max_rung: None,
        };

        assert!(svc.adaptive.is_none());
        assert_eq!(
            GateService::ordered_gate_names(&config),
            vec![
                "compile", "clippy", "test", "diff", "fmt", "custom", "judge"
            ]
        );
    }
}
