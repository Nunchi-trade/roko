//! `MockAgent` — deterministic agent for tests.

use crate::agent::{Agent, AgentResult};
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Body, Context, Kind, Provenance, Signal};

/// An agent that returns a canned response. Deterministic; used in tests.
///
/// Configure its behavior via [`MockAgent::reply`] or [`MockAgent::fail_with`].
pub struct MockAgent {
    reply: String,
    fail: bool,
    usage: Usage,
    name: String,
}

impl MockAgent {
    /// A mock that always returns `reply`.
    #[must_use]
    pub fn reply(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
            fail: false,
            usage: Usage::zero(),
            name: "mock".into(),
        }
    }

    /// A mock that always fails with `reason`.
    #[must_use]
    pub fn fail_with(reason: impl Into<String>) -> Self {
        Self {
            reply: reason.into(),
            fail: true,
            usage: Usage::zero(),
            name: "mock_fail".into(),
        }
    }

    /// Pre-set usage metrics that the mock will report.
    #[must_use]
    pub const fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }

    /// Override the mock's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

#[async_trait]
impl Agent for MockAgent {
    async fn run(&self, input: &Signal, _ctx: &Context) -> AgentResult {
        let output = input
            .derive(Kind::AgentOutput, Body::text(&self.reply))
            .provenance(Provenance::agent(&self.name))
            .tag("agent", &self.name)
            .build();
        let r = AgentResult::ok(output).with_usage(self.usage);
        if self.fail {
            AgentResult {
                success: false,
                ..r
            }
        } else {
            r
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    #[tokio::test]
    async fn reply_returns_canned_text() {
        let agent = MockAgent::reply("hello from mock");
        let result = agent.run(&prompt("hi"), &Context::at(0)).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap(), "hello from mock");
        assert_eq!(result.output.kind, Kind::AgentOutput);
    }

    #[tokio::test]
    async fn output_tracks_input_as_lineage() {
        let agent = MockAgent::reply("ok");
        let input = prompt("do X");
        let input_id = input.id;
        let result = agent.run(&input, &Context::at(0)).await;
        assert_eq!(result.output.lineage, vec![input_id]);
    }

    #[tokio::test]
    async fn fail_with_sets_success_false() {
        let agent = MockAgent::fail_with("bang");
        let result = agent.run(&prompt("x"), &Context::at(0)).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn usage_is_reported() {
        let agent = MockAgent::reply("x").with_usage(Usage {
            input_tokens: 42,
            output_tokens: 17,
            ..Default::default()
        });
        let r = agent.run(&prompt("x"), &Context::at(0)).await;
        assert_eq!(r.usage.input_tokens, 42);
        assert_eq!(r.usage.output_tokens, 17);
    }

    #[tokio::test]
    async fn output_is_tagged_with_agent_name() {
        let agent = MockAgent::reply("x").with_name("my_mock");
        let r = agent.run(&prompt("x"), &Context::at(0)).await;
        assert_eq!(r.output.tag("agent"), Some("my_mock"));
    }
}
