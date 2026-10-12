use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use opencode_plugin::{HookContext, HookEvent};
use opencode_provider::{
    get_model_context_limit, ChatRequest, Content, ImageUrl, Message, Provider, StreamEvent,
    StreamUsage, ToolDefinition, ToolErrorKind as StreamToolErrorKind,
};

use crate::compaction::{CompactionConfig, CompactionEngine, ModelLimits, TokenUsage};
use crate::message_v2::{
    CompletedTime, ErrorTime, RunningTime, ToolState,
};
use crate::retry::{self, ApiErrorInfo};
use crate::snapshot::{Snapshot, SnapshotPatch};
use crate::{MessageError, MessageInfo, MessageUsage, MessageWithParts, Part};

pub const OUTPUT_TOKEN_MAX: u64 = 8192;

/// Doom loop detection threshold: if the last N tool calls are identical,
/// trigger a permission ask.
const DOOM_LOOP_THRESHOLD: usize = 3;

#[derive(Debug, Clone)]
pub struct LlmAgent {
    pub name: String,
    pub system_prompt: Option<String>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct LlmModelRef {
    pub model_id: String,
    pub provider_id: String,
}

pub struct StreamInput {
    pub user: MessageInfo,
    pub session_id: String,
    pub model: LlmModelRef,
    pub agent: LlmAgent,
    pub system: Vec<String>,
    pub abort: tokio_util::sync::CancellationToken,
    pub messages: Vec<Message>,
    pub small: bool,
    pub tools: HashMap<String, ToolDefinition>,
    pub retries: Option<u32>,
    pub tool_choice: Option<ToolChoice>,
    /// Model pricing: cost per million tokens (input, output, cache_read, cache_write)
    pub cost_rates: Option<CostRates>,
    /// Project working directory for git snapshot tracking.
    /// If None, snapshot tracking is skipped.
    pub work_dir: Option<PathBuf>,
}

/// Per-million-token cost rates for a model
#[derive(Debug, Clone, Default)]
pub struct CostRates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    /// Optional "experimentalOver200K" pricing tier used when total token volume exceeds 200k.
    pub over_200k: Option<CostRateTier>,
}

#[derive(Debug, Clone, Default)]
pub struct CostRateTier {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, Copy)]
pub enum ToolChoice {
    Auto,
    Required,
    None,
}

pub struct StreamOutput {
    pub events: tokio::sync::mpsc::Receiver<StreamEvent>,
}

// ============================================================================
// Process result — mirrors TS "compact" | "stop" | "continue"
// ============================================================================

/// The outcome of a single `StreamProcessor::process` invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessResult {
    /// Triggers session compaction.
    Compact,
    /// Stops processing (blocked by permission or error).
    Stop,
    /// Normal completion — caller may continue the agentic loop.
    Continue,
}

