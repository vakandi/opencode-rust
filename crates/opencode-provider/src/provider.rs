use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::{ChatRequest, ChatResponse, StreamResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub supports_vision: bool,
    pub supports_tools: bool,
    pub cost_per_million_input: f64,
    pub cost_per_million_output: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub models: HashMap<String, ModelInfo>,
    pub source: String,
    pub options: HashMap<String, serde_json::Value>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;

    fn models(&self) -> Vec<ModelInfo>;
    fn get_model(&self, id: &str) -> Option<&ModelInfo>;

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError>;
    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("API error (status {status_code}): {message}")]
    ApiErrorWithStatus { message: String, status_code: u16 },

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Authentication error: {0}")]
    AuthError(String),

    #[error("Rate limit exceeded")]
    RateLimit,

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Stream error: {0}")]
    StreamError(String),

    #[error("Timeout")]
    Timeout,

    #[error("Provider not found: {0}")]
    ProviderNotFound(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Context overflow: {0}")]
    ContextOverflow(String),
}

impl crate::retry::IsRetryable for ProviderError {
    fn is_retryable(&self) -> Option<String> {
        match self {
            ProviderError::RateLimit => Some("Rate limited".to_string()),
            ProviderError::Timeout => Some("Request timed out".to_string()),
            ProviderError::NetworkError(msg) => Some(format!("Network error: {msg}")),
            ProviderError::ApiErrorWithStatus {
                status_code,
                message,
            } => {
                if matches!(status_code, 429 | 500 | 502 | 503 | 504) {
                    Some(format!("API error {status_code}: {message}"))
                } else {
                    None
                }
            }
            ProviderError::ApiError(_)
            | ProviderError::AuthError(_)
            | ProviderError::ModelNotFound(_)
            | ProviderError::InvalidRequest(_)
            | ProviderError::StreamError(_)
            | ProviderError::ProviderNotFound(_)
            | ProviderError::ConfigError(_)
            | ProviderError::ContextOverflow(_) => None,
        }
    }
}

static OVERFLOW_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)prompt is too long").unwrap(),
        Regex::new(r"(?i)input is too long for requested model").unwrap(),
        Regex::new(r"(?i)exceeds the context window").unwrap(),
        Regex::new(r"(?i)input token count.*exceeds the maximum").unwrap(),
        Regex::new(r"(?i)maximum prompt length is \d+").unwrap(),
        Regex::new(r"(?i)reduce the length of the messages").unwrap(),
        Regex::new(r"(?i)maximum context length is \d+ tokens").unwrap(),
        Regex::new(r"(?i)exceeds the limit of \d+").unwrap(),
        Regex::new(r"(?i)exceeds the available context size").unwrap(),
        Regex::new(r"(?i)greater than the context length").unwrap(),
        Regex::new(r"(?i)context window exceeds limit").unwrap(),
        Regex::new(r"(?i)exceeded model token limit").unwrap(),
        Regex::new(r"(?i)context[_ ]length[_ ]exceeded").unwrap(),
    ]
});

static NO_BODY_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^4(00|13)\s*(status code)?\s*\(no body\)").unwrap());

impl ProviderError {
    pub fn api_error_with_status(message: impl Into<String>, status_code: u16) -> Self {
        ProviderError::ApiErrorWithStatus {
            message: message.into(),
            status_code,
        }
    }

    pub fn context_overflow(message: impl Into<String>) -> Self {
        ProviderError::ContextOverflow(message.into())
    }

