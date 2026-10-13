use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    ChatRequest, ChatResponse, Choice, Content, Message, ModelInfo, Provider, ProviderError, Role,
    StreamEvent, StreamResult, Usage,
};

const VERCEL_API_URL: &str = "https://api.vercel.ai/v1/chat/completions";

#[derive(Debug, Clone)]
pub struct VercelConfig {
    pub api_key: String,
    pub base_url: Option<String>,
}

#[derive(Debug)]
pub struct VercelProvider {
    client: Client,
    config: VercelConfig,
    models: Vec<ModelInfo>,
}

impl VercelProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_config(VercelConfig {
            api_key: api_key.into(),
            base_url: None,
        })
    }

    pub fn with_config(config: VercelConfig) -> Self {
        let models = vec![ModelInfo {
            id: "v0-1.0-md".to_string(),
            name: "v0 1.0 (Markdown)".to_string(),
            provider: "vercel".to_string(),
            context_window: 128000,
            max_output_tokens: 8192,
            supports_vision: true,
            supports_tools: false,
            cost_per_million_input: 0.0,
            cost_per_million_output: 0.0,
        }];

        Self {
            client: Client::new(),
            config,
            models,
        }
    }

    fn convert_request(&self, request: ChatRequest) -> VercelRequest {
        let messages: Vec<VercelMessage> = request
            .messages
            .into_iter()
            .map(|msg| VercelMessage {
                role: match msg.role {
                    Role::System => "system".to_string(),
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                    Role::Tool => "user".to_string(),
                },
                content: match msg.content {
                    Content::Text(t) => VercelContent::Text(t),
                    Content::Parts(parts) => {
                        let contents: Vec<VercelContentPart> = parts
                            .into_iter()
                            .filter_map(|p| {
                                if let Some(text) = p.text {
                                    Some(VercelContentPart {
                                        content_type: "text".to_string(),
                                        text: Some(text),
                                        image_url: p
                                            .image_url
                                            .map(|iu| VercelImageUrl { url: iu.url }),
                                    })
                                } else if p.image_url.is_some() {
                                    Some(VercelContentPart {
                                        content_type: "image_url".to_string(),
                                        text: None,
                                        image_url: p
                                            .image_url
                                            .map(|iu| VercelImageUrl { url: iu.url }),
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect();
                        VercelContent::Parts(contents)
                    }
                },
            })
            .collect();

        VercelRequest {
            model: request.model,
            messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            stream: true,
        }
    }
}

#[async_trait]
impl Provider for VercelProvider {
    fn id(&self) -> &str {
        "vercel"
    }

    fn name(&self) -> &str {
        "Vercel AI"
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
        let url = self.config.base_url.as_deref().unwrap_or(VERCEL_API_URL);
        let vercel_request = self.convert_request(request);

        let response = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&vercel_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::api_error_with_status(
                format!("{}: {}", status, body),
                status.as_u16(),
            ));
        }

        let vercel_response: VercelResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_vercel_response(vercel_response))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let url = self.config.base_url.as_deref().unwrap_or(VERCEL_API_URL);
        let mut vercel_request = self.convert_request(request);
        vercel_request.stream = true;

        let response = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Accept", "text/event-stream")
            .json(&vercel_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::api_error_with_status(
                format!("{}: {}", status, body),
                status.as_u16(),
            ));
        }

        let stream = response
            .bytes_stream()
            .map(move |chunk_result| match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    for line in text.lines() {
                        if line.starts_with("data: ") {
                            let data = &line[6..];
                            if data == "[DONE]" {
                                return Ok(StreamEvent::Done);
                            }
                            if let Some(event) = parse_vercel_sse(data) {
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
struct VercelRequest {
    model: String,
    messages: Vec<VercelMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct VercelMessage {
    role: String,
    content: VercelContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum VercelContent {
    Text(String),
    Parts(Vec<VercelContentPart>),
}

#[derive(Debug, Serialize)]
struct VercelContentPart {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<VercelImageUrl>,
}

#[derive(Debug, Serialize)]
struct VercelImageUrl {
    url: String,
}

#[derive(Debug, Deserialize)]
struct VercelResponse {
    id: String,
    model: String,
    choices: Vec<VercelChoice>,
    usage: Option<VercelUsage>,
}

#[derive(Debug, Deserialize)]
struct VercelChoice {
    _index: u32,
    message: VercelResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VercelResponseMessage {
    _role: String,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VercelUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct VercelStreamResponse {
    choices: Vec<VercelStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct VercelStreamChoice {
    delta: VercelDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VercelDelta {
    content: Option<String>,
}

fn convert_vercel_response(response: VercelResponse) -> ChatResponse {
    let content = response
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();

    let usage = response.usage.map(|u| Usage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
    });

    ChatResponse {
        id: response.id,
        model: response.model,
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content: Content::Text(content),
                cache_control: None,
                provider_options: None,
            },
            finish_reason: response
                .choices
                .first()
                .and_then(|c| c.finish_reason.clone()),
        }],
        usage,
    }
}

fn parse_vercel_sse(data: &str) -> Option<StreamEvent> {
    if data.is_empty() {
        return None;
    }

    let response: VercelStreamResponse = serde_json::from_str(data).ok()?;

    let choice = response.choices.first()?;

    if let Some(content) = &choice.delta.content {
        if !content.is_empty() {
            return Some(StreamEvent::TextDelta(content.clone()));
        }
    }

    if let Some(reason) = &choice.finish_reason {
        if reason == "tool_calls" {
            return Some(StreamEvent::Done);
        }
    }

    None
}
