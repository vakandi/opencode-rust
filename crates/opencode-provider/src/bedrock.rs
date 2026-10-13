use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    ChatRequest, ChatResponse, Choice, Content, Message, ModelInfo, Provider, ProviderError, Role,
    StreamEvent, StreamResult, Usage,
};

const BEDROCK_RUNTIME_URL: &str = "https://bedrock-runtime.{region}.amazonaws.com";

#[derive(Debug, Clone)]
pub struct BedrockConfig {
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub endpoint_url: Option<String>,
}

#[derive(Debug)]
pub struct BedrockProvider {
    client: Client,
    config: BedrockConfig,
    models: Vec<ModelInfo>,
}

impl BedrockProvider {
    pub fn new(
        region: impl Into<String>,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
    ) -> Self {
        Self::with_config(BedrockConfig {
            region: region.into(),
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token: None,
            endpoint_url: None,
        })
    }

    pub fn with_config(config: BedrockConfig) -> Self {
        let models = vec![
            ModelInfo {
                id: "anthropic.claude-sonnet-4-20250514-v1:0".to_string(),
                name: "Claude Sonnet 4 (Bedrock)".to_string(),
                provider: "amazon-bedrock".to_string(),
                context_window: 200000,
                max_output_tokens: 16000,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 3.0,
                cost_per_million_output: 15.0,
            },
            ModelInfo {
                id: "anthropic.claude-3-5-sonnet-20241022-v2:0".to_string(),
                name: "Claude 3.5 Sonnet v2 (Bedrock)".to_string(),
                provider: "amazon-bedrock".to_string(),
                context_window: 200000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 3.0,
                cost_per_million_output: 15.0,
            },
            ModelInfo {
                id: "anthropic.claude-3-5-haiku-20241022-v1:0".to_string(),
                name: "Claude 3.5 Haiku (Bedrock)".to_string(),
                provider: "amazon-bedrock".to_string(),
                context_window: 200000,
                max_output_tokens: 8192,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 1.0,
                cost_per_million_output: 5.0,
            },
            ModelInfo {
                id: "anthropic.claude-3-opus-20240229-v1:0".to_string(),
                name: "Claude 3 Opus (Bedrock)".to_string(),
                provider: "amazon-bedrock".to_string(),
                context_window: 200000,
                max_output_tokens: 4096,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 15.0,
                cost_per_million_output: 75.0,
            },
            ModelInfo {
                id: "amazon.nova-pro-v1:0".to_string(),
                name: "Amazon Nova Pro (Bedrock)".to_string(),
                provider: "amazon-bedrock".to_string(),
                context_window: 300000,
                max_output_tokens: 4096,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.8,
                cost_per_million_output: 3.2,
            },
            ModelInfo {
                id: "amazon.nova-lite-v1:0".to_string(),
                name: "Amazon Nova Lite (Bedrock)".to_string(),
                provider: "amazon-bedrock".to_string(),
                context_window: 300000,
                max_output_tokens: 4096,
                supports_vision: true,
                supports_tools: true,
                cost_per_million_input: 0.06,
                cost_per_million_output: 0.24,
            },
            ModelInfo {
                id: "amazon.nova-micro-v1:0".to_string(),
                name: "Amazon Nova Micro (Bedrock)".to_string(),
                provider: "amazon-bedrock".to_string(),
                context_window: 128000,
                max_output_tokens: 4096,
                supports_vision: false,
                supports_tools: true,
                cost_per_million_input: 0.035,
                cost_per_million_output: 0.14,
            },
            ModelInfo {
                id: "meta.llama3-3-70b-instruct-v1:0".to_string(),
                name: "Llama 3.3 70B (Bedrock)".to_string(),
                provider: "amazon-bedrock".to_string(),
                context_window: 128000,
                max_output_tokens: 8192,
                supports_vision: false,
                supports_tools: true,
                cost_per_million_input: 0.72,
                cost_per_million_output: 0.72,
            },
        ];

        Self {
            client: Client::new(),
            config,
            models,
        }
    }

