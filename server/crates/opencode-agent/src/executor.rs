use futures::stream::BoxStream;
use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{AgentInfo, Conversation, ToolCall};
use opencode_plugin::{HookContext, HookEvent};
use opencode_provider::{ChatRequest, Provider, ProviderRegistry, StreamEvent};
use opencode_tool::{ToolContext, ToolError, ToolRegistry};

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Tool error: {0}")]
    ToolError(String),

    #[error("Max steps exceeded")]
    MaxStepsExceeded,

    #[error("No provider available")]
    NoProvider,

    #[error("Invalid response")]
    InvalidResponse,
}

pub struct AgentExecutor {
    agent: AgentInfo,
    conversation: Conversation,
    providers: Arc<ProviderRegistry>,
    tools: Arc<ToolRegistry>,
    disabled_tools: HashSet<String>,
    subsessions: Arc<Mutex<HashMap<String, SubsessionState>>>,
    max_steps: u32,
}

#[derive(Debug, Clone)]
struct SubsessionState {
    agent: AgentInfo,
    conversation: Conversation,
    disabled_tools: HashSet<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedSubsessionState {
    pub agent: AgentInfo,
    pub conversation: Conversation,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}

impl AgentExecutor {
    pub fn new(
        agent: AgentInfo,
        providers: Arc<ProviderRegistry>,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        let max_steps = agent.max_steps.unwrap_or(100);
        let conversation = Conversation::new();

        Self {
            agent,
            conversation,
            providers,
            tools,
            disabled_tools: HashSet::new(),
            subsessions: Arc::new(Mutex::new(HashMap::new())),
            max_steps,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.conversation = Conversation::with_system_prompt(prompt);
        self
    }

    pub fn with_disabled_tools<I>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        self.disabled_tools = tools.into_iter().collect();
        self
    }

    pub fn with_persisted_subsessions(
        mut self,
        states: HashMap<String, PersistedSubsessionState>,
    ) -> Self {
        let subsessions = states
            .into_iter()
            .map(|(id, state)| {
                (
                    id,
                    SubsessionState {
                        agent: state.agent,
                        conversation: state.conversation,
                        disabled_tools: state.disabled_tools.into_iter().collect(),
                    },
                )
            })
            .collect();
        self.subsessions = Arc::new(Mutex::new(subsessions));
        self
    }

    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    pub fn conversation_mut(&mut self) -> &mut Conversation {
        &mut self.conversation
    }

    pub async fn export_subsessions(&self) -> HashMap<String, PersistedSubsessionState> {
        self.subsessions
            .lock()
            .await
            .iter()
            .map(|(id, state)| {
                (
                    id.clone(),
                    PersistedSubsessionState {
                        agent: state.agent.clone(),
                        conversation: state.conversation.clone(),
                        disabled_tools: state.disabled_tools.iter().cloned().collect(),
                    },
                )
            })
            .collect()
    }

    pub async fn execute(&mut self, user_message: impl Into<String>) -> Result<String, AgentError> {
        self.conversation.add_user_message(user_message);

        // Plugin hook: session.start
        opencode_plugin::trigger(
            HookContext::new(HookEvent::SessionStart)
                .with_data("agent", serde_json::json!(&self.agent.name))
                .with_data("max_steps", serde_json::json!(self.max_steps)),
        )
        .await;

        let mut steps = 0;
        let mut final_response = String::new();

        while steps < self.max_steps {
            steps += 1;

            let provider = self.get_provider()?;
            let model_id = self.get_model_id(&provider);

            // Plugin hook: chat.system.transform — let plugins modify system prompt per step
            opencode_plugin::trigger(
                HookContext::new(HookEvent::ChatSystemTransform)
                    .with_data("agent", serde_json::json!(&self.agent.name))
                    .with_data("model_id", serde_json::json!(&model_id))
                    .with_data("step", serde_json::json!(steps)),
            )
            .await;

            let request = ChatRequest::new(model_id, self.conversation.to_provider_messages());

            let stream = provider
                .chat_stream(request)
                .await
                .map_err(|e| AgentError::ProviderError(e.to_string()))?;

            let (response, tool_calls) = self.process_stream(stream).await?;

            if tool_calls.is_empty() {
                final_response = response;
                break;
            }

            self.conversation.add_assistant_message(&response);

            for tool_call in tool_calls {
                let result = self.execute_tool(&tool_call).await;

                let (content, is_error) = match result {
                    Ok(output) => (output, false),
                    Err(e) => (e.to_string(), true),
                };

                self.conversation.add_tool_result(
                    &tool_call.id,
                    &tool_call.name,
                    content,
                    is_error,
                );
            }
        }

        // Plugin hook: session.end
        opencode_plugin::trigger(
            HookContext::new(HookEvent::SessionEnd)
                .with_data("agent", serde_json::json!(&self.agent.name))
                .with_data("steps", serde_json::json!(steps)),
        )
        .await;

        if steps >= self.max_steps {
            return Err(AgentError::MaxStepsExceeded);
        }

        Ok(final_response)
    }

