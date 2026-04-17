use roko_agent::testutil::{
    ParityBackend, run_error_path, run_happy_path, run_session_continuation, run_streaming,
    run_tool_call,
};

#[tokio::test]
async fn happy_path() {
    run_happy_path(ParityBackend::Exec).await.unwrap();
}

#[tokio::test]
async fn error_path() {
    run_error_path(ParityBackend::Exec).await.unwrap();
}

#[tokio::test]
#[ignore = "ExecAgent only captures stdout/stderr; it has no streaming delta protocol"]
async fn streaming() {
    run_streaming(ParityBackend::Exec).await.unwrap();
}

#[tokio::test]
#[ignore = "ExecAgent has no tool-call wire protocol"]
async fn tool_call() {
    run_tool_call(ParityBackend::Exec).await.unwrap();
}

#[tokio::test]
#[ignore = "ExecAgent is stateless and cannot resume a prior session"]
async fn session_continuation() {
    run_session_continuation(ParityBackend::Exec).await.unwrap();
}