/// Additional processor-side signals to be consumed by callers (e.g. permission flow).
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessorEvent {
    DoomLoopDetected {
        tool_name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolExecutionErrorKind {
    PermissionDenied,
    QuestionRejected,
    Other,
}

// ============================================================================
// Stream Processor — full state machine matching TS SessionProcessor
// ============================================================================

/// Tracks reasoning blocks being streamed.
#[derive(Debug, Clone)]
struct ReasoningState {
    _part_id: String,
    text: String,
    _start_time: i64,
    _metadata: Option<serde_json::Value>,
}

/// Tracks tool calls being streamed.
#[derive(Debug, Clone)]
pub struct ToolCallState {
    part_id: String,
    tool_name: String,
    _call_id: String,
    raw_input: String,
    parsed_input: serde_json::Value,
    state: ToolState,
}

/// Tracks the current text block being streamed.
#[derive(Debug, Clone)]
struct TextState {
    _part_id: String,
    text: String,
    _start_time: i64,
    _metadata: Option<serde_json::Value>,
}

/// Full stream processor with state management, retry, doom-loop detection,
/// and abort handling — matching the TS `SessionProcessor.create()`.
pub struct StreamProcessor {
    /// Map of reasoning block id -> state.
    reasoning_map: HashMap<String, ReasoningState>,
    /// Map of tool call id -> state.
    tool_calls: HashMap<String, ToolCallState>,
    /// Current text block being streamed.
    current_text: Option<TextState>,
    /// Current snapshot hash (from start-step).
    snapshot: Option<String>,
    /// Whether processing was blocked (permission denied).
    blocked: bool,
    /// Current retry attempt counter.
    attempt: u32,
    /// Whether compaction is needed.
    needs_compaction: bool,
    /// Session ID.
    session_id: String,
    /// Assistant message ID.
    _assistant_message_id: String,
    /// Accumulated cost.
    cost: f64,
    /// Latest token usage.
    tokens: Option<MessageUsage>,
    /// Finish reason from the last step.
    finish_reason: Option<String>,
    /// Error from the stream, if any.
    error: Option<MessageError>,
    /// All completed tool parts for doom-loop detection.
    completed_tool_parts: Vec<CompletedToolRecord>,
    /// Patches computed from snapshot diffs between step-start and step-finish.
    pending_patches: Vec<SnapshotPatch>,
    /// Non-terminal processor events for the caller.
    pending_events: Vec<ProcessorEvent>,
    /// Compaction policy engine (uses model-specific limits).
    compaction_engine: CompactionEngine,
    /// Resolved model limits for the active stream.
    model_limits: Option<ModelLimits>,
}

/// Minimal record for doom-loop detection.
#[derive(Debug, Clone)]
struct CompletedToolRecord {
    tool_name: String,
    input: serde_json::Value,
}

pub struct LlmProcessor;

impl StreamProcessor {
    /// Create a new stream processor for a given assistant message.
    pub fn new(session_id: String, assistant_message_id: String) -> Self {
        Self {
            reasoning_map: HashMap::new(),
            tool_calls: HashMap::new(),
            current_text: None,
            snapshot: None,
            blocked: false,
            attempt: 0,
            needs_compaction: false,
            session_id,
            _assistant_message_id: assistant_message_id,
            cost: 0.0,
            tokens: None,
            finish_reason: None,
            error: None,
            completed_tool_parts: Vec::new(),
            pending_patches: Vec::new(),
            pending_events: Vec::new(),
            compaction_engine: CompactionEngine::new(CompactionConfig::default()),
            model_limits: None,
        }
    }

    /// Get the accumulated cost.
    pub fn cost(&self) -> f64 {
        self.cost
    }

    /// Get the latest token usage.
    pub fn tokens(&self) -> Option<&MessageUsage> {
        self.tokens.as_ref()
    }

    /// Get the finish reason.
    pub fn finish_reason(&self) -> Option<&str> {
        self.finish_reason.as_deref()
    }

    /// Get the error, if any.
    pub fn error(&self) -> Option<&MessageError> {
        self.error.as_ref()
    }

    /// Drain all pending patches computed from snapshot diffs.
    pub fn take_patches(&mut self) -> Vec<SnapshotPatch> {
        std::mem::take(&mut self.pending_patches)
    }

    /// Drain processor-side events (e.g. doom-loop detection).
    pub fn take_events(&mut self) -> Vec<ProcessorEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// Look up a tool part by its call ID.
    pub fn part_from_tool_call(&self, tool_call_id: &str) -> Option<&ToolCallState> {
        self.tool_calls.get(tool_call_id)
    }

    fn record_tool_call_for_doom_loop(&mut self, tool_name: String, input: serde_json::Value) {
        self.completed_tool_parts.push(CompletedToolRecord {
            tool_name: tool_name.clone(),
            input: input.clone(),
        });

        if self.completed_tool_parts.len() < DOOM_LOOP_THRESHOLD {
            return;
        }

        let recent =
            &self.completed_tool_parts[self.completed_tool_parts.len() - DOOM_LOOP_THRESHOLD..];
        let all_same = recent.iter().all(|r| {
            r.tool_name == tool_name
                && serde_json::to_string(&r.input).ok() == serde_json::to_string(&input).ok()
        });

        if all_same {
            tracing::warn!(
                session_id = %self.session_id,
                tool = %tool_name,
                "Doom loop detected: last {} tool calls are identical",
                DOOM_LOOP_THRESHOLD
            );
            self.pending_events
                .push(ProcessorEvent::DoomLoopDetected { tool_name, input });
            self.blocked = true;
        }
    }

    fn should_block_for_tool_error(kind: Option<StreamToolErrorKind>, error: &str) -> bool {
        matches!(
            classify_tool_error(kind, error),
            ToolExecutionErrorKind::PermissionDenied | ToolExecutionErrorKind::QuestionRejected
        )
    }

    /// Run the full stream processing loop with retry support.
    /// Returns `ProcessResult` matching the TS "compact" | "stop" | "continue".
    pub async fn process(
        &mut self,
        stream_input: StreamInput,
        provider: Arc<dyn Provider>,
    ) -> ProcessResult {
        self.needs_compaction = false;
        self.model_limits = Some(resolve_model_limits(&stream_input, provider.as_ref()));

        loop {
            match self.process_stream(&stream_input, provider.clone()).await {
                Ok(()) => {}
                Err(e) => {
                    tracing::error!(
                        session_id = %self.session_id,
                        error = %e,
                        "Stream processing error"
                    );

                    let error =
                        crate::message_v2::error_from_anyhow(e, &stream_input.model.provider_id);

                    let retry_msg = retry::retryable(&error);
                    if let Some(retry_message) = retry_msg {
                        self.attempt += 1;

                        let api_info = match &error {
                            MessageError::ApiError {
                                message,
                                is_retryable,
                                response_headers,
                                response_body,
                                ..
                            } => Some(ApiErrorInfo {
                                message: message.clone(),
                                is_retryable: *is_retryable,
                                response_headers: response_headers.clone(),
                                response_body: response_body.clone(),
                            }),
                            _ => None,
                        };

                        let delay_ms = retry::delay(self.attempt, api_info.as_ref());

                        tracing::info!(
                            session_id = %self.session_id,
                            attempt = self.attempt,
                            delay_ms = delay_ms,
                            message = %retry_message,
                            "Retrying after error"
                        );

                        // Sleep with cancellation support
                        let _ =
                            retry::sleep_with_cancel(delay_ms, stream_input.abort.clone()).await;

                        continue;
                    }

                    self.error = Some(error);
                }
            }

            // After stream ends (success or non-retryable error):
            // Flush any pending snapshot that wasn't closed by a FinishStep event.
            // This mirrors TS processor.ts lines 379-392 where, after the stream
            // loop exits (e.g. due to error or abort), any remaining snapshot is
            // diffed and emitted as a patch.
            self.flush_pending_snapshot(stream_input.work_dir.as_deref());

            // Mark any incomplete tool parts as error (abort handling)
            self.abort_incomplete_tools();

            if self.needs_compaction {
                return ProcessResult::Compact;
            }
            if self.blocked || self.error.is_some() {
                return ProcessResult::Stop;
            }
            return ProcessResult::Continue;
        }
    }

    /// Process a single stream attempt.
    async fn process_stream(
        &mut self,
        input: &StreamInput,
        provider: Arc<dyn Provider>,
    ) -> anyhow::Result<()> {
        let mut output = LlmProcessor::stream_from_input(input, provider).await?;

        while let Some(event) = output.events.recv().await {
            if input.abort.is_cancelled() {
                return Err(anyhow::anyhow!("Stream aborted"));
            }

            match event {
                StreamEvent::Start => {
                    // Session is now busy — already set by caller
                }

                // ============================================================
                // Reasoning events
                // ============================================================
                StreamEvent::ReasoningStart { id } => {
                    if self.reasoning_map.contains_key(&id) {
                        continue;
                    }
                    let now = chrono::Utc::now().timestamp_millis();
                    let part_id =
                        opencode_core::id::create(opencode_core::id::Prefix::Part, true, None);
                    self.reasoning_map.insert(
                        id,
                        ReasoningState {
                            _part_id: part_id,
                            text: String::new(),
                            _start_time: now,
                            _metadata: None,
                        },
                    );
                }

                StreamEvent::ReasoningDelta { id, text } => {
                    if let Some(state) = self.reasoning_map.get_mut(&id) {
                        state.text.push_str(&text);
                    }
                }

                StreamEvent::ReasoningEnd { id } => {
                    if let Some(mut state) = self.reasoning_map.remove(&id) {
                        state.text = state.text.trim_end().to_string();
                    }
                }

                // ============================================================
                // Tool input streaming events
                // ============================================================
                StreamEvent::ToolInputStart { id, tool_name } => {
                    let part_id = self
                        .tool_calls
                        .get(&id)
                        .map(|tc| tc.part_id.clone())
                        .unwrap_or_else(|| {
                            opencode_core::id::create(opencode_core::id::Prefix::Part, true, None)
                        });

                    self.tool_calls.insert(
                        id.clone(),
                        ToolCallState {
                            part_id,
                            tool_name,
                            _call_id: id,
                            raw_input: String::new(),
                            parsed_input: serde_json::json!({}),
                            state: ToolState::Pending {
                                input: serde_json::json!({}),
                                raw: String::new(),
                            },
                        },
                    );
                }

                StreamEvent::ToolInputDelta { id, delta } => {
                    if let Some(tc) = self.tool_calls.get_mut(&id) {
                        tc.raw_input.push_str(&delta);
                    }
                }

                StreamEvent::ToolInputEnd { id: _ } => {
                    // Input fully received; the ToolCallEnd event will finalize
                }

                // ============================================================
                // Tool call lifecycle (full call assembled)
                // ============================================================
                StreamEvent::ToolCallStart { id, name } => {
                    let part_id = self
                        .tool_calls
                        .get(&id)
                        .map(|tc| tc.part_id.clone())
                        .unwrap_or_else(|| {
                            opencode_core::id::create(opencode_core::id::Prefix::Part, true, None)
                        });

                    self.tool_calls.insert(
                        id.clone(),
                        ToolCallState {
                            part_id,
                            tool_name: name,
                            _call_id: id,
                            raw_input: String::new(),
                            parsed_input: serde_json::json!({}),
                            state: ToolState::Pending {
                                input: serde_json::json!({}),
                                raw: String::new(),
                            },
                        },
                    );
                }

                StreamEvent::ToolCallDelta { id, input } => {
                    if let Some(tc) = self.tool_calls.get_mut(&id) {
                        tc.raw_input.push_str(&input);
                    }
                }

                StreamEvent::ToolCallEnd { id, name, input } => {
                    let now = chrono::Utc::now().timestamp_millis();

                    if let Some(tc) = self.tool_calls.get_mut(&id) {
                        tc.tool_name = name.clone();
                        tc.parsed_input = input.clone();
                        tc.state = ToolState::Running {
                            input: input.clone(),
                            title: None,
                            metadata: None,
                            time: RunningTime { start: now },
                        };
                    }
                    self.record_tool_call_for_doom_loop(name, input);
                }

                // ============================================================
                // Tool result / error
                // ============================================================
                StreamEvent::ToolResult {
                    tool_call_id,
                    output,
                    input,
                    ..
                } => {
                    let now = chrono::Utc::now().timestamp_millis();
                    if let Some(tc) = self.tool_calls.get_mut(&tool_call_id) {
                        if matches!(tc.state, ToolState::Running { .. }) {
                            let start_time = match &tc.state {
                                ToolState::Running { time, .. } => time.start,
                                _ => now,
                            };
                            tc.state = ToolState::Completed {
                                input: input.unwrap_or_else(|| tc.parsed_input.clone()),
                                output: output.output,
                                title: output.title,
                                metadata: output.metadata,
                                time: CompletedTime {
                                    start: start_time,
                                    end: now,
                                    compacted: None,
                                },
                                attachments: None,
                            };
                        }
                    }
                }

                StreamEvent::ToolError {
                    tool_call_id,
                    error,
                    input,
                    kind,
                    ..
                } => {
                    let now = chrono::Utc::now().timestamp_millis();
                    if let Some(tc) = self.tool_calls.get_mut(&tool_call_id) {
                        if matches!(tc.state, ToolState::Running { .. }) {
                            let start_time = match &tc.state {
                                ToolState::Running { time, .. } => time.start,
                                _ => now,
                            };
                            tc.state = ToolState::Error {
                                input: input.unwrap_or_else(|| tc.parsed_input.clone()),
                                error: error.clone(),
                                metadata: None,
                                time: ErrorTime {
                                    start: start_time,
                                    end: now,
                                },
                            };

                            if Self::should_block_for_tool_error(kind, &error) {
                                self.blocked = true;
                            }
                        }
                    }
                }

                // ============================================================
                // Text events
                // ============================================================
                StreamEvent::TextStart => {
                    let now = chrono::Utc::now().timestamp_millis();
                    let part_id =
                        opencode_core::id::create(opencode_core::id::Prefix::Part, true, None);
                    self.current_text = Some(TextState {
                        _part_id: part_id,
                        text: String::new(),
                        _start_time: now,
                        _metadata: None,
                    });
                }

                StreamEvent::TextDelta(delta) => {
                    if let Some(ref mut text_state) = self.current_text {
                        text_state.text.push_str(&delta);
                    }
                }

                StreamEvent::TextEnd => {
                    if let Some(mut text_state) = self.current_text.take() {
                        text_state.text = text_state.text.trim_end().to_string();

                        // Plugin hook: experimental.text.complete
                        let hook_outputs = opencode_plugin::trigger_collect(
                            HookContext::new(HookEvent::TextComplete)
                                .with_session(&self.session_id)
                                .with_data(
                                    "message_id",
                                    serde_json::json!(&self._assistant_message_id),
                                )
                                .with_data("part_id", serde_json::json!(&text_state._part_id))
                                .with_data("text", serde_json::json!(&text_state.text)),
                        )
                        .await;
                        for output in hook_outputs {
                            let Some(payload) = output.payload.as_ref() else {
                                continue;
                            };
                            if let Some(next_text) = extract_text_from_hook_payload(payload) {
                                text_state.text = next_text;
                            }
                        }
                    }
                }

                // ============================================================
                // Step events
                // ============================================================
                StreamEvent::StartStep => {
                    // Track snapshot for diff computation
                    if let Some(ref dir) = input.work_dir {
                        match Snapshot::track(dir) {
                            Ok(hash) => {
                                self.snapshot = Some(hash);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    session_id = %self.session_id,
                                    error = %e,
                                    "Failed to track snapshot at step-start"
                                );
                            }
                        }
                    }
                }

                StreamEvent::FinishStep {
                    finish_reason,
                    usage,
                    provider_metadata,
                    ..
                } => {
                    let step_usage = usage_from_stream(
                        &usage,
                        provider_metadata.as_ref(),
                        &input.model.provider_id,
                    );
                    let step_cost = compute_cost(
                        &step_usage,
                        input.cost_rates.as_ref(),
                        &input.model.provider_id,
                    );

                    self.cost += step_cost;
                    let mut usage_with_cost = step_usage.clone();
                    usage_with_cost.total_cost = step_cost;
                    self.tokens = Some(usage_with_cost.clone());
                    self.finish_reason = finish_reason;

                    // Check compaction using model-specific context limits.
                    if let Some(ref limits) = self.model_limits {
                        let token_usage = token_usage_from_message_usage(&usage_with_cost);
                        if self.compaction_engine.is_overflow(&token_usage, limits) {
                            self.needs_compaction = true;
                        }
                    }

                    // Compute patch from snapshot diff (matching TS Snapshot.patch)
                    if let Some(ref snapshot_hash) = self.snapshot {
                        if let Some(ref dir) = input.work_dir {
                            match Snapshot::diff(dir, snapshot_hash) {
                                Ok(diffs) => {
                                    let files: Vec<String> =
                                        diffs.into_iter().map(|d| d.path).collect();
                                    if !files.is_empty() {
                                        self.pending_patches.push(SnapshotPatch {
                                            hash: snapshot_hash.clone(),
                                            files,
                                        });
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        session_id = %self.session_id,
                                        error = %e,
                                        "Failed to compute snapshot patch"
                                    );
                                }
                            }
                        }
                    }

                    self.snapshot = None;
                }

                // ============================================================
                // Terminal events
                // ============================================================
                StreamEvent::Finish => {}

                StreamEvent::Done => {
                    break;
                }

                StreamEvent::Error(e) => {
                    return Err(anyhow::anyhow!("Stream error: {}", e));
                }

                StreamEvent::Usage { .. } => {}
            }

            if self.needs_compaction {
                break;
            }
        }

        Ok(())
    }

    /// Flush any pending snapshot that wasn't closed by a `FinishStep` event.
    ///
    /// When the stream ends abruptly (error, abort, or missing `finish-step`),
    /// there may be a snapshot from `StartStep` that was never diffed. This
    /// mirrors the TS processor.ts post-loop logic (lines 379-392) that
    /// computes the final patch if `snapshot` is still set.
    fn flush_pending_snapshot(&mut self, work_dir: Option<&Path>) {
        let snapshot_hash = match self.snapshot.take() {
            Some(h) => h,
            None => return,
        };
        let dir = match work_dir {
            Some(d) => d,
            None => return,
        };

        match Snapshot::diff(dir, &snapshot_hash) {
            Ok(diffs) => {
                let files: Vec<String> = diffs.into_iter().map(|d| d.path).collect();
                if !files.is_empty() {
                    self.pending_patches.push(SnapshotPatch {
                        hash: snapshot_hash,
                        files,
                    });
                }
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %self.session_id,
                    error = %e,
                    "Failed to compute snapshot patch during flush"
                );
            }
        }
    }

    /// Mark any incomplete tool parts as "error" with "Tool execution aborted".
    /// Mirrors the TS abort handling at the end of the process loop.
    fn abort_incomplete_tools(&mut self) {
        let now = chrono::Utc::now().timestamp_millis();
        for tc in self.tool_calls.values_mut() {
            match &tc.state {
                ToolState::Completed { .. } | ToolState::Error { .. } => {
                    // Already terminal — skip
                }
                _ => {
                    tc.state = ToolState::Error {
                        input: tc.parsed_input.clone(),
                        error: "Tool execution aborted".to_string(),
                        metadata: None,
                        time: ErrorTime {
                            start: now,
                            end: now,
                        },
                    };
                }
            }
        }
    }
}

