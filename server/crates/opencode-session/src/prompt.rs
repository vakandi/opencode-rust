use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use base64::Engine;
use futures::StreamExt;
use opencode_plugin::{HookContext, HookEvent};
use opencode_provider::transform::{apply_caching, ProviderType};
use opencode_provider::{
    get_model_context_limit, ChatRequest, ChatResponse, Content, ContentPart, Message, Provider,
    Role, StreamEvent, ToolDefinition,
};

use crate::compaction::{
    CompactionConfig, CompactionEngine, MessageForPrune, ModelLimits, PruneToolPart, TokenUsage,
    ToolPartStatus,
};
use crate::message_v2::{
    AssistantTime, AssistantTokens, CacheTokens, CompactionPart as V2CompactionPart, MessageInfo,
    MessagePath, MessageWithParts, ModelRef as V2ModelRef, Part as V2Part, StepFinishPart,
    StepStartPart, StepTokens, UserTime,
};
use crate::summary::{summarize_into_session, SummarizeInput};
use crate::system::SystemPrompt;
use crate::{MessagePart, MessageRole, PartType, Session, SessionMessage, SessionStateManager};

const MAX_STEPS: u32 = 100;

/// Deliberately far below `MAX_STEPS`: a subagent must not burn the parent's budget.
pub const SUBAGENT_MAX_STEPS: u32 = 24;

fn env_secs(name: &str, default_secs: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default_secs)
}

/// Per provider request. A normal agent turn finishes in seconds; this only fires on a
/// stall, so it can be generous without risking the parent.
pub fn subagent_request_timeout() -> Duration {
    Duration::from_secs(env_secs("OPENCODE_SUBAGENT_REQUEST_TIMEOUT_SECS", 120))
}

/// Wall-clock bound for the whole subagent turn, so 24 slow steps still cannot hang.
pub fn subagent_total_timeout() -> Duration {
    Duration::from_secs(env_secs("OPENCODE_SUBAGENT_TOTAL_TIMEOUT_SECS", 900))
}

#[derive(Debug, Clone)]
pub struct PromptInput {
    pub session_id: String,
    pub message_id: Option<String>,
    pub model: Option<ModelRef>,
    pub agent: Option<String>,
    pub no_reply: bool,
    pub system: Option<String>,
    pub variant: Option<String>,
    pub parts: Vec<PartInput>,
    pub tools: Option<HashMap<String, bool>>,
    pub permission: Option<opencode_permission::PermissionRuleset>,
}

#[derive(Debug, Clone)]
pub struct ModelRef {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PartInput {
    Text {
        text: String,
    },
    File {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
    Agent {
        name: String,
    },
    Subtask {
        prompt: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        agent: String,
    },
}

impl TryFrom<serde_json::Value> for PartInput {
    type Error = String;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value).map_err(|e| format!("Invalid PartInput: {}", e))
    }
}

impl PartInput {
    /// Parse a JSON array of parts into a Vec<PartInput>, skipping invalid entries.
    pub fn parse_array(value: &serde_json::Value) -> Vec<PartInput> {
        match value.as_array() {
            Some(arr) => arr
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect(),
            None => Vec::new(),
        }
    }
}

struct PromptState {
    cancel_token: CancellationToken,
}

#[derive(Debug, Clone)]
struct PendingSubtask {
    part_index: usize,
    subtask_id: String,
    agent: String,
    prompt: String,
    description: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct PersistedSubsession {
    agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default)]
    disabled_tools: Vec<String>,
    #[serde(default)]
    history: Vec<PersistedSubsessionTurn>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedSubsessionTurn {
    prompt: String,
    output: String,
}

/// LLM parameters derived from agent configuration.
#[derive(Debug, Clone, Default)]
pub struct AgentParams {
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

pub type SessionUpdateHook = Arc<dyn Fn(&Session) + Send + Sync + 'static>;

pub struct SessionPrompt {
    state: Arc<Mutex<HashMap<String, PromptState>>>,
    session_state: Arc<RwLock<SessionStateManager>>,
    mcp_clients: Option<Arc<opencode_mcp::McpClientRegistry>>,
    lsp_registry: Option<Arc<opencode_lsp::LspClientRegistry>>,
}

impl SessionPrompt {
    pub fn new(session_state: Arc<RwLock<SessionStateManager>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            session_state,
            mcp_clients: None,
            lsp_registry: None,
        }
    }

    pub fn with_mcp_clients(mut self, clients: Arc<opencode_mcp::McpClientRegistry>) -> Self {
        self.mcp_clients = Some(clients);
        self
    }

    pub fn with_lsp_registry(mut self, registry: Arc<opencode_lsp::LspClientRegistry>) -> Self {
        self.lsp_registry = Some(registry);
        self
    }

    pub async fn assert_not_busy(&self, session_id: &str) -> anyhow::Result<()> {
        let state = self.state.lock().await;
        if state.contains_key(session_id) {
            return Err(anyhow::anyhow!("Session {} is busy", session_id));
        }
        Ok(())
    }

    pub async fn create_user_message(
        &self,
        input: &PromptInput,
        session: &mut Session,
    ) -> anyhow::Result<()> {
        // Collect text parts for the primary message
        let text = input
            .parts
            .iter()
            .filter_map(|p| match p {
                PartInput::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let has_non_text = input
            .parts
            .iter()
            .any(|p| !matches!(p, PartInput::Text { .. }));

        if text.is_empty() && !has_non_text {
            return Err(anyhow::anyhow!("No content in prompt"));
        }

        let project_root = session.directory.clone();

        // Create the user message with text (or empty if only non-text parts)
        let msg = if text.is_empty() {
            session.add_user_message(" ")
        } else {
            session.add_user_message(&text)
        };

        // Add non-text parts to the message
        for part in &input.parts {
            match part {
                PartInput::Text { .. } => {} // already handled above
                PartInput::File {
                    url,
                    filename,
                    mime,
                } => {
                    self.add_file_part(
                        msg,
                        url,
                        filename.as_deref(),
                        mime.as_deref(),
                        &project_root,
                    )
                    .await;
                }
                PartInput::Agent { name } => {
                    msg.add_agent(name.clone());
                    // Add synthetic text instructing the LLM to invoke the agent
                    msg.add_text(format!(
                        "Use the above message and context to generate a prompt and call the task tool with subagent: {}",
                        name
                    ));
                }
                PartInput::Subtask {
                    prompt,
                    description,
                    agent,
                } => {
                    let subtask_id = format!("sub_{}", uuid::Uuid::new_v4());
                    let description = description.clone().unwrap_or_else(|| prompt.clone());
                    msg.add_subtask(subtask_id.clone(), description.clone());
                    let mut pending = msg
                        .metadata
                        .get("pending_subtasks")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    pending.push(serde_json::json!({
                        "id": subtask_id,
                        "agent": agent,
                        "prompt": prompt,
                        "description": description,
                    }));
                    msg.metadata.insert(
                        "pending_subtasks".to_string(),
                        serde_json::Value::Array(pending),
                    );
                }
            }
        }

        Ok(())
    }

    async fn add_file_part(
        &self,
        msg: &mut SessionMessage,
        raw_url: &str,
        filename: Option<&str>,
        mime: Option<&str>,
        project_root: &str,
    ) {
        let filename = filename
            .filter(|name| !name.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| Self::filename_from_url(raw_url));
        let mime = mime
            .filter(|m| !m.is_empty())
            .unwrap_or("application/octet-stream")
            .to_string();

        if raw_url.starts_with("mcp://") {
            self.add_mcp_resource_part(msg, raw_url, &filename, &mime)
                .await;
            return;
        }

        if raw_url.starts_with("data:") {
            self.add_data_url_part(msg, raw_url, &filename, &mime).await;
            return;
        }

        if raw_url.starts_with("file://") {
            self.add_file_url_part(msg, raw_url, &filename, &mime, project_root)
                .await;
            return;
        }

        msg.add_file(raw_url.to_string(), filename, mime);
    }

    async fn add_mcp_resource_part(
        &self,
        msg: &mut SessionMessage,
        raw_url: &str,
        filename: &str,
        mime: &str,
    ) {
        let Some((client_name, uri)) = Self::parse_mcp_resource_url(raw_url) else {
            msg.add_text(format!("Failed to parse MCP resource URL: {}", raw_url));
            msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
            return;
        };

        msg.add_text(format!("Reading MCP resource: {} ({})", filename, uri));

        let Some(registry) = &self.mcp_clients else {
            msg.add_text(
                "MCP client registry is not configured; unable to read resource content."
                    .to_string(),
            );
            msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
            return;
        };

        let Some(client) = registry.get(&client_name).await else {
            msg.add_text(format!("MCP client `{}` is not connected.", client_name));
            msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
            return;
        };

        match client.read_resource(&uri).await {
            Ok(result) => {
                let mut text_chunks = Vec::new();
                let mut binary_chunks = Vec::new();
                for content in result.contents {
                    if let Some(text) = content.text {
                        if !text.trim().is_empty() {
                            text_chunks.push(text);
                        }
                        continue;
                    }

                    if content.blob.is_some() {
                        binary_chunks.push(
                            content
                                .mime_type
                                .clone()
                                .unwrap_or_else(|| mime.to_string()),
                        );
                    }
                }

                if !text_chunks.is_empty() {
                    msg.add_text(SystemPrompt::mcp_resource_reminder(
                        filename,
                        &uri,
                        &text_chunks.join("\n\n"),
                    ));
                }

                let has_binary = !binary_chunks.is_empty();
                for mime in binary_chunks {
                    msg.add_text(format!("[Binary content: {}]", mime));
                }

                if text_chunks.is_empty() && !has_binary {
                    msg.add_text(format!("MCP resource `{}` returned no readable text.", uri));
                }
            }
            Err(err) => {
                msg.add_text(format!("Failed to read MCP resource `{}`: {}", uri, err));
            }
        }

        msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
    }

    async fn add_data_url_part(
        &self,
        msg: &mut SessionMessage,
        raw_url: &str,
        filename: &str,
        mime: &str,
    ) {
        if let Some(text) = Self::decode_data_url_text(raw_url, mime) {
            msg.add_text(format!(
                "Called the Read tool with the following input: {}",
                serde_json::json!({ "filePath": filename })
            ));
            msg.add_text(text);
        }

        msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
    }

    async fn add_file_url_part(
        &self,
        msg: &mut SessionMessage,
        raw_url: &str,
        filename: &str,
        mime: &str,
        project_root: &str,
    ) {
        let parsed = match url::Url::parse(raw_url) {
            Ok(url) => url,
            Err(err) => {
                msg.add_text(format!("Invalid file URL `{}`: {}", raw_url, err));
                msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
                return;
            }
        };

        let file_path = match parsed.to_file_path() {
            Ok(path) => path,
            Err(_) => {
                msg.add_text(format!("Invalid file path URL `{}`", raw_url));
                msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
                return;
            }
        };

        let metadata = match tokio::fs::metadata(&file_path).await {
            Ok(meta) => meta,
            Err(err) => {
                msg.add_text(format!(
                    "Read tool failed to read {} with error: {}",
                    file_path.display(),
                    err
                ));
                msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
                return;
            }
        };

        if metadata.is_dir() {
            let listing = Self::read_directory_preview(&file_path).await;
            msg.add_text(format!(
                "Called the Read tool with the following input: {}",
                serde_json::json!({ "filePath": file_path.display().to_string() })
            ));
            msg.add_text(listing);
            msg.add_file(
                raw_url.to_string(),
                filename.to_string(),
                "application/x-directory".to_string(),
            );
            return;
        }

        let bytes = match tokio::fs::read(&file_path).await {
            Ok(bytes) => bytes,
            Err(err) => {
                msg.add_text(format!(
                    "Read tool failed to read {} with error: {}",
                    file_path.display(),
                    err
                ));
                msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
                return;
            }
        };

        if Self::is_binary_asset_mime(mime) {
            let data_url = format!(
                "data:{};base64,{}",
                mime,
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
            msg.add_file(data_url, filename.to_string(), mime.to_string());
            return;
        }

        let mut text = String::from_utf8_lossy(&bytes).to_string();
        let mut read_args = serde_json::json!({
            "filePath": file_path.display().to_string(),
        });

        if let Some((start, end)) = self.resolve_file_line_window(&file_path, &parsed).await {
            text = Self::slice_lines(&text, start, end);
            if let Some(obj) = read_args.as_object_mut() {
                obj.insert("offset".to_string(), serde_json::json!(start));
                if let Some(end) = end {
                    obj.insert(
                        "limit".to_string(),
                        serde_json::json!(end.saturating_sub(start).saturating_add(1)),
                    );
                }
            }
        }

        msg.add_text(format!(
            "Called the Read tool with the following input: {}",
            read_args
        ));
        msg.add_text(text);
        Self::inject_instruction_prompt(msg, &file_path, Path::new(project_root));
        msg.add_file(raw_url.to_string(), filename.to_string(), mime.to_string());
    }

    fn inject_instruction_prompt(msg: &mut SessionMessage, file_path: &Path, project_root: &Path) {
        let mut loaded = Self::loaded_instruction_paths(msg);
        let mut prompt_chunks = Vec::new();

        for instruction in crate::instruction::resolve_agents_for_file(file_path, project_root) {
            if loaded.insert(instruction.path.clone()) {
                prompt_chunks.push(format!(
                    "Instructions from: {}\n{}",
                    instruction.path, instruction.content
                ));
            }
        }

        if prompt_chunks.is_empty() {
            return;
        }

        msg.add_text(SystemPrompt::system_reminder(&prompt_chunks.join("\n\n")));
        Self::store_loaded_instruction_paths(msg, loaded);
    }

    fn loaded_instruction_paths(msg: &SessionMessage) -> HashSet<String> {
        msg.metadata
            .get("loaded_instruction_files")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn store_loaded_instruction_paths(msg: &mut SessionMessage, loaded: HashSet<String>) {
        if loaded.is_empty() {
            return;
        }

        let mut paths: Vec<String> = loaded.into_iter().collect();
        paths.sort();
        msg.metadata.insert(
            "loaded_instruction_files".to_string(),
            serde_json::json!(paths),
        );
    }

    async fn resolve_file_line_window(
        &self,
        file_path: &Path,
        file_url: &url::Url,
    ) -> Option<(usize, Option<usize>)> {
        let (start, mut end) = Self::parse_line_window(file_url)?;
        if end == Some(start) {
            if let Some(symbol_end) = self.lookup_symbol_end_line(file_path, start).await {
                end = Some(symbol_end);
            }
        }
        Some((start, end))
    }

    async fn lookup_symbol_end_line(&self, file_path: &Path, start_line: usize) -> Option<usize> {
        let registry = self.lsp_registry.as_ref()?;
        let clients = registry.list().await;
        if clients.is_empty() {
            return None;
        }

        let content = tokio::fs::read_to_string(file_path).await.ok();
        for (_, client) in clients {
            if let Some(content) = content.as_deref() {
                let language = opencode_lsp::detect_language(file_path);
                let _ = client.open_document(file_path, content, language).await;
            }

            let symbols = match client.document_symbol(file_path).await {
                Ok(symbols) => symbols,
                Err(_) => continue,
            };

            for symbol in symbols {
                let symbol_start = symbol.location.range.start.line as usize + 1;
                if symbol_start != start_line {
                    continue;
                }

                let symbol_end = symbol.location.range.end.line as usize + 1;
                if symbol_end >= start_line {
                    return Some(symbol_end);
                }
            }
        }

        None
    }

    fn parse_line_window(file_url: &url::Url) -> Option<(usize, Option<usize>)> {
        let start = file_url.query_pairs().find_map(|(key, value)| {
            if key != "start" {
                return None;
            }
            value.parse::<usize>().ok().map(|n| n.max(1))
        })?;

        let end = file_url.query_pairs().find_map(|(key, value)| {
            if key != "end" {
                return None;
            }
            value.parse::<usize>().ok().map(|n| n.max(1))
        });

        Some((start, end))
    }

    fn decode_data_url_text(url: &str, mime: &str) -> Option<String> {
        if !Self::is_text_mime(mime) {
            return None;
        }

        let (metadata, payload) = url.split_once(',')?;
        if metadata.contains(";base64") {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(payload.as_bytes())
                .ok()?;
            return Some(String::from_utf8_lossy(&bytes).to_string());
        }

        Some(payload.to_string())
    }

    fn parse_mcp_resource_url(url: &str) -> Option<(String, String)> {
        let parsed = url::Url::parse(url).ok()?;
        if parsed.scheme() != "mcp" {
            return None;
        }

        let client_name = parsed.host_str()?.to_string();
        let mut uri = parsed.path().trim_start_matches('/').to_string();
        if let Some(query) = parsed.query() {
            if !query.is_empty() {
                if !uri.is_empty() {
                    uri.push('?');
                }
                uri.push_str(query);
            }
        }

        if uri.is_empty() {
            return None;
        }

        Some((client_name, uri))
    }

    fn filename_from_url(url: &str) -> String {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(last) = parsed
                .path_segments()
                .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
            {
                return last.to_string();
            }
        }
        String::new()
    }

    fn is_text_mime(mime: &str) -> bool {
        mime.starts_with("text/")
            || matches!(
                mime,
                "application/json"
                    | "application/xml"
                    | "application/javascript"
                    | "application/typescript"
                    | "application/x-sh"
                    | "application/x-shellscript"
            )
    }

    fn is_binary_asset_mime(mime: &str) -> bool {
        mime.starts_with("image/") || mime == "application/pdf"
    }

    fn slice_lines(text: &str, start: usize, end: Option<usize>) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return String::new();
        }

        let start_idx = start.saturating_sub(1).min(lines.len());
        let end_idx = end.unwrap_or(lines.len()).min(lines.len());
        if start_idx >= end_idx {
            return String::new();
        }

        lines[start_idx..end_idx].join("\n")
    }

    async fn read_directory_preview(path: &Path) -> String {
        let mut entries = match tokio::fs::read_dir(path).await {
            Ok(entries) => entries,
            Err(err) => return format!("Failed to list directory {}: {}", path.display(), err),
        };

        let mut names = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            names.push(entry.file_name().to_string_lossy().to_string());
            if names.len() >= 200 {
                names.push("... (truncated)".to_string());
                break;
            }
        }

        if names.is_empty() {
            return format!("Directory is empty: {}", path.display());
        }

        names.sort();
        names.join("\n")
    }

