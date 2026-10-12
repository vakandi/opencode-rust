use std::collections::HashMap;
use std::fmt;

use crate::models;
use crate::{CacheControl, Content, ContentPart, Message};
use serde::{Deserialize, Serialize};

macro_rules! hashmap {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut map = HashMap::new();
        $(map.insert($key.to_string(), $value);)*
        map
    }};
}

#[derive(Debug, Clone, Copy)]
pub enum ProviderType {
    Anthropic,
    OpenRouter,
    Bedrock,
    OpenAI,
    Gateway,
    Other,
}

impl ProviderType {
    pub fn from_provider_id(id: &str) -> Self {
        let id_lower = id.to_lowercase();
        if id_lower == "anthropic" || id_lower.contains("claude") {
            ProviderType::Anthropic
        } else if id_lower == "openrouter" {
            ProviderType::OpenRouter
        } else if id_lower == "bedrock" || id_lower.contains("bedrock") {
            ProviderType::Bedrock
        } else if id_lower == "gateway" {
            ProviderType::Gateway
        } else if id_lower == "openai" || id_lower == "azure" {
            ProviderType::OpenAI
        } else {
            ProviderType::Other
        }
    }

    pub fn supports_caching(&self) -> bool {
        matches!(
            self,
            ProviderType::Anthropic
                | ProviderType::OpenRouter
                | ProviderType::Bedrock
                | ProviderType::Gateway
        )
    }

    pub fn supports_interleaved_thinking(&self) -> bool {
        matches!(self, ProviderType::Anthropic | ProviderType::OpenRouter)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningContent {
    pub text: String,
    pub signature: Option<String>,
}

/// The TS source uses `Flag.OPENCODE_EXPERIMENTAL_OUTPUT_TOKEN_MAX || 32_000`.
/// We default to 32_000 to match the TS constant.
pub const OUTPUT_TOKEN_MAX: u64 = 32_000;

const WIDELY_SUPPORTED_EFFORTS: &[&str] = &["low", "medium", "high"];
const OPENAI_EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh"];

/// Maps model ID prefix to provider slug used in providerOptions.
/// Example: "amazon/nova-2-lite" -> "bedrock"
const SLUG_OVERRIDES: &[(&str, &str)] = &[("amazon", "bedrock")];

fn slug_override(key: &str) -> Option<&'static str> {
    SLUG_OVERRIDES
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
}

// ---------------------------------------------------------------------------
// apply_caching
// ---------------------------------------------------------------------------

pub fn apply_caching(messages: &mut [Message], provider_type: ProviderType) {
    if !provider_type.supports_caching() {
        return;
    }

    let system_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.role, crate::Role::System))
        .map(|(i, _)| i)
        .take(2)
        .collect();

    let total = messages.len();
    let final_indices: Vec<usize> = (total.saturating_sub(2)..total).collect();

    let mut indices_to_cache: Vec<usize> = Vec::new();
    for idx in system_indices.into_iter().chain(final_indices.into_iter()) {
        if !indices_to_cache.contains(&idx) {
            indices_to_cache.push(idx);
        }
    }

    for idx in indices_to_cache {
        if let Some(msg) = messages.get_mut(idx) {
            apply_cache_to_message(msg, provider_type);
        }
    }
}

fn apply_cache_to_message(message: &mut Message, provider_type: ProviderType) {
    // TS applyCaching uses providerOptions with multiple provider keys merged via mergeDeep.
    // We replicate that by setting provider_options on the message or its last content part.
    let provider_opts = build_cache_provider_options();

    let provider_id_str = match provider_type {
        ProviderType::Anthropic => "anthropic",
        ProviderType::Bedrock => "bedrock",
        _ => "",
    };
    let use_message_level = provider_id_str == "anthropic" || provider_id_str.contains("bedrock");

    if !use_message_level {
        if let Content::Parts(parts) = &mut message.content {
            if let Some(last_part) = parts.last_mut() {
                let existing = last_part.provider_options.get_or_insert_with(HashMap::new);
                merge_deep_into(existing, &provider_opts);
                return;
            }
        }
    }

    // Fall back to message-level providerOptions
    let existing = message.provider_options.get_or_insert_with(HashMap::new);
    merge_deep_into(existing, &provider_opts);
}

fn build_cache_provider_options() -> HashMap<String, serde_json::Value> {
    use serde_json::json;
    let mut opts = HashMap::new();
    opts.insert(
        "anthropic".to_string(),
        json!({"cacheControl": {"type": "ephemeral"}}),
    );
    opts.insert(
        "openrouter".to_string(),
        json!({"cacheControl": {"type": "ephemeral"}}),
    );
    opts.insert(
        "bedrock".to_string(),
        json!({"cachePoint": {"type": "default"}}),
    );
    opts.insert(
        "openaiCompatible".to_string(),
        json!({"cache_control": {"type": "ephemeral"}}),
    );
    opts.insert(
        "copilot".to_string(),
        json!({"copilot_cache_control": {"type": "ephemeral"}}),
    );
    opts
}

/// Deep-merge `source` into `target`. For nested JSON objects, recurse; otherwise overwrite.
fn merge_deep_into(
    target: &mut HashMap<String, serde_json::Value>,
    source: &HashMap<String, serde_json::Value>,
) {
    for (k, v) in source {
        if let Some(existing) = target.get_mut(k) {
            if let (Some(existing_obj), Some(new_obj)) = (existing.as_object_mut(), v.as_object()) {
                for (nk, nv) in new_obj {
                    existing_obj.insert(nk.clone(), nv.clone());
                }
                continue;
            }
        }
        target.insert(k.clone(), v.clone());
    }
}

// ---------------------------------------------------------------------------
// normalize_messages_for_caching
// ---------------------------------------------------------------------------