impl LlmProcessor {
    /// Start a stream from a `StreamInput` and provider.
    pub async fn stream(
        input: StreamInput,
        provider: Arc<dyn Provider>,
    ) -> anyhow::Result<StreamOutput> {
        Self::stream_from_input(&input, provider).await
    }

    /// Internal: build the request and start streaming.
    async fn stream_from_input(
        input: &StreamInput,
        provider: Arc<dyn Provider>,
    ) -> anyhow::Result<StreamOutput> {
        let session_id = input.session_id.clone();
        let model_id = input.model.model_id.clone();
        let provider_id = input.model.provider_id.clone();
        let agent_name = input.agent.name.clone();

        tracing::info!(
            session_id = %session_id,
            model_id = %model_id,
            provider_id = %provider_id,
            agent = %agent_name,
            "Starting LLM stream"
        );

        let mut system_prompt = build_system_prompt(input);
        let system_header_before_hooks = system_prompt.first().cloned().unwrap_or_default();

        // Plugin hook: chat.system.transform — let plugins modify the system prompt
        let system_hook_outputs = opencode_plugin::trigger_collect(
            HookContext::new(HookEvent::ChatSystemTransform)
                .with_session(&session_id)
                .with_data("model_id", serde_json::json!(&model_id))
                .with_data("provider_id", serde_json::json!(&provider_id))
                .with_data("system", serde_json::json!(&system_prompt)),
        )
        .await;
        apply_chat_system_hook_outputs(&mut system_prompt, system_hook_outputs);
        rejoin_system_prompt_if_needed(&mut system_prompt, &system_header_before_hooks);

        let mut tools = resolve_tools(input.tools.clone(), &input.agent);
        // Inject LiteLLM dummy tool if needed
        inject_litellm_dummy_tool(&mut tools, &input.messages, &input.model.provider_id);

        let mut max_tokens = if input.small {
            Some(1024u64)
        } else {
            Some(input.agent.max_tokens.unwrap_or(OUTPUT_TOKEN_MAX))
        };

        let mut temperature = if input.small {
            Some(0.5f32)
        } else {
            input.agent.temperature
        };
        let mut top_p = input.agent.top_p;
        let mut provider_options: HashMap<String, serde_json::Value> = HashMap::new();

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Plugin hook: chat.params — let plugins modify LLM request parameters
        let params_hook_outputs = opencode_plugin::trigger_collect(
            HookContext::new(HookEvent::ChatParams)
                .with_session(&session_id)
                .with_data("model_id", serde_json::json!(&model_id))
                .with_data("provider_id", serde_json::json!(&provider_id))
                .with_data("max_tokens", serde_json::json!(max_tokens))
                .with_data("temperature", serde_json::json!(temperature))
                .with_data("top_p", serde_json::json!(top_p))
                .with_data("agent", serde_json::json!(&agent_name))
                .with_data("options", serde_json::json!(&provider_options)),
        )
        .await;
        apply_chat_params_hook_outputs(
            &mut max_tokens,
            &mut temperature,
            &mut top_p,
            &mut provider_options,
            params_hook_outputs,
        );

        // Plugin hook: chat.headers — let plugins inject custom HTTP headers
        let header_hook_outputs = opencode_plugin::trigger_collect(
            HookContext::new(HookEvent::ChatHeaders)
                .with_session(&session_id)
                .with_data("model_id", serde_json::json!(&model_id))
                .with_data("provider_id", serde_json::json!(&provider_id))
                .with_data("headers", serde_json::json!({})),
        )
        .await;
        let plugin_headers = collect_chat_header_hook_outputs(header_hook_outputs);
        if !plugin_headers.is_empty() {
            provider_options.insert("headers".to_string(), serde_json::json!(plugin_headers));
        }

        let request = ChatRequest {
            model: model_id,
            messages: build_messages(&system_prompt, &input.messages),
            max_tokens,
            temperature,
            system: None,
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.into_values().collect())
            },
            stream: Some(true),
            top_p,
            variant: None,
            session_id: Some(session_id.clone()),
            provider_options: if provider_options.is_empty() {
                None
            } else {
                Some(provider_options)
            },
        };

        let abort = input.abort.clone();

        tokio::spawn(async move {
            match provider.chat_stream(request).await {
                Ok(mut stream) => {
                    while let Some(event_result) = stream.next().await {
                        if abort.is_cancelled() {
                            tracing::info!("LLM stream cancelled");
                            break;
                        }

                        match event_result {
                            Ok(event) => {
                                if tx.send(event).await.is_err() {
                                    tracing::warn!("Stream receiver dropped");
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::error!("Stream error: {}", e);
                                let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to start stream: {}", e);
                    let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                }
            }
        });

        Ok(StreamOutput { events: rx })
    }

    pub fn has_tool_calls(messages: &[Message]) -> bool {
        for msg in messages {
            if let Content::Parts(parts) = &msg.content {
                for part in parts {
                    if part.tool_use.is_some() || part.tool_result.is_some() {
                        return true;
                    }
                }
            }
        }
        false
    }
}

fn build_system_prompt(input: &StreamInput) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();

    // Use agent prompt if available, otherwise fall back to system prompts
    if let Some(ref prompt) = input.agent.system_prompt {
        parts.push(prompt.clone());
    }

    // Any custom prompts passed into this call
    parts.extend(input.system.clone());

    // Any custom prompt from the last user message
    if let MessageInfo::User {
        system: Some(ref user_system),
        ..
    } = input.user
    {
        parts.push(user_system.clone());
    }

    // Filter empty strings and join into a single system block
    let joined: String = parts
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    let mut system = vec![joined];

    // Rejoin logic: maintain 2-part structure for caching.
    // If plugins added extra parts and the header is unchanged,
    // collapse everything after the header into a single second part.
    let header = system.first().cloned().unwrap_or_default();
    if system.len() > 2 && system.first().map(|s| s.as_str()) == Some(header.as_str()) {
        let rest: String = system[1..].join("\n");
        system = vec![header, rest];
    }

    system
}

fn hook_payload_object(
    payload: &serde_json::Value,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    payload
        .get("output")
        .and_then(|value| value.as_object())
        .or_else(|| payload.as_object())
        .or_else(|| payload.get("data").and_then(|value| value.as_object()))
}

fn parse_string_array(value: &serde_json::Value) -> Option<Vec<String>> {
    value.as_array().map(|items| {
        items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect()
    })
}

fn apply_chat_system_hook_outputs(
    system_prompt: &mut Vec<String>,
    hook_outputs: Vec<opencode_plugin::HookOutput>,
) {
    for output in hook_outputs {
        let Some(payload) = output.payload.as_ref() else {
            continue;
        };
        let Some(object) = hook_payload_object(payload) else {
            continue;
        };
        let Some(system) = object.get("system").and_then(parse_string_array) else {
            continue;
        };
        *system_prompt = system;
    }
}

fn rejoin_system_prompt_if_needed(system_prompt: &mut Vec<String>, original_header: &str) {
    if system_prompt.len() > 2
        && system_prompt.first().map(|s| s.as_str()) == Some(original_header)
    {
        let header = system_prompt.first().cloned().unwrap_or_default();
        let rest = system_prompt[1..].join("\n");
        *system_prompt = vec![header, rest];
    }
}

fn value_to_f32(value: &serde_json::Value) -> Option<f32> {
    value.as_f64().map(|v| v as f32)
}

fn apply_chat_params_hook_outputs(
    max_tokens: &mut Option<u64>,
    temperature: &mut Option<f32>,
    top_p: &mut Option<f32>,
    provider_options: &mut HashMap<String, serde_json::Value>,
    hook_outputs: Vec<opencode_plugin::HookOutput>,
) {
    for output in hook_outputs {
        let Some(payload) = output.payload.as_ref() else {
            continue;
        };
        let Some(object) = hook_payload_object(payload) else {
            continue;
        };

        if let Some(next_max_tokens) = object
            .get("maxTokens")
            .or_else(|| object.get("max_tokens"))
            .and_then(|value| value.as_u64())
        {
            *max_tokens = Some(next_max_tokens);
        }
        if let Some(next_temperature) = object.get("temperature").and_then(value_to_f32) {
            *temperature = Some(next_temperature);
        }
        if let Some(next_top_p) = object
            .get("topP")
            .or_else(|| object.get("top_p"))
            .and_then(value_to_f32)
        {
            *top_p = Some(next_top_p);
        }
        if let Some(top_k) = object
            .get("topK")
            .or_else(|| object.get("top_k"))
            .and_then(value_to_f32)
        {
            provider_options.insert("topK".to_string(), serde_json::json!(top_k));
        }
        if let Some(options) = object.get("options").and_then(|value| value.as_object()) {
            for (key, value) in options {
                provider_options.insert(key.clone(), value.clone());
            }
        }
        if let Some(options) = object
            .get("provider_options")
            .or_else(|| object.get("providerOptions"))
            .and_then(|value| value.as_object())
        {
            for (key, value) in options {
                provider_options.insert(key.clone(), value.clone());
            }
        }
    }
}

fn collect_chat_header_hook_outputs(
    hook_outputs: Vec<opencode_plugin::HookOutput>,
) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for output in hook_outputs {
        let Some(payload) = output.payload.as_ref() else {
            continue;
        };
        let Some(object) = hook_payload_object(payload) else {
            continue;
        };
        let Some(incoming) = object.get("headers").and_then(|value| value.as_object()) else {
            continue;
        };
        for (key, value) in incoming {
            if let Some(value_str) = value.as_str() {
                headers.insert(key.clone(), value_str.to_string());
            }
        }
    }
    headers
}