    async fn execute_subsession(
        &mut self,
        user_message: impl Into<String>,
    ) -> Result<String, AgentError> {
        self.conversation.add_user_message(user_message);

        let mut steps = 0;
        let mut final_response = String::new();

        while steps < self.max_steps {
            steps += 1;

            let provider = self.get_provider()?;
            let model_id = self.get_model_id(&provider);
            let request = ChatRequest::new(model_id, self.conversation.to_provider_messages());

            let stream = provider
                .chat_stream(request)
                .await
                .map_err(|e| AgentError::ProviderError(e.to_string()))?;

            let (response, tool_calls) = self.process_stream(stream).await?;

            if tool_calls.is_empty() {
                final_response = response;
                break;
            }

            self.conversation.add_assistant_message(&response);

            for tool_call in tool_calls {
                let result = self.execute_tool_without_subsessions(&tool_call).await;

                let (content, is_error) = match result {
                    Ok(output) => (output, false),
                    Err(e) => (e.to_string(), true),
                };

                self.conversation.add_tool_result(
                    &tool_call.id,
                    &tool_call.name,
                    content,
                    is_error,
                );
            }
        }

        if steps >= self.max_steps {
            return Err(AgentError::MaxStepsExceeded);
        }

        Ok(final_response)
    }

    pub async fn execute_streaming(
        &mut self,
        user_message: String,
    ) -> Result<BoxStream<'static, Result<StreamEvent, AgentError>>, AgentError> {
        self.conversation.add_user_message(user_message);

        let provider = self.get_provider()?;
        let model_id = self.get_model_id(&provider);
        let request = ChatRequest::new(model_id, self.conversation.to_provider_messages());

        let stream = provider
            .chat_stream(request)
            .await
            .map_err(|e| AgentError::ProviderError(e.to_string()))?;

        Ok(stream
            .map(|r| r.map_err(|e| AgentError::ProviderError(e.to_string())))
            .boxed())
    }

    fn get_provider(&self) -> Result<Arc<dyn Provider>, AgentError> {
        if let Some(ref model_ref) = self.agent.model {
            self.providers
                .get(&model_ref.provider_id)
                .ok_or(AgentError::NoProvider)
        } else {
            let providers = self.providers.list();
            if providers.is_empty() {
                return Err(AgentError::NoProvider);
            }
            Ok(providers.into_iter().next().unwrap())
        }
    }

    fn get_model_id(&self, provider: &Arc<dyn Provider>) -> String {
        if let Some(ref model_ref) = self.agent.model {
            model_ref.model_id.clone()
        } else {
            let models = provider.models();
            models.first().map(|m| m.id.clone()).unwrap_or_default()
        }
    }

