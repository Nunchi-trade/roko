//! Primitive 1: `RunBlock` — a completed run summary.
//!
//! Pushed into scrollback via `insert_before` when a run/turn finishes.
//! This is the canonical "clack-style" structured output block.
//!
//! ```text
//! ◆ agent      auditor@v1  ·  eid://roko/auditor.v1  (attested)
//! │ predict    $0.043  ·  12.4s  ·  route: haiku → sonnet
//! │ gates      secret_scan ✔   cost_ceiling ✔   policy ✔
//! │ knowledge  loaded 7 engrams (3 agents, 0.91 conf)
//! │ actual     $0.031  (-28% vs predicted)  ·  routed to haiku
//! └ deposited  2 new engrams → /infra/payments-svc
//! ```

use ratatui::text::Line;

use crate::tui::Theme;

use super::super::styled;
use super::super::symbols;

/// Data for rendering a completed run summary block.
#[derive(Debug, Clone)]
pub struct RunBlockData {
    /// Agent name (e.g. "auditor@v1").
    pub agent_name: String,
    /// Agent identity URI (e.g. "eid://roko/auditor.v1").
    pub identity: Option<String>,
    /// Identity attestation status.
    pub attested: bool,

    /// Predicted cost in USD.
    pub predicted_cost: Option<f64>,
    /// Predicted time in seconds.
    pub predicted_time: Option<f64>,
    /// Predicted route (model name).
    pub predicted_route: Option<String>,

    /// Gate verdicts: (name, passed).
    pub gate_verdicts: Vec<(String, bool)>,

    /// Knowledge engrams loaded before dispatch.
    pub knowledge_loaded: Option<KnowledgeInfo>,

    /// Actual cost in USD.
    pub actual_cost: Option<f64>,
    /// Actual model used.
    pub actual_route: Option<String>,
    /// Actual duration in seconds.
    pub actual_time: Option<f64>,

    /// Tool calls made during execution.
    pub tool_calls: Vec<ToolCallInfo>,

    /// Knowledge engrams deposited after run.
    pub deposited_count: usize,
    /// Deposit target path.
    pub deposited_path: Option<String>,

    /// Chain anchor info.
    pub chain_block: Option<u64>,
}

/// Knowledge query result summary.
#[derive(Debug, Clone)]
pub struct KnowledgeInfo {
    /// Number of engrams loaded.
    pub count: usize,
    /// Topic path.
    pub topic: String,
    /// Number of distinct source agents.
    pub agent_count: usize,
    /// Average confidence score.
    pub avg_confidence: f64,
}

/// Summary of a tool call.
#[derive(Debug, Clone)]
pub struct ToolCallInfo {
    /// Tool name (e.g. "ReadFile").
    pub name: String,
    /// Brief summary (e.g. "src/auth.rs (247 lines)").
    pub summary: String,
    /// Duration in seconds.
    pub duration_s: f64,
}

impl RunBlockData {
    /// Render this block as styled lines for scrollback.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::with_capacity(10);

        // Header: ◆ agent  name · identity (attested)
        let detail = self.identity.as_ref().map(|id| {
            let status = if self.attested { "attested" } else { "local" };
            format!("{id}  ({status})")
        });
        lines.push(styled::section_start(
            theme,
            "agent",
            &self.agent_name,
            detail.as_deref(),
        ));

        // Prediction line
        if let Some(cost) = self.predicted_cost {
            let mut parts = vec![format!("${cost:.3}")];
            if let Some(t) = self.predicted_time {
                parts.push(format!("{t:.1}s"));
            }
            if let Some(ref r) = self.predicted_route {
                parts.push(format!("route: {r}"));
            }
            let value = parts.join(&format!("  {}  ", symbols::SEP));
            lines.push(styled::continuation(theme, "predict", &value, None));
        }

        // Knowledge line
        if let Some(ref k) = self.knowledge_loaded {
            let value = format!(
                "loaded {} engrams from {} ({} agents, {:.2} conf)",
                k.count, k.topic, k.agent_count, k.avg_confidence,
            );
            lines.push(styled::continuation(theme, "knowledge", &value, None));
        }