fn extract_text_from_hook_payload(payload: &serde_json::Value) -> Option<String> {
    hook_payload_object(payload)
        .and_then(|object| object.get("text"))
        .and_then(|value| value.as_str())
        .map(|text| text.to_string())
}

fn resolve_model_limits(input: &StreamInput, provider: &dyn Provider) -> ModelLimits {
    let model = provider.get_model(&input.model.model_id);
    ModelLimits {
        context: model
            .map(|info| info.context_window)
            .unwrap_or_else(|| get_model_context_limit(&input.model.model_id)),
        max_input: None,
        max_output: input
            .agent
            .max_tokens
            .or_else(|| model.map(|info| info.max_output_tokens))
            .unwrap_or(OUTPUT_TOKEN_MAX),
    }
}

fn token_usage_from_message_usage(usage: &MessageUsage) -> TokenUsage {
    TokenUsage {
        input: usage.input_tokens,
        output: usage.output_tokens + usage.reasoning_tokens,
        cache_read: usage.cache_read_tokens,
        cache_write: usage.cache_write_tokens,
        total: usage.input_tokens
            + usage.output_tokens
            + usage.reasoning_tokens
            + usage.cache_read_tokens
            + usage.cache_write_tokens,
    }
}

/// Convert `StreamUsage` from the provider into session-level `MessageUsage`.
fn usage_from_stream(
    usage: &StreamUsage,
    provider_metadata: Option<&serde_json::Value>,
    provider_id: &str,
) -> MessageUsage {
    let (meta_cache_read, meta_cache_write) =
        extract_cache_tokens_from_metadata(provider_metadata, provider_id);
    let cache_read_tokens = usage.cache_read_tokens.max(meta_cache_read);
    let cache_write_tokens = usage.cache_write_tokens.max(meta_cache_write);

    MessageUsage {
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        // Anthropic/Bedrock `input_tokens` do not include cached usage; keep cache
        // tokens explicit so total usage and compaction logic can account for them.
        cache_write_tokens,
        cache_read_tokens,
        total_cost: 0.0, // Cost is computed separately
    }
}

