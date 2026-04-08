//! `EpisodePolicy` — emits an Episode signal that ties a prompt, an agent
//! output, and a set of gate verdicts together into one replayable record.

use roko_core::{Body, Context, Decay, Kind, Policy, Provenance, Signal};

/// A policy that wraps a full run (prompt → agent → gates) in one Episode signal.
///
/// Unlike the test-suite `EpisodePolicy` in `tests/`, this one emits a *single*
/// Episode signal whose lineage points at the prompt, the agent output, and
/// every gate verdict — i.e. the full run graph.
pub struct EpisodePolicy {
    name: String,
}

impl Default for EpisodePolicy {
    fn default() -> Self {
        Self {
            name: "cli_episode".into(),
        }
    }
}

impl EpisodePolicy {
    /// Construct a new episode policy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a single Episode signal for a completed run.
    ///
    /// The returned signal's lineage is `[prompt_id, agent_output_id, verdict_ids...]`.
    /// Its body is a JSON summary of the run (pass/fail counts, agent success).
    #[must_use]
    pub fn record_run(
        &self,
        prompt: &Signal,
        agent_output: &Signal,
        agent_success: bool,
        verdicts: &[Signal],
        ctx: &Context,
    ) -> Signal {
        let passed = verdicts
            .iter()
            .filter(|v| v.tag("passed") == Some("true"))
            .count();
        let failed = verdicts
            .iter()
            .filter(|v| v.tag("passed") == Some("false"))
            .count();

        let summary = serde_json::json!({
            "prompt_id": prompt.id.to_hex(),
            "agent_output_id": agent_output.id.to_hex(),
            "agent_success": agent_success,
            "gates_passed": passed,
            "gates_failed": failed,
            "logged_at_ms": ctx.now_ms,
        });

        let mut lineage = vec![prompt.id, agent_output.id];
        lineage.extend(verdicts.iter().map(|v| v.id));

        let overall_pass = agent_success && failed == 0;

        Signal::builder(Kind::Episode)
            .body(Body::from_json(&summary).unwrap_or_else(|_| Body::empty()))
            .provenance(Provenance::trusted(&self.name))
            .lineage(lineage)
            .decay(Decay::WISDOM)
            .tag("passed", if overall_pass { "true" } else { "false" })
            .tag("gates_passed", passed.to_string())
            .tag("gates_failed", failed.to_string())
            .build()
    }
}

impl Policy for EpisodePolicy {
    fn decide(&self, _stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        // The CLI drives `record_run` directly; the streaming Policy API is
        // unused here but implemented so `EpisodePolicy` still satisfies the
        // trait contract for composition with future runtimes.
        Vec::new()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind, Verdict};

    fn verdict_sig(gate: &str, passed: bool) -> Signal {
        Signal::builder(Kind::GateVerdict)
            .body(Body::from_json(&Verdict::pass(gate)).unwrap_or(Body::empty()))
            .tag("passed", if passed { "true" } else { "false" })
            .tag("gate", gate)
            .build()
    }

    #[test]
    fn record_run_summarises_verdicts() {
        let policy = EpisodePolicy::new();
        let prompt = Signal::builder(Kind::Prompt).body(Body::text("p")).build();
        let out = Signal::builder(Kind::AgentOutput)
            .body(Body::text("o"))
            .build();
        let v1 = verdict_sig("g1", true);
        let v2 = verdict_sig("g2", false);
        let ep = policy.record_run(
            &prompt,
            &out,
            true,
            &[v1.clone(), v2.clone()],
            &Context::at(42),
        );
        assert_eq!(ep.kind, Kind::Episode);
        assert_eq!(ep.tag("gates_passed"), Some("1"));
        assert_eq!(ep.tag("gates_failed"), Some("1"));
        assert_eq!(ep.tag("passed"), Some("false"));
        assert!(ep.lineage.contains(&prompt.id));
        assert!(ep.lineage.contains(&out.id));
        assert!(ep.lineage.contains(&v1.id));
        assert!(ep.lineage.contains(&v2.id));
    }

    #[test]
    fn overall_pass_when_agent_succeeds_and_no_gate_failures() {
        let policy = EpisodePolicy::new();
        let prompt = Signal::builder(Kind::Prompt).body(Body::text("p")).build();
        let out = Signal::builder(Kind::AgentOutput)
            .body(Body::text("o"))
            .build();
        let ep = policy.record_run(
            &prompt,
            &out,
            true,
            &[verdict_sig("g", true)],
            &Context::at(0),
        );
        assert_eq!(ep.tag("passed"), Some("true"));
    }
}
