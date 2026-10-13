use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod subprocess;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl HookOutput {
    pub fn with_payload(payload: serde_json::Value) -> Self {
        Self {
            payload: Some(payload),
        }
    }
}

impl From<()> for HookOutput {
    fn from(_: ()) -> Self {
        Self::default()
    }
}

impl From<serde_json::Value> for HookOutput {
    fn from(payload: serde_json::Value) -> Self {
        HookOutput::with_payload(payload)
    }
}

pub type HookResult = Result<HookOutput, HookError>;

pub type HookHandler =
    Box<dyn Fn(HookContext) -> Pin<Box<dyn Future<Output = HookResult> + Send>> + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HookEvent {
    // Original events
    ConfigLoaded,
    SessionStart,
    SessionEnd,
    ToolCall,
    ToolResult,
    MessageSent,
    MessageReceived,
    Error,
    FileChange,
    ProviderChange,

    // Tool lifecycle hooks (matches TS "tool.execute.before" / "tool.execute.after")
    ToolExecuteBefore,
    ToolExecuteAfter,

    // Tool definition transform (matches TS "tool.definition")
    ToolDefinition,

    // Chat / LLM hooks
    ChatSystemTransform,
    ChatMessagesTransform,
    ChatParams,
    ChatHeaders,
    ChatMessage,

    // Session compaction (matches TS "experimental.session.compacting")
    SessionCompacting,

    // Text completion (matches TS "experimental.text.complete")
    TextComplete,

    // Shell environment (matches TS "shell.env")
    ShellEnv,

    // Command execution (matches TS "command.execute.before")
    CommandExecuteBefore,

    // Permission (matches TS "permission.ask")
    PermissionAsk,

    /// Plugin event notification (matches TS `event`).
    ///
    /// Deliberately a unit variant: `HookEvent` derives `Eq`/`Hash`, which a
    /// `serde_json::Value` cannot satisfy. The event object itself travels in
    /// `HookContext::data` under the `event` key, which is what omo reads as
    /// `input.event.type` / `input.event.properties`.
    PluginEvent,
}

