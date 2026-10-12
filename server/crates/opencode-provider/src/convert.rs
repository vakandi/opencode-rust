//! Message conversion functions for the OpenAI Responses API and
//! OpenAI-compatible chat completions.
//!
//! Mirrors the TS files:
//! - `convert-to-openai-responses-input.ts`
//! - `convert-to-openai-compatible-chat-messages.ts`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::responses::{CallWarning, ResponsesInput, SystemMessageMode};

// ---------------------------------------------------------------------------
// Prompt Types (language-model-agnostic)
// ---------------------------------------------------------------------------

/// A prompt message with role and content parts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: PromptRole,
    pub content: PromptContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PromptRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PromptContent {
    Text(String),
    Parts(Vec<PromptPart>),
}

// PLACEHOLDER_CONVERT_1
