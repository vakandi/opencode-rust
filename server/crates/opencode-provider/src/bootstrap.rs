use async_trait::async_trait;
use crate::anthropic::AnthropicProvider;
use crate::auth::AuthInfo;
use crate::azure::AzureProvider;
use crate::bedrock::BedrockProvider;
use crate::cerebras::CerebrasProvider;
use crate::cohere::CohereProvider;
use crate::deepinfra::DeepInfraProvider;
use crate::deepseek::DeepSeekProvider;
use crate::github_copilot::GitHubCopilotProvider;
use crate::gitlab::GitLabProvider;
use crate::google::GoogleProvider;
use crate::groq::GroqProvider;
use crate::mistral::MistralProvider;
use crate::models::{ModelInfo, ModelInterleaved, ModelsData, ProviderInfo as ModelsProviderInfo};
use crate::openai::OpenAIProvider;
use crate::openrouter::OpenRouterProvider;
use crate::perplexity::PerplexityProvider;
use crate::provider::{
    ModelInfo as RuntimeModelInfo, Provider as RuntimeProvider, ProviderRegistry,
};
use crate::together::TogetherProvider;
use crate::vercel::VercelProvider;
use crate::vertex::GoogleVertexProvider;
use crate::xai::XaiProvider;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Arc;
use tracing;

// ---------------------------------------------------------------------------
// Error types matching TS ModelNotFoundError and InitError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("Model not found: provider={provider_id} model={model_id}")]
    ModelNotFound {
        provider_id: String,
        model_id: String,
        suggestions: Vec<String>,
    },

    #[error("Provider initialization failed: {provider_id}")]
    InitError {
        provider_id: String,
        #[source]
        cause: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

// ---------------------------------------------------------------------------
// BUNDLED_PROVIDERS map (TS npm package -> provider name)
// ---------------------------------------------------------------------------

/// Map of bundled SDK package names to their provider identifiers.
/// Mirrors the TS `BUNDLED_PROVIDERS` record.
/* PLACEHOLDER_BUNDLED_PROVIDERS */
pub static BUNDLED_PROVIDERS: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("@ai-sdk/amazon-bedrock", "amazon-bedrock");
    m.insert("@ai-sdk/anthropic", "anthropic");
    m.insert("@ai-sdk/azure", "azure");
    m.insert("@ai-sdk/google", "google");
    m.insert("@ai-sdk/google-vertex", "google-vertex");
    m.insert("@ai-sdk/google-vertex/anthropic", "google-vertex-anthropic");
    m.insert("@ai-sdk/openai", "openai");
    m.insert("@ai-sdk/openai-compatible", "openai-compatible");
    m.insert("@openrouter/ai-sdk-provider", "openrouter");
    m.insert("@ai-sdk/xai", "xai");
    m.insert("@ai-sdk/mistral", "mistral");
    m.insert("@ai-sdk/groq", "groq");
    m.insert("@ai-sdk/deepinfra", "deepinfra");
    m.insert("@ai-sdk/cerebras", "cerebras");
    m.insert("@ai-sdk/cohere", "cohere");
    m.insert("@ai-sdk/gateway", "gateway");
    m.insert("@ai-sdk/togetherai", "togetherai");
    m.insert("@ai-sdk/perplexity", "perplexity");
    m.insert("@ai-sdk/vercel", "vercel");
    m.insert("@gitlab/gitlab-ai-provider", "gitlab");
    m.insert("@ai-sdk/github-copilot", "github-copilot");
    m
});

// ---------------------------------------------------------------------------
// Helper functions matching TS helpers
// ---------------------------------------------------------------------------

/// Check if a model ID represents GPT-5 or later.
pub fn is_gpt5_or_later(model_id: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^gpt-(\d+)").unwrap());
    if let Some(caps) = RE.captures(model_id) {
        if let Some(num) = caps.get(1) {
            if let Ok(n) = num.as_str().parse::<u32>() {
                return n >= 5;
            }
        }
    }
    false
}

/// Determine whether to use the Copilot responses API for a given model.
pub fn should_use_copilot_responses_api(model_id: &str) -> bool {
    is_gpt5_or_later(model_id) && !model_id.starts_with("gpt-5-mini")
}

// ---------------------------------------------------------------------------
// Provider.Model - the runtime model type (matches TS Provider.Model)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub temperature: bool,
    pub reasoning: bool,
    pub attachment: bool,
    pub toolcall: bool,
    pub input: ModalitySet,
    pub output: ModalitySet,
    pub interleaved: InterleavedConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModalitySet {
    pub text: bool,
    pub audio: bool,
    pub image: bool,
    pub video: bool,
    pub pdf: bool,
}

impl Default for ModalitySet {
    fn default() -> Self {
        Self {
            text: false,
            audio: false,
            image: false,
            video: false,
            pdf: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InterleavedConfig {
    Bool(bool),
    Field { field: String },
}

impl Default for InterleavedConfig {
    fn default() -> Self {
        InterleavedConfig::Bool(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCostCache {
    pub read: f64,
    pub write: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCostOver200K {
    pub input: f64,
    pub output: f64,
    pub cache: ModelCostCache,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelCost {
    pub input: f64,
    pub output: f64,
    pub cache: ModelCostCache,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_over_200k: Option<ModelCostOver200K>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelLimit {
    pub context: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    pub output: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelApi {
    pub id: String,
    pub url: String,
    pub npm: String,
}

/// Runtime model type matching TS `Provider.Model`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    pub provider_id: String,
    pub api: ProviderModelApi,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    pub capabilities: ModelCapabilities,
    pub cost: ProviderModelCost,
    pub limit: ProviderModelLimit,
    pub status: String,
    pub options: HashMap<String, serde_json::Value>,
    pub headers: HashMap<String, String>,
    pub release_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}

// ---------------------------------------------------------------------------
// Provider.Info - the runtime provider type (matches TS Provider.Info)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderState {
    pub id: String,
    pub name: String,
    pub source: String, // "env" | "config" | "custom" | "api"
    pub env: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub options: HashMap<String, serde_json::Value>,
    pub models: HashMap<String, ProviderModel>,
}

// ---------------------------------------------------------------------------
// CustomLoaderResult - result from a custom loader (matches TS CustomLoader return)
// ---------------------------------------------------------------------------

/// Result of a custom loader operation.
pub struct CustomLoaderResult {
    /// Whether the provider should be auto-loaded even without env/auth keys.
    pub autoload: bool,
    /// Options to merge into the provider.
    pub options: HashMap<String, serde_json::Value>,
    /// Whether this loader provides a custom getModel function.
    pub has_custom_get_model: bool,
    /// Models to add/override (legacy, kept for backward compat).
    pub models: HashMap<String, ModelInfo>,
    /// Headers to apply to all models (legacy).
    pub headers: HashMap<String, String>,
    /// Models to remove by ID pattern (legacy).
    pub blacklist: Vec<String>,
}

impl Default for CustomLoaderResult {
    fn default() -> Self {
        Self {
            autoload: false,
            options: HashMap::new(),
            has_custom_get_model: false,
            models: HashMap::new(),
            headers: HashMap::new(),
            blacklist: Vec::new(),
        }
    }
}

/// Trait for provider-specific model loading customization.
pub trait CustomLoader: Send + Sync {
    fn load(
        &self,
        provider: &ModelsProviderInfo,
        provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult;
}

// ---------------------------------------------------------------------------
// Custom loader implementations for all 14+ providers
// ---------------------------------------------------------------------------

/// Anthropic custom loader - adds correct beta headers.
struct AnthropicLoader;

impl CustomLoader for AnthropicLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.headers.insert(
            "anthropic-beta".to_string(),
            "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14".to_string(),
        );
        result
    }
}

/// OpenCode custom loader - checks API keys, filters paid models if no key.
struct OpenCodeLoader;

impl CustomLoader for OpenCodeLoader {
    fn load(
        &self,
        provider: &ModelsProviderInfo,
        provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let has_key = provider.env.iter().any(|e| std::env::var(e).is_ok())
            || provider_state
                .and_then(|state| {
                    provider_option_string(state, &["apiKey", "api_key", "apikey"])
                })
                .is_some();

        if !has_key {
            // Remove paid models (cost.input > 0)
            let paid_ids: Vec<String> = provider
                .models
                .iter()
                .filter(|(_, m)| m.cost.as_ref().map(|c| c.input > 0.0).unwrap_or(false))
                .map(|(id, _)| id.clone())
                .collect();
            for id in &paid_ids {
                result.blacklist.push(id.clone());
            }
        }

        let remaining = provider.models.len().saturating_sub(result.blacklist.len());
        result.autoload = remaining > 0;

        if !has_key {
            result.options.insert(
                "apiKey".to_string(),
                serde_json::Value::String("public".to_string()),
            );
        }

        result
    }
}

/// OpenAI custom loader - uses responses API.
struct OpenAILoader;

impl CustomLoader for OpenAILoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.has_custom_get_model = true;
        // Blacklist non-chat models
        result.blacklist.extend(vec![
            "whisper".to_string(),
            "tts".to_string(),
            "dall-e".to_string(),
            "embedding".to_string(),
            "moderation".to_string(),
        ]);
        result
    }
}

/// GitHub Copilot custom loader - conditional responses vs chat API.
struct GitHubCopilotLoader;

impl CustomLoader for GitHubCopilotLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.has_custom_get_model = true;
        result
    }
}

/// GitHub Copilot Enterprise custom loader - same as GitHub Copilot.
struct GitHubCopilotEnterpriseLoader;

impl CustomLoader for GitHubCopilotEnterpriseLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.has_custom_get_model = true;
        result
    }
}

/// Azure custom loader - conditional getModel based on useCompletionUrls.
struct AzureLoader;

impl CustomLoader for AzureLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.has_custom_get_model = true;
        result
    }
}

/// Azure Cognitive Services custom loader - resource name handling.
struct AzureCognitiveServicesLoader;

impl CustomLoader for AzureCognitiveServicesLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.has_custom_get_model = true;

        if let Ok(resource_name) = std::env::var("AZURE_COGNITIVE_SERVICES_RESOURCE_NAME") {
            result.options.insert(
                "baseURL".to_string(),
                serde_json::Value::String(format!(
                    "https://{}.cognitiveservices.azure.com/openai",
                    resource_name
                )),
            );
        }

        result
    }
}

/// Amazon Bedrock custom loader - the most complex loader.
/// Handles region resolution, AWS credential chain, cross-region model prefixing.
struct AmazonBedrockLoader;

impl AmazonBedrockLoader {
    fn provider_option_string(state: Option<&ProviderState>, keys: &[&str]) -> Option<String> {
        let state = state?;
        for key in keys {
            let Some(value) = state.options.get(*key) else {
                continue;
            };
            match value {
                serde_json::Value::String(s) if !s.trim().is_empty() => return Some(s.clone()),
                serde_json::Value::Number(n) => return Some(n.to_string()),
                serde_json::Value::Bool(b) => return Some(b.to_string()),
                _ => {}
            }
        }
        None
    }