    pub fn is_overflow(message: &str) -> bool {
        if OVERFLOW_PATTERNS.iter().any(|p| p.is_match(message)) {
            return true;
        }
        NO_BODY_PATTERN.is_match(message)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ParsedAPICallError {
    #[serde(rename = "context_overflow")]
    ContextOverflow {
        message: String,
        response_body: Option<String>,
    },
    #[serde(rename = "api_error")]
    ApiError {
        message: String,
        status_code: Option<u16>,
        is_retryable: bool,
        response_headers: Option<HashMap<String, String>>,
        response_body: Option<String>,
        metadata: Option<HashMap<String, String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ParsedStreamError {
    #[serde(rename = "context_overflow")]
    ContextOverflow {
        message: String,
        response_body: String,
    },
    #[serde(rename = "api_error")]
    ApiError {
        message: String,
        is_retryable: bool,
        response_body: String,
    },
}

pub fn parse_api_call_error(provider_id: &str, error: &ProviderError) -> ParsedAPICallError {
    let message = format_error_message(provider_id, error);

    if ProviderError::is_overflow(&message) {
        return ParsedAPICallError::ContextOverflow {
            message,
            response_body: None,
        };
    }

    let (status_code, is_retryable) = match error {
        ProviderError::ApiErrorWithStatus { status_code, .. } => {
            let retryable = if provider_id.starts_with("openai") {
                is_openai_error_retryable(*status_code)
            } else {
                matches!(status_code, 429 | 500 | 502 | 503 | 504)
            };
            (Some(*status_code), retryable)
        }
        ProviderError::RateLimit => (Some(429), true),
        ProviderError::Timeout => (None, true),
        ProviderError::NetworkError(_) => (None, true),
        _ => (None, false),
    };

    ParsedAPICallError::ApiError {
        message,
        status_code,
        is_retryable,
        response_headers: None,
        response_body: None,
        metadata: None,
    }
}

fn format_error_message(provider_id: &str, error: &ProviderError) -> String {
    // GitHub Copilot 403 special case
    if provider_id.contains("github-copilot") {
        if let ProviderError::ApiErrorWithStatus {
            status_code: 403, ..
        } = error
        {
            return "Please reauthenticate with the copilot provider to ensure your credentials work properly with OpenCode.".to_string();
        }
    }
    error.to_string()
}

fn is_openai_error_retryable(status: u16) -> bool {
    // OpenAI sometimes returns 404 for models that are actually available
    status == 404 || matches!(status, 429 | 500 | 502 | 503 | 504)
}

pub fn parse_stream_error(data: &str) -> Option<ParsedStreamError> {
    let body: serde_json::Value = serde_json::from_str(data).ok()?;

    if body.get("type")?.as_str()? != "error" {
        return None;
    }

    let error = body.get("error")?;
    let code = error.get("code")?.as_str()?;
    let response_body = serde_json::to_string(&body).unwrap_or_default();

    match code {
        "context_length_exceeded" => Some(ParsedStreamError::ContextOverflow {
            message: "Input exceeds context window of this model".to_string(),
            response_body,
        }),
        "insufficient_quota" => Some(ParsedStreamError::ApiError {
            message: "Quota exceeded. Check your plan and billing details.".to_string(),
            is_retryable: false,
            response_body,
        }),
        "usage_not_included" => Some(ParsedStreamError::ApiError {
            message: "To use Codex with your ChatGPT plan, upgrade to Plus: https://chatgpt.com/explore/plus.".to_string(),
            is_retryable: false,
            response_body,
        }),
        "invalid_prompt" => {
            let msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Invalid prompt.")
                .to_string();
            Some(ParsedStreamError::ApiError {
                message: msg,
                is_retryable: false,
                response_body,
            })
        }
        _ => None,
    }
}

pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn Provider>>,
    provider_info: HashMap<String, ProviderInfo>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            provider_info: HashMap::new(),
        }
    }

    pub fn register<P: Provider + 'static>(&mut self, provider: P) {
        let id = provider.id().to_string();
        let models = provider.models();
        let info = ProviderInfo {
            id: id.clone(),
            name: provider.name().to_string(),
            models: models.into_iter().map(|m| (m.id.clone(), m)).collect(),
            source: "bundled".to_string(),
            options: HashMap::new(),
        };
        self.provider_info.insert(id.clone(), info);
        self.providers.insert(id, Arc::new(provider));
    }

    pub fn register_arc(&mut self, provider: Arc<dyn Provider>) {
        let id = provider.id().to_string();
        let models = provider.models();
        let info = ProviderInfo {
            id: id.clone(),
            name: provider.name().to_string(),
            models: models.into_iter().map(|m| (m.id.clone(), m)).collect(),
            source: "bundled".to_string(),
            options: HashMap::new(),
        };
        self.provider_info.insert(id.clone(), info);
        self.providers.insert(id, provider);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(id).cloned()
    }

    pub fn get_info(&self, id: &str) -> Option<&ProviderInfo> {
        self.provider_info.get(id)
    }

    pub fn list(&self) -> Vec<Arc<dyn Provider>> {
        self.providers.values().cloned().collect()
    }

    pub fn list_providers(&self) -> Vec<&ProviderInfo> {
        self.provider_info.values().collect()
    }

