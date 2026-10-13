use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub role: MessageRole,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

impl AgentMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            tool_calls: Vec::new(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            tool_calls: Vec::new(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            tool_calls: Vec::new(),
        }
    }

    pub fn assistant_with_tools(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            tool_calls,
        }
    }

    pub fn tool_result(
        _tool_call_id: impl Into<String>,
        _name: impl Into<String>,
        content: impl Into<String>,
        _is_error: bool,
    ) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            tool_calls: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub messages: Vec<AgentMessage>,
}

impl Conversation {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub fn with_system_prompt(prompt: impl Into<String>) -> Self {
        let mut conv = Self::new();
        conv.messages.push(AgentMessage::system(prompt));
        conv
    }

    pub fn add_user_message(&mut self, content: impl Into<String>) {
        self.messages.push(AgentMessage::user(content));
    }

    pub fn add_assistant_message(&mut self, content: impl Into<String>) {
        self.messages.push(AgentMessage::assistant(content));
    }

    pub fn add_tool_result(
        &mut self,
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) {
        self.messages.push(AgentMessage::tool_result(
            tool_call_id,
            name,
            content,
            is_error,
        ));
    }

    pub fn to_provider_messages(&self) -> Vec<opencode_provider::Message> {
        self.messages
            .iter()
            .map(|m| match m.role {
                MessageRole::System => opencode_provider::Message::system(&m.content),
                MessageRole::User => opencode_provider::Message::user(&m.content),
                MessageRole::Assistant => opencode_provider::Message::assistant(&m.content),
                MessageRole::Tool => opencode_provider::Message::assistant(&m.content),
            })
            .collect()
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}
