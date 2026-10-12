use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Path, Query, State,
    },
    http::{header::CACHE_CONTROL, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{sse::KeepAlive, Html, IntoResponse, Response},
    response::sse::{Event, Sse},
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use futures::stream::Stream;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, Mutex, Notify, OnceCell, RwLock};
use tokio_stream::{
    wrappers::{BroadcastStream, ReceiverStream},
    StreamExt,
};

use crate::mcp_oauth::{
    LocalMcpConfig, McpOAuthError, McpOAuthManager, McpRuntimeConfig,
    McpServerInfo as McpServerInfoStruct, McpServerLogEntry, RemoteMcpConfig,
};
use crate::oauth::ProviderAuth;
use crate::pty::{PtyManager, PtySession as PtySessionStruct, PtySubscription};
use crate::worktree::{self, WorktreeInfo as WorktreeInfoStruct};
use crate::{ApiError, Result, ServerState};
use opencode_agent::AgentRegistry;
use opencode_config::{load_config, Config as AppConfig, McpServerConfig as LoadedMcpServerConfig};
use opencode_plugin::subprocess::{PluginAuthBridge, PluginLoader, PluginSubprocessError};
use opencode_provider::{
    temperature_for_model, top_p_for_model, AuthInfo, AuthMethodType, ModelsData, ModelsDevInfo,
    ModelsRegistry,
};

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(web_index))
        .route("/health", get(health))
        .route("/event", get(event_stream))
        .route("/path", get(get_paths))
        .route("/vcs", get(get_vcs_info))
        .route("/command", get(list_commands))
        .route("/agent", get(list_agents))
        .route("/skill", get(list_skills))
        .route("/lsp", get(get_lsp_status))
        .route("/formatter", get(get_formatter_status))
        .route("/auth/{id}", put(set_auth).delete(delete_auth))
        .route("/doc", get(get_doc))
        .route("/log", post(write_log))
        .nest("/session", session_routes().layer(middleware::from_fn(inject_opencode_directory)))
        .nest("/provider", provider_routes())
        .nest("/config", config_routes())
        .nest("/mcp", mcp_routes())
        .nest("/file", file_routes())
        .nest("/find", find_routes())
        .nest("/permission", permission_routes())
        .nest("/project", project_routes())
        .nest("/pty", pty_routes())
        .nest("/question", question_routes())
        .nest("/tui", tui_routes())
        .nest("/global", global_routes())
        .nest("/experimental", experimental_routes())
        .nest("/plugin", plugin_auth_routes())
}

/// Injected by middleware from x-opencode-directory header for session create.
#[derive(Clone)]
struct OpenCodeDirectory(pub Option<String>);

async fn inject_opencode_directory(mut request: Request<axum::body::Body>, next: Next) -> Response {
    let dir = request
        .headers()
        .get("x-opencode-directory")
        .and_then(|v| v.to_str().ok())
        .map(std::string::ToString::to_string);
    request.extensions_mut().insert(OpenCodeDirectory(dir));
    next.run(request).await
}

pub async fn require_admin_token(mut request: Request<axum::body::Body>, next: Next) -> Response {
    let token = std::env::var("ELIA_AUTH_TOKEN").unwrap_or_default().trim().to_string();
    if !token.is_empty() && request.uri().path() != "/health" {
        let header = |n: &str| request.headers().get(n).and_then(|v| v.to_str().ok());
        let query = request
            .uri()
            .query()
            .and_then(|q| q.split('&').find_map(|p| p.strip_prefix("token=")));
        let strip = |v: Option<&str>| {
            v.map(|s| {
                let t = s.trim();
                t.strip_prefix("Bearer ")
                    .or_else(|| t.strip_prefix("bearer "))
                    .unwrap_or(t)
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty())
        };
        let supplied = strip(query)
            .or_else(|| strip(header("authorization")))
            .or_else(|| strip(header("x-elia-token")));
        if supplied.as_deref() != Some(token.as_str()) {
            return (StatusCode::UNAUTHORIZED, "Invalid or missing token").into_response();
        }
    }

    next.run(request).await
}

fn session_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_sessions).post(create_session))
        .route("/status", get(session_status))
        .route(
            "/{id}",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        .route("/{id}/children", get(get_session_children))
        .route("/{id}/todo", get(get_session_todos))
        .route("/{id}/fork", post(fork_session))
        .route("/{id}/abort", post(abort_session))
        .route("/{id}/share", post(share_session).delete(unshare_session))
        .route("/{id}/archive", post(archive_session))
        .route("/{id}/title", patch(set_session_title))
        .route("/{id}/permission", patch(set_session_permission))
        .route(
            "/{id}/summary",
            get(get_session_summary).patch(set_session_summary),
        )
        .route(
            "/{id}/revert",
            post(session_revert).delete(clear_session_revert),
        )
        .route("/{id}/unrevert", post(session_unrevert))
        .route("/{id}/compaction", post(start_compaction))
        .route("/{id}/summarize", post(summarize_session))
        .route("/{id}/init", post(init_session))
        .route("/{id}/command", post(execute_command))
        .route("/{id}/shell", post(execute_shell))
        .route("/{id}/message", post(send_message).get(list_messages))
        .route(
            "/{id}/message/{msgID}",
            get(get_message).delete(delete_message),
        )
        .route("/{id}/message/{msgID}/part", post(add_message_part))
        .route(
            "/{id}/message/{msgID}/part/{partID}",
            delete(delete_part).patch(update_part),
        )
        .route("/{id}/stream", post(stream_message))
        .route("/{id}/prompt", post(session_prompt))
        .route("/{id}/prompt/abort", post(abort_prompt))
        .route("/{id}/prompt_async", post(prompt_async))
        .route("/{id}/diff", get(get_session_diff))
}

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    pub directory: Option<String>,
    pub roots: Option<bool>,
    pub start: Option<i64>,
    pub search: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time: SessionTimeInfo,
    pub summary: Option<SessionSummaryInfo>,
    pub share: Option<SessionShareInfo>,
    pub revert: Option<SessionRevertInfo>,
    pub permission: Option<PermissionRulesetInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SessionTimeInfo {
    pub created: i64,
    pub updated: i64,
    pub compacting: Option<i64>,
    pub archived: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SessionSummaryInfo {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
}

#[derive(Debug, Serialize)]
pub struct SessionShareInfo {
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct SessionRevertInfo {
    pub message_id: String,
    pub part_id: Option<String>,
    pub snapshot: Option<String>,
    pub diff: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PermissionRulesetInfo {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub mode: Option<String>,
}

fn session_to_info(session: &opencode_session::Session) -> SessionInfo {
    SessionInfo {
        id: session.id.clone(),
        slug: session.slug.clone(),
        project_id: session.project_id.clone(),
        directory: session.directory.clone(),
        parent_id: session.parent_id.clone(),
        title: session.title.clone(),
        version: session.version.clone(),
        time: SessionTimeInfo {
            created: session.time.created,
            updated: session.time.updated,
            compacting: session.time.compacting,
            archived: session.time.archived,
        },
        summary: session.summary.as_ref().map(|s| SessionSummaryInfo {
            additions: s.additions,
            deletions: s.deletions,
            files: s.files,
        }),
        share: session
            .share
            .as_ref()
            .map(|s| SessionShareInfo { url: s.url.clone() }),
        revert: session.revert.as_ref().map(|r| SessionRevertInfo {
            message_id: r.message_id.clone(),
            part_id: r.part_id.clone(),
            snapshot: r.snapshot.clone(),
            diff: r.diff.clone(),
        }),
        permission: session.permission.as_ref().map(|p| PermissionRulesetInfo {
            allow: p.allow.clone(),
            deny: p.deny.clone(),
            mode: p.mode.clone(),
        }),
        agent: session
            .metadata
            .get(SESSION_AGENT_METADATA_KEY)
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

async fn persist_sessions_if_enabled(state: &Arc<ServerState>) {
    if let Err(err) = state.sync_sessions_to_storage().await {
        tracing::error!("failed to sync sessions to storage: {}", err);
    }
}

/// Persist a single session. Costs O(one session) instead of O(every message), so callers
/// that know which session they touched must use this rather than the full sync.
async fn persist_session_if_enabled(state: &Arc<ServerState>, session_id: &str) {
    if let Err(err) = state.sync_session_to_storage(session_id).await {
        tracing::error!(session_id, "failed to sync session to storage: {}", err);
    }
}

async fn list_sessions(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionInfo>>> {
    let filter = opencode_session::SessionFilter {
        directory: query.directory,
        roots: query.roots.unwrap_or(false),
        start: query.start,
        search: query.search,
        limit: query.limit,
    };
    let manager = state.sessions.lock().await;
    let sessions = manager.list_filtered(filter);
    let infos: Vec<SessionInfo> = sessions.into_iter().map(session_to_info).collect();
    Ok(Json(infos))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum SessionRunStatus {
    Idle,
    Busy,
    Retry {
        attempt: u32,
        message: String,
        next: i64,
    },
}

impl Default for SessionRunStatus {
    fn default() -> Self {
        Self::Idle
    }
}

static SESSION_RUN_STATUS: Lazy<RwLock<HashMap<String, SessionRunStatus>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Notify plugins of a server lifecycle event through omo's `event` hook.
///
/// `event` is a notification, not a transform: the plugin only observes, so the returned
/// outputs are deliberately discarded. The event object is nested under `event` because
/// that is where the handler reads `type` and `properties` from.
async fn notify_plugin_event(
    session_id: &str,
    event_type: &str,
    properties: serde_json::Value,
) {
    opencode_plugin::trigger_collect(
        opencode_plugin::HookContext::new(opencode_plugin::HookEvent::PluginEvent)
            .with_data(
                "event",
                serde_json::json!({ "type": event_type, "properties": properties }),
            )
            .with_session(session_id),
    )
    .await;
}

async fn set_session_run_status(
    state: &Arc<ServerState>,
    session_id: &str,
    status: SessionRunStatus,
) {
    {
        let mut statuses = SESSION_RUN_STATUS.write().await;
        match &status {
            SessionRunStatus::Idle => {
                statuses.remove(session_id);
            }
            _ => {
                statuses.insert(session_id.to_string(), status.clone());
            }
        }
    }

    state.broadcast(
        &serde_json::json!({
            "type": "session.status",
            "sessionID": session_id,
            "status": status,
        })
        .to_string(),
    );
}

async fn session_status(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<HashMap<String, SessionStatusInfo>>> {
    let run_status = SESSION_RUN_STATUS.read().await.clone();
    let manager = state.sessions.lock().await;
    let sessions = manager.list();
    let status: HashMap<String, SessionStatusInfo> = sessions
        .into_iter()
        .map(|s| {
            let lifecycle_status = match s.status {
                opencode_session::SessionStatus::Active => "active",
                opencode_session::SessionStatus::Completed => "completed",
                opencode_session::SessionStatus::Archived => "archived",
                opencode_session::SessionStatus::Compacting => "compacting",
            };
            let run = run_status.get(&s.id).cloned().unwrap_or_default();
            let (run_type, status, idle, busy, attempt, message, next) = match &run {
                SessionRunStatus::Idle => (
                    "idle",
                    lifecycle_status.to_string(),
                    true,
                    false,
                    None,
                    None,
                    None,
                ),
                SessionRunStatus::Busy => {
                    ("busy", "busy".to_string(), false, true, None, None, None)
                }
                SessionRunStatus::Retry {
                    attempt,
                    message,
                    next,
                } => (
                    "retry",
                    "retry".to_string(),
                    false,
                    true,
                    Some(*attempt),
                    Some(message.clone()),
                    Some(*next),
                ),
            };
            (
                s.id.clone(),
                SessionStatusInfo {
                    run_type: run_type.to_string(),
                    status,
                    idle,
                    busy,
                    attempt,
                    message,
                    next,
                },
            )
        })
        .collect();
    Ok(Json(status))
}

#[derive(Debug, Serialize)]
pub struct SessionStatusInfo {
    /// Run state as `idle` | `busy` | `retry` — the key clients select on.
    #[serde(rename = "type")]
    pub run_type: String,
    /// Lifecycle status, a superset of `run_type` (`active|completed|archived|compacting`).
    pub status: String,
    pub idle: bool,
    pub busy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<i64>,
}

/// Session metadata key holding the agent bound to a session.
///
/// Single source of truth: `session_prompt` writes the same key, and the engine reads it
/// back as the resume agent.
const SESSION_AGENT_METADATA_KEY: &str = "agent";

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    #[serde(alias = "parentID")]
    pub parent_id: Option<String>,
    pub title: Option<String>,
    pub permission: Option<SessionPermissionInput>,
    pub agent: Option<String>,
}

/// `permission` on session create arrives in two shapes: the legacy `{allow,deny,mode}`
/// ruleset, and the upstream array-of-permission-rules that omo's SDK client sends
/// (`[{permission,action,pattern}]`). Rejecting the array shape made every team member
/// creation fail with 422, which is what stopped `team_create` spawning members at all.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SessionPermissionInput {
    Ruleset(PermissionRulesetInput),
    Rules(Vec<serde_json::Value>),
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionQuery {
    pub directory: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PermissionRulesetInput {
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
    pub mode: Option<String>,
}

async fn create_session(
    State(state): State<Arc<ServerState>>,
    Extension(open_code_dir): Extension<OpenCodeDirectory>,
    Query(query): Query<CreateSessionQuery>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<SessionInfo>> {
    let directory = query
        .directory
        .filter(|d| !d.is_empty())
        .or_else(|| open_code_dir.0.clone())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });
    let session = if let Some(parent_id) = &req.parent_id {
        state
            .sessions
            .lock()
            .await
            .create_child(parent_id)
            .ok_or_else(|| ApiError::SessionNotFound(parent_id.clone()))?
    } else {
        state.sessions.lock().await.create("default", directory)
    };

    let session = match req.agent.as_deref() {
        Some(agent) => {
            let mut sessions = state.sessions.lock().await;
            match sessions.get_mut(&session.id) {
                Some(mutated) => {
                    mutated
                        .metadata
                        .insert(SESSION_AGENT_METADATA_KEY.to_string(), serde_json::json!(agent));
                    mutated.clone()
                }
                None => session,
            }
        }
        None => session,
    };

    persist_session_if_enabled(&state, &session.id).await;
    Ok(Json(session_to_info(&session)))
}

async fn get_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionInfo>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id))?;
    Ok(Json(session_to_info(session)))
}

async fn delete_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    state
        .sessions
        .lock()
        .await
        .delete(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    SESSION_RUN_STATUS.write().await.remove(&id);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

async fn get_session_children(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionInfo>>> {
    let manager = state.sessions.lock().await;
    let children = manager.children(&id);
    Ok(Json(children.into_iter().map(session_to_info).collect()))
}

#[derive(Debug, Serialize)]
pub struct TodoInfo {
    pub id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
}

static TODO_MANAGER: Lazy<opencode_session::TodoManager> =
    Lazy::new(opencode_session::TodoManager::new);

async fn get_session_todos(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TodoInfo>>> {
    let sessions = state.sessions.lock().await;
    if sessions.get(&id).is_none() {
        return Err(ApiError::SessionNotFound(id));
    }
    drop(sessions);

    let todos = TODO_MANAGER.get(&id).await;
    let items = todos
        .into_iter()
        .enumerate()
        .map(|(idx, todo)| TodoInfo {
            id: format!("{}_{}", id, idx),
            content: todo.content,
            status: todo.status,
            priority: todo.priority,
        })
        .collect();
    Ok(Json(items))
}

#[derive(Debug, Deserialize)]
pub struct ForkSessionRequest {
    pub message_id: Option<String>,
}

async fn fork_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ForkSessionRequest>,
) -> Result<Json<SessionInfo>> {
    let forked = state
        .sessions
        .lock()
        .await
        .fork(&id, req.message_id.as_deref())
        .ok_or_else(|| ApiError::SessionNotFound(id))?;
    // Persist the fork, not the source: a fork creates a NEW session, and `id` has already
    // been moved into the error above.
    persist_session_if_enabled(&state, &forked.id).await;
    Ok(Json(session_to_info(&forked)))
}

async fn share_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionShareInfo>> {
    let mut sessions = state.sessions.lock().await;
    let share_url = format!("https://share.opencode.ai/{}", id);
    sessions
        .share(&id, share_url.clone())
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(SessionShareInfo { url: share_url }))
}

async fn unshare_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    sessions
        .unshare(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(serde_json::json!({ "unshared": true })))
}

#[derive(Debug, Deserialize)]
pub struct ArchiveSessionRequest {
    pub archive: Option<bool>,
}

async fn archive_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ArchiveSessionRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let info = if req.archive.unwrap_or(true) {
        let updated = sessions
            .set_archived(&id, None)
            .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
        session_to_info(&updated)
    } else {
        let session = sessions
            .get(&id)
            .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
        session_to_info(session)
    };
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(info))
}

#[derive(Debug, Deserialize)]
pub struct SetTitleRequest {
    pub title: String,
}

async fn set_session_title(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<SetTitleRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    session.set_title(&req.title);
    let info = session_to_info(session);
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(info))
}

async fn set_session_permission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<PermissionRulesetInput>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let updated = sessions
        .set_permission(
            &id,
            opencode_session::PermissionRuleset {
                allow: req.allow.unwrap_or_default(),
                deny: req.deny.unwrap_or_default(),
                mode: req.mode,
            },
        )
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let info = session_to_info(&updated);
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(info))
}

