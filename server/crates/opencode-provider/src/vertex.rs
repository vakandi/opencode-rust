use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    ChatRequest, ChatResponse, Choice, Content, Message, ModelInfo, Provider, ProviderError, Role,
    StreamEvent, StreamResult, Usage,
};

const VERTEX_API_BASE: &str = "https://aiplatform.googleapis.com/v1";

#[derive(Debug, Clone)]
pub struct VertexConfig {
    pub access_token: String,
    pub project_id: String,
    pub location: String,
    pub base_url: Option<String>,
}

#[derive(Debug)]
pub struct GoogleVertexProvider {
    client: Client,
    config: VertexConfig,
    models: Vec<ModelInfo>,
}

impl GoogleVertexProvider {
    pub fn new(
        access_token: impl Into<String>,
        project_id: impl Into<String>,
        location: impl Into<String>,
    ) -> Self {
        Self::with_config(VertexConfig {
            access_token: access_token.into(),
            project_id: project_id.into(),
            location: location.into(),
            base_url: None,
        })
    }

    pub fn with_config(config: VertexConfig) -> Self {
        let models = vec![
            ModelInfo {
                id: "gemini-2.0-flash".to_string(),
                name: "Gemini 2.0 Flash (Vertex)".to_string(),
                provider: "google-vertex".to_string(),
                context_window: 1_000_000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.1,
                cost_per_million_output: 0.4,
            },
            ModelInfo {
                id: "gemini-2.0-flash-lite".to_string(),
                name: "Gemini 2.0 Flash Lite (Vertex)".to_string(),
                provider: "google-vertex".to_string(),
                context_window: 1_000_000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.075,
                cost_per_million_output: 0.3,
            },
            ModelInfo {
                id: "gemini-1.5-pro".to_string(),
                name: "Gemini 1.5 Pro (Vertex)".to_string(),
                provider: "google-vertex".to_string(),
                context_window: 2_000_000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 1.25,
                cost_per_million_output: 5.0,
            },
            ModelInfo {
                id: "gemini-1.5-flash".to_string(),
                name: "Gemini 1.5 Flash (Vertex)".to_string(),
                provider: "google-vertex".to_string(),
                context_window: 1_000_000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.075,
                cost_per_million_output: 0.3,
            },
            ModelInfo {
                id: "gemini-1.0-pro".to_string(),
                name: "Gemini 1.0 Pro (Vertex)".to_string(),
                provider: "google-vertex".to_string(),
                context_window: 32_000,
                max_output_tokens: 2048,
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

    fn build_url(&self, model: &str, method: &str) -> String {
        let base = self.config.base_url.as_deref().unwrap_or(VERTEX_API_BASE);
        format!(
            "{}/projects/{}/locations/{}/publishers/google/models/{}:{}",
            base, self.config.project_id, self.config.location, model, method
        )
    }

    fn convert_request(&self, request: ChatRequest) -> VertexRequest {
        let mut contents = Vec::new();
        let mut system_instruction = None;

        for msg in request.messages {
            match msg.role {
                Role::System => {
                    if let Content::Text(text) = msg.content {
                        system_instruction = Some(VertexContent {
                            parts: vec![VertexPart::text(&text)],
                            role: "user".to_string(),
                        });
                    }
                }
                Role::User => {
                    let parts = self.content_to_parts(&msg.content);
                    contents.push(VertexContent {
                        parts,
                        role: "user".to_string(),
                    });
                }
                Role::Assistant => {
                    let parts = self.content_to_parts(&msg.content);
                    contents.push(VertexContent {
                        parts,
                        role: "model".to_string(),
                    });
                }
                Role::Tool => {
                    // Tool messages are handled as user messages with function response
                    if let Content::Parts(parts) = msg.content {
                        let vertex_parts: Vec<VertexPart> = parts
                            .into_iter()
                            .filter_map(|p| {
                                p.tool_result.map(|tr| {
                                    VertexPart::function_response(&tr.tool_use_id, &tr.content)
                                })
                            })
                            .collect();
                        if !vertex_parts.is_empty() {
                            contents.push(VertexContent {
                                parts: vertex_parts,
                                role: "user".to_string(),
                            });
                        }
                    }
                }
            }
        }

        let generation_config = VertexGenerationConfig {
            max_output_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
        };

        VertexRequest {
            contents,
            system_instruction,
            generation_config: Some(generation_config),
            tools: None,
        }
    }

    fn content_to_parts(&self, content: &Content) -> Vec<VertexPart> {
        match content {
            Content::Text(text) => vec![VertexPart::text(text)],
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| {
                    if let Some(text) = &p.text {
                        Some(VertexPart::text(text))
                    } else if let Some(tool_use) = &p.tool_use {
                        Some(VertexPart::function_call(
                            &tool_use.name,
                            tool_use.input.clone(),
                        ))
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }
}

#[async_trait]
impl Provider for GoogleVertexProvider {
    fn id(&self) -> &str {
        "google-vertex"
    }

    fn name(&self) -> &str {
        "Google Vertex AI"
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
        let url = self.build_url(&request.model, "generateContent");
        let vertex_request = self.convert_request(request);

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .json(&vertex_request)
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

        let vertex_response: VertexResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_vertex_response(vertex_response))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let url = self.build_url(&request.model, "streamGenerateContent");
        let vertex_request = self.convert_request(request);

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .header("Accept", "text/event-stream")
            .json(&vertex_request)
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

        let stream = response.bytes_stream().map(move |chunk_result| {
            match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    for line in text.lines() {
                        if line.starts_with("data: ") {
                            let data = &line[6..];
                            if let Some(event) = parse_vertex_sse(data) {
                                return Ok(event);
                            }
                        } else if !line.is_empty() && !line.starts_with(':') {
                            // Vertex sometimes returns raw JSON without "data: " prefix
                            if let Some(event) = parse_vertex_sse(line) {
                                return Ok(event);
                            }
                        }
                    }
                    Ok(StreamEvent::TextDelta(String::new()))
                }
                Err(e) => Err(ProviderError::StreamError(e.to_string())),
            }
        });

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Serialize)]
struct VertexRequest {
    contents: Vec<VertexContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<VertexContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<VertexGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<VertexTool>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct VertexContent {
    parts: Vec<VertexPart>,
    role: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct VertexPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<VertexFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<VertexFunctionResponse>,
}

impl VertexPart {
    fn text(t: &str) -> Self {
        Self {
            text: Some(t.to_string()),
            function_call: None,
            function_response: None,
        }
    }

    fn function_call(name: &str, args: serde_json::Value) -> Self {
        Self {
            text: None,
            function_call: Some(VertexFunctionCall {
                name: name.to_string(),
                args,
            }),
            function_response: None,
        }
    }

    fn function_response(name: &str, response: &str) -> Self {
        Self {
            text: None,
            function_call: None,
            function_response: Some(VertexFunctionResponse {
                name: name.to_string(),
                response: serde_json::json!({ "content": response }),
            }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct VertexFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct VertexFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct VertexGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

#[derive(Debug, Serialize)]
struct VertexTool {
    function_declarations: Vec<VertexFunctionDeclaration>,
}

#[derive(Debug, Serialize)]
struct VertexFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct VertexResponse {
    candidates: Vec<VertexCandidate>,
    #[serde(default)]
    usage_metadata: Option<VertexUsage>,
}

#[derive(Debug, Deserialize)]
struct VertexCandidate {
    content: VertexContent,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    _safety_ratings: Option<Vec<VertexSafetyRating>>,
}

#[derive(Debug, Deserialize)]
struct VertexUsage {
    prompt_token_count: u64,
    candidates_token_count: u64,
    total_token_count: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct VertexSafetyRating {
    category: String,
    probability: String,
}

fn convert_vertex_response(response: VertexResponse) -> ChatResponse {
    let candidate = response.candidates.first();

    let content = candidate
        .and_then(|c| c.content.parts.first())
        .map(|p| {
            if let Some(text) = &p.text {
                Content::Text(text.clone())
            } else if let Some(fc) = &p.function_call {
                Content::Parts(vec![crate::ContentPart {
                    content_type: "tool_use".to_string(),
                    text: None,
                    image_url: None,
                    tool_use: Some(crate::ToolUse {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: fc.name.clone(),
                        input: fc.args.clone(),
                    }),
                    tool_result: None,
                    cache_control: None,
                    filename: None,
                    media_type: None,
                    provider_options: None,
                }])
            } else {
                Content::Text(String::new())
            }
        })
        .unwrap_or(Content::Text(String::new()));

    let usage = response.usage_metadata.map(|u| Usage {
        prompt_tokens: u.prompt_token_count,
        completion_tokens: u.candidates_token_count,
        total_tokens: u.total_token_count,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
    });

    ChatResponse {
        id: format!("vertex_{}", uuid::Uuid::new_v4()),
        model: "google-vertex".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content,
                cache_control: None,
                provider_options: None,
            },
            finish_reason: candidate.and_then(|c| c.finish_reason.clone()),
        }],
        usage,
    }
}

fn parse_vertex_sse(data: &str) -> Option<StreamEvent> {
    if data.is_empty() || data == "[DONE]" {
        return None;
    }

    let response: VertexResponse = serde_json::from_str(data).ok()?;

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
