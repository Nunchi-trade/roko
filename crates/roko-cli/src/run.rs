//! The universal loop: prompt → compose → agent → gate → persist → policy.
//!
//! This is the body of `roko run <prompt>`. It reads [`Config`], opens a
//! [`FileSubstrate`] under `.roko/`, seeds prompt sections, composes them
//! into a single Prompt signal, invokes the configured `ExecAgent`, runs
//! each configured gate on the working directory, and emits an Episode.

use crate::clean;
use crate::config::{Config, GateConfig, PromptFile};
use crate::episode::EpisodePolicy;
use anyhow::{anyhow, Context as _, Result};
use roko_agent::{Agent, AgentResult, ExecAgent};
use roko_compose::{Placement, PromptComposer, PromptSection, SectionPriority};
use roko_core::{
    Body, Budget, Composer, Context, Gate, Kind, Provenance, Signal, Substrate, Verdict,
};
use roko_fs::FileSubstrate;
use roko_gate::{BuildSystem, ClippyGate, CompileGate, GatePayload, ShellGate, TestGate};
use roko_learn::episode_logger::{Episode, EpisodeLogger, GateVerdict as EpGateVerdict, Usage};
use roko_std::NoOpScorer;
use std::path::{Path, PathBuf};

/// Summary of a single `run` invocation.
#[derive(Debug, Clone)]
pub struct RunReport {
    /// Content hash of the episode signal emitted at the end.
    pub episode_id: String,
    /// Content hash of the assembled prompt signal.
    pub prompt_id: String,
    /// Content hash of the agent's output signal.
    pub agent_output_id: String,
    /// Whether the agent invocation succeeded (exit code 0, no timeout).
    pub agent_success: bool,
    /// Per-gate verdicts in declaration order: (gate name, passed).
    pub gate_verdicts: Vec<(String, bool)>,
    /// How many signals are now in the substrate.
    pub total_signals: usize,
}

impl RunReport {
    /// True if the agent succeeded and every configured gate passed.
    #[must_use]
    pub fn overall_success(&self) -> bool {
        self.agent_success && self.gate_verdicts.iter().all(|(_, ok)| *ok)
    }
}