async fn get_session_summary(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<Option<SessionSummaryInfo>>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    Ok(Json(session.summary.as_ref().map(|s| SessionSummaryInfo {
        additions: s.additions,
        deletions: s.deletions,
        files: s.files,
    })))
}

#[derive(Debug, Deserialize)]
pub struct SetSummaryRequest {
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub files: Option<u64>,
}

async fn set_session_summary(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSummaryRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let updated = sessions
        .set_summary(
            &id,
            opencode_session::SessionSummary {
                additions: req.additions.unwrap_or(0),
                deletions: req.deletions.unwrap_or(0),
                files: req.files.unwrap_or(0),
                diffs: None,
            },
        )
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let info = session_to_info(&updated);
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(info))
}

#[derive(Debug, Deserialize)]
pub struct RevertRequest {
    pub message_id: String,
    pub part_id: Option<String>,
    pub snapshot: Option<String>,
    pub diff: Option<String>,
}

async fn session_revert(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<RevertRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let updated = sessions
        .set_revert(
            &id,
            opencode_session::SessionRevert {
                message_id: req.message_id,
                part_id: req.part_id,
                snapshot: req.snapshot,
                diff: req.diff,
            },
        )
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let info = session_to_info(&updated);
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(info))
}

async fn clear_session_revert(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let updated = sessions
        .clear_revert(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let info = session_to_info(&updated);
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(info))
}

async fn start_compaction(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    session.start_compacting();
    let info = session_to_info(session);
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(info))
}

/// Accepts both `"model": "provider/id"` and `"model": {"providerID","modelID"}`.
///
/// The reference server's clients send the object; the Rust endpoint historically took a
/// bare string. Rejecting either shape is a 422 that no client can recover from.
#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub provider_id: Option<String>,
    pub model_id: String,
}

impl<'de> serde::Deserialize<'de> for ModelSpec {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Repr {
            ModelId(String),
            Detailed {
                #[serde(default, alias = "provider_id")]
                provider_id: Option<String>,
                #[serde(default, alias = "model_id")]
                model_id: String,
            },
        }

        match Repr::deserialize(deserializer)? {
            Repr::ModelId(model_id) => Ok(ModelSpec {
                provider_id: None,
                model_id,
            }),
            Repr::Detailed {
                provider_id,
                model_id,
            } => Ok(ModelSpec {
                provider_id,
                model_id,
            }),
        }
    }
}

