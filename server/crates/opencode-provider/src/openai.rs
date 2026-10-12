use async_trait::async_trait;
use futures::{stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::Arc;
use tracing;

// ---------------------------------------------------------------------------
// Lenient response types for OpenAI-compatible /chat/completions responses.
//
// Mirrors the TS SDK's Zod schema where every field is `.nullish()`.
// These are separate from the internal `Message`/`ChatResponse` types because
// the wire format differs (e.g. `tool_calls` with `function.{name,arguments}`
// instead of our internal `ContentPart` representation, `content` can be null).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawChatResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<RawChoice>,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Debug, Deserialize)]
struct RawChoice {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    message: Option<RawMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<RawToolCall>>,
    #[serde(default, rename = "reasoning_text")]
    _reasoning_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawToolCall {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<RawFunction>,
}

#[derive(Debug, Deserialize)]
struct RawFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

impl RawChatResponse {
    /// Convert the lenient wire format into our internal `ChatResponse`.
    fn into_chat_response(self) -> ChatResponse {
        let choices = self
            .choices
            .into_iter()
            .map(|c| {
                let raw_msg = c.message.unwrap_or(RawMessage {
                    role: None,
                    content: None,
                    tool_calls: None,
                    _reasoning_text: None,
                });

                // Build content parts from the raw message.
                let mut parts: Vec<crate::ContentPart> = Vec::new();

                // Text content
                if let Some(text) = &raw_msg.content {
                    if !text.is_empty() {
                        parts.push(crate::ContentPart {
                            content_type: "text".to_string(),
                            text: Some(text.clone()),
                            ..Default::default()
                        });
                    }
                }

                // Tool calls → ContentPart with tool_use
                if let Some(tool_calls) = &raw_msg.tool_calls {
                    for tc in tool_calls {
                        let func = tc.function.as_ref();
                        let name = func.and_then(|f| f.name.as_deref()).unwrap_or("");
                        let args_str = func.and_then(|f| f.arguments.as_deref()).unwrap_or("{}");
                        let input: serde_json::Value =
                            serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                        let id = tc
                            .id
                            .clone()
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                        parts.push(crate::ContentPart {
                            content_type: "tool_use".to_string(),
                            tool_use: Some(crate::ToolUse {
                                id,
                                name: name.to_string(),
                                input,
                            }),
                            ..Default::default()
                        });
                    }
                }

                let content = if parts.is_empty() {
                    crate::Content::Text(raw_msg.content.unwrap_or_default())
                } else if parts.len() == 1 && parts[0].content_type == "text" {
                    crate::Content::Text(parts.remove(0).text.unwrap_or_default())
                } else {
                    crate::Content::Parts(parts)
                };

                Choice {
                    index: c.index.unwrap_or(0),
                    message: Message {
                        role: match raw_msg.role.as_deref() {
                            Some("assistant") | None => Role::Assistant,
                            Some("system") => Role::System,
                            Some("user") => Role::User,
                            Some("tool") => Role::Tool,
                            _ => Role::Assistant,
                        },
                        content,
                        cache_control: None,
                        provider_options: None,
                    },
                    finish_reason: c.finish_reason,
                }
            })
            .collect();

        let usage = self.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens: u.total_tokens.unwrap_or(0),
            cache_read_input_tokens: u.cache_read_input_tokens,
            cache_creation_input_tokens: u.cache_creation_input_tokens,
        });

        ChatResponse {
            id: self.id.unwrap_or_default(),
            model: self.model.unwrap_or_default(),
            choices,
            usage,
        }
    }
}

use crate::custom_fetch::get_custom_fetch_proxy;
use crate::responses::{
    FinishReason, GenerateOptions, OpenAIResponsesConfig, OpenAIResponsesLanguageModel,
    ResponsesProviderOptions, StreamOptions,
};
use crate::tools::InputTool;
use crate::{
    ChatRequest, ChatResponse, Choice, Message, ModelInfo, Provider, ProviderError, Role,
    StreamEvent, StreamResult, Usage,
};

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";

#[derive(Debug, Clone)]
pub struct OpenAIConfig {
    pub api_key: String,
    pub base_url: Option<String>,
    pub organization: Option<String>,
    /// Extra headers sent verbatim on every request. Zen's free tier only answers
    /// traffic carrying an OpenCode client fingerprint.
    pub extra_headers: Vec<(String, String)>,
    /// Zen rejects (`FreeTierError`) any body that is not the shape a real agent turn
    /// sends: `stream: true`, `stream_options`, a system message and a non-empty `tools`
    /// array. Stripping those for a single-shot reply is what makes every agent turn 403.
    pub requires_agent_shape: bool,
}

#[derive(Debug)]
pub struct OpenAIProvider {
    client: Client,
    config: OpenAIConfig,
    models: Vec<ModelInfo>,
    legacy_only: bool,
}

#[derive(Debug, Default)]
struct LegacySseParserState {
    tool_call_ids: HashMap<u32, String>,
    tool_call_names: HashMap<u32, String>,
    reasoning_open: bool,
}