    /// Apply cross-region model ID prefixing based on region.
    /// Returns the (possibly prefixed) model ID.
    // TODO: Wire for Bedrock cross-region routing
    #[allow(dead_code)]
    pub fn prefix_model_id(model_id: &str, region: &str) -> String {
        // Skip if model already has a cross-region inference profile prefix
        let cross_region_prefixes = ["global.", "us.", "eu.", "jp.", "apac.", "au."];
        if cross_region_prefixes
            .iter()
            .any(|p| model_id.starts_with(p))
        {
            return model_id.to_string();
        }

        let region_prefix = region.split('-').next().unwrap_or("");
        let mut result_id = model_id.to_string();

        match region_prefix {
            "us" => {
                let model_requires_prefix = [
                    "nova-micro",
                    "nova-lite",
                    "nova-pro",
                    "nova-premier",
                    "nova-2",
                    "claude",
                    "deepseek",
                ]
                .iter()
                .any(|m| model_id.contains(m));
                let is_gov_cloud = region.starts_with("us-gov");
                if model_requires_prefix && !is_gov_cloud {
                    result_id = format!("{}.{}", region_prefix, model_id);
                }
            }
            "eu" => {
                let region_requires_prefix = [
                    "eu-west-1",
                    "eu-west-2",
                    "eu-west-3",
                    "eu-north-1",
                    "eu-central-1",
                    "eu-south-1",
                    "eu-south-2",
                ]
                .iter()
                .any(|r| region.contains(r));
                let model_requires_prefix =
                    ["claude", "nova-lite", "nova-micro", "llama3", "pixtral"]
                        .iter()
                        .any(|m| model_id.contains(m));
                if region_requires_prefix && model_requires_prefix {
                    result_id = format!("{}.{}", region_prefix, model_id);
                }
            }
            "ap" => {
                let is_australia_region = region == "ap-southeast-2" || region == "ap-southeast-4";
                let is_tokyo_region = region == "ap-northeast-1";

                if is_australia_region
                    && ["anthropic.claude-sonnet-4-5", "anthropic.claude-haiku"]
                        .iter()
                        .any(|m| model_id.contains(m))
                {
                    result_id = format!("au.{}", model_id);
                } else if is_tokyo_region {
                    let model_requires_prefix = ["claude", "nova-lite", "nova-micro", "nova-pro"]
                        .iter()
                        .any(|m| model_id.contains(m));
                    if model_requires_prefix {
                        result_id = format!("jp.{}", model_id);
                    }
                } else {
                    // Other APAC regions use apac. prefix
                    let model_requires_prefix = ["claude", "nova-lite", "nova-micro", "nova-pro"]
                        .iter()
                        .any(|m| model_id.contains(m));
                    if model_requires_prefix {
                        result_id = format!("apac.{}", model_id);
                    }
                }
            }
            _ => {}
        }

        result_id
    }
}

impl CustomLoader for AmazonBedrockLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        // Region precedence: config options > env var > default.
        let region = Self::provider_option_string(provider_state, &["region"])
            .or_else(|| std::env::var("AWS_REGION").ok())
            .unwrap_or_else(|| "us-east-1".to_string());

        // Credential options from config or environment.
        let profile = Self::provider_option_string(provider_state, &["profile"])
            .or_else(|| std::env::var("AWS_PROFILE").ok());
        let endpoint = Self::provider_option_string(
            provider_state,
            &["endpoint", "endpointUrl", "endpointURL"],
        );

        let aws_access_key_id = Self::provider_option_string(provider_state, &["accessKeyId"])
            .or_else(|| std::env::var("AWS_ACCESS_KEY_ID").ok());
        let aws_secret_access_key =
            Self::provider_option_string(provider_state, &["secretAccessKey"])
                .or_else(|| std::env::var("AWS_SECRET_ACCESS_KEY").ok());
        let aws_bearer_token =
            Self::provider_option_string(provider_state, &["awsBearerTokenBedrock", "bearerToken"])
                .or_else(|| std::env::var("AWS_BEARER_TOKEN_BEDROCK").ok());
        let aws_web_identity_token_file =
            Self::provider_option_string(provider_state, &["webIdentityTokenFile"])
                .or_else(|| std::env::var("AWS_WEB_IDENTITY_TOKEN_FILE").ok());
        let container_creds = std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_ok()
            || std::env::var("AWS_CONTAINER_CREDENTIALS_FULL_URI").is_ok();

        if profile.is_none()
            && aws_access_key_id.is_none()
            && aws_secret_access_key.is_none()
            && aws_bearer_token.is_none()
            && aws_web_identity_token_file.is_none()
            && !container_creds
        {
            result.autoload = false;
            return result;
        }

        result.autoload = true;
        result
            .options
            .insert("region".to_string(), serde_json::Value::String(region));
        if let Some(profile) = profile {
            result
                .options
                .insert("profile".to_string(), serde_json::Value::String(profile));
        }
        if let Some(endpoint) = endpoint {
            result
                .options
                .insert("endpoint".to_string(), serde_json::Value::String(endpoint));
        }
        result.has_custom_get_model = true;

        result
    }
}

/// OpenRouter custom loader - adds custom headers.
struct OpenRouterLoader;

impl CustomLoader for OpenRouterLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.headers.insert(
            "HTTP-Referer".to_string(),
            "https://opencode.ai/".to_string(),
        );
        result
            .headers
            .insert("X-Title".to_string(), "opencode".to_string());
        result
    }
}

/// ZenMux custom loader - same branding headers as OpenRouter.
struct ZenMuxLoader;

impl CustomLoader for ZenMuxLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.headers.insert(
            "HTTP-Referer".to_string(),
            "https://opencode.ai/".to_string(),
        );
        result
            .headers
            .insert("X-Title".to_string(), "opencode".to_string());
        result
    }
}

/// Vercel custom loader - adds custom headers.
struct VercelLoader;

impl CustomLoader for VercelLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.headers.insert(
            "http-referer".to_string(),
            "https://opencode.ai/".to_string(),
        );
        result
            .headers
            .insert("x-title".to_string(), "opencode".to_string());
        result
    }
}

/// Google Vertex custom loader - project/location env var resolution.
struct GoogleVertexLoader;

impl CustomLoader for GoogleVertexLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let project = std::env::var("GOOGLE_CLOUD_PROJECT")
            .or_else(|_| std::env::var("GCP_PROJECT"))
            .or_else(|_| std::env::var("GCLOUD_PROJECT"))
            .ok();
        let location = std::env::var("GOOGLE_CLOUD_LOCATION")
            .or_else(|_| std::env::var("VERTEX_LOCATION"))
            .unwrap_or_else(|_| "us-east5".to_string());

        if let Some(ref proj) = project {
            result.autoload = true;
            result.options.insert(
                "project".to_string(),
                serde_json::Value::String(proj.clone()),
            );
            result
                .options
                .insert("location".to_string(), serde_json::Value::String(location));
            result.has_custom_get_model = true;
        }

        result
    }
}

/// Google Vertex Anthropic custom loader - similar to google-vertex.
struct GoogleVertexAnthropicLoader;

impl CustomLoader for GoogleVertexAnthropicLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let project = std::env::var("GOOGLE_CLOUD_PROJECT")
            .or_else(|_| std::env::var("GCP_PROJECT"))
            .or_else(|_| std::env::var("GCLOUD_PROJECT"))
            .ok();
        let location = std::env::var("GOOGLE_CLOUD_LOCATION")
            .or_else(|_| std::env::var("VERTEX_LOCATION"))
            .unwrap_or_else(|_| "global".to_string());

        if let Some(ref proj) = project {
            result.autoload = true;
            result.options.insert(
                "project".to_string(),
                serde_json::Value::String(proj.clone()),
            );
            result
                .options
                .insert("location".to_string(), serde_json::Value::String(location));
            result.has_custom_get_model = true;
        }

        result
    }
}

/// SAP AI Core custom loader - service key and deployment ID.
struct SapAiCoreLoader;

impl CustomLoader for SapAiCoreLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let env_service_key = std::env::var("AICORE_SERVICE_KEY").ok();
        result.autoload = env_service_key.is_some();

        if env_service_key.is_some() {
            if let Ok(deployment_id) = std::env::var("AICORE_DEPLOYMENT_ID") {
                result.options.insert(
                    "deploymentId".to_string(),
                    serde_json::Value::String(deployment_id),
                );
            }
            if let Ok(resource_group) = std::env::var("AICORE_RESOURCE_GROUP") {
                result.options.insert(
                    "resourceGroup".to_string(),
                    serde_json::Value::String(resource_group),
                );
            }
        }
        result.has_custom_get_model = true;

        result
    }
}

/// GitLab custom loader - instance URL, auth type, User-Agent, feature flags.
struct GitLabLoader;

impl CustomLoader for GitLabLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let instance_url = std::env::var("GITLAB_INSTANCE_URL")
            .unwrap_or_else(|_| "https://gitlab.com".to_string());
        let api_key = std::env::var("GITLAB_TOKEN").ok();

        result.autoload = api_key.is_some();

        result.options.insert(
            "instanceUrl".to_string(),
            serde_json::Value::String(instance_url),
        );
        if let Some(key) = api_key {
            result
                .options
                .insert("apiKey".to_string(), serde_json::Value::String(key));
        }

        // User-Agent header
        let user_agent = format!(
            "opencode/0.1.0 gitlab-ai-provider/0.1.0 ({} {}; {})",
            std::env::consts::OS,
            "unknown",
            std::env::consts::ARCH,
        );
        let mut ai_gateway_headers = HashMap::new();
        ai_gateway_headers.insert(
            "User-Agent".to_string(),
            serde_json::Value::String(user_agent),
        );
        result.options.insert(
            "aiGatewayHeaders".to_string(),
            serde_json::to_value(ai_gateway_headers).unwrap_or_default(),
        );

        // Feature flags
        let mut feature_flags = HashMap::new();
        feature_flags.insert(
            "duo_agent_platform_agentic_chat".to_string(),
            serde_json::Value::Bool(true),
        );
        feature_flags.insert(
            "duo_agent_platform".to_string(),
            serde_json::Value::Bool(true),
        );
        result.options.insert(
            "featureFlags".to_string(),
            serde_json::to_value(feature_flags).unwrap_or_default(),
        );

        result.has_custom_get_model = true;

        result
    }
}

/// Cloudflare Workers AI custom loader - account ID and API key.
struct CloudflareWorkersAiLoader;

impl CustomLoader for CloudflareWorkersAiLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let account_id = std::env::var("CLOUDFLARE_ACCOUNT_ID").ok();
        if account_id.is_none() {
            result.autoload = false;
            return result;
        }
        let account_id = account_id.unwrap();

        let api_key = std::env::var("CLOUDFLARE_API_KEY").ok();
        result.autoload = api_key.is_some();

        if let Some(key) = api_key {
            result
                .options
                .insert("apiKey".to_string(), serde_json::Value::String(key));
        }
        result.options.insert(
            "baseURL".to_string(),
            serde_json::Value::String(format!(
                "https://api.cloudflare.com/client/v4/accounts/{}/ai/v1",
                account_id
            )),
        );
        result.has_custom_get_model = true;

        result
    }
}

/// Cloudflare AI Gateway custom loader - account ID, gateway ID, API token.
struct CloudflareAiGatewayLoader;

