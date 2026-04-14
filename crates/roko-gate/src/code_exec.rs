//! `CodeExecutionGate` — Gemini-backed self-verification via built-in Python.
//!
//! The gate asks Gemini to write and execute Python checks against an agent's
//! proposed change, then accepts the output only when Gemini reports a clean
//! code-execution outcome. No local test process is spawned.

use async_trait::async_trait;
use roko_agent::Agent;
use roko_agent::gemini::{CodeExecutionResultPart, GeminiMetadata, GeminiNativeAgent};
use roko_core::{Body, Context, Engram, Gate, Kind, Verdict};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Structured signal body consumed by [`CodeExecutionGate`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeExecutionPayload {
    /// The task or acceptance criteria the change is meant to satisfy.
    pub task_description: String,
    /// The changed code, diff, or textual output to validate.
    pub changes: String,
}

/// Provider-agnostic result returned by a code-execution backend.
#[derive(Clone, Debug)]
pub struct CodeExecutionOutcome {
    /// Model-authored natural-language summary, if any.
    pub content: String,
    /// Gemini-native execution metadata.
    pub metadata: GeminiMetadata,
}

/// Minimal async interface the gate delegates to.
///
/// [`GeminiNativeAgent`] implements this in production; tests provide a mock.
#[async_trait]
pub trait CodeExecutionBackend: Send + Sync {
    /// Execute the given validation prompt and return Gemini metadata.
    async fn validate_with_code_execution(
        &self,
        prompt: &str,
        ctx: &Context,
    ) -> Result<CodeExecutionOutcome, String>;
}

#[async_trait]
impl CodeExecutionBackend for GeminiNativeAgent {
    async fn validate_with_code_execution(
        &self,
        prompt: &str,
        ctx: &Context,
    ) -> Result<CodeExecutionOutcome, String> {
        let input = Engram::builder(Kind::Prompt)
            .body(Body::text(prompt))
            .build();
        let result = self.run(&input, ctx).await;
        if !result.success {
            let reason = result
                .output
                .body
                .as_text()
                .map(str::to_string)
                .unwrap_or_else(|_| "gemini validation request failed".to_string());
            return Err(reason);
        }

        let metadata_tag = result
            .output
            .tag("gemini_meta")
            .ok_or_else(|| "gemini validation response missing gemini_meta tag".to_string())?;
        let metadata: GeminiMetadata = serde_json::from_str(metadata_tag)
            .map_err(|error| format!("gemini validation metadata decode failed: {error}"))?;

        Ok(CodeExecutionOutcome {
            content: result.output.body.as_text().unwrap_or_default().to_string(),
            metadata,
        })
    }
}

/// Gate that verifies changes using Gemini's built-in Python sandbox.
pub struct CodeExecutionGate<A = GeminiNativeAgent> {
    agent: A,
    name: String,
}

impl<A> CodeExecutionGate<A> {
    /// Construct a gate backed by `agent`.
    #[must_use]
    pub fn new(agent: A) -> Self {
        Self {
            agent,
            name: "code_exec".to_string(),
        }
    }

    /// Override the gate's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn extract_payload(signal: &Engram) -> Option<CodeExecutionPayload> {
        if let Ok(payload) = signal.body.as_json::<CodeExecutionPayload>() {
            return Some(payload);
        }

        if let Ok(changes) = signal.body.as_text() {
            if changes.trim().is_empty() {
                return None;
            }
            return Some(CodeExecutionPayload {
                task_description: signal
                    .tag("task_description")
                    .or_else(|| signal.tag("task"))
                    .unwrap_or_default()
                    .to_string(),
                changes: changes.to_string(),
            });
        }

        None
    }

    fn build_prompt(payload: &CodeExecutionPayload) -> String {
        format!(
            "Verify this code change is correct by writing and running Python tests:\n\n\
             Task: {}\n\n\
             Changes:\n```\n{}\n```\n\n\
             Write Python code that validates the changes are logically correct. \
             Focus on edge cases and invariants.",
            payload.task_description, payload.changes
        )
    }

    fn first_failure(results: &[CodeExecutionResultPart]) -> Option<&CodeExecutionResultPart> {
        results.iter().find(|result| result.outcome != "OUTCOME_OK")
    }
}

