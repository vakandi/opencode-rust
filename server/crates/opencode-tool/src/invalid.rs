use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvalidParams {
    pub tool_name: String,
    pub error_message: String,
    pub received_args: Option<serde_json::Value>,
}

pub struct InvalidTool;

#[async_trait]
impl Tool for InvalidTool {
    fn id(&self) -> &str {
        "invalid"
    }

    fn description(&self) -> &str {
        "Handle invalid tool calls. This tool is used when an invalid or unknown tool is requested."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "toolName": {
                    "type": "string",
                    "description": "The name of the invalid tool that was requested"
                },
                "errorMessage": {
                    "type": "string",
                    "description": "Description of why the tool call is invalid"
                },
                "receivedArgs": {
                    "type": "object",
                    "description": "The arguments that were passed to the invalid tool"
                }
            },
            "required": ["toolName", "errorMessage"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let params: InvalidParams = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid parameters: {}", e)))?;

        let output = format!(
            "⚠️ Invalid Tool Call\n\nTool: {}\nError: {}\n\nPlease check the available tools and try again with a valid tool name.",
            params.tool_name,
            params.error_message
        );

        let mut metadata = Metadata::new();
        metadata.insert("tool_name".to_string(), serde_json::json!(params.tool_name));
        metadata.insert(
            "error_message".to_string(),
            serde_json::json!(params.error_message),
        );

        Ok(ToolResult {
            output,
            title: format!("Invalid Tool: {}", params.tool_name),
            metadata,
            truncated: false,
        })
    }
}