impl CustomLoader for CloudflareAiGatewayLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();

        let account_id = std::env::var("CLOUDFLARE_ACCOUNT_ID").ok();
        let gateway = std::env::var("CLOUDFLARE_GATEWAY_ID").ok();

        if account_id.is_none() || gateway.is_none() {
            result.autoload = false;
            return result;
        }

        let api_token = std::env::var("CLOUDFLARE_API_TOKEN")
            .or_else(|_| std::env::var("CF_AIG_TOKEN"))
            .ok();

        result.autoload = api_token.is_some();

        if let Some(ref token) = api_token {
            result.options.insert(
                "apiKey".to_string(),
                serde_json::Value::String(token.clone()),
            );
        }
        if let Some(ref acc) = account_id {
            result.options.insert(
                "accountId".to_string(),
                serde_json::Value::String(acc.clone()),
            );
        }
        if let Some(ref gw) = gateway {
            result
                .options
                .insert("gateway".to_string(), serde_json::Value::String(gw.clone()));
        }
        result.has_custom_get_model = true;

        result
    }
}

/// Cerebras custom loader - adds custom header.
struct CerebrasLoader;

impl CustomLoader for CerebrasLoader {
    fn load(
        &self,
        _provider: &ModelsProviderInfo,
        _provider_state: Option<&ProviderState>,
    ) -> CustomLoaderResult {
        let mut result = CustomLoaderResult::default();
        result.headers.insert(
            "X-Cerebras-3rd-Party-Integration".to_string(),
            "opencode".to_string(),
        );
        result
    }
}