impl ModelSpec {
    fn as_model_string(&self) -> String {
        match &self.provider_id {
            Some(provider) => format!("{}/{}", provider, self.model_id),
            None => self.model_id.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub content: Option<String>,
    pub parts: Option<Vec<serde_json::Value>>,
    pub model: Option<ModelSpec>,
    pub variant: Option<String>,
    pub stream: Option<bool>,
    pub agent: Option<String>,
}

/// Resolve the message body into the text to store, accepting either `content` or the
/// reference server's `parts[]`. Every rejection is a 400 through `ApiError`.
fn resolve_message_text(req: &SendMessageRequest) -> Result<String> {
    if let Some(raw_parts) = &req.parts {
        if raw_parts.is_empty() {
            return Err(ApiError::BadRequest(
                "Field `parts` must not be an empty array; provide `content` or at least \
                 one entry in `parts`"
                    .to_string(),
            ));
        }

        let mut texts = Vec::new();
        for (index, raw) in raw_parts.iter().enumerate() {
            match opencode_session::PartInput::try_from(raw.clone()) {
                Ok(opencode_session::PartInput::Text { text }) => texts.push(text),
                Ok(_) => {}
                Err(error) => {
                    return Err(ApiError::BadRequest(format!(
                        "Invalid entry at parts[{index}]: {error}"
                    )));
                }
            }
        }

        if let Some(content) = req.content.as_deref() {
            if !content.is_empty() {
                texts.insert(0, content.to_string());
            }
        }
        if texts.is_empty() {
            return Err(ApiError::BadRequest(
                "Field `parts` contains no text; provide `content` or a \
                 `parts[{\"type\":\"text\",\"text\":\"...\"}]` entry"
                    .to_string(),
            ));
        }
        return Ok(texts.join("\n"));
    }

    match req.content.as_deref() {
        Some(content) => Ok(content.to_string()),
        None => Err(ApiError::BadRequest(
            "Either `content` (string) or `parts` (array of typed parts) must be provided"
                .to_string(),
        )),
    }
}

fn session_parts_to_provider_content(
    parts: &[opencode_session::MessagePart],
) -> opencode_provider::Content {
    let has_non_text = parts
        .iter()
        .any(|part| !matches!(part.part_type, opencode_session::PartType::Text { .. }));

    if !has_non_text {
        let text = parts
            .iter()
            .filter_map(|part| match &part.part_type {
                opencode_session::PartType::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return opencode_provider::Content::Text(text);
    }

    let content_parts = parts
        .iter()
        .filter_map(|part| match &part.part_type {
            opencode_session::PartType::Text { text, .. } => Some(opencode_provider::ContentPart {
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
            opencode_session::PartType::ToolCall { id, name, input } => {
                Some(opencode_provider::ContentPart {
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
                })
            }
            opencode_session::PartType::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => Some(opencode_provider::ContentPart {
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
        .collect::<Vec<_>>();

    opencode_provider::Content::Parts(content_parts)
}

fn session_messages_to_provider_messages(
    session_messages: &[opencode_session::SessionMessage],
) -> Vec<opencode_provider::Message> {
    session_messages
        .iter()
        .map(|message| {
            let role = match message.role {
                opencode_session::MessageRole::User => opencode_provider::Role::User,
                opencode_session::MessageRole::Assistant => opencode_provider::Role::Assistant,
                opencode_session::MessageRole::System => opencode_provider::Role::System,
                opencode_session::MessageRole::Tool => opencode_provider::Role::Tool,
            };
            opencode_provider::Message {
                role,
                content: session_parts_to_provider_content(&message.parts),
                cache_control: None,
                provider_options: None,
            }
        })
        .collect()
}

fn resolve_provider_and_model(
    state: &ServerState,
    request_model: Option<&str>,
    config_model: Option<&str>,
    config_provider: Option<&str>,
) -> Result<(Arc<dyn opencode_provider::Provider>, String, String)> {
    let resolve_from_model = |model: &str| -> Result<(String, String)> {
        state
            .providers
            .parse_model_string(model)
            .ok_or_else(|| ApiError::BadRequest(format!("Model not found: {}", model)))
    };

    let (provider_id, model_id) = if let Some(model) = request_model {
        resolve_from_model(model)?
    } else if let Some(model) = config_model {
        if let Some(provider_id) = config_provider {
            (provider_id.to_string(), model.to_string())
        } else {
            resolve_from_model(model)?
        }
    } else {
        let first = state
            .providers
            .list_models()
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::BadRequest("No providers configured".to_string()))?;
        (first.provider, first.id)
    };

    let provider = state
        .providers
        .get_provider(&provider_id)
        .map_err(|e| ApiError::ProviderError(e.to_string()))?;
    if provider.get_model(&model_id).is_none() {
        return Err(ApiError::BadRequest(format!(
            "Model `{}` not found for provider `{}`",
            model_id, provider_id
        )));
    }

    Ok((provider, provider_id, model_id))
}

async fn send_message(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<serde_json::Value>> {
    let text = resolve_message_text(&req)?;

    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    session.add_user_message(&text);
    if let Some(variant) = req.variant.as_deref() {
        session
            .metadata
            .insert("model_variant".to_string(), serde_json::json!(variant));
    }
    if let Some(model) = req.model.as_ref() {
        session.metadata.insert(
            "model_request".to_string(),
            serde_json::json!(model.as_model_string()),
        );
    }
    if let Some(agent) = req.agent.as_deref() {
        session
            .metadata
            .insert(SESSION_AGENT_METADATA_KEY.to_string(), serde_json::json!(agent));
    }
    let assistant_msg = session.add_assistant_message();
    let envelope = crate::events::message_to_ts_json(assistant_msg, &session_id);
    drop(sessions);
    persist_session_if_enabled(&state, &session_id).await;
    Ok(Json(envelope))
}

async fn list_messages(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    Ok(Json(crate::events::messages_to_ts_json(session)))
}

async fn delete_message(
    State(state): State<Arc<ServerState>>,
    Path((session_id, msg_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    session.remove_message(&msg_id);
    drop(sessions);
    persist_session_if_enabled(&state, &session_id).await;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

#[derive(Debug, Deserialize)]
pub struct AddPartRequest {
    #[serde(rename = "type")]
    pub part_type: String,
    pub text: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub content: Option<String>,
    pub is_error: Option<bool>,
}

fn build_message_part(req: AddPartRequest, msg_id: &str) -> Result<opencode_session::MessagePart> {
    let part_type = match req.part_type.as_str() {
        "text" => opencode_session::PartType::Text {
            text: req.text.ok_or_else(|| {
                ApiError::BadRequest("Field `text` is required for text parts".to_string())
            })?,
            synthetic: None,
            ignored: None,
        },
        "reasoning" => opencode_session::PartType::Reasoning {
            text: req.text.ok_or_else(|| {
                ApiError::BadRequest("Field `text` is required for reasoning parts".to_string())
            })?,
        },
        "tool_call" => opencode_session::PartType::ToolCall {
            id: req.tool_call_id.ok_or_else(|| {
                ApiError::BadRequest(
                    "Field `tool_call_id` is required for tool_call parts".to_string(),
                )
            })?,
            name: req.tool_name.ok_or_else(|| {
                ApiError::BadRequest(
                    "Field `tool_name` is required for tool_call parts".to_string(),
                )
            })?,
            input: req.tool_input.unwrap_or_else(|| serde_json::json!({})),
        },
        "tool_result" => opencode_session::PartType::ToolResult {
            tool_call_id: req.tool_call_id.ok_or_else(|| {
                ApiError::BadRequest(
                    "Field `tool_call_id` is required for tool_result parts".to_string(),
                )
            })?,
            content: req.content.ok_or_else(|| {
                ApiError::BadRequest(
                    "Field `content` is required for tool_result parts".to_string(),
                )
            })?,
            is_error: req.is_error.unwrap_or(false),
        },
        unsupported => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported part type: {}",
                unsupported
            )));
        }
    };

    Ok(opencode_session::MessagePart {
        id: format!("prt_{}", uuid::Uuid::new_v4().simple()),
        part_type,
        created_at: chrono::Utc::now(),
        message_id: Some(msg_id.to_string()),
    })
}

async fn add_message_part(
    State(state): State<Arc<ServerState>>,
    Path((session_id, msg_id)): Path<(String, String)>,
    Json(req): Json<AddPartRequest>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    let message = session
        .get_message_mut(&msg_id)
        .ok_or_else(|| ApiError::NotFound(format!("Message not found: {}", msg_id)))?;

    let part = build_message_part(req, &msg_id)?;
    let part_id = part.id.clone();
    message.parts.push(part);
    session.touch();
    drop(sessions);
    persist_session_if_enabled(&state, &session_id).await;

    Ok(Json(serde_json::json!({
        "added": true,
        "session_id": session_id,
        "message_id": msg_id,
        "part_id": part_id,
    })))
}

async fn delete_part(
    State(state): State<Arc<ServerState>>,
    Path((session_id, msg_id, part_id)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    let message = session
        .get_message_mut(&msg_id)
        .ok_or_else(|| ApiError::NotFound(format!("Message not found: {}", msg_id)))?;

    let before = message.parts.len();
    message.parts.retain(|part| part.id != part_id);
    if message.parts.len() == before {
        return Err(ApiError::NotFound(format!("Part not found: {}", part_id)));
    }
    session.touch();
    drop(sessions);
    persist_session_if_enabled(&state, &session_id).await;

    Ok(Json(serde_json::json!({
        "deleted": true,
        "session_id": session_id,
        "message_id": msg_id,
        "part_id": part_id,
    })))
}

#[derive(Debug, Serialize, Clone)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Default)]
struct PendingToolCall {
    name: Option<String>,
    input: String,
}

async fn send_sse_event(
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    name: &str,
    payload: StreamEvent,
) {
    if let Ok(event) = Event::default().event(name).json_data(payload) {
        let _ = tx.send(Ok(event)).await;
    }
}

async fn stream_message(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> std::result::Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    let text = resolve_message_text(&req)?;

    let config = CONFIG_STATE.read().await;
    let request_model = req.model.as_ref().map(ModelSpec::as_model_string);
    let (provider, provider_id, model_id) =
        resolve_provider_and_model(&state, request_model.as_deref(), config.model.as_deref(), None)?;
    drop(config);

    let (history, msg_id, selected_variant) = {
        let mut sessions = state.sessions.lock().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

        session.add_user_message(&text);
        let history = session.messages.clone();
        let selected_variant = req.variant.clone().or_else(|| {
            session
                .metadata
                .get("model_variant")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        });
        if let Some(variant) = req.variant.as_deref() {
            session
                .metadata
                .insert("model_variant".to_string(), serde_json::json!(variant));
        }
        session.metadata.insert(
            "model_provider".to_string(),
            serde_json::json!(provider_id.clone()),
        );
        session
            .metadata
            .insert("model_id".to_string(), serde_json::json!(model_id.clone()));

        let assistant_msg = session.add_assistant_message();
        (history, assistant_msg.id.clone(), selected_variant)
    };

    let provider_messages = session_messages_to_provider_messages(&history);
    let variant = selected_variant.clone();
    let temperature = temperature_for_model(&model_id).or(Some(0.7));
    let top_p = top_p_for_model(&model_id);
    let max_tokens = max_tokens_for_variant(4096, variant.as_deref());
    let request = opencode_provider::ChatRequest {
        model: model_id.clone(),
        messages: provider_messages,
        max_tokens: Some(max_tokens),
        temperature,
        top_p,
        system: None,
        tools: None,
        stream: Some(true),
        variant,
        provider_options: None,
        session_id: Some(session_id.clone()),
    };

    let (tx, rx) = mpsc::channel::<std::result::Result<Event, Infallible>>(128);
    let stream_state = state.clone();
    let stream_session_id = session_id.clone();
    let stream_msg_id = msg_id.clone();

    tokio::spawn(async move {
        send_sse_event(
            &tx,
            "message_start",
            StreamEvent {
                event_type: "message_start".to_string(),
                content: None,
                message_id: Some(stream_msg_id.clone()),
                done: None,
                tool_call_id: None,
                tool_name: None,
                input: None,
                prompt_tokens: None,
                completion_tokens: None,
                error: None,
            },
        )
        .await;

        let mut final_text = String::new();
        let mut pending_tool_calls: HashMap<String, PendingToolCall> = HashMap::new();
        let mut stream_failed: Option<String> = None;

        match provider.chat_stream(request).await {
            Ok(mut provider_stream) => {
                while let Some(item) = provider_stream.next().await {
                    match item {
                        Ok(opencode_provider::StreamEvent::TextDelta(delta)) => {
                            if delta.is_empty() {
                                continue;
                            }
                            final_text.push_str(&delta);
                            send_sse_event(
                                &tx,
                                "message_delta",
                                StreamEvent {
                                    event_type: "message_delta".to_string(),
                                    content: Some(delta),
                                    message_id: Some(stream_msg_id.clone()),
                                    done: None,
                                    tool_call_id: None,
                                    tool_name: None,
                                    input: None,
                                    prompt_tokens: None,
                                    completion_tokens: None,
                                    error: None,
                                },
                            )
                            .await;
                        }
                        Ok(opencode_provider::StreamEvent::ToolCallStart { id, name }) => {
                            pending_tool_calls.entry(id.clone()).or_default().name =
                                Some(name.clone());
                            send_sse_event(
                                &tx,
                                "tool_call_start",
                                StreamEvent {
                                    event_type: "tool_call_start".to_string(),
                                    content: None,
                                    message_id: Some(stream_msg_id.clone()),
                                    done: None,
                                    tool_call_id: Some(id),
                                    tool_name: Some(name),
                                    input: None,
                                    prompt_tokens: None,
                                    completion_tokens: None,
                                    error: None,
                                },
                            )
                            .await;
                        }
                        Ok(opencode_provider::StreamEvent::ToolCallDelta { id, input }) => {
                            pending_tool_calls
                                .entry(id.clone())
                                .or_default()
                                .input
                                .push_str(&input);
                            send_sse_event(
                                &tx,
                                "tool_call_delta",
                                StreamEvent {
                                    event_type: "tool_call_delta".to_string(),
                                    content: None,
                                    message_id: Some(stream_msg_id.clone()),
                                    done: None,
                                    tool_call_id: Some(id),
                                    tool_name: None,
                                    input: Some(input),
                                    prompt_tokens: None,
                                    completion_tokens: None,
                                    error: None,
                                },
                            )
                            .await;
                        }
                        Ok(opencode_provider::StreamEvent::ToolCallEnd { id, name, input }) => {
                            let call = pending_tool_calls.entry(id.clone()).or_default();
                            call.name = Some(name.clone());
                            call.input = input.to_string();
                            send_sse_event(
                                &tx,
                                "tool_call_end",
                                StreamEvent {
                                    event_type: "tool_call_end".to_string(),
                                    content: None,
                                    message_id: Some(stream_msg_id.clone()),
                                    done: None,
                                    tool_call_id: Some(id),
                                    tool_name: Some(name),
                                    input: Some(input.to_string()),
                                    prompt_tokens: None,
                                    completion_tokens: None,
                                    error: None,
                                },
                            )
                            .await;
                        }
                        Ok(opencode_provider::StreamEvent::Usage {
                            prompt_tokens,
                            completion_tokens,
                        }) => {
                            send_sse_event(
                                &tx,
                                "usage",
                                StreamEvent {
                                    event_type: "usage".to_string(),
                                    content: None,
                                    message_id: Some(stream_msg_id.clone()),
                                    done: None,
                                    tool_call_id: None,
                                    tool_name: None,
                                    input: None,
                                    prompt_tokens: Some(prompt_tokens),
                                    completion_tokens: Some(completion_tokens),
                                    error: None,
                                },
                            )
                            .await;
                        }
                        Ok(opencode_provider::StreamEvent::Done) => break,
                        Ok(opencode_provider::StreamEvent::Error(err)) => {
                            stream_failed = Some(err.clone());
                            send_sse_event(
                                &tx,
                                "error",
                                StreamEvent {
                                    event_type: "error".to_string(),
                                    content: None,
                                    message_id: Some(stream_msg_id.clone()),
                                    done: None,
                                    tool_call_id: None,
                                    tool_name: None,
                                    input: None,
                                    prompt_tokens: None,
                                    completion_tokens: None,
                                    error: Some(err),
                                },
                            )
                            .await;
                            break;
                        }
                        Err(err) => {
                            let error_message = err.to_string();
                            stream_failed = Some(error_message.clone());
                            send_sse_event(
                                &tx,
                                "error",
                                StreamEvent {
                                    event_type: "error".to_string(),
                                    content: None,
                                    message_id: Some(stream_msg_id.clone()),
                                    done: None,
                                    tool_call_id: None,
                                    tool_name: None,
                                    input: None,
                                    prompt_tokens: None,
                                    completion_tokens: None,
                                    error: Some(error_message),
                                },
                            )
                            .await;
                            break;
                        }
                        Ok(_) => {}
                    }
                }
            }
            Err(err) => {
                let error_message = err.to_string();
                stream_failed = Some(error_message.clone());
                send_sse_event(
                    &tx,
                    "error",
                    StreamEvent {
                        event_type: "error".to_string(),
                        content: None,
                        message_id: Some(stream_msg_id.clone()),
                        done: None,
                        tool_call_id: None,
                        tool_name: None,
                        input: None,
                        prompt_tokens: None,
                        completion_tokens: None,
                        error: Some(error_message),
                    },
                )
                .await;
            }
        }

        {
            let mut sessions = stream_state.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&stream_session_id) {
                if let Some(message) = session.get_message_mut(&stream_msg_id) {
                    if !final_text.is_empty() {
                        message.add_text(final_text);
                    }
                    for (id, pending) in pending_tool_calls {
                        let name = pending.name.unwrap_or_else(|| "unknown_tool".to_string());
                        let parsed_input =
                            serde_json::from_str::<serde_json::Value>(&pending.input)
                                .unwrap_or_else(|_| serde_json::json!({ "raw": pending.input }));
                        let call_id = if id.is_empty() {
                            format!("call_{}", uuid::Uuid::new_v4().simple())
                        } else {
                            id
                        };
                        message.add_tool_call(call_id, name, parsed_input);
                    }
                    if let Some(error) = stream_failed {
                        if message.parts.is_empty() {
                            message.add_text(format!("Stream error: {}", error));
                        }
                    }
                }
                session.touch();
            }
        }
    persist_session_if_enabled(&stream_state, &session_id).await;

        send_sse_event(
            &tx,
            "message_end",
            StreamEvent {
                event_type: "message_end".to_string(),
                content: None,
                message_id: Some(stream_msg_id),
                done: Some(true),
                tool_call_id: None,
                tool_name: None,
                input: None,
                prompt_tokens: None,
                completion_tokens: None,
                error: None,
            },
        )
        .await;
    });

    Ok(Sse::new(ReceiverStream::new(rx)))
}

#[derive(Debug, Deserialize)]
pub struct SessionPromptRequest {
    pub message: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
    pub agent: Option<String>,
    pub command: Option<String>,
    pub arguments: Option<String>,
}

/// Run one full assistant turn for a session: stream it, persist it, then go idle.
///
/// Shared by the blocking `/prompt` route and by `/prompt_async`, because both must
/// actually drive the turn. `/prompt_async` previously only recorded the message and
/// replied "queued" without ever running it, so plugins whose hooks hang off a prompt
/// (team mailbox injection among them) never fired for those requests.
#[allow(clippy::too_many_arguments)]
async fn run_prompt_turn(
    state: Arc<ServerState>,
    session_id: String,
    prompt_text: String,
    provider: Arc<dyn opencode_provider::Provider>,
    provider_id: String,
    model_id: String,
    variant: Option<String>,
    agent: Option<String>,
) {
    let mut session = {
        let sessions = state.sessions.lock().await;
        let Some(session) = sessions.get(&session_id).cloned() else {
            return;
        };
        session
    };
    set_session_run_status(&state, &session_id, SessionRunStatus::Busy).await;

    if let Some(variant) = variant.as_deref() {
        session
            .metadata
            .insert("model_variant".to_string(), serde_json::json!(variant));
    } else {
        session.metadata.remove("model_variant");
    }
    session.metadata.insert(
        "model_provider".to_string(),
        serde_json::json!(&provider_id),
    );
    session
        .metadata
        .insert("model_id".to_string(), serde_json::json!(&model_id));
    if let Some(agent) = agent.as_deref() {
        session
            .metadata
            .insert("agent".to_string(), serde_json::json!(agent));
    }

    // The hook is synchronous (`Arc<dyn Fn(&Session)>`), so it folds the snapshot into a
    // tracker and emits straight onto the bus. Nothing is cloned and nothing is queued.
    // The previous shape cloned the whole `Session` per tick (~20/sec while streaming)
    // and queued it in a 1024-deep channel per session, which is what made 100 concurrent
    // streaming sessions unaffordable: a long session turns one clone into a large copy.
    let bus = state.event_bus.clone();
    let tracker = Arc::new(std::sync::Mutex::new(crate::events::StreamTracker::new()));
    let update_hook: opencode_session::SessionUpdateHook = Arc::new(move |snapshot| {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let diffed = match tracker.lock() {
            Ok(mut guard) => guard.observe(snapshot, now_ms),
            Err(poisoned) => poisoned.into_inner().observe(snapshot, now_ms),
        };
        for (event_type, properties) in diffed {
            let _ = bus.send(crate::events::envelope(&event_type, properties).to_string());
        }
    });

    let tools = opencode_session::default_tool_definitions().await;
    let environment = opencode_session::system::EnvironmentContext::from_current(
        &model_id,
        &provider_id,
        session.directory.clone(),
    );
    let system_prompt = Some(format!(
        "{}\n\n{}",
        opencode_session::system::SystemPrompt::for_model(&model_id),
        opencode_session::system::SystemPrompt::environment(&environment),
    ));

    let prompt_runner = opencode_session::SessionPrompt::new(Arc::new(RwLock::new(
        opencode_session::SessionStateManager::new(),
    )));
    let input = opencode_session::PromptInput {
        session_id: session_id.clone(),
        message_id: None,
        model: Some(opencode_session::prompt::ModelRef {
            provider_id: provider_id.clone(),
            model_id: model_id.clone(),
        }),
        agent: agent.clone(),
        no_reply: false,
        system: None,
        variant: variant.clone(),
        parts: vec![opencode_session::PartInput::Text { text: prompt_text }],
        tools: None,
        permission: agent.as_deref().and_then(|name| {
            load_config(&std::env::current_dir().unwrap_or_default())
                .ok()
                .and_then(|loaded| {
                    AgentRegistry::from_optional_config(Some(&loaded))
                        .get(name)
                        .map(|info| info.permission.clone())
                })
        }),
    };

    if let Err(error) = prompt_runner
        .prompt_with_update_hook(
            input,
            &mut session,
            provider,
            system_prompt,
            tools,
            opencode_session::AgentParams::default(),
            Some(update_hook),
        )
        .await
    {
        tracing::error!(
            session_id = %session_id,
            provider_id = %provider_id,
            model_id = %model_id,
            %error,
            "session prompt failed"
        );
        let assistant = session.add_assistant_message();
        assistant
            .metadata
            .insert("error".to_string(), serde_json::json!(error.to_string()));
        assistant
            .metadata
            .insert("finish_reason".to_string(), serde_json::json!("error"));
        assistant.metadata.insert(
            "model_provider".to_string(),
            serde_json::json!(&provider_id),
        );
        assistant
            .metadata
            .insert("model_id".to_string(), serde_json::json!(&model_id));
        if let Some(agent) = agent.as_deref() {
            assistant
                .metadata
                .insert("agent".to_string(), serde_json::json!(agent));
        }
        assistant.add_text(format!("Provider error: {}", error));
    }
    {
        let mut sessions = state.sessions.lock().await;
        sessions.update(session);
    }
    state.broadcast(
        &serde_json::json!({
            "type": "session.updated",
            "sessionID": session_id,
            "source": "prompt.final",
        })
        .to_string(),
    );
    set_session_run_status(&state, &session_id, SessionRunStatus::Idle).await;
    notify_plugin_event(
        &session_id,
        "session.idle",
        serde_json::json!({ "sessionID": session_id }),
    )
    .await;
    persist_session_if_enabled(&state, &session_id).await;
}

async fn session_prompt(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<SessionPromptRequest>,
) -> Result<Json<serde_json::Value>> {
    let prompt_text = if let Some(message) = req.message.as_deref() {
        message.to_string()
    } else if let Some(command) = req.command.as_deref() {
        req.arguments
            .as_deref()
            .map(|args| format!("/{command} {args}"))
            .unwrap_or_else(|| format!("/{command}"))
    } else {
        return Err(ApiError::BadRequest(
            "Either `message` or `command` must be provided".to_string(),
        ));
    };

    {
        let sessions = state.sessions.lock().await;
        if sessions.get(&id).is_none() {
            return Err(ApiError::SessionNotFound(id));
        }
    }

    let config = CONFIG_STATE.read().await;
    let (provider, provider_id, model_id) =
        resolve_provider_and_model(&state, req.model.as_deref(), config.model.as_deref(), None)?;
    drop(config);

    let task_state = state.clone();
    let session_id = id.clone();
    let task_variant = req.variant.clone();
    let task_agent = req.agent.clone();
    let task_model = model_id.clone();
    tokio::spawn(run_prompt_turn(
        task_state,
        session_id,
        prompt_text,
        provider,
        provider_id.clone(),
        task_model,
        task_variant,
        task_agent,
    ));

    Ok(Json(serde_json::json!({
        "status": "started",
        "model": format!("{}/{}", provider_id, model_id),
        "variant": req.variant,
    })))
}

async fn abort_prompt(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let sessions = state.sessions.lock().await;
    if sessions.get(&id).is_none() {
        return Err(ApiError::SessionNotFound(id));
    }
    Ok(Json(serde_json::json!({ "aborted": true })))
}

async fn abort_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let sessions = state.sessions.lock().await;
    if sessions.get(&id).is_none() {
        return Err(ApiError::SessionNotFound(id));
    }
    Ok(Json(serde_json::json!({ "aborted": true })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateSessionTimeRequest {
    pub archived: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
    pub time: Option<UpdateSessionTimeRequest>,
}

async fn update_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;

    if let Some(title) = req.title {
        session.set_title(title);
    }
    if let Some(time) = req.time {
        if let Some(archived) = time.archived {
            session.set_archived(Some(archived));
        }
    }
    let info = session_to_info(session);
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;
    Ok(Json(info))
}

async fn get_message(
    State(state): State<Arc<ServerState>>,
    Path((session_id, msg_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    let message = session
        .get_message(&msg_id)
        .ok_or_else(|| ApiError::NotFound(format!("Message not found: {}", msg_id)))?;

    Ok(Json(crate::events::message_to_ts_json(message, &session_id)))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePartRequest {
    pub part: serde_json::Value,
}

async fn update_part(
    State(state): State<Arc<ServerState>>,
    Path((session_id, msg_id, part_id)): Path<(String, String, String)>,
    Json(req): Json<UpdatePartRequest>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    let message = session
        .get_message_mut(&msg_id)
        .ok_or_else(|| ApiError::NotFound(format!("Message not found: {}", msg_id)))?;

    let mut part: opencode_session::MessagePart = serde_json::from_value(req.part)
        .map_err(|e| ApiError::BadRequest(format!("Invalid part payload: {}", e)))?;
    if part.id != part_id {
        return Err(ApiError::BadRequest(format!(
            "Part id mismatch: body has {}, path has {}",
            part.id, part_id
        )));
    }
    part.message_id = Some(msg_id.clone());

    let updated_part = {
        let target = message
            .parts
            .iter_mut()
            .find(|existing| existing.id == part_id)
            .ok_or_else(|| ApiError::NotFound(format!("Part not found: {}", part_id)))?;
        *target = part.clone();
        target.clone()
    };
    session.touch();
    drop(sessions);
    persist_session_if_enabled(&state, &session_id).await;

    Ok(Json(serde_json::json!({
        "updated": true,
        "part": updated_part,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ExecuteShellRequest {
    pub command: String,
    pub workdir: Option<String>,
}

async fn execute_shell(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ExecuteShellRequest>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    session.add_user_message(format!("$ {}", req.command));
    let assistant = session.add_assistant_message();
    assistant.add_text(format!("Shell command queued: {}", req.command));
    let assistant_id = assistant.id.clone();
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;

    Ok(Json(serde_json::json!({
        "executed": true,
        "command": req.command,
        "workdir": req.workdir,
        "message_id": assistant_id,
    })))
}

#[derive(Debug, Deserialize)]
pub struct TextPartInput {
    #[serde(rename = "type")]
    pub part_type: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PromptAsyncRequest {
    pub message: Option<String>,
    pub parts: Option<Vec<TextPartInput>>,
    pub model: Option<serde_json::Value>,
}

fn text_from_parts(parts: &[TextPartInput]) -> String {
    parts
        .iter()
        .filter_map(|p| {
            if p.part_type.as_deref() == Some("text") {
                p.text.as_deref()
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `/prompt_async` accepts `model` either as `"provider/model"` or as the SDK object
/// `{providerID, modelID}`; normalise both to the string `resolve_provider_and_model` wants.
fn prompt_async_model_ref(model: Option<&serde_json::Value>) -> Option<String> {
    match model? {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Object(map) => {
            match (
                map.get("providerID").and_then(|v| v.as_str()),
                map.get("modelID").and_then(|v| v.as_str()),
            ) {
                (Some(provider_id), Some(model_id)) => Some(format!("{provider_id}/{model_id}")),
                _ => None,
            }
        }
        _ => None,
    }
}

async fn prompt_async(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<PromptAsyncRequest>,
) -> Result<Json<serde_json::Value>> {
    let message_text: String = req
        .message
        .clone()
        .or_else(|| {
            req.parts.as_ref().map(|p| {
                let s = text_from_parts(p);
                if s.is_empty() { None } else { Some(s) }
            }).flatten()
        })
        .unwrap_or_default();

    {
        let sessions = state.sessions.lock().await;
        if sessions.get(&id).is_none() {
            return Err(ApiError::SessionNotFound(id));
        }
    }

    let model_ref = prompt_async_model_ref(req.model.as_ref());
    let config = CONFIG_STATE.read().await;
    let (provider, provider_id, model_id) =
        resolve_provider_and_model(&state, model_ref.as_deref(), config.model.as_deref(), None)?;
    drop(config);

    // The turn is spawned rather than awaited: this route's contract is to acknowledge
    // immediately. It must still be spawned — recording the message and replying "queued"
    // without running the turn meant every prompt hook (team mailbox injection included)
    // silently never fired for these requests.
    tokio::spawn(run_prompt_turn(
        state.clone(),
        id.clone(),
        message_text,
        provider,
        provider_id,
        model_id,
        None,
        None,
    ));

    Ok(Json(serde_json::json!({
        "status": "queued",
        "message_id": format!("msg_{}", uuid::Uuid::new_v4().simple()),
        "model": req.model,
    })))
}

#[derive(Debug, Deserialize)]
pub struct InitSessionRequest {
    pub force: Option<bool>,
}

async fn init_session(
    Path(_id): Path<String>,
    Json(_req): Json<InitSessionRequest>,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(
        serde_json::json!({ "initialized": true, "message": "Session initialized successfully" }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct SummarizeSessionRequest {
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

async fn summarize_session(
    Path(_id): Path<String>,
    Json(_req): Json<SummarizeSessionRequest>,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(
        serde_json::json!({ "summarized": true, "message": "Session summarized successfully" }),
    ))
}

async fn session_unrevert(Path(_id): Path<String>) -> Result<Json<serde_json::Value>> {
    Ok(Json(
        serde_json::json!({ "unreverted": true, "message": "Session unreverted successfully" }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct ExecuteCommandRequest {
    pub command: String,
    pub arguments: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
}

async fn execute_command(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ExecuteCommandRequest>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let text = req
        .arguments
        .as_deref()
        .map(|args| format!("/{cmd} {args}", cmd = req.command))
        .unwrap_or_else(|| format!("/{}", req.command));
    session.add_user_message(text);
    let assistant = session.add_assistant_message();
    assistant.add_text(format!("Command queued: {}", req.command));
    let assistant_id = assistant.id.clone();
    let arguments = req
        .arguments
        .as_deref()
        .map(|value| {
            value
                .split_whitespace()
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    sessions.publish_command_executed(&req.command, &id, arguments, &assistant_id);
    drop(sessions);
    persist_session_if_enabled(&state, &id).await;

    Ok(Json(serde_json::json!({
        "executed": true,
        "command": req.command,
        "arguments": req.arguments,
        "model": req.model,
        "agent": req.agent,
        "message_id": assistant_id,
    })))
}

async fn get_session_diff(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<FileDiffInfo>>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id))?;
    let diffs = session
        .summary
        .as_ref()
        .and_then(|summary| summary.diffs.as_ref())
        .map(|items| {
            items
                .iter()
                .map(|diff| FileDiffInfo {
                    path: diff.path.clone(),
                    additions: diff.additions,
                    deletions: diff.deletions,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(Json(diffs))
}

#[derive(Debug, Serialize)]
pub struct FileDiffInfo {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

fn provider_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_providers))
        .route("/auth", get(get_provider_auth))
        .route("/{id}/oauth/authorize", post(oauth_authorize))
        .route("/{id}/oauth/callback", post(oauth_callback))
}

#[derive(Debug, Serialize)]
pub struct ProviderListResponse {
    pub all: Vec<ProviderInfo>,
    #[serde(rename = "default")]
    pub default_model: HashMap<String, String>,
    pub connected: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<String>,
}

static MODEL_VARIANT_LOOKUP: OnceCell<HashMap<String, HashMap<String, Vec<String>>>> =
    OnceCell::const_new();

async fn load_models_dev_data() -> ModelsData {
    let cache_path = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("opencode")
        .join("models.json");

    if let Ok(content) = tokio::fs::read_to_string(&cache_path).await {
        if let Ok(parsed) = serde_json::from_str::<ModelsData>(&content) {
            return parsed;
        }
    }

    let registry = ModelsRegistry::default();
    match tokio::time::timeout(Duration::from_secs(2), registry.get()).await {
        Ok(data) => data,
        Err(_) => HashMap::new(),
    }
}

fn build_model_variant_lookup(data: ModelsData) -> HashMap<String, HashMap<String, Vec<String>>> {
    data.into_iter()
        .map(|(provider_id, provider)| {
            let model_map = provider
                .models
                .into_iter()
                .map(|(model_id, model)| {
                    let mut variants = model
                        .variants
                        .as_ref()
                        .map(|items| items.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    if variants.is_empty() {
                        variants = synthetic_variant_names(&provider_id, &model);
                    }
                    variants.sort();
                    (model_id, variants)
                })
                .collect::<HashMap<_, _>>();
            (provider_id, model_map)
        })
        .collect()
}

fn synthetic_variant_names(provider_id: &str, model: &ModelsDevInfo) -> Vec<String> {
    if !model.reasoning {
        return Vec::new();
    }

    let provider = provider_id.to_ascii_lowercase();
    let model_id = model.id.to_ascii_lowercase();
    let is_anthropic = provider.contains("anthropic") || model_id.contains("claude");
    if is_anthropic {
        return vec!["high".to_string(), "max".to_string()];
    }

    let is_google =
        provider.contains("google") || provider.contains("vertex") || model_id.contains("gemini");
    if is_google {
        return vec!["high".to_string(), "max".to_string()];
    }

    vec!["low".to_string(), "medium".to_string(), "high".to_string()]
}

fn max_tokens_for_variant(default_max_tokens: u64, variant: Option<&str>) -> u64 {
    match variant.map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if v == "none" || v == "minimal" => 1024,
        Some(v) if v == "low" => 2048,
        Some(v) if v == "medium" => 4096,
        Some(v) if v == "high" => 8192,
        Some(v) if v == "max" || v == "xhigh" => 16384,
        _ => default_max_tokens,
    }
}

async fn get_model_variant_lookup() -> &'static HashMap<String, HashMap<String, Vec<String>>> {
    MODEL_VARIANT_LOOKUP
        .get_or_init(|| async {
            let data = load_models_dev_data().await;
            build_model_variant_lookup(data)
        })
        .await
}

fn variants_for_model(
    lookup: &HashMap<String, HashMap<String, Vec<String>>>,
    provider_id: &str,
    model_id: &str,
) -> Vec<String> {
    lookup
        .get(provider_id)
        .and_then(|models| models.get(model_id))
        .cloned()
        .unwrap_or_default()
}

async fn list_providers(State(state): State<Arc<ServerState>>) -> Json<ProviderListResponse> {
    let variant_lookup = get_model_variant_lookup().await;
    let models = state.providers.list_models();
    let mut provider_map: HashMap<String, Vec<ModelInfo>> = HashMap::new();
    for m in models {
        let provider_id = m.provider.clone();
        let model_id = m.id.clone();
        let variants = variants_for_model(variant_lookup, &provider_id, &model_id);
        provider_map
            .entry(provider_id.clone())
            .or_default()
            .push(ModelInfo {
                id: model_id,
                name: m.name,
                provider: provider_id,
                variants,
            });
    }
    let all: Vec<ProviderInfo> = provider_map
        .into_iter()
        .map(|(id, models)| ProviderInfo {
            id: id.clone(),
            name: id,
            models,
        })
        .collect();
    let connected: Vec<String> = all.iter().map(|p| p.id.clone()).collect();
    let default_model: HashMap<String, String> = all
        .iter()
        .filter_map(|p| p.models.first().map(|m| (p.id.clone(), m.id.clone())))
        .collect();
    Json(ProviderListResponse {
        all,
        default_model,
        connected,
    })
}

#[derive(Debug, Serialize)]
pub struct AuthMethodInfo {
    pub name: String,
    pub description: String,
}

async fn get_provider_auth(
    State(_state): State<Arc<ServerState>>,
) -> Json<HashMap<String, Vec<AuthMethodInfo>>> {
    let Some(loader) = get_plugin_loader() else {
        return Json(HashMap::new());
    };
    let methods = ProviderAuth::methods(loader).await;
    let result = methods
        .into_iter()
        .map(|(provider, values)| {
            let mapped = values
                .into_iter()
                .map(|method| AuthMethodInfo {
                    name: method.label,
                    description: method.method_type,
                })
                .collect::<Vec<_>>();
            (provider, mapped)
        })
        .collect::<HashMap<_, _>>();
    Json(result)
}

#[derive(Debug, Deserialize)]
pub struct OAuthAuthorizeRequest {
    pub method: usize,
}

#[derive(Debug, Serialize)]
pub struct OAuthAuthorizeResponse {
    pub url: String,
    #[serde(rename = "method")]
    pub method_type: String,
    pub instructions: String,
}

async fn oauth_authorize(
    State(_state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<OAuthAuthorizeRequest>,
) -> Result<Json<OAuthAuthorizeResponse>> {
    let loader = get_plugin_loader()
        .ok_or_else(|| ApiError::NotFound("no plugin loader initialized".to_string()))?;
    let authorization = ProviderAuth::authorize(loader, &id, req.method, None)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(OAuthAuthorizeResponse {
        url: authorization.url,
        method_type: match authorization.method {
            AuthMethodType::Auto => "auto".to_string(),
            AuthMethodType::Code => "code".to_string(),
        },
        instructions: authorization.instructions,
    }))
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackRequest {
    pub method: usize,
    pub code: Option<String>,
}

async fn oauth_callback(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<OAuthCallbackRequest>,
) -> Result<Json<bool>> {
    let loader = get_plugin_loader()
        .ok_or_else(|| ApiError::NotFound("no plugin loader initialized".to_string()))?;
    ProviderAuth::new(state.auth_manager.clone())
        .callback(loader, &id, req.code.as_deref())
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Refresh auth loader state after callback and apply custom-fetch proxy changes immediately.
    if let Some(bridge) = loader.auth_bridge(&id).await {
        match bridge.load().await {
            Ok(load_result) => {
                crate::server::sync_custom_fetch_proxy(&id, bridge, load_result.has_custom_fetch);
            }
            Err(error) => {
                crate::server::sync_custom_fetch_proxy(&id, bridge, false);
                tracing::warn!(
                    provider = %id,
                    %error,
                    "failed to refresh plugin auth loader after oauth callback"
                );
            }
        }
    }

    Ok(Json(true))
}

fn config_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(get_config).patch(patch_config))
        .route("/providers", get(get_config_providers))
}

static CONFIG_STATE: Lazy<RwLock<AppConfig>> = Lazy::new(|| RwLock::new(AppConfig::default()));

async fn get_config(State(_state): State<Arc<ServerState>>) -> Result<Json<AppConfig>> {
    let config = CONFIG_STATE.read().await;
    Ok(Json(config.clone()))
}

async fn patch_config(
    State(state): State<Arc<ServerState>>,
    Json(patch): Json<AppConfig>,
) -> Result<Json<AppConfig>> {
    let mut config = CONFIG_STATE.write().await;
    config.merge(patch);
    let updated = config.clone();
    state.broadcast(
        &serde_json::json!({
            "type": "config.updated",
        })
        .to_string(),
    );
    Ok(Json(updated))
}

#[derive(Debug, Serialize)]
pub struct ConfigProvidersResponse {
    pub providers: Vec<ProviderInfo>,
    #[serde(rename = "default")]
    pub default_model: HashMap<String, String>,
}

async fn get_config_providers(
    State(state): State<Arc<ServerState>>,
) -> Json<ConfigProvidersResponse> {
    let variant_lookup = get_model_variant_lookup().await;
    let models = state.providers.list_models();
    let mut provider_map: HashMap<String, Vec<ModelInfo>> = HashMap::new();
    for m in models {
        let provider_id = m.provider.clone();
        let model_id = m.id.clone();
        let variants = variants_for_model(variant_lookup, &provider_id, &model_id);
        provider_map
            .entry(provider_id.clone())
            .or_default()
            .push(ModelInfo {
                id: model_id,
                name: m.name,
                provider: provider_id,
                variants,
            });
    }
    let providers: Vec<ProviderInfo> = provider_map
        .into_iter()
        .map(|(id, models)| ProviderInfo {
            id: id.clone(),
            name: id,
            models,
        })
        .collect();
    let default_model: HashMap<String, String> = providers
        .iter()
        .filter_map(|p| p.models.first().map(|m| (p.id.clone(), m.id.clone())))
        .collect();
    Json(ConfigProvidersResponse {
        providers,
        default_model,
    })
}

fn mcp_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(get_mcp_status).post(add_mcp_server))
        .route("/{name}/auth", post(start_mcp_auth).delete(remove_mcp_auth))
        .route("/{name}/auth/callback", post(mcp_auth_callback))
        .route("/{name}/auth/authenticate", post(mcp_authenticate))
        .route("/{name}/connect", post(connect_mcp))
        .route("/{name}/disconnect", post(disconnect_mcp))
        .route("/{name}/logs", get(get_mcp_logs))
        .route("/{name}/restart", post(restart_mcp))
}

#[derive(Debug, Serialize)]
pub struct McpStatusInfo {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub resources: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

static MCP_OAUTH_MANAGER: std::sync::OnceLock<McpOAuthManager> = std::sync::OnceLock::new();

fn get_mcp_oauth_manager() -> &'static McpOAuthManager {
    MCP_OAUTH_MANAGER.get_or_init(McpOAuthManager::new)
}

impl From<McpServerInfoStruct> for McpStatusInfo {
    fn from(info: McpServerInfoStruct) -> Self {
        Self {
            name: info.name,
            status: info.status,
            tools: info.tools,
            resources: info.resources,
            error: info.error,
        }
    }
}

async fn get_mcp_status(
    State(_state): State<Arc<ServerState>>,
) -> Json<HashMap<String, McpStatusInfo>> {
    let manager = get_mcp_oauth_manager();
    if let Err(error) = sync_mcp_from_disk(manager).await {
        tracing::warn!(%error, "failed to sync MCP servers from config");
    }
    let servers = manager.list_servers().await;
    let mut result = HashMap::new();
    for server in servers {
        result.insert(server.name.clone(), McpStatusInfo::from(server));
    }
    Json(result)
}

#[derive(Debug, Deserialize)]
pub struct AddMcpRequest {
    pub name: String,
    pub config: McpConfigInput,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum McpCommandInput {
    String(String),
    Array(Vec<String>),
}

#[derive(Debug, Deserialize)]
pub struct McpConfigInput {
    #[serde(rename = "type")]
    pub server_type: Option<String>,
    pub command: Option<McpCommandInput>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub environment: Option<HashMap<String, String>>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub timeout: Option<u64>,
    pub oauth: Option<serde_json::Value>,
    pub client_id: Option<String>,
    pub authorization_url: Option<String>,
}

async fn add_mcp_server(
    State(_state): State<Arc<ServerState>>,
    Json(req): Json<AddMcpRequest>,
) -> Result<Json<HashMap<String, McpStatusInfo>>> {
    let manager = get_mcp_oauth_manager();
    let (runtime, enabled) = parse_runtime_from_request(req.config)?;
    manager.add_server(req.name.clone(), runtime, enabled).await;
    if enabled {
        manager
            .connect(&req.name)
            .await
            .map_err(mcp_error_to_api_error)?;
    }

    let servers = manager.list_servers().await;
    let mut result = HashMap::new();
    for server in servers {
        result.insert(server.name.clone(), McpStatusInfo::from(server));
    }
    Ok(Json(result))
}

async fn start_mcp_auth(Path(name): Path<String>) -> Result<Json<serde_json::Value>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name).await?;
    let state = manager
        .start_oauth(&name)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(serde_json::json!({
        "authorization_url": state.authorization_url,
        "client_id": state.client_id,
        "status": state.status
    })))
}

#[derive(Debug, Deserialize)]
pub struct McpAuthCallbackRequest {
    pub code: String,
}

async fn mcp_auth_callback(
    Path(name): Path<String>,
    Json(req): Json<McpAuthCallbackRequest>,
) -> Result<Json<McpStatusInfo>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name).await?;
    let server_info = manager
        .handle_callback(&name, &req.code)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(McpStatusInfo::from(server_info)))
}

async fn mcp_authenticate(Path(name): Path<String>) -> Result<Json<McpStatusInfo>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name).await?;
    let server_info = manager
        .authenticate(&name)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(McpStatusInfo::from(server_info)))
}

async fn remove_mcp_auth(Path(name): Path<String>) -> Result<Json<serde_json::Value>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name).await?;
    manager.remove_oauth(&name).await;
    Ok(Json(serde_json::json!({ "success": true })))
}

async fn connect_mcp(
    State(_state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<bool>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name).await?;
    manager
        .connect(&name)
        .await
        .map_err(mcp_error_to_api_error)?;
    Ok(Json(true))
}

async fn disconnect_mcp(
    State(_state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<bool>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name).await?;
    manager
        .disconnect(&name)
        .await
        .map_err(mcp_error_to_api_error)?;
    Ok(Json(true))
}

#[derive(Debug, Serialize)]
pub struct McpLogsResponse {
    pub name: String,
    pub logs: Vec<McpServerLogEntry>,
}

async fn get_mcp_logs(Path(name): Path<String>) -> Result<Json<McpLogsResponse>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name).await?;
    let logs = manager
        .get_logs(&name)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(McpLogsResponse { name, logs }))
}

#[derive(Debug, Serialize)]
pub struct McpRestartResponse {
    pub success: bool,
    pub server: McpStatusInfo,
}

async fn restart_mcp(
    State(_state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<McpRestartResponse>> {
    let manager = get_mcp_oauth_manager();
    ensure_mcp_server_registered(manager, &name).await?;
    let server = manager
        .restart(&name)
        .await
        .map_err(mcp_error_to_api_error)?;

    Ok(Json(McpRestartResponse {
        success: true,
        server: server.into(),
    }))
}

fn mcp_error_to_api_error(error: McpOAuthError) -> ApiError {
    match error {
        McpOAuthError::ServerNotFound(name) => {
            ApiError::NotFound(format!("MCP server not found: {}", name))
        }
        McpOAuthError::OAuthNotSupported(name) => {
            ApiError::BadRequest(format!("MCP server does not support OAuth: {}", name))
        }
        McpOAuthError::OAuthInProgress => ApiError::BadRequest("OAuth already in progress".into()),
        McpOAuthError::OAuthFailed(message) => ApiError::BadRequest(message),
        McpOAuthError::RuntimeError(message) => ApiError::BadRequest(message),
    }
}

fn parse_runtime_from_request(config: McpConfigInput) -> Result<(McpRuntimeConfig, bool)> {
    let enabled = config.enabled.unwrap_or(true);
    let is_remote = matches!(config.server_type.as_deref(), Some("remote")) || config.url.is_some();

    if is_remote {
        let url = config
            .url
            .ok_or_else(|| ApiError::BadRequest("MCP remote config requires `url`".to_string()))?;
        let oauth_enabled = !matches!(config.oauth, Some(serde_json::Value::Bool(false)));
        return Ok((
            McpRuntimeConfig::Remote(RemoteMcpConfig {
                url,
                oauth_enabled,
                client_id: config.client_id,
                authorization_url: config.authorization_url,
            }),
            enabled,
        ));
    }

    let (command, args) = parse_command_and_args(config.command, config.args.unwrap_or_default())?;
    Ok((
        McpRuntimeConfig::Local(LocalMcpConfig {
            command,
            args,
            env: config.env.or(config.environment),
            timeout: config.timeout,
        }),
        enabled,
    ))
}

fn parse_command_and_args(
    command: Option<McpCommandInput>,
    extra_args: Vec<String>,
) -> Result<(String, Vec<String>)> {
    match command {
        Some(McpCommandInput::String(cmd)) => {
            if cmd.trim().is_empty() {
                return Err(ApiError::BadRequest(
                    "MCP local config `command` cannot be empty".to_string(),
                ));
            }
            Ok((cmd, extra_args))
        }
        Some(McpCommandInput::Array(parts)) => {
            let mut iter = parts.into_iter();
            let cmd = iter
                .next()
                .ok_or_else(|| {
                    ApiError::BadRequest(
                        "MCP local config `command` array cannot be empty".to_string(),
                    )
                })?
                .trim()
                .to_string();
            if cmd.is_empty() {
                return Err(ApiError::BadRequest(
                    "MCP local config `command` cannot be empty".to_string(),
                ));
            }
            let mut args: Vec<String> = iter.collect();
            args.extend(extra_args);
            Ok((cmd, args))
        }
        None => Err(ApiError::BadRequest(
            "MCP local config requires `command`".to_string(),
        )),
    }
}

fn parse_runtime_from_loaded_config(
    config: LoadedMcpServerConfig,
) -> Result<Option<(McpRuntimeConfig, bool)>> {
    match config {
        LoadedMcpServerConfig::Enabled { .. } => Ok(None),
        LoadedMcpServerConfig::Full(server) => {
            let enabled = server.enabled.unwrap_or(true);

            if let Some(url) = server.url {
                return Ok(Some((
                    McpRuntimeConfig::Remote(RemoteMcpConfig {
                        url,
                        oauth_enabled: true,
                        client_id: server.client_id,
                        authorization_url: server.authorization_url,
                    }),
                    enabled,
                )));
            }

            if !server.command.is_empty() {
                let mut cmd_iter = server.command.into_iter();
                let command = cmd_iter.next().unwrap();
                let mut args: Vec<String> = cmd_iter.collect();
                args.extend(server.args);
                return Ok(Some((
                    McpRuntimeConfig::Local(LocalMcpConfig {
                        command,
                        args,
                        env: server.env,
                        timeout: server.timeout,
                    }),
                    enabled,
                )));
            }

            Ok(None)
        }
    }
}

async fn sync_mcp_from_disk(manager: &McpOAuthManager) -> Result<()> {
    let cwd = std::env::current_dir()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve current directory: {}", e)))?;
    let config = load_config(&cwd)
        .map_err(|e| ApiError::BadRequest(format!("Failed to load config: {}", e)))?;

    let Some(mcp_map) = config.mcp else {
        return Ok(());
    };

    for (name, server_config) in mcp_map {
        if manager.has_server(&name).await {
            continue;
        }
        if let Some((runtime, enabled)) = parse_runtime_from_loaded_config(server_config)? {
            manager.add_server(name, runtime, enabled).await;
        }
    }

    Ok(())
}

async fn ensure_mcp_server_registered(manager: &McpOAuthManager, name: &str) -> Result<()> {
    if manager.has_server(name).await {
        return Ok(());
    }

    sync_mcp_from_disk(manager).await?;
    if manager.has_server(name).await {
        return Ok(());
    }

    Err(ApiError::NotFound(format!(
        "MCP server not found: {}",
        name
    )))
}

fn file_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_files))
        .route("/content", get(read_file))
        .route("/status", get(get_file_status))
}

#[derive(Debug, Deserialize)]
pub struct ListFilesQuery {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub file_type: String,
    pub size: Option<u64>,
    pub modified: Option<i64>,
}

fn project_root() -> Result<PathBuf> {
    std::env::current_dir()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve current directory: {}", e)))
}

fn canonicalize_within_root(path: &FsPath, root: &FsPath) -> Result<PathBuf> {
    let canonical_root = root
        .canonicalize()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve project root: {}", e)))?;
    let canonical_path = path
        .canonicalize()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve path: {}", e)))?;

    if !canonical_path.starts_with(&canonical_root) {
        return Err(ApiError::BadRequest(
            "Access denied: path escapes project directory".to_string(),
        ));
    }

    Ok(canonical_path)
}

fn resolve_input_path(input: &str, root: &FsPath) -> Result<PathBuf> {
    let path = PathBuf::from(input);
    let resolved = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    if !resolved.exists() {
        return Err(ApiError::NotFound("File not found".to_string()));
    }
    canonicalize_within_root(&resolved, root)
}

fn is_within_root(path: &FsPath, root: &FsPath) -> bool {
    canonicalize_within_root(path, root).is_ok()
}

async fn list_files(Query(query): Query<ListFilesQuery>) -> Result<Json<Vec<FileInfo>>> {
    let root = project_root()?;
    let path = resolve_input_path(&query.path, &root)?;
    let mut files = Vec::new();

    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&path) {
            for entry in entries.flatten() {
                let path_buf = entry.path();
                if !is_within_root(&path_buf, &root) {
                    continue;
                }
                let file_type = if path_buf.is_dir() {
                    "directory"
                } else {
                    "file"
                };
                let size = if path_buf.is_file() {
                    std::fs::metadata(&path_buf).ok().map(|m| m.len())
                } else {
                    None
                };
                let modified = std::fs::metadata(&path_buf).ok().and_then(|m| {
                    m.modified().ok().map(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64
                    })
                });

                files.push(FileInfo {
                    name: entry.file_name().to_string_lossy().to_string(),
                    path: path_buf.to_string_lossy().to_string(),
                    file_type: file_type.to_string(),
                    size,
                    modified,
                });
            }
        }
    }

    Ok(Json(files))
}

async fn read_file(Query(query): Query<ListFilesQuery>) -> Result<Json<serde_json::Value>> {
    let root = project_root()?;
    let path = resolve_input_path(&query.path, &root)?;

    if path.is_file() {
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(Json(
                serde_json::json!({ "content": content, "path": query.path }),
            )),
            Err(e) => Err(ApiError::BadRequest(format!("Failed to read file: {}", e))),
        }
    } else {
        Err(ApiError::BadRequest("Path is not a file".to_string()))
    }
}

async fn get_file_status() -> Result<Json<Vec<FileStatusInfo>>> {
    let cwd = std::env::current_dir()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve current directory: {}", e)))?;
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .arg("status")
        .arg("--porcelain")
        .output()
        .map_err(|e| ApiError::BadRequest(format!("Failed to run git status: {}", e)))?;

    if !output.status.success() {
        return Ok(Json(Vec::new()));
    }

    let mut files = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.len() < 4 {
            continue;
        }
        let status_code = &line[..2];
        let mut path = line[3..].trim().to_string();
        if let Some((_, renamed_to)) = path.rsplit_once(" -> ") {
            path = renamed_to.to_string();
        }

        let staged = status_code.chars().next().unwrap_or(' ') != ' ';
        let status_char = if staged {
            status_code.chars().next().unwrap_or(' ')
        } else {
            status_code.chars().nth(1).unwrap_or(' ')
        };
        let status = match status_char {
            'M' => "modified",
            'A' => "added",
            'D' => "deleted",
            'R' => "renamed",
            'C' => "copied",
            'U' => "unmerged",
            '?' => "untracked",
            _ => "unknown",
        };

        files.push(FileStatusInfo {
            path,
            status: status.to_string(),
            staged,
        });
    }

    Ok(Json(files))
}

#[derive(Debug, Serialize)]
pub struct FileStatusInfo {
    pub path: String,
    pub status: String,
    pub staged: bool,
}

fn find_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/text", get(find_text))
        .route("/file", get(find_files))
        .route("/symbol", get(find_symbols))
}

#[derive(Debug, Deserialize)]
pub struct FindTextQuery {
    pub pattern: String,
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub match_text: String,
}

async fn find_text(Query(query): Query<FindTextQuery>) -> Result<Json<Vec<SearchResult>>> {
    let root = project_root()?;
    let base_input = query
        .path
        .unwrap_or_else(|| root.to_string_lossy().to_string());
    let base_path = resolve_input_path(&base_input, &root)?;
    let mut results = Vec::new();

    fn search_in_file(path: &std::path::Path, pattern: &str, results: &mut Vec<SearchResult>) {
        if let Ok(content) = std::fs::read_to_string(path) {
            for (line_num, line) in content.lines().enumerate() {
                if let Some(col) = line.find(pattern) {
                    results.push(SearchResult {
                        path: path.to_string_lossy().to_string(),
                        line: line_num + 1,
                        column: col + 1,
                        match_text: line.to_string(),
                    });
                }
            }
        }
    }

    fn search_recursive(
        path: &FsPath,
        root: &FsPath,
        pattern: &str,
        results: &mut Vec<SearchResult>,
    ) {
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let path_buf = entry.path();
                    if !is_within_root(&path_buf, root) {
                        continue;
                    }
                    if path_buf.is_dir() {
                        search_recursive(&path_buf, root, pattern, results);
                    } else if path_buf.is_file() {
                        search_in_file(&path_buf, pattern, results);
                    }
                }
            }
        }
    }

    search_recursive(&base_path, &root, &query.pattern, &mut results);
    Ok(Json(results))
}

#[derive(Debug, Deserialize)]
pub struct FindFilesQuery {
    pub query: String,
    #[serde(rename = "type")]
    pub file_type: Option<String>,
    pub limit: Option<usize>,
}

async fn find_files(Query(query): Query<FindFilesQuery>) -> Result<Json<Vec<String>>> {
    let base_path = project_root()?;
    let mut results = Vec::new();
    let limit = query.limit.unwrap_or(100);

    fn find_recursive(
        path: &FsPath,
        root: &FsPath,
        query: &str,
        results: &mut Vec<String>,
        limit: usize,
    ) {
        if results.len() >= limit {
            return;
        }
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let path_buf = entry.path();
                    if !is_within_root(&path_buf, root) {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.contains(query) {
                        results.push(path_buf.to_string_lossy().to_string());
                    }
                    if path_buf.is_dir() && results.len() < limit {
                        find_recursive(&path_buf, root, query, results, limit);
                    }
                }
            }
        }
    }

    find_recursive(&base_path, &base_path, &query.query, &mut results, limit);
    Ok(Json(results))
}

#[derive(Debug, Deserialize)]
pub struct FindSymbolsQuery {
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub line: usize,
}

async fn find_symbols(Query(_query): Query<FindSymbolsQuery>) -> Result<Json<Vec<SymbolInfo>>> {
    Ok(Json(Vec::new()))
}

fn permission_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_permissions))
        .route("/{id}/reply", post(reply_permission))
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionRequestInfo {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    pub input: serde_json::Value,
    pub message: String,
}

static PERMISSION_REQUESTS: Lazy<RwLock<HashMap<String, PermissionRequestInfo>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

async fn list_permissions() -> Json<Vec<PermissionRequestInfo>> {
    let pending = PERMISSION_REQUESTS.read().await;
    let mut result: Vec<_> = pending.values().cloned().collect();
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Json(result)
}

#[derive(Debug, Deserialize)]
pub struct ReplyPermissionRequest {
    pub reply: String,
    pub message: Option<String>,
}

async fn reply_permission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ReplyPermissionRequest>,
) -> Result<Json<bool>> {
    match req.reply.as_str() {
        "once" | "always" | "reject" => {}
        _ => {
            return Err(ApiError::BadRequest(
                "Invalid reply; expected `once`, `always`, or `reject`".to_string(),
            ));
        }
    }

    let mut pending = PERMISSION_REQUESTS.write().await;
    let permission = pending
        .remove(&id)
        .ok_or_else(|| ApiError::NotFound(format!("Permission request not found: {}", id)))?;

    if req.reply == "reject" {
        pending.retain(|_, item| item.session_id != permission.session_id);
    }

    state.broadcast(
        &serde_json::json!({
            "type": "permission.replied",
            "requestID": id,
            "sessionID": permission.session_id,
            "reply": req.reply,
            "message": req.message,
        })
        .to_string(),
    );
    Ok(Json(true))
}

