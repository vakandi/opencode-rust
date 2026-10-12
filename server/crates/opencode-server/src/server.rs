use async_trait::async_trait;
use axum::http::{header::HeaderValue, request::Parts};
use futures::StreamExt;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::{broadcast, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;

use opencode_config::load_config;
use opencode_plugin::init_global;
use opencode_plugin::subprocess::{
    PluginAuthBridge, PluginContext, PluginFetchRequest, PluginLoader,
};
use opencode_provider::{
    bootstrap_config_from_raw, create_registry_from_bootstrap_config, register_custom_fetch_proxy,
    unregister_custom_fetch_proxy, AuthInfo, AuthManager, ConfigModel as BootstrapConfigModel,
    ConfigProvider as BootstrapConfigProvider, CustomFetchProxy, CustomFetchRequest,
    CustomFetchResponse, CustomFetchStreamResponse, ProviderError, ProviderRegistry,
};
use opencode_session::SessionManager;
use opencode_storage::{Database, MessageRepository, SessionRepository};

use crate::routes;

const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:4096";

struct PluginBridgeFetchProxy {
    bridge: Arc<PluginAuthBridge>,
}

#[async_trait]
impl CustomFetchProxy for PluginBridgeFetchProxy {
    async fn fetch(
        &self,
        request: CustomFetchRequest,
    ) -> Result<CustomFetchResponse, ProviderError> {
        let response = self
            .bridge
            .fetch_proxy(PluginFetchRequest {
                url: request.url,
                method: request.method,
                headers: request.headers,
                body: request.body,
            })
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        Ok(CustomFetchResponse {
            status: response.status,
            headers: response.headers,
            body: response.body,
        })
    }

    async fn fetch_stream(
        &self,
        request: CustomFetchRequest,
    ) -> Result<CustomFetchStreamResponse, ProviderError> {
        let response = self
            .bridge
            .fetch_proxy_stream(PluginFetchRequest {
                url: request.url,
                method: request.method,
                headers: request.headers,
                body: request.body,
            })
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let stream = ReceiverStream::new(response.chunks)
            .map(|item| item.map_err(|e| ProviderError::NetworkError(e.to_string())));
        Ok(CustomFetchStreamResponse {
            status: response.status,
            headers: response.headers,
            stream: Box::pin(stream),
        })
    }
}

pub(crate) fn sync_custom_fetch_proxy(
    provider_id: &str,
    bridge: Arc<PluginAuthBridge>,
    enabled: bool,
) {
    if enabled {
        register_custom_fetch_proxy(
            provider_id.to_string(),
            Arc::new(PluginBridgeFetchProxy {
                bridge: bridge.clone(),
            }),
        );
        if provider_id == "github-copilot" {
            register_custom_fetch_proxy(
                "github-copilot-enterprise",
                Arc::new(PluginBridgeFetchProxy { bridge }),
            );
        }
    } else {
        unregister_custom_fetch_proxy(provider_id);
        if provider_id == "github-copilot" {
            unregister_custom_fetch_proxy("github-copilot-enterprise");
        }
    }
}

pub struct ServerState {
    pub sessions: Mutex<SessionManager>,
    pub providers: ProviderRegistry,
    pub auth_manager: Arc<AuthManager>,
    pub event_bus: broadcast::Sender<String>,
    /// Serializes `sync_sessions_to_storage`.
    ///
    /// The sync deletes and re-inserts every message of every in-memory session. Two
    /// concurrent syncs interleave their read/insert phases and collide on the primary key
    /// (`UNIQUE constraint failed: messages.id`), which loses writes. Serializing is the
    /// minimum correct fix; the sync remains a full rewrite until it is made incremental.
    sync_gate: tokio::sync::Mutex<()>,
    session_repo: Option<SessionRepository>,
    message_repo: Option<MessageRepository>,
}

impl ServerState {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            sessions: Mutex::new(SessionManager::new()),
            providers: ProviderRegistry::new(),
            auth_manager: Arc::new(AuthManager::new()),
            event_bus: tx,
            sync_gate: tokio::sync::Mutex::new(()),
            session_repo: None,
            message_repo: None,
        }
    }

    pub async fn new_with_storage() -> anyhow::Result<Self> {
        Self::new_with_storage_for_url(DEFAULT_SERVER_URL.to_string()).await
    }

    pub async fn new_with_storage_for_url(server_url: String) -> anyhow::Result<Self> {
        let mut state = Self::new();
        let auth_manager = Arc::new(AuthManager::load_from_file(&auth_data_dir()).await);
        state.auth_manager = auth_manager.clone();
        load_plugin_auth_store(&server_url, auth_manager.clone()).await;
        let auth_store = auth_manager.list().await;

        // Load config and convert providers to bootstrap format
        let cwd = std::env::current_dir().unwrap_or_default();
        let bootstrap_config = match load_config(&cwd) {
            Ok(config) => {
                let config = apply_config_hook(config).await;
                let providers = convert_config_providers_for_bootstrap(&config);
                bootstrap_config_from_raw(
                    providers,
                    config.disabled_providers.clone(),
                    config.enabled_providers.clone(),
                    config.model.clone(),
                    config.small_model.clone(),
                )
            }
            Err(error) => {
                tracing::warn!(%error, "failed to load config for provider bootstrap, using defaults");
                opencode_provider::BootstrapConfig::default()
            }
        };

        state.providers = create_registry_from_bootstrap_config(&bootstrap_config, &auth_store);
        let db = Database::new().await?;
        let pool = db.pool().clone();
        state.session_repo = Some(SessionRepository::new(pool.clone()));
        state.message_repo = Some(MessageRepository::new(pool));
        state.load_sessions_from_storage().await?;
        Ok(state)
    }

    pub fn broadcast(&self, event: &str) {
        let _ = self.event_bus.send(event.to_string());
    }

    async fn load_sessions_from_storage(&self) -> anyhow::Result<()> {
        let (Some(session_repo), Some(message_repo)) = (&self.session_repo, &self.message_repo)
        else {
            return Ok(());
        };

        let stored_sessions = session_repo.list(None, 100_000).await?;
        let mut manager = self.sessions.lock().await;

        for mut stored in stored_sessions {
            let stored_messages = message_repo.list_for_session(&stored.id).await?;
            stored.messages = stored_messages;
            let session: opencode_session::Session =
                serde_json::from_value(serde_json::to_value(stored)?)?;
            manager.update(session);
        }

        Ok(())
    }

    pub async fn sync_sessions_to_storage(&self) -> anyhow::Result<()> {
        let (Some(session_repo), Some(message_repo)) = (&self.session_repo, &self.message_repo)
        else {
            return Ok(());
        };

        let _serialized = self.sync_gate.lock().await;

        let snapshot: Vec<opencode_session::Session> = {
            let manager = self.sessions.lock().await;
            manager.list().into_iter().cloned().collect()
        };

        let snapshot_ids: HashSet<String> = snapshot.iter().map(|s| s.id.clone()).collect();
        let persisted = session_repo.list(None, 100_000).await?;

        for stale in persisted {
            if !snapshot_ids.contains(&stale.id) {
                message_repo.delete_for_session(&stale.id).await?;
                session_repo.delete(&stale.id).await?;
            }
        }

        for session in snapshot {
            write_session(&session_repo, &message_repo, &session).await?;
        }

        Ok(())
    }

    /// Persist a single session instead of rewriting every session.
    ///
    /// The full sync costs O(all messages) per mutating request, and 22 routes trigger it,
    /// so N concurrent sessions do O(N x all-messages) work. Serialized, that queue is what
    /// made 100 concurrent requests time out. Callers that know which session they touched
    /// use this and pay O(one session).
    pub async fn sync_session_to_storage(&self, session_id: &str) -> anyhow::Result<()> {
        let (Some(session_repo), Some(message_repo)) = (&self.session_repo, &self.message_repo)
        else {
            return Ok(());
        };

        let _serialized = self.sync_gate.lock().await;

        let session = {
            let manager = self.sessions.lock().await;
            manager.get(session_id).cloned()
        };
        let Some(session) = session else {
            // The session is gone from memory, so its rows should go too.
            message_repo.delete_for_session(session_id).await?;
            session_repo.delete(session_id).await?;
            return Ok(());
        };

        write_session(&session_repo, &message_repo, &session).await
    }
}

