use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time: SessionTimeInfo,
    #[serde(default)]
    pub revert: Option<SessionRevertInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTimeInfo {
    pub created: i64,
    pub updated: i64,
    pub compacting: Option<i64>,
    pub archived: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRevertInfo {
    pub message_id: String,
    #[serde(default)]
    pub part_id: Option<String>,
    #[serde(default)]
    pub snapshot: Option<String>,
    #[serde(default)]
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatusInfo {
    pub status: String,
    pub idle: bool,
    pub busy: bool,
    #[serde(default)]
    pub attempt: Option<u32>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub next: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePart {
    pub id: String,
    #[serde(rename = "type")]
    pub part_type: String,
    pub text: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(alias = "toolCall")]
    pub tool_call: Option<ToolCall>,
    #[serde(alias = "toolResult")]
    pub tool_result: Option<ToolResult>,
    #[serde(default)]
    pub synthetic: Option<bool>,
    #[serde(default)]
    pub ignored: Option<bool>,
    #[serde(default, rename = "callID", alias = "call_id")]
    pub call_id: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub state: Option<ToolState>,
}

impl MessagePart {
    /// The server spells a file part with its fields flat on the part
    /// (`{"type":"file","url":...,"filename":...,"mime":...}`), not nested under a `file` key.
    pub fn file_info(&self) -> Option<FileInfo> {
        self.filename.as_ref().map(|filename| FileInfo {
            url: self.url.clone().unwrap_or_default(),
            filename: filename.clone(),
            mime: self.mime.clone().unwrap_or_default(),
        })
    }
}

/// The `state` object every `tool` part carries on the wire.
///
/// `status` is a plain `String` on purpose: a status this build has never heard of must not
/// fail deserialization of the whole transcript. Precedence lives in [`ToolState::status_rank`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolState {
    pub status: String,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
}

/// Lifecycle rank of a tool invocation: `pending < running < completed | error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolRank {
    Pending,
    Running,
    Terminal,
}

impl ToolState {
    /// `None` for a status this build does not know, so it always loses a fold against a
    /// recognized sibling rather than being mistaken for the most advanced state.
    pub fn status_rank(&self) -> Option<ToolRank> {
        match self.status.as_str() {
            "pending" => Some(ToolRank::Pending),
            "running" => Some(ToolRank::Running),
            "completed" | "error" => Some(ToolRank::Terminal),
            _ => None,
        }
    }