    async fn start(&self, session_id: &str) -> Option<CancellationToken> {
        let state = self.state.lock().await;
        if state.contains_key(session_id) {
            return None;
        }
        drop(state);

        let token = CancellationToken::new();
        let mut state = self.state.lock().await;
        state.insert(
            session_id.to_string(),
            PromptState {
                cancel_token: token.clone(),
            },
        );
        Some(token)
    }

    async fn resume(&self, session_id: &str) -> Option<CancellationToken> {
        let state = self.state.lock().await;
        state.get(session_id).map(|s| s.cancel_token.clone())
    }

    pub async fn is_running(&self, session_id: &str) -> bool {
        let state = self.state.lock().await;
        state.contains_key(session_id)
    }

    async fn finish_run(&self, session_id: &str) {
        let mut state = self.state.lock().await;
        state.remove(session_id);
        drop(state);

        let mut session_state = self.session_state.write().await;
        session_state.set_idle(session_id);
    }

    pub async fn cancel(&self, session_id: &str) {
        let mut state = self.state.lock().await;
        if let Some(prompt_state) = state.remove(session_id) {
            prompt_state.cancel_token.cancel();
        }

        let mut session_state = self.session_state.write().await;
        session_state.set_idle(session_id);
    }

    pub async fn prompt(
        &self,
        input: PromptInput,
        session: &mut Session,
        provider: Arc<dyn Provider>,
        system_prompt: Option<String>,
        tools: Vec<ToolDefinition>,
        agent_params: AgentParams,
    ) -> anyhow::Result<()> {
        self.prompt_with_update_hook(
            input,
            session,
            provider,
            system_prompt,
            tools,
            agent_params,
            None,
        )
        .await
    }

    pub async fn prompt_with_update_hook(
        &self,
        input: PromptInput,
        session: &mut Session,
        provider: Arc<dyn Provider>,
        system_prompt: Option<String>,
        tools: Vec<ToolDefinition>,
        agent_params: AgentParams,
        update_hook: Option<SessionUpdateHook>,
    ) -> anyhow::Result<()> {
        self.assert_not_busy(&input.session_id).await?;

        let cancel_token = self.start(&input.session_id).await;
        let token = match cancel_token {
            Some(t) => t,
            None => return Err(anyhow::anyhow!("Session already running")),
        };

        self.create_user_message(&input, session).await?;
        session.touch();
        Self::emit_session_update(update_hook.as_ref(), session);

        if input.no_reply {
            self.finish_run(&input.session_id).await;
            return Ok(());
        }

        {
            let mut session_state = self.session_state.write().await;
            session_state.set_busy(&input.session_id);
        }

        let session_id = input.session_id.clone();
        let model_id = input
            .model
            .as_ref()
            .map(|m| m.model_id.clone())
            .unwrap_or_else(|| "default".to_string());
        let provider_id = input
            .model
            .as_ref()
            .map(|m| m.provider_id.clone())
            .unwrap_or_else(|| "anthropic".to_string());

        let result = Self::loop_inner(
            session_id.clone(),
            token,
            provider,
            model_id,
            provider_id,
            session,
            input.agent.as_deref(),
            system_prompt,
            tools,
            &agent_params,
            update_hook,
            input.permission.clone(),
        )
        .await;

        self.finish_run(&session_id).await;

        if let Err(e) = result {
            tracing::error!("Prompt loop error for session {}: {}", session_id, e);
            return Err(e);
        }

        Ok(())
    }

    pub async fn resume_session(
        &self,
        session_id: &str,
        session: &mut Session,
        provider: Arc<dyn Provider>,
        system_prompt: Option<String>,
        tools: Vec<ToolDefinition>,
        agent_params: AgentParams,
        permission: Option<opencode_permission::PermissionRuleset>,
    ) -> anyhow::Result<()> {
        let token = self.resume(session_id).await;

        let token = match token {
            Some(t) => t,
            None => {
                return Err(anyhow::anyhow!(
                    "Session {} is not running, cannot resume",
                    session_id
                ));
            }
        };

        let model = session.messages.iter().rev().find_map(|m| match m.role {
            MessageRole::User => session
                .metadata
                .get("model_provider")
                .and_then(|p| p.as_str())
                .zip(session.metadata.get("model_id").and_then(|i| i.as_str()))
                .map(|(provider_id, model_id)| ModelRef {
                    provider_id: provider_id.to_string(),
                    model_id: model_id.to_string(),
                }),
            _ => None,
        });

        let model_id = model
            .as_ref()
            .map(|m| m.model_id.clone())
            .unwrap_or_else(|| "default".to_string());
        let provider_id = model
            .as_ref()
            .map(|m| m.provider_id.clone())
            .unwrap_or_else(|| "anthropic".to_string());

        let session_id = session_id.to_string();
        let resume_agent = session
            .metadata
            .get("agent")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        {
            let mut session_state = self.session_state.write().await;
            session_state.set_busy(&session_id);
        }

        let result = Self::loop_inner(
            session_id.clone(),
            token,
            provider,
            model_id,
            provider_id,
            session,
            resume_agent.as_deref(),
            system_prompt,
            tools,
            &agent_params,
            None,
            permission.clone(),
        )
        .await;

        self.finish_run(&session_id).await;

        if let Err(e) = result {
            tracing::error!("Resume prompt loop error for session {}: {}", session_id, e);
            return Err(e);
        }

        Ok(())
    }

    async fn loop_inner(
        session_id: String,
        token: CancellationToken,
        provider: Arc<dyn Provider>,
        model_id: String,
        provider_id: String,
        session: &mut Session,
        agent_name: Option<&str>,
        system_prompt: Option<String>,
        tools: Vec<ToolDefinition>,
        agent_params: &AgentParams,
        update_hook: Option<SessionUpdateHook>,
        permission: Option<opencode_permission::PermissionRuleset>,
    ) -> anyhow::Result<()> {
        let mut step = 0u32;
        let provider_type = ProviderType::from_provider_id(&provider_id);
        let mut post_first_step_ran = false;

        loop {
            if token.is_cancelled() {
                tracing::info!("Prompt loop cancelled for session {}", session_id);
                break;
            }

            let mut filtered_messages = Self::filter_compacted_messages(&session.messages);

            // Order is taken from the vector, never from the ids: message ids are random
            // UUIDs, so comparing them lexicographically decides this branch by coin flip
            // and silently drops a legitimate turn about half the time.
            let last_user_index = filtered_messages
                .iter()
                .rposition(|m| matches!(m.role, MessageRole::User));

            let last_assistant_index = filtered_messages
                .iter()
                .rposition(|m| matches!(m.role, MessageRole::Assistant));

            let Some(last_user_index) = last_user_index else {
                return Err(anyhow::anyhow!("No user message found"));
            };

            if Self::process_pending_subtasks(session, provider.clone(), &model_id, &provider_id)
                .await?
            {
                tracing::info!("Processed pending subtask parts for session {}", session_id);
                continue;
            }

            if let Some(assistant_index) = last_assistant_index {
                let assistant = &filtered_messages[assistant_index];
                let has_finish = assistant.parts.iter().any(|p| match &p.part_type {
                    PartType::Text { text, .. } => !text.is_empty(),
                    _ => false,
                });

                // Prose next to a tool call is narration, not an answer: the model writes
                // "I'll search now..." and requests the tool in the same message. Treating
                // that text as the end of the turn ended the loop the moment the result
                // landed, so the model never read it and the session closed on an
                // intention. Any tool call in the last assistant message means another turn
                // is owed, whether or not its result already arrived.
                let has_tool_call = assistant
                    .parts
                    .iter()
                    .any(|p| matches!(p.part_type, PartType::ToolCall { .. }));

                if has_finish && !has_tool_call && last_user_index < assistant_index {
                    tracing::info!("Prompt loop complete for session {}", session_id);
                    break;
                }
            }

            step += 1;
            if step > MAX_STEPS {
                tracing::warn!("Max steps reached for session {}", session_id);
                break;
            }

            if Self::should_compact(
                &filtered_messages,
                provider.as_ref(),
                &model_id,
                agent_params.max_tokens,
            ) {
                tracing::info!(
                    "Context overflow detected, triggering compaction for session {}",
                    session_id
                );
                if let Some(summary) = Self::trigger_compaction(session, &filtered_messages) {
                    tracing::info!("Compaction complete, summary: {}", summary);

                    // Notify plugins that compaction occurred (mirrors TS Plugin.trigger).
                    let _ = opencode_plugin::trigger_collect(
                        HookContext::new(HookEvent::SessionCompacting)
                            .with_session(&session_id)
                            .with_data("auto", serde_json::json!(true))
                            .with_data("completed", serde_json::json!(true)),
                    )
                    .await;
                }
            }

            tracing::info!("Prompt loop step {} for session {}", step, session_id);

            // Plugin hook: chat.messages.transform — let plugins modify messages before sending
            let message_hook_outputs = opencode_plugin::trigger_collect(
                HookContext::new(HookEvent::ChatMessagesTransform)
                    .with_session(&session_id)
                    .with_data("message_count", serde_json::json!(filtered_messages.len()))
                    .with_data("messages", serde_json::json!(messages_to_wire(&filtered_messages))),
            )
            .await;
            apply_chat_messages_hook_outputs(&mut filtered_messages, message_hook_outputs);

            let mut prompt_messages = filtered_messages;
            if let Some(agent) = agent_name {
                let was_plan = was_plan_agent(&prompt_messages);
                prompt_messages = insert_reminders(&prompt_messages, agent, was_plan);
            }

            let mut chat_messages =
                Self::build_chat_messages(&prompt_messages, system_prompt.as_deref())?;

            apply_caching(&mut chat_messages, provider_type);
            let resolved_tools =
                merge_tool_definitions(tools.clone(), Self::mcp_tools_from_session(session));

            let request = ChatRequest {
                model: model_id.clone(),
                messages: chat_messages,
                max_tokens: Some(agent_params.max_tokens.unwrap_or(8192)),
                temperature: agent_params.temperature,
                system: None,
                tools: if resolved_tools.is_empty() {
                    None
                } else {
                    Some(resolved_tools.clone())
                },
                stream: Some(true),
                top_p: agent_params.top_p,
                variant: None,
                provider_options: None,
                session_id: Some(session_id.clone()),
            };

            // Stream the response (matching TS streamText approach).
            let mut stream = match provider.chat_stream(request).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Provider error for session {}: {}", session_id, e);
                    return Err(anyhow::anyhow!("{}", e));
                }
            };

            // Create assistant message placeholder before consuming the stream so
            // callers can observe incremental output updates.
            let assistant_index = session.messages.len();
            let assistant_message_id =
                opencode_core::id::create(opencode_core::id::Prefix::Message, true, None);
            let mut assistant_metadata = HashMap::new();
            assistant_metadata.insert(
                "model_provider".to_string(),
                serde_json::json!(&provider_id),
            );
            assistant_metadata.insert("model_id".to_string(), serde_json::json!(&model_id));
            if let Some(agent) = agent_name {
                assistant_metadata.insert("agent".to_string(), serde_json::json!(agent));
                assistant_metadata.insert("mode".to_string(), serde_json::json!(agent));
            }
            session.messages.push(SessionMessage {
                id: assistant_message_id,
                session_id: session_id.clone(),
                role: MessageRole::Assistant,
                parts: Vec::new(),
                created_at: chrono::Utc::now(),
                metadata: assistant_metadata,
                usage: None,
            });
            session.touch();
            Self::emit_session_update(update_hook.as_ref(), session);

            // Consume stream events to build the assistant message incrementally.
            // tool_calls keyed by id: (name, input_json_fragments)
            let mut tool_calls: HashMap<String, (String, String)> = HashMap::new();
            let mut finish_reason: Option<String> = None;
            let mut prompt_tokens: u64 = 0;
            let mut completion_tokens: u64 = 0;
            let mut last_emit = Instant::now() - Duration::from_millis(50);

