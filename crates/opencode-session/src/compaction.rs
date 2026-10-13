use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use opencode_core::bus::{Bus, BusEventDef};
use opencode_plugin::{HookContext, HookEvent};
use serde::{Deserialize, Serialize};

use crate::llm::{
    collect_stream, to_model_messages, LlmAgent, LlmModelRef, LlmProcessor, StreamInput,
};
use crate::message_v2::{
    AssistantTime, AssistantTokens, CacheTokens, CompletedTime, MessageInfo, MessagePath,
    MessageWithParts, ModelRef, Part, TextTime, ToolState, UserTime,
};
use opencode_provider::{Message, Provider};

const COMPACTION_BUFFER: u64 = 20_000;
const PRUNE_MINIMUM: u64 = 20_000;
const PRUNE_PROTECT: u64 = 40_000;

const PRUNE_PROTECTED_TOOLS: &[&str] = &["skill"];

/// Bus event definition for session.compacted (mirrors TS Event.Compacted).
pub const EVENT_COMPACTED: BusEventDef = BusEventDef::new("session.compacted");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    pub auto: bool,
    pub reserved: Option<u64>,
    pub prune: bool,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            auto: true,
            reserved: None,
            prune: true,
        }
    }
}

/// Input for the compaction process.
#[derive(Debug, Clone)]
pub struct CompactionInput {
    /// The parent message ID that triggered compaction.
    pub parent_id: String,
    /// The session ID.
    pub session_id: String,
    /// Messages to summarize (already converted to provider messages).
    /// Used as fallback when `messages_with_parts` is empty.
    pub messages: Vec<Message>,
    /// Full messages with parts for conversion via `to_model_messages()`.
    /// Mirrors TS `MessageV2.toModelMessages(input.messages, model)`.
    pub messages_with_parts: Vec<MessageWithParts>,
    /// Cancellation token.
    pub abort: tokio_util::sync::CancellationToken,
    /// Whether this was auto-triggered.
    pub auto: bool,
    /// Model to use for summarization.
    pub model: LlmModelRef,
    /// Optional custom prompt (e.g. from a plugin hook).
    pub custom_prompt: Option<String>,
    /// Optional context strings injected by plugins.
    /// Mirrors TS `compacting.context`.
    pub plugin_context: Option<Vec<String>>,
    /// Current working directory (for assistant message path).
    pub cwd: Option<String>,
    /// Worktree root (for assistant message path).
    pub root: Option<String>,
    /// Variant from the original user message.
    pub variant: Option<String>,
    /// Agent name from the original user message (for auto-continue).
    pub original_agent: Option<String>,
}

/// Result of the compaction process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionResult {
    /// Compaction completed successfully; continue the conversation.
    Continue,
    /// Compaction completed but the caller should stop (error or user-initiated).
    Stop,
}

#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

impl TokenUsage {
    pub fn new(input: u64, output: u64) -> Self {
        Self {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            total: input + output,
        }
    }

    pub fn with_cache(mut self, read: u64, write: u64) -> Self {
        self.cache_read = read;
        self.cache_write = write;
        self.total = self.input + self.output + read + write;
        self
    }
}

#[derive(Debug, Clone)]
pub struct ModelLimits {
    pub context: u64,
    pub max_input: Option<u64>,
    pub max_output: u64,
}

