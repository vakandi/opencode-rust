use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::ToolRegistry;

#[cfg(feature = "lsp")]
use opencode_lsp::LspClientRegistry;

pub type Metadata = HashMap<String, serde_json::Value>;

static FILE_LOCKS: std::sync::OnceLock<Arc<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>>> =
    std::sync::OnceLock::new();

fn get_file_locks() -> Arc<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>> {
    FILE_LOCKS
        .get_or_init(|| Arc::new(std::sync::Mutex::new(HashMap::new())))
        .clone()
}

pub async fn with_file_lock<F, Fut, T>(filepath: &str, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let lock = {
        let locks = get_file_locks();
        let mut locks_guard = locks.lock().unwrap();
        locks_guard
            .entry(filepath.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };

    let _guard = lock.lock().await;
    f().await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionDef {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multiple: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

pub type AskCallback = Arc<
    dyn (Fn(
            PermissionRequest,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type QuestionCallback = Arc<
    dyn (Fn(
            Vec<QuestionDef>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<Vec<String>>, ToolError>> + Send>,
        >) + Send
        + Sync,
>;

pub type SwitchAgentCallback = Arc<
    dyn (Fn(
            String,
            Option<String>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type CreateSubsessionCallback = Arc<
    dyn (Fn(
            String,
            Option<String>,
            Option<String>,
            Vec<String>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, ToolError>> + Send>>)
        + Send
        + Sync,
>;
pub type PromptSubsessionCallback = Arc<
    dyn (Fn(
            String,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type FileTimeAssertCallback = Arc<
    dyn (Fn(
            String,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type FileTimeReadCallback = Arc<
    dyn (Fn(
            String,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type PublishBusCallback = Arc<
    dyn (Fn(
            String,
            serde_json::Value,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>)
        + Send
        + Sync,
>;

pub type UpdatePartCallback = Arc<
    dyn (Fn(
            serde_json::Value,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type UpdateMessageCallback = Arc<
    dyn (Fn(
            serde_json::Value,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type LspTouchFileCallback = Arc<
    dyn (Fn(
            String,
            bool,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItemData {
    pub content: String,
    pub status: String,
    pub priority: String,
}

pub type TodoUpdateCallback = Arc<
    dyn (Fn(
            String,
            Vec<TodoItemData>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

pub type TodoGetCallback = Arc<
    dyn (Fn(
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<TodoItemData>, ToolError>> + Send>,
        >) + Send
        + Sync,
>;

pub type GetLastModelCallback = Arc<
    dyn (Fn(
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<String>, ToolError>> + Send>,
        >) + Send
        + Sync,
>;

pub type CreateSyntheticMessageCallback = Arc<
    dyn (Fn(
            String,
            Option<String>,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ToolError>> + Send>>)
        + Send
        + Sync,
>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub permission: String,
    pub patterns: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub always: Vec<String>,
}

impl PermissionRequest {
    pub fn new(permission: impl Into<String>) -> Self {
        Self {
            permission: permission.into(),
            patterns: Vec::new(),
            metadata: HashMap::new(),
            always: Vec::new(),
        }
    }

    pub fn with_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.patterns.push(pattern.into());
        self
    }

    pub fn with_patterns(mut self, patterns: Vec<String>) -> Self {
        self.patterns = patterns;
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    pub fn with_always(mut self, always: impl Into<String>) -> Self {
        self.always.push(always.into());
        self
    }

    pub fn always_allow(mut self) -> Self {
        self.always.push("*".to_string());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub title: String,
    pub output: String,
    pub metadata: Metadata,
    pub truncated: bool,
}

impl ToolResult {
    pub fn simple(title: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            output: output.into(),
            metadata: Metadata::new(),
            truncated: false,
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub message_id: String,
    pub agent: String,
    pub call_id: Option<String>,
    pub directory: String,
    pub worktree: String,
    pub abort: CancellationToken,
    pub extra: HashMap<String, serde_json::Value>,
    pub ask: Option<AskCallback>,
    pub permission: Option<opencode_permission::PermissionRuleset>,
    pub ask_question: Option<QuestionCallback>,
    pub switch_agent: Option<SwitchAgentCallback>,
    pub create_subsession: Option<CreateSubsessionCallback>,
    pub prompt_subsession: Option<PromptSubsessionCallback>,
    pub file_time_assert: Option<FileTimeAssertCallback>,
    pub file_time_read: Option<FileTimeReadCallback>,
    pub publish_bus: Option<PublishBusCallback>,
    pub update_part: Option<UpdatePartCallback>,
    pub update_message: Option<UpdateMessageCallback>,
    pub lsp_touch_file: Option<LspTouchFileCallback>,
    pub todo_update: Option<TodoUpdateCallback>,
    pub todo_get: Option<TodoGetCallback>,
    pub get_last_model: Option<GetLastModelCallback>,
    pub create_synthetic_message: Option<CreateSyntheticMessageCallback>,
    pub project_root: String,
    pub registry: Option<Arc<ToolRegistry>>,
    #[cfg(feature = "lsp")]
    pub lsp_registry: Option<Arc<LspClientRegistry>>,
}

impl ToolContext {
    pub fn new(session_id: String, message_id: String, directory: String) -> Self {
        Self {
            session_id,
            message_id,
            agent: String::new(),
            call_id: None,
            directory: directory.clone(),
            worktree: directory.clone(),
            abort: CancellationToken::new(),
            extra: HashMap::new(),
            ask: None,
            permission: None,
            ask_question: None,
            switch_agent: None,
            create_subsession: None,
            prompt_subsession: None,
            file_time_assert: None,
            file_time_read: None,
            publish_bus: None,
            update_part: None,
            update_message: None,
            lsp_touch_file: None,
            todo_update: None,
            todo_get: None,
            get_last_model: None,
            create_synthetic_message: None,
            project_root: directory,
            registry: None,
            #[cfg(feature = "lsp")]
            lsp_registry: None,
        }
    }

    pub fn with_agent(mut self, agent: String) -> Self {
        self.agent = agent;
        self
    }

    pub fn with_abort(mut self, abort: CancellationToken) -> Self {
        self.abort = abort;
        self
    }

    pub fn with_permission(mut self, ruleset: opencode_permission::PermissionRuleset) -> Self {
        self.permission = Some(ruleset);
        self
    }

    pub fn with_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    #[cfg(feature = "lsp")]
    pub fn with_lsp_registry(mut self, lsp_registry: Arc<LspClientRegistry>) -> Self {
        self.lsp_registry = Some(lsp_registry);
        self
    }

    pub fn with_ask<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(PermissionRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.ask = Some(Arc::new(move |req| Box::pin(callback(req))));
        self
    }

    pub async fn ask_permission(&self, request: PermissionRequest) -> Result<(), ToolError> {
        if let Some(ref ruleset) = self.permission {
            // A tool that asked nothing (no pattern) is judged on its name alone, which
            // is all bash can offer — its command text is not a path.
            let patterns: Vec<String> = if request.patterns.is_empty() {
                vec!["*".to_string()]
            } else {
                request.patterns.clone()
            };
            for pattern in &patterns {
                let rule = opencode_permission::evaluate(
                    &request.permission,
                    pattern,
                    std::slice::from_ref(ruleset),
                );
                match rule.action {
                    opencode_permission::PermissionAction::Allow => {}
                    opencode_permission::PermissionAction::Ask => {
                        return Err(ToolError::PermissionDenied(format!(
                            "'{}' on '{}' needs approval from the user",
                            request.permission, pattern
                        )))
                    }
                    opencode_permission::PermissionAction::Deny => {
                        return Err(ToolError::PermissionDenied(format!(
                            "'{}' on '{}' is denied by the agent's permission rules",
                            request.permission, pattern
                        )))
                    }
                }
            }
            return Ok(());
        }
        if let Some(ref callback) = self.ask {
            callback(request).await
        } else {
            Ok(())
        }
    }

    pub fn with_ask_question<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(Vec<QuestionDef>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<Vec<String>>, ToolError>> + Send + 'static,
    {
        self.ask_question = Some(Arc::new(move |questions| Box::pin(callback(questions))));
        self
    }

    pub async fn question(
        &self,
        questions: Vec<QuestionDef>,
    ) -> Result<Vec<Vec<String>>, ToolError> {
        if let Some(ref callback) = self.ask_question {
            callback(questions).await
        } else {
            Err(ToolError::ExecutionError(
                "Question callback not configured".to_string(),
            ))
        }
    }

    pub fn with_switch_agent<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, Option<String>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.switch_agent = Some(Arc::new(move |agent, model| {
            Box::pin(callback(agent, model))
        }));
        self
    }

    pub async fn do_switch_agent(
        &self,
        agent: String,
        model: Option<String>,
    ) -> Result<(), ToolError> {
        if let Some(ref callback) = self.switch_agent {
            callback(agent, model).await
        } else {
            Ok(())
        }
    }

    pub fn with_create_subsession<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, Option<String>, Option<String>, Vec<String>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, ToolError>> + Send + 'static,
    {
        self.create_subsession = Some(Arc::new(move |agent, title, model, disabled_tools| {
            Box::pin(callback(agent, title, model, disabled_tools))
        }));
        self
    }

    pub async fn do_create_subsession(
        &self,
        agent: String,
        title: Option<String>,
        model: Option<String>,
        disabled_tools: Vec<String>,
    ) -> Result<String, ToolError> {
        if let Some(ref callback) = self.create_subsession {
            callback(agent, title, model, disabled_tools).await
        } else {
            Ok(format!("task_{}_{}", agent, uuid::Uuid::new_v4()))
        }
    }

    pub fn with_prompt_subsession<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, ToolError>> + Send + 'static,
    {
        self.prompt_subsession = Some(Arc::new(move |session_id, prompt| {
            Box::pin(callback(session_id, prompt))
        }));
        self
    }

    pub async fn do_prompt_subsession(
        &self,
        session_id: String,
        prompt: String,
    ) -> Result<String, ToolError> {
        if let Some(ref callback) = self.prompt_subsession {
            callback(session_id, prompt).await
        } else {
            Err(ToolError::ExecutionError(
                "Subsession prompt callback not configured".to_string(),
            ))
        }
    }

    pub fn with_file_time_assert<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.file_time_assert = Some(Arc::new(move |session_id, file_path| {
            Box::pin(callback(session_id, file_path))
        }));
        self
    }

    pub async fn do_file_time_assert(&self, file_path: String) -> Result<(), ToolError> {
        if let Some(ref callback) = self.file_time_assert {
            callback(self.session_id.clone(), file_path).await
        } else {
            Ok(())
        }
    }

    pub fn with_file_time_read<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.file_time_read = Some(Arc::new(move |session_id, file_path| {
            Box::pin(callback(session_id, file_path))
        }));
        self
    }

    pub async fn do_file_time_read(&self, file_path: String) -> Result<(), ToolError> {
        if let Some(ref callback) = self.file_time_read {
            callback(self.session_id.clone(), file_path).await
        } else {
            Ok(())
        }
    }

    pub fn with_publish_bus<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.publish_bus = Some(Arc::new(move |event_type, properties| {
            Box::pin(callback(event_type, properties))
        }));
        self
    }

    pub async fn do_publish_bus(&self, event_type: &str, properties: serde_json::Value) {
        if let Some(ref callback) = self.publish_bus {
            callback(event_type.to_string(), properties).await;
        }
    }

    pub fn with_update_part<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.update_part = Some(Arc::new(move |part| Box::pin(callback(part))));
        self
    }

    pub async fn do_update_part(&self, part: serde_json::Value) -> Result<(), ToolError> {
        if let Some(ref callback) = self.update_part {
            callback(part).await
        } else {
            Ok(())
        }
    }

    pub fn with_update_message<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.update_message = Some(Arc::new(move |msg| Box::pin(callback(msg))));
        self
    }

    pub async fn do_update_message(&self, msg: serde_json::Value) -> Result<(), ToolError> {
        if let Some(ref callback) = self.update_message {
            callback(msg).await
        } else {
            Ok(())
        }
    }

    pub fn with_lsp_touch_file<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, bool) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.lsp_touch_file = Some(Arc::new(move |file_path, write| {
            Box::pin(callback(file_path, write))
        }));
        self
    }

    pub async fn do_lsp_touch_file(&self, file_path: String, write: bool) -> Result<(), ToolError> {
        if let Some(ref callback) = self.lsp_touch_file {
            callback(file_path, write).await
        } else {
            Ok(())
        }
    }

    pub fn with_todo_update<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, Vec<TodoItemData>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.todo_update = Some(Arc::new(move |session_id, todos| {
            Box::pin(callback(session_id, todos))
        }));
        self
    }

    pub async fn do_todo_update(&self, todos: Vec<TodoItemData>) -> Result<(), ToolError> {
        if let Some(ref callback) = self.todo_update {
            return callback(self.session_id.clone(), todos).await;
        }
        fallback_todo_write(&self.session_id, todos);
        Ok(())
    }

    pub fn with_todo_get<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<TodoItemData>, ToolError>> + Send + 'static,
    {
        self.todo_get = Some(Arc::new(move |session_id| Box::pin(callback(session_id))));
        self
    }

    pub async fn do_todo_get(&self) -> Result<Vec<TodoItemData>, ToolError> {
        if let Some(ref callback) = self.todo_get {
            return callback(self.session_id.clone()).await;
        }
        Ok(fallback_todo_read(&self.session_id))
    }

    pub fn with_get_last_model<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Option<String>, ToolError>> + Send + 'static,
    {
        self.get_last_model = Some(Arc::new(move |session_id| Box::pin(callback(session_id))));
        self
    }

    pub async fn do_get_last_model(&self) -> Option<String> {
        if let Some(ref callback) = self.get_last_model {
            callback(self.session_id.clone()).await.ok().flatten()
        } else {
            None
        }
    }

    pub fn with_create_synthetic_message<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String, Option<String>, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), ToolError>> + Send + 'static,
    {
        self.create_synthetic_message = Some(Arc::new(move |session_id, agent, text| {
            Box::pin(callback(session_id, agent, text))
        }));
        self
    }

    pub async fn do_create_synthetic_message(
        &self,
        agent: Option<String>,
        text: String,
    ) -> Result<(), ToolError> {
        if let Some(ref callback) = self.create_synthetic_message {
            callback(self.session_id.clone(), agent, text).await
        } else {
            Ok(())
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.abort.is_cancelled()
    }

    pub fn is_external_path(&self, path: &str) -> bool {
        let abs_path = if std::path::Path::new(path).is_absolute() {
            path.to_string()
        } else {
            format!("{}/{}", self.directory, path)
        };
        !abs_path.starts_with(&self.project_root)
    }
}

static FALLBACK_TODO_STORE: std::sync::OnceLock<
    Arc<std::sync::Mutex<HashMap<String, Vec<TodoItemData>>>>,
> = std::sync::OnceLock::new();

type FallbackTodoMap = Arc<std::sync::Mutex<HashMap<String, Vec<TodoItemData>>>>;

fn fallback_todo_store() -> &'static FallbackTodoMap {
    FALLBACK_TODO_STORE.get_or_init(|| Arc::new(std::sync::Mutex::new(HashMap::new())))
}

fn fallback_todo_write(session_id: &str, todos: Vec<TodoItemData>) {
    let Ok(mut guard) = fallback_todo_store().lock() else {
        return;
    };
    if todos.is_empty() {
        guard.remove(session_id);
    } else {
        guard.insert(session_id.to_string(), todos);
    }
}

fn fallback_todo_read(session_id: &str) -> Vec<TodoItemData> {
    let Ok(guard) = fallback_todo_store().lock() else {
        return Vec::new();
    };
    guard.get(session_id).cloned().unwrap_or_default()
}

/// Reads a session's todos from the process-wide store that backs
/// `do_todo_update` when no `with_todo_update` callback is installed.
///
/// Every real dispatch path builds its `ToolContext` without those callbacks,
/// which is why the fallback exists at all: without it `do_todo_update`
/// discarded the list, `do_todo_get` answered empty, and
/// `GET /session/{id}/todo` could only ever return `[]`. Out-of-crate callers
/// read the same store through this function so there is one source of truth
/// rather than a second copy.
pub fn fallback_todos(session_id: &str) -> Vec<TodoItemData> {
    fallback_todo_read(session_id)
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("session_id", &self.session_id)
            .field("message_id", &self.message_id)
            .field("agent", &self.agent)
            .field("directory", &self.directory)
            .field("worktree", &self.worktree)
            .finish()
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn id(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError>;

    fn validate(&self, args: &serde_json::Value) -> Result<(), ToolError> {
        let _ = args;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Execution error: {0}")]
    ExecutionError(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Question rejected: {0}")]
    QuestionRejected(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Binary file: {0}")]
    BinaryFile(String),

    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Cancelled")]
    Cancelled,
}

impl ToolError {
    pub fn with_suggestions(msg: impl Into<String>, suggestions: &[String]) -> Self {
        let msg = msg.into();
        if suggestions.is_empty() {
            ToolError::FileNotFound(msg)
        } else {
            ToolError::FileNotFound(format!(
                "{}\n\nDid you mean one of these?\n{}",
                msg,
                suggestions.join("\n")
            ))
        }
    }
}

#[cfg(test)]
mod fallback_todo_tests {
    use super::*;

    fn ctx(session_id: &str) -> ToolContext {
        ToolContext::new(session_id.to_string(), "message-1".to_string(), ".".to_string())
    }

    fn item(content: &str, status: &str) -> TodoItemData {
        TodoItemData {
            content: content.to_string(),
            status: status.to_string(),
            priority: "medium".to_string(),
        }
    }

    /// A dispatch-shaped ToolContext installs no todo callbacks, so before the
    /// fallback this was a silent no-op and read back empty.
    #[tokio::test]
    async fn write_then_read_round_trips_without_callbacks() {
        let id = format!("roundtrip-{}", uuid::Uuid::new_v4());
        let c = ctx(&id);

        c.do_todo_update(vec![item("first", "pending"), item("second", "completed")])
            .await
            .expect("update must succeed");

        let read = c.do_todo_get().await.expect("get must succeed");
        assert_eq!(read.len(), 2, "expected both items, got {:?}", read);
        assert_eq!(read[0].content, "first");
        assert_eq!(read[1].status, "completed");
    }

    /// The out-of-crate accessor must observe the same data as do_todo_get, or
    /// the HTTP route would need a second store.
    #[tokio::test]
    async fn fallback_todos_sees_what_do_todo_update_wrote() {
        let id = format!("shared-{}", uuid::Uuid::new_v4());
        ctx(&id)
            .do_todo_update(vec![item("visible", "in_progress")])
            .await
            .expect("update must succeed");

        let via_accessor = fallback_todos(&id);
        assert_eq!(via_accessor.len(), 1);
        assert_eq!(via_accessor[0].content, "visible");
        assert_eq!(via_accessor[0].status, "in_progress");
    }

    #[tokio::test]
    async fn empty_write_clears_and_sessions_stay_isolated() {
        let a = format!("iso-a-{}", uuid::Uuid::new_v4());
        let b = format!("iso-b-{}", uuid::Uuid::new_v4());

        ctx(&a)
            .do_todo_update(vec![item("mine", "pending")])
            .await
            .expect("update must succeed");

        assert!(
            ctx(&b).do_todo_get().await.expect("get").is_empty(),
            "an untouched session must read empty"
        );
        assert!(
            fallback_todos(&format!("unknown-{}", uuid::Uuid::new_v4()))
                .is_empty(),
            "an unknown session must read empty rather than fail"
        );

        ctx(&a).do_todo_update(Vec::new()).await.expect("clear");
        assert!(
            ctx(&a).do_todo_get().await.expect("get").is_empty(),
            "an empty write must clear the list"
        );
    }

    /// An installed callback must still win over the fallback, so this change
    /// does not hijack a host that deliberately owns its own store.
    #[tokio::test]
    async fn installed_callback_takes_precedence_over_the_fallback() {
        let id = format!("callback-{}", uuid::Uuid::new_v4());
        let c = ctx(&id).with_todo_update(|_session_id, _todos| async move { Ok(()) });
        c.do_todo_update(vec![item("routed", "pending")])
            .await
            .expect("update must succeed");
        assert!(
            fallback_todos(&id).is_empty(),
            "the callback path must not also write the fallback store"
        );
    }
}