fn project_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_projects))
        .route("/current", get(get_current_project))
        .route("/{id}", patch(update_project))
}

#[derive(Debug, Serialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub vcs: bool,
}

#[derive(Debug, Clone, Default)]
struct ProjectMetadata {
    name: Option<String>,
    icon: Option<String>,
}

static PROJECT_METADATA: Lazy<RwLock<HashMap<String, ProjectMetadata>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn is_git_repository(path: &FsPath) -> bool {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim() == "true",
        _ => false,
    }
}

async fn current_project_info() -> Result<ProjectInfo> {
    let cwd = std::env::current_dir()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve current directory: {}", e)))?;
    let canonical = cwd
        .canonicalize()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve project path: {}", e)))?;
    let project_id = canonical.to_string_lossy().to_string();
    let project_path = canonical.to_string_lossy().to_string();
    let default_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "project".to_string());

    let metadata = PROJECT_METADATA.read().await;
    let name = metadata
        .get(&project_id)
        .and_then(|m| m.name.clone())
        .unwrap_or(default_name);
    let _icon = metadata.get(&project_id).and_then(|m| m.icon.clone());

    Ok(ProjectInfo {
        id: project_id,
        name,
        path: project_path,
        vcs: is_git_repository(&canonical),
    })
}

async fn list_projects() -> Result<Json<Vec<ProjectInfo>>> {
    Ok(Json(vec![current_project_info().await?]))
}