/// Status of a tool part for pruning purposes.
/// Mirrors TS `part.state.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolPartStatus {
    Pending,
    Running,
    Completed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneToolPart {
    pub id: String,
    pub tool: String,
    pub output: String,
    /// Mirrors TS `part.state.status`.
    pub status: ToolPartStatus,
    /// Mirrors TS `part.state.time.compacted`.
    pub compacted: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageForPrune {
    pub role: String,
    pub parts: Vec<PruneToolPart>,
    pub summary: bool,
}

/// Trait for session-level operations needed by the compaction engine.
///
/// This abstracts the Session.messages(), Session.updateMessage(), and
/// Session.updatePart() calls from the TS source so the engine can be
/// tested independently.
#[allow(async_fn_in_trait)]
pub trait SessionOps: Send + Sync {
    /// Fetch all messages (with parts) for a session.
    async fn messages(&self, session_id: &str) -> anyhow::Result<Vec<MessageWithParts>>;

    /// Upsert a message (create or update). Returns the persisted message info.
    async fn update_message(&self, info: MessageInfo) -> anyhow::Result<MessageInfo>;

    /// Upsert a part on a message.
    async fn update_part(
        &self,
        session_id: &str,
        message_id: &str,
        part: Part,
    ) -> anyhow::Result<()>;
}

/// Input for the `create()` function (mirrors TS `SessionCompaction.create`).
#[derive(Debug, Clone)]
pub struct CreateCompactionInput {
    pub session_id: String,
    pub agent: String,
    pub model: ModelRef,
    pub auto: bool,
}

pub struct CompactionEngine {
    config: CompactionConfig,
    bus: Option<Arc<Bus>>,
}

impl CompactionEngine {
    pub fn new(config: CompactionConfig) -> Self {
        Self { config, bus: None }
    }

    pub fn with_bus(mut self, bus: Arc<Bus>) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Check whether the conversation has overflowed the context window.
    ///
    /// Mirrors TS `isOverflow`. The `total` field on `TokenUsage` is used
    /// first; if it is zero we fall back to summing the individual fields
    /// (input + output + cache_read + cache_write) -- matching the TS
    /// `input.tokens.total || input.tokens.input + ...` logic.
    pub fn is_overflow(&self, usage: &TokenUsage, limits: &ModelLimits) -> bool {
        if !self.config.auto {
            return false;
        }

        if limits.context == 0 {
            return false;
        }

        // TS: const count = input.tokens.total || input.tokens.input + ...
        let count = if usage.total > 0 {
            usage.total
        } else {
            usage.input + usage.output + usage.cache_read + usage.cache_write
        };

        let reserved = self
            .config
            .reserved
            .unwrap_or_else(|| COMPACTION_BUFFER.min(limits.max_output));

        let usable = limits
            .max_input
            .map(|input| input.saturating_sub(reserved))
            .unwrap_or_else(|| limits.context.saturating_sub(limits.max_output));

        count >= usable
    }

    pub fn estimate_tokens(text: &str) -> u64 {
        let char_count = text.chars().count() as u64;
        char_count / 4
    }

    pub fn generate_summary_prompt() -> String {
        r#"Provide a detailed prompt for continuing our conversation above.
Focus on information that would be helpful for continuing the conversation, including what we did, what we're doing, which files we're working on, and what we're going to do next.
The summary that you construct will be used so that another agent can read it and continue the work.

When constructing the summary, try to stick to this template:
---
## Goal

[What goal(s) is the user trying to accomplish?]

## Instructions

- [What important instructions did the user give you that are relevant]
- [If there is a plan or spec, include information about it so next agent can continue using it]

## Discoveries

[What notable things were learned during this conversation that would be useful for the next agent to know when continuing the work]

## Accomplished

[What work has been completed, what work is still in progress, and what work is left?]

## Relevant files / directories

[Construct a structured list of relevant files that have been read, edited, or created that pertain to the task at hand. If all the files in a directory are relevant, include the path to the directory.]
---"#.to_string()
    }

    pub fn should_prune_tool_result(output: &str, is_protected: bool) -> bool {
        if is_protected {
            return false;
        }

        let estimated = Self::estimate_tokens(output);
        estimated > PRUNE_MINIMUM
    }

    /// Prune old tool results to save context space.
    ///
    /// Mirrors TS `SessionCompaction.prune`. Walks backwards through messages,
    /// skipping the most recent 2 user turns, and erases output of tool parts
    /// whose cumulative token count exceeds `PRUNE_PROTECT`.
    ///
    /// Returns the IDs of pruned parts. The caller is responsible for
    /// persisting the updated parts via `SessionOps::update_part`.
    pub fn prune(&self, messages: &mut [MessageForPrune]) -> Vec<String> {
        // TS: if (config.compaction?.prune === false) return
        if !self.config.prune {
            return vec![];
        }

        if !messages.is_empty() && !Self::should_prune(messages) {
            return vec![];
        }

        tracing::info!("pruning");

        let mut total: u64 = 0;
        let mut pruned: u64 = 0;
        let mut to_prune: Vec<(usize, usize)> = Vec::new();
        let mut turns = 0;

        'outer: for msg_index in (0..messages.len()).rev() {
            let msg = &messages[msg_index];
            if msg.role == "user" {
                turns += 1;
            }
            if turns < 2 {
                continue;
            }
            // TS: if (msg.info.role === "assistant" && msg.info.summary) break loop
            if msg.role == "assistant" && msg.summary {
                break;
            }

            for part_index in (0..msg.parts.len()).rev() {
                let part = &msg.parts[part_index];
                // Skip non-tool parts
                if part.tool.is_empty() {
                    continue;
                }

                // TS: if (part.state.status === "completed") { ... }
                // Only prune completed tool parts
                if part.status != ToolPartStatus::Completed {
                    continue;
                }

                if PRUNE_PROTECTED_TOOLS.contains(&part.tool.as_str()) {
                    continue;
                }

                // TS: if (part.state.time.compacted) break loop
                if part.compacted.is_some() {
                    break 'outer;
                }

                let estimate = Self::estimate_tokens(&part.output);
                total += estimate;
                if total > PRUNE_PROTECT {
                    pruned += estimate;
                    to_prune.push((msg_index, part_index));
                }
            }
        }

        tracing::info!(pruned = pruned, total = total, "found");

        let mut pruned_ids = Vec::new();
        if pruned > PRUNE_MINIMUM {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            for (msg_idx, part_idx) in &to_prune {
                let part = &mut messages[*msg_idx].parts[*part_idx];
                // TS: part.state.time.compacted = Date.now()
                part.compacted = Some(now);
                pruned_ids.push(part.id.clone());
            }

            tracing::info!(count = to_prune.len(), "pruned");
        }

        pruned_ids
    }

    fn should_prune(messages: &[MessageForPrune]) -> bool {
        for msg in messages {
            for part in &msg.parts {
                if !part.tool.is_empty()
                    && part.status == ToolPartStatus::Completed
                    && part.compacted.is_none()
                {
                    return true;
                }
            }
        }
        false
    }

    pub fn create_compaction_part(auto: bool) -> CompactionPart {
        // Plugin hook: session.compacting — notify plugins that compaction is starting.
        // We spawn this as a fire-and-forget task since this method is sync.
        tokio::spawn(async move {
            opencode_plugin::trigger(
                HookContext::new(HookEvent::SessionCompacting)
                    .with_data("auto", serde_json::json!(auto)),
            )
            .await;
        });

        CompactionPart {
            auto,
            created_at: Utc::now(),
        }
    }

    /// Run the full compaction process using LLM summarization.
    ///
    /// Mirrors TS `SessionCompaction.process`. Creates an assistant message
    /// with full metadata, fires the `experimental.session.compacting` plugin
    /// hook (with context injection), converts messages via
    /// `to_model_messages()`, runs the LLM, and handles auto-continue.
    pub async fn process<S: SessionOps>(
        &self,
        input: CompactionInput,
        provider: Arc<dyn Provider>,
        session_ops: Option<&S>,
    ) -> anyhow::Result<CompactionResult> {
        tracing::info!(
            session_id = %input.session_id,
            parent_id = %input.parent_id,
            auto = input.auto,
            "Starting compaction process"
        );

        let now_ms = Utc::now().timestamp_millis();

        // TS: const msg = await Session.updateMessage({ ... role: "assistant", summary: true, ... })
        // Create the assistant message with full metadata.
        let assistant_id =
            opencode_core::id::create(opencode_core::id::Prefix::Message, false, None);
        let assistant_info = MessageInfo::Assistant {
            id: assistant_id.clone(),
            session_id: input.session_id.clone(),
            time: AssistantTime {
                created: now_ms,
                completed: None,
            },
            parent_id: input.parent_id.clone(),
            model_id: input.model.model_id.clone(),
            provider_id: input.model.provider_id.clone(),
            mode: "compaction".to_string(),
            agent: "compaction".to_string(),
            path: MessagePath {
                cwd: input.cwd.clone().unwrap_or_default(),
                root: input.root.clone().unwrap_or_default(),
            },
            summary: Some(true),
            cost: 0.0,
            tokens: AssistantTokens {
                total: None,
                input: 0,
                output: 0,
                reasoning: 0,
                cache: CacheTokens { read: 0, write: 0 },
            },
            error: None,
            structured: None,
            variant: input.variant.clone(),
            finish: None,
        };

        // Persist the assistant message if session ops are available.
        if let Some(ops) = session_ops {
            let _ = ops.update_message(assistant_info.clone()).await;
        }

        // TS: const compacting = await Plugin.trigger("experimental.session.compacting", ...)
        // Fire the plugin hook and collect context / prompt override.
        let hook_ctx = HookContext::new(HookEvent::SessionCompacting)
            .with_session(&input.session_id)
            .with_data("auto", serde_json::json!(input.auto));
        let hook_outputs = opencode_plugin::trigger_collect(hook_ctx).await;

        // Build the compaction prompt.
        // TS: const promptText = compacting.prompt ?? [defaultPrompt, ...compacting.context].join("\n\n")
        let default_prompt = Self::generate_summary_prompt();
        let prompt_text = resolve_compaction_prompt(
            default_prompt,
            input.custom_prompt.clone(),
            input.plugin_context.clone(),
            hook_outputs,
        );

        // Compaction agent: low temperature, no tools, capped output.
        let agent = LlmAgent {
            name: "compaction".to_string(),
            system_prompt: None,
            temperature: Some(0.0),
            top_p: None,
            max_tokens: Some(4096),
        };

        // TS: messages: [...MessageV2.toModelMessages(input.messages, model), { role: "user", ... }]
        // Convert MessageWithParts to provider messages, then append the compaction prompt.
        let mut llm_messages = if !input.messages_with_parts.is_empty() {
            to_model_messages(&input.messages_with_parts)
        } else {
            input.messages
        };
        llm_messages.push(Message::user(prompt_text));

        // Build a synthetic MessageInfo::User for the stream input.
        let user_info = MessageInfo::User {
            id: opencode_core::id::create(opencode_core::id::Prefix::Message, false, None),
            session_id: input.session_id.clone(),
            time: UserTime { created: now_ms },
            agent: "compaction".to_string(),
            model: ModelRef {
                provider_id: input.model.provider_id.clone(),
                model_id: input.model.model_id.clone(),
            },
            format: None,
            summary: None,
            system: None,
            tools: None,
            variant: input.variant.clone(),
        };

        let stream_input = StreamInput {
            user: user_info,
            session_id: input.session_id.clone(),
            model: input.model.clone(),
            agent,
            system: vec![],
            abort: input.abort.clone(),
            messages: llm_messages,
            small: false,
            tools: HashMap::new(),
            retries: Some(1),
            tool_choice: None,
            cost_rates: None,
            work_dir: None,
        };

        match LlmProcessor::stream(stream_input, provider).await {
            Ok(output) => {
                let result = collect_stream(output, input.abort.clone()).await?;

                tracing::info!(
                    session_id = %input.session_id,
                    summary_length = result.text.len(),
                    "Compaction summary generated"
                );

                // TS: if (processor.message.error) return "stop"
                // If the LLM returned empty text, treat it as an error.
                if result.text.is_empty() {
                    tracing::warn!(
                        session_id = %input.session_id,
                        "Compaction produced empty summary"
                    );
                    return Ok(CompactionResult::Stop);
                }

                // Persist the generated summary as the assistant text part.
                if let Some(ops) = session_ops {
                    let now_part = Utc::now().timestamp_millis();
                    let mut metadata = HashMap::new();
                    metadata.insert("summary".to_string(), serde_json::json!(true));

                    let summary_part = Part::Text {
                        id: opencode_core::id::create(opencode_core::id::Prefix::Part, false, None),
                        session_id: input.session_id.clone(),
                        message_id: assistant_id.clone(),
                        text: result.text.clone(),
                        synthetic: Some(true),
                        ignored: None,
                        time: Some(TextTime {
                            start: Some(now_part),
                            end: Some(now_part),
                        }),
                        metadata: Some(metadata),
                    };
                    let _ = ops
                        .update_part(&input.session_id, &assistant_id, summary_part)
                        .await;

                    let mut completed_info = assistant_info.clone();
                    if let MessageInfo::Assistant { time, .. } = &mut completed_info {
                        time.completed = Some(now_part);
                    }
                    let _ = ops.update_message(completed_info).await;
                }

                // TS: if (result === "continue" && input.auto) { ... create continue message ... }
                if input.auto {
                    if let Some(ops) = session_ops {
                        // Create a synthetic user message for auto-continue.
                        let continue_msg_id = opencode_core::id::create(
                            opencode_core::id::Prefix::Message,
                            false,
                            None,
                        );
                        let continue_user = MessageInfo::User {
                            id: continue_msg_id.clone(),
                            session_id: input.session_id.clone(),
                            time: UserTime {
                                created: Utc::now().timestamp_millis(),
                            },
                            agent: input
                                .original_agent
                                .clone()
                                .unwrap_or_else(|| "default".to_string()),
                            model: ModelRef {
                                provider_id: input.model.provider_id.clone(),
                                model_id: input.model.model_id.clone(),
                            },
                            format: None,
                            summary: None,
                            system: None,
                            tools: None,
                            variant: input.variant.clone(),
                        };
                        let _ = ops.update_message(continue_user).await;

                        // Create a synthetic text part with the continue prompt.
                        let continue_part_id =
                            opencode_core::id::create(opencode_core::id::Prefix::Part, false, None);
                        let now_part = Utc::now().timestamp_millis();
                        let continue_part = Part::Text {
                            id: continue_part_id,
                            session_id: input.session_id.clone(),
                            message_id: continue_msg_id.clone(),
                            text: generate_continue_message(),
                            synthetic: Some(true),
                            ignored: None,
                            time: Some(TextTime {
                                start: Some(now_part),
                                end: Some(now_part),
                            }),
                            metadata: None,
                        };
                        let _ = ops
                            .update_part(&input.session_id, &continue_msg_id, continue_part)
                            .await;
                    }
                }

                // TS: Bus.publish(Event.Compacted, { sessionID: input.sessionID })
                if let Some(ref bus) = self.bus {
                    bus.publish(
                        &EVENT_COMPACTED,
                        serde_json::json!({ "sessionID": input.session_id }),
                    )
                    .await;
                }

                // Fire the session.compacting hook to notify plugins that
                // compaction finished.
                opencode_plugin::trigger(
                    HookContext::new(HookEvent::SessionCompacting)
                        .with_session(&input.session_id)
                        .with_data("auto", serde_json::json!(input.auto))
                        .with_data("completed", serde_json::json!(true)),
                )
                .await;

                if input.auto {
                    Ok(CompactionResult::Continue)
                } else {
                    Ok(CompactionResult::Stop)
                }
            }
            Err(e) => {
                tracing::error!(
                    session_id = %input.session_id,
                    error = %e,
                    "Compaction LLM call failed"
                );
                Ok(CompactionResult::Stop)
            }
        }
    }

    /// Create a compaction user message and compaction part.
    ///
    /// Mirrors TS `SessionCompaction.create`. Creates a user message with the
    /// given agent/model, then attaches a compaction part with the `auto` flag.
    pub async fn create<S: SessionOps>(
        input: CreateCompactionInput,
        session_ops: &S,
    ) -> anyhow::Result<(MessageInfo, Part)> {
        let msg_id = opencode_core::id::create(opencode_core::id::Prefix::Message, false, None);
        let now_ms = Utc::now().timestamp_millis();

        // TS: const msg = await Session.updateMessage({ role: "user", ... })
        let user_msg = MessageInfo::User {
            id: msg_id.clone(),
            session_id: input.session_id.clone(),
            time: UserTime { created: now_ms },
            agent: input.agent.clone(),
            model: input.model.clone(),
            format: None,
            summary: None,
            system: None,
            tools: None,
            variant: None,
        };
        let persisted_msg = session_ops.update_message(user_msg.clone()).await?;

        let persisted_id = match &persisted_msg {
            MessageInfo::User { id, .. } => id.clone(),
            MessageInfo::Assistant { id, .. } => id.clone(),
        };

        // TS: await Session.updatePart({ type: "compaction", auto: input.auto, ... })
        let part_id = opencode_core::id::create(opencode_core::id::Prefix::Part, false, None);
        let compaction_part = Part::Compaction(crate::message_v2::CompactionPart {
            id: part_id,
            session_id: input.session_id.clone(),
            message_id: persisted_id.clone(),
            auto: input.auto,
        });
        session_ops
            .update_part(&input.session_id, &persisted_id, compaction_part.clone())
            .await?;

        Ok((persisted_msg, compaction_part))
    }

    /// Prune tool results for a session using `SessionOps` for persistence.
    ///
    /// This is the high-level async version that mirrors the TS `prune()`
    /// function which fetches messages, prunes, and persists updates.
    pub async fn prune_session<S: SessionOps>(
        &self,
        session_id: &str,
        session_ops: &S,
    ) -> anyhow::Result<Vec<String>> {
        if !self.config.prune {
            return Ok(vec![]);
        }

        let msgs = session_ops.messages(session_id).await?;
        let mut prune_msgs = messages_to_prune_format(&msgs);
        let pruned_ids = self.prune(&mut prune_msgs);

        // Persist the compacted timestamps back via session ops.
        // We need to map pruned IDs back to their message_id + part for update.
        for msg in &msgs {
            let message_id = match &msg.info {
                MessageInfo::User { id, .. } => id,
                MessageInfo::Assistant { id, .. } => id,
            };
            for part in &msg.parts {
                if let Part::Tool(tool_part) = part {
                    if pruned_ids.contains(&tool_part.id) {
                        // Re-create the part with the compacted timestamp set.
                        if let ToolState::Completed {
                            input,
                            output,
                            title,
                            metadata,
                            time,
                            attachments,
                        } = &tool_part.state
                        {
                            let updated_part = Part::Tool(crate::message_v2::ToolPart {
                                id: tool_part.id.clone(),
                                session_id: tool_part.session_id.clone(),
                                message_id: tool_part.message_id.clone(),
                                call_id: tool_part.call_id.clone(),
                                tool: tool_part.tool.clone(),
                                state: ToolState::Completed {
                                    input: input.clone(),
                                    output: output.clone(),
                                    title: title.clone(),
                                    metadata: metadata.clone(),
                                    time: CompletedTime {
                                        start: time.start,
                                        end: time.end,
                                        compacted: Some(Utc::now().timestamp_millis()),
                                    },
                                    attachments: attachments.clone(),
                                },
                                metadata: tool_part.metadata.clone(),
                            });
                            let _ = session_ops
                                .update_part(session_id, message_id, updated_part)
                                .await;
                        }
                    }
                }
            }
        }

        Ok(pruned_ids)
    }
}