    async fn process_stream(
        &mut self,
        mut stream: opencode_provider::StreamResult,
    ) -> Result<(String, Vec<ToolCall>), AgentError> {
        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta(text)) => {
                    content.push_str(&text);
                }
                Ok(StreamEvent::ToolCallStart { id, name }) => {
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: serde_json::Value::Null,
                    });
                }
                Ok(StreamEvent::ToolCallDelta { id, input }) => {
                    if let Some(tc) = tool_calls.iter_mut().find(|t| t.id == id) {
                        if tc.arguments.is_null() {
                            tc.arguments =
                                serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
                        }
                    }
                }
                Ok(StreamEvent::Done) => break,
                Ok(StreamEvent::Error(e)) => {
                    return Err(AgentError::ProviderError(e));
                }
                Err(e) => {
                    return Err(AgentError::ProviderError(e.to_string()));
                }
                _ => {}
            }
        }

        Ok((content, tool_calls))
    }

    async fn execute_tool(&self, tool_call: &ToolCall) -> Result<String, ToolError> {
        if self.disabled_tools.contains(&tool_call.name) {
            return Err(ToolError::PermissionDenied(format!(
                "Tool '{}' is disabled for this subagent session",
                tool_call.name
            )));
        }
        self.ensure_tool_allowed_at(&tool_call.name, Some(&tool_call.arguments))?;

        let directory = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let current_model = self.current_model_string();
        let base_ctx = ToolContext::new("default".to_string(), "default".to_string(), directory)
            .with_agent(self.agent.name.clone())
            .with_get_last_model({
                let current_model = current_model.clone();
                move |_session_id| {
                    let current_model = current_model.clone();
                    async move { Ok(current_model) }
                }
            });
        let ctx = self.with_subsession_callbacks(base_ctx);

        self.tools
            .execute(&tool_call.name, tool_call.arguments.clone(), ctx)
            .await
            .map(|r| r.output)
    }

    async fn execute_tool_without_subsessions(
        &self,
        tool_call: &ToolCall,
    ) -> Result<String, ToolError> {
        if self.disabled_tools.contains(&tool_call.name) {
            return Err(ToolError::PermissionDenied(format!(
                "Tool '{}' is disabled for this subagent session",
                tool_call.name
            )));
        }
        self.ensure_tool_allowed_at(&tool_call.name, Some(&tool_call.arguments))?;

        let directory = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let current_model = self.current_model_string();
        let base_ctx = ToolContext::new("default".to_string(), "default".to_string(), directory)
            .with_agent(self.agent.name.clone())
            .with_get_last_model({
                let current_model = current_model.clone();
                move |_session_id| {
                    let current_model = current_model.clone();
                    async move { Ok(current_model) }
                }
            });
        let ctx = self.with_subsession_callbacks(base_ctx);

        self.tools
            .execute(&tool_call.name, tool_call.arguments.clone(), ctx)
            .await
            .map(|r| r.output)
    }

    fn with_subsession_callbacks(&self, ctx: ToolContext) -> ToolContext {
        let subsessions = self.subsessions.clone();
        let providers = self.providers.clone();
        let tools = self.tools.clone();

        ctx.with_create_subsession({
            let subsessions = subsessions.clone();
            move |agent_name, _title, model, disabled_tools| {
                let subsessions = subsessions.clone();
                async move {
                    let cwd = std::env::current_dir().unwrap_or_default();
                    let registry = crate::AgentRegistry::from_project_dir(&cwd);
                    let mut agent = registry.get(&agent_name).cloned().ok_or_else(|| {
                        ToolError::InvalidArguments(format!(
                            "Unknown agent type: {} is not a valid agent type",
                            agent_name
                        ))
                    })?;

                    if let Some((provider_id, model_id)) = parse_model_string(model.as_deref()) {
                        agent = agent.with_model(model_id, provider_id);
                    }

                    let conversation = if let Some(system_prompt) = &agent.system_prompt {
                        Conversation::with_system_prompt(system_prompt.clone())
                    } else {
                        Conversation::new()
                    };

                    let session_id =
                        format!("task_{}_{}", agent_name, uuid::Uuid::new_v4().simple());
                    let mut store = subsessions.lock().await;
                    store.insert(
                        session_id.clone(),
                        SubsessionState {
                            agent,
                            conversation,
                            disabled_tools: disabled_tools.into_iter().collect(),
                        },
                    );
                    Ok(session_id)
                }
            }
        })
        .with_prompt_subsession({
            let subsessions = subsessions.clone();
            let providers = providers.clone();
            let tools = tools.clone();
            move |session_id, prompt| {
                let subsessions = subsessions.clone();
                let providers = providers.clone();
                let tools = tools.clone();
                async move {
                    let state = {
                        let store = subsessions.lock().await;
                        store.get(&session_id).cloned()
                    }
                    .ok_or_else(|| {
                        ToolError::ExecutionError(format!(
                            "Unknown subagent session: {}. Start without task_id first.",
                            session_id
                        ))
                    })?;

                    let mut executor =
                        AgentExecutor::new(state.agent, providers.clone(), tools.clone())
                            .with_disabled_tools(state.disabled_tools.iter().cloned());
                    executor.conversation = state.conversation;

                    let output = executor.execute_subsession(prompt).await.map_err(|e| {
                        ToolError::ExecutionError(format!("Subagent execution failed: {}", e))
                    })?;

                    let mut store = subsessions.lock().await;
                    if let Some(state) = store.get_mut(&session_id) {
                        state.conversation = executor.conversation.clone();
                    }

                    Ok(output)
                }
            }
        })
    }

    fn current_model_string(&self) -> Option<String> {
        if let Some(model) = self.agent.model.as_ref() {
            return Some(format!("{}:{}", model.provider_id, model.model_id));
        }

        let provider = self.get_provider().ok()?;
        let model_id = self.get_model_id(&provider);
        if model_id.is_empty() {
            return None;
        }
        Some(format!("{}:{}", provider.id(), model_id))
    }

    fn ensure_tool_allowed(&self, tool_name: &str) -> Result<(), ToolError> {
        self.ensure_tool_allowed_at(tool_name, None)
    }