/// Compute cost from stream usage using model-specific pricing rates.
/// Falls back to Claude-like pricing ($3/$15 per million) if no rates provided.
fn compute_cost(usage: &MessageUsage, rates: Option<&CostRates>, provider_id: &str) -> f64 {
    let rates = rates.cloned().unwrap_or(CostRates {
        input: 3.0,
        output: 15.0,
        cache_read: 0.3,
        cache_write: 3.75,
        over_200k: None,
    });

    let total_tokens = usage.input_tokens
        + usage.output_tokens
        + usage.reasoning_tokens
        + usage.cache_read_tokens
        + usage.cache_write_tokens;
    let use_over_200k = total_tokens > 200_000;

    let (input_rate, output_rate, cache_read_rate, cache_write_rate) = if use_over_200k {
        if let Some(tier) = rates.over_200k.as_ref() {
            (tier.input, tier.output, tier.cache_read, tier.cache_write)
        } else {
            (
                rates.input,
                rates.output,
                rates.cache_read,
                rates.cache_write,
            )
        }
    } else {
        (
            rates.input,
            rates.output,
            rates.cache_read,
            rates.cache_write,
        )
    };

    // Keep input billing resilient for providers where cache tokens may be folded into input.
    let billable_input_tokens = if is_anthropic_or_bedrock_provider(provider_id) {
        usage
            .input_tokens
            .saturating_sub(usage.cache_read_tokens + usage.cache_write_tokens)
    } else {
        usage.input_tokens
    };

    // Reasoning tokens are billed at output-token rates.
    let billable_output_tokens = usage.output_tokens + usage.reasoning_tokens;

    let input_cost = (billable_input_tokens as f64) * input_rate / 1_000_000.0;
    let output_cost = (billable_output_tokens as f64) * output_rate / 1_000_000.0;
    let cache_read_cost = (usage.cache_read_tokens as f64) * cache_read_rate / 1_000_000.0;
    let cache_write_cost = (usage.cache_write_tokens as f64) * cache_write_rate / 1_000_000.0;

    input_cost + output_cost + cache_read_cost + cache_write_cost
}