    fn get_endpoint(&self) -> String {
        if let Some(ref url) = self.config.endpoint_url {
            return url.clone();
        }
        BEDROCK_RUNTIME_URL.replace("{region}", &self.config.region)
    }

    fn convert_request(&self, request: ChatRequest) -> BedrockConverseRequest {
        let mut messages = Vec::new();
        let mut system = Vec::new();

        for msg in request.messages {
            match msg.role {
                Role::System => {
                    if let Content::Text(text) = msg.content {
                        system.push(BedrockSystemContent { text });
                    }
                }
                Role::User => {
                    let content = match msg.content {
                        Content::Text(t) => vec![BedrockContentBlock::text(&t)],
                        Content::Parts(parts) => parts
                            .iter()
                            .filter_map(|p| p.text.as_ref().map(|t| BedrockContentBlock::text(t)))
                            .collect(),
                    };
                    messages.push(BedrockMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
                Role::Assistant => {
                    let content = match msg.content {
                        Content::Text(t) => vec![BedrockContentBlock::text(&t)],
                        Content::Parts(parts) => parts
                            .iter()
                            .filter_map(|p| p.text.as_ref().map(|t| BedrockContentBlock::text(t)))
                            .collect(),
                    };
                    messages.push(BedrockMessage {
                        role: "assistant".to_string(),
                        content,
                    });
                }
                Role::Tool => {}
            }
        }

        BedrockConverseRequest {
            messages,
            system: if system.is_empty() {
                None
            } else {
                Some(system)
            },
            inference_config: Some(BedrockInferenceConfig {
                max_tokens: request.max_tokens,
                temperature: request.temperature,
            }),
        }
    }

    async fn sign_request(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<reqwest::header::HeaderMap, ProviderError> {
        let mut headers = reqwest::header::HeaderMap::new();

        let now = chrono::Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = now.format("%Y%m%d").to_string();

        let service = "bedrock";
        let region = &self.config.region;

        let body_hash = hex::encode(sha256(body));

        headers.insert("Content-Type", "application/json".parse().unwrap());
        headers.insert("X-Amz-Date", amz_date.parse().unwrap());
        headers.insert(
            "Host",
            format!("bedrock-runtime.{}.amazonaws.com", region)
                .parse()
                .unwrap(),
        );

        if let Some(ref token) = self.config.session_token {
            headers.insert("X-Amz-Security-Token", token.parse().unwrap());
        }

        let canonical_request =
            format!(
            "{}\n{}\n\nhost:bedrock-runtime.{}.amazonaws.com\nx-amz-date:{}\n\nhost;x-amz-date\n{}",
            method, path, region,
            headers.get("X-Amz-Date").unwrap().to_str().unwrap(),
            body_hash
        );

        let credential_scope = format!("{}/{}/{}/aws4_request", date_stamp, region, service);

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            headers.get("X-Amz-Date").unwrap().to_str().unwrap(),
            credential_scope,
            hex::encode(sha256(canonical_request.as_bytes()))
        );

        let signing_key =
            Self::get_signature_key(&self.config.secret_access_key, &date_stamp, region, service);

        let signature = hex::encode(Self::hmac_sha256(&signing_key, string_to_sign.as_bytes()));

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders=host;x-amz-date, Signature={}",
            self.config.access_key_id, credential_scope, signature
        );

        headers.insert("Authorization", authorization.parse().unwrap());

        Ok(headers)
    }

    fn get_signature_key(key: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
        let k_date = Self::hmac_sha256(format!("AWS4{}", key).as_bytes(), date.as_bytes());
        let k_region = Self::hmac_sha256(&k_date, region.as_bytes());
        let k_service = Self::hmac_sha256(&k_region, service.as_bytes());
        Self::hmac_sha256(&k_service, b"aws4_request")
    }

    fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }
}

