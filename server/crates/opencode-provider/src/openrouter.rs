use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::{
    ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StreamEvent, StreamResult,
};

const OPENROUTER_API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

#[derive(Debug, Clone)]
pub struct OpenRouterConfig {
    pub api_key: String,
    pub base_url: Option<String>,
    pub site_url: Option<String>,
    pub site_name: Option<String>,
}

#[derive(Debug)]
pub struct OpenRouterProvider {
    client: Client,
    config: OpenRouterConfig,
    models: Vec<ModelInfo>,
}

impl OpenRouterProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_config(OpenRouterConfig {
            api_key: api_key.into(),
            base_url: None,
            site_url: None,
            site_name: None,
        })
    }

    pub fn with_config(config: OpenRouterConfig) -> Self {
        let models = vec![
            ModelInfo {
                id: "anthropic/claude-sonnet-4".to_string(),
                name: "Claude Sonnet 4 (OpenRouter)".to_string(),
                provider: "openrouter".to_string(),
                context_window: 200000,
                max_output_tokens: 16000,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 3.0,
                cost_per_million_output: 15.0,
            },
            ModelInfo {
                id: "anthropic/claude-3.5-sonnet".to_string(),
                name: "Claude 3.5 Sonnet (OpenRouter)".to_string(),
                provider: "openrouter".to_string(),
                context_window: 200000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 3.0,
                cost_per_million_output: 15.0,
            },
            ModelInfo {
                id: "openai/gpt-4o".to_string(),
                name: "GPT-4o (OpenRouter)".to_string(),
                provider: "openrouter".to_string(),
                context_window: 128000,
                max_output_tokens: 16384,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 2.5,
                cost_per_million_output: 10.0,
            },
            ModelInfo {
                id: "openai/gpt-4o-mini".to_string(),
                name: "GPT-4o Mini (OpenRouter)".to_string(),
                provider: "openrouter".to_string(),
                context_window: 128000,
                max_output_tokens: 16384,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.15,
                cost_per_million_output: 0.6,
            },
            ModelInfo {
                id: "google/gemini-2.5-pro-preview".to_string(),
                name: "Gemini 2.5 Pro (OpenRouter)".to_string(),
                provider: "openrouter".to_string(),
                context_window: 1000000,
                max_output_tokens: 65536,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 1.25,
                cost_per_million_output: 10.0,
            },
            ModelInfo {
                id: "google/gemini-2.0-flash-001".to_string(),
                name: "Gemini 2.0 Flash (OpenRouter)".to_string(),
                provider: "openrouter".to_string(),
                context_window: 1000000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.1,
                cost_per_million_output: 0.4,
            },
            ModelInfo {
                id: "deepseek/deepseek-chat".to_string(),
                name: "DeepSeek Chat (OpenRouter)".to_string(),
                provider: "openrouter".to_string(),
                context_window: 64000,
                max_output_tokens: 8192,
                supports_vision: false,
                supports_tools: true,
                cost_per_million_input: 0.14,
                cost_per_million_output: 0.28,
            },
            ModelInfo {
                id: "meta-llama/llama-3.3-70b-instruct".to_string(),
                name: "Llama 3.3 70B (OpenRouter)".to_string(),
                provider: "openrouter".to_string(),
                context_window: 131072,
                max_output_tokens: 8192,
                supports_vision: false,
                supports_tools: true,
                cost_per_million_input: 0.35,
                cost_per_million_output: 0.4,
            },
        ];

        Self {
            client: Client::new(),
            config,
            models,
        }
    }

    fn build_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {}", self.config.api_key).parse().unwrap(),
        );
        headers.insert("Content-Type", "application/json".parse().unwrap());

        let site_url = self
            .config
            .site_url
            .as_deref()
            .unwrap_or("https://opencode.ai/");
        headers.insert("HTTP-Referer", site_url.parse().unwrap());

        let site_name = self.config.site_name.as_deref().unwrap_or("opencode");
        headers.insert("X-Title", site_name.parse().unwrap());

        headers
    }
}

#[async_trait]
impl Provider for OpenRouterProvider {
    fn id(&self) -> &str {
        "openrouter"
    }

    fn name(&self) -> &str {
        "OpenRouter"
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
        let url = self
            .config
            .base_url
            .as_deref()
            .unwrap_or(OPENROUTER_API_URL);

        let response = client
            .post(url)
            .headers(self.build_headers())
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
        let url = self
            .config
            .base_url
            .as_deref()
            .unwrap_or(OPENROUTER_API_URL);

        let mut stream_request = request;
        stream_request.stream = Some(true);

        let response = client
            .post(url)
            .headers(self.build_headers())
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
            .map(move |chunk_result| match chunk_result {
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
