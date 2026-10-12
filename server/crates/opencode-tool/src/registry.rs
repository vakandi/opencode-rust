use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{Tool, ToolContext, ToolError, ToolResult};
use opencode_plugin::{HookContext, HookEvent};

/// Tools that should not appear in suggestion lists when a tool is not found.
const FILTERED_FROM_SUGGESTIONS: &[&str] = &["invalid", "patch", "batch"];

pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    plugin_tools: RwLock<HashSet<String>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            plugin_tools: RwLock::new(HashSet::new()),
        }
    }

    pub async fn register<T: Tool + 'static>(&self, tool: T) {
        let mut tools = self.tools.write().await;
        tools.insert(tool.id().to_string(), Arc::new(tool));
    }

    /// Register a tool whose schema was authored by the plugin itself.
    ///
    /// Such a schema is already authoritative (the host converted it from the plugin's own
    /// definition), so `list_schemas` must not spend a `tool.definition` round-trip on it.
    pub async fn register_plugin<T: Tool + 'static>(&self, tool: T) {
        let id = tool.id().to_string();
        self.plugin_tools.write().await.insert(id.clone());
        self.tools.write().await.insert(id, Arc::new(tool));
    }

    pub async fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(id).cloned()
    }

    pub async fn list(&self) -> Vec<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.values().cloned().collect()
    }

    /// Returns all registered tool IDs.
    pub async fn list_ids(&self) -> Vec<String> {
        let tools = self.tools.read().await;
        tools.keys().cloned().collect()
    }

    /// Given a tool name that was not found, returns a list of available tool names
    /// filtered to exclude tools in `FILTERED_FROM_SUGGESTIONS`.
    pub async fn suggest_tools(&self, _requested: &str) -> Vec<String> {
        let tools = self.tools.read().await;
        let mut names: Vec<String> = tools
            .keys()
            .filter(|name| !FILTERED_FROM_SUGGESTIONS.contains(&name.as_str()))
            .cloned()
            .collect();
        names.sort();
        names
    }

    pub async fn list_schemas(&self) -> Vec<ToolSchema> {
        let from_plugin: HashSet<String> = self.plugin_tools.read().await.clone();
        let mut schemas: Vec<ToolSchema> = {
            let tools = self.tools.read().await;
            tools
                .values()
                .map(|t| ToolSchema {
                    name: t.id().to_string(),
                    description: t.description().to_string(),
                    parameters: t.parameters(),
                })
                .collect()
        };

        // Trigger tool.definition hook for each schema so plugins can transform them.
        // Plugin-authored tools are skipped: their schema came from the plugin already, and
        // one round-trip per tool (each able to burn the full RPC timeout) is what made a
        // single prompt take minutes.
        for schema in &mut schemas {
            if from_plugin.contains(&schema.name) {
                continue;
            }
            let hook_outputs = opencode_plugin::trigger_collect(
                HookContext::new(HookEvent::ToolDefinition)
                    .with_data("tool_id", serde_json::json!(&schema.name))
                    .with_data("description", serde_json::json!(&schema.description))
                    .with_data("parameters", schema.parameters.clone()),
            )
            .await;
            for output in hook_outputs {
                if let Some(payload) = output.payload.as_ref() {
                    apply_tool_definition_payload(schema, payload);
                }
            }
        }

        schemas
    }

    pub async fn execute(
        &self,
        tool_id: &str,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let tool = match self.get(tool_id).await {
            Some(t) => t,
            None => {
                let suggestions = self.suggest_tools(tool_id).await;
                return Err(ToolError::InvalidArguments(format!(
                    "Tool '{}' not found in registry. Available tools: {}",
                    tool_id,
                    suggestions.join(", ")
                )));
            }
        };

        let mut args = args;
        // Plugin hook: tool.execute.before
        let mut before_hook_ctx = HookContext::new(HookEvent::ToolExecuteBefore)
            .with_session(&ctx.session_id)
            .with_data("tool", serde_json::json!(tool_id))
            .with_data("args", args.clone());
        if let Some(call_id) = &ctx.call_id {
            before_hook_ctx = before_hook_ctx.with_data("call_id", serde_json::json!(call_id));
        }
        let before_outputs = opencode_plugin::trigger_collect(before_hook_ctx).await;
        for output in before_outputs {
            if let Some(payload) = output.payload.as_ref() {
                apply_tool_before_payload(&mut args, payload);
            }
        }

        tool.validate(&args)?;
        let mut result = tool.execute(args.clone(), ctx.clone()).await;

        // Plugin hook: tool.execute.after
        let mut hook_ctx = HookContext::new(HookEvent::ToolExecuteAfter)
            .with_session(&ctx.session_id)
            .with_data("tool", serde_json::json!(tool_id))
            .with_data("args", args);
        if let Some(call_id) = &ctx.call_id {
            hook_ctx = hook_ctx.with_data("call_id", serde_json::json!(call_id));
        }

        hook_ctx = match &result {
            Ok(r) => hook_ctx
                .with_data("title", serde_json::json!(&r.title))
                .with_data("output", serde_json::json!(&r.output))
                .with_data("metadata", serde_json::json!(&r.metadata))
                .with_data("error", serde_json::json!(false)),
            Err(e) => hook_ctx
                .with_data("output", serde_json::json!(e.to_string()))
                .with_data("error", serde_json::json!(true)),
        };

        let after_outputs = opencode_plugin::trigger_collect(hook_ctx).await;
        if let Ok(tool_result) = &mut result {
            for output in after_outputs {
                if let Some(payload) = output.payload.as_ref() {
                    apply_tool_after_payload(tool_result, payload);
                }
            }
        }

        result
    }
}