fn is_anthropic_or_bedrock_provider(provider_id: &str) -> bool {
    let lower = provider_id.to_ascii_lowercase();
    lower.contains("anthropic") || lower.contains("bedrock")
}

fn metadata_u64(metadata: &serde_json::Value, pointer: &str) -> Option<u64> {
    metadata.pointer(pointer).and_then(|v| v.as_u64())
}

fn extract_cache_tokens_from_metadata(
    provider_metadata: Option<&serde_json::Value>,
    provider_id: &str,
) -> (u64, u64) {
    if !is_anthropic_or_bedrock_provider(provider_id) {
        return (0, 0);
    }

    let Some(metadata) = provider_metadata else {
        return (0, 0);
    };

    let cache_write_tokens = metadata_u64(metadata, "/anthropic/cacheCreationInputTokens")
        .or_else(|| metadata_u64(metadata, "/cacheCreationInputTokens"))
        .or_else(|| metadata_u64(metadata, "/bedrock/cacheCreationInputTokens"))
        .or_else(|| metadata_u64(metadata, "/amazon-bedrock/cacheCreationInputTokens"))
        .or_else(|| metadata_u64(metadata, "/usage/cacheCreationInputTokens"))
        .unwrap_or(0);

    let cache_read_tokens = metadata_u64(metadata, "/anthropic/cacheReadInputTokens")
        .or_else(|| metadata_u64(metadata, "/cacheReadInputTokens"))
        .or_else(|| metadata_u64(metadata, "/bedrock/cacheReadInputTokens"))
        .or_else(|| metadata_u64(metadata, "/amazon-bedrock/cacheReadInputTokens"))
        .or_else(|| metadata_u64(metadata, "/usage/cacheReadInputTokens"))
        .unwrap_or(0);

    (cache_read_tokens, cache_write_tokens)
}

fn classify_tool_error(kind: Option<StreamToolErrorKind>, error: &str) -> ToolExecutionErrorKind {
    if let Some(kind) = kind {
        return match kind {
            StreamToolErrorKind::PermissionDenied => ToolExecutionErrorKind::PermissionDenied,
            StreamToolErrorKind::QuestionRejected => ToolExecutionErrorKind::QuestionRejected,
            StreamToolErrorKind::ExecutionError => ToolExecutionErrorKind::Other,
        };
    }

    let lower = error.to_ascii_lowercase();
    if lower.starts_with("permission denied:") || lower.contains("permission denied") {
        return ToolExecutionErrorKind::PermissionDenied;
    }
    if lower.starts_with("question rejected:") || lower.contains("question rejected") {
        return ToolExecutionErrorKind::QuestionRejected;
    }
    ToolExecutionErrorKind::Other
}

fn build_messages(system: &[String], user_messages: &[Message]) -> Vec<Message> {
    let mut messages: Vec<Message> = system.iter().map(|s| Message::system(s)).collect();

    messages.extend(user_messages.iter().cloned());

    messages
}

fn resolve_tools(
    tools: HashMap<String, ToolDefinition>,
    _agent: &LlmAgent,
) -> HashMap<String, ToolDefinition> {
    tools
}

/// Repair a tool call by trying case-insensitive matching.
/// If the tool name doesn't match any known tool, try lowercase.
/// If still no match, return "invalid" as the tool name.
pub fn repair_tool_call(name: &str, tools: &HashMap<String, ToolDefinition>) -> String {
    // Exact match
    if tools.contains_key(name) {
        return name.to_string();
    }

    // Try lowercase
    let lower = name.to_lowercase();
    if lower != name && tools.contains_key(&lower) {
        tracing::info!(
            original = name,
            repaired = %lower,
            "Repairing tool call: case mismatch"
        );
        return lower;
    }

    // No match - return "invalid"
    tracing::warn!(tool_name = name, "Unknown tool call, mapping to 'invalid'");
    "invalid".to_string()
}