async fn write_session(
    session_repo: &SessionRepository,
    message_repo: &MessageRepository,
    session: &opencode_session::Session,
) -> anyhow::Result<()> {
    let mut stored_session: opencode_types::Session =
        serde_json::from_value(serde_json::to_value(session)?)?;
    let stored_messages: Vec<opencode_types::SessionMessage> = stored_session.messages.clone();
    stored_session.messages.clear();

    if session_repo.get(&stored_session.id).await?.is_some() {
        session_repo.update(&stored_session).await?;
    } else {
        session_repo.create(&stored_session).await?;
    }

    message_repo.delete_for_session(&stored_session.id).await?;
    for message in stored_messages {
        message_repo.create(&message).await?;
    }
    Ok(())
}

/// Convert opencode_config::ProviderConfig map to bootstrap ConfigProvider map.
fn convert_config_providers_for_bootstrap(
    config: &opencode_config::Config,
) -> std::collections::HashMap<String, BootstrapConfigProvider> {
    let Some(ref providers) = config.provider else {
        return std::collections::HashMap::new();
    };

    providers
        .iter()
        .map(|(id, provider)| (id.clone(), provider_to_bootstrap(provider)))
        .collect()
}

fn provider_to_bootstrap(provider: &opencode_config::ProviderConfig) -> BootstrapConfigProvider {
    let mut options = provider.options.clone().unwrap_or_default();
    if let Some(api_key) = &provider.api_key {
        options
            .entry("apiKey".to_string())
            .or_insert_with(|| serde_json::Value::String(api_key.clone()));
    }
    if let Some(base_url) = &provider.base_url {
        options
            .entry("baseURL".to_string())
            .or_insert_with(|| serde_json::Value::String(base_url.clone()));
    }

    let models = provider.models.as_ref().map(|models| {
        models
            .iter()
            .map(|(id, model)| (id.clone(), model_to_bootstrap(id, model)))
            .collect()
    });

    BootstrapConfigProvider {
        name: provider.name.clone(),
        api: provider.base_url.clone(),
        npm: provider.npm.clone(),
        options: (!options.is_empty()).then_some(options),
        models,
        blacklist: (!provider.blacklist.is_empty()).then_some(provider.blacklist.clone()),
        whitelist: (!provider.whitelist.is_empty()).then_some(provider.whitelist.clone()),
        ..Default::default()
    }
}

