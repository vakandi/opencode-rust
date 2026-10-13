//! The turn loop: `run` must not return until the agent is genuinely finished.
//!
//! The bug these pin: `run` called `AgentExecutor::execute_streaming`, one provider call that
//! never executes a tool. A turn that asked for a tool printed `[Calling tool: …]` and the process
//! exited. `RunSessionDriver` drives `SessionPrompt` instead, which runs tools and re-prompts
//! until the model answers without asking for anything.
//!
//! These drive the REAL toolset (`bash`) rather than an injected test tool: the session loop
//! builds its own registry internally, so a test-only tool would prove nothing about production.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use opencode_cli::run_session::{RunSessionDriver, TurnOutcome};
use opencode_provider::message::{ChatRequest, ChatResponse};
use opencode_provider::provider::{ModelInfo, Provider, ProviderError};
use opencode_provider::stream::{StreamEvent, StreamResult, StreamUsage};
use opencode_session::prompt::AgentParams;
use opencode_session::session::Session;

fn model() -> ModelInfo {
    ModelInfo {
        id: "mock-model".to_string(),
        name: "Mock".to_string(),
        provider: "mock".to_string(),
        context_window: 128_000,
        max_output_tokens: 8192,
        supports_vision: false,
        supports_tools: true,
        cost_per_million_input: 0.0,
        cost_per_million_output: 0.0,
    }
}

fn finish(reason: &str) -> StreamEvent {
    StreamEvent::FinishStep {
        finish_reason: Some(reason.to_string()),
        usage: StreamUsage::default(),
        provider_metadata: None,
    }
}

#[derive(Clone)]
enum Script {
    /// Call 1 asks for `bash` with this command; later calls answer in prose.
    ToolThenText(String),
    /// Every call asks for `bash true` — a model that never stops.
    AlwaysTool,
    /// Every call answers in prose.
    AlwaysText,
}

struct ScriptedProvider {
    calls: AtomicUsize,
    script: Script,
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn id(&self) -> &str {
        "mock"
    }
    fn name(&self) -> &str {
        "Mock"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![model()]
    }
    fn get_model(&self, _id: &str) -> Option<&ModelInfo> {
        None
    }
    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            id: "mock".to_string(),
            model: "mock-model".to_string(),
            choices: vec![],
            usage: None,
        })
    }
    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let ask = match &self.script {
            Script::AlwaysTool => true,
            Script::ToolThenText(_) => n == 0,
            Script::AlwaysText => false,
        };
        let command = match &self.script {
            Script::ToolThenText(cmd) => cmd.clone(),
            _ => "true".to_string(),
        };
        let events: Vec<Result<StreamEvent, ProviderError>> = if ask {
            vec![
                Ok(StreamEvent::ToolCallStart {
                    id: "c1".to_string(),
                    name: "bash".to_string(),
                }),
                Ok(StreamEvent::ToolCallEnd {
                    id: "c1".to_string(),
                    name: "bash".to_string(),
                    // bash requires both `command` and `description`; omitting the latter makes
                    // the tool reject its own arguments.
                    input: serde_json::json!({ "command": command, "description": "test step" }),
                }),
                Ok(finish("tool-calls")),
            ]
        } else {
            vec![
                Ok(StreamEvent::TextDelta("all done".to_string())),
                Ok(finish("stop")),
            ]
        };
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn driver(script: Script, sink: Arc<Mutex<Vec<u8>>>) -> RunSessionDriver {
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider {
        calls: AtomicUsize::new(0),
        script,
    });
    let session = Session::new("proj", std::env::temp_dir().to_string_lossy().to_string());
    RunSessionDriver::new(provider, session, None, AgentParams::default())
        .await
        .with_sink(Box::new(Sink(sink)))
}

#[tokio::test]
async fn t1_run_executes_the_tool_and_keeps_going_until_the_final_answer() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let sink = Arc::new(Mutex::new(Vec::new()));
    let mut d = driver(
        Script::ToolThenText(format!("touch {}", marker.display())),
        sink.clone(),
    )
    .await;

    let outcome = tokio::time::timeout(Duration::from_secs(60), d.run_turn("do it"))
        .await
        .expect("run_turn must terminate, never hang")
        .unwrap();

    assert_eq!(outcome, TurnOutcome::Completed);
    assert!(
        marker.exists(),
        "the requested tool must actually execute — the old `run` executed zero and exited"
    );
    let out = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
    assert!(
        out.contains("all done"),
        "run must keep going until the model's real final answer, got {:?}",
        out
    );
}

#[tokio::test]
async fn t2_transcript_is_streamed_not_buffered_until_the_end() {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let mut d = driver(Script::AlwaysText, sink.clone()).await;

    d.run_turn("say something").await.unwrap();

    let out = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
    assert_eq!(
        out, "all done",
        "only the assistant's text belongs on stdout — the user prompt leaked through before"
    );
}

#[tokio::test]
async fn t3_a_model_that_never_stops_is_truncated_rather_than_hanging_forever() {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let mut d = driver(Script::AlwaysTool, sink).await;

    let outcome = tokio::time::timeout(Duration::from_secs(90), d.run_turn("loop forever"))
        .await
        .expect("an endless model must still terminate instead of hanging")
        .unwrap();

    assert_eq!(
        outcome,
        TurnOutcome::MaxStepsTruncated,
        "hitting the step ceiling is truncation, and must be reported as such"
    );
}

#[tokio::test]
async fn t4_run_waits_for_outstanding_tool_work_before_returning() {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let mut d = driver(Script::ToolThenText("sleep 0.4".to_string()), sink).await;

    let started = Instant::now();
    d.run_turn("do it slowly").await.unwrap();

    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "run_turn returned before the tool finished ({:?}) — this is exactly the cut-off bug",
        started.elapsed()
    );
}

#[tokio::test]
async fn t5_a_turn_with_no_tools_completes_in_one_round_trip() {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let mut d = driver(Script::AlwaysText, sink).await;

    assert_eq!(d.run_turn("hello").await.unwrap(), TurnOutcome::Completed);
}