/// Run the universal loop once for `prompt_text` under `workdir`.
///
/// - Opens (or creates) `workdir/.roko/signals.jsonl`.
/// - Seeds a role + task `PromptSection`, composes them under the config's budget.
/// - Invokes the configured `ExecAgent`.
/// - Runs every gate in the config in declaration order; each gate sees the
///   same `GatePayload` pointing at `workdir`.
/// - Records an Episode signal and persists everything.
#[allow(clippy::too_many_lines)]
pub async fn run_once(
    workdir: &Path,
    config: &Config,
    prompt_text: &str,
) -> Result<RunReport> {
    let substrate_dir = workdir.join(".roko");
    let substrate = FileSubstrate::open(substrate_dir)
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;

    let ctx = Context::now();

    // Seed prompt sections: system role + user prompt + any injected files.
    let mut sections: Vec<Signal> = Vec::with_capacity(2 + config.prompt.files.len());

    let role_sig = PromptSection::new("role", &config.prompt.role)
        .with_priority(SectionPriority::Critical)
        .with_placement(Placement::Start)
        .into_signal()
        .map_err(|e| anyhow!("build role section: {e}"))?;
    sections.push(role_sig);

    // File-injected sections: one per `[[prompt.files]]` entry in roko.toml.
    for file in &config.prompt.files {
        let section = load_file_section(workdir, file)?;
        sections.push(section);
    }

    let task_sig = PromptSection::new("task", prompt_text)
        .with_priority(SectionPriority::Critical)
        .with_placement(Placement::End)
        .into_signal()
        .map_err(|e| anyhow!("build task section: {e}"))?;
    sections.push(task_sig);

    for sig in &sections {
        substrate
            .put(sig.clone())
            .await
            .map_err(|e| anyhow!("persist prompt section: {e}"))?;
    }

    // Compose the prompt under the configured budget.
    let composer = PromptComposer::new();
    let prompt = composer
        .compose(
            &sections,
            &Budget::tokens(config.prompt.token_budget),
            &NoOpScorer,
            &ctx,
        )
        .map_err(|e| anyhow!("compose prompt: {e}"))?;
    substrate
        .put(prompt.clone())
        .await
        .map_err(|e| anyhow!("persist prompt: {e}"))?;

    // Run the agent (ExecAgent wraps any stdin/stdout CLI).
    let mut agent = ExecAgent::new(&config.agent.command, config.agent.args.clone())
        .with_timeout_ms(config.agent.timeout_ms);
    for (k, v) in &config.agent.env {
        agent = agent.with_env_var(k, v);
    }
    let agent_result: AgentResult = agent.run(&prompt, &ctx).await;

    // Optionally post-process the agent output to strip ANSI escapes and
    // reasoning-model thinking traces. The raw body is preserved as an
    // AgentMessage trace so nothing is lost.
    let final_output_sig = if config.agent.clean_output {
        maybe_clean_output(&prompt, &agent_result, &substrate).await?
    } else {
        agent_result.output.clone()
    };
    if config.agent.clean_output {
        // clean path already wrote both signals; skip the normal write
    } else {
        substrate
            .put(agent_result.output.clone())
            .await
            .map_err(|e| anyhow!("persist agent output: {e}"))?;
    }
    for trace in &agent_result.trace {
        substrate
            .put(trace.clone())
            .await
            .map_err(|e| anyhow!("persist agent trace: {e}"))?;
    }

    // Run every configured gate against the working dir.
    let gate_input = build_gate_input(workdir, final_output_sig.id)?;
    substrate
        .put(gate_input.clone())
        .await
        .map_err(|e| anyhow!("persist gate input: {e}"))?;

    let mut verdict_sigs: Vec<Signal> = Vec::new();
    let mut verdict_summary: Vec<(String, bool)> = Vec::new();
    for gate_cfg in &config.gates {
        let verdict = run_gate(gate_cfg, &gate_input, &ctx).await;
        let sig = gate_input
            .derive(
                Kind::GateVerdict,
                Body::from_json(&verdict).map_err(|e| anyhow!("encode verdict: {e}"))?,
            )
            .provenance(Provenance::trusted("cli_gate"))
            .tag("passed", verdict.passed.to_string())
            .tag("gate", &verdict.gate)
            .build();
        substrate
            .put(sig.clone())
            .await
            .map_err(|e| anyhow!("persist verdict: {e}"))?;
        verdict_summary.push((verdict.gate.clone(), verdict.passed));
        verdict_sigs.push(sig);
    }

    // Emit the wrap-up Episode signal.
    let policy = EpisodePolicy::new();
    let episode = policy.record_run(
        &prompt,
        &final_output_sig,
        agent_result.success,
        &verdict_sigs,
        &ctx,
    );
    substrate
        .put(episode.clone())
        .await
        .map_err(|e| anyhow!("persist episode: {e}"))?;

    let total_signals = substrate
        .len()
        .await
        .map_err(|e| anyhow!("count signals: {e}"))?;

    // ── Episode JSONL logging (§I.3.1) ──────────────────────────────────
    //
    // Persist a structured Episode record to `.roko/memory/episodes.jsonl`
    // so the learning subsystem can replay history. Logging errors are
    // intentionally non-fatal: a failed append must never fail the run.
    let ep_path = workdir.join(".roko/memory/episodes.jsonl");
    let ep_logger = EpisodeLogger::new(&ep_path);
    let overall_ok = agent_result.success && verdict_summary.iter().all(|(_, ok)| *ok);
    let mut ep_record = Episode::new("roko-cli", prompt.id.to_hex());
    ep_record.input_signal_hash = prompt.id.to_hex();
    ep_record.output_signal_hash = final_output_sig.id.to_hex();
    ep_record.gate_verdicts = verdict_summary
        .iter()
        .map(|(name, passed)| EpGateVerdict::new(name, *passed))
        .collect();
    ep_record.success = overall_ok;
    ep_record.usage = Usage {
        wall_ms: u64::try_from(ctx.now_ms).unwrap_or(0),
        ..Usage::default()
    };
    if let Err(e) = ep_logger.append(&ep_record).await {
        eprintln!("warning: episode log append failed: {e}");
    }

    Ok(RunReport {
        episode_id: episode.id.to_hex(),
        prompt_id: prompt.id.to_hex(),
        agent_output_id: final_output_sig.id.to_hex(),
        agent_success: agent_result.success,
        gate_verdicts: verdict_summary,
        total_signals,
    })
}

fn load_file_section(workdir: &Path, spec: &PromptFile) -> Result<Signal> {
    let full_path = if spec.path.is_absolute() {
        spec.path.clone()
    } else {
        workdir.join(&spec.path)
    };
    let contents = std::fs::read_to_string(&full_path)
        .with_context(|| format!("read prompt file {}", full_path.display()))?;
    let name = spec
        .name
        .clone()
        .unwrap_or_else(|| spec.path.display().to_string());
    let priority = match spec.priority.as_deref() {
        Some("low") => SectionPriority::Low,
        Some("high") => SectionPriority::High,
        Some("critical") => SectionPriority::Critical,
        _ => SectionPriority::Normal,
    };
    let labeled = format!("File `{}`:\n\n{}", spec.path.display(), contents);
    let mut section = PromptSection::new(&name, labeled)
        .with_priority(priority)
        .with_placement(Placement::Middle);
    if let Some(cap) = spec.hard_cap {
        section = section.with_hard_cap(cap);
    }
    section
        .into_signal()
        .map_err(|e| anyhow!("build file section for {}: {e}", spec.path.display()))
}

