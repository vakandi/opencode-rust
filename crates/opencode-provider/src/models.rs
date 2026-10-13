use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

pub const MODELS_DEV_URL: &str = "https://models.dev";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub context_over_200k: Option<Box<ModelCost>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimit {
    pub context: u64,
    #[serde(default)]
    pub input: Option<u64>,
    pub output: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelModalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProvider {
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub api: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelInterleaved {
    Bool(bool),
    Field { field: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub attachment: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub temperature: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub interleaved: Option<ModelInterleaved>,
    #[serde(default)]
    pub cost: Option<ModelCost>,
    pub limit: ModelLimit,
    #[serde(default)]
    pub modalities: Option<ModelModalities>,
    #[serde(default)]
    pub experimental: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub provider: Option<ModelProvider>,
    #[serde(default)]
    pub variants: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    #[serde(default)]
    pub api: Option<String>,
    pub name: String,
    pub env: Vec<String>,
    pub id: String,
    #[serde(default)]
    pub npm: Option<String>,
    pub models: HashMap<String, ModelInfo>,
}

pub type ModelsData = HashMap<String, ProviderInfo>;

pub struct ModelsRegistry {
    data: Arc<RwLock<Option<ModelsData>>>,
    cache_path: PathBuf,
}

impl ModelsRegistry {
    pub fn new(cache_path: PathBuf) -> Self {
        Self {
            data: Arc::new(RwLock::new(None)),
            cache_path,
        }
    }

    pub async fn get(&self) -> ModelsData {
        let data = self.data.read().await;
        if let Some(ref d) = *data {
            return d.clone();
        }
        drop(data);

        self.load().await
    }

    async fn load(&self) -> ModelsData {
        if let Ok(content) = tokio::fs::read_to_string(&self.cache_path).await {
            if let Ok(parsed) = serde_json::from_str::<ModelsData>(&content) {
                let mut data = self.data.write().await;
                *data = Some(parsed.clone());
                return parsed;
            }
        }

        self.fetch().await
    }

    async fn fetch(&self) -> ModelsData {
        let url = format!("{}/api.json", MODELS_DEV_URL);

        match reqwest::Client::new()
            .get(&url)
            .header("User-Agent", "opencode-rust")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => match response.text().await {
                Ok(text) => {
                    if let Ok(parsed) = serde_json::from_str::<ModelsData>(&text) {
                        let _ = tokio::fs::write(&self.cache_path, &text).await;
                        let mut data = self.data.write().await;
                        *data = Some(parsed.clone());
                        return parsed;
                    }
                }
                _ => {}
            },
            _ => {}
        }

        HashMap::new()
    }

    pub async fn refresh(&self) {
        self.fetch().await;
    }

    pub async fn get_provider(&self, provider_id: &str) -> Option<ProviderInfo> {
        let data = self.get().await;
        data.get(provider_id).cloned()
    }

    pub async fn get_model(&self, provider_id: &str, model_id: &str) -> Option<ModelInfo> {
        let data = self.get().await;
        data.get(provider_id)
            .and_then(|p| p.models.get(model_id).cloned())
    }

    pub async fn list_models_for_provider(&self, provider_id: &str) -> Vec<ModelInfo> {
        let data = self.get().await;
        data.get(provider_id)
            .map(|p| p.models.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Apply custom loaders and filtering to the loaded models data
    pub async fn get_with_customization(&self, enable_experimental: bool) -> ModelsData {
        let mut data = self.get().await;
        crate::bootstrap::apply_custom_loaders(&mut data);
        crate::bootstrap::filter_models_by_status(&mut data, enable_experimental);
        data
    }
}

impl Default for ModelsRegistry {
    fn default() -> Self {
        let cache_path = dirs::cache_dir()
            .unwrap_or_else(|| std::env::temp_dir())
            .join("opencode")
            .join("models.json");
        Self::new(cache_path)
    }
}

pub fn default_model_limits() -> (u64, u64) {
    (4096, 128000)
}

pub fn get_model_context_limit(model_id: &str) -> u64 {
    let lower = model_id.to_lowercase();

    if lower.contains("gpt-4") || lower.contains("gpt-4") {
        if lower.contains("32k") {
            return 32768;
        }
        if lower.contains("128k") || lower.contains("turbo") {
            return 128000;
        }
        return 8192;
    }

    if lower.contains("claude-3") || lower.contains("claude-3") {
        return 200000;
    }

    if lower.contains("claude-2") {
        return 100000;
    }

    if lower.contains("gemini") {
        if lower.contains("pro") || lower.contains("ultra") {
            return 1000000;
        }
        return 32000;
    }

    if lower.contains("llama") {
        return 128000;
    }

    128000
}

pub fn supports_vision(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();

    lower.contains("vision")
        || lower.contains("gpt-4")
        || lower.contains("claude-3")
        || lower.contains("gemini")
        || lower.contains("qwen-vl")
}

pub fn supports_function_calling(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();

    !lower.contains("embedding") && !lower.contains("whisper") && !lower.contains("tts")
}
