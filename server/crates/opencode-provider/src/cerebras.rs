use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::{
    ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StreamEvent, StreamResult,
};

const CEREBRAS_API_URL: &str = "https://api.cerebras.ai/v1/chat/completions";

#[derive(Debug)]
pub struct CerebrasProvider {
    client: Client,
    api_key: String,
    models: Vec<ModelInfo>,
}

impl CerebrasProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            models: vec![
                ModelInfo {
                    id: "llama-3.3-70b".to_string(),
                    name: "Llama 3.3 70B (Cerebras)".to_string(),
                    provider: "cerebras".to_string(),
                    context_window: 128000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 0.6,
                    cost_per_million_output: 0.6,
                },
                ModelInfo {
                    id: "llama-3.1-8b".to_string(),
                    name: "Llama 3.1 8B (Cerebras)".to_string(),
                    provider: "cerebras".to_string(),
                    context_window: 128000,
                    max_output_tokens: 8192,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: 0.1,
                    cost_per_million_output: 0.1,
                },
            ],
        }
    }
}

#[async_trait]
impl Provider for CerebrasProvider {
    fn id(&self) -> &str {
        "cerebras"
    }

    fn name(&self) -> &str {
        "Cerebras"
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
        let url = CEREBRAS_API_URL;

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": request.messages,
        });

        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = serde_json::json!(max_tokens);
        }
        if let Some(tools) = &request.tools {
            if !tools.is_empty() {
                body["tools"] = serde_json::json!(tools);
            }
        }

        let response = client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(error_text));
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(chat_response)
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let url = CEREBRAS_API_URL;

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": request.messages,
            "stream": true,
        });

        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = serde_json::json!(max_tokens);
        }
        if let Some(tools) = &request.tools {
            if !tools.is_empty() {
                body["tools"] = serde_json::json!(tools);
            }
        }

        let response = client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(error_text));
        }

        let stream = response
            .bytes_stream()
            .then(|result| async move {
                match result {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        let mut events: Vec<Result<StreamEvent, ProviderError>> = Vec::new();

                        for line in text.lines() {
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    events.push(Ok(StreamEvent::Done));
                                    continue;
                                }

                                if let Some(event) = crate::stream::parse_openai_sse(data) {
                                    events.push(Ok(event));
                                }
                            }
                        }

                        events
                    }
                    Err(e) => vec![Err(ProviderError::StreamError(e.to_string()))],
                }
            })
            .flat_map(|events| futures::stream::iter(events));

        Ok(Box::pin(stream))
    }
}