/// Post-process the agent output to strip ANSI escapes + thinking traces.
/// Persists both the raw output (as an `AgentMessage` trace) and the cleaned
/// version (as the canonical `AgentOutput`). Returns the cleaned signal.
async fn maybe_clean_output(
    prompt: &Signal,
    agent_result: &AgentResult,
    substrate: &FileSubstrate,
) -> Result<Signal> {
    let raw = agent_result.output.body.as_text().unwrap_or("").to_string();
    let cleaned = clean::clean(&raw);
    if cleaned == raw.trim() {
        // No-op cleaning — just persist the original and move on.
        substrate
            .put(agent_result.output.clone())
            .await
            .map_err(|e| anyhow!("persist agent output: {e}"))?;
        return Ok(agent_result.output.clone());
    }

    // Persist the raw version as a trace signal so nothing is lost.
    let raw_trace = agent_result
        .output
        .derive(Kind::AgentMessage, Body::text(&raw))
        .provenance(Provenance::agent("exec:raw"))
        .tag("stream", "raw_stdout")
        .build();
    substrate
        .put(raw_trace)
        .await
        .map_err(|e| anyhow!("persist raw agent output trace: {e}"))?;

    // Build a fresh AgentOutput signal whose body is the cleaned text. The
    // new signal chains to the prompt (not the raw output) so lineage stays
    // linear: prompt → cleaned_output → gate_input → verdict → episode.
    let clean_sig = prompt
        .derive(Kind::AgentOutput, Body::text(&cleaned))
        .provenance(
            agent_result
                .output
                .tag("agent")
                .map_or_else(|| Provenance::agent("exec"), Provenance::agent),
        )
        .tag("cleaned", "true")
        .tag(
            "agent",
            agent_result.output.tag("agent").unwrap_or("exec"),
        )
        .build();
    substrate
        .put(clean_sig.clone())
        .await
        .map_err(|e| anyhow!("persist cleaned agent output: {e}"))?;
    Ok(clean_sig)
}

fn build_gate_input(workdir: &Path, parent_id: roko_core::ContentHash) -> Result<Signal> {
    let working_dir: PathBuf = workdir
        .canonicalize()
        .with_context(|| format!("canonicalize workdir {}", workdir.display()))?;
    let payload = GatePayload::in_dir(working_dir).with_label("roko-cli");
    let body = Body::from_json(&payload).map_err(|e| anyhow!("encode gate payload: {e}"))?;
    Ok(Signal::builder(Kind::Task)
        .body(body)
        .provenance(Provenance::trusted("cli_run"))
        .lineage([parent_id])
        .build())
}

async fn run_gate(cfg: &GateConfig, input: &Signal, ctx: &Context) -> Verdict {
    match cfg {
        GateConfig::Shell { program, args, timeout_ms } => {
            ShellGate::new(program, args.clone())
                .with_timeout_ms(*timeout_ms)
                .verify(input, ctx)
                .await
        }
        GateConfig::Compile { build_system, timeout_ms } => {
            match parse_build_system(build_system) {
                Ok(bs) => CompileGate::new(bs)
                    .with_timeout_ms(*timeout_ms)
                    .verify(input, ctx)
                    .await,
                Err(e) => Verdict::fail("compile", e),
            }
        }
        GateConfig::Clippy { build_system, timeout_ms } => {
            match parse_build_system(build_system) {
                Ok(bs) => ClippyGate::new(bs)
                    .with_timeout_ms(*timeout_ms)
                    .verify(input, ctx)
                    .await,
                Err(e) => Verdict::fail("clippy", e),
            }
        }
        GateConfig::Test { build_system, timeout_ms } => {
            match parse_build_system(build_system) {
                Ok(bs) => TestGate::new(bs)
                    .with_timeout_ms(*timeout_ms)
                    .verify(input, ctx)
                    .await,
                Err(e) => Verdict::fail("test", e),
            }
        }
    }
}

fn parse_build_system(s: &str) -> Result<BuildSystem, String> {
    match s.to_ascii_lowercase().as_str() {
        "cargo" => Ok(BuildSystem::Cargo),
        "npm" => Ok(BuildSystem::Npm),
        "go" => Ok(BuildSystem::Go),
        "python" | "py" => Ok(BuildSystem::Python),
        "forge" => Ok(BuildSystem::Forge),
        "make" => Ok(BuildSystem::Make),
        other => Err(format!("unknown build_system: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_build_system_accepts_known_names() {
        assert!(matches!(parse_build_system("cargo"), Ok(BuildSystem::Cargo)));
        assert!(matches!(parse_build_system("NPM"), Ok(BuildSystem::Npm)));
        assert!(matches!(parse_build_system("py"), Ok(BuildSystem::Python)));
        assert!(parse_build_system("bazel").is_err());
    }

    #[test]
    fn run_report_overall_success_requires_all_gates() {
        let r = RunReport {
            episode_id: "a".into(),
            prompt_id: "b".into(),
            agent_output_id: "c".into(),
            agent_success: true,
            gate_verdicts: vec![("g1".into(), true), ("g2".into(), true)],
            total_signals: 5,
        };
        assert!(r.overall_success());

        let r = RunReport {
            gate_verdicts: vec![("g1".into(), true), ("g2".into(), false)],
            ..r
        };
        assert!(!r.overall_success());
    }
}