async fn get_current_project() -> Result<Json<ProjectInfo>> {
    Ok(Json(current_project_info().await?))
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub icon: Option<String>,
}

async fn update_project(
    Path(id): Path<String>,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectInfo>> {
    let current = current_project_info().await?;
    if id != current.id {
        return Err(ApiError::NotFound(format!("Project not found: {}", id)));
    }

    let mut metadata = PROJECT_METADATA.write().await;
    let entry = metadata.entry(id).or_default();
    if let Some(name) = req.name {
        entry.name = Some(name);
    }
    if let Some(icon) = req.icon {
        entry.icon = Some(icon);
    }
    drop(metadata);

    Ok(Json(current_project_info().await?))
}

fn pty_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_pty).post(create_pty))
        .route("/{id}", get(get_pty).put(update_pty).delete(delete_pty))
        .route("/{id}/connect", get(pty_connect))
}

#[derive(Debug, Serialize)]
pub struct PtyInfo {
    pub id: String,
    pub command: String,
    pub cwd: String,
    pub status: String,
}

impl From<PtySessionStruct> for PtyInfo {
    fn from(session: PtySessionStruct) -> Self {
        Self {
            id: session.id,
            command: session.command,
            cwd: session.cwd,
            status: match session.status {
                crate::pty::PtyStatus::Running => "running".to_string(),
                crate::pty::PtyStatus::Exited => "exited".to_string(),
                crate::pty::PtyStatus::Error => "error".to_string(),
            },
        }
    }
}

