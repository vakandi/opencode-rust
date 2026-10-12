use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use opencode_provider::{
    parse_api_call_error, parse_stream_error, Content, ContentPart, ImageUrl,
    Message as ProviderMessage, ParsedAPICallError, ParsedStreamError, ProviderError, Role,
    ToolResult as ProviderToolResult, ToolUse,
};


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FilePartSource {
    #[serde(rename = "file")]
    File { path: String, text: FileSourceText },
    #[serde(rename = "symbol")]
    Symbol {
        path: String,
        name: String,
        kind: i32,
        range: LspRange,
        text: FileSourceText,
    },
    #[serde(rename = "resource")]
    Resource {
        client_name: String,
        uri: String,
        text: FileSourceText,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSourceText {
    pub value: String,
    pub start: i32,
    pub end: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: i32,
    pub character: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub mime: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<FilePartSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<AgentSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSource {
    pub value: String,
    pub start: i32,
    pub end: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub auto: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub prompt: String,
    pub description: String,
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub attempt: i32,
    pub error: ApiError,
    pub time: RetryTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryTime {
    pub created: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStartPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepFinishPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    pub cost: f64,
    pub tokens: StepTokens,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepTokens {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i32>,
    pub input: i32,
    pub output: i32,
    pub reasoning: i32,
    pub cache: CacheTokens,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheTokens {
    pub read: i32,
    pub write: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum ToolState {
    #[serde(rename = "pending")]
    Pending {
        input: serde_json::Value,
        raw: String,
    },
    #[serde(rename = "running")]
    Running {
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
        time: RunningTime,
    },
    #[serde(rename = "completed")]
    Completed {
        input: serde_json::Value,
        output: String,
        title: String,
        metadata: HashMap<String, serde_json::Value>,
        time: CompletedTime,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<FilePart>>,
    },
    #[serde(rename = "error")]
    Error {
        input: serde_json::Value,
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
        time: ErrorTime,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningTime {
    pub start: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedTime {
    pub start: i64,
    pub end: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compacted: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorTime {
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPart {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub call_id: String,
    pub tool: String,
    pub state: ToolState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum MessageInfo {
    #[serde(rename = "user")]
    User {
        id: String,
        session_id: String,
        time: UserTime,
        agent: String,
        model: ModelRef,
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<OutputFormat>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<UserSummary>,
        #[serde(skip_serializing_if = "Option::is_none")]
        system: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tools: Option<HashMap<String, bool>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
    },
    #[serde(rename = "assistant")]
    Assistant {
        id: String,
        session_id: String,
        time: AssistantTime,
        parent_id: String,
        model_id: String,
        provider_id: String,
        mode: String,
        agent: String,
        path: MessagePath,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<bool>,
        cost: f64,
        tokens: AssistantTokens,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<MessageError>,
        #[serde(skip_serializing_if = "Option::is_none")]
        structured: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        finish: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTime {
    pub created: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub diffs: Vec<FileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_content: Option<String>,
    pub new_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantTime {
    pub created: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePath {
    pub cwd: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantTokens {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i32>,
    pub input: i32,
    pub output: i32,
    pub reasoning: i32,
    pub cache: CacheTokens,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "name")]
pub enum MessageError {
    #[serde(rename = "OutputLengthError")]
    OutputLengthError { message: String },
    #[serde(rename = "AbortedError")]
    AbortedError { message: String },
    #[serde(rename = "StructuredOutputError")]
    StructuredOutputError { message: String, retries: i32 },
    #[serde(rename = "AuthError")]
    AuthError {
        provider_id: String,
        message: String,
    },
    #[serde(rename = "APIError")]
    ApiError {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status_code: Option<i32>,
        is_retryable: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        response_headers: Option<HashMap<String, String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        response_body: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, String>>,
    },
    #[serde(rename = "ContextOverflowError")]
    ContextOverflowError {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        response_body: Option<String>,
    },
    #[serde(rename = "UnknownError")]
    Unknown { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputFormat {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "json_schema")]
    JsonSchema {
        schema: serde_json::Value,
        #[serde(default = "default_retry_count")]
        retry_count: i32,
    },
}

fn default_retry_count() -> i32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWithParts {
    pub info: MessageInfo,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Part {
    #[serde(rename = "text")]
    Text {
        id: String,
        session_id: String,
        message_id: String,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        synthetic: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ignored: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        time: Option<TextTime>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
    #[serde(rename = "subtask")]
    Subtask(SubtaskPart),
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        session_id: String,
        message_id: String,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
        time: ReasoningTime,
    },
    #[serde(rename = "file")]
    File(FilePart),
    #[serde(rename = "tool")]
    Tool(ToolPart),
    #[serde(rename = "step-start")]
    StepStart(StepStartPart),
    #[serde(rename = "step-finish")]
    StepFinish(StepFinishPart),
    #[serde(rename = "snapshot")]
    Snapshot {
        id: String,
        session_id: String,
        message_id: String,
        snapshot: String,
    },
    #[serde(rename = "patch")]
    Patch {
        id: String,
        session_id: String,
        message_id: String,
        hash: String,
        files: Vec<String>,
    },
    #[serde(rename = "agent")]
    Agent(AgentPart),
    #[serde(rename = "retry")]
    Retry(RetryPart),
    #[serde(rename = "compaction")]
    Compaction(CompactionPart),
}

impl Part {
    /// Get the ID of this part, regardless of variant
    pub fn id(&self) -> Option<&str> {
        match self {
            Part::Text { id, .. } => Some(id),
            Part::Subtask(p) => Some(&p.id),
            Part::Reasoning { id, .. } => Some(id),
            Part::File(p) => Some(&p.id),
            Part::Tool(p) => Some(&p.id),
            Part::StepStart(p) => Some(&p.id),
            Part::StepFinish(p) => Some(&p.id),
            Part::Snapshot { id, .. } => Some(id),
            Part::Patch { id, .. } => Some(id),
            Part::Agent(p) => Some(&p.id),
            Part::Retry(p) => Some(&p.id),
            Part::Compaction(p) => Some(&p.id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningTime {
    pub start: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<i32>,
    pub is_retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_headers: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartDelta {
    pub session_id: String,
    pub message_id: String,
    pub part_id: String,
    pub field: String,
    pub delta: String,
}

/// Events emitted by the message system, mirroring the TS `MessageV2.Event` namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessageEvent {
    #[serde(rename = "message.updated")]
    Updated { info: MessageInfo },
    #[serde(rename = "message.removed")]
    Removed {
        session_id: String,
        message_id: String,
    },
    #[serde(rename = "message.part.updated")]
    PartUpdated { part: Part },
    #[serde(rename = "message.part.delta")]
    PartDelta {
        session_id: String,
        message_id: String,
        part_id: String,
        field: String,
        delta: String,
    },
    #[serde(rename = "message.part.removed")]
    PartRemoved {
        session_id: String,
        message_id: String,
        part_id: String,
    },
}

/// Minimal model context needed by [`to_model_messages`].
///
/// Callers construct this from whatever provider/model representation they have.
#[derive(Debug, Clone)]
pub struct ModelContext {
    /// The provider identifier, e.g. `"anthropic"`.
    pub provider_id: String,
    /// The model identifier, e.g. `"claude-sonnet-4-20250514"`.
    pub model_id: String,
    /// The npm SDK package name used by the provider, e.g. `"@ai-sdk/anthropic"`.
    /// Used to decide whether media can be inlined in tool results.
    pub api_npm: String,
    /// The provider-level API id (used for Gemini version checks).
    pub api_id: String,
}

/// Filter messages down to the window after the last compaction boundary.
///
/// The TS equivalent (`MessageV2.filterCompacted`) accepts an `AsyncIterable`
/// produced by the paginated `MessageV2.stream()` generator, which allows lazy
/// loading and early termination. In Rust we accept a pre-loaded `Vec` instead.
/// This is an intentional design choice: Rust's ownership model and the SQLite
/// backend make eager loading into a Vec both simpler and efficient enough for
/// typical session sizes. The functional semantics (newest-first iteration,
/// early break on compaction boundary, final reverse) are identical.
pub async fn filter_compacted(messages: Vec<MessageWithParts>) -> Vec<MessageWithParts> {
    let mut result = Vec::new();
    let mut completed = std::collections::HashSet::new();

    for msg in messages {
        match &msg.info {
            MessageInfo::User { id, .. } => {
                let has_compaction = msg
                    .parts
                    .iter()
                    .any(|p| matches!(p, Part::Compaction { .. }));
                if completed.contains(id) && has_compaction {
                    result.push(msg);
                    break;
                }
            }
            MessageInfo::Assistant {
                summary,
                finish,
                parent_id,
                ..
            } => {
                if summary.is_some() && finish.is_some() {
                    completed.insert(parent_id.clone());
                }
            }
        }
        result.push(msg);
    }

    result.reverse();
    result
}

/// Convert an arbitrary error into a [`MessageError`].
///
/// This mirrors the TS `MessageV2.fromError` function. It inspects the error
/// chain for well-known provider error types and falls back to `Unknown`.
pub fn error_from_anyhow(e: anyhow::Error, provider_id: &str) -> MessageError {
    let err_str = e.to_string();

    // 1. AbortError – the operation was cancelled / aborted.
    if err_str.contains("abort") || err_str.contains("cancelled") || err_str.contains("AbortError")
    {
        return MessageError::AbortedError { message: err_str };
    }

    // 2. OutputLengthError – model hit its max output token limit.
    if err_str.contains("output length")
        || err_str.contains("max_tokens")
        || err_str.contains("output_length")
        || err_str.contains("OutputLengthError")
    {
        return MessageError::OutputLengthError { message: err_str };
    }

    // 3. AuthError – API key / credential issues.
    if err_str.contains("auth")
        || err_str.contains("api key")
        || err_str.contains("API key")
        || err_str.contains("LoadAPIKeyError")
        || err_str.contains("unauthorized")
        || err_str.contains("Unauthorized")
    {
        return MessageError::AuthError {
            provider_id: provider_id.to_string(),
            message: err_str,
        };
    }

    // 4. ECONNRESET / connection reset.
    if err_str.contains("ECONNRESET")
        || err_str.contains("connection reset")
        || err_str.contains("Connection reset")
    {
        let mut metadata = HashMap::new();
        metadata.insert("code".to_string(), "ECONNRESET".to_string());
        metadata.insert("message".to_string(), err_str.clone());
        return MessageError::ApiError {
            message: "Connection reset by server".to_string(),
            status_code: None,
            is_retryable: true,
            response_headers: None,
            response_body: None,
            metadata: Some(metadata),
        };
    }

    // 5. Try to downcast to ProviderError for structured handling.
    if let Some(provider_err) = e.downcast_ref::<ProviderError>() {
        let parsed = parse_api_call_error(provider_id, provider_err);
        return match parsed {
            ParsedAPICallError::ContextOverflow {
                message,
                response_body,
            } => MessageError::ContextOverflowError {
                message,
                response_body,
            },
            ParsedAPICallError::ApiError {
                message,
                status_code,
                is_retryable,
                response_headers,
                response_body,
                metadata,
            } => MessageError::ApiError {
                message,
                status_code: status_code.map(|s| s as i32),
                is_retryable,
                response_headers,
                response_body,
                metadata,
            },
        };
    }

    // 6. Context overflow heuristic on the raw string.
    if ProviderError::is_overflow(&err_str) {
        return MessageError::ContextOverflowError {
            message: err_str,
            response_body: None,
        };
    }

    // 7. Try to parse as a stream error (JSON body with `type: "error"`).
    if let Some(parsed) = try_parse_stream_error(&err_str) {
        return parsed;
    }

    // 8. Generic connection / network errors that are retryable.
    if err_str.contains("connection") || err_str.contains("reset") || err_str.contains("timed out")
    {
        return MessageError::ApiError {
            message: err_str,
            status_code: None,
            is_retryable: true,
            response_headers: None,
            response_body: None,
            metadata: None,
        };
    }

    // 9. Fallback.
    MessageError::Unknown { message: err_str }
}

/// Attempt to interpret `raw` as a JSON stream error body and convert it.
fn try_parse_stream_error(raw: &str) -> Option<MessageError> {
    let parsed = parse_stream_error(raw)?;
    Some(match parsed {
        ParsedStreamError::ContextOverflow {
            message,
            response_body,
        } => MessageError::ContextOverflowError {
            message,
            response_body: Some(response_body),
        },
        ParsedStreamError::ApiError {
            message,
            is_retryable,
            response_body,
        } => MessageError::ApiError {
            message,
            status_code: None,
            is_retryable,
            response_headers: None,
            response_body: Some(response_body),
            metadata: None,
        },
    })
}

// ---------------------------------------------------------------------------
// to_model_messages – convert MessageWithParts[] into provider Message[]
// ---------------------------------------------------------------------------

/// Determines whether the given provider SDK supports media (images, PDFs)
/// directly inside tool result content blocks.
///
/// Providers that do NOT support this require media to be extracted and
/// re-injected as a separate user message.
fn supports_media_in_tool_results(api_npm: &str, api_id: &str) -> bool {
    match api_npm {
        "@ai-sdk/anthropic"
        | "@ai-sdk/openai"
        | "@ai-sdk/amazon-bedrock"
        | "@ai-sdk/google-vertex/anthropic" => true,
        "@ai-sdk/google" => {
            let lower = api_id.to_lowercase();
            lower.contains("gemini-3") && !lower.contains("gemini-2")
        }
        _ => false,
    }
}

/// Extract the base64 payload from a `data:` URL, stripping the prefix.
// TODO: Wire for file attachment support
#[allow(dead_code)]
fn extract_base64_data(url: &str) -> &str {
    if let Some(comma_idx) = url.find(',') {
        &url[comma_idx + 1..]
    } else {
        url
    }
}

/// Convert a tool output value into provider-level content parts.
///
/// The TS `toModelOutput` helper handles three shapes:
/// - plain string  -> single text tool result
/// - object with `text` + optional `attachments` -> text + media parts
/// - anything else -> JSON-serialised text
fn tool_output_to_content_parts(
    output: &str,
    attachments: &[FilePart],
    compacted: bool,
    supports_media: bool,
) -> (String, Vec<ContentPart>, Vec<MediaAttachment>) {
    let text = if compacted {
        "[Old tool result content cleared]".to_string()
    } else {
        output.to_string()
    };

    let effective_attachments: Vec<&FilePart> = if compacted {
        vec![]
    } else {
        attachments.iter().collect()
    };

    let mut inline_parts = Vec::new();
    let mut deferred_media = Vec::new();

    for att in &effective_attachments {
        let is_media = att.mime.starts_with("image/") || att.mime == "application/pdf";
        if is_media && !supports_media {
            deferred_media.push(MediaAttachment {
                mime: att.mime.clone(),
                url: att.url.clone(),
            });
        } else if is_media
            && supports_media
            && att.url.starts_with("data:")
            && att.url.contains(',')
        {
            inline_parts.push(ContentPart {
                content_type: "image_url".to_string(),
                image_url: Some(ImageUrl {
                    url: att.url.clone(),
                }),
                media_type: Some(att.mime.clone()),
                ..Default::default()
            });
        }
    }

    (text, inline_parts, deferred_media)
}

/// A media attachment that could not be inlined in a tool result and must be
/// sent as a separate user message.
#[derive(Debug, Clone)]
struct MediaAttachment {
    mime: String,
    url: String,
}

/// Convert a sequence of [`MessageWithParts`] into provider-level
/// [`ProviderMessage`]s suitable for sending to an LLM API.
///
/// This is the Rust equivalent of the TS `MessageV2.toModelMessages()`.
///
/// The function:
/// - Filters ignored text parts and non-media file parts from user messages.
/// - Converts assistant text / reasoning / tool parts into the provider format.
/// - Handles tool state (completed, error, pending/running interruption).
/// - Extracts media from tool results for providers that don't support inline
///   media, injecting them as separate user messages.
/// - Skips assistant messages that have errors (unless aborted with real parts).
pub fn to_model_messages(input: &[MessageWithParts], model: &ModelContext) -> Vec<ProviderMessage> {
    let mut result: Vec<ProviderMessage> = Vec::new();
    let supports_media = supports_media_in_tool_results(&model.api_npm, &model.api_id);
    let model_key = format!("{}/{}", model.provider_id, model.model_id);

    for msg in input {
        if msg.parts.is_empty() {
            continue;
        }

        match &msg.info {
            // ---------------------------------------------------------------
            // User messages
            // ---------------------------------------------------------------
            MessageInfo::User { .. } => {
                let mut parts: Vec<ContentPart> = Vec::new();

                for part in &msg.parts {
                    match part {
                        Part::Text { text, ignored, .. } => {
                            if ignored != &Some(true) {
                                parts.push(ContentPart {
                                    content_type: "text".to_string(),
                                    text: Some(text.clone()),
                                    ..Default::default()
                                });
                            }
                        }
                        Part::File(fp) => {
                            if fp.mime != "text/plain" && fp.mime != "application/x-directory" {
                                parts.push(ContentPart {
                                    content_type: "file".to_string(),
                                    image_url: Some(ImageUrl {
                                        url: fp.url.clone(),
                                    }),
                                    media_type: Some(fp.mime.clone()),
                                    filename: fp.filename.clone(),
                                    ..Default::default()
                                });
                            }
                        }
                        Part::Compaction(_) => {
                            parts.push(ContentPart {
                                content_type: "text".to_string(),
                                text: Some("What did we do so far?".to_string()),
                                ..Default::default()
                            });
                        }
                        Part::Subtask(_) => {
                            parts.push(ContentPart {
                                content_type: "text".to_string(),
                                text: Some(
                                    "The following tool was executed by the user".to_string(),
                                ),
                                ..Default::default()
                            });
                        }
                        _ => {}
                    }
                }

                if !parts.is_empty() {
                    result.push(ProviderMessage {
                        role: Role::User,
                        content: Content::Parts(parts),
                        cache_control: None,
                        provider_options: None,
                    });
                }
            }

            // ---------------------------------------------------------------
            // Assistant messages
            // ---------------------------------------------------------------
            MessageInfo::Assistant {
                provider_id,
                model_id: msg_model_id,
                error,
                ..
            } => {
                let different_model = model_key != format!("{}/{}", provider_id, msg_model_id);

                // Skip messages with errors, unless it's an AbortedError and
                // the message has substantive parts (not just step-start / reasoning).
                if let Some(err) = error {
                    let is_aborted = matches!(err, MessageError::AbortedError { .. });
                    let has_real_parts = msg
                        .parts
                        .iter()
                        .any(|p| !matches!(p, Part::StepStart(_) | Part::Reasoning { .. }));
                    if !(is_aborted && has_real_parts) {
                        continue;
                    }
                }

                let mut assistant_parts: Vec<ContentPart> = Vec::new();
                let mut tool_results: Vec<ProviderMessage> = Vec::new();
                let mut pending_media: Vec<MediaAttachment> = Vec::new();

                for part in &msg.parts {
                    match part {
                        Part::Text { text, metadata, .. } => {
                            let provider_meta = if different_model {
                                None
                            } else {
                                metadata.as_ref().map(|m| {
                                    m.iter()
                                        .map(|(k, v)| (k.clone(), v.clone()))
                                        .collect::<HashMap<String, serde_json::Value>>()
                                })
                            };
                            assistant_parts.push(ContentPart {
                                content_type: "text".to_string(),
                                text: Some(text.clone()),
                                provider_options: provider_meta,
                                ..Default::default()
                            });
                        }
                        Part::StepStart(_) => {
                            // step-start is a UI-only marker; skip for model messages
                        }
                        Part::Tool(tp) => {
                            let call_meta = if different_model {
                                None
                            } else {
                                tp.metadata.clone()
                            };
                            // Emit the tool_use on the assistant side
                            assistant_parts.push(ContentPart {
                                content_type: "tool_use".to_string(),
                                tool_use: Some(ToolUse {
                                    id: tp.call_id.clone(),
                                    name: tp.tool.clone(),
                                    input: match &tp.state {
                                        ToolState::Pending { input, .. }
                                        | ToolState::Running { input, .. }
                                        | ToolState::Completed { input, .. }
                                        | ToolState::Error { input, .. } => input.clone(),
                                    },
                                }),
                                provider_options: call_meta.clone(),
                                ..Default::default()
                            });

                            // Emit the corresponding tool result
                            match &tp.state {
                                ToolState::Completed {
                                    output,
                                    attachments,
                                    time,
                                    ..
                                } => {
                                    let compacted = time.compacted.is_some();
                                    let atts = attachments.as_deref().unwrap_or(&[]);
                                    let (text, _inline, media) = tool_output_to_content_parts(
                                        output,
                                        atts,
                                        compacted,
                                        supports_media,
                                    );

                                    pending_media.extend(media);

                                    tool_results.push(ProviderMessage {
                                        role: Role::Tool,
                                        content: Content::Parts(vec![ContentPart {
                                            content_type: "tool_result".to_string(),
                                            text: Some(text),
                                            tool_result: Some(ProviderToolResult {
                                                tool_use_id: tp.call_id.clone(),
                                                content: output.clone(),
                                                is_error: Some(false),
                                            }),
                                            ..Default::default()
                                        }]),
                                        cache_control: None,
                                        provider_options: None,
                                    });
                                }
                                ToolState::Error { error, .. } => {
                                    tool_results.push(ProviderMessage {
                                        role: Role::Tool,
                                        content: Content::Parts(vec![ContentPart {
                                            content_type: "tool_result".to_string(),
                                            text: Some(error.clone()),
                                            tool_result: Some(ProviderToolResult {
                                                tool_use_id: tp.call_id.clone(),
                                                content: error.clone(),
                                                is_error: Some(true),
                                            }),
                                            ..Default::default()
                                        }]),
                                        cache_control: None,
                                        provider_options: None,
                                    });
                                }
                                ToolState::Pending { .. } | ToolState::Running { .. } => {
                                    tool_results.push(ProviderMessage {
                                        role: Role::Tool,
                                        content: Content::Parts(vec![ContentPart {
                                            content_type: "tool_result".to_string(),
                                            text: Some(
                                                "[Tool execution was interrupted]".to_string(),
                                            ),
                                            tool_result: Some(ProviderToolResult {
                                                tool_use_id: tp.call_id.clone(),
                                                content: "[Tool execution was interrupted]"
                                                    .to_string(),
                                                is_error: Some(true),
                                            }),
                                            ..Default::default()
                                        }]),
                                        cache_control: None,
                                        provider_options: None,
                                    });
                                }
                            }
                        }
                        Part::Reasoning { text, metadata, .. } => {
                            let provider_meta = if different_model {
                                None
                            } else {
                                metadata.as_ref().map(|m| {
                                    m.iter()
                                        .map(|(k, v)| (k.clone(), v.clone()))
                                        .collect::<HashMap<String, serde_json::Value>>()
                                })
                            };
                            assistant_parts.push(ContentPart {
                                content_type: "reasoning".to_string(),
                                text: Some(text.clone()),
                                provider_options: provider_meta,
                                ..Default::default()
                            });
                        }
                        _ => {}
                    }
                }

                // Only emit the assistant message if it has content
                if !assistant_parts.is_empty() {
                    result.push(ProviderMessage {
                        role: Role::Assistant,
                        content: Content::Parts(assistant_parts),
                        cache_control: None,
                        provider_options: None,
                    });

                    // Append tool result messages
                    result.extend(tool_results);

                    // Inject deferred media as a separate user message
                    if !pending_media.is_empty() {
                        let mut media_parts = vec![ContentPart {
                            content_type: "text".to_string(),
                            text: Some("Attached image(s) from tool result:".to_string()),
                            ..Default::default()
                        }];
                        for att in &pending_media {
                            media_parts.push(ContentPart {
                                content_type: "file".to_string(),
                                image_url: Some(ImageUrl {
                                    url: att.url.clone(),
                                }),
                                media_type: Some(att.mime.clone()),
                                ..Default::default()
                            });
                        }
                        result.push(ProviderMessage {
                            role: Role::User,
                            content: Content::Parts(media_parts),
                            cache_control: None,
                            provider_options: None,
                        });
                    }
                }
            }
        }
    }

    // Filter out messages that consist only of step-start markers
    result
        .into_iter()
        .filter(|msg| match &msg.content {
            Content::Text(t) => !t.is_empty(),
            Content::Parts(parts) => parts.iter().any(|p| p.content_type != "step-start"),
        })
        .collect()
}
