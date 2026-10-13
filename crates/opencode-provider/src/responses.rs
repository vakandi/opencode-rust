//! OpenAI Responses API types and streaming support.
//!
//! This module provides full feature parity with the TypeScript SDK's
//! `openai-responses-language-model.ts`, including:
//! - Response chunk types (12+ discriminated types)
//! - Streaming state management
//! - Response parsing schemas
//! - Model configuration detection
//! - Provider options schema

use futures::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::custom_fetch::{get_custom_fetch_proxy, CustomFetchRequest};
use crate::message::{Content, ContentPart, Message, Role, ToolResult, ToolUse};
use crate::provider::ProviderError;
use crate::responses_convert::convert_to_openai_responses_input;
use crate::stream::{StreamEvent, StreamResult, StreamUsage, ToolResultOutput};
use crate::tools::{prepare_responses_tools, InputTool, InputToolChoice, ResponsesTool};

// ---------------------------------------------------------------------------
// Provider Options
// ---------------------------------------------------------------------------

/// Include values for the Responses API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResponsesIncludeValue {
    #[serde(rename = "web_search_call.action.sources")]
    WebSearchCallActionSources,
    #[serde(rename = "code_interpreter_call.outputs")]
    CodeInterpreterCallOutputs,
    #[serde(rename = "computer_call_output.output.image_url")]
    ComputerCallOutputImageUrl,
    #[serde(rename = "file_search_call.results")]
    FileSearchCallResults,
    #[serde(rename = "message.input_image.image_url")]
    MessageInputImageUrl,
    #[serde(rename = "message.output_text.logprobs")]
    MessageOutputTextLogprobs,
    #[serde(rename = "reasoning.encrypted_content")]
    ReasoningEncryptedContent,
}

/// Service tier options.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceTier {
    Auto,
    Flex,
    Priority,
}

/// Text verbosity options.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextVerbosity {
    Low,
    Medium,
    High,
}

/// Reasoning effort levels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

/// Reasoning summary mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    Auto,
    Concise,
    Detailed,
}

/// Provider-specific options for the OpenAI Responses API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResponsesProviderOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<ResponsesIncludeValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Return log probabilities. `true` = max (20), or a number 1..=20.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<LogprobsSetting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<ServiceTier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict_json_schema: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_verbosity: Option<TextVerbosity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// Logprobs can be `true` (use max=20) or a specific number 1..=20.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LogprobsSetting {
    Enabled(bool),
    TopN(u8),
}

impl LogprobsSetting {
    pub fn top_logprobs(&self) -> Option<u8> {
        match self {
            LogprobsSetting::Enabled(true) => Some(TOP_LOGPROBS_MAX),
            LogprobsSetting::Enabled(false) => None,
            LogprobsSetting::TopN(n) => Some(*n),
        }
    }
}

pub const TOP_LOGPROBS_MAX: u8 = 20;

// ---------------------------------------------------------------------------
// Model Configuration
// ---------------------------------------------------------------------------