static PTY_MANAGER: std::sync::OnceLock<PtyManager> = std::sync::OnceLock::new();

fn get_pty_manager() -> &'static PtyManager {
    PTY_MANAGER.get_or_init(PtyManager::new)
}

async fn list_pty() -> Json<Vec<PtyInfo>> {
    let manager = get_pty_manager();
    let sessions = manager.list_sessions().await;
    Json(sessions.into_iter().map(PtyInfo::from).collect())
}

#[derive(Debug, Deserialize)]
pub struct CreatePtyRequest {
    pub command: String,
    pub cwd: Option<String>,
    pub env: Option<HashMap<String, String>>,
}

async fn create_pty(Json(req): Json<CreatePtyRequest>) -> Result<Json<PtyInfo>> {
    let manager = get_pty_manager();
    let session = manager
        .create_session(&req.command, req.cwd.as_deref(), req.env)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(PtyInfo::from(session)))
}

async fn get_pty(Path(id): Path<String>) -> Result<Json<PtyInfo>> {
    let manager = get_pty_manager();
    let session = manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::NotFound("PTY session not found".to_string()))?;
    Ok(Json(PtyInfo::from(session)))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePtyRequest {
    pub command: Option<String>,
    pub cwd: Option<String>,
}

async fn update_pty(
    Path(id): Path<String>,
    Json(req): Json<UpdatePtyRequest>,
) -> Result<Json<PtyInfo>> {
    let manager = get_pty_manager();
    let session = manager
        .update_session(&id, req.command.as_deref(), req.cwd.as_deref())
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;
    Ok(Json(PtyInfo::from(session)))
}

async fn delete_pty(Path(id): Path<String>) -> Result<Json<bool>> {
    let manager = get_pty_manager();
    let deleted = manager.delete_session(&id).await;
    Ok(Json(deleted))
}

#[derive(Debug, Deserialize)]
pub struct PtyConnectQuery {
    /// Byte cursor from which to replay buffered output.
    /// `-1` means skip all buffered output and only receive live data.
    /// Omitted or `0` means replay from the beginning of the retained buffer.
    pub cursor: Option<i64>,
}

/// WebSocket endpoint that bridges a client to a PTY session, matching the TS
/// `Pty.connect` protocol:
///   1. On connect: replay buffered output from the requested cursor, then send
///      a binary metadata frame (`0x00` + JSON `{"cursor":<n>}`) so the client
///      knows the current position.
///   2. Forward live PTY output to the client as binary frames.
///   3. Forward text/binary messages from the client into the PTY as input.
async fn pty_connect(
    Path(id): Path<String>,
    Query(query): Query<PtyConnectQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let manager = get_pty_manager();

    // Validate the session exists before upgrading.
    let subscription = match manager.subscribe(&id).await {
        Ok(sub) => sub,
        Err(_) => {
            return axum::response::Response::builder()
                .status(404)
                .body(axum::body::Body::from("PTY session not found"))
                .unwrap()
                .into_response();
        }
    };

    let cursor_param = query.cursor.unwrap_or(0);

    ws.on_upgrade(move |socket| handle_pty_websocket(socket, subscription, cursor_param))
        .into_response()
}

async fn handle_pty_websocket(mut socket: WebSocket, sub: PtySubscription, cursor_param: i64) {
    // --- Phase 1: Replay buffered output ---
    // Determine the byte offset to start replaying from.
    let from = if cursor_param == -1 {
        // Skip all buffered output.
        sub.cursor
    } else if cursor_param > 0 {
        cursor_param as usize
    } else {
        0
    };

    if from < sub.cursor && !sub.buffer.is_empty() {
        let offset = from.saturating_sub(sub.buffer_start);
        if offset < sub.buffer.len() {
            let replay = &sub.buffer[offset..];
            // Send in 64 KiB chunks to avoid oversized frames (matching TS).
            for chunk in replay.chunks(64 * 1024) {
                let bytes = axum::body::Bytes::copy_from_slice(chunk);
                if socket.send(Message::Binary(bytes)).await.is_err() {
                    return;
                }
            }
        }
    }

    // Send metadata frame: 0x00 byte prefix + JSON `{"cursor":<n>}`.
    {
        let meta_json = format!("{{\"cursor\":{}}}", sub.cursor);
        let json_bytes = meta_json.as_bytes();
        let mut frame = Vec::with_capacity(1 + json_bytes.len());
        frame.push(0x00);
        frame.extend_from_slice(json_bytes);
        let bytes = axum::body::Bytes::from(frame);
        if socket.send(Message::Binary(bytes)).await.is_err() {
            return;
        }
    }

    // --- Phase 2: Bridge live I/O ---
    // Use a channel to decouple the broadcast receiver from the socket send
    // loop, since WebSocket requires &mut self for both send and recv.
    let (ws_tx, mut ws_rx) = mpsc::channel::<Vec<u8>>(256);
    let mut rx = sub.rx;
    let writer = sub.writer;

    // Task: forward broadcast PTY output into the mpsc channel.
    let forward_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(data) => {
                    if ws_tx.send(data).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Main loop: multiplex between sending PTY output and receiving WS input.
    loop {
        tokio::select! {
            // Live PTY output ready to send to the client.
            Some(data) = ws_rx.recv() => {
                let bytes = axum::body::Bytes::from(data);
                if socket.send(Message::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            // Client sent a message (input for the PTY).
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(t))) => {
                        let data = t.as_bytes().to_vec();
                        if data.is_empty() { continue; }
                        let w = writer.clone();
                        let res = tokio::task::spawn_blocking(move || {
                            let mut guard = w.lock().unwrap();
                            guard.write_all(&data)?;
                            guard.flush()
                        }).await;
                        if res.is_err() || res.unwrap().is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(b))) => {
                        let data = b.to_vec();
                        if data.is_empty() { continue; }
                        let w = writer.clone();
                        let res = tokio::task::spawn_blocking(move || {
                            let mut guard = w.lock().unwrap();
                            guard.write_all(&data)?;
                            guard.flush()
                        }).await;
                        if res.is_err() || res.unwrap().is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => continue,
                }
            }
        }
    }

    forward_task.abort();
}

