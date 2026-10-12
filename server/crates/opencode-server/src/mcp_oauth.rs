use chrono::Utc;
use opencode_mcp::client::{McpClientRegistry, McpServerConfig as McpClientConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct LocalMcpConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<HashMap<String, String>>,
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RemoteMcpConfig {
    pub url: String,
    pub oauth_enabled: bool,
    pub client_id: Option<String>,
    pub authorization_url: Option<String>,
}

#[derive(Debug, Clone)]
pub enum McpRuntimeConfig {
    Local(LocalMcpConfig),
    Remote(RemoteMcpConfig),
}

impl McpRuntimeConfig {
    fn oauth_required(&self) -> bool {
        matches!(
            self,
            McpRuntimeConfig::Remote(RemoteMcpConfig {
                oauth_enabled: true,
                ..
            })
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthState {
    pub server_name: String,
    pub authorization_url: String,
    pub client_id: Option<String>,
    pub status: McpOAuthStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum McpOAuthStatus {
    Pending,
    Authorized,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub resources: usize,
    pub error: Option<String>,
    pub oauth_required: bool,
    pub oauth_status: Option<McpOAuthStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerLogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone)]
struct ManagedServer {
    config: McpRuntimeConfig,
    enabled: bool,
}

pub struct McpOAuthManager {
    oauth_states: Arc<RwLock<HashMap<String, McpOAuthState>>>,
    servers: Arc<RwLock<HashMap<String, ManagedServer>>>,
    statuses: Arc<RwLock<HashMap<String, McpServerInfo>>>,
    logs: Arc<RwLock<HashMap<String, Vec<McpServerLogEntry>>>>,
    clients: Arc<McpClientRegistry>,
}

impl McpOAuthManager {
    pub fn new() -> Self {
        Self {
            oauth_states: Arc::new(RwLock::new(HashMap::new())),
            servers: Arc::new(RwLock::new(HashMap::new())),
            statuses: Arc::new(RwLock::new(HashMap::new())),
            logs: Arc::new(RwLock::new(HashMap::new())),
            clients: Arc::new(McpClientRegistry::new()),
        }
    }

    async fn log_event(
        &self,
        server_name: &str,
        level: impl Into<String>,
        message: impl Into<String>,
    ) {
        let entry = McpServerLogEntry {
            timestamp: Utc::now().to_rfc3339(),
            level: level.into(),
            message: message.into(),
        };
        self.logs
            .write()
            .await
            .entry(server_name.to_string())
            .or_default()
            .push(entry);
    }

    pub async fn has_server(&self, server_name: &str) -> bool {
        self.servers.read().await.contains_key(server_name)
    }

    pub async fn add_server(
        &self,
        server_name: String,
        config: McpRuntimeConfig,
        enabled: bool,
    ) -> McpServerInfo {
        let managed = ManagedServer { config, enabled };
        self.servers
            .write()
            .await
            .insert(server_name.clone(), managed.clone());

        let status = self
            .default_status_for(
                &server_name,
                &managed,
                self.oauth_status_for(&server_name).await,
            )
            .await;
        self.statuses
            .write()
            .await
            .insert(server_name.clone(), status.clone());
        self.log_event(&server_name, "info", "Server registered")
            .await;
        status
    }

    pub async fn connect(&self, server_name: &str) -> Result<McpServerInfo, McpOAuthError> {
        self.log_event(server_name, "info", "Connect requested")
            .await;
        let managed = self.managed_server(server_name).await?;
        let oauth_status = self.oauth_status_for(server_name).await;
        let oauth_required = managed.config.oauth_required();

        if !managed.enabled {
            let info = McpServerInfo {
                name: server_name.to_string(),
                status: "disabled".to_string(),
                tools: 0,
                resources: 0,
                error: None,
                oauth_required,
                oauth_status,
            };
            self.statuses
                .write()
                .await
                .insert(server_name.to_string(), info.clone());
            self.log_event(server_name, "info", "Server is disabled")
                .await;
            return Ok(info);
        }

        let info = match managed.config {
            McpRuntimeConfig::Local(local) => {
                if let Err(error) = self.clients.remove(server_name).await {
                    tracing::warn!(server = server_name, %error, "failed to remove existing MCP client");
                }

                let env = local
                    .env
                    .map(|map| map.into_iter().collect::<Vec<(String, String)>>());

                match self
                    .clients
                    .add_client(McpClientConfig {
                        name: server_name.to_string(),
                        command: local.command,
                        args: local.args,
                        env,
                        timeout_ms: local.timeout,
                    })
                    .await
                {
                    Ok(_) => {
                        let tool_count = self
                            .clients
                            .tool_registry()
                            .list_for_server(server_name)
                            .await
                            .len();
                        self.log_event(server_name, "info", "Connected local MCP server")
                            .await;
                        McpServerInfo {
                            name: server_name.to_string(),
                            status: "connected".to_string(),
                            tools: tool_count,
                            resources: 0,
                            error: None,
                            oauth_required,
                            oauth_status: None,
                        }
                    }
                    Err(error) => {
                        self.log_event(server_name, "error", format!("Connect failed: {}", error))
                            .await;
                        McpServerInfo {
                            name: server_name.to_string(),
                            status: "failed".to_string(),
                            tools: 0,
                            resources: 0,
                            error: Some(error.to_string()),
                            oauth_required,
                            oauth_status: None,
                        }
                    }
                }
            }
            McpRuntimeConfig::Remote(_) => {
                if oauth_required && oauth_status != Some(McpOAuthStatus::Authorized) {
                    self.log_event(server_name, "info", "Remote MCP requires OAuth")
                        .await;
                    McpServerInfo {
                        name: server_name.to_string(),
                        status: "needs_auth".to_string(),
                        tools: 0,
                        resources: 0,
                        error: None,
                        oauth_required,
                        oauth_status,
                    }
                } else {
                    self.log_event(
                        server_name,
                        "error",
                        "Remote MCP transport is not implemented in opencode-rust-rewrite",
                    )
                    .await;
                    McpServerInfo {
                        name: server_name.to_string(),
                        status: "failed".to_string(),
                        tools: 0,
                        resources: 0,
                        error: Some(
                            "Remote MCP transport is not implemented in opencode-rust-rewrite"
                                .to_string(),
                        ),
                        oauth_required,
                        oauth_status,
                    }
                }
            }
        };

        self.statuses
            .write()
            .await
            .insert(server_name.to_string(), info.clone());
        self.log_event(
            server_name,
            "info",
            format!("Status updated: {}", info.status),
        )
        .await;
        Ok(info)
    }

    pub async fn disconnect(&self, server_name: &str) -> Result<McpServerInfo, McpOAuthError> {
        self.log_event(server_name, "info", "Disconnect requested")
            .await;
        let managed = self.managed_server(server_name).await?;
        if let McpRuntimeConfig::Local(_) = managed.config {
            self.clients
                .remove(server_name)
                .await
                .map_err(|e| McpOAuthError::RuntimeError(e.to_string()))?;
        }

        let info = McpServerInfo {
            name: server_name.to_string(),
            status: "disabled".to_string(),
            tools: 0,
            resources: 0,
            error: None,
            oauth_required: managed.config.oauth_required(),
            oauth_status: self.oauth_status_for(server_name).await,
        };
        self.statuses
            .write()
            .await
            .insert(server_name.to_string(), info.clone());
        self.log_event(server_name, "info", "Disconnected").await;
        Ok(info)
    }

    pub async fn start_oauth(&self, server_name: &str) -> Result<McpOAuthState, McpOAuthError> {
        let managed = self.managed_server(server_name).await?;
        let remote = match managed.config {
            McpRuntimeConfig::Remote(remote) => remote,
            McpRuntimeConfig::Local(_) => {
                return Err(McpOAuthError::OAuthNotSupported(server_name.to_string()));
            }
        };

        if !remote.oauth_enabled {
            return Err(McpOAuthError::OAuthNotSupported(server_name.to_string()));
        }

        if self
            .oauth_states
            .read()
            .await
            .get(server_name)
            .is_some_and(|state| state.status == McpOAuthStatus::Pending)
        {
            return Err(McpOAuthError::OAuthInProgress);
        }

        let authorization_url = remote
            .authorization_url
            .unwrap_or_else(|| format!("{}/oauth/authorize", remote.url.trim_end_matches('/')));

        let state = McpOAuthState {
            server_name: server_name.to_string(),
            authorization_url,
            client_id: remote
                .client_id
                .or_else(|| Some(format!("mcp_client_{}", server_name))),
            status: McpOAuthStatus::Pending,
        };

        self.oauth_states
            .write()
            .await
            .insert(server_name.to_string(), state.clone());
        self.log_event(server_name, "info", "OAuth flow started")
            .await;

        let mut statuses = self.statuses.write().await;
        let info = statuses
            .entry(server_name.to_string())
            .or_insert_with(|| McpServerInfo {
                name: server_name.to_string(),
                status: "needs_auth".to_string(),
                tools: 0,
                resources: 0,
                error: None,
                oauth_required: true,
                oauth_status: Some(McpOAuthStatus::Pending),
            });
        info.status = "needs_auth".to_string();
        info.error = None;
        info.oauth_required = true;
        info.oauth_status = Some(McpOAuthStatus::Pending);
        drop(statuses);

        Ok(state)
    }

    pub async fn handle_callback(
        &self,
        server_name: &str,
        _code: &str,
    ) -> Result<McpServerInfo, McpOAuthError> {
        self.managed_server(server_name).await?;

        let mut states = self.oauth_states.write().await;
        let state = states
            .get_mut(server_name)
            .ok_or_else(|| McpOAuthError::OAuthFailed("No pending OAuth flow".to_string()))?;
        state.status = McpOAuthStatus::Authorized;
        drop(states);
        self.log_event(server_name, "info", "OAuth callback completed")
            .await;

        self.connect(server_name).await
    }

    pub async fn authenticate(&self, server_name: &str) -> Result<McpServerInfo, McpOAuthError> {
        let managed = self.managed_server(server_name).await?;
        if !managed.config.oauth_required() {
            return Err(McpOAuthError::OAuthNotSupported(server_name.to_string()));
        }

        let current = self.oauth_status_for(server_name).await;
        if current == Some(McpOAuthStatus::Authorized) {
            self.log_event(server_name, "info", "OAuth already authorized")
                .await;
            return self.connect(server_name).await;
        }

        self.start_oauth(server_name).await?;
        self.connect(server_name).await
    }

    pub async fn get_server(&self, server_name: &str) -> Option<McpServerInfo> {
        if let Some(status) = self.statuses.read().await.get(server_name) {
            return Some(status.clone());
        }

        let managed = self.servers.read().await.get(server_name).cloned()?;
        Some(
            self.default_status_for(
                server_name,
                &managed,
                self.oauth_status_for(server_name).await,
            )
            .await,
        )
    }

    pub async fn list_servers(&self) -> Vec<McpServerInfo> {
        let servers = self.servers.read().await.clone();
        let statuses = self.statuses.read().await.clone();

        let mut out = Vec::with_capacity(servers.len());
        for (name, managed) in servers {
            if let Some(status) = statuses.get(&name) {
                out.push(status.clone());
                continue;
            }
            out.push(
                self.default_status_for(&name, &managed, self.oauth_status_for(&name).await)
                    .await,
            );
        }
        out
    }

    pub async fn get_logs(
        &self,
        server_name: &str,
    ) -> Result<Vec<McpServerLogEntry>, McpOAuthError> {
        self.managed_server(server_name).await?;
        Ok(self
            .logs
            .read()
            .await
            .get(server_name)
            .cloned()
            .unwrap_or_default())
    }

    pub async fn restart(&self, server_name: &str) -> Result<McpServerInfo, McpOAuthError> {
        self.managed_server(server_name).await?;
        self.log_event(server_name, "info", "Restart requested")
            .await;
        let _ = self.disconnect(server_name).await;
        self.connect(server_name).await
    }

    pub async fn remove_oauth(&self, server_name: &str) -> bool {
        let removed = self
            .oauth_states
            .write()
            .await
            .remove(server_name)
            .is_some();
        self.log_event(server_name, "info", "OAuth state removed")
            .await;

        if let Some(managed) = self.servers.read().await.get(server_name).cloned() {
            let mut info = self
                .statuses
                .read()
                .await
                .get(server_name)
                .cloned()
                .unwrap_or_else(|| McpServerInfo {
                    name: server_name.to_string(),
                    status: "disabled".to_string(),
                    tools: 0,
                    resources: 0,
                    error: None,
                    oauth_required: managed.config.oauth_required(),
                    oauth_status: None,
                });

            info.oauth_status = None;
            if managed.config.oauth_required() {
                info.status = if managed.enabled {
                    "needs_auth".to_string()
                } else {
                    "disabled".to_string()
                };
                info.error = None;
            }
            self.statuses
                .write()
                .await
                .insert(server_name.to_string(), info);
        }

        removed
    }

    async fn managed_server(&self, server_name: &str) -> Result<ManagedServer, McpOAuthError> {
        self.servers
            .read()
            .await
            .get(server_name)
            .cloned()
            .ok_or_else(|| McpOAuthError::ServerNotFound(server_name.to_string()))
    }

    async fn oauth_status_for(&self, server_name: &str) -> Option<McpOAuthStatus> {
        self.oauth_states
            .read()
            .await
            .get(server_name)
            .map(|state| state.status.clone())
    }

    async fn default_status_for(
        &self,
        server_name: &str,
        managed: &ManagedServer,
        oauth_status: Option<McpOAuthStatus>,
    ) -> McpServerInfo {
        let oauth_required = managed.config.oauth_required();
        let status = if !managed.enabled {
            "disabled"
        } else if oauth_required && oauth_status != Some(McpOAuthStatus::Authorized) {
            "needs_auth"
        } else {
            "disabled"
        };

        McpServerInfo {
            name: server_name.to_string(),
            status: status.to_string(),
            tools: 0,
            resources: 0,
            error: None,
            oauth_required,
            oauth_status,
        }
    }
}

impl Default for McpOAuthManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum McpOAuthError {
    #[error("MCP server not found: {0}")]
    ServerNotFound(String),

    #[error("MCP server does not support OAuth: {0}")]
    OAuthNotSupported(String),

    #[error("OAuth already in progress")]
    OAuthInProgress,

    #[error("OAuth failed: {0}")]
    OAuthFailed(String),

    #[error("MCP runtime error: {0}")]
    RuntimeError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn remote_server_requires_auth_before_connecting() {
        let manager = McpOAuthManager::new();
        manager
            .add_server(
                "remote".to_string(),
                McpRuntimeConfig::Remote(RemoteMcpConfig {
                    url: "https://example.com/mcp".to_string(),
                    oauth_enabled: true,
                    client_id: None,
                    authorization_url: None,
                }),
                true,
            )
            .await;

        let status = manager
            .connect("remote")
            .await
            .expect("connect should return runtime status");
        assert_eq!(status.status, "needs_auth");
        assert_eq!(status.oauth_status, None);
    }

    #[tokio::test]
    async fn oauth_callback_marks_remote_server_authorized() {
        let manager = McpOAuthManager::new();
        manager
            .add_server(
                "remote".to_string(),
                McpRuntimeConfig::Remote(RemoteMcpConfig {
                    url: "https://example.com/mcp".to_string(),
                    oauth_enabled: true,
                    client_id: None,
                    authorization_url: Some("https://idp.example.com/oauth/authorize".to_string()),
                }),
                true,
            )
            .await;

        let oauth_state = manager
            .start_oauth("remote")
            .await
            .expect("oauth should start");
        assert_eq!(
            oauth_state.authorization_url,
            "https://idp.example.com/oauth/authorize"
        );

        let status = manager
            .handle_callback("remote", "oauth-code")
            .await
            .expect("callback should update oauth state");
        assert_eq!(status.oauth_status, Some(McpOAuthStatus::Authorized));
    }

    #[tokio::test]
    async fn remove_oauth_reverts_status_to_needs_auth() {
        let manager = McpOAuthManager::new();
        manager
            .add_server(
                "remote".to_string(),
                McpRuntimeConfig::Remote(RemoteMcpConfig {
                    url: "https://example.com/mcp".to_string(),
                    oauth_enabled: true,
                    client_id: None,
                    authorization_url: None,
                }),
                true,
            )
            .await;

        manager
            .start_oauth("remote")
            .await
            .expect("oauth should start");
        manager
            .handle_callback("remote", "oauth-code")
            .await
            .expect("callback should authorize");

        assert!(manager.remove_oauth("remote").await);

        let status = manager
            .get_server("remote")
            .await
            .expect("remote server should exist");
        assert_eq!(status.status, "needs_auth");
        assert_eq!(status.oauth_status, None);
    }
}