fn model_to_bootstrap(id: &str, model: &opencode_config::ModelConfig) -> BootstrapConfigModel {
    let mut options = HashMap::new();
    if let Some(api_key) = &model.api_key {
        options.insert(
            "apiKey".to_string(),
            serde_json::Value::String(api_key.clone()),
        );
    }

    let variants = model.variants.as_ref().map(|variants| {
        variants
            .iter()
            .map(|(name, variant)| (name.clone(), variant_to_bootstrap(variant)))
            .collect()
    });

    BootstrapConfigModel {
        id: model.model.clone().or_else(|| Some(id.to_string())),
        name: model.name.clone(),
        provider: model.base_url.as_ref().map(|url| {
            opencode_provider::bootstrap::ConfigModelProvider {
                api: Some(url.clone()),
                npm: None,
            }
        }),
        options: (!options.is_empty()).then_some(options),
        variants,
        ..Default::default()
    }
}

fn variant_to_bootstrap(
    variant: &opencode_config::ModelVariantConfig,
) -> HashMap<String, serde_json::Value> {
    let mut values = variant.extra.clone();
    if let Some(disabled) = variant.disabled {
        values.insert("disabled".to_string(), serde_json::Value::Bool(disabled));
    }
    values
}

fn auth_data_dir() -> PathBuf {
    if let Ok(path) = std::env::var("OPENCODE_DATA_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    opencode_paths::provider_auth_dir(|| {
        dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(std::env::temp_dir)
    })
}