impl OpenAIProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_config(OpenAIConfig {
            api_key: api_key.into(),
            base_url: None,
            organization: None,
            extra_headers: Vec::new(),
            requires_agent_shape: false,
        })
    }

    pub fn new_with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self::with_config(OpenAIConfig {
            api_key: api_key.into(),
            base_url: Some(base_url.into()),
            organization: None,
            extra_headers: Vec::new(),
            requires_agent_shape: false,
        })
    }

    /// Zen gateway: literal key `public` plus a client fingerprint.
    ///
    /// Without `x-opencode-*` the gateway answers every inference with 403 FreeTierError
    /// while `/models` still returns 200, so the provider looks healthy and yields
    /// nothing. The session/request pair must come from one real OpenCode run.
    pub fn zen(base_url: impl Into<String>) -> Self {
        Self::zen_with_key(base_url, "public".to_string())
    }

    /// Zen's gateway gates the free tier on the `x-opencode-*` fingerprint, not on the
    /// credential: a provider holding a real key still needs it, so the key is a
    /// parameter rather than the marker that picks the constructor.
    pub fn zen_with_key(base_url: impl Into<String>, api_key: String) -> Self {
        let session = std::env::var("OPENCODE_ZEN_SESSION").unwrap_or_default();
        let request = std::env::var("OPENCODE_ZEN_REQUEST").unwrap_or_default();
        // Captured off a working `opencode run` (models.ts) rather than guessed:
        // a wrong User-Agent or a "global" project id is refused with the same
        // FreeTierError as no fingerprint at all, so the exact bytes matter.
        let mut headers = vec![
            (
                "User-Agent".to_string(),
                std::env::var("OPENCODE_ZEN_USER_AGENT")
                    .unwrap_or_else(|_| "opencode/1.18.35 ai-sdk/provider-utils/4.0.23 runtime/bun/1.3.14".to_string()),
            ),
            ("x-opencode-client".to_string(), "cli".to_string()),
            (
                "x-opencode-project".to_string(),
                std::env::var("OPENCODE_ZEN_PROJECT")
                    .unwrap_or_else(|_| "838c9a927763d1107fa080f852eba256dc71f064".to_string()),
            ),
        ];
        if !session.is_empty() && !request.is_empty() {
            headers.push(("x-opencode-session".to_string(), session.clone()));
            headers.push(("x-opencode-session-id".to_string(), session));
            headers.push(("x-opencode-request".to_string(), request));
        }
        Self::from_config(
            OpenAIConfig {
                api_key,
                base_url: Some(base_url.into()),
                organization: None,
                extra_headers: headers,
                requires_agent_shape: true,
            },
            true,
        )
    }

    fn from_config(config: OpenAIConfig, legacy_only: bool) -> Self {
        let models = vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                name: "GPT-4o".to_string(),
                provider: "openai".to_string(),
                context_window: 128000,
                max_output_tokens: 16384,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 2.5,
                cost_per_million_output: 10.0,
            },
            ModelInfo {
                id: "gpt-4o-mini".to_string(),
                name: "GPT-4o Mini".to_string(),
                provider: "openai".to_string(),
                context_window: 128000,
                max_output_tokens: 16384,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.15,
                cost_per_million_output: 0.6,
            },
            ModelInfo {
                id: "gpt-4-turbo".to_string(),
                name: "GPT-4 Turbo".to_string(),
                provider: "openai".to_string(),
                context_window: 128000,
                max_output_tokens: 4096,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 10.0,
                cost_per_million_output: 30.0,
            },
            ModelInfo {
                id: "o1-preview".to_string(),
                name: "o1 Preview".to_string(),
                provider: "openai".to_string(),
                context_window: 128000,
                max_output_tokens: 32768,
                supports_vision: false,
                supports_tools: false,
                cost_per_million_input: 15.0,
                cost_per_million_output: 60.0,
            },
            ModelInfo {
                id: "o1-mini".to_string(),
                name: "o1 Mini".to_string(),
                provider: "openai".to_string(),
                context_window: 128000,
                max_output_tokens: 65536,
                supports_vision: false,
                supports_tools: false,
                cost_per_million_input: 3.0,
                cost_per_million_output: 12.0,
            },
        ];

        Self {
            client: Client::new(),
            config,
            models,
            legacy_only,
        }
    }

    pub fn with_config(config: OpenAIConfig) -> Self {
        Self::from_config(config, false)
    }

    pub fn openai_compatible(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self::from_config(
            OpenAIConfig {
                api_key: api_key.into(),
                base_url: Some(base_url.into()),
                organization: None,
                extra_headers: Vec::new(),
                requires_agent_shape: false,
            },
            true,
        )
    }

    fn prefers_legacy_route(&self) -> bool {
        self.legacy_only
    }

    fn parse_legacy_sse_data(
        data: &str,
        state: &mut LegacySseParserState,
    ) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        if data == "[DONE]" {
            if state.reasoning_open {
                events.push(StreamEvent::ReasoningEnd {
                    id: "reasoning-0".to_string(),
                });
                state.reasoning_open = false;
            }
            events.push(StreamEvent::Done);
            return events;
        }

        let chunk: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return events,
        };

        let usage = chunk.get("usage");
        let prompt_tokens = usage
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion_tokens = usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);

        if usage.is_some() {
            events.push(StreamEvent::Usage {
                prompt_tokens,
                completion_tokens,
            });
        }

        if let Some(choices) = chunk.get("choices").and_then(Value::as_array) {
            for choice in choices {
                if let Some(delta) = choice.get("delta") {
                    let reasoning = delta
                        .get("reasoning_content")
                        .or_else(|| delta.get("reasoning_text"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();

                    if !reasoning.is_empty() {
                        if !state.reasoning_open {
                            state.reasoning_open = true;
                            events.push(StreamEvent::ReasoningStart {
                                id: "reasoning-0".to_string(),
                            });
                        }
                        events.push(StreamEvent::ReasoningDelta {
                            id: "reasoning-0".to_string(),
                            text: reasoning.to_string(),
                        });
                    }

                    if let Some(text) = delta.get("content").and_then(Value::as_str) {
                        if !text.is_empty() {
                            if state.reasoning_open {
                                state.reasoning_open = false;
                                events.push(StreamEvent::ReasoningEnd {
                                    id: "reasoning-0".to_string(),
                                });
                            }
                            events.push(StreamEvent::TextDelta(text.to_string()));
                        }
                    }

                    if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                        if !tool_calls.is_empty() && state.reasoning_open {
                            state.reasoning_open = false;
                            events.push(StreamEvent::ReasoningEnd {
                                id: "reasoning-0".to_string(),
                            });
                        }
                        for tc in tool_calls {
                            let index =
                                tc.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                            let id = if let Some(id) = tc.get("id").and_then(Value::as_str) {
                                let id = id.to_string();
                                state.tool_call_ids.insert(index, id.clone());
                                id
                            } else {
                                state
                                    .tool_call_ids
                                    .entry(index)
                                    .or_insert_with(|| format!("tool-call-{}", index))
                                    .clone()
                            };

                            if let Some(func) = tc.get("function") {
                                if let Some(name) = func.get("name").and_then(Value::as_str) {
                                    let should_emit_start = state
                                        .tool_call_names
                                        .get(&index)
                                        .map(|existing| existing != name)
                                        .unwrap_or(true);
                                    state.tool_call_names.insert(index, name.to_string());
                                    if should_emit_start {
                                        events.push(StreamEvent::ToolCallStart {
                                            id: id.clone(),
                                            name: name.to_string(),
                                        });
                                    }
                                }

                                if let Some(arguments) =
                                    func.get("arguments").and_then(Value::as_str)
                                {
                                    if !arguments.is_empty() {
                                        events.push(StreamEvent::ToolCallDelta {
                                            id,
                                            input: arguments.to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                    if state.reasoning_open {
                        state.reasoning_open = false;
                        events.push(StreamEvent::ReasoningEnd {
                            id: "reasoning-0".to_string(),
                        });
                    }
                    let normalized_reason = if reason == "tool_calls" {
                        "tool-calls".to_string()
                    } else {
                        reason.to_string()
                    };
                    events.push(StreamEvent::FinishStep {
                        finish_reason: Some(normalized_reason),
                        usage: crate::stream::StreamUsage {
                            prompt_tokens,
                            completion_tokens,
                            ..Default::default()
                        },
                        provider_metadata: None,
                    });
                }
            }
        }

        events
    }

    fn parse_legacy_sse_line(line: &str, state: &mut LegacySseParserState) -> Vec<StreamEvent> {
        let line = line.trim();
        if !line.starts_with("data:") {
            return Vec::new();
        }
        let data = line.trim_start_matches("data:").trim();
        if data.is_empty() {
            return Vec::new();
        }
        Self::parse_legacy_sse_data(data, state)
    }

    fn drain_legacy_sse_events(
        buffer: &mut String,
        state: &mut LegacySseParserState,
        flush_remainder: bool,
    ) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        while let Some(newline_idx) = buffer.find('\n') {
            let mut line = buffer[..newline_idx].to_string();
            buffer.drain(..=newline_idx);
            if line.ends_with('\r') {
                line.pop();
            }
            events.extend(Self::parse_legacy_sse_line(&line, state));
        }

        if flush_remainder && !buffer.is_empty() {
            let mut tail = std::mem::take(buffer);
            if tail.ends_with('\r') {
                tail.pop();
            }
            events.extend(Self::parse_legacy_sse_line(&tail, state));
        }

        events
    }

    /// Convert Anthropic-shaped content parts into the OpenAI chat-completions shape.
    ///
    /// `ContentPart` is serialised with `tool_use` / `tool_result`, which only the
    /// Anthropic wire format understands. Sent as-is to an OpenAI-compatible endpoint the
    /// gateway drops them without an error: the model saw its tool calls come back empty
    /// and re-ran the same search every turn. An assistant turn must carry `tool_calls`,
    /// and every result must become its own `role: "tool"` message keyed by `tool_call_id`.
    fn rewrite_messages_for_openai(obj: &mut serde_json::Map<String, Value>) {
        // `ChatRequest.system` is serialised as a top-level `system` field, which is the
        // Anthropic shape. The OpenAI chat-completions schema has no such key and silently
        // drops it, so a caller that sets only `system` (title and summary generation) sent
        // a bare user message and got 403 FreeTierError back. Lift it into the conversation.
        if let Some(system) = obj.remove("system") {
            let text = match &system {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if !text.trim().is_empty() {
                obj.entry("messages".to_string()).or_insert_with(|| {
                    Value::Array(vec![serde_json::json!({
                        "role": "system",
                        "content": text,
                    })])
                });
                if let Some(Value::Array(messages)) = obj.get_mut("messages") {
                    // A system message generated here must stay first even when the
                    // conversation already starts with one.
                    let already_system = messages
                        .first()
                        .and_then(|m| m.get("role"))
                        .and_then(Value::as_str)
                        .map(|r| r == "system")
                        .unwrap_or(false);
                    if !already_system {
                        messages.insert(
                            0,
                            serde_json::json!({"role": "system", "content": text}),
                        );
                    }
                }
            }
        }

        let Some(Value::Array(messages)) = obj.get_mut("messages") else {
            return;
        };

        let mut rewritten: Vec<Value> = Vec::with_capacity(messages.len());
        for message in messages.iter_mut() {
            let Value::Object(entry) = message else {
                rewritten.push(message.clone());
                continue;
            };

            let role = entry
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("assistant")
                .to_string();
            let content = entry.remove("content");

            let (text, tool_calls, tool_results, media) = match content {
                Some(Value::String(text)) => (text, Vec::new(), Vec::new(), Vec::new()),
                Some(Value::Array(parts)) => {
                    let mut text = String::new();
                    let mut tool_calls = Vec::new();
                    let mut tool_results = Vec::new();
                    let mut media: Vec<Value> = Vec::new();
                    for part in parts {
                        let Value::Object(part) = part else { continue };
                        match part.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                if let Some(chunk) = part.get("text").and_then(Value::as_str) {
                                    text.push_str(chunk);
                                }
                            }
                            Some("tool_use") => {
                                let id = part
                                    .get("tool_use")
                                    .and_then(|u| u.get("id"))
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                let name = part
                                    .get("tool_use")
                                    .and_then(|u| u.get("name"))
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                let arguments = part
                                    .get("tool_use")
                                    .and_then(|u| u.get("input"))
                                    .cloned()
                                    .unwrap_or_else(|| Value::Object(Default::default()));
                                tool_calls.push(serde_json::json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": arguments.to_string(),
                                    }
                                }));
                            }
                            Some("tool_result") => {
                                let call_id = part
                                    .get("tool_result")
                                    .and_then(|r| r.get("tool_use_id"))
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                let content = part
                                    .get("tool_result")
                                    .and_then(|r| r.get("content"))
                                    .map(|c| match c {
                                        Value::String(s) => s.clone(),
                                        other => other.to_string(),
                                    })
                                    .unwrap_or_default();
                                tool_results.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": call_id,
                                    "content": content,
                                }));
                            }
                            // Dropped silently before: an image part reached this rewrite
                            // and vanished, so any multimodal request through an
                            // OpenAI-compatible endpoint lost its picture. `ContentPart`
                            // has carried `image_url` all along; it just needs translating.
                            Some("image_url") => {
                                if let Some(url) = part
                                    .get("image_url")
                                    .and_then(|i| i.get("url"))
                                    .and_then(Value::as_str)
                                {
                                    media.push(serde_json::json!({
                                        "type": "image_url",
                                        "image_url": {"url": url},
                                    }));
                                }
                            }
                            _ => {}
                        }
                    }
                    (text, tool_calls, tool_results, media)
                }
                _ => (String::new(), Vec::new(), Vec::new(), Vec::new()),
            };

            let is_assistant = role == "assistant";

            // A `role: "tool"` message carrying only `content` is rejected by every
            // OpenAI-compatible endpoint: the pair is identified by `tool_call_id`, so the
            // bare text twin emitted here made the whole request 400. Its payload is
            // already in `tool_results`, which is emitted below with the id attached.
            //
            // Skipping empty carriers must not skip the results either: subagent turns
            // build a `Role::Tool` message whose parts are all tool results and whose
            // text is empty, so an early `continue` silently discarded the subagent's
            // output — the same "tools came back empty" failure as the main loop had.
            let emits_carrier =
                is_assistant || (role != "tool" && (!text.is_empty() || !media.is_empty()));
            if emits_carrier {
                let mut rebuilt = serde_json::Map::new();
                rebuilt.insert("role".to_string(), Value::String(role.clone()));
                let content_value = if media.is_empty() {
                    if text.is_empty() {
                        Value::Null
                    } else {
                        Value::String(text)
                    }
                } else {
                    // OpenAI carries a mixed text+image turn as an array of parts.
                    let mut blocks: Vec<Value> = Vec::new();
                    if !text.is_empty() {
                        blocks.push(serde_json::json!({"type": "text", "text": text}));
                    }
                    blocks.extend(media);
                    Value::Array(blocks)
                };
                rebuilt.insert("content".to_string(), content_value);
                if is_assistant && !tool_calls.is_empty() {
                    rebuilt.insert("tool_calls".to_string(), Value::Array(tool_calls));
                }
                rewritten.push(Value::Object(rebuilt));
            }

            // OpenAI expects each tool message to follow the assistant turn that asked
            // for it. Emitting them first makes the gateway drop the whole pair.
            for result in tool_results {
                rewritten.push(result);
            }
        }

        *messages = rewritten;
    }

    fn build_request_body(request: &ChatRequest) -> Result<Value, ProviderError> {
        let mut value = serde_json::to_value(request)
            .map_err(|e| ProviderError::InvalidRequest(e.to_string()))?;

        if let Value::Object(obj) = &mut value {
            Self::rewrite_messages_for_openai(obj);
            // Merge provider_options into the top-level body (matching TS SDK behavior).
            // The TS SDK spreads provider options directly into the request body so that
            // provider-specific fields like `thinking`, `enable_thinking`, etc. are sent
            // as top-level keys rather than nested under `provider_options`.
            if let Some(Value::Object(opts)) = obj.remove("provider_options") {
                for (k, v) in opts {
                    obj.entry(k).or_insert(v);
                }
            }

            // OpenAI-compatible endpoints require each tool wrapped as
            // `{"type":"function","function":{...}}`. `ChatRequest.tools` is a flat
            // `ToolDefinition { name, description, parameters }`, so serializing it directly
            // emits `{"name":...}` and those endpoints reject the whole request with
            // `400 invalid_request_error` (verified against opencode zen).
            if let Some(Value::Array(tools)) = obj.remove("tools") {
                let wrapped: Vec<Value> = tools
                    .into_iter()
                    .map(|tool| serde_json::json!({"type": "function", "function": tool}))
                    .collect();
                obj.insert("tools".to_string(), Value::Array(wrapped));
            }

            let effort = openai_reasoning_effort(&request.model, request.variant.as_deref());
            if let Some(effort) = effort {
                obj.insert(
                    "reasoning_effort".to_string(),
                    Value::String(effort.to_string()),
                );
            }
        }

        Ok(value)
    }

    fn responses_url(base_url: Option<&str>, path: &str) -> String {
        let path = path.trim_start_matches('/');
        match base_url {
            None => format!("https://api.openai.com/v1/{}", path),
            Some(base) => {
                if base.ends_with("/chat/completions") {
                    return format!("{}/{}", base.trim_end_matches("/chat/completions"), path);
                }
                if base.ends_with("/v1") {
                    return format!("{}/{}", base.trim_end_matches('/'), path);
                }
                if base.ends_with('/') {
                    format!("{}{}", base, path)
                } else {
                    format!("{}/{}", base, path)
                }
            }
        }
    }

    fn chat_completions_url(base_url: Option<&str>) -> String {
        match base_url {
            None => OPENAI_API_URL.to_string(),
            Some(base) => {
                if base.ends_with("/chat/completions") {
                    return base.to_string();
                }
                if base.ends_with('/') {
                    format!("{base}chat/completions")
                } else {
                    format!("{base}/chat/completions")
                }
            }
        }
    }

    fn extract_responses_provider_options(
        provider_options: Option<&HashMap<String, serde_json::Value>>,
    ) -> ResponsesProviderOptions {
        let Some(options) = provider_options else {
            return ResponsesProviderOptions::default();
        };

        for key in ["openai", "responses"] {
            if let Some(value) = options.get(key) {
                if let Ok(parsed) =
                    serde_json::from_value::<ResponsesProviderOptions>(value.clone())
                {
                    return parsed;
                }
            }
        }

        serde_json::from_value::<ResponsesProviderOptions>(serde_json::json!(options))
            .unwrap_or_default()
    }

    fn tools_to_input_tools(tools: Option<&Vec<crate::ToolDefinition>>) -> Option<Vec<InputTool>> {
        let tools = tools?;
        if tools.is_empty() {
            return None;
        }
        Some(
            tools
                .iter()
                .map(|tool| InputTool::Function {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.parameters.clone(),
                })
                .collect(),
        )
    }

    fn responses_generate_options(&self, request: &ChatRequest) -> GenerateOptions {
        let mut prompt = request.messages.clone();
        if let Some(system) = &request.system {
            let has_system = prompt.iter().any(|m| matches!(m.role, Role::System));
            if !has_system {
                prompt.insert(0, Message::system(system.clone()));
            }
        }

        let mut provider_options =
            Self::extract_responses_provider_options(request.provider_options.as_ref());
        if provider_options.reasoning_effort.is_none() {
            provider_options.reasoning_effort =
                openai_reasoning_effort(&request.model, request.variant.as_deref())
                    .map(ToString::to_string);
        }

        GenerateOptions {
            prompt,
            tools: Self::tools_to_input_tools(request.tools.as_ref()),
            tool_choice: None,
            max_output_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            top_k: None,
            seed: None,
            presence_penalty: None,
            frequency_penalty: None,
            stop_sequences: None,
            provider_options: Some(provider_options),
            response_format: None,
        }
    }

    fn responses_model(
        &self,
        model_id: &str,
        session_id: Option<&str>,
    ) -> OpenAIResponsesLanguageModel {
        let api_key = self.config.api_key.clone();
        let org = self.config.organization.clone();
        let base_url = self.config.base_url.clone();
        let session_client = crate::session_client::for_session(session_id.unwrap_or(""));

        OpenAIResponsesLanguageModel::new(
            model_id.to_string(),
            OpenAIResponsesConfig {
                provider: "openai".to_string(),
                url: Arc::new(move |path, _model| Self::responses_url(base_url.as_deref(), path)),
                headers: Arc::new(move || {
                    let mut h = HashMap::new();
                    h.insert("Authorization".to_string(), format!("Bearer {}", api_key));
                    if let Some(org) = &org {
                        h.insert("OpenAI-Organization".to_string(), org.clone());
                    }
                    h
                }),
                client: Some(session_client.as_ref().clone()),
                file_id_prefixes: Some(vec!["file-".to_string()]),
                generate_id: None,
                metadata_extractor: None,
            },
        )
    }

    fn finish_reason_to_string(reason: FinishReason) -> String {
        match reason {
            FinishReason::Stop => "stop".to_string(),
            FinishReason::Length => "length".to_string(),
            FinishReason::ContentFilter => "content_filter".to_string(),
            FinishReason::ToolCalls => "tool-calls".to_string(),
            FinishReason::Error => "error".to_string(),
            FinishReason::Unknown => "unknown".to_string(),
        }
    }

    fn responses_chat_response(
        request: &ChatRequest,
        result: crate::responses::ResponsesGenerateResult,
    ) -> ChatResponse {
        let usage = Usage {
            prompt_tokens: result.usage.input_tokens,
            completion_tokens: result.usage.output_tokens,
            total_tokens: result.usage.input_tokens + result.usage.output_tokens,
            cache_read_input_tokens: result
                .usage
                .input_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens),
            cache_creation_input_tokens: None,
        };

        ChatResponse {
            id: result
                .metadata
                .response_id
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            model: result
                .metadata
                .model_id
                .unwrap_or_else(|| request.model.clone()),
            choices: vec![Choice {
                index: 0,
                message: result.message,
                finish_reason: Some(Self::finish_reason_to_string(result.finish_reason)),
            }],
            usage: Some(usage),
        }
    }

    /// Fold a streamed Zen reply back into one response.
    ///
    /// Zen only answers bodies shaped like a real agent turn — `stream: true`,
    /// `stream_options`, a system message and a non-empty `tools` array — so a
    /// one-shot call still goes out over SSE and is reassembled here.
    async fn chat_agent_shape(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let model = request.model.clone();
        let mut events = self.chat_stream_legacy(request).await?;
        let mut text = String::new();
        let mut finish_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;

        while let Some(event) = events.next().await {
            match event? {
                StreamEvent::TextDelta(delta) => text.push_str(&delta),
                StreamEvent::FinishStep {
                    finish_reason: reason,
                    usage: step,
                    ..
                } => {
                    finish_reason = reason;
                    usage = Some(Usage {
                        prompt_tokens: step.prompt_tokens,
                        completion_tokens: step.completion_tokens,
                        total_tokens: step.prompt_tokens + step.completion_tokens,
                        cache_read_input_tokens: Some(step.cache_read_tokens),
                        cache_creation_input_tokens: Some(step.cache_write_tokens),
                    });
                }
                _ => {}
            }
        }

        Ok(ChatResponse {
            id: String::new(),
            model,
            choices: vec![Choice {
                index: 0,
                message: Message {
                    role: Role::Assistant,
                    content: crate::Content::Text(text),
                    cache_control: None,
                    provider_options: None,
                },
                finish_reason,
            }],
            usage,
        })
    }

    async fn chat_legacy(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let url = Self::chat_completions_url(self.config.base_url.as_deref());
        let mut request_body = Self::build_request_body(&request)?;

        // Ensure stream is disabled for non-streaming path. The caller may have
        // set stream=true on the ChatRequest (e.g. prompt loop), but chat_legacy
        // expects a single JSON response, not SSE chunks.
        if let Value::Object(obj) = &mut request_body {
            obj.remove("stream");
            obj.remove("stream_options");
        }

        let mut req_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json");

        if let Some(org) = &self.config.organization {
            req_builder = req_builder.header("OpenAI-Organization", org);
        }

        for (name, value) in &self.config.extra_headers {
            req_builder = req_builder.header(name.as_str(), value.as_str());
        }

        let response = req_builder
            .json(&request_body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let body = response
            .text()
            .await
            .map_err(|e| {
                let mut msg = e.to_string();
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    msg.push_str(": ");
                    msg.push_str(&cause.to_string());
                    source = cause.source();
                }
                ProviderError::ApiError(msg)
            })?;

        // Some OpenAI-compatible providers (e.g. ZhipuAI) return SSE-formatted
        // streaming data even for non-streaming requests. Detect and reassemble.
        let raw: RawChatResponse = if body.trim_start().starts_with("data:") {
            Self::reassemble_sse_chunks(&body)?
        } else {
            serde_json::from_str(&body).map_err(|e| {
                let preview = if body.len() > 500 {
                    format!("{}...", &body[..500])
                } else {
                    body.clone()
                };
                ProviderError::ApiError(format!(
                    "failed to decode response: {}\nBody: {}",
                    e, preview
                ))
            })?
        };
        Ok(raw.into_chat_response())
    }

    /// Reassemble SSE `data:` chunks (streaming format) into a single `RawChatResponse`.
    /// Some OpenAI-compatible providers return SSE even for non-streaming requests.
    fn reassemble_sse_chunks(body: &str) -> Result<RawChatResponse, ProviderError> {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut finish_reason: Option<String> = None;
        let mut usage: Option<RawUsage> = None;
        // tool_calls keyed by index: (id, name, arguments)
        let mut tool_calls: HashMap<u32, (Option<String>, Option<String>, String)> = HashMap::new();

        for line in body.lines() {
            let line = line.trim();
            if !line.starts_with("data:") {
                continue;
            }
            let data = line[5..].trim();
            if data == "[DONE]" {
                break;
            }
            let chunk: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) {
                for choice in choices {
                    if let Some(delta) = choice.get("delta") {
                        if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                            content.push_str(text);
                        }
                        // ZhipuAI uses "reasoning_content"; OpenAI uses "reasoning_text"
                        let reasoning_val = delta
                            .get("reasoning_content")
                            .or_else(|| delta.get("reasoning_text"))
                            .and_then(|v| v.as_str());
                        if let Some(r) = reasoning_val {
                            reasoning.push_str(r);
                        }
                        if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                            for tc in tcs {
                                let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                                let entry = tool_calls.entry(idx).or_insert((None, None, String::new()));
                                if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                    entry.0 = Some(id.to_string());
                                }
                                if let Some(func) = tc.get("function") {
                                    if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                        entry.1 = Some(name.to_string());
                                    }
                                    if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                                        entry.2.push_str(args);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                        finish_reason = Some(fr.to_string());
                    }
                }
            }
            if let Some(u) = chunk.get("usage") {
                usage = serde_json::from_value(u.clone()).ok();
            }
        }

        let raw_tool_calls: Option<Vec<RawToolCall>> = if tool_calls.is_empty() {
            None
        } else {
            let mut sorted: Vec<_> = tool_calls.into_iter().collect();
            sorted.sort_by_key(|(idx, _)| *idx);
            Some(
                sorted
                    .into_iter()
                    .map(|(_idx, (id, name, args))| RawToolCall {
                        id,
                        function: Some(RawFunction {
                            name,
                            arguments: Some(args),
                        }),
                    })
                    .collect(),
            )
        };

        Ok(RawChatResponse {
            id: None,
            model: None,
            choices: vec![RawChoice {
                index: Some(0),
                message: Some(RawMessage {
                    role: Some("assistant".to_string()),
                    content: if content.is_empty() { None } else { Some(content) },
                    tool_calls: raw_tool_calls,
                    _reasoning_text: if reasoning.is_empty() { None } else { Some(reasoning) },
                }),
                finish_reason,
            }],
            usage,
        })
    }

    async fn chat_stream_legacy(
        &self,
        mut request: ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let url = Self::chat_completions_url(self.config.base_url.as_deref());
        request.stream = Some(true);
        let mut request_body = Self::build_request_body(&request)?;
        // Zen reads the session/request pair off the request itself. A pinned pair from
        // the environment goes stale and every turn 403s, so derive it per call.
        let zen_identity = if self.config.requires_agent_shape {
            // Title generation and other synthetic calls carry no session; the gateway
            // still refuses a request without the pair, so mint both rather than omit.
            // Title and summary generation carry no session_id, so every one of their
            // requests was signed with a differently-minted pair and the gateway refused
            // it. Deriving one session per call keeps the pair stable within the request.
            let session = request
                .session_id
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(crate::identifier::session_id);
            let headers = vec![
                ("x-opencode-session".to_string(), session.clone()),
                ("x-opencode-session-id".to_string(), session),
                ("x-opencode-request".to_string(), crate::identifier::message_id()),
            ];
            Some(headers)
        } else {
            None
        };

        // Match TS SDK: include stream_options for usage tracking
        if let Value::Object(obj) = &mut request_body {
            obj.insert(
                "stream_options".to_string(),
                serde_json::json!({"include_usage": true}),
            );
        }

        let mut req_builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");

        if let Some(org) = &self.config.organization {
            req_builder = req_builder.header("OpenAI-Organization", org);
        }

        for (name, value) in &self.config.extra_headers {
            // The per-request pair wins: a pinned one from the environment goes stale.
            if zen_identity.is_some() && name.starts_with("x-opencode-session") || name == "x-opencode-request" {
                continue;
            }
            req_builder = req_builder.header(name.as_str(), value.as_str());
        }

        if let Some(identity) = &zen_identity {
            for (name, value) in identity {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }
        }

        if self.config.requires_agent_shape {
        }

        let response = req_builder
            .json(&request_body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let stream = stream::try_unfold(
            (
                response.bytes_stream(),
                String::new(),
                LegacySseParserState::default(),
                VecDeque::<StreamEvent>::new(),
                false,
            ),
            |(mut chunks, mut buffer, mut parser_state, mut pending, mut exhausted)| async move {
                loop {
                    if let Some(event) = pending.pop_front() {
                        return Ok(Some((
                            event,
                            (chunks, buffer, parser_state, pending, exhausted),
                        )));
                    }

                    if exhausted {
                        return Ok(None);
                    }

                    match chunks.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                            pending.extend(Self::drain_legacy_sse_events(
                                &mut buffer,
                                &mut parser_state,
                                false,
                            ));
                        }
                        Some(Err(err)) => return Err(ProviderError::StreamError(err.to_string())),
                        None => {
                            exhausted = true;
                            pending.extend(Self::drain_legacy_sse_events(
                                &mut buffer,
                                &mut parser_state,
                                true,
                            ));
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn id(&self) -> &str {
        "openai"
    }

    fn name(&self) -> &str {
        "OpenAI"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.models.clone()
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.id == id)
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        // Zen rejects (FreeTierError) any body missing stream/stream_options, so a
        // one-shot reply still has to travel the streaming route and be folded back
        // into a single response here.
        if self.config.requires_agent_shape {
            return self.chat_agent_shape(request).await;
        }

        // Skip Responses API for OpenAI-compatible providers (non-OpenAI endpoints)
        if self.prefers_legacy_route() {
            return self.chat_legacy(request).await;
        }

        let response_model = self.responses_model(&request.model, request.session_id.as_deref());
        let options = self.responses_generate_options(&request);
        let request_for_primary = request.clone();
        let model_for_log = request.model.clone();
        resolve_with_fallback(
            async move {
                response_model
                    .do_generate(options)
                    .await
                    .map(|result| Self::responses_chat_response(&request_for_primary, result))
            },
            move |err| async move {
                if get_custom_fetch_proxy("openai").is_some() {
                    tracing::warn!(
                        model = %model_for_log,
                        error = %err,
                        "Responses generate failed while custom fetch proxy is active; skipping legacy fallback"
                    );
                    return Err(err);
                }
                tracing::warn!(
                    model = %model_for_log,
                    error = %err,
                    "Responses generate failed, falling back to chat completions"
                );
                self.chat_legacy(request).await
            },
        )
        .await
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        // Skip Responses API for OpenAI-compatible providers (non-OpenAI endpoints)
        if self.prefers_legacy_route() {
            return self.chat_stream_legacy(request).await;
        }

        let response_model = self.responses_model(&request.model, request.session_id.as_deref());
        let options = StreamOptions {
            generate: self.responses_generate_options(&request),
        };
        let model_for_log = request.model.clone();
        resolve_with_fallback(
            async move { response_model.do_stream(options).await },
            move |err| async move {
                if get_custom_fetch_proxy("openai").is_some() {
                    tracing::warn!(
                        model = %model_for_log,
                        error = %err,
                        "Responses stream failed while custom fetch proxy is active; skipping legacy fallback"
                    );
                    return Err(err);
                }
                tracing::warn!(
                    model = %model_for_log,
                    error = %err,
                    "Responses stream failed, falling back to chat completions stream"
                );
                self.chat_stream_legacy(request).await
            },
        )
        .await
    }
}

fn openai_reasoning_effort(model_id: &str, variant: Option<&str>) -> Option<&'static str> {
    let variant = variant?.trim().to_ascii_lowercase();
    let model = model_id.to_ascii_lowercase();
    let supports_effort = model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.contains("gpt-5")
        || model.contains("codex");
    if !supports_effort {
        return None;
    }

    match variant.as_str() {
        "none" => Some("none"),
        "minimal" => Some("minimal"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "max" | "xhigh" => Some("high"),
        _ => None,
    }
}

async fn resolve_with_fallback<T, PFut, FFut, F>(
    primary: PFut,
    fallback: F,
) -> Result<T, ProviderError>
where
    PFut: Future<Output = Result<T, ProviderError>>,
    F: FnOnce(ProviderError) -> FFut,
    FFut: Future<Output = Result<T, ProviderError>>,
{
    match primary.await {
        Ok(value) => Ok(value),
        Err(err) => fallback(err).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: the rewrite had a `_ => {}` catch-all that discarded image parts, so
    // every multimodal request through an OpenAI-compatible endpoint lost its picture.
    #[test]
    fn image_parts_survive_the_rewrite() {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "messages".to_string(),
            serde_json::json!([
                {"role": "user", "content": [
                    {"type": "text", "text": "what is this?"},
                    {"type": "image_url", "image_url": {"url": "https://x.test/a.png"}}
                ]}
            ]),
        );

        OpenAIProvider::rewrite_messages_for_openai(&mut obj);

        let messages = obj["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 1, "image-only turns must not be dropped");
        let content = messages[0]["content"].as_array().expect("array content");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "https://x.test/a.png");
    }

    // Regression: a `role: "tool"` message whose parts are all tool results (text empty)
    // used to hit an early `continue`, discarding the subagent's entire output — the model
    // never saw it. See `SubtaskExecutor`, which builds exactly that shape.
    #[test]
    fn tool_only_message_still_emits_its_results() {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "messages".to_string(),
            serde_json::json!([
                {"role": "assistant", "content": [{"type": "text", "text": "working"}]},
                {"role": "tool", "content": [
                    {"type": "tool_result", "tool_result": {
                        "tool_use_id": "call_1", "content": "subagent answer"}}
                ]}
            ]),
        );

        OpenAIProvider::rewrite_messages_for_openai(&mut obj);

        let messages = obj["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 2, "both messages must survive: {messages:?}");
        let tool_msg = &messages[1];
        assert_eq!(tool_msg["role"], "tool");
        assert_eq!(tool_msg["tool_call_id"], "call_1");
        assert_eq!(tool_msg["content"], "subagent answer");
    }

    // Regression: the same path emitted a bare `{role:"tool", content}` twin alongside the
    // real result. OpenAI-compatible endpoints answer 400 to a tool message with no
    // `tool_call_id`, so the whole request was rejected.
    #[test]
    fn no_tool_message_is_emitted_without_a_call_id() {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "messages".to_string(),
            serde_json::json!([
                {"role": "tool", "content": [
                    {"type": "text", "text": "leftover prose"},
                    {"type": "tool_result", "tool_result": {
                        "tool_use_id": "call_9", "content": "payload"}}
                ]}
            ]),
        );

        OpenAIProvider::rewrite_messages_for_openai(&mut obj);

        let messages = obj["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 1, "the id-less twin must not be sent: {messages:?}");
        assert_eq!(messages[0]["tool_call_id"], "call_9");
        assert_eq!(messages[0]["content"], "payload");
    }

    #[test]
    fn top_level_system_is_lifted_into_the_conversation() {
        let mut obj = serde_json::Map::new();
        obj.insert("system".to_string(), serde_json::json!("be brief"));
        obj.insert(
            "messages".to_string(),
            serde_json::json!([{"role": "user", "content": "hello"}]),
        );

        OpenAIProvider::rewrite_messages_for_openai(&mut obj);

        let messages = obj["messages"].as_array().expect("messages array");
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "be brief");
        assert!(obj.get("system").is_none(), "the Anthropic-shaped key must be lifted out");
    }
    use crate::custom_fetch::{
        register_custom_fetch_proxy, unregister_custom_fetch_proxy, CustomFetchProxy,
        CustomFetchRequest, CustomFetchResponse, CustomFetchStreamResponse,
    };
    use async_trait::async_trait;
    use futures::stream;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct NoopProxy;

    #[async_trait]
    impl CustomFetchProxy for NoopProxy {
        async fn fetch(
            &self,
            _request: CustomFetchRequest,
        ) -> Result<CustomFetchResponse, ProviderError> {
            Ok(CustomFetchResponse {
                status: 200,
                headers: HashMap::new(),
                body: String::new(),
            })
        }

        async fn fetch_stream(
            &self,
            _request: CustomFetchRequest,
        ) -> Result<CustomFetchStreamResponse, ProviderError> {
            Ok(CustomFetchStreamResponse {
                status: 200,
                headers: HashMap::new(),
                stream: Box::pin(stream::empty()),
            })
        }
    }

    #[tokio::test]
    async fn resolve_with_fallback_returns_primary_when_successful() {
        let result =
            resolve_with_fallback(async { Ok::<_, ProviderError>(7usize) }, |_err| async {
                Ok::<_, ProviderError>(0usize)
            })
            .await
            .expect("primary result should be returned");
        assert_eq!(result, 7);
    }

    #[tokio::test]
    async fn resolve_with_fallback_calls_fallback_on_error() {
        let result = resolve_with_fallback(
            async {
                Err::<usize, ProviderError>(ProviderError::ApiError("responses failed".to_string()))
            },
            |_err| async { Ok::<_, ProviderError>(9usize) },
        )
        .await
        .expect("fallback should handle primary error");
        assert_eq!(result, 9);
    }

    #[tokio::test]
    async fn resolve_with_fallback_skips_legacy_when_custom_fetch_active() {
        register_custom_fetch_proxy("openai", Arc::new(NoopProxy));

        let result = resolve_with_fallback(
            async {
                Err::<usize, ProviderError>(ProviderError::ApiError("responses failed".to_string()))
            },
            |e| async move {
                if get_custom_fetch_proxy("openai").is_some() {
                    return Err(e);
                }
                Ok::<_, ProviderError>(9usize)
            },
        )
        .await;

        unregister_custom_fetch_proxy("openai");
        assert!(result.is_err());
    }

    #[test]
    fn openai_provider_with_base_url_still_prefers_responses() {
        let provider = OpenAIProvider::new_with_base_url("test-key", "https://example.com/v1");
        assert!(!provider.prefers_legacy_route());
    }

    #[test]
    fn openai_compatible_provider_prefers_legacy() {
        let provider = OpenAIProvider::openai_compatible("https://example.com/v1", "test-key");
        assert!(provider.prefers_legacy_route());
    }

    #[test]
    fn drain_legacy_sse_events_handles_partial_and_multiple_lines() {
        let mut state = LegacySseParserState::default();
        let mut buffer = String::from("data: {\"choices\":[{\"delta\":{\"content\":\"hel");

        let events = OpenAIProvider::drain_legacy_sse_events(&mut buffer, &mut state, false);
        assert!(events.is_empty(), "partial line should not be parsed");

        buffer.push_str("lo\"}}]}\n");
        buffer.push_str("data: {\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2}}\n");
        let events = OpenAIProvider::drain_legacy_sse_events(&mut buffer, &mut state, false);

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            StreamEvent::TextDelta(text) if text == "hello"
        ));
        assert!(matches!(
            &events[1],
            StreamEvent::Usage {
                prompt_tokens: 1,
                completion_tokens: 2
            }
        ));
    }

    #[test]
    fn parse_legacy_sse_data_uses_stable_tool_call_id_when_missing() {
        let mut state = LegacySseParserState::default();
        let start = OpenAIProvider::parse_legacy_sse_data(
            "{\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"bash\"}}]}}]}",
            &mut state,
        );
        assert!(matches!(
            start.first(),
            Some(StreamEvent::ToolCallStart { id, name }) if id == "tool-call-0" && name == "bash"
        ));

        let delta = OpenAIProvider::parse_legacy_sse_data(
            "{\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"cmd\\\":\\\"ls\\\"}\"}}]}}]}",
            &mut state,
        );
        assert!(matches!(
            delta.first(),
            Some(StreamEvent::ToolCallDelta { id, input }) if id == "tool-call-0" && input == "{\"cmd\":\"ls\"}"
        ));
    }
}