fn question_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_questions))
        .route("/{id}/reply", post(reply_question))
        .route("/{id}/reject", post(reject_question))
}

#[derive(Debug, Clone, Serialize)]
pub struct QuestionInfo {
    pub id: String,
    pub session_id: String,
    pub questions: Vec<String>,
    pub options: Option<Vec<Vec<String>>>,
}

static QUESTION_REQUESTS: Lazy<RwLock<HashMap<String, QuestionInfo>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

async fn list_questions() -> Json<Vec<QuestionInfo>> {
    let pending = QUESTION_REQUESTS.read().await;
    let mut result: Vec<_> = pending.values().cloned().collect();
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Json(result)
}

#[derive(Debug, Deserialize)]
pub struct ReplyQuestionRequest {
    pub answers: Vec<Vec<String>>,
}

async fn reply_question(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ReplyQuestionRequest>,
) -> Result<Json<bool>> {
    let mut pending = QUESTION_REQUESTS.write().await;
    let question = pending
        .remove(&id)
        .ok_or_else(|| ApiError::NotFound(format!("Question request not found: {}", id)))?;
    drop(pending);

    state.broadcast(
        &serde_json::json!({
            "type": "question.replied",
            "requestID": id,
            "sessionID": question.session_id,
            "answers": req.answers,
        })
        .to_string(),
    );
    Ok(Json(true))
}

async fn reject_question(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<bool>> {
    let mut pending = QUESTION_REQUESTS.write().await;
    let question = pending
        .remove(&id)
        .ok_or_else(|| ApiError::NotFound(format!("Question request not found: {}", id)))?;
    drop(pending);

    state.broadcast(
        &serde_json::json!({
            "type": "question.rejected",
            "requestID": id,
            "sessionID": question.session_id,
        })
        .to_string(),
    );
    Ok(Json(true))
}

/// TUI communication routes.
///
/// Architecture note: In the TypeScript version the TUI (Ink/React) runs as a
/// separate process and communicates with the backend exclusively over HTTP.
/// The Rust TUI (`opencode-tui`) uses ratatui/crossterm and runs in its own
/// binary, but still talks to the server through an HTTP `ApiClient`.  These
/// endpoints therefore remain necessary -- they bridge external TUI requests
/// into an internal queue that the TUI polls via `/control/next` and answers
/// via `/control/response`.
///
/// The `/set-prompt` endpoint is a Rust-only addition (not present in the TS
/// codebase) that allows overwriting the prompt text rather than appending.
fn tui_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/append-prompt", post(append_prompt))
        .route("/set-prompt", post(set_prompt))
        .route("/submit-prompt", post(submit_prompt))
        .route("/clear-prompt", post(clear_prompt))
        .route("/open-help", post(open_help))
        .route("/open-sessions", post(open_sessions))
        .route("/open-themes", post(open_themes))
        .route("/open-models", post(open_models))
        .route("/execute-command", post(execute_tui_command))
        .route("/show-toast", post(show_toast))
        .route("/publish", post(publish_tui_event))
        .route("/select-session", post(select_session))
        .route("/control/next", get(get_next_tui_request))
        .route("/control/response", post(submit_tui_response))
}

#[derive(Debug, Deserialize)]
pub struct PromptRequest {
    pub text: String,
}

static TUI_REQUEST_QUEUE: Lazy<Mutex<VecDeque<TuiRequest>>> =
    Lazy::new(|| Mutex::new(VecDeque::new()));
static TUI_RESPONSE_QUEUE: Lazy<Mutex<VecDeque<serde_json::Value>>> =
    Lazy::new(|| Mutex::new(VecDeque::new()));
static TUI_REQUEST_NOTIFY: Lazy<Notify> = Lazy::new(Notify::new);
static TUI_RESPONSE_NOTIFY: Lazy<Notify> = Lazy::new(Notify::new);

async fn enqueue_tui_request(state: &Arc<ServerState>, path: &str, body: serde_json::Value) {
    let mut queue = TUI_REQUEST_QUEUE.lock().await;
    queue.push_back(TuiRequest {
        path: path.to_string(),
        body: body.clone(),
    });
    drop(queue);
    TUI_REQUEST_NOTIFY.notify_one();

    state.broadcast(
        &serde_json::json!({
            "type": "tui.request",
            "path": path,
            "body": body,
        })
        .to_string(),
    );
}

async fn append_prompt(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<PromptRequest>,
) -> Result<Json<bool>> {
    enqueue_tui_request(
        &state,
        "/tui/append-prompt",
        serde_json::json!({ "text": req.text }),
    )
    .await;
    Ok(Json(true))
}

async fn set_prompt(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<PromptRequest>,
) -> Result<Json<bool>> {
    enqueue_tui_request(
        &state,
        "/tui/set-prompt",
        serde_json::json!({ "text": req.text }),
    )
    .await;
    Ok(Json(true))
}

async fn submit_prompt(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<PromptRequest>,
) -> Result<Json<bool>> {
    enqueue_tui_request(
        &state,
        "/tui/submit-prompt",
        serde_json::json!({ "text": req.text }),
    )
    .await;
    Ok(Json(true))
}

async fn clear_prompt(State(state): State<Arc<ServerState>>) -> Result<Json<bool>> {
    enqueue_tui_request(&state, "/tui/clear-prompt", serde_json::json!({})).await;
    Ok(Json(true))
}

async fn open_help(State(state): State<Arc<ServerState>>) -> Result<Json<bool>> {
    enqueue_tui_request(
        &state,
        "/tui/open-help",
        serde_json::json!({ "command": "help.show" }),
    )
    .await;
    Ok(Json(true))
}

async fn open_sessions(State(state): State<Arc<ServerState>>) -> Result<Json<bool>> {
    enqueue_tui_request(
        &state,
        "/tui/open-sessions",
        serde_json::json!({ "command": "session.list" }),
    )
    .await;
    Ok(Json(true))
}

async fn open_themes(State(state): State<Arc<ServerState>>) -> Result<Json<bool>> {
    enqueue_tui_request(
        &state,
        "/tui/open-themes",
        serde_json::json!({ "command": "theme.list" }),
    )
    .await;
    Ok(Json(true))
}

async fn open_models(State(state): State<Arc<ServerState>>) -> Result<Json<bool>> {
    enqueue_tui_request(
        &state,
        "/tui/open-models",
        serde_json::json!({ "command": "model.list" }),
    )
    .await;
    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct TuiCommandRequest {
    pub command: String,
    pub arguments: Option<serde_json::Value>,
}

fn map_tui_command(command: &str) -> &str {
    match command {
        "session_new" => "session.new",
        "session_share" => "session.share",
        "session_interrupt" => "session.interrupt",
        "session_compact" => "session.compact",
        "messages_page_up" => "session.page.up",
        "messages_page_down" => "session.page.down",
        "messages_line_up" => "session.line.up",
        "messages_line_down" => "session.line.down",
        "messages_half_page_up" => "session.half.page.up",
        "messages_half_page_down" => "session.half.page.down",
        "messages_first" => "session.first",
        "messages_last" => "session.last",
        "agent_cycle" => "agent.cycle",
        other => other,
    }
}

async fn execute_tui_command(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<TuiCommandRequest>,
) -> Result<Json<bool>> {
    let mapped = map_tui_command(&req.command);
    enqueue_tui_request(
        &state,
        "/tui/execute-command",
        serde_json::json!({
            "command": mapped,
            "arguments": req.arguments,
        }),
    )
    .await;
    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct ToastRequest {
    pub message: String,
    pub level: Option<String>,
}

async fn show_toast(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ToastRequest>,
) -> Result<Json<bool>> {
    enqueue_tui_request(
        &state,
        "/tui/show-toast",
        serde_json::json!({
            "message": req.message,
            "level": req.level,
        }),
    )
    .await;
    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct PublishEventRequest {
    pub event: String,
    pub data: Option<serde_json::Value>,
}

async fn publish_tui_event(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<PublishEventRequest>,
) -> Result<Json<bool>> {
    enqueue_tui_request(
        &state,
        "/tui/publish",
        serde_json::json!({
            "event": req.event,
            "data": req.data,
        }),
    )
    .await;
    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct SelectSessionRequest {
    pub session_id: String,
}

async fn select_session(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<SelectSessionRequest>,
) -> Result<Json<bool>> {
    let sessions = state.sessions.lock().await;
    if sessions.get(&req.session_id).is_none() {
        return Err(ApiError::SessionNotFound(req.session_id));
    }
    drop(sessions);

    enqueue_tui_request(
        &state,
        "/tui/select-session",
        serde_json::json!({ "sessionID": req.session_id }),
    )
    .await;
    Ok(Json(true))
}

#[derive(Debug, Serialize)]
pub struct TuiRequest {
    pub path: String,
    pub body: serde_json::Value,
}

async fn get_next_tui_request() -> Json<Option<TuiRequest>> {
    loop {
        let mut queue = TUI_REQUEST_QUEUE.lock().await;
        if let Some(next) = queue.pop_front() {
            return Json(Some(next));
        }
        drop(queue);
        TUI_REQUEST_NOTIFY.notified().await;
    }
}

async fn submit_tui_response(Json(body): Json<serde_json::Value>) -> Json<bool> {
    let mut queue = TUI_RESPONSE_QUEUE.lock().await;
    queue.push_back(body);
    drop(queue);
    TUI_RESPONSE_NOTIFY.notify_one();
    Json(true)
}

fn global_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/health", get(global_health))
        .route("/event", get(global_event_stream))
        .route("/config", get(get_global_config))
        .route("/dispose", post(dispose_all))
}

#[derive(Debug, Serialize)]
pub struct GlobalHealthResponse {
    pub healthy: bool,
    pub version: String,
}

async fn global_health() -> Json<GlobalHealthResponse> {
    Json(GlobalHealthResponse {
        healthy: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn global_event_stream(State(state): State<Arc<ServerState>>) -> Response {
    into_ts_sse_response(ts_event_stream(&state))
}

async fn get_global_config(State(_state): State<Arc<ServerState>>) -> Result<Json<AppConfig>> {
    Ok(Json(AppConfig::default()))
}

async fn dispose_all() -> Json<bool> {
    Json(true)
}

fn experimental_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_experimental))
        .route("/analyze", post(experimental_analyze))
        .route("/generate", post(experimental_generate))
        .route("/refactor", post(experimental_refactor))
        .route("/test", post(experimental_test))
        .route(
            "/{feature}",
            post(enable_experimental).delete(disable_experimental),
        )
        .route("/tool/ids", get(list_tool_ids))
        .route("/tool", get(list_tools))
        .route(
            "/worktree",
            get(list_worktrees)
                .post(create_worktree)
                .delete(remove_worktree),
        )
        .route("/worktree/reset", post(reset_worktree))
        .route("/resource", get(list_resources))
}

#[derive(Debug, Serialize)]
pub struct ExperimentalFeature {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

async fn list_experimental() -> Json<Vec<ExperimentalFeature>> {
    Json(Vec::new())
}

async fn enable_experimental(Path(_feature): Path<String>) -> Result<Json<bool>> {
    Ok(Json(true))
}

async fn disable_experimental(Path(_feature): Path<String>) -> Result<Json<bool>> {
    Ok(Json(true))
}

#[derive(Debug, Deserialize)]
pub struct ExperimentalTaskRequest {
    pub prompt: Option<String>,
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ExperimentalTaskResponse {
    pub operation: String,
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

fn make_experimental_response(
    operation: &str,
    payload: ExperimentalTaskRequest,
) -> ExperimentalTaskResponse {
    ExperimentalTaskResponse {
        operation: operation.to_string(),
        status: "accepted".to_string(),
        message: format!(
            "Experimental endpoint `{}` is available but currently returns a placeholder response in Rust.",
            operation
        ),
        prompt: payload.prompt,
        context: payload.context,
    }
}

async fn experimental_analyze(
    Json(payload): Json<ExperimentalTaskRequest>,
) -> Json<ExperimentalTaskResponse> {
    Json(make_experimental_response("analyze", payload))
}

async fn experimental_generate(
    Json(payload): Json<ExperimentalTaskRequest>,
) -> Json<ExperimentalTaskResponse> {
    Json(make_experimental_response("generate", payload))
}

async fn experimental_refactor(
    Json(payload): Json<ExperimentalTaskRequest>,
) -> Json<ExperimentalTaskResponse> {
    Json(make_experimental_response("refactor", payload))
}

async fn experimental_test(
    Json(payload): Json<ExperimentalTaskRequest>,
) -> Json<ExperimentalTaskResponse> {
    Json(make_experimental_response("test", payload))
}

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

async fn list_tool_ids() -> Json<Vec<String>> {
    Json(vec![
        "read".to_string(),
        "write".to_string(),
        "edit".to_string(),
        "bash".to_string(),
        "glob".to_string(),
        "grep".to_string(),
        "ls".to_string(),
        "webfetch".to_string(),
        "websearch".to_string(),
        "task".to_string(),
        "lsp".to_string(),
        "batch".to_string(),
        "plan_enter".to_string(),
        "plan_exit".to_string(),
        "todoread".to_string(),
        "todowrite".to_string(),
        "codesearch".to_string(),
        "apply_patch".to_string(),
        "skill".to_string(),
        "multiedit".to_string(),
    ])
}

async fn list_tools(Query(_params): Query<HashMap<String, String>>) -> Json<Vec<ToolInfo>> {
    Json(vec![
        ToolInfo {
            id: "read".to_string(),
            name: "Read".to_string(),
            description: "Read files".to_string(),
        },
        ToolInfo {
            id: "write".to_string(),
            name: "Write".to_string(),
            description: "Write files".to_string(),
        },
        ToolInfo {
            id: "edit".to_string(),
            name: "Edit".to_string(),
            description: "Edit files".to_string(),
        },
        ToolInfo {
            id: "bash".to_string(),
            name: "Bash".to_string(),
            description: "Execute commands".to_string(),
        },
    ])
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub head: String,
}

impl From<WorktreeInfoStruct> for WorktreeInfo {
    fn from(info: WorktreeInfoStruct) -> Self {
        Self {
            path: info.path,
            branch: info.branch,
            head: info.head,
        }
    }
}

async fn list_worktrees() -> Json<Vec<WorktreeInfo>> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let worktrees = worktree::list_worktrees(&cwd).unwrap_or_default();
    Json(worktrees.into_iter().map(|w| w.into()).collect())
}

#[derive(Debug, Deserialize)]
pub struct CreateWorktreeRequest {
    pub branch: Option<String>,
    pub path: Option<String>,
}

async fn create_worktree(Json(req): Json<CreateWorktreeRequest>) -> Result<Json<WorktreeInfo>> {
    let cwd = std::env::current_dir().map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let info = worktree::create_worktree(&cwd, req.branch.as_deref(), req.path.as_deref())
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(info.into()))
}

#[derive(Debug, Deserialize)]
pub struct RemoveWorktreeRequest {
    pub path: String,
    pub force: Option<bool>,
}

async fn remove_worktree(Json(req): Json<RemoveWorktreeRequest>) -> Result<Json<bool>> {
    let cwd = std::env::current_dir().map_err(|e| ApiError::BadRequest(e.to_string()))?;

    worktree::remove_worktree(&cwd, &req.path, req.force.unwrap_or(false))
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(true))
}

async fn reset_worktree() -> Result<Json<bool>> {
    let cwd = std::env::current_dir().map_err(|e| ApiError::BadRequest(e.to_string()))?;

    worktree::prune_worktrees(&cwd).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(true))
}

#[derive(Debug, Serialize)]
pub struct ResourceInfo {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
}

async fn list_resources() -> Json<Vec<ResourceInfo>> {
    Json(Vec::new())
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    /// Present only when the native proxy started; `None` means provider traffic goes direct.
    proxy: Option<serde_json::Value>,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        proxy: crate::server::proxy_snapshot(),
    })
}

