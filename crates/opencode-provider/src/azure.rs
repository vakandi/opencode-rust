use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;

use crate::{
    ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, StreamEvent, StreamResult,
};

#[derive(Debug, Clone)]
pub struct AzureConfig {
    pub api_key: String,
    pub endpoint: String,
    pub deployment_name: Option<String>,
    pub api_version: Option<String>,
}

#[derive(Debug)]
pub struct AzureProvider {
    client: Client,
    config: AzureConfig,
    models: Vec<ModelInfo>,
}

impl AzureProvider {
    pub fn new(api_key: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self::with_config(AzureConfig {
            api_key: api_key.into(),
            endpoint: endpoint.into(),
            deployment_name: None,
            api_version: None,
        })
    }

    pub fn with_config(config: AzureConfig) -> Self {
        let models = vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                name: "GPT-4o (Azure)".to_string(),
                provider: "azure".to_string(),
                context_window: 128000,
                max_output_tokens: 16384,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 2.5,
                cost_per_million_output: 10.0,
            },
            ModelInfo {
                id: "gpt-4o-mini".to_string(),
                name: "GPT-4o Mini (Azure)".to_string(),
                provider: "azure".to_string(),
                context_window: 128000,
                max_output_tokens: 16384,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.15,
                cost_per_million_output: 0.6,
            },
            ModelInfo {
                id: "gpt-4-turbo".to_string(),
                name: "GPT-4 Turbo (Azure)".to_string(),
                provider: "azure".to_string(),
                context_window: 128000,
                max_output_tokens: 4096,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 10.0,
                cost_per_million_output: 30.0,
            },
            ModelInfo {
                id: "gpt-35-turbo".to_string(),
                name: "GPT-3.5 Turbo (Azure)".to_string(),
                provider: "azure".to_string(),
                context_window: 16384,
                max_output_tokens: 4096,
                supports_vision: false,
                supports_tools: true,
                cost_per_million_input: 0.5,
                cost_per_million_output: 1.5,
            },
        ];

        Self {
            client: Client::new(),
            config,
            models,
        }
    }

    fn build_url(&self, model: &str, stream: bool) -> String {
        let deployment = self.config.deployment_name.as_deref().unwrap_or(model);
        let api_version = self
            .config
            .api_version
            .as_deref()
            .unwrap_or("2024-02-15-preview");

        let endpoint = self.config.endpoint.trim_end_matches('/');

        if stream {
            format!(
                "{}/openai/deployments/{}/chat/completions?api-version={}",
                endpoint, deployment, api_version
            )
        } else {
            format!(
                "{}/openai/deployments/{}/chat/completions?api-version={}",
                endpoint, deployment, api_version
            )
        }
    }

    fn build_request_body(request: &ChatRequest) -> Result<Value, ProviderError> {
        let mut value = serde_json::to_value(request)
            .map_err(|e| ProviderError::InvalidRequest(e.to_string()))?;
        let effort = openai_reasoning_effort(&request.model, request.variant.as_deref());
        if let Some(effort) = effort {
            if let Value::Object(obj) = &mut value {
                obj.insert(
                    "reasoning_effort".to_string(),
                    Value::String(effort.to_string()),
                );
            }
        }
        Ok(value)
    }
}

#[async_trait]
impl Provider for AzureProvider {
    fn id(&self) -> &str {
        "azure"
    }

    fn name(&self) -> &str {
        "Azure OpenAI"
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
        let url = self.build_url(&request.model, false);
        let request_body = Self::build_request_body(&request)?;

        let response = client
            .post(&url)
            .header("api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&request_body)
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
        let url = self.build_url(&request.model, true);

        let mut stream_request = request;
        stream_request.stream = Some(true);
        let request_body = Self::build_request_body(&stream_request)?;

        let response = client
            .post(&url)
            .header("api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&request_body)
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

fn openai_reasoning_effort(model_id: &str, variant: Option<&str>) -> Option<&'static str> {
    let variant = variant?.trim().to_ascii_lowercase();
    let model = model_id.to_ascii_lowercase();
    let supports_effort = model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.contains("gpt-5")
        || model.contains("codex");
    if !supports_effort {
        return None;
    }

    match variant.as_str() {
        "none" => Some("none"),
        "minimal" => Some("minimal"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "max" | "xhigh" => Some("high"),
        _ => None,
    }
}