#[derive(Debug, Clone)]
pub struct HookContext {
    pub event: HookEvent,
    pub data: HashMap<String, serde_json::Value>,
    pub session_id: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl HookContext {
    pub fn new(event: HookEvent) -> Self {
        Self {
            event,
            data: HashMap::new(),
            session_id: None,
            timestamp: Utc::now(),
        }
    }

    pub fn with_data(mut self, key: &str, value: serde_json::Value) -> Self {
        self.data.insert(key.to_string(), value);
        self
    }

    pub fn with_session(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.data.get(key)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("Hook execution failed: {0}")]
    ExecutionError(String),

    #[error("Hook not found: {0}")]
    NotFound(String),

    #[error("Hook timeout")]
    Timeout,
}

pub struct Hook {
    pub name: String,
    pub event: HookEvent,
    pub handler: HookHandler,
    pub priority: i32,
    pub enabled: bool,
}

impl Hook {
    pub fn new<F, Fut, R>(name: &str, event: HookEvent, handler: F) -> Self
    where
        F: Fn(HookContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R, HookError>> + Send + 'static,
        R: Into<HookOutput> + Send + 'static,
    {
        Self {
            name: name.to_string(),
            event,
            handler: Box::new(move |ctx| {
                let future = handler(ctx);
                Box::pin(async move { future.await.map(Into::into) })
            }),
            priority: 0,
            enabled: true,
        }
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

pub struct PluginSystem {
    hooks: RwLock<HashMap<HookEvent, Vec<Arc<Hook>>>>,
}

impl PluginSystem {
    pub fn new() -> Self {
        Self {
            hooks: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, hook: Hook) {
        let mut hooks = self.hooks.write().await;
        let entry = hooks.entry(hook.event.clone()).or_insert_with(Vec::new);
        entry.push(Arc::new(hook));
        entry.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    pub async fn trigger(&self, context: HookContext) -> Vec<HookResult> {
        let hooks = self.hooks.read().await;
        let mut results = Vec::new();

        if let Some(hook_list) = hooks.get(&context.event) {
            for hook in hook_list {
                if !hook.enabled {
                    continue;
                }
                let result = (hook.handler)(context.clone()).await;
                results.push(result);
            }
        }

        results
    }

    pub async fn remove(&self, event: &HookEvent, name: &str) -> bool {
        let mut hooks = self.hooks.write().await;
        if let Some(hook_list) = hooks.get_mut(event) {
            let initial_len = hook_list.len();
            hook_list.retain(|h| h.name != name);
            return hook_list.len() < initial_len;
        }
        false
    }

    pub async fn list(&self) -> Vec<(HookEvent, String, bool)> {
        let hooks = self.hooks.read().await;
        let mut result = Vec::new();

        for (event, hook_list) in hooks.iter() {
            for hook in hook_list {
                result.push((event.clone(), hook.name.clone(), hook.enabled));
            }
        }

        result
    }
}

impl Default for PluginSystem {
    fn default() -> Self {
        Self::new()
    }
}

pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn register_hooks(
        &self,
        system: &PluginSystem,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

pub struct PluginRegistry {
    plugins: RwLock<Vec<Arc<dyn Plugin>>>,
    hook_system: Arc<PluginSystem>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: RwLock::new(Vec::new()),
            hook_system: Arc::new(PluginSystem::new()),
        }
    }

    pub async fn register(&self, plugin: Arc<dyn Plugin>) {
        plugin.register_hooks(&self.hook_system).await;
        let mut plugins = self.plugins.write().await;
        plugins.push(plugin);
    }

    pub fn hook_system(&self) -> Arc<PluginSystem> {
        self.hook_system.clone()
    }

    pub async fn list(&self) -> Vec<(String, String)> {
        let plugins = self.plugins.read().await;
        plugins
            .iter()
            .map(|p| (p.name().to_string(), p.version().to_string()))
            .collect()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Global Plugin System
// ============================================================================

static GLOBAL_PLUGIN_SYSTEM: std::sync::OnceLock<Arc<PluginSystem>> = std::sync::OnceLock::new();

/// Initialize the global plugin system. Call once at startup.
pub fn init_global(system: Arc<PluginSystem>) {
    let existing = GLOBAL_PLUGIN_SYSTEM.get_or_init(|| Arc::clone(&system));
    if !Arc::ptr_eq(existing, &system) {
        tracing::debug!("global plugin system already initialized; ignoring duplicate init");
    }
}

/// Get the global plugin system, creating a default one if not initialized.
pub fn global() -> Arc<PluginSystem> {
    GLOBAL_PLUGIN_SYSTEM
        .get_or_init(|| Arc::new(PluginSystem::new()))
        .clone()
}

/// Convenience: trigger a hook event on the global plugin system.
/// Errors from individual hooks are logged but do not propagate.
pub async fn trigger(context: HookContext) {
    let system = global();
    let results = system.trigger(context).await;
    for result in results {
        if let Err(e) = result {
            tracing::warn!("Plugin hook error: {}", e);
        }
    }
}

/// Convenience: trigger a hook event and collect successful outputs.
/// Errors from individual hooks are logged but do not propagate.
pub async fn trigger_collect(context: HookContext) -> Vec<HookOutput> {
    let system = global();
    let results = system.trigger(context).await;
    let mut outputs = Vec::new();
    for result in results {
        match result {
            Ok(output) => outputs.push(output),
            Err(e) => tracing::warn!("Plugin hook error: {}", e),
        }
    }
    outputs
}

/// Convenience: build a HookContext and trigger it on the global system.
pub async fn trigger_event(event: HookEvent) {
    trigger(HookContext::new(event)).await;
}

/// Convenience: build a HookContext with session and trigger it.
pub async fn trigger_session_event(event: HookEvent, session_id: &str) {
    trigger(HookContext::new(event).with_session(session_id)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hook_new_supports_unit_result() {
        let system = PluginSystem::new();
        system
            .register(Hook::new("unit", HookEvent::SessionStart, |_ctx| async {
                Ok(())
            }))
            .await;

        let result = system
            .trigger(HookContext::new(HookEvent::SessionStart))
            .await;
        assert_eq!(result.len(), 1);
        assert!(result[0].as_ref().unwrap().payload.is_none());
    }

    #[tokio::test]
    async fn hook_new_supports_payload_result() {
        let system = PluginSystem::new();
        system
            .register(Hook::new(
                "payload",
                HookEvent::SessionCompacting,
                |_ctx| async {
                    Ok(serde_json::json!({
                        "prompt": "override",
                        "context": ["ctx1"]
                    }))
                },
            ))
            .await;

        let result = system
            .trigger(HookContext::new(HookEvent::SessionCompacting))
            .await;
        assert_eq!(result.len(), 1);
        let payload = result[0]
            .as_ref()
            .unwrap()
            .payload
            .as_ref()
            .expect("payload should be present");
        assert_eq!(
            payload.get("prompt").and_then(|v| v.as_str()),
            Some("override")
        );
    }
}

/// Tools contributed by loaded TS/JS plugins.
///
/// `create_default_registry()` in `opencode-tool` rebuilds a fresh registry on every
/// prompt, so tools cannot be injected into one long-lived registry. They are recorded
/// here at load time and re-registered into each freshly built registry instead.
pub mod plugin_tools {
    use std::sync::{Arc, OnceLock};
    use tokio::sync::RwLock;

    use crate::subprocess::client::PluginSubprocess;

    #[derive(Clone)]
    pub struct LoadedPluginTool {
        pub name: String,
        pub description: String,
        pub parameters: serde_json::Value,
        pub client: Arc<PluginSubprocess>,
    }

    static REGISTRY: OnceLock<RwLock<Vec<LoadedPluginTool>>> = OnceLock::new();

    fn registry() -> &'static RwLock<Vec<LoadedPluginTool>> {
        REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
    }

    /// Record every tool a freshly initialized plugin contributed.
    pub async fn register(client: Arc<PluginSubprocess>) {
        let contributed: Vec<(String, serde_json::Value)> = client
            .tools()
            .iter()
            .map(|(name, def)| (name.clone(), def.clone()))
            .collect();

        if contributed.is_empty() {
            tracing::debug!(
                plugin = client.name(),
                "plugin contributed no tools (tools map empty)"
            );
            return;
        }

        let mut entries = registry().write().await;
        for (name, def) in contributed {
            if entries.iter().any(|e| e.name == name) {
                continue;
            }
            entries.push(LoadedPluginTool {
                description: def
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                parameters: def
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type":"object","properties":{}})),
                name,
                client: Arc::clone(&client),
            });
        }
    }

    pub async fn list() -> Vec<LoadedPluginTool> {
        registry().read().await.clone()
    }
}
