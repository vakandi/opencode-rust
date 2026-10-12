use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Usage statistics for a single message
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessageUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub id: String,
    pub session_id: String,
    pub role: MessageRole,
    pub parts: Vec<MessagePart>,
    pub created_at: DateTime<Utc>,
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagePart {
    pub id: String,
    pub part_type: PartType,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PartType {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        synthetic: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ignored: Option<bool>,
    },
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        is_error: bool,
    },

    Reasoning {
        text: String,
    },
    File {
        url: String,
        filename: String,
        mime: String,
    },
    StepStart {
        id: String,
        name: String,
    },
    StepFinish {
        id: String,
        output: Option<String>,
    },
    Snapshot {
        content: String,
    },
    Patch {
        old_string: String,
        new_string: String,
        filepath: String,
    },
    Agent {
        name: String,
        status: String,
    },
    Subtask {
        id: String,
        description: String,
        status: String,
    },
    Retry {
        count: u32,
        reason: String,
    },
    Compaction {
        summary: String,
    },
}

impl SessionMessage {
    pub fn user(session_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: crate::identifier::message_id(),
            session_id: session_id.into(),
            role: MessageRole::User,
            parts: vec![MessagePart {
                id: crate::identifier::part_id(),
                part_type: PartType::Text { text: text.into(), synthetic: None, ignored: None },
                created_at: Utc::now(),
                message_id: None,
            }],
            created_at: Utc::now(),
            metadata: HashMap::new(),
            usage: None,
        }
    }

    pub fn assistant(session_id: impl Into<String>) -> Self {
        Self {
            id: crate::identifier::message_id(),
            session_id: session_id.into(),
            role: MessageRole::Assistant,
            parts: Vec::new(),
            created_at: Utc::now(),
            metadata: HashMap::new(),
            usage: None,
        }
    }

    pub fn add_text(&mut self, text: impl Into<String>) {
        self.parts.push(MessagePart {
            id: crate::identifier::part_id(),
            part_type: PartType::Text { text: text.into(), synthetic: None, ignored: None },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_reasoning(&mut self, text: impl Into<String>) {
        self.parts.push(MessagePart {
            id: crate::identifier::part_id(),
            part_type: PartType::Reasoning { text: text.into() },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_tool_call(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) {
        self.parts.push(MessagePart {
            id: crate::identifier::part_id(),
            part_type: PartType::ToolCall {
                id: id.into(),
                name: name.into(),
                input,
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_tool_result(
        &mut self,
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) {
        self.parts.push(MessagePart {
            id: crate::identifier::part_id(),
            part_type: PartType::ToolResult {
                tool_call_id: tool_call_id.into(),
                content: content.into(),
                is_error,
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_file(
        &mut self,
        url: impl Into<String>,
        filename: impl Into<String>,
        mime: impl Into<String>,
    ) {
        self.parts.push(MessagePart {
            id: crate::identifier::part_id(),
            part_type: PartType::File {
                url: url.into(),
                filename: filename.into(),
                mime: mime.into(),
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_agent(&mut self, name: impl Into<String>) {
        self.parts.push(MessagePart {
            id: crate::identifier::part_id(),
            part_type: PartType::Agent {
                name: name.into(),
                status: "pending".to_string(),
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn add_subtask(&mut self, id: impl Into<String>, description: impl Into<String>) {
        self.parts.push(MessagePart {
            id: crate::identifier::part_id(),
            part_type: PartType::Subtask {
                id: id.into(),
                description: description.into(),
                status: "pending".to_string(),
            },
            created_at: Utc::now(),
            message_id: None,
        });
    }

    pub fn get_text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match &p.part_type {
                PartType::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn get_reasoning(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match &p.part_type {
                PartType::Reasoning { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}