    pub fn list_models(&self) -> Vec<ModelInfo> {
        self.providers.values().flat_map(|p| p.models()).collect()
    }

    pub fn find_model(&self, model_id: &str) -> Option<(String, ModelInfo)> {
        for (provider_id, provider) in &self.providers {
            if let Some(model) = provider.get_model(model_id) {
                return Some((provider_id.clone(), model.clone()));
            }
        }
        None
    }

    pub fn merge_config(&mut self, provider_id: &str, options: HashMap<String, serde_json::Value>) {
        if let Some(info) = self.provider_info.get_mut(provider_id) {
            info.options.extend(options);
        }
    }

    pub fn get_provider(&self, provider_id: &str) -> Result<Arc<dyn Provider>, ProviderError> {
        self.providers
            .get(provider_id)
            .cloned()
            .ok_or_else(|| ProviderError::ProviderNotFound(provider_id.to_string()))
    }

    pub fn get_language_model(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<(Arc<dyn Provider>, ModelInfo), ProviderError> {
        let provider = self.get_provider(provider_id)?;
        let model = provider
            .get_model(model_id)
            .cloned()
            .ok_or_else(|| ProviderError::ModelNotFound(model_id.to_string()))?;
        Ok((provider, model))
    }

    pub fn parse_model_string(&self, model_string: &str) -> Option<(String, String)> {
        if let Some(pos) = model_string.find('/') {
            let provider_id = &model_string[..pos];
            let model_id = &model_string[pos + 1..];
            Some((provider_id.to_string(), model_id.to_string()))
        } else {
            self.find_model(model_string)
                .map(|(provider_id, _)| (provider_id, model_string.to_string()))
        }
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_default_registry() -> ProviderRegistry {
    ProviderRegistry::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overflow_detection_anthropic() {
        assert!(ProviderError::is_overflow("prompt is too long"));
        assert!(ProviderError::is_overflow(
            "The prompt is too long for this model"
        ));
    }

    #[test]
    fn test_overflow_detection_openai() {
        assert!(ProviderError::is_overflow(
            "This model's maximum context length is 128000 tokens"
        ));
    }

    #[test]
    fn test_overflow_detection_bedrock() {
        assert!(ProviderError::is_overflow(
            "input is too long for requested model"
        ));
    }

    #[test]
    fn test_overflow_detection_no_body() {
        assert!(ProviderError::is_overflow("400 (no body)"));
        assert!(ProviderError::is_overflow("413 (no body)"));
        assert!(ProviderError::is_overflow("400 status code (no body)"));
    }

    #[test]
    fn test_no_false_positive_overflow() {
        assert!(!ProviderError::is_overflow("rate limit exceeded"));
        assert!(!ProviderError::is_overflow("authentication failed"));
    }

    #[test]
    fn test_parse_api_call_error_overflow() {
        let error = ProviderError::ApiError("prompt is too long".to_string());
        let parsed = parse_api_call_error("anthropic", &error);
        assert!(matches!(parsed, ParsedAPICallError::ContextOverflow { .. }));
    }

    #[test]
    fn test_parse_api_call_error_github_copilot_403() {
        let error = ProviderError::api_error_with_status("Forbidden", 403);
        let parsed = parse_api_call_error("github-copilot", &error);
        if let ParsedAPICallError::ApiError { message, .. } = parsed {
            assert!(message.contains("reauthenticate"));
        }
    }

    #[test]
    fn test_openai_retryable_404() {
        assert!(is_openai_error_retryable(404));
        assert!(is_openai_error_retryable(429));
        assert!(!is_openai_error_retryable(401));
    }

    #[test]
    fn test_parse_stream_error_context_overflow() {
        let data =
            r#"{"type":"error","error":{"code":"context_length_exceeded","message":"too long"}}"#;
        let parsed = parse_stream_error(data).unwrap();
        assert!(matches!(parsed, ParsedStreamError::ContextOverflow { .. }));
    }

    #[test]
    fn test_parse_stream_error_quota() {
        let data =
            r#"{"type":"error","error":{"code":"insufficient_quota","message":"quota exceeded"}}"#;
        let parsed = parse_stream_error(data).unwrap();
        assert!(matches!(parsed, ParsedStreamError::ApiError { .. }));
    }

    #[test]
    fn test_parse_stream_error_non_error() {
        let data = r#"{"type":"message","content":"hello"}"#;
        assert!(parse_stream_error(data).is_none());
    }
}
