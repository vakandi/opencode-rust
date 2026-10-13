use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    ChatRequest, ChatResponse, Choice, Content, Message, ModelInfo, Provider, ProviderError, Role,
    StreamEvent, StreamResult, Usage,
};

const GOOGLE_API_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";

#[derive(Debug, Clone)]
pub struct GoogleConfig {
    pub api_key: String,
    pub base_url: Option<String>,
}

#[derive(Debug)]
pub struct GoogleProvider {
    client: Client,
    config: GoogleConfig,
    models: Vec<ModelInfo>,
}

impl GoogleProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_config(GoogleConfig {
            api_key: api_key.into(),
            base_url: None,
        })
    }

    pub fn with_config(config: GoogleConfig) -> Self {
        let models = vec![
            ModelInfo {
                id: "gemini-2.5-pro-preview-06-05".to_string(),
                name: "Gemini 2.5 Pro".to_string(),
                provider: "google".to_string(),
                context_window: 1000000,
                max_output_tokens: 65536,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 1.25,
                cost_per_million_output: 10.0,
            },
            ModelInfo {
                id: "gemini-2.0-flash".to_string(),
                name: "Gemini 2.0 Flash".to_string(),
                provider: "google".to_string(),
                context_window: 1000000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.1,
                cost_per_million_output: 0.4,
            },
            ModelInfo {
                id: "gemini-2.0-flash-lite".to_string(),
                name: "Gemini 2.0 Flash Lite".to_string(),
                provider: "google".to_string(),
                context_window: 1000000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.075,
                cost_per_million_output: 0.3,
            },
            ModelInfo {
                id: "gemini-1.5-pro".to_string(),
                name: "Gemini 1.5 Pro".to_string(),
                provider: "google".to_string(),
                context_window: 2000000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 1.25,
                cost_per_million_output: 5.0,
            },
            ModelInfo {
                id: "gemini-1.5-flash".to_string(),
                name: "Gemini 1.5 Flash".to_string(),
                provider: "google".to_string(),
                context_window: 1000000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.075,
                cost_per_million_output: 0.3,
            },
        ];

        Self {
            client: Client::new(),
            config,
            models,
        }
    }

    fn convert_request(&self, request: ChatRequest) -> GoogleRequest {
        let mut contents = Vec::new();
        let mut system_instruction = None;

        for msg in request.messages {
            match msg.role {
                Role::System => {
                    if let Content::Text(text) = msg.content {
                        system_instruction = Some(GoogleContent {
                            parts: vec![GooglePart::text(&text)],
                            role: "user".to_string(),
                        });
                    }
                }
                Role::User => {
                    let text_content = match &msg.content {
                        Content::Text(t) => t.clone(),
                        Content::Parts(parts) => parts
                            .iter()
                            .filter_map(|p| p.text.clone())
                            .collect::<Vec<_>>()
                            .join(" "),
                    };
                    contents.push(GoogleContent {
                        parts: vec![GooglePart::text(&text_content)],
                        role: "user".to_string(),
                    });
                }
                Role::Assistant => {
                    let text_content = match &msg.content {
                        Content::Text(t) => t.clone(),
                        Content::Parts(parts) => parts
                            .iter()
                            .filter_map(|p| p.text.clone())
                            .collect::<Vec<_>>()
                            .join(" "),
                    };
                    contents.push(GoogleContent {
                        parts: vec![GooglePart::text(&text_content)],
                        role: "model".to_string(),
                    });
                }
                Role::Tool => {}
            }
        }

        GoogleRequest {
            contents,
            system_instruction,
            generation_config: Some(GenerationConfig {
                max_output_tokens: request.max_tokens,
                temperature: request.temperature,
            }),
        }
    }
}

#[async_trait]
impl Provider for GoogleProvider {
    fn id(&self) -> &str {
        "google"
    }

    fn name(&self) -> &str {
        "Google AI"
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
        let base_url = self.config.base_url.as_deref().unwrap_or(GOOGLE_API_URL);
        let url = format!(
            "{}/{}:generateContent?key={}",
            base_url, request.model, self.config.api_key
        );

        let google_request = self.convert_request(request);

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&google_request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let google_response: GoogleResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_google_response(google_response))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let base_url = self.config.base_url.as_deref().unwrap_or(GOOGLE_API_URL);
        let url = format!(
            "{}/{}:streamGenerateContent?key={}&alt=sse",
            base_url, request.model, self.config.api_key
        );

        let google_request = self.convert_request(request);

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&google_request)
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
                            if let Some(event) = parse_google_sse(data) {
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
struct GoogleRequest {
    contents: Vec<GoogleContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GoogleContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GoogleContent {
    parts: Vec<GooglePart>,
    role: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GooglePart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

impl GooglePart {
    fn text(t: &str) -> Self {
        Self {
            text: Some(t.to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct GoogleResponse {
    candidates: Vec<GoogleCandidate>,
    usage_metadata: Option<GoogleUsage>,
}

#[derive(Debug, Deserialize)]
struct GoogleCandidate {
    content: GoogleContent,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleUsage {
    prompt_token_count: u64,
    candidates_token_count: u64,
    total_token_count: u64,
}

fn convert_google_response(response: GoogleResponse) -> ChatResponse {
    let content = response
        .candidates
        .first()
        .and_then(|c| c.content.parts.first())
        .and_then(|p| p.text.clone())
        .unwrap_or_default();

    let usage = response.usage_metadata.map(|u| Usage {
        prompt_tokens: u.prompt_token_count,
        completion_tokens: u.candidates_token_count,
        total_tokens: u.total_token_count,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
    });

    ChatResponse {
        id: format!("google_{}", uuid::Uuid::new_v4()),
        model: "google".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message::assistant(&content),
            finish_reason: response
                .candidates
                .first()
                .and_then(|c| c.finish_reason.clone()),
        }],
        usage,
    }
}

fn parse_google_sse(data: &str) -> Option<StreamEvent> {
    if data.is_empty() {
        return None;
    }

    let response: GoogleResponse = serde_json::from_str(data).ok()?;

    let text = response
        .candidates
        .first()?
        .content
        .parts
        .first()?
        .text
        .clone()?;

    Some(StreamEvent::TextDelta(text))
}