/// Inject a dummy tool for LiteLLM proxy compatibility.
/// LiteLLM and some Anthropic proxies require the tools parameter to be present
/// when message history contains tool calls, even if no tools are being used.
pub fn inject_litellm_dummy_tool(
    tools: &mut HashMap<String, ToolDefinition>,
    messages: &[Message],
    provider_id: &str,
) {
    // Check if this is a LiteLLM proxy
    let is_litellm = provider_id.to_lowercase().contains("litellm");

    if !is_litellm {
        return;
    }

    // Only inject if tools is empty and messages contain tool calls
    if !tools.is_empty() {
        return;
    }

    if !LlmProcessor::has_tool_calls(messages) {
        return;
    }

    tracing::info!("Injecting LiteLLM dummy tool for proxy compatibility");

    tools.insert(
        "_noop".to_string(),
        ToolDefinition {
            name: "_noop".to_string(),
            description: Some(
                "Placeholder for LiteLLM/Anthropic proxy compatibility - required when message history contains tool calls but no active tools are needed"
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
    );
}

/// Resolve tools with permission filtering.
/// Removes tools that are disabled by the agent's permission configuration.
pub fn resolve_tools_with_permissions(
    mut tools: HashMap<String, ToolDefinition>,
    disabled_tools: &[String],
    user_tool_overrides: Option<&HashMap<String, bool>>,
) -> HashMap<String, ToolDefinition> {
    // Remove disabled tools
    for tool_name in disabled_tools {
        tools.remove(tool_name);
    }

    // Apply user-level tool overrides
    if let Some(overrides) = user_tool_overrides {
        for (tool_name, enabled) in overrides {
            if !enabled {
                tools.remove(tool_name);
            }
        }
    }

    tools
}

pub struct StreamResult {
    pub text: String,
    pub tool_calls: Vec<ToolCallResult>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolCallResult {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

pub async fn collect_stream(
    mut output: StreamOutput,
    abort: tokio_util::sync::CancellationToken,
) -> anyhow::Result<StreamResult> {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut current_tool: Option<ToolCallResult> = None;
    let mut finish_reason = None;

    while let Some(event) = output.events.recv().await {
        if abort.is_cancelled() {
            break;
        }

        match event {
            StreamEvent::Start => {}
            StreamEvent::TextStart => {}
            StreamEvent::TextDelta(delta) => {
                text.push_str(&delta);
            }
            StreamEvent::TextEnd => {}
            StreamEvent::ReasoningStart { .. } => {}
            StreamEvent::ReasoningDelta { .. } => {}
            StreamEvent::ReasoningEnd { .. } => {}
            StreamEvent::ToolInputStart { id, tool_name } => {
                current_tool = Some(ToolCallResult {
                    id,
                    name: tool_name,
                    input: serde_json::Value::Null,
                });
            }
            StreamEvent::ToolInputDelta { delta, .. } => {
                if let Some(ref mut tool) = current_tool {
                    if tool.input.is_null() {
                        tool.input = serde_json::Value::String(delta);
                    } else if let Some(s) = tool.input.as_str() {
                        let mut existing = s.to_string();
                        existing.push_str(&delta);
                        tool.input = serde_json::Value::String(existing);
                    }
                }
            }
            StreamEvent::ToolInputEnd { .. } => {}
            StreamEvent::ToolCallStart { id, name } => {
                current_tool = Some(ToolCallResult {
                    id,
                    name,
                    input: serde_json::Value::Null,
                });
            }
            StreamEvent::ToolCallDelta { input, .. } => {
                if let Some(ref mut tool) = current_tool {
                    if tool.input.is_null() {
                        tool.input = serde_json::Value::String(input);
                    } else if let Some(s) = tool.input.as_str() {
                        let mut existing = s.to_string();
                        existing.push_str(&input);
                        tool.input = serde_json::Value::String(existing);
                    }
                }
            }
            StreamEvent::ToolCallEnd { id, name, input } => {
                tool_calls.push(ToolCallResult { id, name, input });
                current_tool = None;
            }
            StreamEvent::ToolResult { .. } => {}
            StreamEvent::ToolError { .. } => {}
            StreamEvent::StartStep => {}
            StreamEvent::FinishStep { .. } => {}
            StreamEvent::Finish => {}
            StreamEvent::Done => {
                finish_reason = Some("stop".to_string());
                break;
            }
            StreamEvent::Error(e) => {
                tracing::error!("Stream error: {}", e);
                break;
            }
            StreamEvent::Usage { .. } => {}
        }
    }

    Ok(StreamResult {
        text,
        tool_calls,
        finish_reason,
    })
}

/// Trigger the TextComplete plugin hook after stream collection.
/// Call this after `collect_stream` when you have the final text.
pub async fn notify_text_complete(session_id: &str, text: &str) {
    if !text.is_empty() {
        let _ = opencode_plugin::trigger_collect(
            HookContext::new(HookEvent::TextComplete)
                .with_session(session_id)
                .with_data("text", serde_json::json!(text)),
        )
        .await;
    }
}

pub fn to_model_messages(messages: &[MessageWithParts]) -> Vec<Message> {
    messages
        .iter()
        .filter_map(|msg| message_to_model(&msg.info, &msg.parts))
        .collect()
}

fn message_to_model(info: &MessageInfo, parts: &[Part]) -> Option<Message> {
    let role = match info {
        MessageInfo::User { .. } => opencode_provider::Role::User,
        MessageInfo::Assistant { .. } => opencode_provider::Role::Assistant,
    };

    let content = if parts.len() == 1 {
        match &parts[0] {
            Part::Text { text, .. } => Content::Text(text.clone()),
            _ => Content::Parts(parts.iter().filter_map(part_to_content_part).collect()),
        }
    } else {
        Content::Parts(parts.iter().filter_map(part_to_content_part).collect())
    };

    Some(Message {
        role,
        content,
        cache_control: None,
        provider_options: None,
    })
}

fn part_to_content_part(part: &Part) -> Option<opencode_provider::ContentPart> {
    match part {
        Part::Text { text, .. } => Some(opencode_provider::ContentPart {
            content_type: "text".to_string(),
            text: Some(text.clone()),
            image_url: None,
            tool_use: None,
            tool_result: None,
            cache_control: None,
            filename: None,
            media_type: None,
            provider_options: None,
        }),
        Part::File(file_part) => Some(opencode_provider::ContentPart {
            content_type: "file".to_string(),
            text: None,
            image_url: Some(ImageUrl {
                url: file_part.url.clone(),
            }),
            tool_use: None,
            tool_result: None,
            cache_control: None,
            filename: file_part.filename.clone(),
            media_type: Some(file_part.mime.clone()),
            provider_options: None,
        }),
        Part::Tool(tool_part) => {
            if let crate::ToolState::Completed { output, .. } = &tool_part.state {
                Some(opencode_provider::ContentPart {
                    content_type: "tool_result".to_string(),
                    text: Some(output.clone()),
                    image_url: None,
                    tool_use: None,
                    tool_result: Some(opencode_provider::ToolResult {
                        tool_use_id: tool_part.call_id.clone(),
                        content: output.clone(),
                        is_error: Some(false),
                    }),
                    cache_control: None,
                    filename: None,
                    media_type: None,
                    provider_options: None,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencode_provider::ToolDefinition;

    fn make_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: Some(format!("Test tool: {}", name)),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }
    }

    #[test]
    fn test_repair_tool_call_exact_match() {
        let mut tools = HashMap::new();
        tools.insert("read_file".to_string(), make_tool("read_file"));

        assert_eq!(repair_tool_call("read_file", &tools), "read_file");
    }

    #[test]
    fn test_repair_tool_call_case_mismatch() {
        let mut tools = HashMap::new();
        tools.insert("read_file".to_string(), make_tool("read_file"));

        assert_eq!(repair_tool_call("Read_File", &tools), "read_file");
    }

    #[test]
    fn test_repair_tool_call_unknown() {
        let tools = HashMap::new();
        assert_eq!(repair_tool_call("nonexistent", &tools), "invalid");
    }

    #[test]
    fn test_inject_litellm_dummy_tool_has_tools() {
        let mut tools = HashMap::new();
        tools.insert("existing".to_string(), make_tool("existing"));

        inject_litellm_dummy_tool(&mut tools, &[], "litellm-proxy");
        assert!(!tools.contains_key("_noop")); // Not injected because tools is non-empty
    }

    #[test]
    fn test_resolve_tools_with_permissions_filters() {
        let mut tools = HashMap::new();
        tools.insert("read_file".to_string(), make_tool("read_file"));
        tools.insert("write_file".to_string(), make_tool("write_file"));
        tools.insert("execute".to_string(), make_tool("execute"));

        let disabled = vec!["execute".to_string()];
        let mut overrides = HashMap::new();
        overrides.insert("write_file".to_string(), false);

        let result = resolve_tools_with_permissions(tools, &disabled, Some(&overrides));

        assert!(result.contains_key("read_file"));
        assert!(!result.contains_key("write_file"));
        assert!(!result.contains_key("execute"));
    }

    #[test]
    fn test_has_tool_calls_true() {
        let messages = vec![Message {
            role: opencode_provider::Role::Assistant,
            content: opencode_provider::Content::Parts(vec![opencode_provider::ContentPart {
                content_type: "tool_use".to_string(),
                text: None,
                image_url: None,
                tool_use: Some(opencode_provider::ToolUse {
                    id: "call_1".to_string(),
                    name: "test".to_string(),
                    input: serde_json::json!({}),
                }),
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            }]),
            cache_control: None,
            provider_options: None,
        }];
        assert!(LlmProcessor::has_tool_calls(&messages));
    }

    #[test]
    fn test_has_tool_calls_false() {
        let messages = vec![Message::user("hello")];
        assert!(!LlmProcessor::has_tool_calls(&messages));
    }

    #[test]
    fn test_inject_litellm_dummy_tool_empty_tools_with_tool_calls() {
        let mut tools = HashMap::new();
        let messages = vec![Message {
            role: opencode_provider::Role::Assistant,
            content: opencode_provider::Content::Parts(vec![opencode_provider::ContentPart {
                content_type: "tool_use".to_string(),
                text: None,
                image_url: None,
                tool_use: Some(opencode_provider::ToolUse {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    input: serde_json::json!({}),
                }),
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            }]),
            cache_control: None,
            provider_options: None,
        }];

        inject_litellm_dummy_tool(&mut tools, &messages, "litellm-proxy");
        assert!(tools.contains_key("_noop"));
    }

    #[test]
    fn test_inject_litellm_dummy_tool_non_litellm() {
        let mut tools = HashMap::new();
        let messages = vec![Message {
            role: opencode_provider::Role::Assistant,
            content: opencode_provider::Content::Parts(vec![opencode_provider::ContentPart {
                content_type: "tool_use".to_string(),
                text: None,
                image_url: None,
                tool_use: Some(opencode_provider::ToolUse {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    input: serde_json::json!({}),
                }),
                tool_result: None,
                cache_control: None,
                filename: None,
                media_type: None,
                provider_options: None,
            }]),
            cache_control: None,
            provider_options: None,
        }];

        inject_litellm_dummy_tool(&mut tools, &messages, "anthropic");
        assert!(!tools.contains_key("_noop"));
    }

    #[test]
    fn test_overflow_detection_uses_model_limits() {
        let usage = MessageUsage {
            input_tokens: 124_000,
            output_tokens: 2_000,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_cost: 0.0,
        };
        let limits = ModelLimits {
            context: 128_000,
            max_input: None,
            max_output: 4_096,
        };
        let token_usage = token_usage_from_message_usage(&usage);
        let engine = CompactionEngine::new(CompactionConfig::default());
        assert!(engine.is_overflow(&token_usage, &limits));
    }

    #[test]
    fn test_cost_includes_reasoning_tokens_at_output_rate() {
        let usage = MessageUsage {
            input_tokens: 1_000,
            output_tokens: 500,
            reasoning_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_cost: 0.0,
        };
        let rates = CostRates {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
            over_200k: None,
        };

        let cost = compute_cost(&usage, Some(&rates), "openai");
        let base_without_reasoning = ((usage.input_tokens as f64) * rates.input
            + (usage.output_tokens as f64) * rates.output)
            / 1_000_000.0;
        assert!(cost > base_without_reasoning);
    }

    #[test]
    fn test_anthropic_cache_tokens_extracted_from_metadata() {
        let metadata = serde_json::json!({
            "anthropic": {
                "cacheCreationInputTokens": 500,
                "cacheReadInputTokens": 300
            }
        });
        let usage = StreamUsage {
            prompt_tokens: 1_000,
            completion_tokens: 500,
            ..Default::default()
        };

        let normalized = usage_from_stream(&usage, Some(&metadata), "anthropic");
        assert_eq!(normalized.cache_write_tokens, 500);
        assert_eq!(normalized.cache_read_tokens, 300);
    }

    #[test]
    fn test_doom_loop_emits_event_instead_of_only_blocking() {
        let mut processor = StreamProcessor::new("session-1".into(), "assistant-1".into());
        let input = serde_json::json!({"path": "src/main.rs"});

        for _ in 0..DOOM_LOOP_THRESHOLD {
            processor.record_tool_call_for_doom_loop("read".to_string(), input.clone());
        }

        let events = processor.take_events();
        assert!(events.iter().any(|e| matches!(
            e,
            ProcessorEvent::DoomLoopDetected { tool_name, .. } if tool_name == "read"
        )));
        assert!(processor.blocked);
    }

    #[test]
    fn test_typed_tool_error_classification() {
        assert!(StreamProcessor::should_block_for_tool_error(
            Some(StreamToolErrorKind::PermissionDenied),
            "anything"
        ));
        assert!(StreamProcessor::should_block_for_tool_error(
            Some(StreamToolErrorKind::QuestionRejected),
            "anything"
        ));
        assert!(!StreamProcessor::should_block_for_tool_error(
            Some(StreamToolErrorKind::ExecutionError),
            "anything"
        ));
    }
}