/// Determines model capabilities based on model ID.
#[derive(Debug, Clone)]
pub struct ResponsesModelConfig {
    pub is_reasoning_model: bool,
    pub system_message_mode: SystemMessageMode,
    pub required_auto_truncation: bool,
    pub supports_flex_processing: bool,
    pub supports_priority_processing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemMessageMode {
    System,
    Developer,
    Remove,
}

/// Determine model capabilities from model ID string.
/// Mirrors the TS `getResponsesModelConfig()`.
pub fn get_responses_model_config(model_id: &str) -> ResponsesModelConfig {
    let supports_flex = model_id.starts_with("o3")
        || model_id.starts_with("o4-mini")
        || (model_id.starts_with("gpt-5") && !model_id.starts_with("gpt-5-chat"));

    let supports_priority = model_id.starts_with("gpt-4")
        || model_id.starts_with("gpt-5-mini")
        || (model_id.starts_with("gpt-5")
            && !model_id.starts_with("gpt-5-nano")
            && !model_id.starts_with("gpt-5-chat"))
        || model_id.starts_with("o3")
        || model_id.starts_with("o4-mini");

    let defaults = ResponsesModelConfig {
        is_reasoning_model: false,
        system_message_mode: SystemMessageMode::System,
        required_auto_truncation: false,
        supports_flex_processing: supports_flex,
        supports_priority_processing: supports_priority,
    };

    // gpt-5-chat models are non-reasoning
    if model_id.starts_with("gpt-5-chat") {
        return defaults;
    }

    // o series reasoning models, gpt-5, codex-, computer-use
    if model_id.starts_with('o')
        || model_id.starts_with("gpt-5")
        || model_id.starts_with("codex-")
        || model_id.starts_with("computer-use")
    {
        if model_id.starts_with("o1-mini") || model_id.starts_with("o1-preview") {
            return ResponsesModelConfig {
                is_reasoning_model: true,
                system_message_mode: SystemMessageMode::Remove,
                ..defaults
            };
        }
        return ResponsesModelConfig {
            is_reasoning_model: true,
            system_message_mode: SystemMessageMode::Developer,
            ..defaults
        };
    }

    // gpt models (non-reasoning)
    defaults
}

// ---------------------------------------------------------------------------
// Finish Reason Mapping
// ---------------------------------------------------------------------------

/// Maps OpenAI Responses API incomplete_details.reason to a finish reason.
/// Mirrors TS `mapOpenAIResponseFinishReason()`.
pub fn map_openai_response_finish_reason(
    finish_reason: Option<&str>,
    has_function_call: bool,
) -> FinishReason {
    match finish_reason {
        None => {
            if has_function_call {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }
        }
        Some("max_output_tokens") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some(_) => {
            if has_function_call {
                FinishReason::ToolCalls
            } else {
                FinishReason::Unknown
            }
        }
    }
}

/// Maps OpenAI Compatible chat finish_reason strings.
/// Mirrors TS `mapOpenAICompatibleFinishReason()`.
pub fn map_openai_compatible_finish_reason(finish_reason: Option<&str>) -> FinishReason {
    match finish_reason {
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some("function_call") | Some("tool_calls") => FinishReason::ToolCalls,
        _ => FinishReason::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Unknown,
}

// ---------------------------------------------------------------------------
// Responses API Input Types
// ---------------------------------------------------------------------------

/// Input items for the Responses API request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesInputItem {
    FunctionCall(ResponsesFunctionCall),
    FunctionCallOutput(ResponsesFunctionCallOutput),
    LocalShellCall(ResponsesLocalShellCall),
    LocalShellCallOutput(ResponsesLocalShellCallOutput),
    Reasoning(ResponsesReasoning),
    ItemReference(ResponsesItemReference),
    /// System or developer message (role-based, not type-tagged).
    #[serde(untagged)]
    RoleMessage(ResponsesRoleMessage),
}

/// We need a custom serialization approach since the input items mix
/// tagged and untagged variants. Use `serde_json::Value` as the wire type.
pub type ResponsesInput = Vec<serde_json::Value>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesRoleMessage {
    pub role: String, // "system" | "developer" | "user" | "assistant"
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesFunctionCall {
    #[serde(rename = "type")]
    pub item_type: String, // "function_call"
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesFunctionCallOutput {
    #[serde(rename = "type")]
    pub item_type: String, // "function_call_output"
    pub call_id: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesLocalShellCall {
    #[serde(rename = "type")]
    pub item_type: String, // "local_shell_call"
    pub id: Option<String>,
    pub call_id: String,
    pub action: LocalShellAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalShellAction {
    #[serde(rename = "type")]
    pub action_type: String, // "exec"
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesLocalShellCallOutput {
    #[serde(rename = "type")]
    pub item_type: String, // "local_shell_call_output"
    pub call_id: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesReasoning {
    #[serde(rename = "type")]
    pub item_type: String, // "reasoning"
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
    pub summary: Vec<ReasoningSummaryText>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningSummaryText {
    #[serde(rename = "type")]
    pub text_type: String, // "summary_text"
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesItemReference {
    #[serde(rename = "type")]
    pub item_type: String, // "item_reference"
    pub id: String,
}

// ---------------------------------------------------------------------------
// Responses API Response / Output Types
// ---------------------------------------------------------------------------

/// Usage information from the Responses API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponsesUsage {
    pub input_tokens: u64,
    #[serde(default)]
    pub input_tokens_details: Option<InputTokensDetails>,
    pub output_tokens: u64,
    #[serde(default)]
    pub output_tokens_details: Option<OutputTokensDetails>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InputTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
}

// ---------------------------------------------------------------------------
// Streaming Chunk Types (12+ discriminated types)
// ---------------------------------------------------------------------------

/// All possible chunk types from the Responses API SSE stream.
/// Mirrors the TS `openaiResponsesChunkSchema` union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponsesStreamChunk {
    /// `response.output_text.delta`
    #[serde(rename = "response.output_text.delta")]
    TextDelta {
        item_id: String,
        delta: String,
        #[serde(default)]
        logprobs: Option<Vec<LogprobEntry>>,
    },

    /// `response.created`
    #[serde(rename = "response.created")]
    ResponseCreated { response: ResponseCreatedData },

    /// `response.completed` or `response.incomplete`
    #[serde(rename = "response.completed")]
    ResponseCompleted { response: ResponseFinishedData },
    #[serde(rename = "response.incomplete")]
    ResponseIncomplete { response: ResponseFinishedData },

    /// `response.output_item.added`
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        output_index: usize,
        item: OutputItemAddedItem,
    },

    /// `response.output_item.done`
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        output_index: usize,
        item: OutputItemDoneItem,
    },

    /// `response.function_call_arguments.delta`
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        item_id: String,
        output_index: usize,
        delta: String,
    },

    /// `response.image_generation_call.partial_image`
    #[serde(rename = "response.image_generation_call.partial_image")]
    ImageGenerationPartialImage {
        item_id: String,
        output_index: usize,
        partial_image_b64: String,
    },

    /// `response.code_interpreter_call_code.delta`
    #[serde(rename = "response.code_interpreter_call_code.delta")]
    CodeInterpreterCodeDelta {
        item_id: String,
        output_index: usize,
        delta: String,
    },

    /// `response.code_interpreter_call_code.done`
    #[serde(rename = "response.code_interpreter_call_code.done")]
    CodeInterpreterCodeDone {
        item_id: String,
        output_index: usize,
        code: String,
    },

    /// `response.output_text.annotation.added`
    #[serde(rename = "response.output_text.annotation.added")]
    AnnotationAdded { annotation: AnnotationItem },

    /// `response.reasoning_summary_part.added`
    #[serde(rename = "response.reasoning_summary_part.added")]
    ReasoningSummaryPartAdded {
        item_id: String,
        summary_index: usize,
    },

    /// `response.reasoning_summary_text.delta`
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta {
        item_id: String,
        summary_index: usize,
        delta: String,
    },

    /// `error`
    #[serde(rename = "error")]
    Error {
        code: String,
        message: String,
        #[serde(default)]
        param: Option<String>,
        #[serde(default)]
        sequence_number: Option<u64>,
    },

    /// Fallback for unknown chunk types.
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------------
// Sub-types for streaming chunks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogprobEntry {
    pub token: String,
    pub logprob: f64,
    pub top_logprobs: Vec<TopLogprob>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopLogprob {
    pub token: String,
    pub logprob: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseCreatedData {
    pub id: String,
    pub created_at: u64,
    pub model: String,
    #[serde(default)]
    pub service_tier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFinishedData {
    #[serde(default)]
    pub incomplete_details: Option<IncompleteDetails>,
    pub usage: ResponsesUsage,
    #[serde(default)]
    pub service_tier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncompleteDetails {
    pub reason: String,
}

/// Items that can appear in `response.output_item.added`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputItemAddedItem {
    #[serde(rename = "message")]
    Message { id: String },
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        #[serde(default)]
        encrypted_content: Option<String>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(rename = "web_search_call")]
    WebSearchCall {
        id: String,
        status: String,
        #[serde(default)]
        action: Option<serde_json::Value>,
    },
    #[serde(rename = "computer_call")]
    ComputerCall { id: String, status: String },
    #[serde(rename = "file_search_call")]
    FileSearchCall { id: String },
    #[serde(rename = "image_generation_call")]
    ImageGenerationCall { id: String },
    #[serde(rename = "code_interpreter_call")]
    CodeInterpreterCall {
        id: String,
        container_id: String,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        outputs: Option<Vec<CodeInterpreterOutput>>,
        #[serde(default)]
        status: Option<String>,
    },
}

/// Items that can appear in `response.output_item.done`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputItemDoneItem {
    #[serde(rename = "message")]
    Message { id: String },
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        #[serde(default)]
        encrypted_content: Option<String>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
        #[serde(default)]
        status: Option<String>,
    },
    #[serde(rename = "web_search_call")]
    WebSearchCall {
        id: String,
        status: String,
        #[serde(default)]
        action: Option<serde_json::Value>,
    },
    #[serde(rename = "file_search_call")]
    FileSearchCall {
        id: String,
        #[serde(default)]
        queries: Option<Vec<String>>,
        #[serde(default)]
        results: Option<Vec<FileSearchResult>>,
    },
    #[serde(rename = "code_interpreter_call")]
    CodeInterpreterCall {
        id: String,
        #[serde(default)]
        code: Option<String>,
        container_id: String,
        #[serde(default)]
        outputs: Option<Vec<CodeInterpreterOutput>>,
    },
    #[serde(rename = "image_generation_call")]
    ImageGenerationCall { id: String, result: String },
    #[serde(rename = "local_shell_call")]
    LocalShellCall {
        id: String,
        call_id: String,
        action: LocalShellAction,
    },
    #[serde(rename = "computer_call")]
    ComputerCall {
        id: String,
        #[serde(default)]
        status: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CodeInterpreterOutput {
    #[serde(rename = "logs")]
    Logs { logs: String },
    #[serde(rename = "image")]
    Image { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchResult {
    #[serde(default)]
    pub attributes: HashMap<String, serde_json::Value>,
    pub file_id: String,
    pub filename: String,
    pub score: f64,
    pub text: String,
}

/// Annotation types from `response.output_text.annotation.added`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnnotationItem {
    #[serde(rename = "url_citation")]
    UrlCitation { url: String, title: String },
    #[serde(rename = "file_citation")]
    FileCitation {
        file_id: String,
        #[serde(default)]
        filename: Option<String>,
        #[serde(default)]
        index: Option<u64>,
        #[serde(default)]
        start_index: Option<u64>,
        #[serde(default)]
        end_index: Option<u64>,
        #[serde(default)]
        quote: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Streaming State Management
// ---------------------------------------------------------------------------

/// Tracks state for an ongoing tool call during streaming.
#[derive(Debug, Clone)]
pub struct OngoingToolCall {
    pub tool_name: String,
    pub tool_call_id: String,
    pub code_interpreter: Option<CodeInterpreterState>,
}

#[derive(Debug, Clone)]
pub struct CodeInterpreterState {
    pub container_id: String,
}

/// Tracks active reasoning by output_index.
/// GitHub Copilot rotates encrypted item IDs on every event,
/// so we track by output_index instead of item_id.
#[derive(Debug, Clone)]
pub struct ActiveReasoning {
    /// The item.id from output_item.added
    pub canonical_id: String,
    pub encrypted_content: Option<String>,
    pub summary_parts: Vec<usize>,
}

// ---------------------------------------------------------------------------
// Metadata Extractor Trait
// ---------------------------------------------------------------------------

/// Extracts provider-specific metadata from API responses.
/// Mirrors the TS `MetadataExtractor` type.
pub trait MetadataExtractor: Send + Sync {
    /// Extract metadata from a complete (non-streaming) response body.
    fn extract_metadata(
        &self,
        parsed_body: &serde_json::Value,
    ) -> Option<HashMap<String, serde_json::Value>>;

    /// Create a stream extractor for processing chunks.
    fn create_stream_extractor(&self) -> Box<dyn StreamMetadataExtractor>;
}

/// Processes individual chunks and builds final metadata from accumulated stream data.
pub trait StreamMetadataExtractor: Send + Sync {
    fn process_chunk(&mut self, parsed_chunk: &serde_json::Value);
    fn build_metadata(&self) -> Option<HashMap<String, serde_json::Value>>;
}

// ---------------------------------------------------------------------------
// Response Metadata
// ---------------------------------------------------------------------------

/// Metadata extracted from a response, including token details.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Vec<Vec<LogprobEntry>>>,
}

/// Completion token details (reasoning, prediction accepted/rejected).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionTokenDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub accepted_prediction_tokens: Option<u64>,
    #[serde(default)]
    pub rejected_prediction_tokens: Option<u64>,
}

/// Prompt token details (cached tokens).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptTokenDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

/// Get response metadata from a chat completion response.
/// Mirrors TS `getResponseMetadata()`.
pub fn get_response_metadata(
    id: Option<&str>,
    model: Option<&str>,
    created: Option<u64>,
) -> ResponseMetadata {
    ResponseMetadata {
        response_id: id.map(|s| s.to_string()),
        model_id: model.map(|s| s.to_string()),
        timestamp: created,
        service_tier: None,
        logprobs: None,
    }
}

// ---------------------------------------------------------------------------
// Call Warning Types
// ---------------------------------------------------------------------------

/// Warnings generated during argument preparation.
/// Mirrors TS `LanguageModelV2CallWarning`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CallWarning {
    #[serde(rename = "unsupported-setting")]
    UnsupportedSetting {
        setting: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<String>,
    },
    #[serde(rename = "unsupported-tool")]
    UnsupportedTool {
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },
    #[serde(rename = "other")]
    Other { message: String },
}

// ---------------------------------------------------------------------------
// Validation & Warnings
// ---------------------------------------------------------------------------

/// Generate warnings for unsupported settings.
/// Mirrors the TS `getArgs()` warning logic.
pub fn validate_responses_settings(
    model_config: &ResponsesModelConfig,
    options: &ResponsesProviderOptions,
    top_k: Option<f32>,
    seed: Option<u64>,
    presence_penalty: Option<f32>,
    frequency_penalty: Option<f32>,
    stop_sequences: Option<&[String]>,
    temperature: Option<f32>,
    top_p: Option<f32>,
) -> Vec<CallWarning> {
    let mut warnings = Vec::new();

    if top_k.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "topK".to_string(),
            details: None,
        });
    }
    if seed.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "seed".to_string(),
            details: None,
        });
    }
    if presence_penalty.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "presencePenalty".to_string(),
            details: None,
        });
    }
    if frequency_penalty.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "frequencyPenalty".to_string(),
            details: None,
        });
    }
    if stop_sequences.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "stopSequences".to_string(),
            details: None,
        });
    }

    // Reasoning model validations
    if model_config.is_reasoning_model {
        if temperature.is_some() {
            warnings.push(CallWarning::UnsupportedSetting {
                setting: "temperature".to_string(),
                details: Some("temperature is not supported for reasoning models".to_string()),
            });
        }
        if top_p.is_some() {
            warnings.push(CallWarning::UnsupportedSetting {
                setting: "topP".to_string(),
                details: Some("topP is not supported for reasoning models".to_string()),
            });
        }
    } else {
        if options.reasoning_effort.is_some() {
            warnings.push(CallWarning::UnsupportedSetting {
                setting: "reasoningEffort".to_string(),
                details: Some(
                    "reasoningEffort is not supported for non-reasoning models".to_string(),
                ),
            });
        }
        if options.reasoning_summary.is_some() {
            warnings.push(CallWarning::UnsupportedSetting {
                setting: "reasoningSummary".to_string(),
                details: Some(
                    "reasoningSummary is not supported for non-reasoning models".to_string(),
                ),
            });
        }
    }

    // Flex processing validation
    if options.service_tier == Some(ServiceTier::Flex) && !model_config.supports_flex_processing {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "serviceTier".to_string(),
            details: Some(
                "flex processing is only available for o3, o4-mini, and gpt-5 models".to_string(),
            ),
        });
    }

    // Priority processing validation
    if options.service_tier == Some(ServiceTier::Priority)
        && !model_config.supports_priority_processing
    {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "serviceTier".to_string(),
            details: Some(
                "priority processing is only available for supported models (gpt-4, gpt-5, gpt-5-mini, o3, o4-mini) and requires Enterprise access. gpt-5-nano is not supported"
                    .to_string(),
            ),
        });
    }

    warnings
}