pub fn normalize_messages_for_caching(messages: &mut [Message]) {
    for msg in messages.iter_mut() {
        if matches!(msg.role, crate::Role::Assistant) {
            if let Content::Text(ref text) = msg.content {
                if text.is_empty() {
                    msg.content = Content::Text(" ".to_string());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// apply_interleaved_thinking
// ---------------------------------------------------------------------------

pub fn apply_interleaved_thinking(messages: &mut [Message], provider_type: ProviderType) {
    if !provider_type.supports_interleaved_thinking() {
        return;
    }

    for msg in messages.iter_mut() {
        if matches!(msg.role, crate::Role::Assistant) {
            if let Content::Parts(parts) = &mut msg.content {
                let reasoning_parts: Vec<&ContentPart> = parts
                    .iter()
                    .filter(|p| p.content_type == "reasoning")
                    .collect();

                if !reasoning_parts.is_empty() {
                    let reasoning_text: String = reasoning_parts
                        .iter()
                        .filter_map(|p| p.text.as_ref())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("");

                    parts.retain(|p| p.content_type != "reasoning");

                    if !reasoning_text.is_empty() {
                        if let Some(part) = parts.last_mut() {
                            part.cache_control = Some(CacheControl::ephemeral());
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// extract_reasoning_from_response
// ---------------------------------------------------------------------------

pub fn extract_reasoning_from_response(content: &str) -> (Option<String>, String) {
    let thinking_start = content.find("<thinking>");
    let thinking_end = content.find("</thinking>");

    match (thinking_start, thinking_end) {
        (Some(start), Some(end)) if end > start => {
            let reasoning = content[start + 10..end].trim().to_string();
            let rest = format!("{}{}", content[..start].trim(), content[end + 11..].trim());
            (Some(reasoning), rest)
        }
        _ => (None, content.to_string()),
    }
}

// ---------------------------------------------------------------------------
// normalize_messages
// ---------------------------------------------------------------------------

pub fn normalize_messages(
    messages: &mut Vec<Message>,
    provider_type: ProviderType,
    model_id: &str,
) {
    match provider_type {
        ProviderType::Anthropic => {
            normalize_for_anthropic(messages);
            normalize_tool_call_ids_claude(messages);
        }
        ProviderType::OpenRouter => {
            if model_id.to_lowercase().contains("claude") {
                normalize_tool_call_ids_claude(messages);
            }
            if model_id.to_lowercase().contains("mistral")
                || model_id.to_lowercase().contains("devstral")
            {
                normalize_for_mistral(messages);
            }
        }
        ProviderType::Other => {
            if model_id.to_lowercase().contains("mistral")
                || model_id.to_lowercase().contains("devstral")
            {
                normalize_for_mistral(messages);
            } else if model_id.to_lowercase().contains("claude") {
                normalize_tool_call_ids_claude(messages);
            }
        }
        _ => {}
    }

    // Handle interleaved thinking field (move reasoning to providerOptions)
    normalize_interleaved_field(messages, model_id);
}

/// For models with interleaved thinking that use a specific field
/// (reasoning_content or reasoning_details), move reasoning parts
/// from content into providerOptions.openaiCompatible.<field>.
fn normalize_interleaved_field(_messages: &mut Vec<Message>, _model_id: &str) {
    // This is handled at a higher level via the ModelInfo.interleaved field.
    // The caller should check model.interleaved and pass the field name.
    // For now this is a no-op; the full implementation is in
    // normalize_messages_with_interleaved_field below.
}

/// Normalize messages for models that store reasoning in a specific provider field.
/// Matches the TS: `if (typeof model.capabilities.interleaved === "object" && model.capabilities.interleaved.field)`
pub fn normalize_messages_with_interleaved_field(messages: &mut Vec<Message>, field: &str) {
    use serde_json::json;
    for msg in messages.iter_mut() {
        if !matches!(msg.role, crate::Role::Assistant) {
            continue;
        }
        if let Content::Parts(parts) = &mut msg.content {
            let reasoning_text: String = parts
                .iter()
                .filter(|p| p.content_type == "reasoning")
                .filter_map(|p| p.text.as_ref())
                .cloned()
                .collect::<Vec<_>>()
                .join("");

            parts.retain(|p| p.content_type != "reasoning");

            if !reasoning_text.is_empty() {
                let po = msg.provider_options.get_or_insert_with(HashMap::new);
                let compat = po
                    .entry("openaiCompatible".to_string())
                    .or_insert_with(|| json!({}));
                if let Some(obj) = compat.as_object_mut() {
                    obj.insert(field.to_string(), json!(reasoning_text));
                }
            }
        }
    }
}

fn normalize_for_anthropic(messages: &mut Vec<Message>) {
    // Filter out messages with empty content
    messages.retain(|msg| match &msg.content {
        Content::Text(text) => !text.is_empty(),
        Content::Parts(parts) => parts.iter().any(|p| {
            if p.content_type == "text" || p.content_type == "reasoning" {
                p.text.as_ref().map(|t| !t.is_empty()).unwrap_or(false)
            } else {
                true
            }
        }),
    });

    // Filter out empty text/reasoning parts within messages
    for msg in messages.iter_mut() {
        if let Content::Parts(parts) = &mut msg.content {
            parts.retain(|p| {
                if p.content_type == "text" || p.content_type == "reasoning" {
                    p.text.as_ref().map(|t| !t.is_empty()).unwrap_or(false)
                } else {
                    true
                }
            });
        }
    }
}

fn normalize_tool_call_ids_claude(messages: &mut [Message]) {
    for msg in messages.iter_mut() {
        if matches!(msg.role, crate::Role::Assistant | crate::Role::Tool) {
            if let Content::Parts(parts) = &mut msg.content {
                for part in parts.iter_mut() {
                    if let Some(ref mut tool_use) = part.tool_use {
                        tool_use.id = normalize_tool_call_id(&tool_use.id, true);
                    }
                    if let Some(ref mut tool_result) = part.tool_result {
                        tool_result.tool_use_id =
                            normalize_tool_call_id(&tool_result.tool_use_id, true);
                    }
                }
            }
        }
    }
}

fn normalize_for_mistral(messages: &mut Vec<Message>) {
    for msg in messages.iter_mut() {
        if matches!(msg.role, crate::Role::Assistant | crate::Role::Tool) {
            if let Content::Parts(parts) = &mut msg.content {
                for part in parts.iter_mut() {
                    if let Some(ref mut tool_use) = part.tool_use {
                        tool_use.id = normalize_tool_call_id_mistral(&tool_use.id);
                    }
                    if let Some(ref mut tool_result) = part.tool_result {
                        tool_result.tool_use_id =
                            normalize_tool_call_id_mistral(&tool_result.tool_use_id);
                    }
                }
            }
        }
    }

    let mut i = 0;
    while i < messages.len().saturating_sub(1) {
        let current_is_tool = matches!(messages[i].role, crate::Role::Tool);
        let next_is_user = matches!(messages[i + 1].role, crate::Role::User);

        if current_is_tool && next_is_user {
            messages.insert(i + 1, Message::assistant("Done."));
        }
        i += 1;
    }
}

fn normalize_tool_call_id(id: &str, allow_underscore: bool) -> String {
    if allow_underscore {
        id.chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    } else {
        id.chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect()
    }
}

fn normalize_tool_call_id_mistral(id: &str) -> String {
    let alphanumeric: String = id.chars().filter(|c| c.is_alphanumeric()).collect();
    let first_9: String = alphanumeric.chars().take(9).collect();
    format!("{:0<9}", first_9)
}

// ---------------------------------------------------------------------------
// Modality
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Modality {
    Image,
    Audio,
    Video,
    Pdf,
}

impl fmt::Display for Modality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Modality::Image => write!(f, "image"),
            Modality::Audio => write!(f, "audio"),
            Modality::Video => write!(f, "video"),
            Modality::Pdf => write!(f, "pdf"),
        }
    }
}

pub fn mime_to_modality(mime: &str) -> Option<Modality> {
    if mime.starts_with("image/") {
        Some(Modality::Image)
    } else if mime.starts_with("audio/") {
        Some(Modality::Audio)
    } else if mime.starts_with("video/") {
        Some(Modality::Video)
    } else if mime == "application/pdf" {
        Some(Modality::Pdf)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// unsupported_parts
// ---------------------------------------------------------------------------

pub fn unsupported_parts(messages: &mut [Message], supported_modalities: &[Modality]) {
    for msg in messages.iter_mut() {
        if !matches!(msg.role, crate::Role::User) {
            continue;
        }

        if let Content::Parts(parts) = &mut msg.content {
            for part in parts.iter_mut() {
                if part.content_type != "image" && part.content_type != "file" {
                    continue;
                }

                // Check for empty base64 image data
                if part.content_type == "image" {
                    if let Some(ref image_url) = part.image_url {
                        let url_str = &image_url.url;
                        if url_str.starts_with("data:") {
                            // Match data:<mime>;base64,<data>
                            if let Some(comma_pos) = url_str.find(',') {
                                let data_part = &url_str[comma_pos + 1..];
                                if data_part.is_empty() {
                                    *part = ContentPart {
                                        content_type: "text".to_string(),
                                        text: Some("ERROR: Image file is empty or corrupted. Please provide a valid image.".to_string()),
                                        ..Default::default()
                                    };
                                    continue;
                                }
                            }
                        }
                    }
                }

                let mime = if part.content_type == "image" {
                    part.image_url
                        .as_ref()
                        .and_then(|url| {
                            let url_str = url.url.as_str();
                            if url_str.starts_with("data:") {
                                url_str.split(';').next()
                            } else {
                                None
                            }
                        })
                        .map(|s| s.trim_start_matches("data:").to_string())
                        .unwrap_or_default()
                } else {
                    // For file parts, use media_type field
                    part.media_type.clone().unwrap_or_default()
                };

                if let Some(modality) = mime_to_modality(&mime) {
                    if !supported_modalities.contains(&modality) {
                        // Extract filename for error message
                        let name = if let Some(ref filename) = part.filename {
                            format!("\"{}\"", filename)
                        } else {
                            modality.to_string()
                        };
                        let error_msg = format!(
                            "ERROR: Cannot read {} (this model does not support {} input). Inform the user.",
                            name, modality
                        );
                        *part = ContentPart {
                            content_type: "text".to_string(),
                            text: Some(error_msg),
                            ..Default::default()
                        };
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// temperature / topP / topK
// ---------------------------------------------------------------------------

pub fn temperature_for_model(model_id: &str) -> Option<f32> {
    let id = model_id.to_lowercase();
    if id.contains("qwen") {
        return Some(0.55);
    }
    if id.contains("claude") {
        return None;
    }
    if id.contains("gemini") {
        return Some(1.0);
    }
    if id.contains("glm-4.6") || id.contains("glm-4.7") {
        return Some(1.0);
    }
    if id.contains("minimax-m2") {
        return Some(1.0);
    }
    if id.contains("kimi-k2") {
        if id.contains("thinking") || id.contains("k2.") || id.contains("k2p") {
            return Some(1.0);
        }
        return Some(0.6);
    }
    None
}

pub fn top_p_for_model(model_id: &str) -> Option<f32> {
    let id = model_id.to_lowercase();
    if id.contains("qwen") {
        return Some(1.0);
    }
    if id.contains("minimax-m2")
        || id.contains("kimi-k2.5")
        || id.contains("kimi-k2p5")
        || id.contains("gemini")
    {
        return Some(0.95);
    }
    None
}

pub fn top_k_for_model(model_id: &str) -> Option<u32> {
    let id = model_id.to_lowercase();
    if id.contains("minimax-m2") {
        if id.contains("m2.1") {
            return Some(40);
        }
        return Some(20);
    }
    if id.contains("gemini") {
        return Some(64);
    }
    None
}

// ---------------------------------------------------------------------------
// transform_messages (top-level entry point matching TS `message()`)
// ---------------------------------------------------------------------------

pub fn transform_messages(
    messages: &mut Vec<Message>,
    provider_type: ProviderType,
    model_id: &str,
    supported_modalities: &[Modality],
    npm: &str,
    provider_id: &str,
) {
    unsupported_parts(messages, supported_modalities);
    normalize_messages(messages, provider_type, model_id);

    // TS: apply caching when the model is anthropic/claude/bedrock, but NOT gateway.
    // Checks: providerID == "anthropic", api.id contains "anthropic"/"claude",
    //         model.id contains "anthropic"/"claude", or npm == "@ai-sdk/anthropic"
    let id_lower = model_id.to_lowercase();
    let pid_lower = provider_id.to_lowercase();
    let is_anthropic_like = pid_lower == "anthropic"
        || id_lower.contains("anthropic")
        || id_lower.contains("claude")
        || npm == "@ai-sdk/anthropic";
    if is_anthropic_like && npm != "@ai-sdk/gateway" {
        apply_caching(messages, provider_type);
    }

    // Remap providerOptions keys from stored providerID to expected SDK key
    remap_provider_options(messages, npm, provider_id);
}

/// Remap providerOptions keys from the stored `provider_id` to the expected SDK key.
/// Matches the TS logic that remaps `providerOptions[providerID]` -> `providerOptions[sdkKey]`.
fn remap_provider_options(messages: &mut [Message], npm: &str, provider_id: &str) {
    let key = match sdk_key(npm) {
        Some(k) => k,
        None => return,
    };

    // Skip if the key already matches the provider_id, or if this is Azure
    if key == provider_id || npm == "@ai-sdk/azure" {
        return;
    }

    let remap = |opts: &mut Option<HashMap<String, serde_json::Value>>| {
        let map = match opts.as_mut() {
            Some(m) => m,
            None => return,
        };
        if let Some(val) = map.remove(provider_id) {
            map.insert(key.to_string(), val);
        }
    };

    for msg in messages.iter_mut() {
        remap(&mut msg.provider_options);
        if let Content::Parts(parts) = &mut msg.content {
            for part in parts.iter_mut() {
                remap(&mut part.provider_options);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// normalize_interleaved_thinking
// ---------------------------------------------------------------------------

/// Normalize interleaved thinking content in messages.
/// For providers that don't support interleaved thinking, strip thinking blocks
/// from all but the last assistant message.
pub fn normalize_interleaved_thinking(
    messages: &mut Vec<Message>,
    _provider_type: &ProviderType,
    supports_interleaved: bool,
) {
    if supports_interleaved {
        return;
    }

    let last_assistant_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, crate::Role::Assistant));

    for idx in 0..messages.len() {
        if !matches!(messages[idx].role, crate::Role::Assistant) {
            continue;
        }
        if Some(idx) == last_assistant_idx {
            continue;
        }

        if let Content::Parts(ref mut parts) = messages[idx].content {
            parts
                .retain(|part| part.content_type != "thinking" && part.content_type != "reasoning");

            if parts.is_empty() {
                parts.push(ContentPart {
                    content_type: "text".to_string(),
                    text: Some("[thinking]".to_string()),
                    ..Default::default()
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// apply_caching_per_part
// ---------------------------------------------------------------------------

/// Apply cache control markers at the part level.
pub fn apply_caching_per_part(messages: &mut [Message], provider_type: &ProviderType) {
    match provider_type {
        ProviderType::Anthropic => {
            if let Some(last_user) = messages
                .iter_mut()
                .rev()
                .find(|m| matches!(m.role, crate::Role::User))
            {
                if let Content::Parts(ref mut parts) = last_user.content {
                    if let Some(last_part) = parts.last_mut() {
                        last_part.cache_control = Some(CacheControl::ephemeral());
                    }
                }
                last_user.cache_control = Some(CacheControl::ephemeral());
            }

            for msg in messages.iter_mut() {
                if matches!(msg.role, crate::Role::System) {
                    msg.cache_control = Some(CacheControl::ephemeral());
                    if let Content::Parts(ref mut parts) = msg.content {
                        if let Some(last_part) = parts.last_mut() {
                            last_part.cache_control = Some(CacheControl::ephemeral());
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// max_output_tokens
// ---------------------------------------------------------------------------

/// Get the maximum output tokens for a model, capped at OUTPUT_TOKEN_MAX.
pub fn max_output_tokens(model: &models::ModelInfo) -> u64 {
    let capped = model.limit.output.min(OUTPUT_TOKEN_MAX);
    if capped == 0 {
        OUTPUT_TOKEN_MAX
    } else {
        capped
    }
}

// ---------------------------------------------------------------------------
// sdk_key
// ---------------------------------------------------------------------------

/// Map npm package name to SDK key.
pub fn sdk_key(npm: &str) -> Option<&'static str> {
    match npm {
        "@ai-sdk/github-copilot" => Some("copilot"),
        "@ai-sdk/openai" | "@ai-sdk/azure" => Some("openai"),
        "@ai-sdk/amazon-bedrock" => Some("bedrock"),
        "@ai-sdk/anthropic" | "@ai-sdk/google-vertex/anthropic" => Some("anthropic"),
        "@ai-sdk/google-vertex" | "@ai-sdk/google" => Some("google"),
        "@ai-sdk/gateway" => Some("gateway"),
        "@openrouter/ai-sdk-provider" => Some("openrouter"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// variants
// ---------------------------------------------------------------------------

/// Generate reasoning/thinking configuration variants for a model.
/// Returns a map of variant_name -> config options.
pub fn variants(model: &models::ModelInfo) -> HashMap<String, HashMap<String, serde_json::Value>> {
    use serde_json::json;

    if !model.reasoning {
        return HashMap::new();
    }

    let id = model.id.to_lowercase();

    // Models that don't support configurable reasoning
    if id.contains("deepseek")
        || id.contains("minimax")
        || id.contains("glm")
        || id.contains("mistral")
        || id.contains("kimi")
        || id.contains("k2p5")
    {
        return HashMap::new();
    }

    // Grok special handling
    if id.contains("grok") {
        if id.contains("grok-3-mini") {
            let npm = model.provider.as_ref().and_then(|p| p.npm.as_deref());
            if npm == Some("@openrouter/ai-sdk-provider") {
                return [
                    (
                        "low".into(),
                        hashmap! {"reasoning" => json!({"effort": "low"})},
                    ),
                    (
                        "high".into(),
                        hashmap! {"reasoning" => json!({"effort": "high"})},
                    ),
                ]
                .into_iter()
                .collect();
            }
            return [
                ("low".into(), hashmap! {"reasoningEffort" => json!("low")}),
                ("high".into(), hashmap! {"reasoningEffort" => json!("high")}),
            ]
            .into_iter()
            .collect();
        }
        return HashMap::new();
    }

    let npm = model
        .provider
        .as_ref()
        .and_then(|p| p.npm.as_deref())
        .unwrap_or("");
    let api_id = model
        .provider
        .as_ref()
        .and_then(|p| p.api.as_deref())
        .unwrap_or("");

    match npm {
        "@openrouter/ai-sdk-provider" => {
            if !model.id.contains("gpt")
                && !model.id.contains("gemini-3")
                && !model.id.contains("claude")
            {
                return HashMap::new();
            }
            OPENAI_EFFORTS
                .iter()
                .map(|e| {
                    (
                        e.to_string(),
                        hashmap! {"reasoning" => json!({"effort": *e})},
                    )
                })
                .collect()
        }

        "@ai-sdk/gateway" => {
            if model.id.contains("anthropic") {
                return [
                    (
                        "high".into(),
                        hashmap! {"thinking" => json!({"type": "enabled", "budgetTokens": 16000})},
                    ),
                    (
                        "max".into(),
                        hashmap! {"thinking" => json!({"type": "enabled", "budgetTokens": 31999})},
                    ),
                ]
                .into_iter()
                .collect();
            }
            if model.id.contains("google") {
                if id.contains("2.5") {
                    return [
                        (
                            "high".into(),
                            hashmap! {"thinkingConfig" => json!({"includeThoughts": true, "thinkingBudget": 16000})},
                        ),
                        (
                            "max".into(),
                            hashmap! {"thinkingConfig" => json!({"includeThoughts": true, "thinkingBudget": 24576})},
                        ),
                    ]
                    .into_iter()
                    .collect();
                }
                return ["low", "high"]
                    .iter()
                    .map(|e| {
                        (
                            e.to_string(),
                            hashmap! {
                                "includeThoughts" => json!(true),
                                "thinkingLevel" => json!(*e)
                            },
                        )
                    })
                    .collect();
            }
            OPENAI_EFFORTS
                .iter()
                .map(|e| (e.to_string(), hashmap! {"reasoningEffort" => json!(*e)}))
                .collect()
        }

        "@ai-sdk/github-copilot" => {
            if model.id.contains("gemini") {
                return HashMap::new();
            }
            if model.id.contains("claude") {
                return [(
                    "thinking".into(),
                    hashmap! {"thinking_budget" => json!(4000)},
                )]
                .into_iter()
                .collect();
            }
            let efforts: Vec<&str> =
                if id.contains("5.1-codex-max") || id.contains("5.2") || id.contains("5.3") {
                    vec!["low", "medium", "high", "xhigh"]
                } else {
                    vec!["low", "medium", "high"]
                };
            efforts
                .iter()
                .map(|e| {
                    (
                        e.to_string(),
                        hashmap! {
                            "reasoningEffort" => json!(*e),
                            "reasoningSummary" => json!("auto"),
                            "include" => json!(["reasoning.encrypted_content"])
                        },
                    )
                })
                .collect()
        }

        "@ai-sdk/cerebras"
        | "@ai-sdk/togetherai"
        | "@ai-sdk/xai"
        | "@ai-sdk/deepinfra"
        | "venice-ai-sdk-provider"
        | "@ai-sdk/openai-compatible" => WIDELY_SUPPORTED_EFFORTS
            .iter()
            .map(|e| (e.to_string(), hashmap! {"reasoningEffort" => json!(*e)}))
            .collect(),

        "@ai-sdk/azure" => {
            if id == "o1-mini" {
                return HashMap::new();
            }
            let mut efforts: Vec<&str> = vec!["low", "medium", "high"];
            if id.contains("gpt-5-") || id == "gpt-5" {
                efforts.insert(0, "minimal");
            }
            efforts
                .iter()
                .map(|e| {
                    (
                        e.to_string(),
                        hashmap! {
                            "reasoningEffort" => json!(*e),
                            "reasoningSummary" => json!("auto"),
                            "include" => json!(["reasoning.encrypted_content"])
                        },
                    )
                })
                .collect()
        }

        "@ai-sdk/openai" => {
            if id == "gpt-5-pro" {
                return HashMap::new();
            }
            let efforts: Vec<&str> = if id.contains("codex") {
                if id.contains("5.2") || id.contains("5.3") {
                    vec!["low", "medium", "high", "xhigh"]
                } else {
                    vec!["low", "medium", "high"]
                }
            } else {
                let mut arr: Vec<&str> = vec!["low", "medium", "high"];
                if id.contains("gpt-5-") || id == "gpt-5" {
                    arr.insert(0, "minimal");
                }
                // Check release_date for additional efforts
                let release_date = model.release_date.as_deref().unwrap_or("");
                if release_date >= "2025-11-13" {
                    arr.insert(0, "none");
                }
                if release_date >= "2025-12-04" {
                    arr.push("xhigh");
                }
                arr
            };
            efforts
                .iter()
                .map(|e| {
                    (
                        e.to_string(),
                        hashmap! {
                            "reasoningEffort" => json!(*e),
                            "reasoningSummary" => json!("auto"),
                            "include" => json!(["reasoning.encrypted_content"])
                        },
                    )
                })
                .collect()
        }

        "@ai-sdk/anthropic" | "@ai-sdk/google-vertex/anthropic" => {
            if api_id.contains("opus-4-6") || api_id.contains("opus-4.6") {
                return ["low", "medium", "high", "max"]
                    .iter()
                    .map(|e| {
                        (
                            e.to_string(),
                            hashmap! {
                                "thinking" => json!({"type": "adaptive"}),
                                "effort" => json!(*e)
                            },
                        )
                    })
                    .collect();
            }
            let budget_high = 16_000u64.min(model.limit.output / 2 - 1);
            let budget_max = 31_999u64.min(model.limit.output - 1);
            [
                (
                    "high".into(),
                    hashmap! {"thinking" => json!({"type": "enabled", "budgetTokens": budget_high})},
                ),
                (
                    "max".into(),
                    hashmap! {"thinking" => json!({"type": "enabled", "budgetTokens": budget_max})},
                ),
            ]
            .into_iter()
            .collect()
        }

        "@ai-sdk/amazon-bedrock" => {
            if api_id.contains("opus-4-6") || api_id.contains("opus-4.6") {
                return ["low", "medium", "high", "max"]
                    .iter()
                    .map(|e| {
                        (
                            e.to_string(),
                            hashmap! {
                                "reasoningConfig" => json!({"type": "adaptive", "maxReasoningEffort": *e})
                            },
                        )
                    })
                    .collect();
            }
            if api_id.contains("anthropic") {
                return [
                    (
                        "high".into(),
                        hashmap! {"reasoningConfig" => json!({"type": "enabled", "budgetTokens": 16000})},
                    ),
                    (
                        "max".into(),
                        hashmap! {"reasoningConfig" => json!({"type": "enabled", "budgetTokens": 31999})},
                    ),
                ]
                .into_iter()
                .collect();
            }
            // Amazon Nova models
            WIDELY_SUPPORTED_EFFORTS
                .iter()
                .map(|e| {
                    (
                        e.to_string(),
                        hashmap! {
                            "reasoningConfig" => json!({"type": "enabled", "maxReasoningEffort": *e})
                        },
                    )
                })
                .collect()
        }

        "@ai-sdk/google-vertex" | "@ai-sdk/google" => {
            if id.contains("2.5") {
                return [
                    (
                        "high".into(),
                        hashmap! {"thinkingConfig" => json!({"includeThoughts": true, "thinkingBudget": 16000})},
                    ),
                    (
                        "max".into(),
                        hashmap! {"thinkingConfig" => json!({"includeThoughts": true, "thinkingBudget": 24576})},
                    ),
                ]
                .into_iter()
                .collect();
            }
            ["low", "high"]
                .iter()
                .map(|e| {
                    (
                        e.to_string(),
                        hashmap! {
                            "includeThoughts" => json!(true),
                            "thinkingLevel" => json!(*e)
                        },
                    )
                })
                .collect()
        }

        "@ai-sdk/groq" => ["none", "low", "medium", "high"]
            .iter()
            .map(|e| {
                (
                    e.to_string(),
                    hashmap! {
                        "includeThoughts" => json!(true),
                        "thinkingLevel" => json!(*e)
                    },
                )
            })
            .collect(),

        "@ai-sdk/mistral" | "@ai-sdk/cohere" | "@ai-sdk/perplexity" => HashMap::new(),

        "@mymediset/sap-ai-provider" | "@jerome-benoit/sap-ai-provider-v2" => {
            if api_id.contains("anthropic") {
                return [
                    (
                        "high".into(),
                        hashmap! {"thinking" => json!({"type": "enabled", "budgetTokens": 16000})},
                    ),
                    (
                        "max".into(),
                        hashmap! {"thinking" => json!({"type": "enabled", "budgetTokens": 31999})},
                    ),
                ]
                .into_iter()
                .collect();
            }
            WIDELY_SUPPORTED_EFFORTS
                .iter()
                .map(|e| (e.to_string(), hashmap! {"reasoningEffort" => json!(*e)}))
                .collect()
        }

        _ => HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// options
// ---------------------------------------------------------------------------

/// Generate provider-specific options for a model.
pub fn options(
    model: &models::ModelInfo,
    session_id: &str,
    provider_options: &HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    use serde_json::json;
    let mut result = HashMap::new();

    let npm = model
        .provider
        .as_ref()
        .and_then(|p| p.npm.as_deref())
        .unwrap_or("");
    let api_id = model
        .provider
        .as_ref()
        .and_then(|p| p.api.as_deref())
        .unwrap_or("");
    let model_id = model.id.to_lowercase();
    let provider_id = &model_id; // In the TS, model.providerID is used; here we approximate from model.id

    // OpenAI store=false
    if provider_id == "openai" || npm == "@ai-sdk/openai" || npm == "@ai-sdk/github-copilot" {
        result.insert("store".to_string(), json!(false));
    }

    // OpenRouter usage include
    if npm == "@openrouter/ai-sdk-provider" {
        result.insert("usage".to_string(), json!({"include": true}));
        if api_id.contains("gemini-3") {
            result.insert("reasoning".to_string(), json!({"effort": "high"}));
        }
    }

    // Baseten / opencode chat_template_args
    if provider_id == "baseten"
        || (provider_id.starts_with("opencode")
            && (api_id == "kimi-k2-thinking" || api_id == "glm-4.6"))
    {
        result.insert(
            "chat_template_args".to_string(),
            json!({"enable_thinking": true}),
        );
    }

    // zai/zhipuai thinking config
    if (provider_id == "zai" || provider_id == "zhipuai") && npm == "@ai-sdk/openai-compatible" {
        result.insert(
            "thinking".to_string(),
            json!({"type": "enabled", "clear_thinking": false}),
        );
    }

    // OpenAI prompt cache key
    if provider_id == "openai"
        || provider_options
            .get("setCacheKey")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        result.insert("promptCacheKey".to_string(), json!(session_id));
    }

    // Google thinking config
    if npm == "@ai-sdk/google" || npm == "@ai-sdk/google-vertex" {
        let mut thinking = json!({"includeThoughts": true});
        if api_id.contains("gemini-3") {
            thinking["thinkingLevel"] = json!("high");
        }
        result.insert("thinkingConfig".to_string(), thinking);
    }

    // Anthropic thinking for kimi-k2.5/k2p5 models
    let api_id_lower = api_id.to_lowercase();
    if (npm == "@ai-sdk/anthropic" || npm == "@ai-sdk/google-vertex/anthropic")
        && (api_id_lower.contains("k2p5")
            || api_id_lower.contains("kimi-k2.5")
            || api_id_lower.contains("kimi-k2p5"))
    {
        let budget = 16_000u64.min(model.limit.output / 2 - 1);
        result.insert(
            "thinking".to_string(),
            json!({"type": "enabled", "budgetTokens": budget}),
        );
    }

    // Alibaba-cn enable_thinking
    if provider_id == "alibaba-cn"
        && model.reasoning
        && npm == "@ai-sdk/openai-compatible"
        && !api_id_lower.contains("kimi-k2-thinking")
    {
        result.insert("enable_thinking".to_string(), json!(true));
    }

    // GPT-5 reasoning effort/summary/verbosity
    if api_id.contains("gpt-5") && !api_id.contains("gpt-5-chat") {
        if !api_id.contains("gpt-5-pro") {
            result.insert("reasoningEffort".to_string(), json!("medium"));
            result.insert("reasoningSummary".to_string(), json!("auto"));
        }

        // textVerbosity for non-chat gpt-5.x models
        if api_id.contains("gpt-5.")
            && !api_id.contains("codex")
            && !api_id.contains("-chat")
            && provider_id != "azure"
        {
            result.insert("textVerbosity".to_string(), json!("low"));
        }

        if provider_id.starts_with("opencode") {
            result.insert("promptCacheKey".to_string(), json!(session_id));
            result.insert(
                "include".to_string(),
                json!(["reasoning.encrypted_content"]),
            );
            result.insert("reasoningSummary".to_string(), json!("auto"));
        }
    }

    // Venice promptCacheKey
    if provider_id == "venice" {
        result.insert("promptCacheKey".to_string(), json!(session_id));
    }

    // OpenRouter prompt_cache_key
    if provider_id == "openrouter" {
        result.insert("prompt_cache_key".to_string(), json!(session_id));
    }

    // Gateway caching
    if npm == "@ai-sdk/gateway" {
        result.insert("gateway".to_string(), json!({"caching": "auto"}));
    }

    result
}

// ---------------------------------------------------------------------------
// small_options
// ---------------------------------------------------------------------------

/// Generate small model options (reduced reasoning effort).
pub fn small_options(model: &models::ModelInfo) -> HashMap<String, serde_json::Value> {
    use serde_json::json;
    let mut result = HashMap::new();

    let npm = model
        .provider
        .as_ref()
        .and_then(|p| p.npm.as_deref())
        .unwrap_or("");
    let api_id = model
        .provider
        .as_ref()
        .and_then(|p| p.api.as_deref())
        .unwrap_or("");
    let provider_id = model.id.to_lowercase();

    if provider_id == "openai" || npm == "@ai-sdk/openai" || npm == "@ai-sdk/github-copilot" {
        result.insert("store".to_string(), json!(false));
        if api_id.contains("gpt-5") {
            if api_id.contains("5.") {
                result.insert("reasoningEffort".to_string(), json!("low"));
            } else {
                result.insert("reasoningEffort".to_string(), json!("minimal"));
            }
        }
        return result;
    }

    if provider_id == "google" {
        // gemini-3 uses thinkingLevel, gemini-2.5 uses thinkingBudget
        if api_id.contains("gemini-3") {
            result.insert(
                "thinkingConfig".to_string(),
                json!({"thinkingLevel": "minimal"}),
            );
        } else {
            result.insert("thinkingConfig".to_string(), json!({"thinkingBudget": 0}));
        }
        return result;
    }

    if provider_id == "openrouter" {
        if api_id.contains("google") {
            result.insert("reasoning".to_string(), json!({"enabled": false}));
        } else {
            result.insert("reasoningEffort".to_string(), json!("minimal"));
        }
        return result;
    }

    result
}

// ---------------------------------------------------------------------------
// schema (Gemini schema sanitization)
// ---------------------------------------------------------------------------

/// Sanitize a JSON schema for Gemini/Google models.
/// - Convert integer enums to string enums
/// - Recursive sanitization of nested objects/arrays
/// - Filter required array to only include fields in properties
/// - Remove properties/required from non-object types
/// - Handle empty array items
pub fn schema(model: &models::ModelInfo, input_schema: serde_json::Value) -> serde_json::Value {
    let provider_id = model.id.to_lowercase();
    let api_id = model
        .provider
        .as_ref()
        .and_then(|p| p.api.as_deref())
        .unwrap_or("");

    if provider_id == "google" || api_id.contains("gemini") {
        sanitize_gemini(input_schema)
    } else {
        input_schema
    }
}

fn sanitize_gemini(obj: serde_json::Value) -> serde_json::Value {
    use serde_json::{json, Map, Value};

    match obj {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => obj,
        Value::Array(arr) => Value::Array(arr.into_iter().map(sanitize_gemini).collect()),
        Value::Object(map) => {
            let mut result = Map::new();

            for (key, value) in map {
                if key == "enum" {
                    if let Value::Array(ref enum_vals) = value {
                        // Convert all enum values to strings
                        let string_vals: Vec<Value> = enum_vals
                            .iter()
                            .map(|v| match v {
                                Value::String(s) => Value::String(s.clone()),
                                other => Value::String(other.to_string()),
                            })
                            .collect();
                        result.insert(key, Value::Array(string_vals));

                        // If we have integer/number type with enum, change to string
                        if let Some(Value::String(t)) = result.get("type") {
                            if t == "integer" || t == "number" {
                                result.insert(
                                    "type".to_string(),
                                    Value::String("string".to_string()),
                                );
                            }
                        }
                    } else {
                        result.insert(key, value);
                    }
                } else if value.is_object() || value.is_array() {
                    result.insert(key, sanitize_gemini(value));
                } else {
                    result.insert(key, value);
                }
            }

            // Also check if type was set before enum was processed
            // (enum might appear before type in iteration order)
            if let Some(Value::Array(ref enum_vals)) = result.get("enum") {
                if !enum_vals.is_empty() {
                    if let Some(Value::String(t)) = result.get("type") {
                        if t == "integer" || t == "number" {
                            result.insert("type".to_string(), Value::String("string".to_string()));
                        }
                    }
                }
            }

            // Filter required array to only include fields in properties
            if result.get("type") == Some(&json!("object")) {
                if let (Some(Value::Object(ref props)), Some(Value::Array(ref required))) =
                    (result.get("properties"), result.get("required"))
                {
                    let filtered: Vec<Value> = required
                        .iter()
                        .filter(|r| {
                            if let Value::String(field) = r {
                                props.contains_key(field)
                            } else {
                                false
                            }
                        })
                        .cloned()
                        .collect();
                    result.insert("required".to_string(), Value::Array(filtered));
                }
            }

            // Handle array items
            if result.get("type") == Some(&json!("array")) {
                if !result.contains_key("items") || result.get("items") == Some(&Value::Null) {
                    result.insert("items".to_string(), json!({}));
                }
                // Ensure items has at least a type if it's an empty object
                if let Some(Value::Object(ref mut items)) = result.get_mut("items") {
                    if !items.contains_key("type") {
                        items.insert("type".to_string(), Value::String("string".to_string()));
                    }
                }
            }

            // Remove properties/required from non-object types
            if let Some(Value::String(ref t)) = result.get("type") {
                if t != "object" {
                    result.remove("properties");
                    result.remove("required");
                }
            }

            Value::Object(result)
        }
    }
}

// ---------------------------------------------------------------------------
// provider_options_map (matches TS providerOptions())
// ---------------------------------------------------------------------------

/// Convert provider options to the format expected by the SDK.
/// For gateway, splits options across gateway and upstream provider namespaces.
/// For other providers, wraps under the SDK key.
pub fn provider_options_map(
    model: &models::ModelInfo,
    opts: HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    if opts.is_empty() {
        return opts;
    }

    let npm = model
        .provider
        .as_ref()
        .and_then(|p| p.npm.as_deref())
        .unwrap_or("");
    let api_id = model
        .provider
        .as_ref()
        .and_then(|p| p.api.as_deref())
        .unwrap_or("");
    let provider_id = model.id.to_lowercase();

    if npm == "@ai-sdk/gateway" {
        // Gateway providerOptions are split across two namespaces:
        // - `gateway`: gateway-native routing/caching controls
        // - `<upstream slug>`: provider-specific model options
        let i = api_id.find('/');
        let raw_slug = if let Some(pos) = i {
            if pos > 0 {
                Some(&api_id[..pos])
            } else {
                None
            }
        } else {
            None
        };
        let slug = raw_slug.map(|s| slug_override(s).unwrap_or(s));

        let gateway = opts.get("gateway").cloned();
        let rest: HashMap<String, serde_json::Value> =
            opts.into_iter().filter(|(k, _)| k != "gateway").collect();
        let has_rest = !rest.is_empty();

        let mut result: HashMap<String, serde_json::Value> = HashMap::new();
        if let Some(gw) = gateway.clone() {
            result.insert("gateway".to_string(), gw);
        }

        if has_rest {
            if let Some(slug) = slug {
                result.insert(
                    slug.to_string(),
                    serde_json::to_value(&rest).unwrap_or_default(),
                );
            } else if let Some(ref gw) = gateway {
                if gw.is_object() {
                    let mut merged = gw.clone();
                    if let Some(obj) = merged.as_object_mut() {
                        for (k, v) in &rest {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                    result.insert("gateway".to_string(), merged);
                } else {
                    result.insert(
                        "gateway".to_string(),
                        serde_json::to_value(&rest).unwrap_or_default(),
                    );
                }
            } else {
                result.insert(
                    "gateway".to_string(),
                    serde_json::to_value(&rest).unwrap_or_default(),
                );
            }
        }

        return result;
    }

    let key = sdk_key(npm)
        .map(|s| s.to_string())
        .unwrap_or_else(|| provider_id.clone());
    let mut result = HashMap::new();
    result.insert(key, serde_json::json!(opts));
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, Role};

    #[test]
    fn test_provider_type_detection() {
        assert!(matches!(
            ProviderType::from_provider_id("anthropic"),
            ProviderType::Anthropic
        ));
        assert!(matches!(
            ProviderType::from_provider_id("openrouter"),
            ProviderType::OpenRouter
        ));
        assert!(matches!(
            ProviderType::from_provider_id("bedrock"),
            ProviderType::Bedrock
        ));
        assert!(matches!(
            ProviderType::from_provider_id("openai"),
            ProviderType::OpenAI
        ));
        assert!(matches!(
            ProviderType::from_provider_id("unknown"),
            ProviderType::Other
        ));
    }

    #[test]
    fn test_caching_support() {
        assert!(ProviderType::Anthropic.supports_caching());
        assert!(ProviderType::OpenRouter.supports_caching());
        assert!(!ProviderType::Other.supports_caching());
    }

    #[test]
    fn test_interleaved_thinking_support() {
        assert!(ProviderType::Anthropic.supports_interleaved_thinking());
        assert!(ProviderType::OpenRouter.supports_interleaved_thinking());
        assert!(!ProviderType::OpenAI.supports_interleaved_thinking());
    }

    #[test]
    fn test_apply_caching_anthropic() {
        let mut messages = vec![
            Message::system("System prompt"),
            Message::user("Hello"),
            Message::assistant("Hi there"),
        ];

        apply_caching(&mut messages, ProviderType::Anthropic);

        // Anthropic uses message-level providerOptions
        assert!(messages[0].provider_options.is_some());
        assert!(messages[2].provider_options.is_some());
    }

    #[test]
    fn test_extract_reasoning() {
        let content = "Hello <thinking>let me think</thinking> World";
        let (reasoning, rest) = extract_reasoning_from_response(content);

        assert_eq!(reasoning, Some("let me think".to_string()));
        assert!(rest.contains("Hello"));
        assert!(rest.contains("World"));
    }

    fn default_model_info() -> models::ModelInfo {
        models::ModelInfo {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            family: None,
            release_date: None,
            attachment: false,
            reasoning: false,
            temperature: false,
            tool_call: false,
            interleaved: None,
            cost: None,
            limit: models::ModelLimit {
                context: 128000,
                input: None,
                output: 8192,
            },
            modalities: None,
            experimental: None,
            status: None,
            options: HashMap::new(),
            headers: None,
            provider: None,
            variants: None,
        }
    }

    #[test]
    fn test_max_output_tokens() {
        let model = models::ModelInfo {
            id: "test".to_string(),
            name: "Test".to_string(),
            limit: models::ModelLimit {
                context: 200000,
                input: None,
                output: 64000,
            },
            ..default_model_info()
        };
        assert_eq!(max_output_tokens(&model), OUTPUT_TOKEN_MAX);
    }

    #[test]
    fn test_max_output_tokens_small_model() {
        let model = models::ModelInfo {
            limit: models::ModelLimit {
                context: 128000,
                input: None,
                output: 4096,
            },
            ..default_model_info()
        };
        assert_eq!(max_output_tokens(&model), 4096);
    }

    #[test]
    fn test_variants_non_reasoning() {
        let model = models::ModelInfo {
            reasoning: false,
            ..default_model_info()
        };
        assert!(variants(&model).is_empty());
    }

    #[test]
    fn test_sdk_key_mapping() {
        assert_eq!(sdk_key("@ai-sdk/anthropic"), Some("anthropic"));
        assert_eq!(sdk_key("@ai-sdk/openai"), Some("openai"));
        assert_eq!(sdk_key("@ai-sdk/google"), Some("google"));
        assert_eq!(sdk_key("@ai-sdk/google-vertex"), Some("google"));
        assert_eq!(sdk_key("@ai-sdk/amazon-bedrock"), Some("bedrock"));
        assert_eq!(sdk_key("@openrouter/ai-sdk-provider"), Some("openrouter"));
        assert_eq!(sdk_key("unknown-package"), None);
    }

    #[test]
    fn test_normalize_interleaved_thinking_strips_non_last() {
        let mut messages = vec![
            Message {
                role: Role::Assistant,
                content: Content::Parts(vec![
                    ContentPart {
                        content_type: "thinking".to_string(),
                        text: Some("thinking...".to_string()),
                        ..Default::default()
                    },
                    ContentPart {
                        content_type: "text".to_string(),
                        text: Some("response 1".to_string()),
                        ..Default::default()
                    },
                ]),
                cache_control: None,
                provider_options: None,
            },
            Message::user("follow up"),
            Message {
                role: Role::Assistant,
                content: Content::Parts(vec![
                    ContentPart {
                        content_type: "thinking".to_string(),
                        text: Some("more thinking...".to_string()),
                        ..Default::default()
                    },
                    ContentPart {
                        content_type: "text".to_string(),
                        text: Some("response 2".to_string()),
                        ..Default::default()
                    },
                ]),
                cache_control: None,
                provider_options: None,
            },
        ];

        normalize_interleaved_thinking(&mut messages, &ProviderType::OpenAI, false);

        // First assistant: thinking stripped, text kept
        if let Content::Parts(ref parts) = messages[0].content {
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].content_type, "text");
        } else {
            panic!("Expected Parts content");
        }

        // Last assistant: thinking kept
        if let Content::Parts(ref parts) = messages[2].content {
            assert_eq!(parts.len(), 2);
        } else {
            panic!("Expected Parts content");
        }
    }

    #[test]
    fn test_normalize_interleaved_thinking_supports_interleaved() {
        let mut messages = vec![Message {
            role: Role::Assistant,
            content: Content::Parts(vec![ContentPart {
                content_type: "thinking".to_string(),
                text: Some("thinking...".to_string()),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        }];

        normalize_interleaved_thinking(&mut messages, &ProviderType::Anthropic, true);

        // Nothing stripped when interleaved is supported
        if let Content::Parts(ref parts) = messages[0].content {
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].content_type, "thinking");
        } else {
            panic!("Expected Parts content");
        }
    }

    #[test]
    fn test_apply_caching_per_part_anthropic() {
        let mut messages = vec![
            Message::system("system prompt"),
            Message::user("hello"),
            Message {
                role: Role::Assistant,
                content: Content::Text("response".to_string()),
                cache_control: None,
                provider_options: None,
            },
            Message::user("follow up"),
        ];

        apply_caching_per_part(&mut messages, &ProviderType::Anthropic);

        // System message should have cache control
        assert!(messages[0].cache_control.is_some());

        // Last user message should have cache control
        assert!(messages[3].cache_control.is_some());

        // First user message should NOT have cache control
        assert!(messages[1].cache_control.is_none());
    }

    #[test]
    fn test_output_token_max_is_32000() {
        assert_eq!(OUTPUT_TOKEN_MAX, 32_000);
    }

    #[test]
    fn test_schema_gemini_sanitization() {
        use serde_json::json;
        let model = models::ModelInfo {
            id: "google".to_string(),
            provider: Some(models::ModelProvider {
                npm: Some("@ai-sdk/google".to_string()),
                api: Some("gemini-2.0-flash".to_string()),
            }),
            ..default_model_info()
        };

        let input = json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "integer",
                    "enum": [1, 2, 3]
                }
            },
            "required": ["status", "nonexistent"]
        });

        let result = schema(&model, input);

        // Integer enum should be converted to string enum
        let status = &result["properties"]["status"];
        assert_eq!(status["type"], "string");
        assert_eq!(status["enum"], serde_json::json!(["1", "2", "3"]));

        // Required should be filtered to only existing properties
        assert_eq!(result["required"], serde_json::json!(["status"]));
    }

    #[test]
    fn test_schema_gemini_array_items() {
        use serde_json::json;
        let model = models::ModelInfo {
            id: "google".to_string(),
            provider: Some(models::ModelProvider {
                npm: Some("@ai-sdk/google".to_string()),
                api: Some("gemini-2.0-flash".to_string()),
            }),
            ..default_model_info()
        };

        let input = json!({
            "type": "array"
        });

        let result = schema(&model, input);
        // Empty array should get items with type string
        assert_eq!(result["items"]["type"], "string");
    }

    #[test]
    fn test_variants_sap_anthropic() {
        let model = models::ModelInfo {
            id: "sap-model".to_string(),
            reasoning: true,
            provider: Some(models::ModelProvider {
                npm: Some("@mymediset/sap-ai-provider".to_string()),
                api: Some("anthropic/claude-3.5-sonnet".to_string()),
            }),
            ..default_model_info()
        };

        let v = variants(&model);
        assert!(v.contains_key("high"));
        assert!(v.contains_key("max"));
        let high = &v["high"];
        assert!(high.contains_key("thinking"));
    }

    #[test]
    fn test_variants_sap_non_anthropic() {
        let model = models::ModelInfo {
            id: "sap-model".to_string(),
            reasoning: true,
            provider: Some(models::ModelProvider {
                npm: Some("@jerome-benoit/sap-ai-provider-v2".to_string()),
                api: Some("openai/gpt-4o".to_string()),
            }),
            ..default_model_info()
        };

        let v = variants(&model);
        assert!(v.contains_key("low"));
        assert!(v.contains_key("medium"));
        assert!(v.contains_key("high"));
        assert!(!v.contains_key("max"));
    }

    #[test]
    fn test_variants_venice() {
        let model = models::ModelInfo {
            id: "venice-model".to_string(),
            reasoning: true,
            provider: Some(models::ModelProvider {
                npm: Some("venice-ai-sdk-provider".to_string()),
                api: Some("some-model".to_string()),
            }),
            ..default_model_info()
        };

        let v = variants(&model);
        assert!(v.contains_key("low"));
        assert!(v.contains_key("medium"));
        assert!(v.contains_key("high"));
    }

    #[test]
    fn test_provider_options_map_gateway() {
        use serde_json::json;
        let model = models::ModelInfo {
            id: "gateway-model".to_string(),
            provider: Some(models::ModelProvider {
                npm: Some("@ai-sdk/gateway".to_string()),
                api: Some("anthropic/claude-3.5-sonnet".to_string()),
            }),
            ..default_model_info()
        };

        let mut opts = HashMap::new();
        opts.insert("gateway".to_string(), json!({"caching": "auto"}));
        opts.insert("thinking".to_string(), json!({"type": "enabled"}));

        let result = provider_options_map(&model, opts);
        assert!(result.contains_key("gateway"));
        assert!(result.contains_key("anthropic"));
    }

    #[test]
    fn test_provider_options_map_gateway_amazon() {
        use serde_json::json;
        let model = models::ModelInfo {
            id: "gateway-model".to_string(),
            provider: Some(models::ModelProvider {
                npm: Some("@ai-sdk/gateway".to_string()),
                api: Some("amazon/nova-2-lite".to_string()),
            }),
            ..default_model_info()
        };

        let mut opts = HashMap::new();
        opts.insert("reasoningEffort".to_string(), json!("high"));

        let result = provider_options_map(&model, opts);
        // amazon -> bedrock via SLUG_OVERRIDES
        assert!(result.contains_key("bedrock"));
    }
}
