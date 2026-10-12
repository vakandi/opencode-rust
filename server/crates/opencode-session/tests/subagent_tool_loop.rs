//! Reproduction tests for the subagent `task` tool defect.
//!
//! Before the fix, `SubtaskExecutor::execute_inline` sent ONE non-streaming request
//! with tools attached, then read `parts.first().text`. For a tool-call response the
//! first part is `ContentPart { content_type: "tool_use", text: None }`, so the tool
//! call was discarded and the subagent returned `""`. It also called `provider.chat`
//! with no timeout and no abort token, so a stalled provider blocked the parent
//! turn forever.
//!
//! Each test below fails against the old implementation for exactly that reason.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use opencode_provider::message::{
    ChatRequest, ChatResponse, Choice, Content, ContentPart, Message, Role, ToolDefinition, ToolUse,
};
use opencode_provider::provider::{ModelInfo, Provider, ProviderError};
use opencode_provider::stream::{StreamEvent, StreamResult, StreamUsage};
use opencode_session::prompt::{ModelRef, SubtaskExecutor};
use opencode_tool::{Tool, ToolContext, ToolError, ToolRegistry, ToolResult};

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

struct RecordingTool {
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
}

#[async_trait]
impl Tool for RecordingTool {
    fn id(&self) -> &str {
        "record"
    }
    fn description(&self) -> &str {
        "Records its arguments so the test can prove the subagent really executed it."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "note": { "type": "string" } },
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        self.calls.lock().unwrap().push(args.clone());
        Ok(ToolResult {
            title: "recorded".to_string(),
            output: format!(
                "recorded {}",
                args.get("note").and_then(|v| v.as_str()).unwrap_or("")
            ),
            metadata: Default::default(),
            truncated: false,
        })
    }
}

enum Script {
    /// Call 1 asks for `record`; every later call returns plain text.
    ToolThenText,
    /// Every call asks for `record` — an adversarial provider that never stops.
    AlwaysTool,
}

struct CountingProvider {
    calls: AtomicUsize,
    script: Script,
    offered: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CountingProvider {
    fn new(script: Script) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            script,
            offered: Arc::new(Mutex::new(Vec::new())),
        })
    }
}

#[async_trait]
impl Provider for CountingProvider {
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
            choices: vec![Choice {
                index: 0,
                message: Message {
                    role: Role::Assistant,
                    // Exactly the shape the OLD code stringified: a tool-call answer
                    // whose first part carries no `text`.
                    content: Content::Parts(vec![ContentPart {
                        content_type: "tool_use".to_string(),
                        text: None,
                        image_url: None,
                        tool_use: Some(ToolUse {
                            id: "c1".to_string(),
                            name: "record".to_string(),
                            input: serde_json::json!({ "note": "x" }),
                        }),
                        tool_result: None,
                        cache_control: None,
                        filename: None,
                        media_type: None,
                        provider_options: None,
                    }]),
                    cache_control: None,
                    provider_options: None,
                },
                finish_reason: Some("tool-calls".to_string()),
            }],
            usage: None,
        })
    }
    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let names: Vec<String> = request
            .tools
            .unwrap_or_default()
            .iter()
            .map(|t: &ToolDefinition| t.name.clone())
            .collect();
        self.offered.lock().unwrap().push(names);

        let ask = match self.script {
            Script::AlwaysTool => true,
            Script::ToolThenText => n == 0,
        };
        let events: Vec<Result<StreamEvent, ProviderError>> = if ask {
            vec![
                Ok(StreamEvent::ToolCallStart {
                    id: "c1".to_string(),
                    name: "record".to_string(),
                }),
                Ok(StreamEvent::ToolCallEnd {
                    id: "c1".to_string(),
                    name: "record".to_string(),
                    input: serde_json::json!({ "note": format!("turn-{n}") }),
                }),
                Ok(finish("tool-calls")),
            ]
        } else {
            vec![
                Ok(StreamEvent::TextDelta("done".to_string())),
                Ok(finish("stop")),
            ]
        };
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

/// One delta, then silence forever — the shape that hung the parent turn.
struct StallingProvider {
}

impl StallingProvider {
    fn new() -> Self {
        Self {
        }
    }
}

#[async_trait]
impl Provider for StallingProvider {
    fn id(&self) -> &str {
        "stall"
    }
    fn name(&self) -> &str {
        "Stall"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![model()]
    }
    fn get_model(&self, _id: &str) -> Option<&ModelInfo> {
        None
    }
    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        // The old path used chat(): returning at all is what separates "hangs forever"
        // from "times out".
        futures::future::pending().await
    }
    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let stream = futures::stream::unfold(0usize, |state| async move {
            if state == 0 {
                Some((Ok(StreamEvent::TextDelta("partial".to_string())), 1))
            } else {
                futures::future::pending::<Option<(Result<StreamEvent, ProviderError>, usize)>>()
                    .await
            }
        });
        Ok(Box::pin(stream))
    }
}

