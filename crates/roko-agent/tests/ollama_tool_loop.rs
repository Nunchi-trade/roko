//! Integration test: Ollama tool loop via OpenAI-compatible endpoint.
//!
//! Gate: only runs when `ROKO_TEST_OLLAMA=1` is set. Requires a local
//! Ollama instance at `http://localhost:11434` with `llama3.2` pulled.
//!
//! Tests the full tool-loop cycle:
//!   1. Configure `OpenAiCompatLlmBackend` pointed at Ollama
//!   2. Send a prompt that triggers `read_file` tool use
//!   3. Verify the LLM response includes a tool call
//!   4. Execute the tool call (dispatcher handles it)
//!   5. Send the tool result back to the LLM
//!   6. Verify the LLM produces a final text response

use std::sync::Arc;

use roko_agent::OpenAiCompatLlmBackend;
use roko_agent::dispatcher::ToolDispatcher;
use roko_agent::rate_limit::ProviderRateLimiter;
use roko_agent::tool_loop::{StopReason, ToolLoop};
use roko_agent::translate::{OpenAiTranslator, Translator};
use roko_core::tool::{ToolContext, ToolDef};
use roko_std::tool::builtin::read_file;
use roko_std::tool::handlers::handler_for;
use roko_std::tool::registry::StaticToolRegistry;
use tempfile::tempdir;

fn ollama_enabled() -> bool {
    std::env::var("ROKO_TEST_OLLAMA")
        .map(|v| v == "1")
        .unwrap_or(false)
}

fn read_tools() -> Vec<ToolDef> {
    vec![read_file::tool_def()]
}

fn tool_context(worktree: &std::path::Path) -> ToolContext {
    ToolContext::testing(worktree)
}

/// Full tool-loop cycle: Ollama calls `read_file`, receives the result,
/// and produces a final text answer.
#[tokio::test]
async fn ollama_tool_loop_read_file() {
    if !ollama_enabled() {
        eprintln!("skipping ollama_tool_loop_read_file: set ROKO_TEST_OLLAMA=1 to run");
        return;
    }

    // Seed a temp file for the LLM to read.
    let tempdir = tempdir().expect("tempdir");
    let file_path = tempdir.path().join("test.txt");
    tokio::fs::write(&file_path, "The secret number is 42.")
        .await
        .expect("write test file");

    // Build the backend pointed at Ollama's OpenAI-compat endpoint.
    // Use a generous rate limiter (no real throttling needed for local Ollama).
    let rate_limiter = Arc::new(ProviderRateLimiter::new(600));
    let backend = OpenAiCompatLlmBackend::new("ollama", "llama3.2")
        .with_base_url("http://localhost:11434/v1")
        .with_timeout_ms(60_000)
        .with_rate_limiter(rate_limiter);

    // Wire up the tool dispatcher.
    let registry = Arc::new(StaticToolRegistry::new());
    let resolver = Arc::new(|name: &str| handler_for(name));
    let dispatcher = Arc::new(ToolDispatcher::new(registry, resolver));
    let translator: Arc<dyn Translator> = Arc::new(OpenAiTranslator);
    let tool_loop = ToolLoop::new(translator, dispatcher, Arc::new(backend))
        .with_max_iterations(5);

    let result = tool_loop
        .run(
            "You are a helpful assistant. When asked to read a file, use the read_file tool. \
             After reading the file, summarize its contents in a short sentence.",
            &format!(
                "Read the file at path \"test.txt\" and tell me what it says."
            ),
            &read_tools(),
            &tool_context(tempdir.path()),
        )
        .await;

    // The loop should have completed normally (not errored or hit max iterations).
    assert_eq!(
        result.stop_reason,
        StopReason::Stop,
        "expected clean stop, got {:?}; final_text = {:?}",
        result.stop_reason,
        result.final_text
    );

    // The LLM should have called read_file at least once.
    assert!(
        !result.tool_calls.is_empty(),
        "expected at least one tool call, got none"
    );
    assert!(
        result.tool_calls.iter().any(|tc| tc.name == "read_file"),
        "expected a read_file tool call, got: {:?}",
        result
            .tool_calls
            .iter()
            .map(|tc| tc.name.as_str())
            .collect::<Vec<_>>()
    );

    // The final text should reference the file's content.
    assert!(
        !result.final_text.is_empty(),
        "expected non-empty final text"
    );
    // The LLM should mention "42" or "secret" from the file content.
    let text_lower = result.final_text.to_lowercase();
    assert!(
        text_lower.contains("42") || text_lower.contains("secret"),
        "expected final text to reference file content, got: {:?}",
        result.final_text
    );

    // At least one iteration must have occurred (the tool-call round).
    assert!(
        result.iterations >= 1,
        "expected at least 1 iteration, got {}",
        result.iterations
    );
}