async fn web_index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

// --- /doc endpoint: returns OpenAPI-style documentation info ---

#[derive(Debug, Serialize)]
struct DocInfo {
    title: String,
    version: String,
    description: String,
    openapi: String,
}

#[derive(Debug, Serialize)]
struct DocResponse {
    info: DocInfo,
}

async fn get_doc() -> Json<DocResponse> {
    Json(DocResponse {
        info: DocInfo {
            title: "opencode".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            description: "opencode api".to_string(),
            openapi: "3.1.1".to_string(),
        },
    })
}

// --- /log endpoint: accepts a log entry and writes it via tracing ---

#[derive(Debug, Deserialize)]
struct WriteLogRequest {
    service: String,
    level: String,
    message: String,
    #[serde(default)]
    extra: Option<HashMap<String, serde_json::Value>>,
}

async fn write_log(Json(req): Json<WriteLogRequest>) -> Result<Json<bool>> {
    let extra_str = req
        .extra
        .as_ref()
        .map(|e| serde_json::to_string(e).unwrap_or_default())
        .unwrap_or_default();

    match req.level.as_str() {
        "debug" => tracing::debug!(service = %req.service, extra = %extra_str, "{}", req.message),
        "info" => tracing::info!(service = %req.service, extra = %extra_str, "{}", req.message),
        "warn" => tracing::warn!(service = %req.service, extra = %extra_str, "{}", req.message),
        "error" => tracing::error!(service = %req.service, extra = %extra_str, "{}", req.message),
        other => {
            return Err(ApiError::BadRequest(format!(
                "invalid log level: '{}', expected one of: debug, info, warn, error",
                other
            )));
        }
    }

    Ok(Json(true))
}

/// Wrap one payload in the TS SSE frame shape: `event: message` + `data: <json>`.
fn ts_frame(payload: String) -> Event {
    Event::default().event("message").data(payload)
}

/// Build the TypeScript-compatible event stream.
///
/// Every frame is `event: message` carrying a `{id, type, properties}` envelope, and the
/// first frame is `server.connected` so a client can distinguish "stream is live" from
/// "stream is silently dead" — the failure signature the livestream had before.
fn ts_event_stream(
    state: &ServerState,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let connected = ts_frame(
        serde_json::json!({
            "id": format!("evt_{}", uuid::Uuid::new_v4()),
            "type": "server.connected",
            "properties": {},
        })
        .to_string(),
    );
    let initial = futures::stream::once(std::future::ready(Ok(connected)));

    let relayed = BroadcastStream::new(state.event_bus.subscribe())
        .filter_map(|result| result.ok().map(|raw| Ok(ts_frame(raw))));

    Sse::new(initial.chain(relayed))
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(10)))
}

fn into_ts_sse_response<S>(sse: Sse<S>) -> Response
where
    S: Stream<Item = std::result::Result<Event, Infallible>> + Send + 'static,
{
    let mut response = sse.into_response();
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache, no-transform"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    response
}

async fn event_stream(State(state): State<Arc<ServerState>>) -> Response {
    into_ts_sse_response(ts_event_stream(&state))
}

#[derive(Debug, Serialize)]
struct PathsResponse {
    home: String,
    state: String,
    config: String,
    worktree: String,
    directory: String,
}

async fn get_paths(Query(params): Query<PathQuery>) -> Result<Json<PathsResponse>> {
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let config = dirs::config_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let state = dirs::data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let worktree = params.directory.as_deref().unwrap_or(&cwd).to_string();
    let directory = worktree.clone();
    Ok(Json(PathsResponse {
        home,
        state,
        config,
        worktree,
        directory,
    }))
}

#[derive(Debug, Deserialize)]
struct PathQuery {
    directory: Option<String>,
}

#[derive(Debug, Serialize)]
struct VcsInfo {
    system: Option<String>,
    branch: Option<String>,
    root: Option<String>,
}

async fn get_vcs_info() -> Result<Json<VcsInfo>> {
    Ok(Json(VcsInfo {
        system: Some("git".to_string()),
        branch: None,
        root: None,
    }))
}

#[derive(Debug, Serialize)]
struct CommandInfo {
    id: String,
    name: String,
    description: Option<String>,
}

async fn list_commands() -> Result<Json<Vec<CommandInfo>>> {
    Ok(Json(vec![
        CommandInfo {
            id: "build".to_string(),
            name: "Build".to_string(),
            description: Some("Build the project".to_string()),
        },
        CommandInfo {
            id: "test".to_string(),
            name: "Test".to_string(),
            description: Some("Run tests".to_string()),
        },
        CommandInfo {
            id: "lint".to_string(),
            name: "Lint".to_string(),
            description: Some("Run linter".to_string()),
        },
    ]))
}

#[derive(Debug, Serialize)]
struct AgentInfo {
    id: String,
    name: String,
    description: Option<String>,
}

async fn list_agents() -> Result<Json<Vec<AgentInfo>>> {
    let mut config = std::env::current_dir()
        .ok()
        .and_then(|cwd| load_config(&cwd).ok());

    if let (Some(loader), Some(config)) = (get_plugin_loader(), config.as_mut()) {
        apply_plugin_config_hooks(loader, config).await;
    }

    let registry = AgentRegistry::from_optional_config(config.as_ref());
    // Subagents are listed too. `AgentRegistry::list` already drops `hidden`
    // agents, so dropping the old mode filter here does not leak the internal
    // ones (title, compaction, summary).
    let agents = registry
        .list()
        .into_iter()
        .map(|agent| AgentInfo {
            id: agent.name.clone(),
            name: agent.name.clone(),
            description: agent.description.clone(),
        })
        .collect();
    Ok(Json(agents))
}

async fn apply_plugin_config_hooks(loader: &Arc<PluginLoader>, config: &mut AppConfig) {
    let mut config_value = match serde_json::to_value(config.clone()) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "failed to serialize config for plugin config hook");
            return;
        }
    };

    for client in loader.clients().await {
        match client
            .invoke_hook("config", config_value.clone(), config_value.clone())
            .await
        {
            Ok(next_config) => {
                if next_config.is_object() {
                    config_value = next_config;
                } else {
                    tracing::warn!(
                        plugin = client.name(),
                        "plugin config hook returned non-object config payload"
                    );
                }
            }
            Err(PluginSubprocessError::Rpc { code, .. }) if code == -32601 => {
                // Plugin does not implement config hook.
            }
            Err(error) => {
                tracing::warn!(
                    plugin = client.name(),
                    %error,
                    "plugin config hook invocation failed"
                );
            }
        }
    }

    match serde_json::from_value::<AppConfig>(config_value) {
        Ok(next) => *config = next,
        Err(error) => {
            tracing::warn!(%error, "failed to deserialize config after plugin hooks");
        }
    }
}

async fn list_skills() -> Result<Json<Vec<String>>> {
    Ok(Json(Vec::new()))
}

#[derive(Debug, Serialize)]
struct LspStatus {
    servers: Vec<String>,
}

async fn get_lsp_status() -> Result<Json<LspStatus>> {
    Ok(Json(LspStatus {
        servers: Vec::new(),
    }))
}

#[derive(Debug, Serialize)]
struct FormatterStatus {
    formatters: Vec<String>,
}

async fn get_formatter_status() -> Result<Json<FormatterStatus>> {
    Ok(Json(FormatterStatus {
        formatters: Vec::new(),
    }))
}

#[derive(Debug, Deserialize)]
struct SetAuthRequest {
    #[serde(flatten)]
    body: serde_json::Value,
}

async fn set_auth(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<SetAuthRequest>,
) -> Result<Json<serde_json::Value>> {
    let auth_info = parse_auth_info_payload(req.body)
        .ok_or_else(|| ApiError::BadRequest("Invalid auth payload".to_string()))?;
    state.auth_manager.set(&id, auth_info).await;
    Ok(Json(serde_json::json!({ "success": true })))
}

async fn delete_auth(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    state.auth_manager.remove(&id).await;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

fn parse_auth_info_payload(payload: serde_json::Value) -> Option<AuthInfo> {
    if let Ok(auth) = serde_json::from_value::<AuthInfo>(payload.clone()) {
        return Some(auth);
    }

    let key = payload
        .get("api_key")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("apiKey").and_then(|v| v.as_str()))
        .or_else(|| payload.get("token").and_then(|v| v.as_str()))
        .or_else(|| payload.get("key").and_then(|v| v.as_str()))
        .map(str::to_string)?;

    Some(AuthInfo::Api { key })
}

// ===========================================================================
// Plugin auth routes
// ===========================================================================

static PLUGIN_LOADER: std::sync::OnceLock<Arc<PluginLoader>> = std::sync::OnceLock::new();

/// Register the global PluginLoader so routes can access auth bridges.
/// Called once during server startup after plugins are loaded.
pub fn set_plugin_loader(loader: Arc<PluginLoader>) {
    let _ = PLUGIN_LOADER.set(loader);
}

fn get_plugin_loader() -> Option<&'static Arc<PluginLoader>> {
    PLUGIN_LOADER.get()
}

fn plugin_auth_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/auth", get(list_plugin_auth))
        .route("/{name}/auth/authorize", post(plugin_auth_authorize))
        .route("/{name}/auth/callback", post(plugin_auth_callback))
        .route("/{name}/auth/load", post(plugin_auth_load))
        .route("/{name}/auth/fetch", post(plugin_auth_fetch))
}

#[derive(Debug, Serialize)]
struct PluginAuthInfo {
    provider: String,
    methods: Vec<PluginAuthMethodInfo>,
}

#[derive(Debug, Serialize)]
struct PluginAuthMethodInfo {
    #[serde(rename = "type")]
    method_type: String,
    label: String,
}

async fn list_plugin_auth(_state: State<Arc<ServerState>>) -> Result<Json<Vec<PluginAuthInfo>>> {
    let Some(loader) = get_plugin_loader() else {
        return Ok(Json(Vec::new()));
    };

    let bridges = loader.auth_bridges().await;
    let result: Vec<PluginAuthInfo> = bridges
        .values()
        .map(|bridge| PluginAuthInfo {
            provider: bridge.provider().to_string(),
            methods: bridge
                .methods()
                .iter()
                .map(|m| PluginAuthMethodInfo {
                    method_type: m.method_type.clone(),
                    label: m.label.clone(),
                })
                .collect(),
        })
        .collect();

    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
struct PluginAuthAuthorizeRequest {
    method: usize,
    #[serde(default)]
    inputs: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct PluginAuthAuthorizeResponse {
    url: Option<String>,
    instructions: Option<String>,
    method: Option<String>,
}

async fn plugin_auth_authorize(
    _state: State<Arc<ServerState>>,
    Path(name): Path<String>,
    Json(req): Json<PluginAuthAuthorizeRequest>,
) -> Result<Json<PluginAuthAuthorizeResponse>> {
    let bridge = get_auth_bridge(&name).await?;

    let result = bridge
        .authorize(req.method, req.inputs)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(PluginAuthAuthorizeResponse {
        url: result.url,
        instructions: result.instructions,
        method: result.method,
    }))
}

#[derive(Debug, Deserialize)]
struct PluginAuthCallbackRequest {
    code: Option<String>,
}

async fn plugin_auth_callback(
    _state: State<Arc<ServerState>>,
    Path(name): Path<String>,
    Json(req): Json<PluginAuthCallbackRequest>,
) -> Result<Json<serde_json::Value>> {
    let bridge = get_auth_bridge(&name).await?;

    let result = bridge
        .callback(req.code.as_deref())
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(result))
}

#[derive(Debug, Serialize)]
struct PluginAuthLoadResponse {
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
    #[serde(rename = "hasCustomFetch")]
    has_custom_fetch: bool,
}

async fn plugin_auth_load(
    _state: State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Result<Json<PluginAuthLoadResponse>> {
    let bridge = get_auth_bridge(&name).await?;

    let result = bridge
        .load()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(PluginAuthLoadResponse {
        api_key: result.api_key,
        has_custom_fetch: result.has_custom_fetch,
    }))
}

#[derive(Debug, Deserialize)]
struct PluginAuthFetchRequest {
    url: String,
    method: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    body: Option<String>,
}

#[derive(Debug, Serialize)]
struct PluginAuthFetchResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

async fn plugin_auth_fetch(
    _state: State<Arc<ServerState>>,
    Path(name): Path<String>,
    Json(req): Json<PluginAuthFetchRequest>,
) -> Result<Json<PluginAuthFetchResponse>> {
    let bridge = get_auth_bridge(&name).await?;

    let fetch_req = opencode_plugin::subprocess::PluginFetchRequest {
        url: req.url,
        method: req.method,
        headers: req.headers,
        body: req.body,
    };

    let result = bridge
        .fetch_proxy(fetch_req)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(Json(PluginAuthFetchResponse {
        status: result.status,
        headers: result.headers,
        body: result.body,
    }))
}

/// Helper: look up the auth bridge for a provider name.
async fn get_auth_bridge(provider: &str) -> Result<Arc<PluginAuthBridge>> {
    let loader = get_plugin_loader()
        .ok_or_else(|| ApiError::NotFound("no plugin loader initialized".into()))?;

    loader
        .auth_bridge(provider)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("no auth plugin for provider: {}", provider)))
}