// ---------------------------------------------------------------------------
// OpenAI Responses Runtime
// ---------------------------------------------------------------------------

pub type UrlBuilder = Arc<dyn Fn(&str, &str) -> String + Send + Sync>;
pub type HeadersBuilder = Arc<dyn Fn() -> HashMap<String, String> + Send + Sync>;
pub type IdGenerator = Arc<dyn Fn() -> String + Send + Sync>;

#[derive(Clone)]
pub struct OpenAIResponsesConfig {
    pub provider: String,
    pub url: UrlBuilder,
    pub headers: HeadersBuilder,
    pub client: Option<Client>,
    pub file_id_prefixes: Option<Vec<String>>,
    pub generate_id: Option<IdGenerator>,
    pub metadata_extractor: Option<Arc<dyn MetadataExtractor>>,
}

impl std::fmt::Debug for OpenAIResponsesConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIResponsesConfig")
            .field("provider", &self.provider)
            .field("file_id_prefixes", &self.file_id_prefixes)
            .finish()
    }
}

impl Default for OpenAIResponsesConfig {
    fn default() -> Self {
        Self {
            provider: "openai".to_string(),
            url: Arc::new(|path, _model| format!("https://api.openai.com/v1{}", path)),
            headers: Arc::new(HashMap::new),
            client: None,
            file_id_prefixes: None,
            generate_id: None,
            metadata_extractor: None,
        }
    }
}

#[derive(Clone)]
pub struct OpenAIResponsesLanguageModel {
    pub model_id: String,
    pub config: OpenAIResponsesConfig,
}

impl std::fmt::Debug for OpenAIResponsesLanguageModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIResponsesLanguageModel")
            .field("model_id", &self.model_id)
            .field("config", &self.config)
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    pub prompt: Vec<Message>,
    pub tools: Option<Vec<InputTool>>,
    pub tool_choice: Option<InputToolChoice>,
    pub max_output_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<f32>,
    pub seed: Option<u64>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub stop_sequences: Option<Vec<String>>,
    pub provider_options: Option<ResponsesProviderOptions>,
    pub response_format: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    pub generate: GenerateOptions,
}

#[derive(Debug, Clone)]
pub struct PreparedArgs {
    pub web_search_tool_name: Option<String>,
    pub body: Value,
    pub warnings: Vec<CallWarning>,
}

#[derive(Debug, Clone)]
pub struct ResponsesGenerateResult {
    pub message: Message,
    pub finish_reason: FinishReason,
    pub usage: ResponsesUsage,
    pub metadata: ResponseMetadata,
    pub warnings: Vec<CallWarning>,
}

impl OpenAIResponsesLanguageModel {
    pub fn new(model_id: impl Into<String>, config: OpenAIResponsesConfig) -> Self {
        Self {
            model_id: model_id.into(),
            config,
        }
    }