/// Get the custom loader for a provider ID.
fn get_custom_loader(provider_id: &str) -> Option<Box<dyn CustomLoader>> {
    match provider_id {
        "anthropic" => Some(Box::new(AnthropicLoader)),
        "opencode" => Some(Box::new(OpenCodeLoader)),
        "openai" => Some(Box::new(OpenAILoader)),
        "github-copilot" => Some(Box::new(GitHubCopilotLoader)),
        "github-copilot-enterprise" => Some(Box::new(GitHubCopilotEnterpriseLoader)),
        "azure" => Some(Box::new(AzureLoader)),
        "azure-cognitive-services" => Some(Box::new(AzureCognitiveServicesLoader)),
        "amazon-bedrock" => Some(Box::new(AmazonBedrockLoader)),
        "openrouter" => Some(Box::new(OpenRouterLoader)),
        "zenmux" => Some(Box::new(ZenMuxLoader)),
        "vercel" => Some(Box::new(VercelLoader)),
        "google-vertex" => Some(Box::new(GoogleVertexLoader)),
        "google-vertex-anthropic" => Some(Box::new(GoogleVertexAnthropicLoader)),
        "sap-ai-core" => Some(Box::new(SapAiCoreLoader)),
        "gitlab" => Some(Box::new(GitLabLoader)),
        "cloudflare-workers-ai" => Some(Box::new(CloudflareWorkersAiLoader)),
        "cloudflare-ai-gateway" => Some(Box::new(CloudflareAiGatewayLoader)),
        "cerebras" => Some(Box::new(CerebrasLoader)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Transform helpers: from_models_dev_model / from_models_dev_provider
// ---------------------------------------------------------------------------

/// Endpoint that serves opencode zen's free models without a credential.
const DEFAULT_ZEN_BASE_URL: &str = "https://opencode.ai/zen/v1";

/// Transform a models.dev model into a runtime ProviderModel.
pub fn from_models_dev_model(provider: &ModelsProviderInfo, model: &ModelInfo) -> ProviderModel {
    let modalities_input = model
        .modalities
        .as_ref()
        .map(|m| &m.input)
        .cloned()
        .unwrap_or_default();
    let modalities_output = model
        .modalities
        .as_ref()
        .map(|m| &m.output)
        .cloned()
        .unwrap_or_default();

    let interleaved = match model.interleaved.as_ref() {
        Some(ModelInterleaved::Bool(value)) => InterleavedConfig::Bool(*value),
        Some(ModelInterleaved::Field { field }) => InterleavedConfig::Field {
            field: field.clone(),
        },
        None => InterleavedConfig::Bool(false),
    };

    let cost = model.cost.as_ref();
    let over_200k = cost.and_then(|c| c.context_over_200k.as_ref());

    let mut variants = crate::transform::variants(model);
    if let Some(explicit_variants) = &model.variants {
        for (variant_name, options) in explicit_variants {
            variants.insert(variant_name.clone(), options.clone());
        }
    }

    ProviderModel {
        id: model.id.clone(),
        provider_id: provider.id.clone(),
        name: model.name.clone(),
        family: model.family.clone(),
        api: ProviderModelApi {
            id: model.id.clone(),
            url: model
                .provider
                .as_ref()
                .and_then(|p| p.api.clone())
                .or_else(|| provider.api.clone())
                .unwrap_or_default(),
            npm: model
                .provider
                .as_ref()
                .and_then(|p| p.npm.clone())
                .or_else(|| provider.npm.clone())
                .unwrap_or_else(|| "@ai-sdk/openai-compatible".to_string()),
        },
        status: model.status.clone().unwrap_or_else(|| "active".to_string()),
        headers: model.headers.clone().unwrap_or_default(),
        options: model.options.clone(),
        cost: ProviderModelCost {
            input: cost.map(|c| c.input).unwrap_or(0.0),
            output: cost.map(|c| c.output).unwrap_or(0.0),
            cache: ModelCostCache {
                read: cost.and_then(|c| c.cache_read).unwrap_or(0.0),
                write: cost.and_then(|c| c.cache_write).unwrap_or(0.0),
            },
            experimental_over_200k: over_200k.map(|o| ModelCostOver200K {
                input: o.input,
                output: o.output,
                cache: ModelCostCache {
                    read: o.cache_read.unwrap_or(0.0),
                    write: o.cache_write.unwrap_or(0.0),
                },
            }),
        },
        limit: ProviderModelLimit {
            context: model.limit.context,
            input: model.limit.input,
            output: model.limit.output,
        },
        capabilities: ModelCapabilities {
            temperature: model.temperature,
            reasoning: model.reasoning,
            attachment: model.attachment,
            toolcall: model.tool_call,
            input: ModalitySet {
                text: modalities_input.contains(&"text".to_string()),
                audio: modalities_input.contains(&"audio".to_string()),
                image: modalities_input.contains(&"image".to_string()),
                video: modalities_input.contains(&"video".to_string()),
                pdf: modalities_input.contains(&"pdf".to_string()),
            },
            output: ModalitySet {
                text: modalities_output.contains(&"text".to_string()),
                audio: modalities_output.contains(&"audio".to_string()),
                image: modalities_output.contains(&"image".to_string()),
                video: modalities_output.contains(&"video".to_string()),
                pdf: modalities_output.contains(&"pdf".to_string()),
            },
            interleaved,
        },
        release_date: model.release_date.clone().unwrap_or_default(),
        variants: if variants.is_empty() {
            None
        } else {
            Some(variants)
        },
    }
}

/// Transform a models.dev provider into a runtime ProviderState.
pub fn from_models_dev_provider(provider: &ModelsProviderInfo) -> ProviderState {
    let models = provider
        .models
        .iter()
        .map(|(id, model)| (id.clone(), from_models_dev_model(provider, model)))
        .collect();

    ProviderState {
        id: provider.id.clone(),
        source: "custom".to_string(),
        name: provider.name.clone(),
        env: provider.env.clone(),
        key: None,
        options: HashMap::new(),
        models,
    }
}

// ---------------------------------------------------------------------------
// ProviderBootstrapConfig - configuration input for initialization
// ---------------------------------------------------------------------------

/// Configuration for a single model from the config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModel {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub temperature: Option<bool>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub attachment: Option<bool>,
    #[serde(default)]
    pub tool_call: Option<bool>,
    #[serde(default)]
    pub interleaved: Option<bool>,
    #[serde(default)]
    pub cost: Option<ConfigModelCost>,
    #[serde(default)]
    pub limit: Option<ConfigModelLimit>,
    #[serde(default)]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub modalities: Option<ConfigModalities>,
    #[serde(default)]
    pub provider: Option<ConfigModelProvider>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub variants: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModelCost {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModelLimit {
    #[serde(default)]
    pub context: Option<u64>,
    #[serde(default)]
    pub output: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModalities {
    #[serde(default)]
    pub input: Option<Vec<String>>,
    #[serde(default)]
    pub output: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigModelProvider {
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub api: Option<String>,
}

/// Configuration for a single provider from the config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigProvider {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub env: Option<Vec<String>>,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub models: Option<HashMap<String, ConfigModel>>,
    #[serde(default)]
    pub blacklist: Option<Vec<String>>,
    #[serde(default)]
    pub whitelist: Option<Vec<String>>,
}

/// Top-level bootstrap configuration.
#[derive(Debug, Clone, Default)]
pub struct BootstrapConfig {
    /// Provider configs from opencode.json
    pub providers: HashMap<String, ConfigProvider>,
    /// Disabled provider IDs
    pub disabled_providers: HashSet<String>,
    /// Enabled provider IDs (if set, only these are allowed)
    pub enabled_providers: Option<HashSet<String>>,
    /// Whether to enable experimental/alpha models
    pub enable_experimental: bool,
    /// The configured model string (e.g. "anthropic/claude-sonnet-4")
    pub model: Option<String>,
    /// The configured small model string
    pub small_model: Option<String>,
}

// ---------------------------------------------------------------------------
// ProviderBootstrapState - the initialized state with all providers/models
// ---------------------------------------------------------------------------

/// The initialized provider state, analogous to the TS `state()` return value.
pub struct ProviderBootstrapState {
    pub providers: HashMap<String, ProviderState>,
    /// Provider IDs that have custom getModel loaders.
    pub model_loaders: HashSet<String>,
}

impl ProviderBootstrapState {
    /// Initialize the provider bootstrap state from models.dev data and config.
    /// This is the Rust equivalent of the TS `state()` function.
    pub fn init(
        models_dev: &ModelsData,
        config: &BootstrapConfig,
        auth_store: &HashMap<String, AuthInfo>,
    ) -> Self {
        let mut database: HashMap<String, ProviderState> = models_dev
            .iter()
            .map(|(id, p)| (id.clone(), from_models_dev_provider(p)))
            .collect();

        let disabled = &config.disabled_providers;
        let enabled = &config.enabled_providers;

        let mut providers: HashMap<String, ProviderState> = HashMap::new();
        let mut model_loaders: HashSet<String> = HashSet::new();

        // Add GitHub Copilot Enterprise provider that inherits from GitHub Copilot
        if let Some(github_copilot) = database.get("github-copilot").cloned() {
            let mut enterprise = github_copilot.clone();
            enterprise.id = "github-copilot-enterprise".to_string();
            enterprise.name = "GitHub Copilot Enterprise".to_string();
            for model in enterprise.models.values_mut() {
                model.provider_id = "github-copilot-enterprise".to_string();
            }
            database.insert("github-copilot-enterprise".to_string(), enterprise);
        }

        // Helper closure to merge a partial update into providers
        let merge_provider = |providers: &mut HashMap<String, ProviderState>,
                              database: &HashMap<String, ProviderState>,
                              provider_id: &str,
                              patch: ProviderPatch| {
            if let Some(existing) = providers.get_mut(provider_id) {
                apply_patch(existing, patch);
            } else if let Some(base) = database.get(provider_id) {
                let mut merged = base.clone();
                apply_patch(&mut merged, patch);
                providers.insert(provider_id.to_string(), merged);
            }
        };

        // Extend database from config providers
        for (provider_id, cfg_provider) in &config.providers {
            let existing = database.get(provider_id);
            let mut parsed = ProviderState {
                id: provider_id.clone(),
                name: cfg_provider
                    .name
                    .clone()
                    .or_else(|| existing.map(|e| e.name.clone()))
                    .unwrap_or_else(|| provider_id.clone()),
                env: cfg_provider
                    .env
                    .clone()
                    .or_else(|| existing.map(|e| e.env.clone()))
                    .unwrap_or_default(),
                options: merge_json_maps(
                    existing.map(|e| &e.options).unwrap_or(&HashMap::new()),
                    cfg_provider.options.as_ref().unwrap_or(&HashMap::new()),
                ),
                source: "config".to_string(),
                key: None,
                models: existing.map(|e| e.models.clone()).unwrap_or_default(),
            };

            // Process config model overrides
            if let Some(cfg_models) = &cfg_provider.models {
                for (model_id, cfg_model) in cfg_models {
                    let existing_model = parsed
                        .models
                        .get(&cfg_model.id.clone().unwrap_or_else(|| model_id.clone()));
                    let pm = config_to_provider_model(
                        provider_id,
                        model_id,
                        cfg_model,
                        existing_model,
                        cfg_provider,
                        models_dev.get(provider_id),
                    );
                    parsed.models.insert(model_id.clone(), pm);
                }
            }

            database.insert(provider_id.clone(), parsed);
        }

        // Load from env vars
        for (provider_id, provider) in &database {
            if disabled.contains(provider_id) {
                continue;
            }
            let api_key = provider.env.iter().find_map(|e| std::env::var(e).ok());

            // A provider is usable either because it has a credential, or because its custom
            // loader says it can serve without one. opencode zen is the second kind: it
            // publishes free models that need no key, so keying activation solely off an env
            // var made `opencode/space-bunny-free` unresolvable ("Provider not found:
            // opencode") even though the loader computed `autoload` to express exactly this
            // and nothing ever read it.
            let keyless = if api_key.is_none() {
                match (get_custom_loader(provider_id), models_dev.get(provider_id)) {
                    (Some(loader), Some(info)) => Some(loader.load(info, None)),
                    _ => None,
                }
            } else {
                None
            };
            let autoload_keyless = keyless.as_ref().is_some_and(|result| result.autoload);

            if api_key.is_some() || autoload_keyless {
                let key_val = if provider.env.len() == 1 {
                    std::env::var(&provider.env[0]).ok()
                } else {
                    None
                };
                merge_provider(
                    &mut providers,
                    &database,
                    provider_id,
                    ProviderPatch {
                        source: Some("env".to_string()),
                        key: key_val,
                        ..Default::default()
                    },
                );

                // `ProviderPatch` cannot carry a blacklist, so drop the loader-excluded models
                // (the paid ones, when unauthenticated) straight from the merged state.
                // Without this the provider resolves and then 402s on every paid model.
                if let Some(result) = keyless {
                    if !result.blacklist.is_empty() {
                        if let Some(merged) = providers.get_mut(provider_id) {
                            for id in &result.blacklist {
                                merged.models.remove(id);
                            }
                        }
                    }
                }
            }
        }

        // Load from auth store
        for (provider_id, auth) in auth_store {
            if disabled.contains(provider_id) {
                continue;
            }
            let maybe_key = match auth {
                AuthInfo::Api { key } => Some(key.clone()),
                AuthInfo::OAuth { access, .. } => Some(access.clone()),
                AuthInfo::WellKnown { token, .. } => Some(token.clone()),
            };
            if let Some(key) = maybe_key {
                merge_provider(
                    &mut providers,
                    &database,
                    provider_id,
                    ProviderPatch {
                        source: Some("api".to_string()),
                        key: Some(key),
                        ..Default::default()
                    },
                );
            }
        }

        // Apply custom loaders
        for (provider_id, data) in &database {
            if disabled.contains(provider_id) {
                continue;
            }
            if let Some(loader) = get_custom_loader(provider_id) {
                // Build a ModelsProviderInfo for the loader
                let models_provider = to_models_provider_info(data, models_dev.get(provider_id));
                let result = loader.load(&models_provider, Some(data));

                if result.autoload || providers.contains_key(provider_id) {
                    if result.has_custom_get_model {
                        model_loaders.insert(provider_id.clone());
                    }

                    let patch = ProviderPatch {
                        source: if providers.contains_key(provider_id) {
                            None
                        } else {
                            Some("custom".to_string())
                        },
                        options: if result.options.is_empty() {
                            None
                        } else {
                            Some(result.options)
                        },
                        ..Default::default()
                    };
                    merge_provider(&mut providers, &database, provider_id, patch);

                    // Apply headers from loader to all models
                    if !result.headers.is_empty() {
                        if let Some(p) = providers.get_mut(provider_id) {
                            for model in p.models.values_mut() {
                                for (k, v) in &result.headers {
                                    model.headers.insert(k.clone(), v.clone());
                                }
                            }
                        }
                    }

                    // Apply blacklist
                    if !result.blacklist.is_empty() {
                        if let Some(p) = providers.get_mut(provider_id) {
                            p.models.retain(|mid, _| {
                                let lower = mid.to_lowercase();
                                !result.blacklist.iter().any(|pat| lower.contains(pat))
                            });
                        }
                    }
                }
            }
        }

        // Re-apply config overrides (source, env, name, options)
        for (provider_id, cfg_provider) in &config.providers {
            let mut patch = ProviderPatch {
                source: Some("config".to_string()),
                ..Default::default()
            };
            if let Some(ref env) = cfg_provider.env {
                patch.env = Some(env.clone());
            }
            if let Some(ref name) = cfg_provider.name {
                patch.name = Some(name.clone());
            }
            if let Some(ref opts) = cfg_provider.options {
                patch.options = Some(opts.clone());
            }
            merge_provider(&mut providers, &database, provider_id, patch);
        }

        // Filter and clean up providers
        let is_provider_allowed = |pid: &str| -> bool {
            if let Some(ref en) = enabled {
                if !en.contains(pid) {
                    return false;
                }
            }
            !disabled.contains(pid)
        };

        let provider_ids: Vec<String> = providers.keys().cloned().collect();
        for provider_id in provider_ids {
            if !is_provider_allowed(&provider_id) {
                providers.remove(&provider_id);
                continue;
            }

            let cfg_provider = config.providers.get(&provider_id);

            if let Some(provider) = providers.get_mut(&provider_id) {
                let model_ids: Vec<String> = provider.models.keys().cloned().collect();
                for model_id in model_ids {
                    let should_remove = {
                        let model = &provider.models[&model_id];

                        // Remove gpt-5-chat-latest
                        if model_id == "gpt-5-chat-latest" {
                            true
                        } else if provider_id == "openrouter" && model_id == "openai/gpt-5-chat" {
                            true
                        }
                        // Remove alpha models unless experimental enabled
                        else if model.status == "alpha" && !config.enable_experimental {
                            true
                        }
                        // Remove deprecated models
                        else if model.status == "deprecated" {
                            true
                        }
                        // Apply blacklist/whitelist from config
                        else if let Some(cfg) = cfg_provider {
                            if let Some(ref bl) = cfg.blacklist {
                                if bl.contains(&model_id) {
                                    true
                                } else if let Some(ref wl) = cfg.whitelist {
                                    !wl.contains(&model_id)
                                } else {
                                    false
                                }
                            } else if let Some(ref wl) = cfg.whitelist {
                                !wl.contains(&model_id)
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    };

                    if should_remove {
                        provider.models.remove(&model_id);
                    }
                }

                // Remove providers with no models
                if provider.models.is_empty() {
                    providers.remove(&provider_id);
                }
            }
        }

        ProviderBootstrapState {
            providers,
            model_loaders,
        }
    }

    // -----------------------------------------------------------------------
    // Query functions matching TS Provider namespace exports
    // -----------------------------------------------------------------------

    /// Return all providers.
    pub fn list(&self) -> &HashMap<String, ProviderState> {
        &self.providers
    }

    /// Get a provider by ID.
    pub fn get_provider(&self, provider_id: &str) -> Option<&ProviderState> {
        self.providers.get(provider_id)
    }

    /// Get a model by provider ID and model ID, with fuzzy matching on failure.
    pub fn get_model(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<&ProviderModel, BootstrapError> {
        let provider = self.providers.get(provider_id).ok_or_else(|| {
            let available: Vec<String> = self.providers.keys().cloned().collect();
            let suggestions = fuzzy_match(provider_id, &available, 3);
            BootstrapError::ModelNotFound {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                suggestions,
            }
        })?;

        provider.models.get(model_id).ok_or_else(|| {
            let available: Vec<String> = provider.models.keys().cloned().collect();
            let suggestions = fuzzy_match(model_id, &available, 3);
            BootstrapError::ModelNotFound {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                suggestions,
            }
        })
    }

    /// Find the closest matching model for a provider given a list of query strings.
    pub fn closest(&self, provider_id: &str, queries: &[&str]) -> Option<(String, String)> {
        let provider = self.providers.get(provider_id)?;
        for query in queries {
            for model_id in provider.models.keys() {
                if model_id.contains(query) {
                    return Some((provider_id.to_string(), model_id.clone()));
                }
            }
        }
        None
    }

    /// Get the small model for a provider, using priority lists.
    pub fn get_small_model(
        &self,
        provider_id: &str,
        config_small_model: Option<&str>,
    ) -> Option<ProviderModel> {
        // If config specifies a small model, use it
        if let Some(model_str) = config_small_model {
            let parsed = parse_model(model_str);
            return self
                .get_model(&parsed.provider_id, &parsed.model_id)
                .ok()
                .cloned();
        }

        if let Some(provider) = self.providers.get(provider_id) {
            let mut priority: Vec<&str> = vec![
                "claude-haiku-4-5",
                "claude-haiku-4.5",
                "3-5-haiku",
                "3.5-haiku",
                "gemini-3-flash",
                "gemini-2.5-flash",
                "gpt-5-nano",
            ];

            if provider_id.starts_with("opencode") {
                priority = vec!["gpt-5-nano"];
            }
            if provider_id.starts_with("github-copilot") {
                priority = vec!["gpt-5-mini", "claude-haiku-4.5"];
                priority.extend_from_slice(&[
                    "claude-haiku-4-5",
                    "3-5-haiku",
                    "3.5-haiku",
                    "gemini-3-flash",
                    "gemini-2.5-flash",
                    "gpt-5-nano",
                ]);
            }

            for item in &priority {
                if provider_id == "amazon-bedrock" {
                    let cross_region_prefixes = ["global.", "us.", "eu."];
                    let candidates: Vec<&String> = provider
                        .models
                        .keys()
                        .filter(|m| m.contains(item))
                        .collect();

                    // Priority: 1) global. 2) user's region prefix 3) unprefixed
                    if let Some(global_match) = candidates.iter().find(|m| m.starts_with("global."))
                    {
                        return provider.models.get(*global_match).cloned();
                    }

                    let region = provider
                        .options
                        .get("region")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let region_prefix = region.split('-').next().unwrap_or("");
                    if region_prefix == "us" || region_prefix == "eu" {
                        if let Some(regional) = candidates
                            .iter()
                            .find(|m| m.starts_with(&format!("{}.", region_prefix)))
                        {
                            return provider.models.get(*regional).cloned();
                        }
                    }

                    if let Some(unprefixed) = candidates
                        .iter()
                        .find(|m| !cross_region_prefixes.iter().any(|p| m.starts_with(p)))
                    {
                        return provider.models.get(*unprefixed).cloned();
                    }
                } else {
                    for model_id in provider.models.keys() {
                        if model_id.contains(item) {
                            return provider.models.get(model_id).cloned();
                        }
                    }
                }
            }
        }

        // Fallback: check opencode provider for gpt-5-nano
        if let Some(opencode_provider) = self.providers.get("opencode") {
            if let Some(model) = opencode_provider.models.get("gpt-5-nano") {
                return Some(model.clone());
            }
        }

        None
    }

    /// Sort models by priority (matching TS sort function).
    pub fn sort_models(models: &mut Vec<ProviderModel>) {
        let priority_list = ["gpt-5", "claude-sonnet-4", "big-pickle", "gemini-3-pro"];

        models.sort_by(|a, b| {
            let a_pri = priority_list
                .iter()
                .position(|p| a.id.contains(p))
                .map(|i| -(i as i64))
                .unwrap_or(i64::MAX);
            let b_pri = priority_list
                .iter()
                .position(|p| b.id.contains(p))
                .map(|i| -(i as i64))
                .unwrap_or(i64::MAX);

            a_pri
                .cmp(&b_pri)
                .then_with(|| {
                    let a_latest = if a.id.contains("latest") { 0 } else { 1 };
                    let b_latest = if b.id.contains("latest") { 0 } else { 1 };
                    a_latest.cmp(&b_latest)
                })
                .then_with(|| b.id.cmp(&a.id))
        });
    }

    /// Get the default model from config or first available provider.
    pub fn default_model(
        &self,
        config_model: Option<&str>,
        recent: &[(String, String)],
    ) -> Option<ParsedModel> {
        if let Some(model_str) = config_model {
            return Some(parse_model(model_str));
        }

        // Check recent models
        for (provider_id, model_id) in recent {
            if let Some(provider) = self.providers.get(provider_id) {
                if provider.models.contains_key(model_id) {
                    return Some(ParsedModel {
                        provider_id: provider_id.clone(),
                        model_id: model_id.clone(),
                    });
                }
            }
        }

        // Fall back to first provider, sorted models
        let provider = self.providers.values().next()?;
        let mut models: Vec<ProviderModel> = provider.models.values().cloned().collect();
        Self::sort_models(&mut models);
        let model = models.first()?;
        Some(ParsedModel {
            provider_id: provider.id.clone(),
            model_id: model.id.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// ParsedModel and parse_model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ParsedModel {
    pub provider_id: String,
    pub model_id: String,
}

/// Parse a "provider/model" format string.
pub fn parse_model(model_str: &str) -> ParsedModel {
    if let Some(pos) = model_str.find('/') {
        ParsedModel {
            provider_id: model_str[..pos].to_string(),
            model_id: model_str[pos + 1..].to_string(),
        }
    } else {
        ParsedModel {
            provider_id: model_str.to_string(),
            model_id: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ProviderPatch - partial update for ProviderState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ProviderPatch {
    pub source: Option<String>,
    pub name: Option<String>,
    pub env: Option<Vec<String>>,
    pub key: Option<String>,
    pub options: Option<HashMap<String, serde_json::Value>>,
}

/// Apply a partial patch to a ProviderState, merging fields that are `Some`.
fn apply_patch(state: &mut ProviderState, patch: ProviderPatch) {
    if let Some(source) = patch.source {
        state.source = source;
    }
    if let Some(name) = patch.name {
        state.name = name;
    }
    if let Some(env) = patch.env {
        state.env = env;
    }
    if let Some(key) = patch.key {
        state.key = Some(key);
    }
    if let Some(options) = patch.options {
        for (k, v) in options {
            state.options.insert(k, v);
        }
    }
}

/// Merge two JSON option maps, with `overlay` values taking precedence.
fn merge_json_maps(
    base: &HashMap<String, serde_json::Value>,
    overlay: &HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    let mut result = base.clone();
    for (k, v) in overlay {
        result.insert(k.clone(), v.clone());
    }
    result
}

/// Convert a ConfigModel (from user config) into a ProviderModel, using an
/// optional existing model as the base for defaults.
fn config_to_provider_model(
    provider_id: &str,
    model_id: &str,
    cfg: &ConfigModel,
    existing: Option<&ProviderModel>,
    cfg_provider: &ConfigProvider,
    models_provider: Option<&ModelsProviderInfo>,
) -> ProviderModel {
    let api_model_id = cfg.id.clone().unwrap_or_else(|| model_id.to_string());

    let default_npm = cfg_provider
        .npm
        .clone()
        .or_else(|| models_provider.and_then(|p| p.npm.clone()))
        .unwrap_or_else(|| "@ai-sdk/openai-compatible".to_string());
    let default_api = cfg_provider
        .api
        .clone()
        .or_else(|| models_provider.and_then(|p| p.api.clone()))
        .unwrap_or_default();

    let base_cost = existing.map(|e| &e.cost);
    let base_limit = existing.map(|e| &e.limit);
    let base_caps = existing.map(|e| &e.capabilities);

    let cost = {
        let cfg_cost = cfg.cost.as_ref();
        ProviderModelCost {
            input: cfg_cost
                .and_then(|c| c.input)
                .or_else(|| base_cost.map(|c| c.input))
                .unwrap_or(0.0),
            output: cfg_cost
                .and_then(|c| c.output)
                .or_else(|| base_cost.map(|c| c.output))
                .unwrap_or(0.0),
            cache: ModelCostCache {
                read: cfg_cost
                    .and_then(|c| c.cache_read)
                    .or_else(|| base_cost.map(|c| c.cache.read))
                    .unwrap_or(0.0),
                write: cfg_cost
                    .and_then(|c| c.cache_write)
                    .or_else(|| base_cost.map(|c| c.cache.write))
                    .unwrap_or(0.0),
            },
            experimental_over_200k: base_cost.and_then(|c| c.experimental_over_200k.clone()),
        }
    };

    let limit = ProviderModelLimit {
        context: cfg
            .limit
            .as_ref()
            .and_then(|l| l.context)
            .or_else(|| base_limit.map(|l| l.context))
            .unwrap_or(128000),
        input: base_limit.and_then(|l| l.input),
        output: cfg
            .limit
            .as_ref()
            .and_then(|l| l.output)
            .or_else(|| base_limit.map(|l| l.output))
            .unwrap_or(4096),
    };

    let modalities_input = cfg
        .modalities
        .as_ref()
        .and_then(|m| m.input.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            if base_caps.map(|c| c.input.text).unwrap_or(true) {
                vec!["text".to_string()]
            } else {
                vec![]
            }
        });
    let modalities_output = cfg
        .modalities
        .as_ref()
        .and_then(|m| m.output.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            if base_caps.map(|c| c.output.text).unwrap_or(true) {
                vec!["text".to_string()]
            } else {
                vec![]
            }
        });

    let interleaved = match cfg.interleaved {
        Some(v) => InterleavedConfig::Bool(v),
        None => existing
            .map(|e| e.capabilities.interleaved.clone())
            .unwrap_or_default(),
    };

    let options = merge_json_maps(
        &existing.map(|e| e.options.clone()).unwrap_or_default(),
        cfg.options.as_ref().unwrap_or(&HashMap::new()),
    );
    let headers = merge_string_maps(
        &existing.map(|e| e.headers.clone()).unwrap_or_default(),
        cfg.headers.as_ref().unwrap_or(&HashMap::new()),
    );

    ProviderModel {
        id: model_id.to_string(),
        provider_id: provider_id.to_string(),
        name: cfg.name.clone().unwrap_or_else(|| {
            if cfg.id.as_deref().is_some_and(|id| id != model_id) {
                model_id.to_string()
            } else {
                existing
                    .map(|e| e.name.clone())
                    .unwrap_or_else(|| model_id.to_string())
            }
        }),
        family: cfg
            .family
            .clone()
            .or_else(|| existing.and_then(|e| e.family.clone())),
        api: ProviderModelApi {
            id: api_model_id,
            url: cfg
                .provider
                .as_ref()
                .and_then(|p| p.api.clone())
                .or_else(|| existing.map(|e| e.api.url.clone()))
                .unwrap_or_else(|| default_api.clone()),
            npm: cfg
                .provider
                .as_ref()
                .and_then(|p| p.npm.clone())
                .or_else(|| existing.map(|e| e.api.npm.clone()))
                .unwrap_or_else(|| default_npm.clone()),
        },

        status: cfg
            .status
            .clone()
            .or_else(|| existing.map(|e| e.status.clone()))
            .unwrap_or_else(|| "active".to_string()),
        cost,
        limit,
        capabilities: ModelCapabilities {
            temperature: cfg
                .temperature
                .or_else(|| base_caps.map(|c| c.temperature))
                .unwrap_or(true),
            reasoning: cfg
                .reasoning
                .or_else(|| base_caps.map(|c| c.reasoning))
                .unwrap_or(false),
            attachment: cfg
                .attachment
                .or_else(|| base_caps.map(|c| c.attachment))
                .unwrap_or(false),
            toolcall: cfg
                .tool_call
                .or_else(|| base_caps.map(|c| c.toolcall))
                .unwrap_or(true),
            input: ModalitySet {
                text: modalities_input.contains(&"text".to_string()),
                audio: modalities_input.contains(&"audio".to_string()),
                image: modalities_input.contains(&"image".to_string()),
                video: modalities_input.contains(&"video".to_string()),
                pdf: modalities_input.contains(&"pdf".to_string()),
            },
            output: ModalitySet {
                text: modalities_output.contains(&"text".to_string()),
                audio: modalities_output.contains(&"audio".to_string()),
                image: modalities_output.contains(&"image".to_string()),
                video: modalities_output.contains(&"video".to_string()),
                pdf: modalities_output.contains(&"pdf".to_string()),
            },
            interleaved,
        },
        options,
        headers,
        release_date: cfg
            .release_date
            .clone()
            .or_else(|| existing.map(|e| e.release_date.clone()))
            .unwrap_or_default(),
        variants: cfg
            .variants
            .clone()
            .or_else(|| existing.and_then(|e| e.variants.clone())),
    }
}

/// Merge two String->String maps, with `overlay` values taking precedence.
fn merge_string_maps(
    base: &HashMap<String, String>,
    overlay: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut result = base.clone();
    for (k, v) in overlay {
        result.insert(k.clone(), v.clone());
    }
    result
}

/// Convert a ProviderState back to a ModelsProviderInfo for use by custom loaders.
/// Falls back to the original models.dev data when available.
fn to_models_provider_info(
    state: &ProviderState,
    original: Option<&ModelsProviderInfo>,
) -> ModelsProviderInfo {
    // If we have the original models.dev data, prefer it (loaders expect that shape).
    if let Some(orig) = original {
        return orig.clone();
    }

    // Otherwise, reconstruct a minimal ModelsProviderInfo from the runtime state.
    let models = state
        .models
        .iter()
        .map(|(id, pm)| {
            let mi = ModelInfo {
                id: pm.id.clone(),
                name: pm.name.clone(),
                family: pm.family.clone(),
                release_date: Some(pm.release_date.clone()),
                attachment: pm.capabilities.attachment,
                reasoning: pm.capabilities.reasoning,
                temperature: pm.capabilities.temperature,
                tool_call: pm.capabilities.toolcall,
                interleaved: match &pm.capabilities.interleaved {
                    InterleavedConfig::Bool(b) => Some(ModelInterleaved::Bool(*b)),
                    InterleavedConfig::Field { field } => Some(ModelInterleaved::Field {
                        field: field.clone(),
                    }),
                },
                cost: Some(crate::models::ModelCost {
                    input: pm.cost.input,
                    output: pm.cost.output,
                    cache_read: Some(pm.cost.cache.read),
                    cache_write: Some(pm.cost.cache.write),
                    context_over_200k: None,
                }),
                limit: crate::models::ModelLimit {
                    context: pm.limit.context,
                    input: pm.limit.input,
                    output: pm.limit.output,
                },
                modalities: None,
                experimental: None,
                status: Some(pm.status.clone()),
                options: pm.options.clone(),
                headers: if pm.headers.is_empty() {
                    None
                } else {
                    Some(pm.headers.clone())
                },
                provider: Some(crate::models::ModelProvider {
                    npm: Some(pm.api.npm.clone()),
                    api: Some(pm.api.url.clone()),
                }),
                variants: pm.variants.clone(),
            };
            (id.clone(), mi)
        })
        .collect();

    ModelsProviderInfo {
        id: state.id.clone(),
        name: state.name.clone(),
        env: state.env.clone(),
        api: None,
        npm: None,
        models,
    }
}

/// Simple fuzzy string matching: returns up to `max` candidates from `options`
/// that share a common substring with `query`, sorted by edit-distance-like score.
fn fuzzy_match(query: &str, options: &[String], max: usize) -> Vec<String> {
    let query_lower = query.to_lowercase();
    let mut scored: Vec<(usize, &String)> = options
        .iter()
        .filter_map(|opt| {
            let opt_lower = opt.to_lowercase();
            // Score: length of longest common substring (simple heuristic)
            let score = longest_common_substring_len(&query_lower, &opt_lower);
            if score >= 2 || opt_lower.contains(&query_lower) || query_lower.contains(&opt_lower) {
                Some((score, opt))
            } else {
                None
            }
        })
        .collect();

    // Sort descending by score
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(max)
        .map(|(_, s)| s.clone())
        .collect()
}

/// Length of the longest common substring between two strings.
fn longest_common_substring_len(a: &str, b: &str) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut max_len = 0;
    // Simple O(n*m) DP approach
    let mut prev = vec![0usize; b_bytes.len() + 1];
    for i in 1..=a_bytes.len() {
        let mut curr = vec![0usize; b_bytes.len() + 1];
        for j in 1..=b_bytes.len() {
            if a_bytes[i - 1] == b_bytes[j - 1] {
                curr[j] = prev[j - 1] + 1;
                if curr[j] > max_len {
                    max_len = curr[j];
                }
            }
        }
        prev = curr;
    }
    max_len
}

// ---------------------------------------------------------------------------
// Public API functions exported from lib.rs
// ---------------------------------------------------------------------------

fn env_any(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn provider_option_string(provider: &ProviderState, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = provider.options.get(*key) else {
            continue;
        };
        match value {
            serde_json::Value::String(s) if !s.trim().is_empty() => return Some(s.clone()),
            serde_json::Value::Number(n) => return Some(n.to_string()),
            serde_json::Value::Bool(b) => return Some(b.to_string()),
            _ => {}
        }
    }
    None
}

fn provider_secret(provider: &ProviderState, fallback_env: &[&str]) -> Option<String> {
    provider_option_string(provider, &["apiKey", "api_key", "apikey"])
        .or_else(|| provider.key.clone().filter(|k| !k.trim().is_empty()))
        .or_else(|| {
            provider
                .env
                .iter()
                .find_map(|name| std::env::var(name).ok())
                .filter(|k| !k.trim().is_empty())
        })
        .or_else(|| env_any(fallback_env))
}

fn provider_base_url(provider: &ProviderState) -> Option<String> {
    provider_option_string(provider, &["baseURL", "baseUrl", "url", "api"]).or_else(|| {
        provider
            .models
            .values()
            .find_map(|model| (!model.api.url.trim().is_empty()).then(|| model.api.url.clone()))
    })
    .or_else(|| {
        // GLM Coding Plan requires a dedicated endpoint instead of the generic API.
        // TS users commonly configure this as provider id `zhipuai-coding-plan`.
        if provider.id == "zhipuai-coding-plan" {
            Some("https://open.bigmodel.cn/api/coding/paas/v4".to_string())
        } else {
            None
        }
    })
}

fn create_concrete_provider(
    provider_id: &str,
    provider: &ProviderState,
) -> Option<Arc<dyn RuntimeProvider>> {
    match provider_id {
        "anthropic" => {
            let api_key = provider_secret(provider, &["ANTHROPIC_API_KEY"])?;
            Some(Arc::new(AnthropicProvider::new(api_key)))
        }
        "openai" => {
            let api_key = provider_secret(provider, &["OPENAI_API_KEY"])?;
            if let Some(base_url) = provider_option_string(provider, &["baseURL", "baseUrl"])
                .or_else(|| env_any(&["OPENAI_BASE_URL"]))
            {
                Some(Arc::new(OpenAIProvider::new_with_base_url(
                    api_key, base_url,
                )))
            } else {
                Some(Arc::new(OpenAIProvider::new(api_key)))
            }
        }
        "opencode" => {
            // opencode zen serves its free models with no credential at all, so neither the
            // key nor the base URL may be required here: both used `?`, which returned None
            // and dropped the provider entirely ("Provider not found: opencode") whenever
            // OPENCODE_API_KEY was unset — even though `POST /zen/v1/chat/completions`
            // answers unauthenticated for models like space-bunny-free.
            let api_key = provider_secret(provider, &["OPENCODE_API_KEY"]).unwrap_or_default();
            let base_url = provider_base_url(provider)
                .unwrap_or_else(|| DEFAULT_ZEN_BASE_URL.to_string());
            // Never route Zen through openai_compatible(): that constructor carries no
            // fingerprint, so a stored key silently downgraded every free run to a 403
            // FreeTierError. The key is not what makes Zen answer.
            Some(Arc::new(OpenAIProvider::zen_with_key(
                base_url,
                if api_key.is_empty() { "public".to_string() } else { api_key },
            )))
        }
        "google" => {
            let api_key = provider_secret(
                provider,
                &["GOOGLE_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY"],
            )?;
            Some(Arc::new(GoogleProvider::new(api_key)))
        }
        "azure" => {
            let api_key = provider_secret(provider, &["AZURE_API_KEY", "AZURE_OPENAI_API_KEY"])?;
            let endpoint =
                provider_option_string(provider, &["endpoint", "baseURL", "baseUrl", "url"])
                    .or_else(|| env_any(&["AZURE_ENDPOINT", "AZURE_OPENAI_ENDPOINT"]))?;
            Some(Arc::new(AzureProvider::new(api_key, endpoint)))
        }
        "amazon-bedrock" => {
            let region = provider_option_string(provider, &["region"])
                .or_else(|| env_any(&["AWS_REGION"]))
                .unwrap_or_else(|| "us-east-1".to_string());
            let access_key_id = provider_option_string(provider, &["accessKeyId"])
                .or_else(|| env_any(&["AWS_ACCESS_KEY_ID"]))?;
            let secret_access_key = provider_option_string(provider, &["secretAccessKey"])
                .or_else(|| env_any(&["AWS_SECRET_ACCESS_KEY"]))?;
            let session_token = provider_option_string(provider, &["sessionToken"])
                .or_else(|| env_any(&["AWS_SESSION_TOKEN"]));
            let endpoint_url = provider_option_string(
                provider,
                &["endpointUrl", "endpointURL", "baseURL", "baseUrl"],
            );

            Some(Arc::new(BedrockProvider::with_config(
                crate::bedrock::BedrockConfig {
                    region,
                    access_key_id,
                    secret_access_key,
                    session_token,
                    endpoint_url,
                },
            )))
        }
        "openrouter" => {
            let api_key = provider_secret(provider, &["OPENROUTER_API_KEY"])?;
            Some(Arc::new(OpenRouterProvider::new(api_key)))
        }
        "mistral" => {
            let api_key = provider_secret(provider, &["MISTRAL_API_KEY"])?;
            Some(Arc::new(MistralProvider::new(api_key)))
        }
        "groq" => {
            let api_key = provider_secret(provider, &["GROQ_API_KEY"])?;
            Some(Arc::new(GroqProvider::new(api_key)))
        }
        "deepinfra" => {
            let api_key = provider_secret(provider, &["DEEPINFRA_API_KEY"])?;
            Some(Arc::new(DeepInfraProvider::new(api_key)))
        }
        "deepseek" => {
            let api_key = provider_secret(provider, &["DEEPSEEK_API_KEY"])?;
            Some(Arc::new(DeepSeekProvider::new(api_key)))
        }
        "xai" => {
            let api_key = provider_secret(provider, &["XAI_API_KEY"])?;
            Some(Arc::new(XaiProvider::new(api_key)))
        }
        "cerebras" => {
            let api_key = provider_secret(provider, &["CEREBRAS_API_KEY"])?;
            Some(Arc::new(CerebrasProvider::new(api_key)))
        }
        "cohere" => {
            let api_key = provider_secret(provider, &["COHERE_API_KEY"])?;
            Some(Arc::new(CohereProvider::new(api_key)))
        }
        "together" | "togetherai" => {
            let api_key = provider_secret(provider, &["TOGETHER_API_KEY", "TOGETHERAI_API_KEY"])?;
            Some(Arc::new(TogetherProvider::new(api_key)))
        }
        "perplexity" => {
            let api_key = provider_secret(provider, &["PERPLEXITY_API_KEY"])?;
            Some(Arc::new(PerplexityProvider::new(api_key)))
        }
        "vercel" => {
            let api_key = provider_secret(provider, &["VERCEL_API_KEY"])?;
            Some(Arc::new(VercelProvider::new(api_key)))
        }
        "gitlab" => {
            let api_key = provider_secret(provider, &["GITLAB_TOKEN"])?;
            Some(Arc::new(GitLabProvider::new(api_key)))
        }
        "github-copilot" => {
            let token = provider_secret(provider, &["GITHUB_COPILOT_TOKEN"])?;
            Some(Arc::new(GitHubCopilotProvider::new(token)))
        }
        "google-vertex" => {
            let access_token = provider_option_string(provider, &["accessToken", "token"])
                .or_else(|| {
                    env_any(&[
                        "GOOGLE_VERTEX_ACCESS_TOKEN",
                        "GOOGLE_CLOUD_ACCESS_TOKEN",
                        "GOOGLE_OAUTH_ACCESS_TOKEN",
                        "GCP_ACCESS_TOKEN",
                    ])
                })?;
            let project_id = provider_option_string(provider, &["project", "projectId"])
                .or_else(|| env_any(&["GOOGLE_CLOUD_PROJECT", "GCP_PROJECT", "GCLOUD_PROJECT"]))?;
            let location = provider_option_string(provider, &["location"])
                .or_else(|| env_any(&["GOOGLE_CLOUD_LOCATION", "VERTEX_LOCATION"]))
                .unwrap_or_else(|| "us-east5".to_string());
            Some(Arc::new(GoogleVertexProvider::new(
                access_token,
                project_id,
                location,
            )))
        }
        _ => {
            // Fallback for custom providers declared as OpenAI-compatible.
            let is_openai_compatible = provider.models.values().any(|model| {
                model
                    .api
                    .npm
                    .to_ascii_lowercase()
                    .contains("openai-compatible")
            });
            if !is_openai_compatible {
                return None;
            }

            let api_key = provider_secret(provider, &[])?;
            let base_url = provider_base_url(provider)?;
            Some(Arc::new(OpenAIProvider::openai_compatible(base_url, api_key)))
        }
    }
}

struct AliasedProvider {
    id: String,
    name: String,
    inner: Arc<dyn RuntimeProvider>,
    models: Vec<RuntimeModelInfo>,
    model_index: HashMap<String, RuntimeModelInfo>,
}

impl AliasedProvider {
    fn new(
        id: String,
        name: String,
        inner: Arc<dyn RuntimeProvider>,
        models: Vec<RuntimeModelInfo>,
    ) -> Self {
        let model_index = models
            .iter()
            .map(|model| (model.id.clone(), model.clone()))
            .collect();
        Self {
            id,
            name,
            inner,
            models,
            model_index,
        }
    }
}

#[async_trait]
impl RuntimeProvider for AliasedProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn models(&self) -> Vec<RuntimeModelInfo> {
        self.models.clone()
    }

    fn get_model(&self, id: &str) -> Option<&RuntimeModelInfo> {
        self.model_index.get(id)
    }

    async fn chat(&self, request: crate::ChatRequest) -> Result<crate::ChatResponse, crate::ProviderError> {
        self.inner.chat(request).await
    }

    async fn chat_stream(
        &self,
        request: crate::ChatRequest,
    ) -> Result<crate::StreamResult, crate::ProviderError> {
        self.inner.chat_stream(request).await
    }
}

fn state_model_to_runtime(provider_id: &str, model: &ProviderModel) -> RuntimeModelInfo {
    RuntimeModelInfo {
        id: model.id.clone(),
        name: model.name.clone(),
        provider: provider_id.to_string(),
        context_window: model.limit.context,
        max_output_tokens: model.limit.output,
        supports_vision: model.capabilities.input.image
            || model.capabilities.output.image
            || model.capabilities.input.video
            || model.capabilities.output.video,
        supports_tools: model.capabilities.toolcall,
        cost_per_million_input: model.cost.input,
        cost_per_million_output: model.cost.output,
    }
}

fn wrap_provider_for_state(
    provider_state: &ProviderState,
    provider: Arc<dyn RuntimeProvider>,
) -> Arc<dyn RuntimeProvider> {
    let should_wrap = provider_state.id != provider.id()
        || provider_state.name != provider.name()
        || !provider_state.models.is_empty();

    if !should_wrap {
        return provider;
    }

    let models = if provider_state.models.is_empty() {
        provider.models()
    } else {
        provider_state
            .models
            .values()
            .map(|model| state_model_to_runtime(&provider_state.id, model))
            .collect()
    };

    Arc::new(AliasedProvider::new(
        provider_state.id.clone(),
        provider_state.name.clone(),
        provider,
        models,
    ))
}

/// How long a cached models.dev catalogue is served before a refresh is attempted.
const MODELS_CACHE_TTL_SECS: u64 = 12 * 60 * 60;

/// The path this process owns and writes. Reading prefers an existing file elsewhere
/// (the TypeScript server's location) so an already-populated cache is adopted, but the
/// refresh always writes here.
fn models_cache_path() -> std::path::PathBuf {
    if let Some(dir) = dirs::cache_dir() {
        return dir.join("opencode").join("models.json");
    }
    std::env::temp_dir().join("opencode").join("models.json")
}

/// Refresh in the background. Never blocks the boot and never blanks a working
/// catalogue: a failed fetch leaves the existing cache exactly as it was.
fn refresh_models_cache_in_background() {
    tokio::spawn(async {
        let path = models_cache_path();
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, "models.dev refresh skipped: client build failed");
                return;
            }
        };
        let url = format!("{}/api.json", crate::models::MODELS_DEV_URL);
        match client.get(&url).header("User-Agent", "opencode-rust").send().await {
            Ok(response) if response.status().is_success() => match response.text().await {
                Ok(text) => match serde_json::from_str::<ModelsData>(&text) {
                    Ok(_) => {
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        match std::fs::write(&path, &text) {
                            Ok(()) => tracing::info!(path = %path.display(), "models.dev catalogue refreshed"),
                            Err(error) => tracing::warn!(%error, "models.dev refresh could not be written"),
                        }
                    }
                    Err(error) => tracing::warn!(%error, "models.dev response did not parse; keeping the old cache"),
                },
                Err(error) => tracing::warn!(%error, "models.dev body could not be read"),
            },
            Ok(response) => tracing::warn!(status = %response.status(), "models.dev refresh returned non-success"),
            Err(error) => tracing::warn!(%error, "models.dev refresh failed; keeping the old cache"),
        }
    });
}

fn load_models_dev_cache() -> ModelsData {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(dir) = dirs::cache_dir() {
        candidates.push(dir.join("opencode").join("models.json"));
    }
    // The TypeScript server writes this cache to the XDG location (`~/.cache/opencode`), while
    // `dirs::cache_dir()` resolves to `~/Library/Caches` on macOS. Reading only the platform
    // path therefore returned an empty catalogue: every provider failed to register and the
    // registry silently fell back to env-key-only, so keyless providers such as opencode zen
    // reported "Provider not found".
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        candidates.push(std::path::PathBuf::from(xdg).join("opencode").join("models.json"));
    }
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(
            std::path::PathBuf::from(home)
                .join(".cache")
                .join("opencode")
                .join("models.json"),
        );
    }
    candidates.push(std::env::temp_dir().join("opencode").join("models.json"));

    let Some(cache_path) = candidates.iter().find(|path| path.is_file()) else {
        tracing::warn!(
            searched = ?candidates,
            "models.dev catalogue not found; fetching it in the background"
        );
        refresh_models_cache_in_background();
        return HashMap::new();
    };
    let Ok(raw) = fs::read_to_string(cache_path) else {
        refresh_models_cache_in_background();
        return HashMap::new();
    };

    let stale = std::fs::metadata(cache_path)
        .and_then(|meta| meta.modified())
        .map(|modified| {
            modified
                .elapsed()
                .map(|age| age.as_secs() > MODELS_CACHE_TTL_SECS)
                .unwrap_or(true)
        })
        .unwrap_or(true);
    if stale {
        tracing::info!("models.dev cache is older than {MODELS_CACHE_TTL_SECS}s; refreshing in the background");
        refresh_models_cache_in_background();
    }

    if let Ok(parsed) = serde_json::from_str::<ModelsData>(&raw) {
        return parsed;
    }

    // Fallback: tolerate per-provider schema drift instead of dropping everything.
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return HashMap::new();
    };
    let Some(map) = value.as_object() else {
        return HashMap::new();
    };

    let mut data = HashMap::new();
    for (provider_id, provider_value) in map {
        match serde_json::from_value::<ModelsProviderInfo>(provider_value.clone()) {
            Ok(mut provider) => {
                if provider.id.trim().is_empty() {
                    provider.id = provider_id.clone();
                }
                data.insert(provider_id.clone(), provider);
            }
            Err(error) => {
                tracing::debug!(
                    provider = provider_id,
                    %error,
                    "Skipping invalid provider entry from models.dev cache"
                );
            }
        }
    }

    data
}

fn register_fallback_env_providers(registry: &mut ProviderRegistry) {
    let fallback: Vec<(&str, Vec<&str>)> = vec![
        ("anthropic", vec!["ANTHROPIC_API_KEY"]),
        ("openai", vec!["OPENAI_API_KEY"]),
        (
            "google",
            vec!["GOOGLE_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY"],
        ),
        ("azure", vec!["AZURE_API_KEY", "AZURE_OPENAI_API_KEY"]),
        (
            "amazon-bedrock",
            vec!["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"],
        ),
        ("openrouter", vec!["OPENROUTER_API_KEY"]),
        ("mistral", vec!["MISTRAL_API_KEY"]),
        ("groq", vec!["GROQ_API_KEY"]),
        ("deepseek", vec!["DEEPSEEK_API_KEY"]),
        ("xai", vec!["XAI_API_KEY"]),
        ("cerebras", vec!["CEREBRAS_API_KEY"]),
        ("cohere", vec!["COHERE_API_KEY"]),
        ("deepinfra", vec!["DEEPINFRA_API_KEY"]),
        ("together", vec!["TOGETHER_API_KEY", "TOGETHERAI_API_KEY"]),
        ("perplexity", vec!["PERPLEXITY_API_KEY"]),
        ("vercel", vec!["VERCEL_API_KEY"]),
        ("gitlab", vec!["GITLAB_TOKEN"]),
        ("github-copilot", vec!["GITHUB_COPILOT_TOKEN"]),
        (
            "google-vertex",
            vec![
                "GOOGLE_VERTEX_ACCESS_TOKEN",
                "GOOGLE_CLOUD_ACCESS_TOKEN",
                "GOOGLE_OAUTH_ACCESS_TOKEN",
                "GCP_ACCESS_TOKEN",
            ],
        ),
    ];

    for (provider_id, env_keys) in fallback {
        let state = ProviderState {
            id: provider_id.to_string(),
            name: provider_id.to_string(),
            source: "env".to_string(),
            env: env_keys.into_iter().map(|k| k.to_string()).collect(),
            key: None,
            options: HashMap::new(),
            models: HashMap::new(),
        };
        if let Some(provider) = create_concrete_provider(provider_id, &state) {
            registry.register_arc(provider);
        }
    }
}

/// Create a ProviderRegistry populated from environment variables.
/// Scans known provider env vars and registers any that are configured.
pub fn create_registry_from_env() -> ProviderRegistry {
    let auth_store: HashMap<String, AuthInfo> = HashMap::new();
    create_registry_from_env_with_auth_store(&auth_store)
}

/// Create a ProviderRegistry populated from environment variables plus explicit
/// auth store entries (for example plugin-provided auth tokens).
pub fn create_registry_from_env_with_auth_store(
    auth_store: &HashMap<String, AuthInfo>,
) -> ProviderRegistry {
    bootstrap_registry(&BootstrapConfig::default(), auth_store)
}

/// Create a ProviderRegistry using the given bootstrap config and auth store.
/// This is the primary entry point when you have a loaded application config
/// whose provider/model fields have been converted into a `BootstrapConfig`.
pub fn create_registry_from_bootstrap_config(
    config: &BootstrapConfig,
    auth_store: &HashMap<String, AuthInfo>,
) -> ProviderRegistry {
    bootstrap_registry(config, auth_store)
}

fn bootstrap_registry(
    config: &BootstrapConfig,
    auth_store: &HashMap<String, AuthInfo>,
) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();

    let models_dev = load_models_dev_cache();
    let state = ProviderBootstrapState::init(&models_dev, config, auth_store);

    for (provider_id, provider_state) in &state.providers {
        if let Some(provider) = create_concrete_provider(provider_id, provider_state) {
            let provider = wrap_provider_for_state(provider_state, provider);
            let registered_id = provider.id().to_string();
            registry.register_arc(provider);
            if !provider_state.options.is_empty() {
                registry.merge_config(&registered_id, provider_state.options.clone());
            }
            tracing::debug!(
                provider = provider_id,
                concrete_provider = registered_id,
                "Registered provider from bootstrap state"
            );
        } else {
            tracing::debug!(
                provider = provider_id,
                "No concrete provider implementation for bootstrap provider"
            );
        }
    }

    if registry.list().is_empty() {
        tracing::debug!(
            "No providers registered from bootstrap state, falling back to direct env registration"
        );
        register_fallback_env_providers(&mut registry);
    }

    registry
}

/// Build a `BootstrapConfig` from the raw config fields typically found in
/// `opencode_config::Config`. This bridges the gap between the config loader
/// and the provider bootstrap system.
///
/// The `providers` map should be converted from `opencode_config::ProviderConfig`
/// to `ConfigProvider` by the caller (see `config_provider_to_bootstrap` helper).
pub fn bootstrap_config_from_raw(
    providers: HashMap<String, ConfigProvider>,
    disabled_providers: Vec<String>,
    enabled_providers: Vec<String>,
    model: Option<String>,
    small_model: Option<String>,
) -> BootstrapConfig {
    BootstrapConfig {
        providers,
        disabled_providers: disabled_providers.into_iter().collect(),
        enabled_providers: if enabled_providers.is_empty() {
            None
        } else {
            Some(enabled_providers.into_iter().collect())
        },
        enable_experimental: false,
        model,
        small_model,
    }
}

/// Apply custom loaders to models data, mutating it in place.
/// This runs each provider's custom loader and applies blacklists, headers,
/// and option overrides.
pub fn apply_custom_loaders(data: &mut ModelsData) {
    let provider_ids: Vec<String> = data.keys().cloned().collect();

    for provider_id in &provider_ids {
        if let Some(loader) = get_custom_loader(provider_id) {
            let provider_info = match data.get(provider_id) {
                Some(p) => p.clone(),
                None => continue,
            };
            let result = loader.load(&provider_info, None);

            // Apply blacklist: remove models matching any blacklist pattern
            if !result.blacklist.is_empty() {
                if let Some(provider) = data.get_mut(provider_id) {
                    provider.models.retain(|mid, _| {
                        let lower = mid.to_lowercase();
                        !result.blacklist.iter().any(|pat| lower.contains(pat))
                    });
                }
            }

            // Apply headers to all models
            if !result.headers.is_empty() {
                if let Some(provider) = data.get_mut(provider_id) {
                    for model in provider.models.values_mut() {
                        let headers = model.headers.get_or_insert_with(HashMap::new);
                        for (k, v) in &result.headers {
                            headers.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
    }
}

/// Filter models by status, removing deprecated models and optionally alpha models.
pub fn filter_models_by_status(data: &mut ModelsData, enable_experimental: bool) {
    for provider in data.values_mut() {
        provider.models.retain(|_mid, model| {
            let status = model.status.as_deref().unwrap_or("active");
            // Always remove deprecated
            if status == "deprecated" {
                return false;
            }
            // Remove alpha unless experimental is enabled
            if status == "alpha" && !enable_experimental {
                return false;
            }
            true
        });
    }
    // Remove providers with no remaining models
    data.retain(|_pid, provider| !provider.models.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ModelLimit, ModelModalities, ModelProvider};

    fn provider_model(model_id: &str) -> ProviderModel {
        ProviderModel {
            id: model_id.to_string(),
            provider_id: "test".to_string(),
            name: model_id.to_string(),
            api: ProviderModelApi {
                id: model_id.to_string(),
                url: "https://example.com".to_string(),
                npm: "@ai-sdk/openai".to_string(),
            },
            family: None,
            capabilities: ModelCapabilities {
                temperature: true,
                reasoning: true,
                attachment: false,
                toolcall: true,
                input: ModalitySet {
                    text: true,
                    audio: false,
                    image: false,
                    video: false,
                    pdf: false,
                },
                output: ModalitySet {
                    text: true,
                    audio: false,
                    image: false,
                    video: false,
                    pdf: false,
                },
                interleaved: InterleavedConfig::Bool(false),
            },
            cost: ProviderModelCost {
                input: 0.0,
                output: 0.0,
                cache: ModelCostCache {
                    read: 0.0,
                    write: 0.0,
                },
                experimental_over_200k: None,
            },
            limit: ProviderModelLimit {
                context: 128_000,
                input: None,
                output: 8_192,
            },
            status: "active".to_string(),
            options: HashMap::new(),
            headers: HashMap::new(),
            release_date: "2026-01-01".to_string(),
            variants: None,
        }
    }

    fn model_info(model_id: &str) -> ModelInfo {
        ModelInfo {
            id: model_id.to_string(),
            name: model_id.to_string(),
            family: None,
            release_date: Some("2026-01-01".to_string()),
            attachment: false,
            reasoning: true,
            temperature: true,
            tool_call: true,
            interleaved: Some(ModelInterleaved::Bool(false)),
            cost: None,
            limit: ModelLimit {
                context: 128_000,
                input: None,
                output: 8_192,
            },
            modalities: Some(ModelModalities {
                input: vec!["text".to_string()],
                output: vec!["text".to_string()],
            }),
            experimental: None,
            status: Some("active".to_string()),
            options: HashMap::new(),
            headers: None,
            provider: Some(ModelProvider {
                npm: Some("@ai-sdk/openai".to_string()),
                api: Some("https://api.openai.com/v1".to_string()),
            }),
            variants: None,
        }
    }

    fn provider_info(provider_id: &str, model: ModelInfo) -> ModelsProviderInfo {
        let mut models = HashMap::new();
        models.insert(model.id.clone(), model);
        ModelsProviderInfo {
            api: Some("https://example.com".to_string()),
            name: provider_id.to_string(),
            env: vec![],
            id: provider_id.to_string(),
            npm: Some("@ai-sdk/openai".to_string()),
            models,
        }
    }

    fn provider_state(id: &str) -> ProviderState {
        ProviderState {
            id: id.to_string(),
            name: id.to_string(),
            source: "env".to_string(),
            env: vec![],
            key: None,
            options: HashMap::new(),
            models: HashMap::new(),
        }
    }

    #[test]
    fn creates_openai_provider_from_state_key() {
        let mut state = provider_state("openai");
        state.key = Some("test-key".to_string());

        let provider = create_concrete_provider("openai", &state).expect("provider should exist");
        assert_eq!(provider.id(), "openai");
    }

    #[test]
    fn azure_provider_requires_endpoint() {
        let mut state = provider_state("azure");
        state.key = Some("test-key".to_string());
        assert!(create_concrete_provider("azure", &state).is_none());

        state.options.insert(
            "endpoint".to_string(),
            serde_json::Value::String("https://example.openai.azure.com".to_string()),
        );
        let provider = create_concrete_provider("azure", &state).expect("provider should exist");
        assert_eq!(provider.id(), "azure");
    }

    #[test]
    fn creates_bedrock_provider_from_options() {
        let mut state = provider_state("amazon-bedrock");
        state.options.insert(
            "accessKeyId".to_string(),
            serde_json::Value::String("akid".to_string()),
        );
        state.options.insert(
            "secretAccessKey".to_string(),
            serde_json::Value::String("secret".to_string()),
        );
        state.options.insert(
            "region".to_string(),
            serde_json::Value::String("us-east-1".to_string()),
        );

        let provider =
            create_concrete_provider("amazon-bedrock", &state).expect("provider should exist");
        assert_eq!(provider.id(), "amazon-bedrock");
    }

    #[test]
    fn sort_models_prioritizes_big_pickle_over_non_priority_models() {
        let mut models = vec![
            provider_model("my-custom-model"),
            provider_model("big-pickle-v2"),
        ];
        ProviderBootstrapState::sort_models(&mut models);
        assert_eq!(models[0].id, "big-pickle-v2");
    }

    #[test]
    fn apply_custom_loaders_applies_zenmux_headers() {
        let model = model_info("zenmux-model");
        let mut data = HashMap::new();
        data.insert("zenmux".to_string(), provider_info("zenmux", model));

        apply_custom_loaders(&mut data);

        let provider = data.get("zenmux").expect("zenmux provider should exist");
        let model = provider
            .models
            .get("zenmux-model")
            .expect("zenmux model should exist");
        let headers = model.headers.as_ref().expect("headers should be set");
        assert_eq!(
            headers.get("HTTP-Referer").map(String::as_str),
            Some("https://opencode.ai/")
        );
        assert_eq!(headers.get("X-Title").map(String::as_str), Some("opencode"));
    }

    #[test]
    fn bedrock_loader_reads_provider_state_options() {
        let loader = AmazonBedrockLoader;
        let mut state = provider_state("amazon-bedrock");
        state.options.insert(
            "region".to_string(),
            serde_json::Value::String("us-west-2".to_string()),
        );
        state.options.insert(
            "profile".to_string(),
            serde_json::Value::String("dev-profile".to_string()),
        );
        state.options.insert(
            "endpoint".to_string(),
            serde_json::Value::String("https://bedrock.internal".to_string()),
        );

        let result = loader.load(
            &provider_info("amazon-bedrock", model_info("anthropic.claude-3-7-sonnet")),
            Some(&state),
        );
        assert!(result.autoload);
        assert_eq!(
            result.options.get("region"),
            Some(&serde_json::Value::String("us-west-2".to_string()))
        );
        assert_eq!(
            result.options.get("profile"),
            Some(&serde_json::Value::String("dev-profile".to_string()))
        );
        assert_eq!(
            result.options.get("endpoint"),
            Some(&serde_json::Value::String(
                "https://bedrock.internal".to_string()
            ))
        );
        assert!(result.has_custom_get_model);
    }

    #[test]
    fn from_models_dev_model_merges_transform_and_explicit_variants() {
        let mut model = model_info("gpt-5");
        let mut explicit = HashMap::new();
        explicit.insert(
            "custom".to_string(),
            HashMap::from([(
                "reasoningEffort".to_string(),
                serde_json::Value::String("custom".to_string()),
            )]),
        );
        model.variants = Some(explicit);

        let provider = provider_info("openai", model.clone());
        let runtime_model = from_models_dev_model(&provider, &model);
        let variants = runtime_model
            .variants
            .expect("variants should include generated and explicit values");
        assert!(variants.contains_key("custom"));
        assert!(variants.contains_key("low"));
    }
}