    pub fn is_error(&self) -> bool {
        self.status == "error"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub url: String,
    pub filename: String,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    #[serde(alias = "toolCallId")]
    pub tool_call_id: String,
    pub content: String,
    #[serde(alias = "isError")]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageInfoInner {
    pub id: String,
    #[serde(rename = "sessionID", alias = "sessionId")]
    pub session_id: String,
    pub role: String,
    #[serde(rename = "createdAt", alias = "created_at")]
    pub created_at: i64,
    #[serde(default, alias = "completedAt")]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub finish: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// The turn's outcome, which the server nests here instead of hoisting onto `info`:
    /// `metadata.error` and `metadata.finish_reason` on a failed turn, plus `metadata.model_provider`.
    /// Note that `agent` and `model_id` are NOT only here - `events.rs` also flattens them to
    /// top-level `info.agent` / `info.model`, so those two survive via the flat fields. The flat
    /// `finish`/`error` fields above, by contrast, are never populated by this server.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub tokens: MessageTokensInfo,
}

/// One message as the server sends it: identity in `info`, ordered content in `parts`.
///
/// `info` carries no `#[serde(default)]` on its identity fields on purpose. A flat or
/// malformed message must fail to deserialize rather than yield `id: ""` / `role: "user"` /
/// `createdAt: 0`, which a naive `is_ok()` check would wave through into a corrupt transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMessage {
    pub info: MessageInfoInner,
    #[serde(default)]
    pub parts: Vec<MessagePart>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageTokensInfo {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub reasoning: u64,
    #[serde(default, alias = "cacheRead")]
    pub cache_read: u64,
    #[serde(default, alias = "cacheWrite")]
    pub cache_write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRequest {
    pub message: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
    pub command: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteShellRequest {
    pub command: String,
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderListResponse {
    pub providers: Vec<ProviderInfo>,
    #[serde(rename = "default")]
    pub default_model: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub models: Vec<ProviderModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub variants: Vec<String>,
    #[serde(
        default,
        alias = "context_window",
        alias = "contextWindow",
        alias = "contextLength"
    )]
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpStatusInfo {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub resources: usize,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAuthStartInfo {
    pub authorization_url: String,
    pub client_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LspStatusResponse {
    servers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FormatterStatusResponse {
    formatters: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareResponse {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactResponse {
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertRequest {
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertResponse {
    pub success: bool,
}

pub struct ApiClient {
    client: Client,
    base_url: String,
    pub current_session: Arc<RwLock<Option<SessionInfo>>>,
}

impl ApiClient {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url,
            current_session: Arc::new(RwLock::new(None)),
        }
    }

    pub fn create_session(&self, parent_id: Option<String>) -> anyhow::Result<SessionInfo> {
        let url = format!("{}/session", self.base_url);
        let request = CreateSessionRequest { parent_id };

        let response = self.client.post(&url).json(&request).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to create session: {} - {}", status, text);
        }

        let session: SessionInfo = response.json()?;
        Ok(session)
    }

    pub fn get_session(&self, session_id: &str) -> anyhow::Result<SessionInfo> {
        let url = format!("{}/session/{}", self.base_url, session_id);

        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to get session: {} - {}", status, text);
        }

        let session: SessionInfo = response.json()?;
        Ok(session)
    }

    pub fn list_sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        self.list_sessions_filtered(None, None)
    }

    pub fn list_sessions_filtered(
        &self,
        search: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<SessionInfo>> {
        let url = format!("{}/session", self.base_url);
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(search) = search.map(str::trim).filter(|s| !s.is_empty()) {
            params.push(("search", search.to_string()));
        }
        if let Some(limit) = limit.filter(|l| *l > 0) {
            params.push(("limit", limit.to_string()));
        }

        let request = if params.is_empty() {
            self.client.get(&url)
        } else {
            self.client.get(&url).query(&params)
        };
        let response = request.send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to list sessions: {} - {}", status, text);
        }

        let sessions: Vec<SessionInfo> = response.json()?;
        Ok(sessions)
    }

    pub fn get_session_status(&self) -> anyhow::Result<HashMap<String, SessionStatusInfo>> {
        let url = format!("{}/session/status", self.base_url);
        let response = self.client.get(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to get session status: {} - {}", status, text);
        }
        Ok(response.json::<HashMap<String, SessionStatusInfo>>()?)
    }

    pub fn update_session_title(
        &self,
        session_id: &str,
        title: &str,
    ) -> anyhow::Result<SessionInfo> {
        let url = format!("{}/session/{}", self.base_url, session_id);
        let request = UpdateSessionRequest {
            title: Some(title.to_string()),
        };
        let response = self.client.patch(&url).json(&request).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!(
                "Failed to update session `{}` title: {} - {}",
                session_id,
                status,
                text
            );
        }
        let session: SessionInfo = response.json()?;
        Ok(session)
    }

    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let url = format!("{}/session/{}", self.base_url, session_id);
        let response = self.client.delete(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!(
                "Failed to delete session `{}`: {} - {}",
                session_id,
                status,
                text
            );
        }
        let value = response.json::<serde_json::Value>()?;
        Ok(value
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub fn send_prompt(
        &self,
        session_id: &str,
        content: String,
        agent: Option<String>,
        model: Option<String>,
        variant: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/session/{}/prompt", self.base_url, session_id);
        let request = PromptRequest {
            message: content,
            agent,
            model,
            variant,
            command: None,
            arguments: None,
        };

        let response = self.client.post(&url).json(&request).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to send prompt to {}: {} - {}", url, status, text);
        }

        let result: serde_json::Value = response.json()?;
        Ok(result)
    }

    pub fn execute_shell(
        &self,
        session_id: &str,
        command: String,
        workdir: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/session/{}/shell", self.base_url, session_id);
        let request = ExecuteShellRequest { command, workdir };
        let response = self.client.post(&url).json(&request).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to execute shell command: {} - {}", status, text);
        }

        Ok(response.json::<serde_json::Value>()?)
    }

    pub fn abort_session(&self, session_id: &str) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/session/{}/abort", self.base_url, session_id);
        let response = self.client.post(&url).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to abort session: {} - {}", status, text);
        }

        Ok(response.json::<serde_json::Value>()?)
    }

    pub fn get_config_providers(&self) -> anyhow::Result<ProviderListResponse> {
        let url = format!("{}/config/providers", self.base_url);

        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to get providers: {} - {}", status, text);
        }

        let providers: ProviderListResponse = response.json()?;
        Ok(providers)
    }

    pub fn list_agents(&self) -> anyhow::Result<Vec<AgentInfo>> {
        let url = format!("{}/agent", self.base_url);

        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to list agents: {} - {}", status, text);
        }

