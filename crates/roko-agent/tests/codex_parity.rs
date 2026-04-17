use roko_agent::testutil::{
    ParityBackend, run_error_path, run_happy_path, run_session_continuation, run_streaming,
    run_tool_call,
};

#[tokio::test]
async fn happy_path() {
    run_happy_path(ParityBackend::Codex).await.unwrap();
}

#[tokio::test]
async fn streaming() {
    run_streaming(ParityBackend::Codex).await.unwrap();
}

#[tokio::test]
async fn tool_call() {
    run_tool_call(ParityBackend::Codex).await.unwrap();
}

#[tokio::test]
async fn error_path() {
    run_error_path(ParityBackend::Codex).await.unwrap();
}

#[tokio::test]
async fn session_continuation() {
    run_session_continuation(ParityBackend::Codex)
        .await
        .unwrap();
}