    fn build_headers(&self, accept: &str) -> HashMap<String, String> {
        let mut headers = HashMap::from([
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), accept.to_string()),
        ]);
        headers.extend((self.config.headers)());
        headers
    }

    pub async fn get_args(&self, options: &GenerateOptions) -> Result<PreparedArgs, ProviderError> {
        let model_config = get_responses_model_config(&self.model_id);
        let provider_options = options.provider_options.clone().unwrap_or_default();
        let mut warnings = validate_responses_settings(
            &model_config,
            &provider_options,
            options.top_k,
            options.seed,
            options.presence_penalty,
            options.frequency_penalty,
            options.stop_sequences.as_deref(),
            options.temperature,
            options.top_p,
        );

        let strict_json_schema = provider_options.strict_json_schema.unwrap_or(false);
        let prepared_tools = prepare_responses_tools(
            options.tools.as_deref(),
            options.tool_choice.as_ref(),
            strict_json_schema,
        );

        let has_local_shell_tool = prepared_tools
            .tools
            .as_ref()
            .map(|tools| {
                tools
                    .iter()
                    .any(|tool| matches!(tool, ResponsesTool::LocalShell {}))
            })
            .unwrap_or(false);

        let store = provider_options.store.unwrap_or(true);
        let (input, convert_warnings) = convert_to_openai_responses_input(
            &options.prompt,
            model_config.system_message_mode,
            self.config.file_id_prefixes.as_deref(),
            store,
            has_local_shell_tool,
        )
        .await;

        warnings.extend(convert_warnings);
        warnings.extend(prepared_tools.tool_warnings);

        let mut include = provider_options.include.clone().unwrap_or_default();
        if provider_options
            .logprobs
            .as_ref()
            .and_then(LogprobsSetting::top_logprobs)
            .is_some()
        {
            push_include(
                &mut include,
                ResponsesIncludeValue::MessageOutputTextLogprobs,
            );
        }

        if let Some(tools) = &prepared_tools.tools {
            let has_web_search = tools.iter().any(|tool| {
                matches!(
                    tool,
                    ResponsesTool::WebSearch { .. } | ResponsesTool::WebSearchPreview { .. }
                )
            });
            if has_web_search {
                push_include(
                    &mut include,
                    ResponsesIncludeValue::WebSearchCallActionSources,
                );
            }
            let has_code_interpreter = tools
                .iter()
                .any(|tool| matches!(tool, ResponsesTool::CodeInterpreter { .. }));
            if has_code_interpreter {
                push_include(
                    &mut include,
                    ResponsesIncludeValue::CodeInterpreterCallOutputs,
                );
            }
        }

        let mut body = json!({
            "model": self.model_id,
            "input": input,
        });
        let obj = body.as_object_mut().ok_or_else(|| {
            ProviderError::InvalidRequest("failed to build responses request body".to_string())
        })?;

        if let Some(tools) = prepared_tools.tools {
            obj.insert(
                "tools".to_string(),
                serde_json::to_value(tools).map_err(|e| {
                    ProviderError::InvalidRequest(format!("failed to serialize tools: {}", e))
                })?,
            );
        }
        if let Some(tool_choice) = prepared_tools.tool_choice {
            obj.insert(
                "tool_choice".to_string(),
                serde_json::to_value(tool_choice).map_err(|e| {
                    ProviderError::InvalidRequest(format!("failed to serialize tool choice: {}", e))
                })?,
            );
        }
        if let Some(max_output_tokens) = options.max_output_tokens {
            obj.insert(
                "max_output_tokens".to_string(),
                Value::Number(max_output_tokens.into()),
            );
        }
        if !model_config.is_reasoning_model {
            if let Some(temperature) = options.temperature {
                obj.insert("temperature".to_string(), json!(temperature));
            }
            if let Some(top_p) = options.top_p {
                obj.insert("top_p".to_string(), json!(top_p));
            }
        }
        if model_config.required_auto_truncation {
            obj.insert("truncation".to_string(), Value::String("auto".to_string()));
        }

        if !include.is_empty() {
            obj.insert(
                "include".to_string(),
                serde_json::to_value(include).map_err(|e| {
                    ProviderError::InvalidRequest(format!("failed to serialize include: {}", e))
                })?,
            );
        }

        let mut text_obj = serde_json::Map::new();
        if let Some(top_n) = provider_options
            .logprobs
            .as_ref()
            .and_then(LogprobsSetting::top_logprobs)
        {
            text_obj.insert("logprobs".to_string(), Value::Bool(true));
            text_obj.insert("top_logprobs".to_string(), json!(top_n));
        }
        if let Some(verbosity) = provider_options.text_verbosity.clone() {
            text_obj.insert(
                "verbosity".to_string(),
                serde_json::to_value(verbosity).map_err(|e| {
                    ProviderError::InvalidRequest(format!(
                        "failed to serialize text verbosity: {}",
                        e
                    ))
                })?,
            );
        }
        if let Some(format) = &options.response_format {
            text_obj.insert("format".to_string(), format.clone());
        }
        if !text_obj.is_empty() {
            obj.insert("text".to_string(), Value::Object(text_obj));
        }

        if model_config.is_reasoning_model {
            let mut reasoning = serde_json::Map::new();
            if let Some(effort) = provider_options.reasoning_effort.clone() {
                reasoning.insert("effort".to_string(), Value::String(effort));
            }
            if let Some(summary) = provider_options.reasoning_summary.clone() {
                reasoning.insert("summary".to_string(), Value::String(summary));
            }
            if !reasoning.is_empty() {
                obj.insert("reasoning".to_string(), Value::Object(reasoning));
            }
        }

        insert_opt_string(obj, "instructions", provider_options.instructions);
        insert_opt_u64(
            obj,
            "max_tool_calls",
            provider_options.max_tool_calls.map(u64::from),
        );
        insert_opt_value(obj, "metadata", provider_options.metadata);
        insert_opt_bool(
            obj,
            "parallel_tool_calls",
            provider_options.parallel_tool_calls,
        );
        insert_opt_string(
            obj,
            "previous_response_id",
            provider_options.previous_response_id,
        );
        insert_opt_string(obj, "prompt_cache_key", provider_options.prompt_cache_key);
        insert_opt_string(obj, "safety_identifier", provider_options.safety_identifier);
        if let Some(service_tier) = provider_options.service_tier {
            obj.insert(
                "service_tier".to_string(),
                serde_json::to_value(service_tier).map_err(|e| {
                    ProviderError::InvalidRequest(format!(
                        "failed to serialize service tier: {}",
                        e
                    ))
                })?,
            );
        }
        insert_opt_bool(obj, "store", provider_options.store);
        insert_opt_string(obj, "user", provider_options.user);

        let web_search_tool_name = options.tools.as_deref().and_then(|tools| {
            tools.iter().find_map(|tool| match tool {
                InputTool::ProviderDefined { id, .. }
                    if id == "openai.web_search" || id == "openai.web_search_preview" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
        });

        Ok(PreparedArgs {
            web_search_tool_name,
            body,
            warnings,
        })
    }

    pub async fn do_generate(
        &self,
        options: GenerateOptions,
    ) -> Result<ResponsesGenerateResult, ProviderError> {
        let prepared = self.get_args(&options).await?;
        let url = (self.config.url)("/responses", &self.model_id);
        let headers = self.build_headers("application/json");
        let request_body = serde_json::to_string(&prepared.body)
            .map_err(|e| ProviderError::InvalidRequest(format!("failed to encode body: {}", e)))?;

        let (status_code, raw) = if let Some(proxy) = get_custom_fetch_proxy(&self.config.provider)
        {
            let response = proxy
                .fetch(CustomFetchRequest {
                    url,
                    method: "POST".to_string(),
                    headers: headers.clone(),
                    body: Some(request_body),
                })
                .await?;
            (response.status, response.body)
        } else {
            let client = self.config.client.clone().unwrap_or_default();
            let mut request = client.post(url);
            for (k, v) in &headers {
                request = request.header(k, v);
            }
            let response = request
                .body(request_body)
                .send()
                .await
                .map_err(|e| ProviderError::NetworkError(e.to_string()))?;
            let status = response.status().as_u16();
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
            (status, body)
        };
        if status_code >= 400 {
            return Err(ProviderError::ApiErrorWithStatus {
                message: raw,
                status_code,
            });
        }

        let body: Value = serde_json::from_str(&raw)
            .map_err(|e| ProviderError::ApiError(format!("invalid responses payload: {}", e)))?;

        let output = body
            .get("output")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let (parts, has_function_call, logprobs) = parse_output_items(&output);

        let usage = body
            .get("usage")
            .cloned()
            .and_then(|v| serde_json::from_value::<ResponsesUsage>(v).ok())
            .unwrap_or_default();

        let incomplete_reason = body
            .get("incomplete_details")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("reason"))
            .and_then(Value::as_str);
        let finish_reason = map_openai_response_finish_reason(incomplete_reason, has_function_call);

        let metadata = ResponseMetadata {
            response_id: body
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            model_id: body
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            timestamp: body.get("created_at").and_then(Value::as_u64),
            service_tier: body
                .get("service_tier")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            logprobs: if logprobs.is_empty() {
                None
            } else {
                Some(logprobs)
            },
        };

        let message = Message {
            role: Role::Assistant,
            content: if parts.is_empty() {
                Content::Text(String::new())
            } else {
                Content::Parts(parts)
            },
            cache_control: None,
            provider_options: None,
        };

        Ok(ResponsesGenerateResult {
            message,
            finish_reason,
            usage,
            metadata,
            warnings: prepared.warnings,
        })
    }

    pub async fn do_stream(&self, options: StreamOptions) -> Result<StreamResult, ProviderError> {
        let mut prepared = self.get_args(&options.generate).await?;
        let body_obj = prepared.body.as_object_mut().ok_or_else(|| {
            ProviderError::InvalidRequest("invalid responses request".to_string())
        })?;
        body_obj.insert("stream".to_string(), Value::Bool(true));

        let url = (self.config.url)("/responses", &self.model_id);
        let headers = self.build_headers("text/event-stream");
        let request_body = serde_json::to_string(&prepared.body)
            .map_err(|e| ProviderError::InvalidRequest(format!("failed to encode body: {}", e)))?;
        let text_stream: Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send>> =
            if let Some(proxy) = get_custom_fetch_proxy(&self.config.provider) {
                let response = proxy
                    .fetch_stream(CustomFetchRequest {
                        url,
                        method: "POST".to_string(),
                        headers: headers.clone(),
                        body: Some(request_body),
                    })
                    .await?;
                if response.status >= 400 {
                    return Err(ProviderError::ApiErrorWithStatus {
                        message: format!(
                            "custom fetch stream request failed with status {}",
                            response.status
                        ),
                        status_code: response.status,
                    });
                }
                response.stream
            } else {
                let client = self.config.client.clone().unwrap_or_default();
                let mut request = client.post(url);
                for (k, v) in &headers {
                    request = request.header(k, v);
                }
                let response = request
                    .body(request_body)
                    .send()
                    .await
                    .map_err(|e| ProviderError::NetworkError(e.to_string()))?;
                let status = response.status();
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    return Err(ProviderError::ApiErrorWithStatus {
                        message: body,
                        status_code: status.as_u16(),
                    });
                }
                Box::pin(
                    response
                        .bytes_stream()
                        .map(|chunk_result| match chunk_result {
                            Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).to_string()),
                            Err(err) => Err(ProviderError::StreamError(err.to_string())),
                        }),
                )
            };
        let metadata_extractor = self
            .config
            .metadata_extractor
            .as_ref()
            .map(|extractor| extractor.create_stream_extractor());

        let (tx, rx) = mpsc::channel::<Result<StreamEvent, ProviderError>>(256);
        tokio::spawn(async move {
            let _ = tx.send(Ok(StreamEvent::Start)).await;
            let _ = tx.send(Ok(StreamEvent::StartStep)).await;

            let tx = tx;
            let mut text_stream = text_stream;
            let mut stream_metadata_extractor = metadata_extractor;

            let mut buffer = String::new();
            let mut finish_reason = FinishReason::Unknown;
            let mut usage = ResponsesUsage::default();
            let mut logprobs: Vec<Vec<LogprobEntry>> = Vec::new();
            let mut response_id: Option<String> = None;
            let mut ongoing_tool_calls: HashMap<usize, OngoingToolCall> = HashMap::new();
            let mut has_function_call = false;
            let mut active_reasoning: HashMap<usize, ActiveReasoning> = HashMap::new();
            let mut current_reasoning_output_index: Option<usize> = None;
            let mut reasoning_item_to_output_index: HashMap<String, usize> = HashMap::new();
            let mut current_text_id: Option<String> = None;
            let mut text_open = false;
            let mut service_tier: Option<String> = None;

            while let Some(chunk_result) = text_stream.next().await {
                let chunk = match chunk_result {
                    Ok(text) => text,
                    Err(err) => {
                        let _ = tx.send(Err(err)).await;
                        return;
                    }
                };
                buffer.push_str(&chunk);

                while let Some(frame) = drain_next_sse_frame(&mut buffer) {
                    let Some(data) = extract_sse_data(&frame) else {
                        continue;
                    };
                    if data == "[DONE]" {
                        break;
                    }

                    let parsed_value: Value = match serde_json::from_str(&data) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    if let Some(extractor) = stream_metadata_extractor.as_mut() {
                        extractor.process_chunk(&parsed_value);
                    }

                    let parsed_chunk: ResponsesStreamChunk = serde_json::from_value(parsed_value)
                        .unwrap_or(ResponsesStreamChunk::Unknown);

                    for event in process_stream_chunk(
                        parsed_chunk,
                        &mut finish_reason,
                        &mut usage,
                        &mut logprobs,
                        &mut response_id,
                        &mut ongoing_tool_calls,
                        &mut has_function_call,
                        &mut active_reasoning,
                        &mut current_reasoning_output_index,
                        &mut reasoning_item_to_output_index,
                        &mut current_text_id,
                        &mut text_open,
                        &mut service_tier,
                    ) {
                        if tx.send(Ok(event)).await.is_err() {
                            return;
                        }
                    }
                }
            }

            if text_open {
                if tx.send(Ok(StreamEvent::TextEnd)).await.is_err() {
                    return;
                }
            }
            for ongoing in ongoing_tool_calls.into_values() {
                if tx
                    .send(Ok(StreamEvent::ToolInputEnd {
                        id: ongoing.tool_call_id,
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            for reasoning in active_reasoning.into_values() {
                if tx
                    .send(Ok(StreamEvent::ReasoningEnd {
                        id: reasoning.canonical_id,
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            let mut provider_metadata = json!({
                "response_id": response_id,
                "service_tier": service_tier,
            });
            if !logprobs.is_empty() {
                provider_metadata["logprobs"] =
                    serde_json::to_value(logprobs).unwrap_or(Value::Null);
            }
            if let Some(extractor) = stream_metadata_extractor.as_ref() {
                if let Some(extra) = extractor.build_metadata() {
                    provider_metadata["metadata"] =
                        serde_json::to_value(extra).unwrap_or(Value::Null);
                }
            }

            let resolved_reason = if finish_reason == FinishReason::Unknown {
                map_openai_response_finish_reason(None, has_function_call)
            } else {
                finish_reason
            };

            if tx
                .send(Ok(StreamEvent::FinishStep {
                    finish_reason: Some(finish_reason_label(resolved_reason).to_string()),
                    usage: usage_to_stream_usage(&usage),
                    provider_metadata: Some(provider_metadata),
                }))
                .await
                .is_err()
            {
                return;
            }
            if tx.send(Ok(StreamEvent::Finish)).await.is_err() {
                return;
            }
            let _ = tx.send(Ok(StreamEvent::Done)).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

#[allow(clippy::too_many_arguments)]
fn process_stream_chunk(
    chunk: ResponsesStreamChunk,
    finish_reason: &mut FinishReason,
    usage: &mut ResponsesUsage,
    logprobs: &mut Vec<Vec<LogprobEntry>>,
    response_id: &mut Option<String>,
    ongoing_tool_calls: &mut HashMap<usize, OngoingToolCall>,
    has_function_call: &mut bool,
    active_reasoning: &mut HashMap<usize, ActiveReasoning>,
    current_reasoning_output_index: &mut Option<usize>,
    reasoning_item_to_output_index: &mut HashMap<String, usize>,
    current_text_id: &mut Option<String>,
    text_open: &mut bool,
    service_tier: &mut Option<String>,
) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    match chunk {
        ResponsesStreamChunk::OutputItemAdded { output_index, item } => match item {
            OutputItemAddedItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                ongoing_tool_calls.insert(
                    output_index,
                    OngoingToolCall {
                        tool_name: name.clone(),
                        tool_call_id: call_id.clone(),
                        code_interpreter: None,
                    },
                );
                events.push(StreamEvent::ToolInputStart {
                    id: call_id.clone(),
                    tool_name: name,
                });
                if !arguments.is_empty() {
                    events.push(StreamEvent::ToolInputDelta {
                        id: call_id,
                        delta: arguments,
                    });
                }
            }
            OutputItemAddedItem::WebSearchCall { id, .. } => {
                ongoing_tool_calls.insert(
                    output_index,
                    OngoingToolCall {
                        tool_name: "web_search_call".to_string(),
                        tool_call_id: id.clone(),
                        code_interpreter: None,
                    },
                );
                events.push(StreamEvent::ToolInputStart {
                    id: id,
                    tool_name: "web_search_call".to_string(),
                });
            }
            OutputItemAddedItem::CodeInterpreterCall {
                id,
                container_id,
                code,
                ..
            } => {
                ongoing_tool_calls.insert(
                    output_index,
                    OngoingToolCall {
                        tool_name: "code_interpreter_call".to_string(),
                        tool_call_id: id.clone(),
                        code_interpreter: Some(CodeInterpreterState { container_id }),
                    },
                );
                events.push(StreamEvent::ToolInputStart {
                    id: id.clone(),
                    tool_name: "code_interpreter_call".to_string(),
                });
                if let Some(code) = code {
                    if !code.is_empty() {
                        events.push(StreamEvent::ToolInputDelta { id, delta: code });
                    }
                }
            }
            OutputItemAddedItem::FileSearchCall { id } => {
                events.push(StreamEvent::ToolCallStart {
                    id: id.clone(),
                    name: "file_search_call".to_string(),
                });
                events.push(StreamEvent::ToolCallEnd {
                    id,
                    name: "file_search_call".to_string(),
                    input: json!({}),
                });
            }
            OutputItemAddedItem::ImageGenerationCall { id } => {
                events.push(StreamEvent::ToolCallStart {
                    id: id.clone(),
                    name: "image_generation_call".to_string(),
                });
                events.push(StreamEvent::ToolCallEnd {
                    id,
                    name: "image_generation_call".to_string(),
                    input: json!({}),
                });
            }
            OutputItemAddedItem::Message { id } => {
                *current_text_id = Some(id);
                if !*text_open {
                    *text_open = true;
                    events.push(StreamEvent::TextStart);
                }
            }
            OutputItemAddedItem::Reasoning {
                id,
                encrypted_content,
            } => {
                active_reasoning.insert(
                    output_index,
                    ActiveReasoning {
                        canonical_id: id.clone(),
                        encrypted_content,
                        summary_parts: vec![0],
                    },
                );
                reasoning_item_to_output_index.insert(id.clone(), output_index);
                *current_reasoning_output_index = Some(output_index);
                events.push(StreamEvent::ReasoningStart { id });
            }
            OutputItemAddedItem::ComputerCall { id, .. } => {
                events.push(StreamEvent::ToolCallStart {
                    id: id.clone(),
                    name: "computer_call".to_string(),
                });
                events.push(StreamEvent::ToolCallEnd {
                    id,
                    name: "computer_call".to_string(),
                    input: json!({}),
                });
            }
        },
        ResponsesStreamChunk::OutputItemDone { output_index, item } => match item {
            OutputItemDoneItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                ongoing_tool_calls.remove(&output_index);
                events.push(StreamEvent::ToolInputEnd {
                    id: call_id.clone(),
                });
                events.push(StreamEvent::ToolCallEnd {
                    id: call_id.clone(),
                    name: name.clone(),
                    input: parse_json_or_string(arguments),
                });
                *has_function_call = true;
            }
            OutputItemDoneItem::WebSearchCall { id, action, .. } => {
                ongoing_tool_calls.remove(&output_index);
                let input = action.unwrap_or_else(|| json!({}));
                events.push(StreamEvent::ToolInputEnd { id: id.clone() });
                events.push(StreamEvent::ToolCallEnd {
                    id: id.clone(),
                    name: "web_search_call".to_string(),
                    input: input.clone(),
                });
                events.push(StreamEvent::ToolResult {
                    tool_call_id: id,
                    tool_name: "web_search_call".to_string(),
                    input: Some(input.clone()),
                    output: ToolResultOutput {
                        output: serde_json::to_string(&input).unwrap_or_default(),
                        title: "Web Search".to_string(),
                        metadata: HashMap::from([(
                            "providerExecuted".to_string(),
                            Value::Bool(true),
                        )]),
                        attachments: None,
                    },
                });
                *has_function_call = true;
            }
            OutputItemDoneItem::CodeInterpreterCall {
                id,
                code,
                container_id,
                outputs,
            } => {
                ongoing_tool_calls.remove(&output_index);
                if let Some(code) = code {
                    events.push(StreamEvent::ToolCallEnd {
                        id: id.clone(),
                        name: "code_interpreter_call".to_string(),
                        input: json!({
                            "code": code,
                            "container_id": container_id,
                        }),
                    });
                    *has_function_call = true;
                }
                let output_json = json!({ "outputs": outputs });
                events.push(StreamEvent::ToolResult {
                    tool_call_id: id,
                    tool_name: "code_interpreter_call".to_string(),
                    input: None,
                    output: ToolResultOutput {
                        output: serde_json::to_string(&output_json).unwrap_or_default(),
                        title: "Code Interpreter".to_string(),
                        metadata: HashMap::from([(
                            "providerExecuted".to_string(),
                            Value::Bool(true),
                        )]),
                        attachments: None,
                    },
                });
            }
            OutputItemDoneItem::FileSearchCall {
                id,
                queries,
                results,
            } => {
                let output_json = json!({
                    "queries": queries.unwrap_or_default(),
                    "results": results,
                });
                events.push(StreamEvent::ToolResult {
                    tool_call_id: id,
                    tool_name: "file_search_call".to_string(),
                    input: None,
                    output: ToolResultOutput {
                        output: serde_json::to_string(&output_json).unwrap_or_default(),
                        title: "File Search".to_string(),
                        metadata: HashMap::from([(
                            "providerExecuted".to_string(),
                            Value::Bool(true),
                        )]),
                        attachments: None,
                    },
                });
            }
            OutputItemDoneItem::ImageGenerationCall { id, result } => {
                events.push(StreamEvent::ToolResult {
                    tool_call_id: id,
                    tool_name: "image_generation_call".to_string(),
                    input: None,
                    output: ToolResultOutput {
                        output: result,
                        title: "Image Generation".to_string(),
                        metadata: HashMap::from([(
                            "providerExecuted".to_string(),
                            Value::Bool(true),
                        )]),
                        attachments: None,
                    },
                });
            }
            OutputItemDoneItem::LocalShellCall {
                call_id, action, ..
            } => {
                ongoing_tool_calls.remove(&output_index);
                events.push(StreamEvent::ToolInputEnd {
                    id: call_id.clone(),
                });
                events.push(StreamEvent::ToolCallEnd {
                    id: call_id.clone(),
                    name: "local_shell".to_string(),
                    input: json!({ "action": action }),
                });
                *has_function_call = true;
            }
            OutputItemDoneItem::Message { id } => {
                if current_text_id.as_deref() == Some(id.as_str()) && *text_open {
                    *text_open = false;
                    *current_text_id = None;
                    events.push(StreamEvent::TextEnd);
                }
            }
            OutputItemDoneItem::Reasoning { id, .. } => {
                if let Some(index) = reasoning_item_to_output_index.remove(&id) {
                    if let Some(reasoning) = active_reasoning.remove(&index) {
                        events.push(StreamEvent::ReasoningEnd {
                            id: reasoning.canonical_id,
                        });
                    }
                }
            }
            OutputItemDoneItem::ComputerCall { id, status } => {
                let status = status.unwrap_or_else(|| "unknown".to_string());
                events.push(StreamEvent::ToolResult {
                    tool_call_id: id,
                    tool_name: "computer_call".to_string(),
                    input: None,
                    output: ToolResultOutput {
                        output: status,
                        title: "Computer Call".to_string(),
                        metadata: HashMap::from([(
                            "providerExecuted".to_string(),
                            Value::Bool(true),
                        )]),
                        attachments: None,
                    },
                });
            }
        },
        ResponsesStreamChunk::FunctionCallArgumentsDelta {
            output_index,
            delta,
            ..
        } => {
            if let Some(call) = ongoing_tool_calls.get(&output_index) {
                events.push(StreamEvent::ToolInputDelta {
                    id: call.tool_call_id.clone(),
                    delta,
                });
            }
        }
        ResponsesStreamChunk::CodeInterpreterCodeDelta {
            output_index,
            delta,
            ..
        } => {
            if let Some(call) = ongoing_tool_calls.get(&output_index) {
                events.push(StreamEvent::ToolInputDelta {
                    id: call.tool_call_id.clone(),
                    delta,
                });
            }
        }
        ResponsesStreamChunk::CodeInterpreterCodeDone {
            output_index, code, ..
        } => {
            if let Some(call) = ongoing_tool_calls.get(&output_index) {
                events.push(StreamEvent::ToolInputDelta {
                    id: call.tool_call_id.clone(),
                    delta: code.clone(),
                });
                events.push(StreamEvent::ToolInputEnd {
                    id: call.tool_call_id.clone(),
                });
                events.push(StreamEvent::ToolCallEnd {
                    id: call.tool_call_id.clone(),
                    name: call.tool_name.clone(),
                    input: json!({
                        "code": code,
                        "container_id": call.code_interpreter.as_ref().map(|c| c.container_id.clone()),
                    }),
                });
            }
            *has_function_call = true;
        }
        ResponsesStreamChunk::ImageGenerationPartialImage {
            item_id,
            partial_image_b64,
            ..
        } => {
            events.push(StreamEvent::ToolResult {
                tool_call_id: item_id,
                tool_name: "image_generation_call".to_string(),
                input: None,
                output: ToolResultOutput {
                    output: partial_image_b64,
                    title: "Image Generation (partial)".to_string(),
                    metadata: HashMap::from([("partial".to_string(), Value::Bool(true))]),
                    attachments: None,
                },
            });
        }
        ResponsesStreamChunk::ResponseCreated { response } => {
            *response_id = Some(response.id);
            *service_tier = response.service_tier;
        }
        ResponsesStreamChunk::TextDelta {
            item_id,
            delta,
            logprobs: lp,
        } => {
            if !*text_open {
                *text_open = true;
                *current_text_id = Some(item_id);
                events.push(StreamEvent::TextStart);
            }
            if !delta.is_empty() {
                events.push(StreamEvent::TextDelta(delta));
            }
            if let Some(entries) = lp {
                logprobs.push(entries);
            }
        }
        ResponsesStreamChunk::ReasoningSummaryPartAdded {
            item_id,
            summary_index,
        } => {
            let maybe_index = reasoning_item_to_output_index
                .get(&item_id)
                .copied()
                .or(*current_reasoning_output_index);
            if let Some(index) = maybe_index {
                if let Some(reasoning) = active_reasoning.get_mut(&index) {
                    if !reasoning.summary_parts.contains(&summary_index) {
                        reasoning.summary_parts.push(summary_index);
                        if summary_index > 0 {
                            events.push(StreamEvent::ReasoningStart {
                                id: reasoning.canonical_id.clone(),
                            });
                        }
                    }
                }
            }
        }
        ResponsesStreamChunk::ReasoningSummaryTextDelta { item_id, delta, .. } => {
            if let Some(index) = reasoning_item_to_output_index.get(&item_id).copied() {
                if let Some(reasoning) = active_reasoning.get(&index) {
                    events.push(StreamEvent::ReasoningDelta {
                        id: reasoning.canonical_id.clone(),
                        text: delta,
                    });
                }
            }
        }
        ResponsesStreamChunk::ResponseCompleted { response } => {
            *usage = response.usage.clone();
            *service_tier = response.service_tier;
            *finish_reason = map_openai_response_finish_reason(
                response
                    .incomplete_details
                    .as_ref()
                    .map(|d| d.reason.as_str()),
                *has_function_call,
            );
        }
        ResponsesStreamChunk::ResponseIncomplete { response } => {
            *usage = response.usage.clone();
            *service_tier = response.service_tier;
            *finish_reason = map_openai_response_finish_reason(
                response
                    .incomplete_details
                    .as_ref()
                    .map(|d| d.reason.as_str()),
                *has_function_call,
            );
        }
        ResponsesStreamChunk::AnnotationAdded { .. } => {}
        ResponsesStreamChunk::Error { message, .. } => {
            events.push(StreamEvent::Error(message));
            *finish_reason = FinishReason::Error;
        }
        ResponsesStreamChunk::Unknown => {}
    }

    events
}

fn parse_output_items(output: &[Value]) -> (Vec<ContentPart>, bool, Vec<Vec<LogprobEntry>>) {
    let mut parts = Vec::new();
    let mut has_function_call = false;
    let mut logprobs = Vec::new();

    for item in output {
        let Some(item_type) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        match item_type {
            "reasoning" => {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let encrypted = item
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                let summary = item
                    .get("summary")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();

                let mut provider_options = HashMap::new();
                if !id.is_empty() {
                    provider_options.insert("itemId".to_string(), Value::String(id));
                }
                if let Some(encrypted) = encrypted {
                    provider_options
                        .insert("encryptedContent".to_string(), Value::String(encrypted));
                }

                parts.push(ContentPart {
                    content_type: "reasoning".to_string(),
                    text: Some(summary),
                    provider_options: if provider_options.is_empty() {
                        None
                    } else {
                        Some(provider_options)
                    },
                    ..Default::default()
                });
            }
            "message" => {
                for content in item
                    .get("content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                {
                    let Some(content_type) = content.get("type").and_then(Value::as_str) else {
                        continue;
                    };
                    if content_type == "output_text" {
                        let text = content
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if !text.is_empty() {
                            parts.push(ContentPart {
                                content_type: "text".to_string(),
                                text: Some(text),
                                ..Default::default()
                            });
                        }
                        if let Some(lp) = content.get("logprobs").cloned() {
                            if let Ok(parsed) = serde_json::from_value::<Vec<LogprobEntry>>(lp) {
                                if !parsed.is_empty() {
                                    logprobs.push(parsed);
                                }
                            }
                        }
                    }
                }
            }
            "function_call" => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                parts.push(ContentPart {
                    content_type: "tool_use".to_string(),
                    tool_use: Some(ToolUse {
                        id: call_id,
                        name,
                        input: parse_json_or_string(arguments.to_string()),
                    }),
                    ..Default::default()
                });
                has_function_call = true;
            }
            "web_search_call" => {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let action = item.get("action").cloned().unwrap_or_else(|| json!({}));
                parts.push(provider_executed_tool_parts(
                    id,
                    "web_search_call",
                    action.clone(),
                    action,
                ));
                has_function_call = true;
            }
            "file_search_call" => {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let output = json!({
                    "queries": item.get("queries").cloned().unwrap_or_else(|| json!([])),
                    "results": item.get("results").cloned().unwrap_or(Value::Null),
                });
                parts.push(provider_executed_tool_parts(
                    id,
                    "file_search_call",
                    json!({}),
                    output,
                ));
                has_function_call = true;
            }
            "code_interpreter_call" => {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let input = json!({
                    "code": item.get("code").cloned().unwrap_or(Value::Null),
                    "container_id": item.get("container_id").cloned().unwrap_or(Value::Null),
                });
                let output = json!({
                    "outputs": item.get("outputs").cloned().unwrap_or(Value::Null),
                });
                parts.push(provider_executed_tool_parts(
                    id,
                    "code_interpreter_call",
                    input,
                    output,
                ));
                has_function_call = true;
            }
            "image_generation_call" => {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let output = json!({
                    "result": item.get("result").cloned().unwrap_or(Value::Null),
                });
                parts.push(provider_executed_tool_parts(
                    id,
                    "image_generation_call",
                    json!({}),
                    output,
                ));
                has_function_call = true;
            }
            "local_shell_call" => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let action = item.get("action").cloned().unwrap_or_else(|| json!({}));
                parts.push(ContentPart {
                    content_type: "tool_use".to_string(),
                    tool_use: Some(ToolUse {
                        id: call_id,
                        name: "local_shell".to_string(),
                        input: json!({ "action": action }),
                    }),
                    ..Default::default()
                });
                has_function_call = true;
            }
            "computer_call" => {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let output = json!({
                    "status": item.get("status").cloned().unwrap_or(Value::Null),
                });
                parts.push(provider_executed_tool_parts(
                    id,
                    "computer_call",
                    json!({}),
                    output,
                ));
                has_function_call = true;
            }
            _ => {}
        }
    }

    (parts, has_function_call, logprobs)
}

fn provider_executed_tool_parts(
    id: String,
    tool_name: &str,
    input: Value,
    output: Value,
) -> ContentPart {
    let mut provider_options = HashMap::new();
    provider_options.insert("providerExecuted".to_string(), Value::Bool(true));

    ContentPart {
        content_type: "tool_result".to_string(),
        tool_use: Some(ToolUse {
            id: id.clone(),
            name: tool_name.to_string(),
            input,
        }),
        tool_result: Some(ToolResult {
            tool_use_id: id,
            content: serde_json::to_string(&output).unwrap_or_default(),
            is_error: Some(false),
        }),
        provider_options: Some(provider_options),
        ..Default::default()
    }
}

fn usage_to_stream_usage(usage: &ResponsesUsage) -> StreamUsage {
    StreamUsage {
        prompt_tokens: usage.input_tokens,
        completion_tokens: usage.output_tokens,
        reasoning_tokens: usage
            .output_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens)
            .unwrap_or(0),
        cache_read_tokens: usage
            .input_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .unwrap_or(0),
        cache_write_tokens: 0,
    }
}

fn push_include(include: &mut Vec<ResponsesIncludeValue>, value: ResponsesIncludeValue) {
    if !include.contains(&value) {
        include.push(value);
    }
}

fn insert_opt_string(obj: &mut serde_json::Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        obj.insert(key.to_string(), Value::String(value));
    }
}

fn insert_opt_u64(obj: &mut serde_json::Map<String, Value>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        obj.insert(key.to_string(), Value::Number(value.into()));
    }
}

fn insert_opt_bool(obj: &mut serde_json::Map<String, Value>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        obj.insert(key.to_string(), Value::Bool(value));
    }
}

fn insert_opt_value(obj: &mut serde_json::Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        obj.insert(key.to_string(), value);
    }
}

