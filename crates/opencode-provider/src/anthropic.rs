use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    ChatRequest, ChatResponse, Choice, Message, ModelInfo,
    Provider, ProviderError, StreamEvent, StreamResult, Usage,
};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub base_url: Option<String>,
}

#[derive(Debug)]
pub struct AnthropicProvider {
    client: Client,
    config: AnthropicConfig,
    models: Vec<ModelInfo>,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_config(AnthropicConfig {
            api_key: api_key.into(),
            base_url: None,
        })
    }

    pub fn with_config(config: AnthropicConfig) -> Self {
        let models = vec![
            ModelInfo {
                id: "claude-opus-4-6".to_string(),
                name: "Claude Opus 4.6".to_string(),
                provider: "anthropic".to_string(),
                context_window: 200_000,
                max_output_tokens: 128_000,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 5.0,
                cost_per_million_output: 25.0,
            },
            ModelInfo {
                id: "claude-sonnet-4-20250514".to_string(),
                name: "Claude Sonnet 4".to_string(),
                provider: "anthropic".to_string(),
                context_window: 200000,
                max_output_tokens: 16000,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 3.0,
                cost_per_million_output: 15.0,
            },
            ModelInfo {
                id: "claude-3-5-sonnet-20241022".to_string(),
                name: "Claude 3.5 Sonnet".to_string(),
                provider: "anthropic".to_string(),
                context_window: 200000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 3.0,
                cost_per_million_output: 15.0,
            },
            ModelInfo {
                id: "claude-3-5-haiku-20241022".to_string(),
                name: "Claude 3.5 Haiku".to_string(),
                provider: "anthropic".to_string(),
                context_window: 200000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 1.0,
                cost_per_million_output: 5.0,
            },
            ModelInfo {
                id: "claude-3-opus-20240229".to_string(),
                name: "Claude 3 Opus".to_string(),
                provider: "anthropic".to_string(),
                context_window: 200000,
                max_output_tokens: 4096,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 15.0,
                cost_per_million_output: 75.0,
            },
        ];

        Self {
            client: Client::new(),
            config,
            models,
        }
    }

    fn convert_request(&self, request: ChatRequest) -> AnthropicRequest {
        let max_tokens = request.max_tokens.unwrap_or(4096);
        let mut messages = Vec::new();
        let mut system = None;

        for msg in request.messages {
            match msg.role {
                crate::Role::System => {
                    if let crate::Content::Text(text) = msg.content {
                        system = Some(text);
                    }
                }
                _ => {
                    messages.push(AnthropicMessage {
                        role: match msg.role {
                            crate::Role::User => "user".to_string(),
                            crate::Role::Assistant => "assistant".to_string(),
                            _ => "user".to_string(),
                        },
                        content: match msg.content {
                            crate::Content::Text(text) => vec![AnthropicContent::Text { text }],
                            crate::Content::Parts(parts) => parts
                                .into_iter()
                                .filter_map(|p| {
                                    if let Some(text) = p.text {
                                        Some(AnthropicContent::Text { text })
                                    } else {
                                        None
                                    }
                                })
                                .collect(),
                        },
                    });
                }
            }
        }

        AnthropicRequest {
            model: request.model,
            max_tokens,
            messages,
            system,
            stream: request.stream,
            thinking: anthropic_thinking_config(request.variant.as_deref(), max_tokens),
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn name(&self) -> &str {
        "Anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.models.clone()
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.id == id)
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let anthropic_request = self.convert_request(request);

        let url = self.config.base_url.as_deref().unwrap_or(ANTHROPIC_API_URL);

        let response = client
            .post(url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14")
            .header("content-type", "application/json")
            .json(&anthropic_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let anthropic_response: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_response(anthropic_response))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let mut anthropic_request = self.convert_request(request);
        anthropic_request.stream = Some(true);

        let url = self.config.base_url.as_deref().unwrap_or(ANTHROPIC_API_URL);

        let response = client
            .post(url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14")
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&anthropic_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let stream = response
            .bytes_stream()
            .map(move |chunk_result| match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    for line in text.lines() {
                        if line.starts_with("data: ") {
                            let data = &line[6..];
                            if let Some(event) = crate::stream::parse_anthropic_sse(data) {
                                return Ok(event);
                            }
                        }
                    }
                    Ok(StreamEvent::TextDelta(String::new()))
                }
                Err(e) => Err(ProviderError::StreamError(e.to_string())),
            });

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u64,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContent>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum AnthropicContent {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum AnthropicThinking {
    #[serde(rename = "enabled")]
    Enabled {
        #[serde(rename = "budget_tokens")]
        budget_tokens: u64,
    },
}

fn anthropic_thinking_config(variant: Option<&str>, max_tokens: u64) -> Option<AnthropicThinking> {
    let variant = variant?.trim().to_ascii_lowercase();
    let target = match variant.as_str() {
        "low" => 4_000,
        "medium" => 8_000,
        "high" => 16_000,
        "max" | "xhigh" => 31_999,
        _ => return None,
    };

    let ceiling = max_tokens.saturating_sub(1);
    let budget_tokens = target.min(ceiling);
    if budget_tokens == 0 {
        return None;
    }
    Some(AnthropicThinking::Enabled { budget_tokens })
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    id: String,
    model: String,
    content: Vec<AnthropicResponseContent>,
    usage: AnthropicResponseUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponseContent {
    #[serde(rename = "type")]
    _content_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponseUsage {
    input_tokens: u64,
    output_tokens: u64,
}

fn convert_response(response: AnthropicResponse) -> ChatResponse {
    let content = response
        .content
        .iter()
        .filter_map(|c| c.text.clone())
        .collect::<Vec<_>>()
        .join("");

    ChatResponse {
        id: response.id,
        model: response.model,
        choices: vec![Choice {
            index: 0,
            message: Message::assistant(&content),
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(Usage {
            prompt_tokens: response.usage.input_tokens,
            completion_tokens: response.usage.output_tokens,
            total_tokens: response.usage.input_tokens + response.usage.output_tokens,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        }),
    }
}