            while let Some(event_result) = stream.next().await {
                if token.is_cancelled() {
                    tracing::info!("Stream cancelled for session {}", session_id);
                    break;
                }
                match event_result {
                    Ok(StreamEvent::TextDelta(text)) => {
                        if let Some(assistant) = session.messages.get_mut(assistant_index) {
                            Self::append_delta_part(assistant, false, &text);
                        }
                        session.touch();
                        Self::maybe_emit_session_update(
                            update_hook.as_ref(),
                            session,
                            &mut last_emit,
                            false,
                        );
                    }
                    Ok(StreamEvent::TextStart) | Ok(StreamEvent::TextEnd) => {}
                    Ok(StreamEvent::ReasoningStart { .. }) => {}
                    Ok(StreamEvent::ReasoningDelta { text, .. }) => {
                        if let Some(assistant) = session.messages.get_mut(assistant_index) {
                            Self::append_delta_part(assistant, true, &text);
                        }
                        session.touch();
                        Self::maybe_emit_session_update(
                            update_hook.as_ref(),
                            session,
                            &mut last_emit,
                            false,
                        );
                    }
                    Ok(StreamEvent::ReasoningEnd { .. }) => {}
                    Ok(StreamEvent::ToolCallStart { id, name }) => match tool_calls.entry(id) {
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert((name, String::new()));
                        }
                        std::collections::hash_map::Entry::Occupied(mut entry) => {
                            if entry.get().0.is_empty() {
                                entry.get_mut().0 = name;
                            }
                        }
                    },
                    Ok(StreamEvent::ToolCallDelta { id, input }) => {
                        let entry = tool_calls
                            .entry(id)
                            .or_insert_with(|| (String::new(), String::new()));
                        entry.1.push_str(&input);
                    }
                    Ok(StreamEvent::ToolCallEnd { id, name, input }) => {
                        let args_str = serde_json::to_string(&input).unwrap_or_default();
                        tool_calls.insert(id, (name, args_str));
                    }
                    Ok(StreamEvent::ToolInputStart { id, tool_name }) => {
                        match tool_calls.entry(id) {
                            std::collections::hash_map::Entry::Vacant(entry) => {
                                entry.insert((tool_name, String::new()));
                            }
                            std::collections::hash_map::Entry::Occupied(mut entry) => {
                                if entry.get().0.is_empty() {
                                    entry.get_mut().0 = tool_name;
                                }
                            }
                        }
                    }
                    Ok(StreamEvent::ToolInputDelta { id, delta }) => {
                        let entry = tool_calls
                            .entry(id)
                            .or_insert_with(|| (String::new(), String::new()));
                        entry.1.push_str(&delta);
                    }
                    Ok(StreamEvent::ToolInputEnd { .. }) => {}
                    Ok(StreamEvent::FinishStep {
                        finish_reason: fr,
                        usage,
                        ..
                    }) => {
                        finish_reason = fr;
                        prompt_tokens = usage.prompt_tokens;
                        completion_tokens = usage.completion_tokens;
                    }
                    Ok(StreamEvent::Usage {
                        prompt_tokens: pt,
                        completion_tokens: ct,
                    }) => {
                        prompt_tokens = pt;
                        completion_tokens = ct;
                    }
                    Ok(StreamEvent::Done | StreamEvent::Finish) => break,
                    Ok(StreamEvent::Start) => {}
                    Ok(StreamEvent::Error(msg)) => {
                        tracing::error!("Stream error for session {}: {}", session_id, msg);
                        return Err(anyhow::anyhow!("Provider error: {}", msg));
                    }
                    // ToolResult/ToolError come from Responses API; legacy stream
                    // doesn't produce them — tool execution is handled below.
                    Ok(StreamEvent::ToolResult { .. } | StreamEvent::ToolError { .. }) => {}
                    Ok(StreamEvent::StartStep) => {}
                    Err(e) => {
                        tracing::error!("Stream error for session {}: {}", session_id, e);
                        return Err(anyhow::anyhow!("{}", e));
                    }
                }
            }

            // Finalize the placeholder assistant message with tool calls and usage.
            if let Some(assistant_msg) = session.messages.get_mut(assistant_index) {
                for (tc_id, (tc_name, tc_args)) in &tool_calls {
                    let input: serde_json::Value =
                        serde_json::from_str(tc_args).unwrap_or(serde_json::json!({}));
                    assistant_msg.parts.push(crate::MessagePart {
                        id: opencode_core::id::create(opencode_core::id::Prefix::Part, true, None),
                        part_type: PartType::ToolCall {
                            id: tc_id.clone(),
                            name: tc_name.clone(),
                            input,
                        },
                        created_at: chrono::Utc::now(),
                        message_id: None,
                    });
                }
                if let Some(reason) = finish_reason.clone() {
                    assistant_msg
                        .metadata
                        .insert("finish_reason".to_string(), serde_json::json!(reason));
                }
                assistant_msg.metadata.insert(
                    "completed_at".to_string(),
                    serde_json::json!(chrono::Utc::now().timestamp_millis()),
                );
                assistant_msg.metadata.insert(
                    "usage".to_string(),
                    serde_json::json!({
                        "prompt_tokens": prompt_tokens,
                        "completion_tokens": completion_tokens,
                    }),
                );
                assistant_msg.usage = Some(crate::message::MessageUsage {
                    input_tokens: prompt_tokens,
                    output_tokens: completion_tokens,
                    ..Default::default()
                });
            }

            let mut has_tool_calls = session
                .messages
                .get(assistant_index)
                .map(|message| {
                    message
                        .parts
                        .iter()
                        .any(|p| matches!(p.part_type, PartType::ToolCall { .. }))
                })
                .unwrap_or(false);

            session.touch();
            Self::emit_session_update(update_hook.as_ref(), session);

            if !post_first_step_ran {
                Self::ensure_title(session, provider.clone(), &model_id).await;
                let _ = Self::summarize_session(
                    session,
                    &session_id,
                    &provider_id,
                    &model_id,
                    provider.as_ref(),
                )
                .await;
                post_first_step_ran = true;
            }

            // Plugin hook: chat.message — notify plugins of new assistant message
            if let Some(assistant_message) = session.messages.get(assistant_index).cloned() {
                let hook_outputs = opencode_plugin::trigger_collect(
                    HookContext::new(HookEvent::ChatMessage)
                        .with_session(&session_id)
                        .with_data("model_id", serde_json::json!(&model_id))
                        .with_data("provider_id", serde_json::json!(&provider_id))
                        .with_data("message_id", serde_json::json!(&assistant_message.id))
                        .with_data("has_tool_calls", serde_json::json!(has_tool_calls))
                        .with_data("message", serde_json::json!(&assistant_message))
                        .with_data("parts", serde_json::json!(&assistant_message.parts)),
                )
                .await;
                if let Some(current_message) = session.messages.get_mut(assistant_index) {
                    apply_chat_message_hook_outputs(current_message, hook_outputs);
                }
                has_tool_calls = session
                    .messages
                    .get(assistant_index)
                    .map(|message| {
                        message
                            .parts
                            .iter()
                            .any(|p| matches!(p.part_type, PartType::ToolCall { .. }))
                    })
                    .unwrap_or(false);
            }

            if has_tool_calls {
                tracing::info!("Processing tool calls for session {}", session_id);

                let tool_context = opencode_tool::ToolContext::new(
                    session_id.clone(),
                    session
                        .messages
                        .last()
                        .map(|m| m.id.clone())
                        .unwrap_or_default(),
                    session.directory.clone(),
                )
                .with_agent(String::new())
                .with_abort(token.clone());

                let registry = Arc::new(opencode_tool::create_default_registry().await);
                if let Err(e) = Self::execute_tool_calls(
                    session,
                    registry,
                    tool_context,
                    provider.clone(),
                    &provider_id,
                    &model_id,
                    permission.clone(),
                )
                .await
                {
                    tracing::error!("Tool execution error for session {}: {}", session_id, e);
                }
                session.touch();
                Self::emit_session_update(update_hook.as_ref(), session);
                continue;
            }

            if finish_reason.as_deref() != Some("tool-calls") {
                tracing::info!(
                    "Prompt loop complete for session {} with finish: {:?}",
                    session_id,
                    finish_reason
                );
                break;
            }
        }

        // Abort handling: mark any pending tool calls as error when cancelled.
        // Mirrors TS processor.ts lines 393-409 where incomplete tool parts
        // are set to error status with "Tool execution aborted".
        if token.is_cancelled() {
            Self::abort_pending_tool_calls(session);
        }

        Self::prune_after_loop(session);
        session.touch();
        Self::emit_session_update(update_hook.as_ref(), session);

        Ok(())
    }

    fn emit_session_update(update_hook: Option<&SessionUpdateHook>, session: &Session) {
        if let Some(hook) = update_hook {
            hook(session);
        }
    }

    fn maybe_emit_session_update(
        update_hook: Option<&SessionUpdateHook>,
        session: &Session,
        last_emit: &mut Instant,
        force: bool,
    ) {
        let elapsed = last_emit.elapsed();
        if force || elapsed >= Duration::from_millis(50) {
            Self::emit_session_update(update_hook, session);
            *last_emit = Instant::now();
        }
    }

    fn append_delta_part(message: &mut SessionMessage, reasoning: bool, delta: &str) {
        if delta.is_empty() {
            return;
        }

        for part in message.parts.iter_mut().rev() {
            match (&mut part.part_type, reasoning) {
                (PartType::Reasoning { text }, true) => {
                    text.push_str(delta);
                    return;
                }
                (PartType::Text { text, .. }, false) => {
                    text.push_str(delta);
                    return;
                }
                _ => {}
            }
        }

        message.parts.push(crate::MessagePart {
            id: opencode_core::id::create(opencode_core::id::Prefix::Part, true, None),
            part_type: if reasoning {
                PartType::Reasoning {
                    text: delta.to_string(),
                }
            } else {
                PartType::Text {
                    text: delta.to_string(),
                    synthetic: None,
                    ignored: None,
                }
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
    }

    /// Mark any tool calls that lack a corresponding tool result as aborted.
    ///
    /// When the prompt loop is cancelled mid-execution, some tool calls in the
    /// assistant messages may not have received their results yet. This mirrors
    /// the TS abort handling in processor.ts that sets incomplete tool parts to
    /// error status with "Tool execution aborted".
    fn abort_pending_tool_calls(session: &mut Session) {
        // Collect all tool call IDs that already have a result
        let mut resolved_call_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for msg in &session.messages {
            for part in &msg.parts {
                if let PartType::ToolResult { tool_call_id, .. } = &part.part_type {
                    resolved_call_ids.insert(tool_call_id.clone());
                }
            }
        }

        // Find unresolved tool calls and add error results
        let mut pending_calls: Vec<String> = Vec::new();
        for msg in &session.messages {
            for part in &msg.parts {
                if let PartType::ToolCall { id, .. } = &part.part_type {
                    if !resolved_call_ids.contains(id) {
                        pending_calls.push(id.clone());
                    }
                }
            }
        }

        if pending_calls.is_empty() {
            return;
        }

        tracing::info!(
            count = pending_calls.len(),
            "Marking pending tool calls as aborted"
        );

        // Add error results for each pending tool call to the last assistant message
        if let Some(last_assistant) = session
            .messages
            .iter_mut()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
        {
            for call_id in pending_calls {
                last_assistant.add_tool_result(&call_id, "Tool execution aborted", true);
            }
        }
    }

    pub async fn execute_tool_calls(
        session: &mut Session,
        tool_registry: Arc<opencode_tool::ToolRegistry>,
        ctx: opencode_tool::ToolContext,
        provider: Arc<dyn Provider>,
        provider_id: &str,
        model_id: &str,
        permission: Option<opencode_permission::PermissionRuleset>,
    ) -> anyhow::Result<()> {
        let last_assistant = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant));

        let tool_calls: Vec<(String, String, serde_json::Value)> = match last_assistant {
            Some(msg) => msg
                .parts
                .iter()
                .filter_map(|p| match &p.part_type {
                    PartType::ToolCall { id, name, input } => {
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect(),
            None => return Ok(()),
        };

        if tool_calls.is_empty() {
            return Ok(());
        }

        let subsessions = Arc::new(Mutex::new(Self::load_persisted_subsessions(session)));
        let default_model = format!("{}:{}", provider_id, model_id);
        let ctx = Self::with_persistent_subsession_callbacks(
            ctx,
            subsessions.clone(),
            provider,
            tool_registry.clone(),
            default_model,
        );

        // The results must land in the SAME message as the calls they answer. Pushing them
        // as a separate assistant message left the model with a tool_use it never saw
        // resolved: it reported "the tool calls came back empty" while real Exa results
        // sat in an orphan part, and re-searched from scratch every turn.
        let results_index = last_assistant
            .as_ref()
            .map(|m| m.id.clone())
            .and_then(|id| {
                session
                    .messages
                    .iter()
                    .position(|m| m.id == id && m.role == MessageRole::Assistant)
            });

        let mut tool_results_msg = {
            let mut msg = SessionMessage::assistant(ctx.session_id.clone());
            for (call_id, tool_name, input) in tool_calls {
                let mut tool_ctx = ctx.clone();
                tool_ctx.call_id = Some(call_id.clone());
                if let Some(ref ruleset) = permission {
                    tool_ctx = tool_ctx.with_permission(ruleset.clone());
                }
                let result = match tool_registry.execute(&tool_name, input, tool_ctx).await {
                    Ok(result) => (result.output, false),
                    Err(e) => (format!("Error: {}", e), true),
                };

                msg.parts.push(crate::MessagePart {
                    id: format!("prt_{}", uuid::Uuid::new_v4()),
                    part_type: PartType::ToolResult {
                        tool_call_id: call_id,
                        content: result.0,
                        is_error: result.1,
                    },
                    created_at: chrono::Utc::now(),
                    message_id: None,
                });
            }
            msg
        };

        match results_index {
            Some(index) => {
                let parts = &mut session.messages[index].parts;
                parts.append(&mut tool_results_msg.parts);
            }
            None => session.messages.push(tool_results_msg),
        }
        let persisted = subsessions.lock().await.clone();
        Self::save_persisted_subsessions(session, &persisted);
        Ok(())
    }

    fn mcp_tools_from_session(session: &Session) -> Vec<ToolDefinition> {
        session
            .metadata
            .get("mcp_tools")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let name = item.get("name").and_then(|v| v.as_str())?.to_string();
                        let description = item
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let parameters = item
                            .get("parameters")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({"type":"object"}));
                        Some(ToolDefinition {
                            name,
                            description,
                            parameters,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn load_persisted_subsessions(session: &Session) -> HashMap<String, PersistedSubsession> {
        session
            .metadata
            .get("subsessions")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    fn save_persisted_subsessions(
        session: &mut Session,
        subsessions: &HashMap<String, PersistedSubsession>,
    ) {
        if subsessions.is_empty() {
            session.metadata.remove("subsessions");
            return;
        }
        if let Ok(value) = serde_json::to_value(subsessions) {
            session.metadata.insert("subsessions".to_string(), value);
        }
    }

    fn with_persistent_subsession_callbacks(
        ctx: opencode_tool::ToolContext,
        subsessions: Arc<Mutex<HashMap<String, PersistedSubsession>>>,
        provider: Arc<dyn Provider>,
        tool_registry: Arc<opencode_tool::ToolRegistry>,
        default_model: String,
    ) -> opencode_tool::ToolContext {
        let ctx = ctx.with_get_last_model({
            let default_model = default_model.clone();
            move |_session_id| {
                let default_model = default_model.clone();
                async move { Ok(Some(default_model)) }
            }
        });

        let ctx = ctx.with_create_subsession({
            let subsessions = subsessions.clone();
            move |agent, _title, model, disabled_tools| {
                let subsessions = subsessions.clone();
                async move {
                    let session_id = format!("task_{}_{}", agent, uuid::Uuid::new_v4().simple());
                    let mut state = subsessions.lock().await;
                    state.insert(
                        session_id.clone(),
                        PersistedSubsession {
                            agent,
                            model,
                            disabled_tools,
                            history: Vec::new(),
                        },
                    );
                    Ok(session_id)
                }
            }
        });

        ctx.with_prompt_subsession(move |session_id, prompt| {
            let subsessions = subsessions.clone();
            let provider = provider.clone();
            let tool_registry = tool_registry.clone();
            let default_model = default_model.clone();

            async move {
                let current = {
                    let state = subsessions.lock().await;
                    state.get(&session_id).cloned()
                }
                .ok_or_else(|| {
                    opencode_tool::ToolError::ExecutionError(format!(
                        "Unknown subagent session: {}. Start without task_id first.",
                        session_id
                    ))
                })?;

                let output = Self::execute_persisted_subsession_prompt(
                    &current,
                    &prompt,
                    provider,
                    tool_registry,
                    &default_model,
                )
                .await
                .map_err(|e| opencode_tool::ToolError::ExecutionError(e.to_string()))?;

                let mut state = subsessions.lock().await;
                if let Some(existing) = state.get_mut(&session_id) {
                    existing.history.push(PersistedSubsessionTurn {
                        prompt,
                        output: output.clone(),
                    });
                }
                Ok(output)
            }
        })
    }

    async fn execute_persisted_subsession_prompt(
        subsession: &PersistedSubsession,
        prompt: &str,
        provider: Arc<dyn Provider>,
        tool_registry: Arc<opencode_tool::ToolRegistry>,
        default_model: &str,
    ) -> anyhow::Result<String> {
        let mut model =
            Self::parse_model_string(subsession.model.as_deref().unwrap_or(default_model));
        if model.provider_id == "default" && model.model_id == "default" {
            model = Self::parse_model_string(default_model);
        }

        let composed_prompt = Self::compose_subsession_prompt(&subsession.history, prompt);
        let mut executor =
            SubtaskExecutor::new(&subsession.agent, &composed_prompt).with_model(model);
        // 2048 tokens cannot hold an agent mid-thought across a tool loop: it truncates,
        // the model re-asks, and the run burns its step budget without ever answering.
        executor.agent_params = AgentParams {
            max_tokens: None,
            temperature: Some(0.2),
            top_p: None,
        };

        executor
            .execute_inline(provider, &tool_registry, &subsession.disabled_tools)
            .await
    }

    fn parse_model_string(raw: &str) -> ModelRef {
        if let Some((provider_id, model_id)) = raw.split_once(':').or_else(|| raw.split_once('/')) {
            return ModelRef {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
            };
        }
        if raw.is_empty() {
            return ModelRef {
                provider_id: "default".to_string(),
                model_id: "default".to_string(),
            };
        }
        ModelRef {
            provider_id: "default".to_string(),
            model_id: raw.to_string(),
        }
    }

    fn compose_subsession_prompt(history: &[PersistedSubsessionTurn], prompt: &str) -> String {
        if history.is_empty() {
            return prompt.to_string();
        }

        let history_text = history
            .iter()
            .rev()
            .take(8)
            .rev()
            .map(|turn| format!("User:\n{}\n\nAssistant:\n{}", turn.prompt, turn.output))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        format!(
            "Continue this subtask session.\n\nPrevious conversation:\n{}\n\nNew request:\n{}",
            history_text, prompt
        )
    }

    fn build_chat_messages(
        session_messages: &[SessionMessage],
        system_prompt: Option<&str>,
    ) -> anyhow::Result<Vec<Message>> {
        let mut messages = Vec::new();

        if let Some(system) = system_prompt {
            messages.push(Message::system(system));
        }

        for msg in session_messages {
            let content = Self::parts_to_content(&msg.parts);
            let role = match msg.role {
                MessageRole::User => Role::User,
                MessageRole::Assistant => Role::Assistant,
                MessageRole::System => Role::System,
                MessageRole::Tool => Role::Tool,
            };

            messages.push(Message {
                role,
                content,
                cache_control: None,
                provider_options: None,
            });
        }

        Ok(messages)
    }

    fn parts_to_content(parts: &[crate::MessagePart]) -> Content {
        let has_parts = parts
            .iter()
            .any(|p| !matches!(p.part_type, PartType::Text { .. }));

        if !has_parts {
            let text = parts
                .iter()
                .filter_map(|p| match &p.part_type {
                    PartType::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Content::Text(text);
        }

        let content_parts: Vec<ContentPart> = parts
            .iter()
            .filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(ContentPart {
                    content_type: "text".to_string(),
                    text: Some(text.clone()),
                    image_url: None,
                    tool_use: None,
                    tool_result: None,
                    cache_control: None,
                    filename: None,
                    media_type: None,
                    provider_options: None,
                }),
                PartType::ToolCall { id, name, input } => Some(ContentPart {
                    content_type: "tool_use".to_string(),
                    text: None,
                    image_url: None,
                    tool_use: Some(opencode_provider::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    }),
                    tool_result: None,
                    cache_control: None,
                    filename: None,
                    media_type: None,
                    provider_options: None,
                }),
                PartType::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } => Some(ContentPart {
                    content_type: "tool_result".to_string(),
                    text: None,
                    image_url: None,
                    tool_use: None,
                    tool_result: Some(opencode_provider::ToolResult {
                        tool_use_id: tool_call_id.clone(),
                        content: content.clone(),
                        is_error: Some(*is_error),
                    }),
                    cache_control: None,
                    filename: None,
                    media_type: None,
                    provider_options: None,
                }),
                _ => None,
            })
            .collect();

        Content::Parts(content_parts)
    }

    #[allow(dead_code)]
    fn process_response(response: &ChatResponse) -> SessionMessage {
        let now = chrono::Utc::now();

        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or(Content::Text(String::new()));

        let finish_reason = response
            .choices
            .first()
            .and_then(|c| c.finish_reason.clone());

        let parts = match content {
            Content::Text(text) => vec![crate::MessagePart {
                id: format!("prt_{}", uuid::Uuid::new_v4()),
                part_type: PartType::Text { text, synthetic: None, ignored: None },
                created_at: now,
                message_id: None,
            }],
            Content::Parts(content_parts) => content_parts
                .into_iter()
                .filter_map(|p| match p.content_type.as_str() {
                    "text" => p.text.map(|text| crate::MessagePart {
                        id: format!("prt_{}", uuid::Uuid::new_v4()),
                        part_type: PartType::Text { text, synthetic: None, ignored: None },
                        created_at: now,
                        message_id: None,
                    }),
                    "tool_use" => p.tool_use.map(|tu| crate::MessagePart {
                        id: format!("prt_{}", uuid::Uuid::new_v4()),
                        part_type: PartType::ToolCall {
                            id: tu.id,
                            name: tu.name,
                            input: tu.input,
                        },
                        created_at: now,
                        message_id: None,
                    }),
                    _ => None,
                })
                .collect(),
        };

        SessionMessage {
            id: format!("msg_{}", uuid::Uuid::new_v4()),
            session_id: String::new(),
            role: MessageRole::Assistant,
            parts,
            created_at: now,
            metadata: {
                let mut m = HashMap::new();
                if let Some(usage) = &response.usage {
                    m.insert(
                        "tokens_input".to_string(),
                        serde_json::json!(usage.prompt_tokens),
                    );
                    m.insert(
                        "tokens_output".to_string(),
                        serde_json::json!(usage.completion_tokens),
                    );
                }
                if let Some(reason) = finish_reason {
                    m.insert("finish_reason".to_string(), serde_json::json!(reason));
                }
                m
            },
            usage: None,
        }
    }

    fn collect_pending_subtasks(message: &SessionMessage) -> Vec<PendingSubtask> {
        let metadata_by_id: HashMap<String, (String, String, String)> = message
            .metadata
            .get("pending_subtasks")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let id = item.get("id").and_then(|v| v.as_str())?.to_string();
                        let agent = item
                            .get("agent")
                            .and_then(|v| v.as_str())
                            .unwrap_or("general")
                            .to_string();
                        let prompt = item
                            .get("prompt")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let description = item
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some((id, (agent, prompt, description)))
                    })
                    .collect()
            })
            .unwrap_or_default();

        message
            .parts
            .iter()
            .enumerate()
            .filter_map(|(part_index, part)| match &part.part_type {
                PartType::Subtask {
                    id,
                    description,
                    status,
                } if status == "pending" => {
                    let (agent, prompt, meta_description) = metadata_by_id
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| (id.clone(), description.clone(), description.clone()));
                    let description = if meta_description.is_empty() {
                        description.clone()
                    } else {
                        meta_description
                    };
                    let prompt = if prompt.trim().is_empty() {
                        description.clone()
                    } else {
                        prompt
                    };
                    Some(PendingSubtask {
                        part_index,
                        subtask_id: id.clone(),
                        agent,
                        prompt,
                        description,
                    })
                }
                _ => None,
            })
            .collect()
    }

    async fn process_pending_subtasks(
        session: &mut Session,
        provider: Arc<dyn Provider>,
        model_id: &str,
        provider_id: &str,
    ) -> anyhow::Result<bool> {
        let last_user_idx = session
            .messages
            .iter()
            .rposition(|m| matches!(m.role, MessageRole::User));
        let Some(last_user_idx) = last_user_idx else {
            return Ok(false);
        };

        let pending = Self::collect_pending_subtasks(&session.messages[last_user_idx]);
        if pending.is_empty() {
            return Ok(false);
        }

        let mut results: Vec<(usize, String, bool, String, String)> = Vec::new();
        let tool_registry = Arc::new(opencode_tool::create_default_registry().await);
        let mut persisted = Self::load_persisted_subsessions(session);
        let default_model = format!("{}:{}", provider_id, model_id);
        let user_text = session.messages[last_user_idx].get_text();

        for subtask in &pending {
            let combined_prompt = if user_text.trim().is_empty() {
                subtask.prompt.clone()
            } else {
                format!("{}\n\nSubtask: {}", user_text, subtask.prompt)
            };
            let subsession_id = format!("task_subtask_{}", subtask.subtask_id);
            persisted
                .entry(subsession_id.clone())
                .or_insert_with(|| PersistedSubsession {
                    agent: subtask.agent.clone(),
                    model: Some(default_model.clone()),
                    disabled_tools: Vec::new(),
                    history: Vec::new(),
                });
            let state_snapshot =
                persisted
                    .get(&subsession_id)
                    .cloned()
                    .unwrap_or(PersistedSubsession {
                        agent: subtask.agent.clone(),
                        model: Some(default_model.clone()),
                        disabled_tools: Vec::new(),
                        history: Vec::new(),
                    });

            match Self::execute_persisted_subsession_prompt(
                &state_snapshot,
                &combined_prompt,
                provider.clone(),
                tool_registry.clone(),
                &default_model,
            )
            .await
            {
                Ok(output) => {
                    if let Some(existing) = persisted.get_mut(&subsession_id) {
                        existing.history.push(PersistedSubsessionTurn {
                            prompt: combined_prompt,
                            output: output.clone(),
                        });
                    }
                    results.push((
                        subtask.part_index,
                        subtask.subtask_id.clone(),
                        false,
                        subtask.description.clone(),
                        output,
                    ));
                }
                Err(error) => {
                    results.push((
                        subtask.part_index,
                        subtask.subtask_id.clone(),
                        true,
                        subtask.description.clone(),
                        error.to_string(),
                    ));
                }
            }
        }

        for (part_index, subtask_id, is_error, description, output) in results {
            if let Some(part) = session.messages[last_user_idx].parts.get_mut(part_index) {
                if let PartType::Subtask { status, .. } = &mut part.part_type {
                    *status = if is_error {
                        "error".to_string()
                    } else {
                        "completed".to_string()
                    };
                }
            }

            let assistant = session.add_assistant_message();
            assistant
                .metadata
                .insert("subtask_id".to_string(), serde_json::json!(subtask_id));
            assistant.metadata.insert(
                "subtask_status".to_string(),
                serde_json::json!(if is_error { "error" } else { "completed" }),
            );
            assistant.add_text(format!(
                "Subtask `{}` {}:\n{}",
                description,
                if is_error { "failed" } else { "completed" },
                output
            ));
        }

        Self::save_persisted_subsessions(session, &persisted);

        Ok(true)
    }

    fn filter_compacted_messages(messages: &[SessionMessage]) -> Vec<SessionMessage> {
        let start = messages
            .iter()
            .rposition(|m| {
                m.parts
                    .iter()
                    .any(|p| matches!(p.part_type, PartType::Compaction { .. }))
            })
            .unwrap_or(0);
        messages[start..].to_vec()
    }

    fn token_usage_from_messages(messages: &[SessionMessage]) -> TokenUsage {
        let mut usage = TokenUsage {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            total: 0,
        };

        for msg in messages {
            usage.input += msg
                .metadata
                .get("tokens_input")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            usage.output += msg
                .metadata
                .get("tokens_output")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            usage.cache_read += msg
                .metadata
                .get("tokens_cache_read")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            usage.cache_write += msg
                .metadata
                .get("tokens_cache_write")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }
        usage.total = usage.input + usage.output + usage.cache_read + usage.cache_write;
        usage
    }

    fn should_compact(
        messages: &[SessionMessage],
        provider: &dyn Provider,
        model_id: &str,
        max_output_tokens: Option<u64>,
    ) -> bool {
        let usage = Self::token_usage_from_messages(messages);
        let model = provider.get_model(model_id);
        let limits = ModelLimits {
            context: model
                .map(|info| info.context_window)
                .unwrap_or_else(|| get_model_context_limit(model_id)),
            max_input: None,
            max_output: max_output_tokens
                .or_else(|| model.map(|info| info.max_output_tokens))
                .unwrap_or(8192),
        };
        let engine = CompactionEngine::new(CompactionConfig::default());
        if engine.is_overflow(&usage, &limits) {
            return true;
        }

        let total_chars: usize = messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(text.len()),
                _ => None,
            })
            .sum();

        const MAX_CONTEXT_CHARS: usize = 200_000;
        total_chars > MAX_CONTEXT_CHARS
    }

    async fn ensure_title(session: &mut Session, provider: Arc<dyn Provider>, model_id: &str) {
        if !session.is_default_title() {
            return;
        }

        let first_user_text = session
            .messages
            .iter()
            .find(|m| matches!(m.role, MessageRole::User))
            .map(|m| m.get_text())
            .unwrap_or_default();

        if first_user_text.trim().is_empty() {
            return;
        }

        let title =
            generate_session_title_llm(&first_user_text, provider, model_id, &session.id).await;
        if !title.trim().is_empty() {
            session.set_title(title);
        }
    }

    fn to_message_with_parts(
        messages: &[SessionMessage],
        provider_id: &str,
        model_id: &str,
    ) -> Vec<MessageWithParts> {
        let mut out = Vec::with_capacity(messages.len());
        let mut last_user_id = String::new();

        for msg in messages {
            let created = msg.created_at.timestamp_millis();
            let mut parts: Vec<V2Part> = msg
                .parts
                .iter()
                .filter_map(|part| match &part.part_type {
                    PartType::Text { text, .. } => Some(V2Part::Text {
                        id: part.id.clone(),
                        session_id: msg.session_id.clone(),
                        message_id: msg.id.clone(),
                        text: text.clone(),
                        synthetic: None,
                        ignored: None,
                        time: None,
                        metadata: None,
                    }),
                    PartType::File {
                        url,
                        filename,
                        mime,
                    } => Some(V2Part::File(crate::message_v2::FilePart {
                        id: part.id.clone(),
                        session_id: msg.session_id.clone(),
                        message_id: msg.id.clone(),
                        mime: mime.clone(),
                        url: url.clone(),
                        filename: Some(filename.clone()),
                        source: None,
                    })),
                    PartType::Compaction { .. } => Some(V2Part::Compaction(V2CompactionPart {
                        id: part.id.clone(),
                        session_id: msg.session_id.clone(),
                        message_id: msg.id.clone(),
                        auto: true,
                    })),
                    _ => None,
                })
                .collect();

            if let Some(snapshot) = msg
                .metadata
                .get("step_start_snapshot")
                .or_else(|| msg.metadata.get("snapshot"))
                .and_then(|v| v.as_str())
            {
                parts.push(V2Part::StepStart(StepStartPart {
                    id: format!("prt_{}", uuid::Uuid::new_v4()),
                    session_id: msg.session_id.clone(),
                    message_id: msg.id.clone(),
                    snapshot: Some(snapshot.to_string()),
                }));
            }
            if let Some(snapshot) = msg
                .metadata
                .get("step_finish_snapshot")
                .and_then(|v| v.as_str())
            {
                let input = msg
                    .metadata
                    .get("tokens_input")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .clamp(0, i32::MAX as i64) as i32;
                let output = msg
                    .metadata
                    .get("tokens_output")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .clamp(0, i32::MAX as i64) as i32;
                parts.push(V2Part::StepFinish(StepFinishPart {
                    id: format!("prt_{}", uuid::Uuid::new_v4()),
                    session_id: msg.session_id.clone(),
                    message_id: msg.id.clone(),
                    reason: msg
                        .metadata
                        .get("finish_reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("stop")
                        .to_string(),
                    snapshot: Some(snapshot.to_string()),
                    cost: msg
                        .metadata
                        .get("cost")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    tokens: StepTokens {
                        total: Some(input.saturating_add(output)),
                        input,
                        output,
                        reasoning: 0,
                        cache: CacheTokens { read: 0, write: 0 },
                    },
                }));
            }

            let info = match msg.role {
                MessageRole::User => {
                    last_user_id = msg.id.clone();
                    MessageInfo::User {
                        id: msg.id.clone(),
                        session_id: msg.session_id.clone(),
                        time: UserTime { created },
                        agent: msg
                            .metadata
                            .get("agent")
                            .and_then(|v| v.as_str())
                            .unwrap_or("general")
                            .to_string(),
                        model: V2ModelRef {
                            provider_id: msg
                                .metadata
                                .get("model_provider")
                                .and_then(|v| v.as_str())
                                .unwrap_or(provider_id)
                                .to_string(),
                            model_id: msg
                                .metadata
                                .get("model_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(model_id)
                                .to_string(),
                        },
                        format: None,
                        summary: None,
                        system: None,
                        tools: None,
                        variant: msg
                            .metadata
                            .get("variant")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    }
                }
                _ => {
                    let input = msg
                        .metadata
                        .get("tokens_input")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        .clamp(0, i32::MAX as i64) as i32;
                    let output = msg
                        .metadata
                        .get("tokens_output")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        .clamp(0, i32::MAX as i64) as i32;
                    MessageInfo::Assistant {
                        id: msg.id.clone(),
                        session_id: msg.session_id.clone(),
                        time: AssistantTime {
                            created,
                            completed: Some(created),
                        },
                        parent_id: if last_user_id.is_empty() {
                            msg.id.clone()
                        } else {
                            last_user_id.clone()
                        },
                        model_id: msg
                            .metadata
                            .get("model_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(model_id)
                            .to_string(),
                        provider_id: msg
                            .metadata
                            .get("model_provider")
                            .and_then(|v| v.as_str())
                            .unwrap_or(provider_id)
                            .to_string(),
                        mode: msg
                            .metadata
                            .get("mode")
                            .and_then(|v| v.as_str())
                            .unwrap_or("default")
                            .to_string(),
                        agent: msg
                            .metadata
                            .get("agent")
                            .and_then(|v| v.as_str())
                            .unwrap_or("general")
                            .to_string(),
                        path: MessagePath {
                            cwd: ".".to_string(),
                            root: ".".to_string(),
                        },
                        summary: None,
                        cost: msg
                            .metadata
                            .get("cost")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        tokens: AssistantTokens {
                            total: Some(input.saturating_add(output)),
                            input,
                            output,
                            reasoning: 0,
                            cache: CacheTokens { read: 0, write: 0 },
                        },
                        error: None,
                        structured: None,
                        variant: msg
                            .metadata
                            .get("variant")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        finish: msg
                            .metadata
                            .get("finish_reason")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    }
                }
            };

            out.push(MessageWithParts { info, parts });
        }

        out
    }

    async fn summarize_session(
        session: &mut Session,
        session_id: &str,
        provider_id: &str,
        model_id: &str,
        provider: &dyn Provider,
    ) -> anyhow::Result<()> {
        let directory = session.directory.clone();
        let worktree = std::path::Path::new(&directory);
        let last_user = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::User))
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let messages = Self::to_message_with_parts(&session.messages, provider_id, model_id);
        summarize_into_session(
            &SummarizeInput {
                session_id: session_id.to_string(),
                message_id: last_user,
            },
            session,
            &messages,
            worktree,
            Some(provider),
            Some(model_id),
            None,
        )
        .await?;

        Ok(())
    }

    fn prune_after_loop(session: &mut Session) {
        let mut tool_name_by_call: HashMap<String, String> = HashMap::new();
        for msg in &session.messages {
            for part in &msg.parts {
                if let PartType::ToolCall { id, name, .. } = &part.part_type {
                    tool_name_by_call.insert(id.clone(), name.clone());
                }
            }
        }

        let mut prune_messages: Vec<MessageForPrune> = session
            .messages
            .iter()
            .map(|m| {
                let parts: Vec<PruneToolPart> = m
                    .parts
                    .iter()
                    .filter_map(|p| match &p.part_type {
                        PartType::ToolResult {
                            tool_call_id,
                            content,
                            is_error,
                        } => Some(PruneToolPart {
                            id: p.id.clone(),
                            tool: tool_name_by_call
                                .get(tool_call_id)
                                .cloned()
                                .unwrap_or_default(),
                            output: content.clone(),
                            status: if *is_error {
                                ToolPartStatus::Error
                            } else {
                                ToolPartStatus::Completed
                            },
                            compacted: None,
                        }),
                        _ => None,
                    })
                    .collect();
                MessageForPrune {
                    role: match m.role {
                        MessageRole::User => "user".to_string(),
                        _ => "assistant".to_string(),
                    },
                    parts,
                    summary: false,
                }
            })
            .collect();

        let engine = CompactionEngine::new(CompactionConfig::default());
        let pruned_ids = engine.prune(&mut prune_messages);
        if pruned_ids.is_empty() {
            return;
        }
        let pruned: HashSet<String> = pruned_ids.into_iter().collect();
        for msg in &mut session.messages {
            for part in &mut msg.parts {
                if !pruned.contains(&part.id) {
                    continue;
                }
                if let PartType::ToolResult { content, .. } = &mut part.part_type {
                    let compacted = content.chars().take(200).collect::<String>();
                    *content = format!("[tool result compacted]\n{}", compacted);
                }
            }
        }

        // Record the compacting timestamp so the session DB row reflects that pruning occurred.
        session.time.compacting = Some(chrono::Utc::now().timestamp_millis());
        session.touch();
    }

    fn trigger_compaction(session: &mut Session, messages: &[SessionMessage]) -> Option<String> {
        let total_messages = messages.len();
        if total_messages < 10 {
            return None;
        }

        let keep_count = total_messages / 2;
        let summary_parts: Vec<String> = messages
            .iter()
            .take(keep_count)
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();

        let summary = format!(
            "Compacted {} messages. Summary: {}...",
            total_messages - keep_count,
            summary_parts
                .join(" ")
                .chars()
                .take(500)
                .collect::<String>()
        );

        // Persist the compaction summary as a Compaction part on a new assistant message.
        // This mirrors the TS behavior where compaction creates an assistant message with
        // summary=true and a compaction part, so that filter_compacted_messages can find it.
        let mut compaction_msg = SessionMessage::assistant(session.id.clone());
        compaction_msg.parts.push(crate::MessagePart {
            id: format!("prt_{}", uuid::Uuid::new_v4()),
            part_type: PartType::Compaction {
                summary: summary.clone(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
        session.messages.push(compaction_msg);

        // Set the compacting timestamp on the session.
        session.time.compacting = Some(chrono::Utc::now().timestamp_millis());
        session.touch();

        Some(summary)
    }
}

impl Default for SessionPrompt {
    fn default() -> Self {
        Self::new(Arc::new(RwLock::new(SessionStateManager::new())))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PromptError {
    #[error("Session is busy: {0}")]
    Busy(String),
    #[error("No user message found")]
    NoUserMessage,
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Cancelled")]
    Cancelled,
}

/// Regex that matches `@reference` patterns. We use a capturing group for the
/// preceding character instead of a lookbehind (unsupported by the `regex` crate).
/// Group 1 = preceding char (or empty at start of string), Group 2 = the reference name.
const FILE_REFERENCE_REGEX: &str = r"(?:^|([^\w`]))@(\.?[^\s`,.]*(?:\.[^\s`,.]+)*)";

pub async fn resolve_prompt_parts(
    template: &str,
    worktree: &std::path::Path,
    known_agents: &[String],
) -> Vec<PartInput> {
    let mut parts = vec![PartInput::Text {
        text: template.to_string(),
    }];

    let re = regex::Regex::new(FILE_REFERENCE_REGEX).unwrap();
    let mut seen = std::collections::HashSet::new();

    for cap in re.captures_iter(template) {
        // Group 1 is the preceding char — if it matched a word char or backtick
        // the overall pattern wouldn't match (they're excluded by [^\w`]).
        // Group 2 is the actual reference name.
        if let Some(name) = cap.get(2) {
            let name = name.as_str();
            if name.is_empty() || seen.contains(name) {
                continue;
            }
            seen.insert(name.to_string());

            let filepath = if name.starts_with("~/") {
                if let Some(home) = dirs::home_dir() {
                    home.join(&name[2..])
                } else {
                    continue;
                }
            } else {
                worktree.join(name)
            };

            if let Ok(metadata) = tokio::fs::metadata(&filepath).await {
                let url = format!("file://{}", filepath.display());

                if metadata.is_dir() {
                    parts.push(PartInput::File {
                        url,
                        filename: Some(name.to_string()),
                        mime: Some("application/x-directory".to_string()),
                    });
                } else {
                    parts.push(PartInput::File {
                        url,
                        filename: Some(name.to_string()),
                        mime: Some("text/plain".to_string()),
                    });
                }
            } else if known_agents.iter().any(|a| a == name) {
                // Not a file — check if it's a known agent name
                parts.push(PartInput::Agent {
                    name: name.to_string(),
                });
            }
        }
    }

    parts
}

pub fn extract_file_references(template: &str) -> Vec<String> {
    let re = regex::Regex::new(FILE_REFERENCE_REGEX).unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    for cap in re.captures_iter(template) {
        if let Some(name) = cap.get(2) {
            let name = name.as_str().to_string();
            if !name.is_empty() && !seen.contains(&name) {
                seen.insert(name.clone());
                result.push(name);
            }
        }
    }

    result
}

pub fn tool_definitions_from_schemas(schemas: Vec<ToolSchema>) -> Vec<ToolDefinition> {
    schemas
        .into_iter()
        .map(|s| ToolDefinition {
            name: s.name,
            description: Some(s.description),
            parameters: s.parameters,
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub struct SubtaskExecutor {
    pub agent_name: String,
    pub prompt: String,
    pub description: Option<String>,
    pub model: Option<ModelRef>,
    pub agent_params: AgentParams,
    /// A subagent sharing its parent's proxy shares its rate-limit budget.
    pub session_id: String,
}

impl SubtaskExecutor {
    pub fn new(agent_name: &str, prompt: &str) -> Self {
        Self {
            agent_name: agent_name.to_string(),
            prompt: prompt.to_string(),
            description: None,
            model: None,
            agent_params: AgentParams::default(),
            session_id: format!("sub_{}_{}", agent_name, uuid::Uuid::new_v4().simple()),
        }
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    pub fn with_model(mut self, model: ModelRef) -> Self {
        self.model = Some(model);
        self
    }

    pub async fn execute(
        &self,
        provider: Arc<dyn Provider>,
        tool_registry: &opencode_tool::ToolRegistry,
        ctx: &opencode_tool::ToolContext,
    ) -> anyhow::Result<String> {
        let model = self.model.as_ref().cloned().unwrap_or(ModelRef {
            provider_id: "default".to_string(),
            model_id: "default".to_string(),
        });
        let model_ref = format!("{}:{}", model.provider_id, model.model_id);
        let title = self
            .description
            .clone()
            .unwrap_or_else(|| "Subtask".to_string());

        let subsession_id = ctx
            .do_create_subsession(
                self.agent_name.clone(),
                Some(title.clone()),
                Some(model_ref),
                vec!["todowrite".to_string(), "todoread".to_string()],
            )
            .await
            .unwrap_or_else(|_| format!("task_{}_{}", self.agent_name, uuid::Uuid::new_v4()));

        if let Ok(output) = ctx
            .do_prompt_subsession(subsession_id.clone(), self.prompt.clone())
            .await
        {
            return Ok(format!(
                "task_id: {} (for resuming to continue this task if needed)\n\n<task_result>\n{}\n</task_result>",
                subsession_id, output
            ));
        }

        let output = self.execute_inline(provider, tool_registry, &[]).await?;
        Ok(format!(
            "task_id: {} (for resuming to continue this task if needed)\n\n<task_result>\n{}\n</task_result>",
            subsession_id, output
        ))
    }

    /// Runs the subagent without a context. Kept for callers that have no `ToolContext`
    /// to hand; behaviourally identical to `execute_inline_with_ctx` apart from the
    /// abort token, which defaults to a token that is never cancelled.
    pub async fn execute_inline(
        &self,
        provider: Arc<dyn Provider>,
        tool_registry: &opencode_tool::ToolRegistry,
        disabled_tools: &[String],
    ) -> anyhow::Result<String> {
        let ctx = opencode_tool::ToolContext::new(
            format!("task_{}", self.agent_name),
            "subagent".to_string(),
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string()),
        );
        self.execute_inline_with_ctx(provider, tool_registry, disabled_tools, ctx)
            .await
    }

    /// Runs the subagent as a real agent loop: stream, execute the tools the model asks
    /// for, feed the results back, repeat until it answers or a bound is hit.
    ///
    /// Every bound is deliberate. A subagent runs inside the PARENT's tool loop, so a
    /// subagent that can neither be cancelled nor time out takes the parent down with
    /// it — that was the reported hang.
    pub async fn execute_inline_with_ctx(
        &self,
        provider: Arc<dyn Provider>,
        tool_registry: &opencode_tool::ToolRegistry,
        disabled_tools: &[String],
        ctx: opencode_tool::ToolContext,
    ) -> anyhow::Result<String> {
        let model = self.model.as_ref().cloned().unwrap_or(ModelRef {
            provider_id: "default".to_string(),
            model_id: "default".to_string(),
        });
        let disabled: HashSet<&str> = disabled_tools.iter().map(|s| s.as_str()).collect();
        let tools = tool_registry.list_schemas().await;
        let tool_defs: Vec<ToolDefinition> = tools
            .into_iter()
            .filter(|s| !disabled.contains(s.name.as_str()))
            .map(|s| ToolDefinition {
                name: s.name,
                description: Some(s.description),
                parameters: s.parameters,
            })
            .collect();

        let deadline = Instant::now() + subagent_total_timeout();
        let mut messages: Vec<Message> = vec![Message::user(&self.prompt)];
        let mut final_text = String::new();
        let mut last_error: Option<String> = None;

        for step in 0..SUBAGENT_MAX_STEPS {
            if ctx.is_cancelled() {
                anyhow::bail!("subagent aborted before step {step}");
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "subagent exceeded its {}s total deadline before step {step}",
                    subagent_total_timeout().as_secs()
                );
            }

            let request = ChatRequest {
                model: model.model_id.clone(),
                messages: messages.clone(),
                max_tokens: Some(self.agent_params.max_tokens.unwrap_or(8192)),
                temperature: self.agent_params.temperature,
                system: None,
                tools: if tool_defs.is_empty() {
                    None
                } else {
                    Some(tool_defs.clone())
                },
                stream: Some(true),
                top_p: self.agent_params.top_p,
                variant: None,
                provider_options: None,
                session_id: Some(self.session_id.clone()),
            };

            // The per-request window covers opening the stream AND draining it. A
            // provider that accepts the connection and then goes quiet is still the
            // same request, so bounding only the handshake would leave a stall to be
            // caught by the far larger total deadline — which is long enough to look
            // like a hang to the parent turn.
            let request_deadline =
                std::cmp::min(deadline, Instant::now() + subagent_request_timeout());
            let mut stream = match tokio::time::timeout_at(
                tokio::time::Instant::from_std(request_deadline),
                provider.chat_stream(request),
            )
            .await
            {
                Err(_) => anyhow::bail!(
                    "subagent provider timeout after {}s at step {step}",
                    subagent_request_timeout().as_secs()
                ),
                Ok(Err(error)) => return Err(error.into()),
                Ok(Ok(stream)) => stream,
            };

            let mut text = String::new();
            let mut pending: HashMap<String, (String, String)> = HashMap::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut step_error: Option<String> = None;

            // A drain cut short by the deadline must surface as an error. Returning the
            // text collected so far would tell the parent the subagent finished when it
            // was interrupted mid-answer.
            let mut truncated = false;
            loop {
                let next = tokio::time::timeout_at(
                    tokio::time::Instant::from_std(request_deadline),
                    stream.next(),
                )
                .await;
                let event = match next {
                    Ok(Some(event)) => event,
                    Ok(None) => break,
                    Err(_) => {
                        truncated = true;
                        break;
                    }
                };
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        step_error = Some(error.to_string());
                        break;
                    }
                };
                if ctx.is_cancelled() {
                    anyhow::bail!("subagent aborted during step {step}");
                }
                match event {
                    StreamEvent::TextDelta(delta) => text.push_str(&delta),
                    StreamEvent::ToolCallStart { id, name } => {
                        pending.insert(id.clone(), (name, String::new()));
                    }
                    StreamEvent::ToolCallDelta { id, input } => {
                        pending.entry(id.clone()).or_insert_with(|| (String::new(), String::new())).1.push_str(&input);
                    }
                    StreamEvent::ToolCallEnd { id, name, input } => {
                        pending.remove(&id);
                        tool_calls.push((id, name, input));
                    }
                    _ => {}
                }
            }
            // A provider that opens a stream and then goes quiet must not wedge the
            // parent: the deadline already fired in `timeout_at`, so fall through with
            // whatever was collected and let the bounds decide the outcome.
            if let Some(error) = step_error {
                last_error = Some(error);
            }
            if truncated {
                anyhow::bail!(
                    "subagent provider timeout after {}s at step {step}",
                    subagent_request_timeout().as_secs()
                );
            }

            if !text.is_empty() {
                final_text = text.clone();
            }

            if tool_calls.is_empty() {
                if final_text.is_empty() {
                    if let Some(error) = last_error {
                        anyhow::bail!("subagent produced no answer: {error}");
                    }
                    anyhow::bail!("subagent produced no answer at step {step}");
                }
                return Ok(final_text);
            }

            let mut assistant_content: Vec<opencode_provider::message::ContentPart> = Vec::new();
            if !text.is_empty() {
                assistant_content.push(opencode_provider::message::ContentPart {
                    content_type: "text".to_string(),
                    text: Some(text),
                    image_url: None,
                    tool_use: None,
                    tool_result: None,
                    cache_control: None,
                    filename: None,
                    media_type: None,
                    provider_options: None,
                });
            }
            for (id, name, input) in &tool_calls {
                assistant_content.push(opencode_provider::message::ContentPart {
                    content_type: "tool_use".to_string(),
                    text: None,
                    image_url: None,
                    tool_use: Some(opencode_provider::message::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    }),
                    tool_result: None,
                    cache_control: None,
                    filename: None,
                    media_type: None,
                    provider_options: None,
                });
            }
            messages.push(Message {
                role: Role::Assistant,
                content: Content::Parts(assistant_content),
                cache_control: None,
                provider_options: None,
            });

            let mut results: Vec<opencode_provider::message::ContentPart> = Vec::new();
            for (id, name, input) in &tool_calls {
                let outcome = tool_registry
                    .execute(name, input.clone(), ctx.clone())
                    .await;
                let (output, is_error) = match outcome {
                    Ok(result) => (result.output, false),
                    Err(error) => (error.to_string(), true),
                };
                results.push(opencode_provider::message::ContentPart {
                    content_type: "tool_result".to_string(),
                    text: None,
                    image_url: None,
                    tool_use: None,
                    tool_result: Some(opencode_provider::message::ToolResult {
                        tool_use_id: id.clone(),
                        content: output,
                        is_error: Some(is_error),
                    }),
                    cache_control: None,
                    filename: None,
                    media_type: None,
                    provider_options: None,
                });
            }
            messages.push(Message {
                role: Role::Tool,
                content: Content::Parts(results),
                cache_control: None,
                provider_options: None,
            });
        }

        anyhow::bail!("subagent hit the step limit of {SUBAGENT_MAX_STEPS} without a final answer");
    }
}

fn hook_payload_object(
    payload: &serde_json::Value,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    payload
        .get("output")
        .and_then(|value| value.as_object())
        .or_else(|| payload.as_object())
        .or_else(|| payload.get("data").and_then(|value| value.as_object()))
}

/// omo reads `message.info.role` / `message.info.sessionID` / `message.info.model`, while
/// `SessionMessage` serialises flat with a PascalCase role. This projects the wire shape
/// plugins expect, so message-transform hooks stop throwing on a missing `info`.
fn messages_to_wire(messages: &[SessionMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::System => "system",
                MessageRole::Tool => "tool",
            };
            let mut info = serde_json::Map::new();
            info.insert("id".into(), serde_json::json!(m.id));
            info.insert("sessionID".into(), serde_json::json!(m.session_id));
            info.insert("role".into(), serde_json::json!(role));
            info.insert("created".into(), serde_json::json!(m.created_at));
            for (from, into) in [
                ("model_provider", "providerID"),
                ("model_id", "modelID"),
                ("model_variant", "modelVariant"),
            ] {
                if let Some(v) = m.metadata.get(from) {
                    info.insert(into.into(), v.clone());
                }
            }
            serde_json::json!({
                "info": serde_json::Value::Object(info),
                "parts": serde_json::to_value(&m.parts).unwrap_or_else(|_| serde_json::json!([])),
            })
        })
        .collect()
}

/// Inverse of [`messages_to_wire`]. Returns `None` if any entry is not a message we can
/// restore, so the caller can keep its original list rather than send a mangled payload.
fn messages_from_wire(values: &[serde_json::Value]) -> Option<Vec<SessionMessage>> {
    values
        .iter()
        .enumerate()
        .map(|(index, v)| {
            let info = v.get("info")?.as_object()?;
            let role = match info.get("role")?.as_str()? {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "system" => MessageRole::System,
                "tool" => MessageRole::Tool,
                _ => return None,
            };
            let mut metadata = serde_json::Map::new();
            for (from, into) in [
                ("providerID", "model_provider"),
                ("modelID", "model_id"),
                ("modelVariant", "model_variant"),
            ] {
                if let Some(val) = info.get(from) {
                    metadata.insert(into.into(), val.clone());
                }
            }
            let created = info
                .get("created")
                .and_then(|c| serde_json::from_value(c.clone()).ok())
                .unwrap_or_else(chrono::Utc::now);
            let metadata: std::collections::HashMap<String, serde_json::Value> =
                metadata.into_iter().collect();
            let session_id = info.get("sessionID")?.as_str()?.to_string();
            // Plugins may splice in their own turns (omo injects `<peer_message>` as a bare
            // `{role, sessionID}`) that carry no id. Rejecting one would discard the entire
            // batch via the `collect` into `Option`, silently dropping the injection, so
            // derive a stable id from position instead. Mirrors how a missing `created` is
            // already tolerated above.
            let id = info
                .get("id")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("msg_synthetic_{session_id}_{index}"));
            let parts = parts_from_wire(v.get("parts")?, &session_id, index)?;
            Some(SessionMessage {
                id,
                session_id,
                role,
                parts,
                created_at: created,
                metadata,
                usage: None,
            })
        })
        .collect()
}

/// Parse a hook-supplied part list.
///
/// Two shapes reach us. Ours nests the variant (`{"id":…,"part_type":{"type":"text",…}}`)
/// because that is what `serde_json::to_value(&MessagePart)` emits. Plugins emit the
/// original flat TS shape instead (`{"type":"text","text":…}`), which is exactly what the
/// internally tagged `PartType` expects — so it deserializes on its own, provided the
/// wrapper fields we require are supplied. Rejecting a flat part would fail the whole
/// message list, so nothing here is allowed to bail on a missing id or timestamp.
fn parts_from_wire(
    value: &serde_json::Value,
    session_id: &str,
    message_index: usize,
) -> Option<Vec<MessagePart>> {
    let values = value.as_array()?;
    let mut parts = Vec::with_capacity(values.len());
    for (part_index, part) in values.iter().enumerate() {
        if let Ok(parsed) = serde_json::from_value::<MessagePart>(part.clone()) {
            parts.push(parsed);
            continue;
        }
        let part_type = serde_json::from_value::<PartType>(part.clone()).ok()?;
        let id = part
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("prt_synthetic_{session_id}_{message_index}_{part_index}"));
        let created_at = part
            .get("created_at")
            .or_else(|| part.get("time").and_then(|time| time.get("start")))
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_else(chrono::Utc::now);
        let message_id = part
            .get("message_id")
            .or_else(|| part.get("messageID"))
            .and_then(|value| value.as_str())
            .map(str::to_string);
        parts.push(MessagePart {
            id,
            part_type,
            created_at,
            message_id,
        });
    }
    Some(parts)
}

fn apply_chat_messages_hook_outputs(
    messages: &mut Vec<SessionMessage>,
    hook_outputs: Vec<opencode_plugin::HookOutput>,
) {
    for output in hook_outputs {
        let Some(payload) = output.payload.as_ref() else {
            continue;
        };
        let Some(object) = hook_payload_object(payload) else {
            continue;
        };
        let Some(next_messages) = object.get("messages").and_then(|value| value.as_array()) else {
            continue;
        };
        // The payload is in the wire shape we sent, so it must be restored through the same
        // projection. Anything unrecognised leaves the current messages untouched rather than
        // replacing them with a partial list.
        if let Some(next) = messages_from_wire(next_messages) {
            *messages = next;
        }
    }
}

fn apply_chat_message_hook_outputs(
    message: &mut SessionMessage,
    hook_outputs: Vec<opencode_plugin::HookOutput>,
) {
    for output in hook_outputs {
        let Some(payload) = output.payload.as_ref() else {
            continue;
        };
        let Some(object) = hook_payload_object(payload) else {
            continue;
        };
        if let Some(next_message) = object.get("message") {
            if let Ok(parsed) = serde_json::from_value::<SessionMessage>(next_message.clone()) {
                *message = parsed;
            }
        }
        if let Some(next_parts) = object.get("parts").and_then(|value| value.as_array()) {
            let parsed = serde_json::from_value::<Vec<crate::MessagePart>>(serde_json::Value::Array(
                next_parts.clone(),
            ));
            if let Ok(parts) = parsed {
                message.parts = parts;
            }
        }
    }
}

pub fn should_compact(messages: &[SessionMessage], max_tokens: u64) -> bool {
    let total_chars: usize = messages
        .iter()
        .map(|m| {
            m.parts
                .iter()
                .filter_map(|p| match &p.part_type {
                    PartType::Text { text, .. } => Some(text.len()),
                    _ => None,
                })
                .sum::<usize>()
        })
        .sum();

    let estimated_tokens = total_chars / 4;
    estimated_tokens > max_tokens as usize
}

pub fn trigger_compaction(session: &mut Session, messages: &[SessionMessage]) -> Option<String> {
    if !should_compact(messages, 100000) {
        return None;
    }

    let text_content: String = messages
        .iter()
        .rev()
        .take(10)
        .flat_map(|m| {
            m.parts.iter().filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
        })
        .collect::<Vec<_>>()
        .join("\n");

    let summary = format!(
        "[Context Compaction Triggered]\nRecent messages summarized:\n{}",
        &text_content[..text_content.len().min(500)]
    );

    // Persist the compaction summary as a Compaction part on a new assistant message.
    let mut compaction_msg = SessionMessage::assistant(session.id.clone());
    compaction_msg.parts.push(crate::MessagePart {
        id: format!("prt_{}", uuid::Uuid::new_v4()),
        part_type: PartType::Compaction {
            summary: summary.clone(),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    session.messages.push(compaction_msg);

    // Set the compacting timestamp on the session.
    session.time.compacting = Some(chrono::Utc::now().timestamp_millis());
    session.touch();

    Some(summary)
}

// Additional prompt functions for advanced features

const STRUCTURED_OUTPUT_DESCRIPTION: &str = r#"Use this tool to return your final response in the requested structured format.

IMPORTANT:
- You MUST call this tool exactly once at the end of your response
- The input must be valid JSON matching the required schema
- Complete all necessary research and tool calls BEFORE calling this tool
- This tool provides your final answer - no further actions are taken after calling it"#;

const STRUCTURED_OUTPUT_SYSTEM_PROMPT: &str = r#"IMPORTANT: The user has requested structured output. You MUST use the StructuredOutput tool to provide your final response. Do NOT respond with plain text - you MUST call the StructuredOutput tool with your answer formatted according to the schema."#;

pub struct StructuredOutputConfig {
    pub schema: serde_json::Value,
}

pub fn create_structured_output_tool(schema: serde_json::Value) -> ToolDefinition {
    let mut tool_schema = schema;
    if let Some(obj) = tool_schema.as_object_mut() {
        obj.remove("$schema");
    }

    ToolDefinition {
        name: "StructuredOutput".to_string(),
        description: Some(STRUCTURED_OUTPUT_DESCRIPTION.to_string()),
        parameters: tool_schema,
    }
}

pub fn structured_output_system_prompt() -> String {
    STRUCTURED_OUTPUT_SYSTEM_PROMPT.to_string()
}

pub fn extract_structured_output(parts: &[crate::MessagePart]) -> Option<serde_json::Value> {
    for part in parts {
        if let PartType::ToolCall { name, input, .. } = &part.part_type {
            if name == "StructuredOutput" {
                return Some(input.clone());
            }
        }
    }
    None
}

const PROMPT_PLAN: &str = r#"You are in PLAN mode. The user wants you to create a plan before executing.

## Your task:
1. Understand the user's request thoroughly
2. Explore the codebase to understand the current state
3. Create a detailed plan in the plan file
4. Use the plan_exit tool when done planning

## Important:
- Do NOT make any edits or run commands (except read operations)
- Only create/modify the plan file
- Ask clarifying questions if needed
- Use explore subagent to understand the codebase"#;

const BUILD_SWITCH: &str = r#"The user has approved your plan and wants you to execute it.

## Your task:
1. Execute the plan step by step
2. Make the necessary changes to the codebase
3. Test your changes
4. Verify the implementation matches the plan

## Important:
- You may now use all tools including edit, write, bash
- Follow the plan closely but adapt as needed
- Report progress to the user"#;

pub fn insert_reminders(
    messages: &[SessionMessage],
    agent_name: &str,
    was_plan: bool,
) -> Vec<SessionMessage> {
    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User));

    if let Some(idx) = last_user_idx {
        let mut messages = messages.to_vec();

        if agent_name == "plan" {
            let reminder_text = PROMPT_PLAN.to_string();
            messages[idx].parts.push(crate::MessagePart {
                id: format!("prt_{}", uuid::Uuid::new_v4()),
                part_type: PartType::Text {
                    text: reminder_text,
                    synthetic: None,
                    ignored: None,
                },
                created_at: chrono::Utc::now(),
                message_id: None,
            });
        }

        if was_plan && agent_name == "build" {
            let reminder_text = BUILD_SWITCH.to_string();
            messages[idx].parts.push(crate::MessagePart {
                id: format!("prt_{}", uuid::Uuid::new_v4()),
                part_type: PartType::Text {
                    text: reminder_text,
                    synthetic: None,
                    ignored: None,
                },
                created_at: chrono::Utc::now(),
                message_id: None,
            });
        }

        messages
    } else {
        messages.to_vec()
    }
}

pub fn was_plan_agent(messages: &[SessionMessage]) -> bool {
    messages.iter().any(|m| {
        if let Some(agent) = m.metadata.get("agent") {
            agent.as_str() == Some("plan")
        } else {
            false
        }
    })
}

pub struct ResolvedTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub fn merge_tool_definitions(
    base: Vec<ToolDefinition>,
    extra: Vec<ToolDefinition>,
) -> Vec<ToolDefinition> {
    let mut merged: HashMap<String, ToolDefinition> = HashMap::new();
    for tool in base.into_iter().chain(extra) {
        merged.insert(tool.name.clone(), tool);
    }

    let mut tools: Vec<ToolDefinition> = merged.into_values().collect();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

pub async fn resolve_tools_with_mcp(
    tool_registry: &opencode_tool::ToolRegistry,
    mcp_tools: Vec<ToolDefinition>,
) -> Vec<ToolDefinition> {
    let base = tool_registry
        .list_schemas()
        .await
        .into_iter()
        .map(|s| ToolDefinition {
            name: s.name,
            description: Some(s.description),
            parameters: s.parameters,
        })
        .collect();

    merge_tool_definitions(base, mcp_tools)
}

pub async fn resolve_tools_with_mcp_registry(
    tool_registry: &opencode_tool::ToolRegistry,
    mcp_registry: Option<&opencode_mcp::McpToolRegistry>,
) -> Vec<ToolDefinition> {
    let dynamic_mcp_tools = if let Some(registry) = mcp_registry {
        registry
            .list()
            .await
            .into_iter()
            .map(|tool| ToolDefinition {
                name: tool.full_name,
                description: tool.description,
                parameters: tool.input_schema,
            })
            .collect()
    } else {
        Vec::new()
    };

    resolve_tools_with_mcp(tool_registry, dynamic_mcp_tools).await
}

pub async fn resolve_tools(tool_registry: &opencode_tool::ToolRegistry) -> Vec<ToolDefinition> {
    resolve_tools_with_mcp_registry(tool_registry, None).await
}

/// Tool definitions for the default registry, with no MCP tools merged in.
pub async fn default_tool_definitions() -> Vec<ToolDefinition> {
    let tool_registry = opencode_tool::create_default_registry().await;
    resolve_tools(&tool_registry).await
}

pub fn max_steps_for_agent(agent_steps: Option<u32>) -> u32 {
    agent_steps.unwrap_or(MAX_STEPS)
}

pub fn generate_session_title(first_user_message: &str) -> String {
    let first_line = first_user_message.lines().next().unwrap_or("").trim();

    if first_line.len() > 100 {
        format!("{}...", &first_line[..97])
    } else if first_line.is_empty() {
        "New Session".to_string()
    } else {
        first_line.to_string()
    }
}

/// Generate a session title using an LLM (matching TS `ensureTitle`).
/// Falls back to `generate_session_title` on any failure.
pub async fn generate_session_title_llm(
    first_user_message: &str,
    provider: Arc<dyn Provider>,
    model_id: &str,
    session_id: &str,
) -> String {
    let fallback = generate_session_title(first_user_message);

    let request = ChatRequest {
        model: model_id.to_string(),
        messages: vec![Message {
            role: Role::User,
            content: Content::Text(format!(
                "Generate a short title (under 80 chars) for this conversation. \
                     Reply with ONLY the title, no quotes or explanation.\n\n{}",
                first_user_message
            )),
            cache_control: None,
            provider_options: None,
        }],
        tools: None,
        // The session id is what routes this call through the per-session residential
        // proxy. Left empty, `for_session` hands back a direct client, Zen sees traffic
        // from outside OpenCode and answers 403 FreeTierError — which is why every title
        // call failed while ordinary agent turns succeeded.
        session_id: Some(session_id.to_string()),
        system: Some(
            "You generate concise conversation titles. Reply with only the title.".to_string(),
        ),
        max_tokens: Some(100),
        temperature: Some(0.0),
        top_p: None,
        stream: None,
        provider_options: None,
        variant: None,
    };

    match provider.chat(request).await {
        Ok(response) => {
            // Extract text from the first choice
            let text = response
                .choices
                .first()
                .map(|c| match &c.message.content {
                    Content::Text(t) => t.clone(),
                    Content::Parts(parts) => parts
                        .iter()
                        .filter_map(|p| p.text.clone())
                        .collect::<Vec<_>>()
                        .join(""),
                })
                .unwrap_or_default();

            // Clean up: remove thinking tags, take first non-empty line
            let cleaned = text
                .replace(|c: char| c == '"' || c == '\'', "")
                .lines()
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && !l.starts_with("<think>"))
                .unwrap_or("")
                .to_string();

            if cleaned.is_empty() {
                fallback
            } else if cleaned.len() > 100 {
                format!("{}...", &cleaned[..97])
            } else {
                cleaned
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to generate title via LLM, using fallback");
            fallback
        }
    }
}

/// Input for the `shell()` function.
#[derive(Debug, Clone)]
pub struct ShellInput {
    pub session_id: String,
    pub command_str: String,
    pub agent: Option<String>,
    pub model: Option<ModelRef>,
    pub abort: Option<CancellationToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellInvocation {
    program: String,
    args: Vec<String>,
}

fn resolve_shell_invocation(shell_env: Option<&str>, command: &str) -> ShellInvocation {
    let shell = shell_env
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("/bin/bash")
        .to_string();
    let shell_name = Path::new(&shell)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_lowercase();
    let escaped = serde_json::to_string(command).unwrap_or_else(|_| "\"\"".to_string());

    let args = match shell_name.as_str() {
        "nu" | "fish" => vec!["-c".to_string(), command.to_string()],
        "zsh" => vec![
            "-c".to_string(),
            "-l".to_string(),
            format!(
                r#"
[[ -f ~/.zshenv ]] && source ~/.zshenv >/dev/null 2>&1 || true
[[ -f "${{ZDOTDIR:-$HOME}}/.zshrc" ]] && source "${{ZDOTDIR:-$HOME}}/.zshrc" >/dev/null 2>&1 || true
eval {}
"#,
                escaped
            ),
        ],
        "bash" => vec![
            "-c".to_string(),
            "-l".to_string(),
            format!(
                r#"
shopt -s expand_aliases
[[ -f ~/.bashrc ]] && source ~/.bashrc >/dev/null 2>&1 || true
eval {}
"#,
                escaped
            ),
        ],
        "cmd" => vec!["/c".to_string(), command.to_string()],
        "powershell" | "pwsh" => vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            command.to_string(),
        ],
        _ => vec!["-c".to_string(), command.to_string()],
    };

    ShellInvocation {
        program: shell,
        args,
    }
}

/// Execute a shell command in the session context (matching TS `SessionPrompt.shell`).
///
/// Creates a user message + assistant message with a tool call part recording
/// the shell execution and its output. The command is provided by the user
/// through the session UI and is intentionally executed as-is.
pub async fn shell_exec(input: &ShellInput, session: &mut Session) -> anyhow::Result<String> {
    // Create synthetic user message
    let _user_msg = session.add_user_message("The following tool was executed by the user");

    // Create assistant message with tool call
    let assistant_msg = session.add_assistant_message();
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    assistant_msg.add_tool_call(
        &call_id,
        "bash",
        serde_json::json!({ "command": input.command_str }),
    );

    let invocation =
        resolve_shell_invocation(std::env::var("SHELL").ok().as_deref(), &input.command_str);
    let abort = input.abort.clone().unwrap_or_else(CancellationToken::new);

    let mut command = tokio::process::Command::new(&invocation.program);
    command
        .args(&invocation.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = command.spawn()?;
    let mut stdout = String::new();
    let mut stderr = String::new();

    let stdout_task = child.stdout.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).to_string()
        })
    });
    let stderr_task = child.stderr.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).to_string()
        })
    });

    let mut aborted = false;
    tokio::select! {
        _ = child.wait() => {}
        _ = abort.cancelled() => {
            aborted = true;
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }

    if let Some(task) = stdout_task {
        if let Ok(out) = task.await {
            stdout = out;
        }
    }
    if let Some(task) = stderr_task {
        if let Ok(out) = task.await {
            stderr = out;
        }
    }

    let mut result = if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{}\n{}", stdout, stderr)
    };
    if aborted {
        result.push_str("\n\n<metadata>\nUser aborted the command\n</metadata>");
    }

    // Record the tool result
    assistant_msg.add_tool_result(&call_id, &result, aborted);

    Ok(result)
}

