//! Exposes MCP server tools through the ordinary tool registry.
//!
//! The MCP client knows how to list and call tools, but the agent only ever sees
//! `ToolRegistry`. Without this bridge an MCP tool would be advertised to the
//! model and then fail on execution as "not found in registry".

use std::sync::Arc;

use opencode_mcp::{McpClient, McpClientRegistry};
use opencode_tool::{Tool, ToolContext, ToolError, ToolResult};

pub struct McpProxyTool {
    schema: opencode_provider::ToolDefinition,
    registry: Arc<McpClientRegistry>,
    server_name: String,
    bare_name: String,
}

impl McpProxyTool {
    pub fn new(
        schema: opencode_provider::ToolDefinition,
        registry: Arc<McpClientRegistry>,
        server_name: String,
        bare_name: String,
    ) -> Self {
        Self {
            schema,
            registry,
            server_name,
            bare_name,
        }
    }
}

#[async_trait::async_trait]
impl Tool for McpProxyTool {
    fn id(&self) -> &str {
        &self.schema.name
    }

    fn description(&self) -> &str {
        self.schema.description.as_deref().unwrap_or_default()
    }

    fn parameters(&self) -> serde_json::Value {
        self.schema.parameters.clone()
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let client: Arc<McpClient> = self.registry.client(&self.server_name).await.ok_or_else(
            || ToolError::ExecutionError(format!("MCP server '{}' is not connected", self.server_name)),
        )?;

        let result = client
            .call_tool(&self.bare_name, Some(args))
            .await
            .map_err(|e| ToolError::ExecutionError(format!("MCP call failed: {}", e)))?;

        Ok(ToolResult {
            title: self.schema.name.clone(),
            output: render_content(&result),
            metadata: std::collections::HashMap::new(),
            truncated: false,
        })
    }
}

fn render_content(result: &opencode_mcp::CallToolResult) -> String {
    let mut out = String::new();
    for part in &result.content {
        match part {
            opencode_mcp::ContentBlock { text: Some(text), .. } => {
                out.push_str(text);
                out.push('\n');
            }
            other => out.push_str(&format!("{:?}\n", other)),
        }
    }
    if out.trim().is_empty() {
        out = serde_json::to_string(result).unwrap_or_default();
    }
    out
}

/// Every tool the started MCP servers expose, ready to register.
pub async fn proxy_tools() -> Vec<McpProxyTool> {
    let Some(registry) = crate::mcp_loader::registry() else {
        return Vec::new();
    };
    registry
        .tool_registry()
        .list()
        .await
        .into_iter()
        .map(|tool| {
            let schema = opencode_provider::ToolDefinition {
                name: tool.full_name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            };
            McpProxyTool::new(
                schema,
                registry.clone(),
                tool.server_name.clone(),
                tool.name.clone(),
            )
        })
        .collect()
}