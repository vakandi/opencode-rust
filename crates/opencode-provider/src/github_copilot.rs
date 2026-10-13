use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tracing;

use crate::bootstrap::should_use_copilot_responses_api;
use crate::custom_fetch::get_custom_fetch_proxy;
use crate::responses::{
    FinishReason, GenerateOptions, OpenAIResponsesConfig, OpenAIResponsesLanguageModel,
    ResponsesProviderOptions, StreamOptions,
};
use crate::tools::InputTool;
use crate::{
    ChatRequest, ChatResponse, Choice, Content, Message, ModelInfo, Provider, ProviderError, Role,
    StreamEvent, StreamResult, Usage,
};

const COPILOT_API_URL: &str = "https://api.githubcopilot.com/chat/completions";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopilotRoute {
    Responses,
    Legacy,
}

fn select_copilot_route(model_id: &str) -> CopilotRoute {
    if should_use_copilot_responses_api(model_id) {
        CopilotRoute::Responses
    } else {
        CopilotRoute::Legacy
    }
}

async fn resolve_with_fallback<T, PFut, FFut, F>(
    primary: PFut,
    fallback: F,
) -> Result<T, ProviderError>
where
    PFut: Future<Output = Result<T, ProviderError>>,
    F: FnOnce(ProviderError) -> FFut,
    FFut: Future<Output = Result<T, ProviderError>>,
{
    match primary.await {
        Ok(value) => Ok(value),
        Err(err) => fallback(err).await,
    }
}

#[derive(Debug, Clone)]
pub struct GitHubCopilotConfig {
    pub oauth_token: String,
    pub base_url: Option<String>,
}

#[derive(Debug)]
pub struct GitHubCopilotProvider {
    client: Client,
    config: GitHubCopilotConfig,
    models: Vec<ModelInfo>,
}

impl GitHubCopilotProvider {
    pub fn new(oauth_token: impl Into<String>) -> Self {
        Self::with_config(GitHubCopilotConfig {
            oauth_token: oauth_token.into(),
            base_url: None,
        })
    }

