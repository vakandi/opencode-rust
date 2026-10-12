use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::{
    ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StreamEvent, StreamResult,
};

const PERPLEXITY_API_URL: &str = "https://api.perplexity.ai/chat/completions";

#[derive(Debug)]
pub struct PerplexityProvider {
    client: Client,
    api_key: String,
    models: Vec<ModelInfo>,
}

impl PerplexityProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            models: vec![
                ModelInfo {
                    id: "sonar-pro".to_string(),
                    name: "Sonar Pro".to_string(),
                    provider: "perplexity".to_string(),
                    context_window: 200000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 3.0,
                    cost_per_million_output: 15.0,
                },
                ModelInfo {
                    id: "sonar".to_string(),
                    name: "Sonar".to_string(),
                    provider: "perplexity".to_string(),
                    context_window: 127000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 1.0,
                    cost_per_million_output: 1.0,
                },
                ModelInfo {
                    id: "sonar-reasoning-pro".to_string(),
                    name: "Sonar Reasoning Pro".to_string(),
                    provider: "perplexity".to_string(),
                    context_window: 127000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 2.0,
                    cost_per_million_output: 8.0,
                },
                ModelInfo {
                    id: "sonar-reasoning".to_string(),
                    name: "Sonar Reasoning".to_string(),
                    provider: "perplexity".to_string(),
                    context_window: 127000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 1.0,
                    cost_per_million_output: 5.0,
                },
            ],
        }
    }
}

#[async_trait]
impl Provider for PerplexityProvider {
    fn id(&self) -> &str {
        "perplexity"
    }
    fn name(&self) -> &str {
        "Perplexity"
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
            .post(PERPLEXITY_API_URL)
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
            .post(PERPLEXITY_API_URL)
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