        // Empty bar line before tool calls (if any)
        if !self.tool_calls.is_empty() {
            lines.push(styled::bar_empty(theme));
        }

        // Tool calls (collapsed)
        for tc in &self.tool_calls {
            lines.push(styled::tool_call_collapsed(
                theme,
                &tc.name,
                &tc.summary,
                tc.duration_s,
            ));
        }

        // Empty bar line after tool calls
        if !self.tool_calls.is_empty() {
            lines.push(styled::bar_empty(theme));
        }

        // Gates line
        if !self.gate_verdicts.is_empty() {
            lines.push(styled::gates_line(theme, &self.gate_verdicts));
        }

        // Actual cost line
        if let Some(actual) = self.actual_cost {
            let delta = self
                .predicted_cost
                .map(|p| styled::cost_delta(actual, p))
                .unwrap_or_else(|| format!("${actual:.3}"));

            let detail = self.actual_route.as_ref().map(|r| format!("routed to {r}"));
            lines.push(styled::continuation(
                theme,
                "actual",
                &delta,
                detail.as_deref(),
            ));
        }

        // Chain anchor
        if let Some(block) = self.chain_block {
            lines.push(styled::continuation(
                theme,
                "chain",
                &format!("anchored block #{block}"),
                Some("mirage-rs local"),
            ));
        }

        // Deposited knowledge
        if self.deposited_count > 0 {
            let path = self.deposited_path.as_deref().unwrap_or("knowledge store");
            lines.push(styled::section_end(
                theme,
                "deposited",
                &format!(
                    "{} new engram{} {} {}",
                    self.deposited_count,
                    if self.deposited_count == 1 { "" } else { "s" },
                    symbols::ARROW,
                    path,
                ),
            ));
        } else if self.actual_cost.is_some() {
            // Close the block even without deposits
            lines.push(styled::section_end(theme, "", ""));
        }

        lines
    }
}

impl Default for RunBlockData {
    fn default() -> Self {
        Self {
            agent_name: String::new(),
            identity: None,
            attested: false,
            predicted_cost: None,
            predicted_time: None,
            predicted_route: None,
            gate_verdicts: Vec::new(),
            knowledge_loaded: None,
            actual_cost: None,
            actual_route: None,
            actual_time: None,
            tool_calls: Vec::new(),
            deposited_count: 0,
            deposited_path: None,
            chain_block: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_block_minimal() {
        let theme = Theme::dark();
        let data = RunBlockData {
            agent_name: "researcher@v2".into(),
            actual_cost: Some(0.031),
            ..Default::default()
        };
        let lines = data.to_lines(&theme);
        assert!(!lines.is_empty());
        // Should have at least header + actual cost
        assert!(lines.len() >= 2);
    }

    #[test]
    fn run_block_full() {
        let theme = Theme::dark();
        let data = RunBlockData {
            agent_name: "auditor@v1".into(),
            identity: Some("eid://roko/auditor.v1".into()),
            attested: true,
            predicted_cost: Some(0.043),
            predicted_time: Some(12.4),
            predicted_route: Some("haiku".into()),
            gate_verdicts: vec![
                ("compile".into(), true),
                ("test".into(), true),
                ("clippy".into(), false),
            ],
            knowledge_loaded: Some(KnowledgeInfo {
                count: 7,
                topic: "/infra/payments-svc".into(),
                agent_count: 3,
                avg_confidence: 0.91,
            }),
            actual_cost: Some(0.031),
            actual_route: Some("haiku".into()),
            actual_time: Some(9.8),
            tool_calls: vec![ToolCallInfo {
                name: "ReadFile".into(),
                summary: "src/auth.rs (247 lines)".into(),
                duration_s: 0.3,
            }],
            deposited_count: 2,
            deposited_path: Some("/infra/payments-svc".into()),
            chain_block: Some(4821),
        };
        let lines = data.to_lines(&theme);
        // Full block should have: header, predict, knowledge, bar, tool,
        // bar, gates, actual, chain, deposited
        assert!(lines.len() >= 8, "got {} lines", lines.len());
    }
}
