use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures::Stream;
use once_cell::sync::Lazy;

use crate::provider::ProviderError;

pub type CustomFetchChunkStream = Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send>>;

#[derive(Debug, Clone)]
pub struct CustomFetchRequest {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CustomFetchResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

pub struct CustomFetchStreamResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub stream: CustomFetchChunkStream,
}

#[async_trait]
pub trait CustomFetchProxy: Send + Sync {
    async fn fetch(
        &self,
        request: CustomFetchRequest,
    ) -> Result<CustomFetchResponse, ProviderError>;

    async fn fetch_stream(
        &self,
        request: CustomFetchRequest,
    ) -> Result<CustomFetchStreamResponse, ProviderError>;
}

static CUSTOM_FETCH_PROXIES: Lazy<RwLock<HashMap<String, Arc<dyn CustomFetchProxy>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

pub fn register_custom_fetch_proxy(
    provider_id: impl Into<String>,
    proxy: Arc<dyn CustomFetchProxy>,
) {
    if let Ok(mut guard) = CUSTOM_FETCH_PROXIES.write() {
        guard.insert(provider_id.into(), proxy);
    }
}

pub fn unregister_custom_fetch_proxy(provider_id: &str) {
    if let Ok(mut guard) = CUSTOM_FETCH_PROXIES.write() {
        guard.remove(provider_id);
    }
}

pub fn clear_custom_fetch_proxies() {
    if let Ok(mut guard) = CUSTOM_FETCH_PROXIES.write() {
        guard.clear();
    }
}

pub fn get_custom_fetch_proxy(provider_id: &str) -> Option<Arc<dyn CustomFetchProxy>> {
    CUSTOM_FETCH_PROXIES
        .read()
        .ok()
        .and_then(|guard| guard.get(provider_id).cloned())
}
