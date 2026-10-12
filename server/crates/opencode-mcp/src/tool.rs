use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub server_name: String,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

impl McpTool {
    pub fn new(
        server_name: &str,
        name: &str,
        description: Option<String>,
        input_schema: serde_json::Value,
    ) -> Self {
        Self {
            server_name: server_name.to_string(),
            name: name.to_string(),
            full_name: format!("{}_{}", server_name, name),
            description,
            input_schema,
        }
    }
}

pub struct McpToolRegistry {
    tools: RwLock<HashMap<String, McpTool>>,
}

impl McpToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, tool: McpTool) {
        let mut tools = self.tools.write().await;
        tools.insert(tool.full_name.clone(), tool);
    }

    pub async fn register_batch(
        &self,
        server_name: &str,
        tools: Vec<crate::protocol::ToolDefinition>,
    ) {
        let mut registry = self.tools.write().await;
        for tool_def in tools {
            let mcp_tool = McpTool::new(
                server_name,
                &tool_def.name,
                tool_def.description,
                tool_def.input_schema,
            );
            registry.insert(mcp_tool.full_name.clone(), mcp_tool);
        }
    }

    pub async fn get(&self, full_name: &str) -> Option<McpTool> {
        let tools = self.tools.read().await;
        tools.get(full_name).cloned()
    }

    pub async fn list(&self) -> Vec<McpTool> {
        let tools = self.tools.read().await;
        tools.values().cloned().collect()
    }

    pub async fn list_for_server(&self, server_name: &str) -> Vec<McpTool> {
        let tools = self.tools.read().await;
        tools
            .values()
            .filter(|t| t.server_name == server_name)
            .cloned()
            .collect()
    }

    pub async fn clear_server(&self, server_name: &str) {
        let mut tools = self.tools.write().await;
        tools.retain(|_, t| t.server_name != server_name);
    }

    pub async fn clear(&self) {
        let mut tools = self.tools.write().await;
        tools.clear();
    }
}

impl Default for McpToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