async fn registry_with(calls: Arc<Mutex<Vec<serde_json::Value>>>) -> ToolRegistry {
    let registry = ToolRegistry::new();
    registry.register(RecordingTool { calls }).await;
    registry
}

fn executor(prompt: &str) -> SubtaskExecutor {
    SubtaskExecutor::new("general", prompt)
}

fn model_ref() -> ModelRef {
    ModelRef {
        provider_id: "mock".to_string(),
        model_id: "mock-model".to_string(),
    }
}

#[allow(dead_code)]
fn ctx() -> ToolContext {
    ToolContext::new(
        "ses_parent".to_string(),
        "msg".to_string(),
        "/tmp".to_string(),
    )
}

/// T1 — the core defect: the tool must actually run and its result must come back.
#[tokio::test]
async fn subagent_runs_tool_then_returns_final_text() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_with(calls.clone()).await;
    let provider = CountingProvider::new(Script::ToolThenText);

    let out = executor("do the thing")
        .with_model(model_ref())
        .execute_inline(Arc::clone(&provider) as Arc<dyn Provider>, &registry, &[])
        .await
        .expect("subagent must not error");

    assert_eq!(
        out, "done",
        "subagent must return the text produced after the tool result"
    );
    let recorded = calls.lock().unwrap().clone();
    assert_eq!(
        recorded.len(),
        1,
        "the subagent must execute the tool exactly once, got {recorded:?}"
    );
    assert_eq!(recorded[0]["note"], "turn-0");
}

/// T2 — a provider that keeps asking for tools must not loop forever.
#[tokio::test]
async fn subagent_loop_terminates_when_provider_always_requests_tools() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_with(calls).await;
    let provider = CountingProvider::new(Script::AlwaysTool);

    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        executor("loop forever")
            .with_model(model_ref())
            .execute_inline(Arc::clone(&provider) as Arc<dyn Provider>, &registry, &[]),
    )
    .await
    .expect("subagent must terminate, not hang");

    let err = outcome
        .expect_err("a provider that never stops must surface an error");
    assert!(
        err.to_string().contains("step limit"),
        "expected a step-limit error, got: {err}"
    );
    let used = provider.calls.load(Ordering::SeqCst);
    assert!(
        used <= 24,
        "provider was called {used} times, cap is 24"
    );
}

/// T4 — the reported hang: a stalled provider must time out, not block forever.
#[tokio::test]
async fn subagent_provider_stall_times_out_instead_of_hanging() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_with(calls).await;

    let outcome = tokio::time::timeout(
        Duration::from_secs(140),
        executor("stall me")
            .with_model(model_ref())
            .execute_inline(
                Arc::new(StallingProvider::new()) as Arc<dyn Provider>,
                &registry,
                &[],
            ),
    )
    .await;

    assert!(
        outcome.is_ok(),
        "a stalled provider must not block the caller past the request timeout"
    );
    let err = outcome.unwrap().expect_err("a stall must surface an error");
    assert!(
        err.to_string().contains("timeout"),
        "expected a timeout error, got: {err}"
    );
}

/// T5 — a disabled tool must not be offered to the model.
#[tokio::test]
async fn subagent_disabled_tool_is_not_offered() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_with(calls).await;
    let provider = CountingProvider::new(Script::ToolThenText);

    let _ = executor("no record please")
        .with_model(model_ref())
        .execute_inline(
            Arc::clone(&provider) as Arc<dyn Provider>,
            &registry,
            &["record".to_string()],
        )
        .await;

    let offered = provider.offered.lock().unwrap().clone();
    assert!(!offered.is_empty(), "the request must have been sent");
    for list in offered {
        assert!(
            !list.iter().any(|n| n == "record"),
            "a disabled tool must not appear in the request: {list:?}"
        );
    }
}
/// T3 — a cancelled parent must stop the subagent before it does any work.
///
/// The subagent runs inside the parent's tool loop, so a child that ignores
/// cancellation takes the parent down with it. Cancelling up front is the
/// deterministic form: the guard at the top of the first step must fire, and the
/// provider must never be called at all.
#[tokio::test]
async fn subagent_abort_token_cancels_the_loop() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_with(calls).await;
    let provider = CountingProvider::new(Script::AlwaysTool);
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();

    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        executor("abort me")
            .with_model(model_ref())
            .execute_inline_with_ctx(
                Arc::clone(&provider) as Arc<dyn Provider>,
                &registry,
                &[],
                ctx().with_abort(token),
            ),
    )
    .await
    .expect("abort must not hang");

    let err = outcome.expect_err("a cancelled subagent must report an error");
    assert!(
        err.to_string().contains("abort"),
        "expected an abort error, got: {err}"
    );
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        0,
        "a cancelled subagent must not reach the provider"
    );
}
