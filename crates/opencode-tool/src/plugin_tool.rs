use std::sync::Arc;

use async_trait::async_trait;
use opencode_plugin::plugin_tools::LoadedPluginTool;
use opencode_plugin::subprocess::client::PluginSubprocess;

use crate::tool::{Metadata, Tool, ToolContext, ToolError, ToolResult};

pub struct PluginTool {
    name: String,
    description: String,
    parameters: serde_json::Value,
    client: Arc<PluginSubprocess>,
}

impl PluginTool {
    pub fn new(loaded: LoadedPluginTool) -> Self {
        Self {
            name: loaded.name,
            description: loaded.description,
            parameters: loaded.parameters,
            client: loaded.client,
        }
    }
}

#[async_trait]
impl Tool for PluginTool {
    fn id(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> serde_json::Value {
        self.parameters.clone()
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let context = serde_json::json!({
            "sessionID": ctx.session_id,
            "messageID": ctx.message_id,
            "agent": ctx.agent,
            "callID": ctx.call_id,
            "directory": ctx.directory,
            "worktree": ctx.worktree,
        });

        let response = self
            .client
            .tool_execute(&self.name, args, context)
            .await
            .map_err(|e| ToolError::ExecutionError(e.to_string()))?;

        let output = response
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        Ok(ToolResult {
            title: self.name.clone(),
            output,
            metadata: Metadata::new(),
            truncated: false,
        })
    }
}