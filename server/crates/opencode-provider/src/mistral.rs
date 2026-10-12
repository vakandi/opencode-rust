use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::{
    ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StreamEvent, StreamResult,
};

const MISTRAL_API_URL: &str = "https://api.mistral.ai/v1/chat/completions";

#[derive(Debug)]
pub struct MistralProvider {
    client: Client,
    api_key: String,
    models: Vec<ModelInfo>,
}

impl MistralProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            models: vec![
                ModelInfo {
                    id: "mistral-large-latest".to_string(),
                    name: "Mistral Large".to_string(),
                    provider: "mistral".to_string(),
                    context_window: 128000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 2.0,
                    cost_per_million_output: 6.0,
                },
                ModelInfo {
                    id: "mistral-medium-latest".to_string(),
                    name: "Mistral Medium".to_string(),
                    provider: "mistral".to_string(),
                    context_window: 32000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 2.7,
                    cost_per_million_output: 8.1,
                },
                ModelInfo {
                    id: "mistral-small-latest".to_string(),
                    name: "Mistral Small".to_string(),
                    provider: "mistral".to_string(),
                    context_window: 32000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 0.2,
                    cost_per_million_output: 0.6,
                },
                ModelInfo {
                    id: "codestral-latest".to_string(),
                    name: "Codestral".to_string(),
                    provider: "mistral".to_string(),
                    context_window: 32000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 0.3,
                    cost_per_million_output: 0.9,
                },
                ModelInfo {
                    id: "pixtral-12b-2409".to_string(),
                    name: "Pixtral 12B".to_string(),
                    provider: "mistral".to_string(),
                    context_window: 128000,
                    max_output_tokens: 8192,
                    supports_vision: true,
                    supports_tools: false,
                    cost_per_million_input: 0.15,
                    cost_per_million_output: 0.15,
                },
            ],
        }
    }
}

#[async_trait]
impl Provider for MistralProvider {
    fn id(&self) -> &str {
        "mistral"
    }
    fn name(&self) -> &str {
        "Mistral AI"
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
        let response = client
            .post(MISTRAL_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let mut stream_request = request;
        stream_request.stream = Some(true);

        let response = client
            .post(MISTRAL_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&stream_request)
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
            .map(|chunk_result| match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    for line in text.lines() {
                        if line.starts_with("data: ") {
                            let data = &line[6..];
                            if let Some(event) = crate::stream::parse_openai_sse(data) {
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