/// Let plugins mutate the loaded config, then hand back the result.
///
/// `config` is a mutation hook: the plugin edits the object it receives and returns it. A
/// payload that cannot be parsed back into `Config` is ignored, so an unrecognisable
/// mutation can never cost the server its configuration.
async fn apply_config_hook(config: opencode_config::Config) -> opencode_config::Config {
    let Ok(raw) = serde_json::to_value(&config) else {
        return config;
    };
    let outputs = opencode_plugin::trigger_collect(
        opencode_plugin::HookContext::new(opencode_plugin::HookEvent::ConfigLoaded)
            .with_data("config", raw),
    )
    .await;
    for output in outputs {
        let Some(payload) = output.payload.as_ref() else {
            continue;
        };
        let Some(mutated) = payload.get("output") else {
            continue;
        };
        match serde_json::from_value::<opencode_config::Config>(mutated.clone()) {
            Ok(next) => return next,
            Err(error) => {
                tracing::warn!(%error, "config hook returned an unusable config; keeping the original");
            }
        }
    }
    config
}

async fn load_plugin_auth_store(server_url: &str, auth_manager: Arc<AuthManager>) {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            tracing::warn!(%error, "failed to get current directory for plugin bootstrap");
            return;
        }
    };

    let config = match load_config(&cwd) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(%error, "failed to load config for plugin bootstrap");
            return;
        }
    };

    let loader = match PluginLoader::new() {
        Ok(loader) => Arc::new(loader),
        Err(error) => {
            tracing::warn!(%error, "failed to initialize plugin loader");
            return;
        }
    };
    init_global(loader.hook_system());

    let directory = cwd.to_string_lossy().to_string();
    let context = PluginContext {
        worktree: directory.clone(),
        directory,
        server_url: server_url.to_string(),
    };

    if let Err(error) = loader.load_builtins(&context).await {
        tracing::warn!(%error, "failed to load builtin auth plugins");
    }

    if !config.plugin.is_empty() {
        if let Err(error) = loader.load_all(&config.plugin, &context).await {
            tracing::warn!(%error, "failed to load configured plugins");
            return;
        }
    }

    routes::set_plugin_loader(loader.clone());

    let bridges = loader.auth_bridges().await;
    for (provider_id, bridge) in bridges {
        match bridge.load().await {
            Ok(result) => {
                sync_custom_fetch_proxy(&provider_id, bridge.clone(), result.has_custom_fetch);

                if let Some(api_key) = result.api_key {
                    auth_manager
                        .set(
                            &provider_id,
                            AuthInfo::Api {
                                key: api_key.clone(),
                            },
                        )
                        .await;
                    // TS parity: copilot auth can power both standard and enterprise providers.
                    if provider_id == "github-copilot" {
                        auth_manager
                            .set("github-copilot-enterprise", AuthInfo::Api { key: api_key })
                            .await;
                    }
                }
            }
            Err(error) => {
                sync_custom_fetch_proxy(&provider_id, bridge.clone(), false);
                tracing::warn!(provider = provider_id, %error, "failed to load plugin auth");
            }
        }
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

static EXTRA_CORS_WHITELIST: Lazy<RwLock<HashSet<String>>> =
    Lazy::new(|| RwLock::new(HashSet::new()));

fn normalize_origin(origin: &str) -> Option<String> {
    let trimmed = origin.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn set_cors_whitelist(origins: Vec<String>) {
    let mut next = HashSet::new();
    for origin in origins {
        if let Some(normalized) = normalize_origin(&origin) {
            next.insert(normalized);
        }
    }

    match EXTRA_CORS_WHITELIST.write() {
        Ok(mut guard) => *guard = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

fn is_extra_allowed_origin(origin: &str) -> bool {
    let normalized = normalize_origin(origin).unwrap_or_else(|| origin.to_string());
    match EXTRA_CORS_WHITELIST.read() {
        Ok(guard) => guard.contains(&normalized),
        Err(poisoned) => poisoned.into_inner().contains(&normalized),
    }
}

fn is_allowed_origin(origin: &str) -> bool {
    origin.starts_with("http://localhost:")
        || origin.starts_with("http://127.0.0.1:")
        || origin == "tauri://localhost"
        || origin == "http://tauri.localhost"
        || origin == "https://tauri.localhost"
        || (origin.starts_with("https://") && origin.ends_with(".opencode.ai"))
        || is_extra_allowed_origin(origin)
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(
            |origin: &HeaderValue, _parts: &Parts| {
                origin.to_str().map(is_allowed_origin).unwrap_or(false)
            },
        ))
        .allow_methods(Any)
        .allow_headers(Any)
}

/// The running proxy, if one was started. Held in a `OnceLock` so `/health` can report
/// the live egress, and so the listener's task handle is not dropped.
static PROXY: std::sync::OnceLock<Option<opencode_proxy::RunningProxy>> =
    std::sync::OnceLock::new();

/// Bring up the rotating residential proxy.
///
/// # Why this must run before `ServerState::new_with_storage_for_url`
/// That call builds the provider registry, and every provider adapter calls
/// `reqwest::Client::new()`. `reqwest::Config::default()` sets `auto_sys_proxy: true`,
/// and `ClientBuilder::build()` reads the system proxy matcher from the environment at
/// that instant — so an export performed afterwards has no effect on providers that are
/// already built. The ordering is the integration; it is not a style preference.
///
/// Failures degrade rather than abort: an unreachable or empty pool must not stop the
/// server from booting, it must only mean provider traffic goes direct.
pub async fn start_native_proxy() {
    if PROXY.get().is_some() {
        return;
    }
    let cfg = opencode_proxy::ProxyConfig::from_env();
    if !cfg.enabled {
        tracing::info!("native proxy disabled; provider traffic will go direct");
        let _ = PROXY.set(None);
        return;
    }
    match opencode_proxy::start(cfg).await {
        Ok(running) => {
            // MUST precede every provider client construction: a client built without the port
            // snapshots an environment with no proxy and egresses directly.
            opencode_provider::session_client::set_proxy_port(running.port);
            tracing::info!(port = running.port, url = %running.url, "native rotating proxy active");
            let _ = PROXY.set(Some(running));
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "native proxy failed to start; provider traffic will go direct (no proxy env exported)"
            );
            // Deliberately does NOT export a proxy URL pointing at a dead port: that
            // turns a degraded start into an instant, total provider outage.
            let _ = PROXY.set(None);
        }
    }
}

/// Proxy state for `/health`. `None` means no proxy is running.
pub fn proxy_snapshot() -> Option<serde_json::Value> {
    PROXY.get().and_then(|slot| slot.as_ref()).map(|running| running.state.snapshot())
}

pub fn proxy_state() -> Option<Arc<opencode_proxy::ProxyState>> {
    PROXY.get().and_then(|slot| slot.as_ref()).map(|running| Arc::clone(&running.state))
}

/// Send provider traffic out on this host's real IP instead of a residential proxy.
/// Returns false when no proxy is running, which means traffic is already direct.
pub fn set_proxy_direct(direct: bool) -> bool {
    match proxy_state() {
        Some(state) => {
            state.set_direct(direct);
            tracing::info!(direct, "proxy direct mode changed at runtime");
            true
        }
        None => false,
    }
}

/// Give `session_id` its own residential egress.
///
/// Best-effort by design: if the proxy is disabled or the pool is exhausted, the
/// session still runs on the shared egress rather than failing to start.
pub async fn ensure_session_proxy(session_id: &str) {
    let Some(state) = proxy_state() else {
        return;
    };
    if state.egress_for(session_id).is_some() {
        return;
    }
    opencode_proxy::rotate::ensure_session(&state, session_id).await;
}

pub async fn run_server(addr: SocketAddr) -> anyhow::Result<()> {
    // MUST precede the provider registry construction. See `start_native_proxy`.
    start_native_proxy().await;

    let state = Arc::new(ServerState::new_with_storage_for_url(format!("http://{}", addr)).await?);

    let app = routes::router()
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
        .layer(axum::middleware::from_fn(routes::require_admin_token));

    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("Server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

pub async fn run_server_with_state(
    addr: SocketAddr,
    state: Arc<ServerState>,
) -> anyhow::Result<()> {
    let app = routes::router()
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
        .layer(axum::middleware::from_fn(routes::require_admin_token));

    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("Server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn state_with_repos(
        session_repo: SessionRepository,
        message_repo: MessageRepository,
    ) -> ServerState {
        let mut state = ServerState::new();
        state.session_repo = Some(session_repo);
        state.message_repo = Some(message_repo);
        state
    }

    #[tokio::test]
    async fn storage_roundtrip_restores_sessions_and_messages() {
        let db = Database::in_memory()
            .await
            .expect("in-memory db should initialize");
        let pool = db.pool().clone();

        let state = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool.clone()),
        );
        let (session_id, user_created_at, assistant_created_at) = {
            let mut manager = state.sessions.lock().await;
            let session = manager.create("default", ".");
            let session_id = session.id.clone();

            let fixed_user_time = chrono::Utc
                .timestamp_millis_opt(1_700_000_000_000)
                .single()
                .expect("valid user timestamp");
            let fixed_assistant_time = chrono::Utc
                .timestamp_millis_opt(1_700_000_000_123)
                .single()
                .expect("valid assistant timestamp");

            let session = manager
                .get_mut(&session_id)
                .expect("session should be available for mutation");
            let user = session.add_user_message("hello");
            user.created_at = fixed_user_time;
            if let Some(part) = user.parts.first_mut() {
                part.created_at = fixed_user_time;
            }

            let assistant = session.add_assistant_message();
            assistant.created_at = fixed_assistant_time;
            assistant.add_text("world");
            if let Some(part) = assistant.parts.first_mut() {
                part.created_at = fixed_assistant_time;
            }

            (session_id, fixed_user_time, fixed_assistant_time)
        };

        state
            .sync_sessions_to_storage()
            .await
            .expect("session snapshot should sync to storage");

        let reloaded = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool),
        );
        reloaded
            .load_sessions_from_storage()
            .await
            .expect("sessions should reload from storage");

        let manager = reloaded.sessions.lock().await;
        let session = manager
            .get(&session_id)
            .expect("session should exist after reload");
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].created_at, user_created_at);
        assert_eq!(session.messages[1].created_at, assistant_created_at);
        assert_eq!(session.messages[0].get_text(), "hello");
        assert_eq!(session.messages[1].get_text(), "world");
    }

    #[tokio::test]
    async fn sync_removes_deleted_sessions_from_storage() {
        let db = Database::in_memory()
            .await
            .expect("in-memory db should initialize");
        let pool = db.pool().clone();
        let session_repo = SessionRepository::new(pool.clone());

        let state = state_with_repos(
            SessionRepository::new(pool.clone()),
            MessageRepository::new(pool),
        );
        let session_id = {
            let mut manager = state.sessions.lock().await;
            manager.create("default", ".").id
        };

        state
            .sync_sessions_to_storage()
            .await
            .expect("initial snapshot should sync");
        assert_eq!(
            session_repo
                .list(None, 10)
                .await
                .expect("list should succeed")
                .len(),
            1
        );

        {
            let mut manager = state.sessions.lock().await;
            manager.delete(&session_id);
        }

        state
            .sync_sessions_to_storage()
            .await
            .expect("delete sync should succeed");
        assert!(session_repo
            .get(&session_id)
            .await
            .expect("get should succeed")
            .is_none());
    }
}
