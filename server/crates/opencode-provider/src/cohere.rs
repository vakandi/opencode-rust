use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::{
    ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StreamEvent, StreamResult,
};

const COHERE_API_URL: &str = "https://api.cohere.ai/v2/chat";

#[derive(Debug)]
pub struct CohereProvider {
    client: Client,
    api_key: String,
    models: Vec<ModelInfo>,
}

impl CohereProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            models: vec![
                ModelInfo {
                    id: "command-r-plus-08-2024".to_string(),
                    name: "Command R+".to_string(),
                    provider: "cohere".to_string(),
                    context_window: 128000,
                    max_output_tokens: 4096,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 2.5,
                    cost_per_million_output: 10.0,
                },
                ModelInfo {
                    id: "command-r-08-2024".to_string(),
                    name: "Command R".to_string(),
                    provider: "cohere".to_string(),
                    context_window: 128000,
                    max_output_tokens: 4096,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 0.15,
                    cost_per_million_output: 0.6,
                },
                ModelInfo {
                    id: "command".to_string(),
                    name: "Command".to_string(),
                    provider: "cohere".to_string(),
                    context_window: 4096,
                    max_output_tokens: 4096,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 1.0,
                    cost_per_million_output: 2.0,
                },
                ModelInfo {
                    id: "command-light".to_string(),
                    name: "Command Light".to_string(),
                    provider: "cohere".to_string(),
                    context_window: 4096,
                    max_output_tokens: 4096,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 0.3,
                    cost_per_million_output: 0.6,
                },
            ],
        }
    }
}

#[async_trait]
impl Provider for CohereProvider {
    fn id(&self) -> &str {
        "cohere"
    }
    fn name(&self) -> &str {
        "Cohere"
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
            .post(COHERE_API_URL)
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
            .post(COHERE_API_URL)
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
