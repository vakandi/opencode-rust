use chrono::Utc;
use opencode_core::bus::{Bus, BusEventDef};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};

use crate::oauth::McpOAuthManager;
use crate::protocol::*;
use crate::tool::McpToolRegistry;
use crate::transport::{HttpTransport, McpTransport, SseTransport, StdioTransport};

// ---------------------------------------------------------------------------
// McpStatus – mirrors the TS discriminated union `MCP.Status`
// ---------------------------------------------------------------------------

/// Connection status of an MCP server.
///
/// Uses Rust's enum-with-data to model the same state machine as the TS
/// `Status` discriminated union, but with compile-time exhaustiveness checks.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum McpStatus {
    Connected,
    Disabled,
    Failed { error: String },
    NeedsAuth,
    NeedsClientRegistration { error: String },
}

impl McpStatus {
    pub fn is_connected(&self) -> bool {
        matches!(self, McpStatus::Connected)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, McpStatus::Failed { .. })
    }

    pub fn is_needs_auth(&self) -> bool {
        matches!(self, McpStatus::NeedsAuth)
    }
}

impl std::fmt::Display for McpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpStatus::Connected => write!(f, "connected"),
            McpStatus::Disabled => write!(f, "disabled"),
            McpStatus::Failed { error } => write!(f, "failed: {error}"),
            McpStatus::NeedsAuth => write!(f, "needs_auth"),
            McpStatus::NeedsClientRegistration { error } => {
                write!(f, "needs_client_registration: {error}")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum McpClientError {
    #[error("Transport error: {0}")]
    TransportError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Not initialized")]
    NotInitialized,

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Timeout")]
    Timeout,

    #[error("Unauthorized")]
    Unauthorized,

    #[error("OAuth error: {0}")]
    OAuthError(String),
}

// ---------------------------------------------------------------------------
// McpServerConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<Vec<(String, String)>>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
enum RegistryConnectionConfig {
    Stdio(McpServerConfig),
    Http {
        url: String,
        headers: Option<HashMap<String, String>>,
        timeout_ms: Option<u64>,
    },
    Sse {
        url: String,
        headers: Option<HashMap<String, String>>,
        timeout_ms: Option<u64>,
    },
}

// ---------------------------------------------------------------------------
// McpClient
// ---------------------------------------------------------------------------

pub struct McpClient {
    server_name: String,
    transport: Mutex<Option<Box<dyn McpTransport>>>,
    request_id: AtomicU64,
    initialized: RwLock<bool>,
    capabilities: RwLock<Option<ServerCapabilities>>,
    tool_registry: Arc<McpToolRegistry>,
    timeout_ms: u64,
    status: RwLock<McpStatus>,
    oauth_manager: RwLock<Option<Arc<McpOAuthManager>>>,
    bus: Option<Arc<Bus>>,
    /// Set to true when a `notifications/tools/list_changed` is received.
    tools_changed: std::sync::atomic::AtomicBool,
}

pub static MCP_TOOLS_CHANGED_EVENT: BusEventDef = BusEventDef::new("mcp.tools.changed");

impl McpClient {
    pub fn new(server_name: String, tool_registry: Arc<McpToolRegistry>) -> Self {
        Self {
            server_name,
            transport: Mutex::new(None),
            request_id: AtomicU64::new(0),
            initialized: RwLock::new(false),
            capabilities: RwLock::new(None),
            tool_registry,
            timeout_ms: 30000,
            status: RwLock::new(McpStatus::Disabled),
            oauth_manager: RwLock::new(None),
            bus: None,
            tools_changed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn with_bus(mut self, bus: Arc<Bus>) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Attach an OAuth manager for token-based auth on HTTP/SSE transports.
    pub async fn set_oauth_manager(&self, manager: Arc<McpOAuthManager>) {
        let mut guard = self.oauth_manager.write().await;
        *guard = Some(manager);
    }

    // -- Status accessors ----------------------------------------------------

    pub async fn status(&self) -> McpStatus {
        self.status.read().await.clone()
    }

    pub async fn set_status(&self, status: McpStatus) {
        let mut guard = self.status.write().await;
        *guard = status;
    }

    // -- Factory helpers -----------------------------------------------------

    /// Create a client that communicates over stdio with a child process.
    pub async fn stdio(
        server_name: String,
        tool_registry: Arc<McpToolRegistry>,
        config: McpServerConfig,
    ) -> Result<Self, McpClientError> {
        let client = Self::new(server_name, tool_registry);
        client.connect_stdio(config).await?;
        Ok(client)
    }
    /// Create a client that communicates over StreamableHTTP.
    pub async fn http(
        server_name: String,
        tool_registry: Arc<McpToolRegistry>,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<Self, McpClientError> {
        let client = Self::new(server_name, tool_registry);
        client.connect_http(url, headers).await?;
        Ok(client)
    }

    /// Create a client that communicates over SSE.
    pub async fn sse(
        server_name: String,
        tool_registry: Arc<McpToolRegistry>,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<Self, McpClientError> {
        let client = Self::new(server_name, tool_registry);
        client.connect_sse(url, headers).await?;
        Ok(client)
    }

    // -- Connection methods ---------------------------------------------------

    pub async fn connect_stdio(&self, config: McpServerConfig) -> Result<(), McpClientError> {
        let result = self.connect_stdio_inner(config).await;
        match &result {
            Ok(()) => self.set_status(McpStatus::Connected).await,
            Err(e) => {
                self.set_status(McpStatus::Failed {
                    error: e.to_string(),
                })
                .await;
            }
        }
        result
    }

    async fn connect_stdio_inner(&self, config: McpServerConfig) -> Result<(), McpClientError> {
        let transport = StdioTransport::new(&config.command, &config.args, config.env).await?;
        {
            let mut t = self.transport.lock().await;
            *t = Some(Box::new(transport));
        }
        self.initialize().await?;
        self.load_tools().await?;
        Ok(())
    }
    pub async fn connect_http(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<(), McpClientError> {
        let result = self.connect_http_inner(url, headers).await;
        match &result {
            Ok(()) => self.set_status(McpStatus::Connected).await,
            Err(McpClientError::Unauthorized) => {
                self.set_status(McpStatus::NeedsAuth).await;
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("registration") || msg.contains("client_id") {
                    self.set_status(McpStatus::NeedsClientRegistration { error: msg })
                        .await;
                } else {
                    self.set_status(McpStatus::Failed { error: msg }).await;
                }
            }
        }
        result
    }

    async fn connect_http_inner(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<(), McpClientError> {
        // If we have an OAuth manager, try to inject the bearer token.
        let mut merged_headers = headers.unwrap_or_default();
        if let Some(mgr) = self.oauth_manager.read().await.as_ref() {
            match mgr.get_token().await {
                Ok(Some(token)) => {
                    merged_headers.insert("Authorization".to_string(), format!("Bearer {token}"));
                }
                Ok(None) => {
                    // No token available – caller should initiate auth.
                    return Err(McpClientError::Unauthorized);
                }
                Err(e) => {
                    return Err(McpClientError::OAuthError(e.to_string()));
                }
            }
        }

        let transport = HttpTransport::new(url, Some(merged_headers));
        {
            let mut t = self.transport.lock().await;
            *t = Some(Box::new(transport));
        }
        self.initialize().await?;
        self.load_tools().await?;
        Ok(())
    }
    pub async fn connect_sse(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<(), McpClientError> {
        let result = self.connect_sse_inner(url, headers).await;
        match &result {
            Ok(()) => self.set_status(McpStatus::Connected).await,
            Err(McpClientError::Unauthorized) => {
                self.set_status(McpStatus::NeedsAuth).await;
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("registration") || msg.contains("client_id") {
                    self.set_status(McpStatus::NeedsClientRegistration { error: msg })
                        .await;
                } else {
                    self.set_status(McpStatus::Failed { error: msg }).await;
                }
            }
        }
        result
    }

    async fn connect_sse_inner(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<(), McpClientError> {
        let mut merged_headers = headers.unwrap_or_default();
        if let Some(mgr) = self.oauth_manager.read().await.as_ref() {
            match mgr.get_token().await {
                Ok(Some(token)) => {
                    merged_headers.insert("Authorization".to_string(), format!("Bearer {token}"));
                }
                Ok(None) => return Err(McpClientError::Unauthorized),
                Err(e) => return Err(McpClientError::OAuthError(e.to_string())),
            }
        }

        let transport = SseTransport::new(url, Some(merged_headers));
        transport.connect().await?;
        {
            let mut t = self.transport.lock().await;
            *t = Some(Box::new(transport));
        }
        self.initialize().await?;
        self.load_tools().await?;
        Ok(())
    }

    // -- Internal helpers ----------------------------------------------------

    async fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::SeqCst) + 1
    }
    async fn send_request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse, McpClientError> {
        let id = self.next_id().await;
        let request = JsonRpcRequest::new(id, method);
        let request = if let Some(p) = params {
            request.with_params(p)
        } else {
            request
        };

        // Send under the lock, then release it.
        {
            let guard = self.transport.lock().await;
            let transport = guard.as_ref().ok_or(McpClientError::NotInitialized)?;
            transport.send(&request).await?;
        }

        // Re-acquire for receive.
        let guard = self.transport.lock().await;
        let transport = guard.as_ref().ok_or(McpClientError::NotInitialized)?;

        let response = loop {
            match transport.receive().await? {
                Some(JsonRpcMessage::Response(resp)) if resp.id == id => break resp,
                Some(JsonRpcMessage::Notification(notif)) => {
                    self.handle_notification(notif).await;
                    continue;
                }
                Some(_) => continue,
                None => {
                    return Err(McpClientError::TransportError(
                        "Connection closed".to_string(),
                    ));
                }
            }
        };

        if let Some(error) = response.error {
            return Err(McpClientError::ServerError(error.message));
        }

        Ok(response)
    }

    async fn send_request_with_progress_timeout(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        timeout_ms: u64,
    ) -> Result<JsonRpcResponse, McpClientError> {
        let id = self.next_id().await;
        let request = JsonRpcRequest::new(id, method);
        let request = if let Some(p) = params {
            request.with_params(p)
        } else {
            request
        };

        {
            let guard = self.transport.lock().await;
            let transport = guard.as_ref().ok_or(McpClientError::NotInitialized)?;
            transport.send(&request).await?;
        }

        let guard = self.transport.lock().await;
        let transport = guard.as_ref().ok_or(McpClientError::NotInitialized)?;
        let timeout_duration = Duration::from_millis(timeout_ms);
        let mut deadline = tokio::time::Instant::now() + timeout_duration;

        let response = loop {
            let message = match tokio::time::timeout_at(deadline, transport.receive()).await {
                Ok(result) => result?,
                Err(_) => return Err(McpClientError::Timeout),
            };

            match message {
                Some(JsonRpcMessage::Response(resp)) if resp.id == id => break resp,
                Some(JsonRpcMessage::Notification(notif)) => {
                    if Self::is_progress_notification(&notif) {
                        deadline = tokio::time::Instant::now() + timeout_duration;
                    }
                    self.handle_notification(notif).await;
                    continue;
                }
                Some(_) => continue,
                None => {
                    return Err(McpClientError::TransportError(
                        "Connection closed".to_string(),
                    ));
                }
            }
        };

        if let Some(error) = response.error {
            return Err(McpClientError::ServerError(error.message));
        }

        Ok(response)
    }

    async fn initialize(&self) -> Result<(), McpClientError> {
        let params = serde_json::to_value(InitializeParams::default())
            .map_err(|e| McpClientError::ProtocolError(e.to_string()))?;

        let response = self.send_request("initialize", Some(params)).await?;

        let result: InitializeResult = response
            .result
            .ok_or_else(|| McpClientError::ProtocolError("No result in initialize response".into()))
            .and_then(|r| {
                serde_json::from_value(r).map_err(|e| {
                    McpClientError::ProtocolError(format!("Failed to parse initialize result: {e}"))
                })
            })?;

        {
            let mut caps = self.capabilities.write().await;
            *caps = Some(result.capabilities);
        }

        self.send_request("notifications/initialized", None)
            .await
            .ok();

        {
            let mut init = self.initialized.write().await;
            *init = true;
        }

        Ok(())
    }

    /// Handle a server notification received during request/response.
    async fn handle_notification(&self, notif: JsonRpcNotification) {
        match notif.method.as_str() {
            "notifications/tools/list_changed" => {
                tracing::info!(
                    server = %self.server_name,
                    "MCP server tools changed, flagging for reload"
                );
                self.tools_changed.store(true, Ordering::SeqCst);
            }
            "notifications/resources/list_changed" => {
                tracing::debug!(
                    server = %self.server_name,
                    "MCP server resources changed (not yet handled)"
                );
            }
            "notifications/prompts/list_changed" => {
                tracing::debug!(
                    server = %self.server_name,
                    "MCP server prompts changed (not yet handled)"
                );
            }
            other => {
                tracing::debug!(
                    server = %self.server_name,
                    method = other,
                    "Unhandled MCP notification"
                );
            }
        }
    }

    fn is_progress_notification(notif: &JsonRpcNotification) -> bool {
        notif.method == "notifications/progress" || notif.method == "$/progress"
    }

    /// If the server sent a `tools/list_changed` notification, reload tools.
    /// Call this after operations that might trigger notifications.
    pub async fn refresh_tools_if_needed(&self) -> Result<(), McpClientError> {
        if self.tools_changed.swap(false, Ordering::SeqCst) {
            self.load_tools().await?;
            if let Some(bus) = &self.bus {
                bus.publish(
                    &MCP_TOOLS_CHANGED_EVENT,
                    serde_json::json!({ "server": self.server_name }),
                )
                .await;
            }
        }
        Ok(())
    }

    async fn load_tools(&self) -> Result<(), McpClientError> {
        let response = self.send_request("tools/list", None).await?;

        let result: ListToolsResult = response
            .result
            .ok_or_else(|| McpClientError::ProtocolError("No result in tools/list response".into()))
            .and_then(|r| {
                serde_json::from_value(r).map_err(|e| {
                    McpClientError::ProtocolError(format!("Failed to parse tools/list result: {e}"))
                })
            })?;

        self.tool_registry.clear_server(&self.server_name).await;
        self.tool_registry
            .register_batch(&self.server_name, result.tools)
            .await;

        Ok(())
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<CallToolResult, McpClientError> {
        let params = CallToolParams {
            name: name.to_string(),
            arguments,
        };

        let params_value = serde_json::to_value(params)
            .map_err(|e| McpClientError::ProtocolError(e.to_string()))?;

        let response = self
            .send_request_with_progress_timeout("tools/call", Some(params_value), self.timeout_ms)
            .await?;

        // After tool call, check if tools changed notification was received
        self.refresh_tools_if_needed().await.ok();

        let result: CallToolResult = response
            .result
            .ok_or_else(|| McpClientError::ProtocolError("No result in tools/call response".into()))
            .and_then(|r| {
                serde_json::from_value(r).map_err(|e| {
                    McpClientError::ProtocolError(format!("Failed to parse tools/call result: {e}"))
                })
            })?;

        Ok(result)
    }

    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, McpClientError> {
        let params = ReadResourceParams {
            uri: uri.to_string(),
        };
        let params_value = serde_json::to_value(params)
            .map_err(|e| McpClientError::ProtocolError(e.to_string()))?;

        let response = self
            .send_request("resources/read", Some(params_value))
            .await?;
        let result: ReadResourceResult = response
            .result
            .ok_or_else(|| {
                McpClientError::ProtocolError("No result in resources/read response".into())
            })
            .and_then(|r| {
                serde_json::from_value(r).map_err(|e| {
                    McpClientError::ProtocolError(format!(
                        "Failed to parse resources/read result: {e}"
                    ))
                })
            })?;

        Ok(result)
    }

    pub async fn close(&self) -> Result<(), McpClientError> {
        let mut transport = self.transport.lock().await;
        if let Some(t) = transport.as_ref() {
            t.close().await?;
        }
        *transport = None;

        self.tool_registry.clear_server(&self.server_name).await;
        self.set_status(McpStatus::Disabled).await;

        Ok(())
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub async fn is_initialized(&self) -> bool {
        *self.initialized.read().await
    }
}

// ---------------------------------------------------------------------------
// McpClientRegistry
// ---------------------------------------------------------------------------

pub struct McpClientRegistry {
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
    tool_registry: Arc<McpToolRegistry>,
    bus: Option<Arc<Bus>>,
    /// Per-server status, including servers that failed to connect or are
    /// disabled.  Entries here may not have a corresponding client.
    statuses: RwLock<HashMap<String, McpStatus>>,
    connection_configs: RwLock<HashMap<String, RegistryConnectionConfig>>,
    logs: RwLock<HashMap<String, Vec<String>>>,
}
impl McpClientRegistry {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            tool_registry: Arc::new(McpToolRegistry::new()),
            bus: None,
            statuses: RwLock::new(HashMap::new()),
            connection_configs: RwLock::new(HashMap::new()),
            logs: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_bus(mut self, bus: Arc<Bus>) -> Self {
        self.bus = Some(bus);
        self
    }

    // -- Status helpers ------------------------------------------------------

    /// Record the status for a server (called internally after connect
    /// attempts and also usable externally).
    pub async fn set_status(&self, name: &str, status: McpStatus) {
        self.statuses.write().await.insert(name.to_string(), status);
    }

    /// Get the status for a single server.
    pub async fn get_status(&self, name: &str) -> Option<McpStatus> {
        self.statuses.read().await.get(name).cloned()
    }

    /// Return all servers with their current status (including those that
    /// are not connected).
    pub async fn list_with_status(&self) -> Vec<(String, McpStatus)> {
        self.statuses
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    async fn log_event(&self, name: &str, message: impl Into<String>) {
        let line = format!("[{}] {}", Utc::now().to_rfc3339(), message.into());
        self.logs
            .write()
            .await
            .entry(name.to_string())
            .or_default()
            .push(line);
    }

    // -- Client management ---------------------------------------------------

    pub async fn add_stdio(
        &self,
        config: McpServerConfig,
    ) -> Result<Arc<McpClient>, McpClientError> {
        let name = config.name.clone();
        self.connection_configs.write().await.insert(
            name.clone(),
            RegistryConnectionConfig::Stdio(config.clone()),
        );
        self.log_event(&name, "Connecting via stdio").await;

        let timeout = config.timeout_ms;
        let mut client_impl = McpClient::new(name.clone(), self.tool_registry.clone());
        if let Some(timeout_ms) = timeout {
            client_impl = client_impl.with_timeout(timeout_ms);
        }
        if let Some(bus) = &self.bus {
            client_impl = client_impl.with_bus(bus.clone());
        }
        let client = Arc::new(client_impl);

        match client.connect_stdio(config).await {
            Ok(()) => {
                self.set_status(&name, McpStatus::Connected).await;
                self.clients.write().await.insert(name, client.clone());
                self.log_event(client.server_name(), "Connected").await;
                Ok(client)
            }
            Err(e) => {
                let status = client.status().await;
                self.set_status(&name, status).await;
                self.log_event(&name, format!("Connect failed: {}", e))
                    .await;
                Err(e)
            }
        }
    }
    pub async fn add_http(
        &self,
        name: String,
        url: String,
        headers: Option<HashMap<String, String>>,
        timeout_ms: Option<u64>,
    ) -> Result<Arc<McpClient>, McpClientError> {
        self.connection_configs.write().await.insert(
            name.clone(),
            RegistryConnectionConfig::Http {
                url: url.clone(),
                headers: headers.clone(),
                timeout_ms,
            },
        );
        self.log_event(&name, "Connecting via http").await;

        let mut client_impl = McpClient::new(name.clone(), self.tool_registry.clone());
        if let Some(t) = timeout_ms {
            client_impl = client_impl.with_timeout(t);
        }
        if let Some(bus) = &self.bus {
            client_impl = client_impl.with_bus(bus.clone());
        }
        let client = Arc::new(client_impl);

        match client.connect_http(url, headers).await {
            Ok(()) => {
                self.set_status(&name, McpStatus::Connected).await;
                self.clients.write().await.insert(name, client.clone());
                self.log_event(client.server_name(), "Connected").await;
                Ok(client)
            }
            Err(e) => {
                let status = client.status().await;
                self.set_status(&name, status).await;
                self.log_event(&name, format!("Connect failed: {}", e))
                    .await;
                Err(e)
            }
        }
    }

    pub async fn add_sse(
        &self,
        name: String,
        url: String,
        headers: Option<HashMap<String, String>>,
        timeout_ms: Option<u64>,
    ) -> Result<Arc<McpClient>, McpClientError> {
        self.connection_configs.write().await.insert(
            name.clone(),
            RegistryConnectionConfig::Sse {
                url: url.clone(),
                headers: headers.clone(),
                timeout_ms,
            },
        );
        self.log_event(&name, "Connecting via sse").await;

        let mut client_impl = McpClient::new(name.clone(), self.tool_registry.clone());
        if let Some(t) = timeout_ms {
            client_impl = client_impl.with_timeout(t);
        }
        if let Some(bus) = &self.bus {
            client_impl = client_impl.with_bus(bus.clone());
        }
        let client = Arc::new(client_impl);

        match client.connect_sse(url, headers).await {
            Ok(()) => {
                self.set_status(&name, McpStatus::Connected).await;
                self.clients.write().await.insert(name, client.clone());
                self.log_event(client.server_name(), "Connected").await;
                Ok(client)
            }
            Err(e) => {
                let status = client.status().await;
                self.set_status(&name, status).await;
                self.log_event(&name, format!("Connect failed: {}", e))
                    .await;
                Err(e)
            }
        }
    }

    /// Backwards-compatible alias for `add_stdio`.
    pub async fn add_client(
        &self,
        config: McpServerConfig,
    ) -> Result<Arc<McpClient>, McpClientError> {
        self.add_stdio(config).await
    }

    pub async fn get(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.read().await.get(name).cloned()
    }

    pub async fn remove(&self, name: &str) -> Result<(), McpClientError> {
        let client = self.clients.write().await.remove(name);
        if let Some(client) = client {
            client.close().await?;
        }
        self.set_status(name, McpStatus::Disabled).await;
        self.log_event(name, "Disconnected").await;
        Ok(())
    }

    pub async fn list(&self) -> Vec<(String, Arc<McpClient>)> {
        self.clients
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn tool_registry(&self) -> Arc<McpToolRegistry> {
        self.tool_registry.clone()
    }

    pub async fn get_logs(&self, name: &str) -> Vec<String> {
        self.logs
            .read()
            .await
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn restart(&self, name: &str) -> Result<Arc<McpClient>, McpClientError> {
        self.log_event(name, "Restart requested").await;

        let config = self
            .connection_configs
            .read()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| {
                McpClientError::ProtocolError(format!("No restart config found for {}", name))
            })?;

        if let Some(client) = self.clients.write().await.remove(name) {
            client.close().await?;
        }

        match config {
            RegistryConnectionConfig::Stdio(config) => self.add_stdio(config).await,
            RegistryConnectionConfig::Http {
                url,
                headers,
                timeout_ms,
            } => {
                self.add_http(name.to_string(), url, headers, timeout_ms)
                    .await
            }
            RegistryConnectionConfig::Sse {
                url,
                headers,
                timeout_ms,
            } => {
                self.add_sse(name.to_string(), url, headers, timeout_ms)
                    .await
            }
        }
    }
}

impl Default for McpClientRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use tokio::time::{sleep, timeout, Duration};

    struct MockTransport {
        messages: Mutex<VecDeque<(Duration, Option<JsonRpcMessage>)>>,
    }

    impl MockTransport {
        fn new(messages: Vec<(Duration, Option<JsonRpcMessage>)>) -> Self {
            Self {
                messages: Mutex::new(VecDeque::from(messages)),
            }
        }
    }

    #[async_trait]
    impl McpTransport for MockTransport {
        async fn send(&self, _request: &JsonRpcRequest) -> Result<(), McpClientError> {
            Ok(())
        }

        async fn receive(&self) -> Result<Option<JsonRpcMessage>, McpClientError> {
            let next = self.messages.lock().await.pop_front();
            match next {
                Some((delay, message)) => {
                    sleep(delay).await;
                    Ok(message)
                }
                None => Ok(None),
            }
        }

        async fn close(&self) -> Result<(), McpClientError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn call_tool_resets_timeout_on_progress_notification() {
        let tool_registry = Arc::new(McpToolRegistry::new());
        let client = McpClient::new("test-server".to_string(), tool_registry).with_timeout(30);

        let progress = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/progress".to_string(),
            params: Some(serde_json::json!({ "progress": 0.5 })),
        };
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: 1,
            result: Some(serde_json::json!({
                "content": [{ "type": "text", "text": "ok" }]
            })),
            error: None,
        };
        let transport = MockTransport::new(vec![
            (
                Duration::from_millis(15),
                Some(JsonRpcMessage::Notification(progress)),
            ),
            (
                Duration::from_millis(20),
                Some(JsonRpcMessage::Response(response)),
            ),
        ]);

        {
            let mut guard = client.transport.lock().await;
            *guard = Some(Box::new(transport));
        }

        let result = client
            .call_tool("slow-tool", Some(serde_json::json!({ "q": "value" })))
            .await
            .expect("tool call should complete before timeout when progress resets deadline");

        assert_eq!(result.content.len(), 1);
        assert_eq!(result.content[0].text.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn refresh_tools_if_needed_publishes_bus_event_and_reloads_tools() {
        let bus = Arc::new(Bus::new());
        let mut rx = bus.subscribe_channel();
        let tool_registry = Arc::new(McpToolRegistry::new());
        let client =
            McpClient::new("server-a".to_string(), tool_registry.clone()).with_bus(bus.clone());

        tool_registry
            .register(crate::tool::McpTool::new(
                "server-a",
                "stale",
                Some("stale tool".to_string()),
                serde_json::json!({ "type": "object" }),
            ))
            .await;

        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: 1,
            result: Some(serde_json::json!({
                "tools": [{ "name": "fresh", "description": "fresh tool", "inputSchema": { "type": "object" } }]
            })),
            error: None,
        };
        let transport = MockTransport::new(vec![(
            Duration::from_millis(0),
            Some(JsonRpcMessage::Response(response)),
        )]);

        {
            let mut guard = client.transport.lock().await;
            *guard = Some(Box::new(transport));
        }

        client
            .handle_notification(JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "notifications/tools/list_changed".to_string(),
                params: None,
            })
            .await;

        client
            .refresh_tools_if_needed()
            .await
            .expect("tools should refresh successfully");

        let event = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("event should arrive")
            .expect("event channel should be open");
        assert_eq!(event.event_type, MCP_TOOLS_CHANGED_EVENT.event_type);
        assert_eq!(event.properties["server"], "server-a");

        let tools = tool_registry.list_for_server("server-a").await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "fresh");
    }
}