/// Input for the `command()` function.
#[derive(Debug, Clone)]
pub struct CommandInput {
    pub session_id: String,
    pub command: String,
    pub arguments: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub message_id: Option<String>,
    pub variant: Option<String>,
}

/// Resolve a command template with arguments (matching TS `SessionPrompt.command`).
///
/// Replaces `$1`, `$2`, etc. placeholders with positional arguments,
/// and `$ARGUMENTS` with the full argument string.
pub fn resolve_command_template(template: &str, arguments: &str) -> String {
    let args: Vec<&str> = arguments.split_whitespace().collect();

    // Find the highest placeholder index
    let mut max_index = 0u32;
    let placeholder_re = regex::Regex::new(r"\$(\d+)").unwrap();
    for cap in placeholder_re.captures_iter(template) {
        if let Ok(idx) = cap[1].parse::<u32>() {
            if idx > max_index {
                max_index = idx;
            }
        }
    }

    let has_arguments_placeholder = template.contains("$ARGUMENTS");

    // Replace $N placeholders
    let mut result = placeholder_re
        .replace_all(template, |caps: &regex::Captures| {
            let idx: usize = caps[1].parse().unwrap_or(0);
            if idx == 0 || idx > args.len() {
                return String::new();
            }
            let arg_idx = idx - 1;
            // Last placeholder swallows remaining args
            if idx as u32 == max_index {
                args[arg_idx..].join(" ")
            } else {
                args.get(arg_idx).unwrap_or(&"").to_string()
            }
        })
        .to_string();

    // Replace $ARGUMENTS
    result = result.replace("$ARGUMENTS", arguments);

    // If no placeholders and user provided arguments, append them
    if max_index == 0 && !has_arguments_placeholder && !arguments.trim().is_empty() {
        result = format!("{}\n\n{}", result, arguments);
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream;
    use opencode_provider::{
        ChatRequest, ChatResponse, ModelInfo, ProviderError, StreamEvent, StreamResult, StreamUsage,
    };
    use std::sync::Mutex as StdMutex;

    struct StaticModelProvider {
        model: Option<ModelInfo>,
    }

    impl StaticModelProvider {
        fn with_model(model_id: &str, context_window: u64, max_output_tokens: u64) -> Self {
            Self {
                model: Some(ModelInfo {
                    id: model_id.to_string(),
                    name: "Static Model".to_string(),
                    provider: "mock".to_string(),
                    context_window,
                    max_output_tokens,
                    supports_vision: false,
                    supports_tools: false,
                    cost_per_million_input: 0.0,
                    cost_per_million_output: 0.0,
                }),
            }
        }
    }

    #[async_trait]
    impl Provider for StaticModelProvider {
        fn id(&self) -> &str {
            "mock"
        }

        fn name(&self) -> &str {
            "Mock"
        }

        fn models(&self) -> Vec<ModelInfo> {
            self.model.clone().into_iter().collect()
        }

        fn get_model(&self, id: &str) -> Option<&ModelInfo> {
            self.model.as_ref().filter(|model| model.id == id)
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            Err(ProviderError::InvalidRequest(
                "chat() not used in this test".to_string(),
            ))
        }

        async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    struct ScriptedStreamProvider {
        model: ModelInfo,
        events: Vec<StreamEvent>,
    }

    #[async_trait]
    impl Provider for ScriptedStreamProvider {
        fn id(&self) -> &str {
            "mock"
        }

        fn name(&self) -> &str {
            "Mock"
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![self.model.clone()]
        }

        fn get_model(&self, id: &str) -> Option<&ModelInfo> {
            if self.model.id == id {
                Some(&self.model)
            } else {
                None
            }
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            Err(ProviderError::InvalidRequest(
                "chat() not used in this test".to_string(),
            ))
        }

        async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
            Ok(Box::pin(stream::iter(
                self.events
                    .clone()
                    .into_iter()
                    .map(Result::<StreamEvent, ProviderError>::Ok),
            )))
        }
    }

    #[test]
    fn filter_compacted_messages_keeps_tail_after_last_compaction() {
        let session_id = "ses_test".to_string();
        let before = SessionMessage::user(session_id.clone(), "before");
        let mut compact = SessionMessage::assistant(session_id.clone());
        compact.parts.push(crate::MessagePart {
            id: "prt_compact".to_string(),
            part_type: PartType::Compaction {
                summary: "summary".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
        let after = SessionMessage::user(session_id, "after");

        let filtered = SessionPrompt::filter_compacted_messages(&[before, compact, after]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered[0]
            .parts
            .iter()
            .any(|p| matches!(p.part_type, PartType::Compaction { .. })));
    }

    #[test]
    fn insert_reminders_adds_plan_prompt_for_plan_agent() {
        let messages = vec![SessionMessage::user("ses_test", "plan this")];
        let output = insert_reminders(&messages, "plan", false);
        let last = output.last().unwrap();
        let injected = last
            .parts
            .iter()
            .filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(injected.contains("You are in PLAN mode"));
    }

    #[test]
    fn insert_reminders_adds_build_switch_after_plan() {
        let mut user = SessionMessage::user("ses_test", "execute this");
        user.metadata
            .insert("agent".to_string(), serde_json::json!("plan"));
        let output = insert_reminders(&[user], "build", true);
        let last = output.last().unwrap();
        let injected = last
            .parts
            .iter()
            .filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(injected.contains("The user has approved your plan"));
    }

    #[tokio::test]
    async fn prompt_with_update_hook_emits_incremental_snapshots() {
        let prompt = SessionPrompt::default();
        let mut session = Session::new("proj", ".");
        let provider = Arc::new(ScriptedStreamProvider {
            model: ModelInfo {
                id: "test-model".to_string(),
                name: "Test Model".to_string(),
                provider: "mock".to_string(),
                context_window: 8192,
                max_output_tokens: 1024,
                supports_vision: false,
                supports_tools: false,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
            },
            events: vec![
                StreamEvent::Start,
                StreamEvent::TextDelta("Hel".to_string()),
                StreamEvent::TextDelta("lo".to_string()),
                StreamEvent::FinishStep {
                    finish_reason: Some("stop".to_string()),
                    usage: StreamUsage {
                        prompt_tokens: 3,
                        completion_tokens: 2,
                        ..Default::default()
                    },
                    provider_metadata: None,
                },
                StreamEvent::Done,
            ],
        });

        let snapshots = Arc::new(StdMutex::new(Vec::<Session>::new()));
        let snapshot_sink = snapshots.clone();
        let hook: SessionUpdateHook = Arc::new(move |snapshot| {
            snapshot_sink
                .lock()
                .expect("snapshot lock should not poison")
                .push(snapshot.clone());
        });

        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: Some(ModelRef {
                provider_id: "mock".to_string(),
                model_id: "test-model".to_string(),
            }),
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            parts: vec![PartInput::Text {
                text: "Say hello".to_string(),
            }],
            tools: None,
        };

        prompt
            .prompt_with_update_hook(
                input,
                &mut session,
                provider,
                None,
                Vec::new(),
                AgentParams::default(),
                Some(hook),
            )
            .await
            .expect("prompt_with_update_hook should succeed");

        let snapshots_guard = snapshots.lock().expect("snapshot lock should not poison");
        assert!(snapshots_guard.len() >= 3);
        let saw_partial = snapshots_guard.iter().any(|snap| {
            snap.messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, MessageRole::Assistant))
                .map(|m| m.get_text() == "Hel")
                .unwrap_or(false)
        });
        assert!(
            saw_partial,
            "expected at least one streamed partial assistant snapshot"
        );
        drop(snapshots_guard);

        let final_text = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .map(SessionMessage::get_text)
            .unwrap_or_default();
        assert_eq!(final_text, "Hello");
    }

    #[tokio::test]
    async fn create_user_message_persists_pending_subtask_payload() {
        let prompt = SessionPrompt::default();
        let mut session = Session::new("proj", ".");
        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            parts: vec![PartInput::Subtask {
                prompt: "Inspect codegen path".to_string(),
                description: Some("Inspect codegen".to_string()),
                agent: "explore".to_string(),
            }],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let msg = session.messages.last().expect("user message should exist");
        let pending = msg
            .metadata
            .get("pending_subtasks")
            .and_then(|v| v.as_array())
            .expect("pending_subtasks metadata should exist");
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].get("agent").and_then(|v| v.as_str()),
            Some("explore")
        );
        assert_eq!(
            pending[0].get("prompt").and_then(|v| v.as_str()),
            Some("Inspect codegen path")
        );
        assert!(msg.parts.iter().any(|p| match &p.part_type {
            PartType::Subtask { status, .. } => status == "pending",
            _ => false,
        }));
    }

    #[tokio::test]
    async fn create_user_message_decodes_text_data_url() {
        let prompt = SessionPrompt::default();
        let mut session = Session::new("proj", ".");
        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            parts: vec![PartInput::File {
                url: "data:text/plain;base64,SGVsbG8=".to_string(),
                filename: Some("inline.txt".to_string()),
                mime: Some("text/plain".to_string()),
            }],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Hello"));
    }

    #[tokio::test]
    async fn create_user_message_file_url_with_range_reads_only_requested_lines() {
        let prompt = SessionPrompt::default();
        let mut session = Session::new("proj", ".");
        let temp_dir = tempfile::tempdir().expect("tempdir should create");
        let file_path = temp_dir.path().join("sample.rs");
        let content = (1..=30)
            .map(|n| format!("L{:02}", n))
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(&file_path, content)
            .await
            .expect("write should succeed");

        let mut url = url::Url::from_file_path(&file_path).expect("file path should convert");
        url.set_query(Some("start=10&end=20"));

        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            parts: vec![PartInput::File {
                url: url.to_string(),
                filename: Some("sample.rs".to_string()),
                mime: Some("text/plain".to_string()),
            }],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("L10"));
        assert!(text.contains("L20"));
        assert!(!text.contains("L09"));
        assert!(!text.contains("L21"));
    }

    #[tokio::test]
    async fn create_user_message_file_url_injects_nearby_instructions() {
        let prompt = SessionPrompt::default();
        let temp_dir = tempfile::tempdir().expect("tempdir should create");
        let project_root = temp_dir.path().to_path_buf();
        let src_dir = project_root.join("src");
        tokio::fs::create_dir_all(&src_dir)
            .await
            .expect("src directory should create");
        tokio::fs::write(src_dir.join("AGENTS.md"), "Prefer immutable updates")
            .await
            .expect("instructions should write");

        let file_path = src_dir.join("sample.rs");
        tokio::fs::write(&file_path, "fn main() {}")
            .await
            .expect("file should write");

        let mut session = Session::new("proj", project_root.to_string_lossy().to_string());
        let file_url = url::Url::from_file_path(&file_path)
            .expect("file path should convert")
            .to_string();
        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            parts: vec![PartInput::File {
                url: file_url,
                filename: Some("sample.rs".to_string()),
                mime: Some("text/plain".to_string()),
            }],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("<system-reminder>"));
        assert!(text.contains("Instructions from:"));
        assert!(text.contains("Prefer immutable updates"));
    }

    #[tokio::test]
    async fn create_user_message_file_url_dedupes_instruction_injection_per_message() {
        let prompt = SessionPrompt::default();
        let temp_dir = tempfile::tempdir().expect("tempdir should create");
        let project_root = temp_dir.path().to_path_buf();
        let src_dir = project_root.join("src");
        tokio::fs::create_dir_all(&src_dir)
            .await
            .expect("src directory should create");
        tokio::fs::write(src_dir.join("AGENTS.md"), "Shared file rules")
            .await
            .expect("instructions should write");

        let file_a = src_dir.join("a.rs");
        let file_b = src_dir.join("b.rs");
        tokio::fs::write(&file_a, "fn a() {}")
            .await
            .expect("file a should write");
        tokio::fs::write(&file_b, "fn b() {}")
            .await
            .expect("file b should write");

        let mut session = Session::new("proj", project_root.to_string_lossy().to_string());
        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            parts: vec![
                PartInput::File {
                    url: url::Url::from_file_path(&file_a)
                        .expect("file a path should convert")
                        .to_string(),
                    filename: Some("a.rs".to_string()),
                    mime: Some("text/plain".to_string()),
                },
                PartInput::File {
                    url: url::Url::from_file_path(&file_b)
                        .expect("file b path should convert")
                        .to_string(),
                    filename: Some("b.rs".to_string()),
                    mime: Some("text/plain".to_string()),
                },
            ],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(text.matches("Instructions from:").count(), 1);
    }

    #[tokio::test]
    async fn create_user_message_file_url_injects_agents_but_not_claude_for_file_scope() {
        let prompt = SessionPrompt::default();
        let temp_dir = tempfile::tempdir().expect("tempdir should create");
        let project_root = temp_dir.path().to_path_buf();
        let src_dir = project_root.join("src");
        tokio::fs::create_dir_all(&src_dir)
            .await
            .expect("src directory should create");
        tokio::fs::write(project_root.join("AGENTS.md"), "Root agents rule")
            .await
            .expect("root AGENTS should write");
        tokio::fs::write(src_dir.join("CLAUDE.md"), "src claude rule")
            .await
            .expect("src CLAUDE should write");

        let file_path = src_dir.join("sample.rs");
        tokio::fs::write(&file_path, "fn main() {}")
            .await
            .expect("file should write");

        let mut session = Session::new("proj", project_root.to_string_lossy().to_string());
        let file_url = url::Url::from_file_path(&file_path)
            .expect("file path should convert")
            .to_string();
        let input = PromptInput {
            session_id: session.id.clone(),
            message_id: None,
            model: None,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            tools: None,
            parts: vec![PartInput::File {
                url: file_url,
                filename: Some("sample.rs".to_string()),
                mime: Some("text/plain".to_string()),
            }],
        };

        prompt
            .create_user_message(&input, &mut session)
            .await
            .expect("create_user_message should succeed");

        let message = session.messages.last().expect("message should exist");
        let text = message
            .parts
            .iter()
            .filter_map(|part| match &part.part_type {
                PartType::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("Root agents rule"));
        assert!(!text.contains("src claude rule"));
    }

    #[test]
    fn shell_exec_uses_zsh_login_invocation() {
        let invocation = resolve_shell_invocation(Some("/bin/zsh"), "echo hello");
        assert_eq!(invocation.program, "/bin/zsh");
        assert_eq!(invocation.args[0], "-c");
        assert_eq!(invocation.args[1], "-l");
        assert!(invocation.args[2].contains(".zshenv"));
        assert!(invocation.args[2].contains("eval"));
    }

    #[test]
    fn shell_exec_uses_bash_login_invocation() {
        let invocation = resolve_shell_invocation(Some("/bin/bash"), "echo hello");
        assert_eq!(invocation.program, "/bin/bash");
        assert_eq!(invocation.args[0], "-c");
        assert_eq!(invocation.args[1], "-l");
        assert!(invocation.args[2].contains("shopt -s expand_aliases"));
        assert!(invocation.args[2].contains(".bashrc"));
    }

    #[test]
    fn persisted_subsessions_roundtrip_via_session_metadata() {
        let mut session = Session::new("proj", ".");
        let mut map = HashMap::new();
        map.insert(
            "task_explore_1".to_string(),
            PersistedSubsession {
                agent: "explore".to_string(),
                model: Some("anthropic:claude".to_string()),
                disabled_tools: vec!["task".to_string()],
                history: vec![PersistedSubsessionTurn {
                    prompt: "Inspect src".to_string(),
                    output: "Done".to_string(),
                }],
            },
        );

        SessionPrompt::save_persisted_subsessions(&mut session, &map);
        let loaded = SessionPrompt::load_persisted_subsessions(&session);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["task_explore_1"].agent, "explore");
        assert_eq!(loaded["task_explore_1"].history.len(), 1);
    }

    #[test]
    fn parse_model_string_supports_provider_prefix() {
        let model = SessionPrompt::parse_model_string("openai:gpt-4o");
        assert_eq!(model.provider_id, "openai");
        assert_eq!(model.model_id, "gpt-4o");
    }

    #[test]
    fn compose_subsession_prompt_includes_recent_history() {
        let history = vec![PersistedSubsessionTurn {
            prompt: "Find files".to_string(),
            output: "Found 10 files".to_string(),
        }];
        let composed = SessionPrompt::compose_subsession_prompt(&history, "Continue");
        assert!(composed.contains("Previous conversation"));
        assert!(composed.contains("Find files"));
        assert!(composed.contains("Continue"));
    }

    #[tokio::test]
    async fn resolve_tools_with_mcp_registry_merges_dynamic_tools() {
        let tool_registry = opencode_tool::create_default_registry().await;
        let mcp_registry = opencode_mcp::McpToolRegistry::new();
        mcp_registry
            .register(opencode_mcp::McpTool::new(
                "github",
                "search",
                Some("Search GitHub".to_string()),
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    }
                }),
            ))
            .await;

        let tools = resolve_tools_with_mcp_registry(&tool_registry, Some(&mcp_registry)).await;
        assert!(tools.iter().any(|t| t.name == "github_search"));
    }

    #[tokio::test]
    async fn default_tool_definitions_includes_bash_and_read() {
        let names = |defs: &[ToolDefinition]| -> Vec<String> {
            defs.iter().map(|d| d.name.clone()).collect()
        };

        let tools = default_tool_definitions().await;
        assert!(
            !tools.is_empty(),
            "default registry produced no tool definitions"
        );
        assert!(
            tools.iter().any(|t| t.name == "bash"),
            "expected a tool named 'bash', got: {:?}",
            names(&tools)
        );
        assert!(
            tools.iter().any(|t| t.name == "read"),
            "expected a tool named 'read', got: {:?}",
            names(&tools)
        );

        // Stable across calls: guards against a registry being consumed/mutated.
        let second = default_tool_definitions().await;
        assert_eq!(tools.len(), second.len());
        assert_eq!(names(&tools), names(&second));
    }

    #[test]
    fn mcp_tools_from_session_reads_runtime_metadata() {
        let mut session = Session::new("proj", ".");
        session.metadata.insert(
            "mcp_tools".to_string(),
            serde_json::json!([{
                "name": "repo_search",
                "description": "Search repository",
                "parameters": {"type":"object","properties":{"q":{"type":"string"}}}
            }]),
        );

        let tools = SessionPrompt::mcp_tools_from_session(&session);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "repo_search");
    }

    #[test]
    fn prune_after_loop_compacts_large_old_tool_results() {
        let mut session = Session::new("proj", ".");
        let session_id = session.id.clone();

        session
            .messages
            .push(SessionMessage::user(session_id.clone(), "old user message"));

        let mut old_assistant = SessionMessage::assistant(session_id.clone());
        old_assistant.add_tool_call("call_a", "bash", serde_json::json!({"command": "echo a"}));
        old_assistant.add_tool_result("call_a", "A".repeat(140_000), false);
        old_assistant.add_tool_call("call_b", "bash", serde_json::json!({"command": "echo b"}));
        old_assistant.add_tool_result("call_b", "B".repeat(140_000), false);
        session.messages.push(old_assistant);

        session
            .messages
            .push(SessionMessage::user(session_id.clone(), "new user one"));
        session
            .messages
            .push(SessionMessage::assistant(session_id.clone()));
        session
            .messages
            .push(SessionMessage::user(session_id.clone(), "new user two"));
        session.messages.push(SessionMessage::assistant(session_id));

        SessionPrompt::prune_after_loop(&mut session);

        let compacted_count = session
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter_map(|p| match &p.part_type {
                PartType::ToolResult { content, .. } => Some(content),
                _ => None,
            })
            .filter(|c| c.starts_with("[tool result compacted]"))
            .count();

        assert!(
            compacted_count >= 1,
            "expected at least one tool result to be compacted"
        );
    }

    #[test]
    fn should_compact_prefers_provider_model_limits() {
        let provider = StaticModelProvider::with_model("tiny-model", 1000, 100);
        let mut msg = SessionMessage::user("ses_test", "hello");
        msg.metadata
            .insert("tokens_input".to_string(), serde_json::json!(950_u64));

        let compact = SessionPrompt::should_compact(&[msg], &provider, "tiny-model", None);
        assert!(compact);
    }

    // ── PartInput serde round-trip tests ──

    #[test]
    fn part_input_text_round_trip() {
        let part = PartInput::Text {
            text: "hello".to_string(),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "hello");

        let back: PartInput = serde_json::from_value(json).unwrap();
        assert!(matches!(back, PartInput::Text { text } if text == "hello"));
    }

    #[test]
    fn part_input_file_round_trip() {
        let part = PartInput::File {
            url: "file:///tmp/test.rs".to_string(),
            filename: Some("test.rs".to_string()),
            mime: Some("text/plain".to_string()),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "file");
        assert_eq!(json["url"], "file:///tmp/test.rs");
        assert_eq!(json["filename"], "test.rs");

        let back: PartInput = serde_json::from_value(json).unwrap();
        assert!(matches!(back, PartInput::File { url, .. } if url == "file:///tmp/test.rs"));
    }

    #[test]
    fn part_input_agent_round_trip() {
        let part = PartInput::Agent {
            name: "explore".to_string(),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "agent");
        assert_eq!(json["name"], "explore");

        let back: PartInput = serde_json::from_value(json).unwrap();
        assert!(matches!(back, PartInput::Agent { name } if name == "explore"));
    }

    #[test]
    fn part_input_subtask_round_trip() {
        let part = PartInput::Subtask {
            prompt: "do stuff".to_string(),
            description: Some("stuff".to_string()),
            agent: "build".to_string(),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "subtask");
        assert_eq!(json["agent"], "build");

        let back: PartInput = serde_json::from_value(json).unwrap();
        assert!(matches!(back, PartInput::Subtask { agent, .. } if agent == "build"));
    }

    #[test]
    fn part_input_try_from_value() {
        let val = serde_json::json!({"type": "text", "text": "hi"});
        let part = PartInput::try_from(val).unwrap();
        assert!(matches!(part, PartInput::Text { text } if text == "hi"));
    }

    #[test]
    fn part_input_try_from_invalid_value() {
        let val = serde_json::json!({"type": "unknown", "data": 42});
        assert!(PartInput::try_from(val).is_err());
    }

    #[test]
    fn part_input_parse_array_mixed() {
        let arr = serde_json::json!([
            {"type": "text", "text": "hello"},
            {"type": "agent", "name": "explore"},
            {"type": "bogus"},
            {"type": "file", "url": "file:///x", "filename": "x", "mime": "text/plain"}
        ]);
        let parts = PartInput::parse_array(&arr);
        assert_eq!(parts.len(), 3); // bogus entry skipped
        assert!(matches!(&parts[0], PartInput::Text { text } if text == "hello"));
        assert!(matches!(&parts[1], PartInput::Agent { name } if name == "explore"));
        assert!(matches!(&parts[2], PartInput::File { url, .. } if url == "file:///x"));
    }

    #[test]
    fn part_input_parse_array_non_array() {
        let val = serde_json::json!("not an array");
        assert!(PartInput::parse_array(&val).is_empty());
    }

    #[test]
    fn part_input_file_skips_none_fields_in_json() {
        let part = PartInput::File {
            url: "file:///tmp/x".to_string(),
            filename: None,
            mime: None,
        };
        let json = serde_json::to_value(&part).unwrap();
        assert!(json.get("filename").is_none());
        assert!(json.get("mime").is_none());
    }

    // ── resolve_prompt_parts tests ──

    #[tokio::test]
    async fn resolve_prompt_parts_plain_text() {
        let parts =
            resolve_prompt_parts("just plain text", std::path::Path::new("/tmp"), &[]).await;
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], PartInput::Text { text } if text == "just plain text"));
    }

    #[tokio::test]
    async fn resolve_prompt_parts_agent_fallback() {
        // @explore doesn't exist as a file, but is a known agent
        let agents = vec!["explore".to_string(), "build".to_string()];
        let parts = resolve_prompt_parts(
            "check @explore for details",
            std::path::Path::new("/tmp"),
            &agents,
        )
        .await;
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], PartInput::Text { .. }));
        assert!(matches!(&parts[1], PartInput::Agent { name } if name == "explore"));
    }

    #[tokio::test]
    async fn resolve_prompt_parts_deduplicates() {
        let parts = resolve_prompt_parts(
            "see @explore and @explore again",
            std::path::Path::new("/tmp"),
            &["explore".to_string()],
        )
        .await;
        // text + one agent (deduplicated)
        assert_eq!(parts.len(), 2);
    }

    #[tokio::test]
    async fn resolve_prompt_parts_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        tokio::fs::write(&file, "fn main() {}").await.unwrap();

        let parts = resolve_prompt_parts("look at @test.rs", dir.path(), &[]).await;
        assert_eq!(parts.len(), 2);
        assert!(
            matches!(&parts[1], PartInput::File { mime, .. } if mime.as_deref() == Some("text/plain"))
        );
    }

    #[tokio::test]
    async fn resolve_prompt_parts_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("src");
        tokio::fs::create_dir(&sub).await.unwrap();

        let parts = resolve_prompt_parts("look at @src", dir.path(), &[]).await;
        assert_eq!(parts.len(), 2);
        assert!(
            matches!(&parts[1], PartInput::File { mime, .. } if mime.as_deref() == Some("application/x-directory"))
        );
    }

    #[test]
    fn abort_pending_tool_calls_marks_unresolved_calls_as_error() {
        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();

        // Add a user message
        session
            .messages
            .push(SessionMessage::user(sid.clone(), "do something"));

        // Add an assistant message with two tool calls but only one result
        let mut assistant = SessionMessage::assistant(sid.clone());
        assistant.add_tool_call("call_1", "bash", serde_json::json!({"command": "echo a"}));
        assistant.add_tool_call("call_2", "read_file", serde_json::json!({"path": "foo.rs"}));
        assistant.add_tool_result("call_1", "output a", false);
        // call_2 has no result — simulates abort mid-execution
        session.messages.push(assistant);

        SessionPrompt::abort_pending_tool_calls(&mut session);

        // call_2 should now have an error result
        let last_assistant = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .unwrap();

        let error_results: Vec<_> = last_assistant
            .parts
            .iter()
            .filter(|p| matches!(
                &p.part_type,
                PartType::ToolResult { tool_call_id, is_error, content, .. }
                    if tool_call_id == "call_2" && *is_error && content == "Tool execution aborted"
            ))
            .collect();

        assert_eq!(error_results.len(), 1, "call_2 should have an error result");
    }

    #[test]
    fn abort_pending_tool_calls_noop_when_all_resolved() {
        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();

        session
            .messages
            .push(SessionMessage::user(sid.clone(), "do something"));

        let mut assistant = SessionMessage::assistant(sid.clone());
        assistant.add_tool_call("call_1", "bash", serde_json::json!({"command": "echo a"}));
        assistant.add_tool_result("call_1", "output a", false);
        session.messages.push(assistant);

        let part_count_before = session.messages.last().map(|m| m.parts.len()).unwrap_or(0);

        SessionPrompt::abort_pending_tool_calls(&mut session);

        let part_count_after = session.messages.last().map(|m| m.parts.len()).unwrap_or(0);

        assert_eq!(
            part_count_before, part_count_after,
            "No new parts should be added"
        );
    }

    #[test]
    fn abort_pending_tool_calls_handles_multiple_pending() {
        let mut session = Session::new("proj", ".");
        let sid = session.id.clone();

        session
            .messages
            .push(SessionMessage::user(sid.clone(), "do something"));

        let mut assistant = SessionMessage::assistant(sid.clone());
        assistant.add_tool_call("call_1", "bash", serde_json::json!({}));
        assistant.add_tool_call("call_2", "read_file", serde_json::json!({}));
        assistant.add_tool_call("call_3", "write_file", serde_json::json!({}));
        // No results at all — all three are pending
        session.messages.push(assistant);

        SessionPrompt::abort_pending_tool_calls(&mut session);

        let last_assistant = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .unwrap();

        let abort_results: Vec<_> = last_assistant
            .parts
            .iter()
            .filter(|p| {
                matches!(
                    &p.part_type,
                    PartType::ToolResult { is_error, content, .. }
                        if *is_error && content == "Tool execution aborted"
                )
            })
            .collect();

        assert_eq!(
            abort_results.len(),
            3,
            "All three pending calls should be aborted"
        );
    }
}