fn hook_payload_object(payload: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    payload
        .get("output")
        .and_then(|value| value.as_object())
        .or_else(|| payload.as_object())
        .or_else(|| payload.get("data").and_then(|value| value.as_object()))
}

fn apply_tool_definition_payload(schema: &mut ToolSchema, payload: &serde_json::Value) {
    let Some(object) = hook_payload_object(payload) else {
        return;
    };
    if let Some(description) = object.get("description").and_then(|value| value.as_str()) {
        schema.description = description.to_string();
    }
    if let Some(parameters) = object.get("parameters") {
        schema.parameters = parameters.clone();
    }
}

fn apply_tool_before_payload(args: &mut serde_json::Value, payload: &serde_json::Value) {
    let Some(object) = hook_payload_object(payload) else {
        return;
    };
    if let Some(next_args) = object.get("args") {
        *args = next_args.clone();
    }
}

fn apply_tool_after_payload(result: &mut ToolResult, payload: &serde_json::Value) {
    let Some(object) = hook_payload_object(payload) else {
        return;
    };
    if let Some(title) = object.get("title").and_then(|value| value.as_str()) {
        result.title = title.to_string();
    }
    if let Some(output) = object.get("output") {
        if let Some(output_str) = output.as_str() {
            result.output = output_str.to_string();
        } else if !output.is_null() {
            result.output = output.to_string();
        }
    }
    if let Some(metadata) = object.get("metadata").and_then(|value| value.as_object()) {
        result.metadata = metadata
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub async fn create_default_registry() -> ToolRegistry {
    let registry = ToolRegistry::new();

    registry.register(crate::read::ReadTool::new()).await;
    registry.register(crate::write::WriteTool::new()).await;
    registry.register(crate::edit::EditTool::new()).await;
    registry.register(crate::bash::BashTool::new()).await;
    registry.register(crate::glob_tool::GlobTool::new()).await;
    registry.register(crate::grep_tool::GrepTool::new()).await;
    registry.register(crate::ls::LsTool::new()).await;
    registry.register(crate::task::TaskTool::new()).await;
    registry
        .register(crate::question::QuestionTool::new())
        .await;
    registry
        .register(crate::webfetch::WebFetchTool::new())
        .await;
    registry
        .register(crate::websearch::WebSearchTool::new())
        .await;
    registry.register(crate::todo::TodoReadTool).await;
    registry.register(crate::todo::TodoWriteTool).await;
    registry.register(crate::multiedit::MultiEditTool).await;
    registry.register(crate::apply_patch::ApplyPatchTool).await;
    registry.register(crate::skill::SkillTool).await;
    registry.register(crate::lsp_tool::LspTool).await;
    registry.register(crate::batch::BatchTool).await;
    registry.register(crate::codesearch::CodeSearchTool).await;
    registry.register(crate::plan::PlanEnterTool).await;
    registry.register(crate::plan::PlanExitTool).await;
    registry.register(crate::invalid::InvalidTool).await;

    let plugin_tools = opencode_plugin::plugin_tools::list().await;
    if !plugin_tools.is_empty() {
        let names: Vec<&str> = plugin_tools.iter().map(|t| t.name.as_str()).collect();
        tracing::debug!(count = names.len(), tools = ?names, "registering plugin tools");
    }
    for loaded in plugin_tools {
        registry
            .register_plugin(crate::plugin_tool::PluginTool::new(loaded))
            .await;
    }

    registry
}
