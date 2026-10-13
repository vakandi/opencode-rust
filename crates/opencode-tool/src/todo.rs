use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{TodoItemData, Tool, ToolContext, ToolError, ToolResult};

pub struct TodoReadTool;

pub struct TodoWriteTool;

#[derive(Debug, Serialize, Deserialize)]
struct TodoReadInput {
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TodoWriteInput {
    todos: Vec<TodoWriteItem>,
    session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TodoWriteItem {
    id: Option<String>,
    content: String,
    status: Option<String>,
    priority: Option<String>,
}

#[async_trait]
impl Tool for TodoReadTool {
    fn id(&self) -> &str {
        "todoread"
    }

    fn description(&self) -> &str {
        "Read the current todo list for the session. Returns all todo items with their status."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Optional session ID. If not provided, uses current session."
                }
            }
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: TodoReadInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        let session_id = input
            .session_id
            .clone()
            .unwrap_or_else(|| ctx.session_id.clone());

        ctx.ask_permission(
            crate::PermissionRequest::new("todoread")
                .with_metadata("session_id", serde_json::json!(&session_id))
                .always_allow(),
        )
        .await?;

        let todos = ctx.do_todo_get().await?;

        let output = format_todos_from_data(&todos);

        let todos_json: Vec<serde_json::Value> = todos
            .iter()
            .map(|t| {
                serde_json::json!({
                    "content": t.content,
                    "status": t.status,
                    "priority": t.priority
                })
            })
            .collect();

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("todos".to_string(), serde_json::json!(todos_json));
        metadata.insert("count".to_string(), serde_json::json!(todos.len()));

        Ok(ToolResult {
            title: format!("Todo List ({} items)", todos.len()),
            output,
            metadata,
            truncated: false,
        })
    }
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn id(&self) -> &str {
        "todowrite"
    }

    fn description(&self) -> &str {
        "Create or update the todo list for the session. Use this to track tasks and their progress."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "content": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] },
                            "priority": { "type": "string" }
                        },
                        "required": ["content"]
                    },
                    "description": "List of todo items"
                },
                "session_id": {
                    "type": "string",
                    "description": "Optional session ID"
                }
            },
            "required": ["todos"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: TodoWriteInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        let session_id = input
            .session_id
            .clone()
            .unwrap_or_else(|| ctx.session_id.clone());

        ctx.ask_permission(
            crate::PermissionRequest::new("todowrite")
                .with_metadata("session_id", serde_json::json!(&session_id))
                .with_metadata("count", serde_json::json!(input.todos.len()))
                .always_allow(),
        )
        .await?;

        let mut new_todos: Vec<TodoItemData> = Vec::new();

        for item in input.todos {
            let status = match item.status.as_deref() {
                Some("in_progress") => "in_progress".to_string(),
                Some("completed") => "completed".to_string(),
                _ => "pending".to_string(),
            };

            let _id = item.id.unwrap_or_else(|| {
                format!("todo_{}", uuid::Uuid::new_v4().to_string()[..8].to_string())
            });

            new_todos.push(TodoItemData {
                content: item.content,
                status,
                priority: item.priority.unwrap_or_else(|| "medium".to_string()),
            });
        }

        ctx.do_todo_update(new_todos.clone()).await?;

        let output = format_todos_from_data(&new_todos);

        let todos_json: Vec<serde_json::Value> = new_todos
            .iter()
            .map(|t| {
                serde_json::json!({
                    "content": t.content,
                    "status": t.status,
                    "priority": t.priority
                })
            })
            .collect();

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("todos".to_string(), serde_json::json!(todos_json));
        metadata.insert("count".to_string(), serde_json::json!(new_todos.len()));

        Ok(ToolResult {
            title: format!("Updated Todo List ({} items)", new_todos.len()),
            output,
            metadata,
            truncated: false,
        })
    }
}

fn format_todos_from_data(todos: &[TodoItemData]) -> String {
    if todos.is_empty() {
        return "No todos in the list.".to_string();
    }

    let mut output = String::new();
    output.push_str("# Todo List\n\n");

    for (i, todo) in todos.iter().enumerate() {
        let status_icon = match todo.status.as_str() {
            "in_progress" => "🔄",
            "completed" => "✅",
            _ => "⬜",
        };

        let priority_str = if !todo.priority.is_empty() {
            format!(" [{}]", todo.priority)
        } else {
            String::new()
        };

        output.push_str(&format!(
            "{} todo_{}{}\n   {}\n\n",
            status_icon, i, priority_str, todo.content
        ));
    }

    let pending = todos.iter().filter(|t| t.status == "pending").count();
    let in_progress = todos.iter().filter(|t| t.status == "in_progress").count();
    let completed = todos.iter().filter(|t| t.status == "completed").count();

    output.push_str(&format!(
        "Summary: {} pending, {} in progress, {} completed",
        pending, in_progress, completed
    ));

    output
}

impl Default for TodoReadTool {
    fn default() -> Self {
        Self
    }
}

impl Default for TodoWriteTool {
    fn default() -> Self {
        Self
    }
}