fn parse_json_or_string(raw: String) -> Value {
    serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| Value::String(raw))
}

fn drain_next_sse_frame(buffer: &mut String) -> Option<String> {
    let lf = buffer.find("\n\n");
    let crlf = buffer.find("\r\n\r\n");
    let (idx, len) = match (lf, crlf) {
        (Some(a), Some(b)) if a <= b => (a, 2),
        (Some(_a), Some(b)) => (b, 4),
        (Some(a), None) => (a, 2),
        (None, Some(b)) => (b, 4),
        (None, None) => return None,
    };

    let frame = buffer[..idx].to_string();
    buffer.drain(..idx + len);
    Some(frame)
}

fn extract_sse_data(frame: &str) -> Option<String> {
    let mut data_lines = Vec::new();
    for raw_line in frame.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }

    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

fn finish_reason_label(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ContentFilter => "content-filter",
        FinishReason::ToolCalls => "tool-calls",
        FinishReason::Error => "error",
        FinishReason::Unknown => "unknown",
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::Arc;

    use crate::custom_fetch::{
        register_custom_fetch_proxy, unregister_custom_fetch_proxy, CustomFetchProxy,
        CustomFetchRequest, CustomFetchResponse, CustomFetchStreamResponse,
    };

    struct FakeCustomFetchProxy;

    #[async_trait]
    impl CustomFetchProxy for FakeCustomFetchProxy {
        async fn fetch(
            &self,
            _request: CustomFetchRequest,
        ) -> Result<CustomFetchResponse, ProviderError> {
            Ok(CustomFetchResponse {
                status: 200,
                headers: HashMap::new(),
                body: json!({
                    "id": "resp_1",
                    "model": "gpt-5",
                    "output": [
                        {
                            "type": "message",
                            "content": [
                                {"type": "output_text", "text": "hello from proxy"}
                            ]
                        }
                    ],
                    "usage": {
                        "input_tokens": 1,
                        "output_tokens": 1,
                        "total_tokens": 2
                    }
                })
                .to_string(),
            })
        }

        async fn fetch_stream(
            &self,
            _request: CustomFetchRequest,
        ) -> Result<CustomFetchStreamResponse, ProviderError> {
            let frames = vec![
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_stream_1\"}}\n\n"
                    .to_string(),
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_stream_1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n"
                    .to_string(),
                "data: [DONE]\n\n".to_string(),
            ];
            Ok(CustomFetchStreamResponse {
                status: 200,
                headers: HashMap::new(),
                stream: Box::pin(stream::iter(frames.into_iter().map(Ok))),
            })
        }
    }

    #[test]
    fn test_parse_output_items_function_call_and_text() {
        let output = vec![
            json!({
                "type": "message",
                "content": [
                    {"type": "output_text", "text": "hello"}
                ]
            }),
            json!({
                "type": "function_call",
                "call_id": "call_1",
                "name": "grep",
                "arguments": "{\"q\":\"hello\"}"
            }),
        ];

        let (parts, has_tool, _logprobs) = parse_output_items(&output);
        assert!(has_tool);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].content_type, "text");
        assert_eq!(parts[1].content_type, "tool_use");
        assert_eq!(
            parts[1].tool_use.as_ref().map(|t| t.name.as_str()),
            Some("grep")
        );
    }

    #[test]
    fn test_stream_state_machine_function_call_lifecycle() {
        let mut finish_reason = FinishReason::Unknown;
        let mut usage = ResponsesUsage::default();
        let mut logprobs = Vec::new();
        let mut response_id = None;
        let mut ongoing_tool_calls = HashMap::new();
        let mut has_function_call = false;
        let mut active_reasoning = HashMap::new();
        let mut current_reasoning_output_index = None;
        let mut reasoning_item_to_output_index = HashMap::new();
        let mut current_text_id = None;
        let mut text_open = false;
        let mut service_tier = None;

        let added_events = process_stream_chunk(
            ResponsesStreamChunk::OutputItemAdded {
                output_index: 0,
                item: OutputItemAddedItem::FunctionCall {
                    id: "item_1".to_string(),
                    call_id: "call_1".to_string(),
                    name: "grep".to_string(),
                    arguments: "{\"q\":\"x\"}".to_string(),
                },
            },
            &mut finish_reason,
            &mut usage,
            &mut logprobs,
            &mut response_id,
            &mut ongoing_tool_calls,
            &mut has_function_call,
            &mut active_reasoning,
            &mut current_reasoning_output_index,
            &mut reasoning_item_to_output_index,
            &mut current_text_id,
            &mut text_open,
            &mut service_tier,
        );
        assert!(added_events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolInputStart { .. })));

        let done_events = process_stream_chunk(
            ResponsesStreamChunk::OutputItemDone {
                output_index: 0,
                item: OutputItemDoneItem::FunctionCall {
                    id: "item_1".to_string(),
                    call_id: "call_1".to_string(),
                    name: "grep".to_string(),
                    arguments: "{\"q\":\"x\"}".to_string(),
                    status: Some("completed".to_string()),
                },
            },
            &mut finish_reason,
            &mut usage,
            &mut logprobs,
            &mut response_id,
            &mut ongoing_tool_calls,
            &mut has_function_call,
            &mut active_reasoning,
            &mut current_reasoning_output_index,
            &mut reasoning_item_to_output_index,
            &mut current_text_id,
            &mut text_open,
            &mut service_tier,
        );
        assert!(done_events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolInputEnd { .. })));
        assert!(done_events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolCallEnd { .. })));
        assert!(has_function_call);
    }

    #[tokio::test]
    async fn test_do_generate_uses_registered_custom_fetch_proxy() {
        register_custom_fetch_proxy("test-provider", Arc::new(FakeCustomFetchProxy));

        let model = OpenAIResponsesLanguageModel::new(
            "gpt-5",
            OpenAIResponsesConfig {
                provider: "test-provider".to_string(),
                ..Default::default()
            },
        );
        let result = model
            .do_generate(GenerateOptions {
                prompt: vec![Message::user("hello".to_string())],
                ..Default::default()
            })
            .await
            .expect("generate via custom fetch should succeed");

        match result.message.content {
            Content::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].text.as_deref(), Some("hello from proxy"));
            }
            other => panic!("unexpected content: {other:?}"),
        }

        unregister_custom_fetch_proxy("test-provider");
    }

    #[tokio::test]
    async fn test_do_stream_uses_registered_custom_fetch_proxy() {
        register_custom_fetch_proxy("test-provider-stream", Arc::new(FakeCustomFetchProxy));

        let model = OpenAIResponsesLanguageModel::new(
            "gpt-5",
            OpenAIResponsesConfig {
                provider: "test-provider-stream".to_string(),
                ..Default::default()
            },
        );
        let stream = model
            .do_stream(StreamOptions {
                generate: GenerateOptions {
                    prompt: vec![Message::user("hello".to_string())],
                    ..Default::default()
                },
            })
            .await
            .expect("stream via custom fetch should succeed");

        let events: Vec<_> = stream.collect::<Vec<_>>().await;
        assert!(events
            .iter()
            .any(|event| matches!(event, Ok(StreamEvent::Start))));
        assert!(events
            .iter()
            .any(|event| matches!(event, Ok(StreamEvent::Finish))));
        assert!(events
            .iter()
            .any(|event| matches!(event, Ok(StreamEvent::Done))));

        unregister_custom_fetch_proxy("test-provider-stream");
    }
}
