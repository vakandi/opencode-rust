use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::{
    ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StreamEvent, StreamResult,
};

const GROQ_API_URL: &str = "https://api.groq.com/openai/v1/chat/completions";

#[derive(Debug)]
pub struct GroqProvider {
    client: Client,
    api_key: String,
    models: Vec<ModelInfo>,
}

impl GroqProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            models: vec![
                ModelInfo {
                    id: "llama-3.3-70b-versatile".to_string(),
                    name: "Llama 3.3 70B (Groq)".to_string(),
                    provider: "groq".to_string(),
                    context_window: 128000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 0.59,
                    cost_per_million_output: 0.79,
                },
                ModelInfo {
                    id: "llama-3.1-8b-instant".to_string(),
                    name: "Llama 3.1 8B (Groq)".to_string(),
                    provider: "groq".to_string(),
                    context_window: 128000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 0.05,
                    cost_per_million_output: 0.08,
                },
                ModelInfo {
                    id: "mixtral-8x7b-32768".to_string(),
                    name: "Mixtral 8x7B (Groq)".to_string(),
                    provider: "groq".to_string(),
                    context_window: 32768,
                    max_output_tokens: 4096,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 0.24,
                    cost_per_million_output: 0.24,
                },
                ModelInfo {
                    id: "gemma2-9b-it".to_string(),
                    name: "Gemma 2 9B (Groq)".to_string(),
                    provider: "groq".to_string(),
                    context_window: 8192,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 0.2,
                    cost_per_million_output: 0.2,
                },
                ModelInfo {
                    id: "deepseek-r1-distill-llama-70b".to_string(),
                    name: "DeepSeek R1 Llama 70B (Groq)".to_string(),
                    provider: "groq".to_string(),
                    context_window: 128000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 0.75,
                    cost_per_million_output: 0.99,
                },
            ],
        }
    }
}

#[async_trait]
impl Provider for GroqProvider {
    fn id(&self) -> &str {
        "groq"
    }
    fn name(&self) -> &str {
        "Groq"
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
            .post(GROQ_API_URL)
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
            .post(GROQ_API_URL)
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