fn sha256(data: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

#[async_trait]
impl Provider for BedrockProvider {
    fn id(&self) -> &str {
        "amazon-bedrock"
    }

    fn name(&self) -> &str {
        "Amazon Bedrock"
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
        let endpoint = self.get_endpoint();
        let model_id = request.model.clone();
        let model_id_encoded = urlencoding::encode(&model_id);

        let bedrock_request = self.convert_request(request);
        let body = serde_json::to_vec(&bedrock_request)
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        let headers = self
            .sign_request(
                "POST",
                &format!("/model/{}/converse", model_id_encoded),
                &body,
            )
            .await?;

        let url = format!("{}/model/{}/converse", endpoint, model_id_encoded);

        let response = client
            .post(&url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let bedrock_response: BedrockConverseResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_bedrock_response(bedrock_response))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let client =
            crate::session_client::for_session(request.session_id.as_deref().unwrap_or(""));
        let endpoint = self.get_endpoint();
        let model_id = request.model.clone();
        let model_id_encoded = urlencoding::encode(&model_id);

        let bedrock_request = self.convert_request(request);
        let body = serde_json::to_vec(&bedrock_request)
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        let headers = self
            .sign_request(
                "POST",
                &format!("/model/{}/converse-stream", model_id_encoded),
                &body,
            )
            .await?;

        let url = format!("{}/model/{}/converse-stream", endpoint, model_id_encoded);

        let response = client
            .post(&url)
            .headers(headers)
            .header("Accept", "application/vnd.amazon.eventstream")
            .body(body)
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
                    parse_bedrock_stream(&text)
                }
                Err(e) => Err(ProviderError::StreamError(e.to_string())),
            });

        Ok(Box::pin(stream))
    }
}

#[derive(Debug, Serialize)]
struct BedrockConverseRequest {
    messages: Vec<BedrockMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<BedrockSystemContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference_config: Option<BedrockInferenceConfig>,
}

#[derive(Debug, Serialize)]
struct BedrockMessage {
    role: String,
    content: Vec<BedrockContentBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BedrockContentBlock {
    text: String,
}

impl BedrockContentBlock {
    fn text(t: &str) -> Self {
        Self {
            text: t.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct BedrockSystemContent {
    text: String,
}

#[derive(Debug, Serialize)]
struct BedrockInferenceConfig {
    #[serde(rename = "maxTokens", skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct BedrockConverseResponse {
    output: BedrockOutput,
    usage: BedrockUsage,
}

#[derive(Debug, Deserialize)]
struct BedrockOutput {
    message: BedrockResponseMessage,
}

#[derive(Debug, Deserialize)]
struct BedrockResponseMessage {
    _role: String,
    content: Vec<BedrockContentBlock>,
}

#[derive(Debug, Deserialize)]
struct BedrockUsage {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: Option<u64>,
}

fn convert_bedrock_response(response: BedrockConverseResponse) -> ChatResponse {
    let content = response
        .output
        .message
        .content
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();

    ChatResponse {
        id: format!("bedrock_{}", uuid::Uuid::new_v4()),
        model: "amazon-bedrock".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message::assistant(&content),
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(Usage {
            prompt_tokens: response.usage.input_tokens,
            completion_tokens: response.usage.output_tokens,
            total_tokens: response
                .usage
                .total_tokens
                .unwrap_or_else(|| response.usage.input_tokens + response.usage.output_tokens),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        }),
    }
}

fn parse_bedrock_stream(text: &str) -> Result<StreamEvent, ProviderError> {
    if text.contains("\"contentBlockDelta\"") {
        if let Some(start) = text.find("\"text\":") {
            let rest = &text[start + 8..];
            if let Some(end) = rest.find("\"") {
                let text_content = &rest[..end];
                if let Ok(decoded) =
                    serde_json::from_str::<String>(&format!("\"{}\"", text_content))
                {
                    return Ok(StreamEvent::TextDelta(decoded));
                }
                return Ok(StreamEvent::TextDelta(text_content.to_string()));
            }
        }
    }

    if text.contains("\"messageStop\"") {
        return Ok(StreamEvent::Done);
    }

    Ok(StreamEvent::TextDelta(String::new()))
}