        let agents: Vec<AgentInfo> = response.json()?;
        Ok(agents)
    }

    pub fn list_skills(&self) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/skill", self.base_url);
        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to list skills: {} - {}", status, text);
        }

        Ok(response.json::<Vec<String>>()?)
    }

    pub fn get_mcp_status(&self) -> anyhow::Result<Vec<McpStatusInfo>> {
        let url = format!("{}/mcp", self.base_url);

        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to fetch MCP status: {} - {}", status, text);
        }

        let mut servers: Vec<McpStatusInfo> = response
            .json::<HashMap<String, McpStatusInfo>>()?
            .into_values()
            .collect();
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(servers)
    }

    pub fn start_mcp_auth(&self, name: &str) -> anyhow::Result<McpAuthStartInfo> {
        let url = format!("{}/mcp/{}/auth", self.base_url, name);
        let response = self.client.post(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to start MCP auth `{}`: {} - {}", name, status, text);
        }
        Ok(response.json::<McpAuthStartInfo>()?)
    }

    pub fn authenticate_mcp(&self, name: &str) -> anyhow::Result<McpStatusInfo> {
        let url = format!("{}/mcp/{}/auth/authenticate", self.base_url, name);
        let response = self.client.post(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!(
                "Failed to authenticate MCP `{}`: {} - {}",
                name,
                status,
                text
            );
        }
        Ok(response.json::<McpStatusInfo>()?)
    }

    pub fn remove_mcp_auth(&self, name: &str) -> anyhow::Result<bool> {
        let url = format!("{}/mcp/{}/auth", self.base_url, name);
        let response = self.client.delete(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!(
                "Failed to remove MCP auth `{}`: {} - {}",
                name,
                status,
                text
            );
        }
        let value = response.json::<serde_json::Value>()?;
        Ok(value
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub fn connect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        let url = format!("{}/mcp/{}/connect", self.base_url, name);
        let response = self.client.post(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to connect MCP `{}`: {} - {}", name, status, text);
        }
        Ok(response.json::<bool>().unwrap_or(true))
    }

    pub fn disconnect_mcp(&self, name: &str) -> anyhow::Result<bool> {
        let url = format!("{}/mcp/{}/disconnect", self.base_url, name);
        let response = self.client.post(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to disconnect MCP `{}`: {} - {}", name, status, text);
        }
        Ok(response.json::<bool>().unwrap_or(true))
    }

    pub fn get_messages(&self, session_id: &str) -> anyhow::Result<Vec<ApiMessage>> {
        let url = format!("{}/session/{}/message", self.base_url, session_id);

        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to get messages: {} - {}", status, text);
        }

        let body = response.text()?;
        serde_json::from_str(&body).map_err(|err| {
            anyhow::anyhow!(
                "GET /session/{}/message returned a body that is not a list of \
                 {{info, parts}} envelopes: {}",
                session_id,
                err
            )
        })
    }

    pub fn get_lsp_servers(&self) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/lsp", self.base_url);
        let response = self.client.get(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to get LSP status: {} - {}", status, text);
        }
        let status = response.json::<LspStatusResponse>()?;
        Ok(status.servers)
    }

    pub fn get_formatters(&self) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/formatter", self.base_url);
        let response = self.client.get(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("Failed to get formatter status: {} - {}", status, text);
        }
        let status = response.json::<FormatterStatusResponse>()?;
        Ok(status.formatters)
    }

    pub fn share_session(&self, session_id: &str) -> anyhow::Result<ShareResponse> {
        let url = format!("{}/session/{}/share", self.base_url, session_id);
        let response = self.client.post(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!(
                "Failed to share session `{}`: {} - {}",
                session_id,
                status,
                text
            );
        }
        Ok(response.json::<ShareResponse>()?)
    }

    pub fn unshare_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let url = format!("{}/session/{}/share", self.base_url, session_id);
        let response = self.client.delete(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!(
                "Failed to unshare session `{}`: {} - {}",
                session_id,
                status,
                text
            );
        }
        let value = response.json::<serde_json::Value>()?;
        Ok(value
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub fn compact_session(&self, session_id: &str) -> anyhow::Result<CompactResponse> {
        let url = format!("{}/session/{}/compact", self.base_url, session_id);
        let response = self.client.post(&url).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!(
                "Failed to compact session `{}`: {} - {}",
                session_id,
                status,
                text
            );
        }
        Ok(response.json::<CompactResponse>()?)
    }

    pub fn revert_session(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> anyhow::Result<RevertResponse> {
        let url = format!("{}/session/{}/revert", self.base_url, session_id);
        let request = RevertRequest {
            message_id: message_id.to_string(),
        };
        let response = self.client.post(&url).json(&request).send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!(
                "Failed to revert session `{}`: {} - {}",
                session_id,
                status,
                text
            );
        }
        Ok(response.json::<RevertResponse>()?)
    }

    pub fn fork_session(
        &self,
        session_id: &str,
        message_id: Option<&str>,
    ) -> anyhow::Result<SessionInfo> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(msg_id) = message_id {
            params.push(("message_id", msg_id.to_string()));
        }
        let url = format!("{}/session/{}/fork", self.base_url, session_id);
        let request = if params.is_empty() {
            self.client.post(&url)
        } else {
            self.client.post(&url).query(&params)
        };
        let response = request.send()?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!(
                "Failed to fork session `{}`: {} - {}",
                session_id,
                status,
                text
            );
        }
        Ok(response.json::<SessionInfo>()?)
    }

    pub fn set_current_session(&self, session: SessionInfo) {
        let mut current = futures::executor::block_on(self.current_session.write());
        *current = Some(session);
    }

    pub fn get_current_session(&self) -> Option<SessionInfo> {
        let current = futures::executor::block_on(self.current_session.read());
        current.clone()
    }
}