#[async_trait]
impl<A> Gate for CodeExecutionGate<A>
where
    A: CodeExecutionBackend,
{
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Verdict {
        let started = Instant::now();
        let elapsed_ms = |t: Instant| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);

        let Some(payload) = Self::extract_payload(signal) else {
            return Verdict::fail(&self.name, "no changes to validate")
                .with_duration(elapsed_ms(started));
        };
        if payload.changes.trim().is_empty() {
            return Verdict::fail(&self.name, "no changes to validate")
                .with_duration(elapsed_ms(started));
        }

        let prompt = Self::build_prompt(&payload);
        let verdict = match self.agent.validate_with_code_execution(&prompt, ctx).await {
            Ok(outcome) => {
                let results = &outcome.metadata.code_execution_results;
                if results.is_empty() {
                    Verdict::fail(&self.name, "code execution validation produced no results")
                } else if let Some(failure) = Self::first_failure(results) {
                    Verdict::fail(
                        &self.name,
                        format!("code execution validation failed: {}", failure.output),
                    )
                    .with_detail(outcome.content)
                } else {
                    Verdict::pass(&self.name).with_detail(outcome.content)
                }
            }
            Err(error) => Verdict::fail(
                &self.name,
                format!("code execution validation request failed: {error}"),
            ),
        };

        verdict.with_duration(elapsed_ms(started))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;

    struct MockBackend {
        response: Result<CodeExecutionOutcome, String>,
        prompts: Arc<Mutex<Vec<String>>>,
    }

    impl MockBackend {
        fn ok(result: CodeExecutionResultPart) -> (Self, Arc<Mutex<Vec<String>>>) {
            let prompts = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    response: Ok(CodeExecutionOutcome {
                        content: "validation complete".to_string(),
                        metadata: GeminiMetadata {
                            grounding_metadata: None,
                            code_execution_results: vec![result],
                            thinking_tokens: None,
                            cached_tokens: None,
                            safety_ratings: Vec::new(),
                        },
                    }),
                    prompts: Arc::clone(&prompts),
                },
                prompts,
            )
        }
    }

    #[async_trait]
    impl CodeExecutionBackend for MockBackend {
        async fn validate_with_code_execution(
            &self,
            prompt: &str,
            _ctx: &Context,
        ) -> Result<CodeExecutionOutcome, String> {
            self.prompts
                .lock()
                .expect("prompt capture")
                .push(prompt.to_string());
            self.response.clone()
        }
    }

    fn prompt_signal(payload: &CodeExecutionPayload) -> Engram {
        Engram::builder(Kind::Task)
            .body(Body::from_json(payload).expect("json body"))
            .build()
    }

    #[tokio::test]
    async fn code_exec_gate_passes_when_execution_succeeds() {
        let (backend, prompts) = MockBackend::ok(CodeExecutionResultPart {
            outcome: "OUTCOME_OK".to_string(),
            output: "all invariants hold".to_string(),
        });
        let gate = CodeExecutionGate::new(backend);
        let signal = prompt_signal(&CodeExecutionPayload {
            task_description: "Implement safe division".to_string(),
            changes: "fn div(a: i32, b: i32) -> Option<i32> { ... }".to_string(),
        });

        let verdict = gate.verify(&signal, &Context::now()).await;

        assert!(verdict.passed);
        assert_eq!(verdict.gate, "code_exec");
        let prompts = prompts.lock().expect("prompt capture");
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("Task: Implement safe division"));
        assert!(prompts[0].contains("Focus on edge cases and invariants."));
    }

    #[tokio::test]
    async fn code_exec_gate_fails_when_execution_reports_error() {
        let (backend, _) = MockBackend::ok(CodeExecutionResultPart {
            outcome: "OUTCOME_ERROR".to_string(),
            output: "AssertionError: expected None for division by zero".to_string(),
        });
        let gate = CodeExecutionGate::new(backend);
        let signal = prompt_signal(&CodeExecutionPayload {
            task_description: "Implement safe division".to_string(),
            changes: "fn div(a: i32, b: i32) -> Option<i32> { Some(a / b) }".to_string(),
        });

        let verdict = gate.verify(&signal, &Context::now()).await;

        assert!(!verdict.passed);
        assert!(
            verdict
                .reason
                .contains("AssertionError: expected None for division by zero")
        );
    }

    #[tokio::test]
    async fn code_exec_gate_fails_when_backend_returns_no_results() {
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let gate = CodeExecutionGate::new(MockBackend {
            response: Ok(CodeExecutionOutcome {
                content: String::new(),
                metadata: GeminiMetadata {
                    grounding_metadata: None,
                    code_execution_results: Vec::new(),
                    thinking_tokens: None,
                    cached_tokens: None,
                    safety_ratings: Vec::new(),
                },
            }),
            prompts,
        });
        let signal = prompt_signal(&CodeExecutionPayload {
            task_description: "Implement safe division".to_string(),
            changes: "fn div(a: i32, b: i32) -> Option<i32> { ... }".to_string(),
        });

        let verdict = gate.verify(&signal, &Context::now()).await;

        assert!(!verdict.passed);
        assert!(verdict.reason.contains("produced no results"));
    }

    #[tokio::test]
    async fn code_exec_gate_uses_text_body_and_task_tag_fallback() {
        let (backend, prompts) = MockBackend::ok(CodeExecutionResultPart {
            outcome: "OUTCOME_OK".to_string(),
            output: "ok".to_string(),
        });
        let gate = CodeExecutionGate::new(backend);
        let signal = Engram::builder(Kind::Task)
            .body(Body::text("diff --git a/x b/x"))
            .tag("task", "Validate diff output")
            .build();

        let verdict = gate.verify(&signal, &Context::now()).await;

        assert!(verdict.passed);
        let prompts = prompts.lock().expect("prompt capture");
        assert!(prompts[0].contains("Task: Validate diff output"));
        assert!(prompts[0].contains("diff --git a/x b/x"));
    }
}