impl Default for CompactionEngine {
    fn default() -> Self {
        Self::new(CompactionConfig::default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSummary {
    pub created_at: DateTime<Utc>,
    pub tokens_saved: u64,
    pub messages_compacted: usize,
    pub tool_results_pruned: usize,
}

impl CompactionSummary {
    pub fn new(tokens_saved: u64, messages_compacted: usize, tool_results_pruned: usize) -> Self {
        Self {
            created_at: Utc::now(),
            tokens_saved,
            messages_compacted,
            tool_results_pruned,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionPart {
    pub auto: bool,
    pub created_at: DateTime<Utc>,
}

pub fn estimate_message_tokens(content: &str, tool_results: &[String]) -> u64 {
    let content_tokens = CompactionEngine::estimate_tokens(content);
    let tool_tokens: u64 = tool_results
        .iter()
        .map(|r| CompactionEngine::estimate_tokens(r))
        .sum();
    content_tokens + tool_tokens
}

pub fn can_compact_messages(messages: usize, min_messages: usize) -> bool {
    messages >= min_messages
}

pub fn generate_continue_message() -> String {
    "Continue if you have next steps, or stop and ask for clarification if you are unsure how to proceed.".to_string()
}

fn parse_compaction_hook_payload(payload: &serde_json::Value) -> (Option<String>, Vec<String>) {
    let source = payload
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(payload);

    let prompt = source
        .get("prompt")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            source
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        });

    let mut context = Vec::new();
    if let Some(value) = source.get("context") {
        if let Some(values) = value.as_array() {
            context.extend(values.iter().filter_map(|item| {
                item.as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(ToString::to_string)
            }));
        } else if let Some(item) = value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            context.push(item.to_string());
        }
    }

    (prompt, context)
}

fn resolve_compaction_prompt(
    default_prompt: String,
    custom_prompt: Option<String>,
    plugin_context: Option<Vec<String>>,
    hook_outputs: Vec<opencode_plugin::HookOutput>,
) -> String {
    let mut merged_context = plugin_context.unwrap_or_default();
    let mut hook_prompt: Option<String> = None;

    for output in hook_outputs {
        let Some(payload) = output.payload.as_ref() else {
            continue;
        };
        let (prompt, context) = parse_compaction_hook_payload(payload);
        if prompt.is_some() {
            hook_prompt = prompt;
        }
        merged_context.extend(context);
    }

    hook_prompt.or(custom_prompt).unwrap_or_else(|| {
        if merged_context.is_empty() {
            default_prompt
        } else {
            let mut parts = vec![default_prompt];
            parts.extend(merged_context);
            parts.join("\n\n")
        }
    })
}

/// Convenience function to run the full LLM-based compaction for a session.
pub async fn run_compaction<S: SessionOps>(
    session_id: &str,
    parent_id: &str,
    messages: Vec<Message>,
    model: LlmModelRef,
    provider: Arc<dyn Provider>,
    abort: tokio_util::sync::CancellationToken,
    auto: bool,
    config: Option<CompactionConfig>,
    session_ops: Option<&S>,
) -> anyhow::Result<CompactionResult> {
    let engine = CompactionEngine::new(config.unwrap_or_default());

    let input = CompactionInput {
        parent_id: parent_id.to_string(),
        session_id: session_id.to_string(),
        messages,
        messages_with_parts: vec![],
        abort,
        auto,
        model,
        custom_prompt: None,
        plugin_context: None,
        cwd: None,
        root: None,
        variant: None,
        original_agent: None,
    };

    engine.process(input, provider, session_ops).await
}

/// Convert `MessageWithParts` to the `MessageForPrune` format used by the
/// prune algorithm.
pub fn messages_to_prune_format(messages: &[MessageWithParts]) -> Vec<MessageForPrune> {
    messages
        .iter()
        .map(|msg| {
            let role = match &msg.info {
                MessageInfo::User { .. } => "user".to_string(),
                MessageInfo::Assistant { .. } => "assistant".to_string(),
            };
            let summary = match &msg.info {
                MessageInfo::Assistant { summary, .. } => summary.unwrap_or(false),
                _ => false,
            };
            let parts = msg
                .parts
                .iter()
                .filter_map(|p| {
                    if let Part::Tool(tool_part) = p {
                        let (status, output, compacted) = match &tool_part.state {
                            ToolState::Pending { .. } => {
                                (ToolPartStatus::Pending, String::new(), None)
                            }
                            ToolState::Running { .. } => {
                                (ToolPartStatus::Running, String::new(), None)
                            }
                            ToolState::Completed { output, time, .. } => (
                                ToolPartStatus::Completed,
                                output.clone(),
                                time.compacted.map(|t| t as u64),
                            ),
                            ToolState::Error { error, .. } => {
                                (ToolPartStatus::Error, error.clone(), None)
                            }
                        };
                        Some(PruneToolPart {
                            id: tool_part.id.clone(),
                            tool: tool_part.tool.clone(),
                            output,
                            status,
                            compacted,
                        })
                    } else {
                        None
                    }
                })
                .collect();
            MessageForPrune {
                role,
                parts,
                summary,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_trait::async_trait;
    use futures::stream;
    use opencode_provider::{
        ChatRequest, ChatResponse, ModelInfo, ProviderError, StreamEvent, StreamResult,
    };
    use tokio::sync::Mutex;

    #[derive(Clone)]
    struct MockProvider {
        model: ModelInfo,
        stream_events: Vec<StreamEvent>,
        last_request: Arc<Mutex<Option<ChatRequest>>>,
    }

    impl MockProvider {
        fn new(model_id: &str, context_window: u64, max_output_tokens: u64, text: &str) -> Self {
            Self {
                model: ModelInfo {
                    id: model_id.to_string(),
                    name: "Mock Model".to_string(),
                    provider: "mock".to_string(),
                    context_window,
                    max_output_tokens,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 0.0,
                    cost_per_million_output: 0.0,
                },
                stream_events: vec![
                    StreamEvent::Start,
                    StreamEvent::TextDelta(text.to_string()),
                    StreamEvent::Done,
                ],
                last_request: Arc::new(Mutex::new(None)),
            }
        }

        async fn last_request(&self) -> Option<ChatRequest> {
            self.last_request.lock().await.clone()
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn id(&self) -> &str {
            "mock"
        }

        fn name(&self) -> &str {
            "Mock"
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![self.model.clone()]
        }

        fn get_model(&self, id: &str) -> Option<&ModelInfo> {
            if id == self.model.id {
                Some(&self.model)
            } else {
                None
            }
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            Err(ProviderError::InvalidRequest(
                "chat() not used in this test".to_string(),
            ))
        }

        async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
            *self.last_request.lock().await = Some(request);
            Ok(Box::pin(stream::iter(
                self.stream_events
                    .clone()
                    .into_iter()
                    .map(Result::<StreamEvent, ProviderError>::Ok),
            )))
        }
    }

    #[derive(Default)]
    struct MockSessionOps {
        messages: Mutex<Vec<MessageInfo>>,
        parts: Mutex<Vec<(String, String, Part)>>,
    }

    impl MockSessionOps {
        async fn message_updates(&self) -> Vec<MessageInfo> {
            self.messages.lock().await.clone()
        }

        async fn part_updates(&self) -> Vec<(String, String, Part)> {
            self.parts.lock().await.clone()
        }
    }

    impl SessionOps for MockSessionOps {
        async fn messages(&self, _session_id: &str) -> anyhow::Result<Vec<MessageWithParts>> {
            Ok(vec![])
        }

        async fn update_message(&self, info: MessageInfo) -> anyhow::Result<MessageInfo> {
            self.messages.lock().await.push(info.clone());
            Ok(info)
        }

        async fn update_part(
            &self,
            session_id: &str,
            message_id: &str,
            part: Part,
        ) -> anyhow::Result<()> {
            self.parts
                .lock()
                .await
                .push((session_id.to_string(), message_id.to_string(), part));
            Ok(())
        }
    }

    fn make_input(
        model_id: &str,
        custom_prompt: Option<String>,
        plugin_context: Option<Vec<String>>,
    ) -> CompactionInput {
        CompactionInput {
            parent_id: "msg_parent".to_string(),
            session_id: "ses_test".to_string(),
            messages: vec![Message::user("existing content")],
            messages_with_parts: vec![],
            abort: tokio_util::sync::CancellationToken::new(),
            auto: false,
            model: LlmModelRef {
                model_id: model_id.to_string(),
                provider_id: "mock".to_string(),
            },
            custom_prompt,
            plugin_context,
            cwd: Some("/tmp".to_string()),
            root: Some("/tmp".to_string()),
            variant: None,
            original_agent: Some("general".to_string()),
        }
    }

    #[test]
    fn test_compaction_config_default() {
        let config = CompactionConfig::default();
        assert!(config.auto);
        assert!(config.prune);
        assert!(config.reserved.is_none());
    }

    #[test]
    fn test_is_overflow_disabled() {
        let engine = CompactionEngine::new(CompactionConfig {
            auto: false,
            ..Default::default()
        });
        let usage = TokenUsage::new(100000, 5000);
        let limits = ModelLimits {
            context: 128000,
            max_input: None,
            max_output: 8192,
        };
        assert!(!engine.is_overflow(&usage, &limits));
    }

    #[test]
    fn test_is_overflow_within_limits() {
        let engine = CompactionEngine::default();
        let usage = TokenUsage::new(50000, 5000);
        let limits = ModelLimits {
            context: 128000,
            max_input: None,
            max_output: 8192,
        };
        assert!(!engine.is_overflow(&usage, &limits));
    }

    #[test]
    fn test_is_overflow_exceeded() {
        let engine = CompactionEngine::default();
        let usage = TokenUsage::new(120000, 5000);
        let limits = ModelLimits {
            context: 128000,
            max_input: None,
            max_output: 8192,
        };
        assert!(engine.is_overflow(&usage, &limits));
    }

    #[test]
    fn test_estimate_tokens() {
        // 11 chars / 4 = 2
        assert_eq!(CompactionEngine::estimate_tokens("hello world"), 2);
        assert_eq!(CompactionEngine::estimate_tokens(""), 0);
    }

    #[test]
    fn test_generate_summary_prompt_contains_template() {
        let prompt = CompactionEngine::generate_summary_prompt();
        assert!(prompt.contains("## Goal"));
        assert!(prompt.contains("## Accomplished"));
        assert!(prompt.contains("## Relevant files"));
    }

    #[test]
    fn test_should_prune_tool_result() {
        // Small output -- should not prune
        assert!(!CompactionEngine::should_prune_tool_result(
            "small output",
            false
        ));

        // Large output -- should prune
        let large = "x".repeat(100_000);
        assert!(CompactionEngine::should_prune_tool_result(&large, false));

        // Protected -- should not prune even if large
        assert!(!CompactionEngine::should_prune_tool_result(&large, true));
    }

    #[test]
    fn test_compaction_result_variants() {
        assert_eq!(CompactionResult::Continue, CompactionResult::Continue);
        assert_eq!(CompactionResult::Stop, CompactionResult::Stop);
        assert_ne!(CompactionResult::Continue, CompactionResult::Stop);
    }

    #[test]
    fn test_generate_continue_message() {
        let msg = generate_continue_message();
        assert!(msg.contains("Continue"));
    }

    #[test]
    fn test_compaction_input_fields() {
        let abort = tokio_util::sync::CancellationToken::new();
        let input = CompactionInput {
            parent_id: "msg_123".to_string(),
            session_id: "ses_456".to_string(),
            messages: vec![],
            messages_with_parts: vec![],
            abort,
            auto: true,
            model: LlmModelRef {
                model_id: "claude-3".to_string(),
                provider_id: "anthropic".to_string(),
            },
            custom_prompt: Some("custom".to_string()),
            plugin_context: None,
            cwd: None,
            root: None,
            variant: None,
            original_agent: None,
        };
        assert_eq!(input.parent_id, "msg_123");
        assert_eq!(input.session_id, "ses_456");
        assert!(input.auto);
        assert_eq!(input.custom_prompt.as_deref(), Some("custom"));
    }

    #[test]
    fn test_prune_respects_config_disabled() {
        let engine = CompactionEngine::new(CompactionConfig {
            prune: false,
            ..Default::default()
        });
        let mut messages = vec![MessageForPrune {
            role: "assistant".to_string(),
            parts: vec![PruneToolPart {
                id: "p1".to_string(),
                tool: "read_file".to_string(),
                output: "x".repeat(200_000),
                status: ToolPartStatus::Completed,
                compacted: None,
            }],
            summary: false,
        }];
        let pruned = engine.prune(&mut messages);
        assert!(pruned.is_empty());
    }

    #[test]
    fn test_prune_skips_non_completed() {
        let engine = CompactionEngine::default();
        let mut messages = vec![
            MessageForPrune {
                role: "user".to_string(),
                parts: vec![],
                summary: false,
            },
            MessageForPrune {
                role: "user".to_string(),
                parts: vec![],
                summary: false,
            },
            MessageForPrune {
                role: "assistant".to_string(),
                parts: vec![PruneToolPart {
                    id: "p1".to_string(),
                    tool: "read_file".to_string(),
                    output: "x".repeat(200_000),
                    status: ToolPartStatus::Running,
                    compacted: None,
                }],
                summary: false,
            },
        ];
        let pruned = engine.prune(&mut messages);
        assert!(pruned.is_empty());
    }

    #[test]
    fn test_is_overflow_total_fallback() {
        let engine = CompactionEngine::default();
        // total is 0, so fallback to input + output + cache_read + cache_write
        let usage = TokenUsage {
            input: 100_000,
            output: 5_000,
            cache_read: 10_000,
            cache_write: 10_000,
            total: 0,
        };
        let limits = ModelLimits {
            context: 128000,
            max_input: None,
            max_output: 8192,
        };
        assert!(engine.is_overflow(&usage, &limits));
    }

    #[test]
    fn test_is_overflow_zero_context() {
        let engine = CompactionEngine::default();
        let usage = TokenUsage::new(100_000, 5_000);
        let limits = ModelLimits {
            context: 0,
            max_input: None,
            max_output: 8192,
        };
        assert!(!engine.is_overflow(&usage, &limits));
    }

    #[test]
    fn test_create_compaction_input() {
        let input = CreateCompactionInput {
            session_id: "ses_123".to_string(),
            agent: "default".to_string(),
            model: ModelRef {
                provider_id: "anthropic".to_string(),
                model_id: "claude-3".to_string(),
            },
            auto: true,
        };
        assert_eq!(input.session_id, "ses_123");
        assert!(input.auto);
    }

    #[test]
    fn test_tool_part_status_serde() {
        let status = ToolPartStatus::Completed;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"completed\"");
        let deserialized: ToolPartStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ToolPartStatus::Completed);
    }

    #[test]
    fn test_event_compacted_def() {
        assert_eq!(EVENT_COMPACTED.event_type, "session.compacted");
    }

    #[test]
    fn test_estimate_message_tokens() {
        let tokens = estimate_message_tokens("hello", &["world".to_string()]);
        assert_eq!(tokens, 2); // 5/4 + 5/4 = 1 + 1
    }

    #[test]
    fn test_can_compact_messages() {
        assert!(can_compact_messages(10, 5));
        assert!(can_compact_messages(5, 5));
        assert!(!can_compact_messages(4, 5));
    }

    #[test]
    fn test_resolve_compaction_prompt_prefers_hook_override() {
        let prompt = resolve_compaction_prompt(
            "default prompt".to_string(),
            Some("custom prompt".to_string()),
            Some(vec!["input ctx".to_string()]),
            vec![opencode_plugin::HookOutput::with_payload(
                serde_json::json!({
                    "prompt": "hook prompt",
                    "context": ["hook ctx"]
                }),
            )],
        );

        assert_eq!(prompt, "hook prompt");
    }

    #[test]
    fn test_resolve_compaction_prompt_merges_context() {
        let prompt = resolve_compaction_prompt(
            "default prompt".to_string(),
            None,
            Some(vec!["input ctx".to_string()]),
            vec![opencode_plugin::HookOutput::with_payload(
                serde_json::json!({
                    "context": ["hook ctx 1", "hook ctx 2"]
                }),
            )],
        );

        assert!(prompt.contains("default prompt"));
        assert!(prompt.contains("input ctx"));
        assert!(prompt.contains("hook ctx 1"));
        assert!(prompt.contains("hook ctx 2"));
    }

    #[test]
    fn test_parse_compaction_hook_payload_supports_nested_data() {
        let (prompt, context) = parse_compaction_hook_payload(&serde_json::json!({
            "data": {
                "prompt": "nested prompt",
                "context": ["ctx1", "ctx2"]
            }
        }));

        assert_eq!(prompt.as_deref(), Some("nested prompt"));
        assert_eq!(context, vec!["ctx1".to_string(), "ctx2".to_string()]);
    }

    #[tokio::test]
    async fn test_process_persists_summary_text_part() {
        let engine = CompactionEngine::default();
        let provider = Arc::new(MockProvider::new(
            "mock-model",
            8192,
            1024,
            "summary from llm",
        ));
        let ops = MockSessionOps::default();
        let input = make_input("mock-model", None, None);

        let result = engine
            .process(input, provider, Some(&ops))
            .await
            .expect("compaction should succeed");

        assert_eq!(result, CompactionResult::Stop);

        let parts = ops.part_updates().await;
        let summary_text = parts.iter().find_map(|(_, _, part)| match part {
            Part::Text { text, metadata, .. } => Some((text.clone(), metadata.clone())),
            _ => None,
        });
        let (text, metadata) = summary_text.expect("summary text part should be persisted");
        assert_eq!(text, "summary from llm");
        assert_eq!(
            metadata
                .as_ref()
                .and_then(|map| map.get("summary"))
                .and_then(|value| value.as_bool()),
            Some(true)
        );

        let message_updates = ops.message_updates().await;
        let has_completed_summary_message = message_updates.iter().any(|info| match info {
            MessageInfo::Assistant { summary, time, .. } => {
                summary.unwrap_or(false) && time.completed.is_some()
            }
            _ => false,
        });
        assert!(has_completed_summary_message);
    }

    #[tokio::test]
    async fn test_process_uses_custom_prompt_when_no_hook_override() {
        let engine = CompactionEngine::default();
        let provider = Arc::new(MockProvider::new(
            "mock-model",
            8192,
            1024,
            "summary from llm",
        ));
        let input = make_input(
            "mock-model",
            Some("custom compaction prompt".to_string()),
            None,
        );

        engine
            .process(input, provider.clone(), Option::<&MockSessionOps>::None)
            .await
            .expect("compaction should succeed");

        let request = provider
            .last_request()
            .await
            .expect("chat request should be captured");
        let last_message = request
            .messages
            .last()
            .expect("request should have messages");
        let prompt = match &last_message.content {
            opencode_provider::Content::Text(text) => text.clone(),
            _ => String::new(),
        };
        assert_eq!(prompt, "custom compaction prompt");
    }
}