/// Gate a tool call against the path it names. Pathless tools fall back to `"*"`.
fn ensure_tool_allowed_at(
        &self,
        tool_name: &str,
        arguments: Option<&serde_json::Value>,
    ) -> Result<(), ToolError> {
        let path = arguments.and_then(path_from_arguments).unwrap_or_else(|| "*".to_string());
        match self.agent.tool_permission_decision_at(tool_name, &path) {
            crate::PermissionDecision::Allow => Ok(()),
            crate::PermissionDecision::Ask => Err(ToolError::PermissionDenied(format!(
                "Tool '{}' on '{}' requires explicit approval for agent '{}'",
                tool_name, path, self.agent.name
            ))),
            crate::PermissionDecision::Deny => Err(ToolError::PermissionDenied(format!(
                "Tool '{}' on '{}' is denied by agent '{}' permission rules",
                tool_name, path, self.agent.name
            ))),
        }
    }
}

/// Pull the path a tool call operates on out of its arguments.
///
/// Only the keys the file tools actually use are read. A bash call is judged on the
/// tool alone on purpose: its command text is not a path, and pretending otherwise
/// would let `cat /etc/passwd` slip through a rule scoped to the workspace.
fn path_from_arguments(arguments: &serde_json::Value) -> Option<String> {
    const PATH_KEYS: [&str; 6] = ["file_path", "path", "filePath", "directory", "dir", "target"];
    arguments
        .as_object()
        .and_then(|obj| {
            PATH_KEYS
                .iter()
                .find_map(|key| obj.get(*key).and_then(serde_json::Value::as_str))
        })
        .map(str::to_string)
        .filter(|raw| !raw.trim().is_empty())
}

fn parse_model_string(raw: Option<&str>) -> Option<(String, String)> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }

    let (provider, model) = raw.split_once(':').or_else(|| raw.split_once('/'))?;

    if provider.is_empty() || model.is_empty() {
        return None;
    }

    Some((provider.to_string(), model.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencode_permission::{PermissionAction, PermissionRule};

    fn build_executor(agent: AgentInfo) -> AgentExecutor {
        AgentExecutor::new(
            agent,
            Arc::new(ProviderRegistry::new()),
            Arc::new(ToolRegistry::new()),
        )
    }

    #[tokio::test]
    async fn persisted_subsessions_roundtrip() {
        let mut conversation = Conversation::with_system_prompt("subagent prompt");
        conversation.add_user_message("inspect project");
        conversation.add_assistant_message("working on it");

        let mut persisted = HashMap::new();
        persisted.insert(
            "task_explore_1".to_string(),
            PersistedSubsessionState {
                agent: AgentInfo::explore().with_model("gpt-4.1-mini", "openai"),
                conversation: conversation.clone(),
                disabled_tools: vec!["write".to_string(), "edit".to_string()],
            },
        );

        let executor = build_executor(AgentInfo::general()).with_persisted_subsessions(persisted);
        let exported = executor.export_subsessions().await;
        let state = exported
            .get("task_explore_1")
            .expect("expected persisted subsession");

        assert_eq!(state.agent.name, "explore");
        assert_eq!(
            state.conversation.messages.len(),
            conversation.messages.len()
        );

        let mut disabled = state.disabled_tools.clone();
        disabled.sort();
        assert_eq!(disabled, vec!["edit".to_string(), "write".to_string()]);
    }

    #[test]
    fn executor_enforces_explore_allowlist() {
        let executor = build_executor(AgentInfo::explore());

        assert!(executor.ensure_tool_allowed("grep").is_ok());

        let denied = executor
            .ensure_tool_allowed("write")
            .expect_err("write should be denied for explore");
        assert!(
            matches!(denied, ToolError::PermissionDenied(_)),
            "expected permission denied, got: {denied}"
        );
    }

    #[test]
    fn executor_blocks_ask_permissions_without_user_approval() {
        let agent = AgentInfo::custom("review").with_permission(vec![PermissionRule {
            permission: "bash".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Ask,
        }]);
        let executor = build_executor(agent);

        let denied = executor
            .ensure_tool_allowed("bash")
            .expect_err("ask should block direct execution");
        assert!(
            matches!(denied, ToolError::PermissionDenied(_)),
            "expected permission denied, got: {denied}"
        );
    }
}