    pub fn with_config(config: GitHubCopilotConfig) -> Self {
        let models = vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                name: "GPT-4o (GitHub Copilot)".to_string(),
                provider: "github-copilot".to_string(),
                context_window: 128000,
                max_output_tokens: 16384,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
            },
            ModelInfo {
                id: "gpt-4o-mini".to_string(),
                name: "GPT-4o Mini (GitHub Copilot)".to_string(),
                provider: "github-copilot".to_string(),
                context_window: 128000,
                max_output_tokens: 16384,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
            },
            ModelInfo {
                id: "claude-3.5-sonnet".to_string(),
                name: "Claude 3.5 Sonnet (GitHub Copilot)".to_string(),
                provider: "github-copilot".to_string(),
                context_window: 200000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
            },
            ModelInfo {
                id: "claude-3.5-haiku".to_string(),
                name: "Claude 3.5 Haiku (GitHub Copilot)".to_string(),
                provider: "github-copilot".to_string(),
                context_window: 200000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
            },
            ModelInfo {
                id: "o1".to_string(),
                name: "o1 (GitHub Copilot)".to_string(),
                provider: "github-copilot".to_string(),
                context_window: 200000,
                max_output_tokens: 100000,
                supports_vision: true,
                supports_tools: false,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
            },
            ModelInfo {
                id: "o1-mini".to_string(),
                name: "o1 Mini (GitHub Copilot)".to_string(),
                provider: "github-copilot".to_string(),
                context_window: 128000,
                max_output_tokens: 65536,
                supports_vision: false,
                supports_tools: false,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
            },
        ];

        Self {
            client: Client::new(),
            config,
            models,
        }
    }

    fn convert_request(&self, request: ChatRequest) -> CopilotRequest {
        let messages: Vec<CopilotMessage> = request
            .messages
            .into_iter()
            .map(|msg| CopilotMessage {
                role: match msg.role {
                    Role::System => "system".to_string(),
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                    Role::Tool => "user".to_string(),
                },
                content: match msg.content {
                    Content::Text(t) => CopilotContent::Text(t),
                    Content::Parts(parts) => {
                        let contents: Vec<CopilotContentPart> = parts
                            .into_iter()
                            .filter_map(|p| {
                                if let Some(text) = p.text {
                                    Some(CopilotContentPart {
                                        content_type: "text".to_string(),
                                        text: Some(text),
                                        image_url: p
                                            .image_url
                                            .map(|iu| CopilotImageUrl { url: iu.url }),
                                    })
                                } else if p.image_url.is_some() {
                                    Some(CopilotContentPart {
                                        content_type: "image_url".to_string(),
                                        text: None,
                                        image_url: p
                                            .image_url
                                            .map(|iu| CopilotImageUrl { url: iu.url }),
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect();
                        CopilotContent::Parts(contents)
                    }
                },
            })
            .collect();

        CopilotRequest {
            model: request.model,
            messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            stream: false,
        }
    }

    fn responses_url(base_url: Option<&str>, path: &str) -> String {
        let path = path.trim_start_matches('/');
        match base_url {
            None => format!("https://api.githubcopilot.com/{}", path),
            Some(base) => {
                if base.ends_with("/chat/completions") {
                    return format!("{}/{}", base.trim_end_matches("/chat/completions"), path);
                }
                if base.ends_with('/') {
                    format!("{}{}", base, path)
                } else {
                    format!("{}/{}", base, path)
                }
            }
        }
    }

    fn extract_responses_provider_options(
        provider_options: Option<&HashMap<String, serde_json::Value>>,
    ) -> ResponsesProviderOptions {
        let Some(options) = provider_options else {
            return ResponsesProviderOptions::default();
        };

        for key in ["github-copilot", "openai", "responses"] {
            if let Some(value) = options.get(key) {
                if let Ok(parsed) =
                    serde_json::from_value::<ResponsesProviderOptions>(value.clone())
                {
                    return parsed;
                }
            }
        }

        serde_json::from_value::<ResponsesProviderOptions>(serde_json::json!(options))
            .unwrap_or_default()
    }

    fn tools_to_input_tools(tools: Option<&Vec<crate::ToolDefinition>>) -> Option<Vec<InputTool>> {
        let tools = tools?;
        if tools.is_empty() {
            return None;
        }
        Some(
            tools
                .iter()
                .map(|tool| InputTool::Function {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.parameters.clone(),
                })
                .collect(),
        )
    }

    fn responses_generate_options(&self, request: &ChatRequest) -> GenerateOptions {
        let mut prompt = request.messages.clone();
        if let Some(system) = &request.system {
            let has_system = prompt.iter().any(|m| matches!(m.role, Role::System));
            if !has_system {
                prompt.insert(0, Message::system(system.clone()));
            }
        }

        let mut provider_options =
            Self::extract_responses_provider_options(request.provider_options.as_ref());
        if provider_options.reasoning_effort.is_none() {
            provider_options.reasoning_effort =
                copilot_reasoning_effort(&request.model, request.variant.as_deref())
                    .map(ToString::to_string);
        }

        GenerateOptions {
            prompt,
            tools: Self::tools_to_input_tools(request.tools.as_ref()),
            tool_choice: None,
            max_output_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            top_k: None,
            seed: None,
            presence_penalty: None,
            frequency_penalty: None,
            stop_sequences: None,
            provider_options: Some(provider_options),
            response_format: None,
        }
    }

    fn responses_model(
        &self,
        model_id: &str,
        session_id: Option<&str>,
    ) -> OpenAIResponsesLanguageModel {
        let token = self.config.oauth_token.clone();
        let base_url = self.config.base_url.clone();
        let session_client = crate::session_client::for_session(session_id.unwrap_or(""));

        OpenAIResponsesLanguageModel::new(
            model_id.to_string(),
            OpenAIResponsesConfig {
                provider: "github-copilot".to_string(),
                url: Arc::new(move |path, _model| Self::responses_url(base_url.as_deref(), path)),
                headers: Arc::new(move || {
                    let mut h = HashMap::new();
                    h.insert("Authorization".to_string(), format!("Bearer {}", token));
                    h.insert(
                        "Copilot-Integration-Id".to_string(),
                        "vscode-chat".to_string(),
                    );
                    h
                }),
                client: Some(session_client.as_ref().clone()),
                file_id_prefixes: Some(vec!["file-".to_string()]),
                generate_id: None,
                metadata_extractor: None,
            },
        )
    }

    fn finish_reason_to_string(reason: FinishReason) -> String {
        match reason {
            FinishReason::Stop => "stop".to_string(),
            FinishReason::Length => "length".to_string(),
            FinishReason::ContentFilter => "content_filter".to_string(),
            FinishReason::ToolCalls => "tool-calls".to_string(),
            FinishReason::Error => "error".to_string(),
            FinishReason::Unknown => "unknown".to_string(),
        }
    }

    fn responses_chat_response(
        request: &ChatRequest,
        result: crate::responses::ResponsesGenerateResult,
    ) -> ChatResponse {
        let usage = Usage {
            prompt_tokens: result.usage.input_tokens,
            completion_tokens: result.usage.output_tokens,
            total_tokens: result.usage.input_tokens + result.usage.output_tokens,
            cache_read_input_tokens: result
                .usage
                .input_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens),
            cache_creation_input_tokens: None,
        };

        ChatResponse {
            id: result
                .metadata
                .response_id
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            model: result
                .metadata
                .model_id
                .unwrap_or_else(|| request.model.clone()),
            choices: vec![Choice {
                index: 0,
                message: result.message,
                finish_reason: Some(Self::finish_reason_to_string(result.finish_reason)),
            }],
            usage: Some(usage),
        }
    }

    async fn chat_legacy(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let url = self.config.base_url.as_deref().unwrap_or(COPILOT_API_URL);
        let copilot_request = self.convert_request(request);

        let response = client
            .post(url)
            .header("Content-Type", "application/json")
            .header(
                "Authorization",
                format!("Bearer {}", self.config.oauth_token),
            )
            .header("Copilot-Integration-Id", "vscode-chat")
            .json(&copilot_request)
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

        let copilot_response: CopilotResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_copilot_response(copilot_response))
    }

    async fn chat_stream_legacy(
        &self,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let url = self.config.base_url.as_deref().unwrap_or(COPILOT_API_URL);
        let mut copilot_request = self.convert_request(request);
        copilot_request.stream = true;

        let response = client
            .post(url)
            .header("Content-Type", "application/json")
            .header(
                "Authorization",
                format!("Bearer {}", self.config.oauth_token),
            )
            .header("Copilot-Integration-Id", "vscode-chat")
            .header("Accept", "text/event-stream")
            .json(&copilot_request)
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
                            if let Some(event) = parse_copilot_sse(data) {
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

#[async_trait]
impl Provider for GitHubCopilotProvider {
    fn id(&self) -> &str {
        "github-copilot"
    }

    fn name(&self) -> &str {
        "GitHub Copilot"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.models.clone()
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.id == id)
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        if select_copilot_route(&request.model) == CopilotRoute::Responses {
            let response_model =
                self.responses_model(&request.model, request.session_id.as_deref());
            let options = self.responses_generate_options(&request);
            let request_for_primary = request.clone();
            let model_for_log = request.model.clone();
            return resolve_with_fallback(
                async move {
                    response_model
                        .do_generate(options)
                        .await
                        .map(|result| Self::responses_chat_response(&request_for_primary, result))
                },
                move |err| async move {
                    if get_custom_fetch_proxy("github-copilot").is_some() {
                        tracing::warn!(
                            model = %model_for_log,
                            error = %err,
                            "Copilot responses generate failed while custom fetch proxy is active; skipping legacy fallback"
                        );
                        return Err(err);
                    }
                    tracing::warn!(
                        model = %model_for_log,
                        error = %err,
                        "Copilot responses generate failed, falling back to chat completions"
                    );
                    self.chat_legacy(request).await
                },
            )
            .await;
        }

        self.chat_legacy(request).await
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        if select_copilot_route(&request.model) == CopilotRoute::Responses {
            let response_model =
                self.responses_model(&request.model, request.session_id.as_deref());
            let options = StreamOptions {
                generate: self.responses_generate_options(&request),
            };
            let model_for_log = request.model.clone();
            return resolve_with_fallback(
                async move { response_model.do_stream(options).await },
                move |err| async move {
                    if get_custom_fetch_proxy("github-copilot").is_some() {
                        tracing::warn!(
                            model = %model_for_log,
                            error = %err,
                            "Copilot responses stream failed while custom fetch proxy is active; skipping legacy fallback"
                        );
                        return Err(err);
                    }
                    tracing::warn!(
                        model = %model_for_log,
                        error = %err,
                        "Copilot responses stream failed, falling back to chat completions"
                    );
                    self.chat_stream_legacy(request).await
                },
            )
            .await;
        }

        self.chat_stream_legacy(request).await
    }
}

fn copilot_reasoning_effort(model_id: &str, variant: Option<&str>) -> Option<&'static str> {
    let variant = variant?.trim().to_ascii_lowercase();
    let model = model_id.to_ascii_lowercase();
    let supports_effort = model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.contains("gpt-5");
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

#[derive(Debug, Serialize)]
struct CopilotRequest {
    model: String,
    messages: Vec<CopilotMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct CopilotMessage {
    role: String,
    content: CopilotContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum CopilotContent {
    Text(String),
    Parts(Vec<CopilotContentPart>),
}

#[derive(Debug, Serialize)]
struct CopilotContentPart {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<CopilotImageUrl>,
}

#[derive(Debug, Serialize)]
struct CopilotImageUrl {
    url: String,
}

#[derive(Debug, Deserialize)]
struct CopilotResponse {
    id: String,
    model: String,
    choices: Vec<CopilotChoice>,
    usage: Option<CopilotUsage>,
}

#[derive(Debug, Deserialize)]
struct CopilotChoice {
    _index: u32,
    message: CopilotResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotResponseMessage {
    _role: String,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct CopilotStreamResponse {
    choices: Vec<CopilotStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct CopilotStreamChoice {
    delta: CopilotDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotDelta {
    content: Option<String>,
}

fn convert_copilot_response(response: CopilotResponse) -> ChatResponse {
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

fn parse_copilot_sse(data: &str) -> Option<StreamEvent> {
    if data.is_empty() {
        return None;
    }

    let response: CopilotStreamResponse = serde_json::from_str(data).ok()?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_fetch::{
        register_custom_fetch_proxy, unregister_custom_fetch_proxy, CustomFetchProxy,
        CustomFetchRequest, CustomFetchResponse, CustomFetchStreamResponse,
    };
    use async_trait::async_trait;
    use futures::stream;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct NoopProxy;

    #[async_trait]
    impl CustomFetchProxy for NoopProxy {
        async fn fetch(
            &self,
            _request: CustomFetchRequest,
        ) -> Result<CustomFetchResponse, ProviderError> {
            Ok(CustomFetchResponse {
                status: 200,
                headers: HashMap::new(),
                body: String::new(),
            })
        }

        async fn fetch_stream(
            &self,
            _request: CustomFetchRequest,
        ) -> Result<CustomFetchStreamResponse, ProviderError> {
            Ok(CustomFetchStreamResponse {
                status: 200,
                headers: HashMap::new(),
                stream: Box::pin(stream::empty()),
            })
        }
    }

    #[test]
    fn select_route_uses_responses_for_gpt5_models() {
        assert_eq!(select_copilot_route("gpt-5"), CopilotRoute::Responses);
        assert_eq!(select_copilot_route("gpt-5-codex"), CopilotRoute::Responses);
    }

    #[test]
    fn select_route_keeps_legacy_for_non_gpt5_models() {
        assert_eq!(select_copilot_route("gpt-4o"), CopilotRoute::Legacy);
        assert_eq!(
            select_copilot_route("claude-3.5-sonnet"),
            CopilotRoute::Legacy
        );
    }

    #[tokio::test]
    async fn resolve_with_fallback_calls_fallback_on_primary_error() {
        let result = resolve_with_fallback(
            async {
                Err::<usize, ProviderError>(ProviderError::ApiError("responses failed".to_string()))
            },
            |_err| async { Ok::<_, ProviderError>(42usize) },
        )
        .await
        .expect("fallback should handle responses error");
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn resolve_with_fallback_skips_legacy_when_custom_fetch_active() {
        register_custom_fetch_proxy("github-copilot", Arc::new(NoopProxy));

        let result = resolve_with_fallback(
            async {
                Err::<usize, ProviderError>(ProviderError::ApiError("responses failed".to_string()))
            },
            |e| async move {
                if get_custom_fetch_proxy("github-copilot").is_some() {
                    return Err(e);
                }
                Ok::<_, ProviderError>(42usize)
            },
        )
        .await;

        unregister_custom_fetch_proxy("github-copilot");
        assert!(result.is_err());
    }
}
