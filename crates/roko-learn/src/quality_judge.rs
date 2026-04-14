//! Lightweight LLM judge for tasks without compilable output.
//!
//! Some routing tasks, such as research, documentation, and architecture
//! work, do not produce a binary pass/fail artifact. This module provides a
//! cheap judge call that asks a model to score the response on a 0.0 to 1.0
//! scale and returns that value for routing feedback.

use roko_agent::Agent;
use roko_core::{Body, Context, Engram, Kind};

/// Ask a cheap judge model to score a response in `[0.0, 1.0]`.
///
/// The prompt asks the judge model to rate the response against the provided
/// rubric. If the agent call fails, the output is not readable as text, or the
/// response cannot be parsed as a float, this returns `0.0`.
#[must_use]
pub async fn judge_quality(agent: &dyn Agent, prompt: &str, response: &str, rubric: &str) -> f64 {
    let judge_prompt = format!(
        "Rate the quality of this response on a scale of 0.0 to 1.0.\n\
         Rubric: {rubric}\n\
         Prompt: {prompt}\n\
         Response: {response}\n\
         Score (0.0-1.0):"
    );
    let input = Engram::builder(Kind::Prompt)
        .body(Body::text(judge_prompt))
        .build();
    let result = agent.run(&input, &Context::now()).await;
    if !result.success {
        return 0.0;
    }

    let text = match result.output.body.as_text() {
        Ok(text) => text,
        Err(_) => return 0.0,
    };
    parse_score(text).unwrap_or(0.0)
}

fn parse_score(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if let Ok(score) = trimmed.parse::<f64>() {
        return Some(score.clamp(0.0, 1.0));
    }

    if let Some(first_line) = trimmed.lines().next() {
        if let Ok(score) = first_line.trim().parse::<f64>() {
            return Some(score.clamp(0.0, 1.0));
        }
    }

    trimmed
        .split(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')))
        .find_map(|token| token.parse::<f64>().ok())
        .map(|score| score.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_agent::MockAgent;

    #[tokio::test]
    async fn quality_judge_parses_mock_score() {
        let agent = MockAgent::reply("0.73");
        let score = judge_quality(
            &agent,
            "Summarize the architecture",
            "The service uses a three-layer adapter model.",
            "Prefer correctness, clarity, and completeness.",
        )
        .await;

        assert!((score - 0.73).abs() < 1e-9);
        assert!((0.0..=1.0).contains(&score));
    }

    #[tokio::test]
    async fn quality_judge_returns_zero_when_parse_fails() {
        let agent = MockAgent::reply("cannot score this response");
        let score = judge_quality(&agent, "prompt", "response", "rubric").await;
        assert_eq!(score, 0.0);
    }
}
