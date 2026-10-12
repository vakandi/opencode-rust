use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

const MAX_BATCH_SIZE: usize = 25;
const DISALLOWED_TOOLS: &[&str] = &["batch"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchParams {
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub tool: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<serde_json::Value>,
}

pub struct BatchTool;

type BatchFuture = Pin<Box<dyn Future<Output = BatchResult> + Send>>;

#[async_trait]
impl Tool for BatchTool {
    fn id(&self) -> &str {
        "batch"
    }

    fn description(&self) -> &str {
        "Execute multiple tool calls in parallel. Maximum 25 tools per batch. Use this for optimal performance when you need to run multiple independent operations."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "toolCalls": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 25,
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": {
                                "type": "string",
                                "description": "The name of the tool to execute"
                            },
                            "parameters": {
                                "type": "object",
                                "description": "Parameters for the tool"
                            }
                        },
                        "required": ["tool", "parameters"]
                    },
                    "description": "Array of tool calls to execute in parallel"
                }
            },
            "required": ["toolCalls"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let params: BatchParams = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid parameters: {}", e)))?;

        let total_calls = params.tool_calls.len();
        let tool_calls: Vec<_> = params.tool_calls.into_iter().take(MAX_BATCH_SIZE).collect();
        let discarded_count = total_calls.saturating_sub(MAX_BATCH_SIZE);

        if tool_calls.is_empty() {
            return Err(ToolError::ValidationError(
                "Provide at least one tool call".to_string(),
            ));
        }

        let registry = match &ctx.registry {
            Some(r) => r.clone(),
            None => {
                return Err(ToolError::ExecutionError(
                    "Tool registry not available. Batch execution requires registry access."
                        .to_string(),
                ));
            }
        };

        let mut futures: Vec<BatchFuture> = Vec::new();

        for call in tool_calls {
            if DISALLOWED_TOOLS.contains(&call.tool.as_str()) {
                let tool_name = call.tool.clone();
                let err_msg = format!(
                    "Tool '{}' is not allowed in batch. Disallowed: {}",
                    tool_name,
                    DISALLOWED_TOOLS.join(", ")
                );
                futures.push(Box::pin(async move {
                    BatchResult {
                        tool: tool_name,
                        success: false,
                        output: None,
                        title: None,
                        error: Some(err_msg),
                        metadata: None,
                        attachment: None,
                    }
                }) as BatchFuture);
                continue;
            }

            let registry = registry.clone();
            let tool_name = call.tool.clone();
            let tool_params = call.parameters.clone();
            let ctx_clone = ctx.clone();
            let session_id = ctx.session_id.clone();
            let message_id = ctx.message_id.clone();
            let call_id = uuid::Uuid::new_v4().to_string();
            let call_start_time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            futures.push(Box::pin(async move {
                let _ = ctx_clone
                    .do_update_part(serde_json::json!({
                        "id": call_id,
                        "messageID": message_id,
                        "sessionID": session_id,
                        "type": "tool",
                        "tool": tool_name,
                        "callID": call_id,
                        "state": {
                            "status": "running",
                            "input": tool_params,
                            "time": {
                                "start": call_start_time
                            }
                        }
                    }))
                    .await;

                let result = match registry.get(&tool_name).await {
                    Some(tool) => {
                        match tool.execute(tool_params.clone(), ctx_clone.clone()).await {
                            Ok(res) => {
                                let call_end_time = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis()
                                    as u64;

                                let _ = ctx_clone
                                    .do_update_part(serde_json::json!({
                                        "id": call_id,
                                        "messageID": message_id,
                                        "sessionID": session_id,
                                        "type": "tool",
                                        "tool": tool_name,
                                        "callID": call_id,
                                        "state": {
                                            "status": "completed",
                                            "input": tool_params,
                                            "output": res.output,
                                            "title": res.title,
                                            "metadata": res.metadata,
                                            "time": {
                                                "start": call_start_time,
                                                "end": call_end_time
                                            }
                                        }
                                    }))
                                    .await;

                                let attachment = res.metadata.get("attachment").cloned();
                                let metadata_value = if !res.metadata.is_empty() {
                                    Some(serde_json::to_value(&res.metadata).ok()).flatten()
                                } else {
                                    None
                                };

                                BatchResult {
                                    tool: tool_name,
                                    success: true,
                                    output: Some(res.output),
                                    title: Some(res.title),
                                    error: None,
                                    metadata: metadata_value,
                                    attachment,
                                }
                            }
                            Err(e) => {
                                let call_end_time = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis()
                                    as u64;

                                let _ = ctx_clone
                                    .do_update_part(serde_json::json!({
                                        "id": call_id,
                                        "messageID": message_id,
                                        "sessionID": session_id,
                                        "type": "tool",
                                        "tool": tool_name,
                                        "callID": call_id,
                                        "state": {
                                            "status": "error",
                                            "input": tool_params,
                                            "error": e.to_string(),
                                            "time": {
                                                "start": call_start_time,
                                                "end": call_end_time
                                            }
                                        }
                                    }))
                                    .await;

                                BatchResult {
                                    tool: tool_name,
                                    success: false,
                                    output: None,
                                    title: None,
                                    error: Some(e.to_string()),
                                    metadata: None,
                                    attachment: None,
                                }
                            }
                        }
                    }
                    None => {
                        let available = registry.suggest_tools(&tool_name).await;
                        let err_msg = format!(
                            "Tool '{}' not in registry. External tools (MCP, environment) cannot be batched - call them directly. Available tools: {}",
                            tool_name,
                            available.join(", ")
                        );
                        BatchResult {
                            tool: tool_name.clone(),
                            success: false,
                            output: None,
                            title: None,
                            error: Some(err_msg),
                            metadata: None,
                            attachment: None,
                        }
                    }
                };

                result
            }) as BatchFuture);
        }

        let results: Vec<BatchResult> = futures::future::join_all(futures).await;

        let mut final_results = results;

        if discarded_count > 0 {
            final_results.push(BatchResult {
                tool: "batch".to_string(),
                success: false,
                output: None,
                title: None,
                error: Some(format!(
                    "{} additional calls discarded (max {} per batch)",
                    discarded_count, MAX_BATCH_SIZE
                )),
                metadata: None,
                attachment: None,
            });
        }

        let successful = final_results.iter().filter(|r| r.success).count();
        let failed = final_results.len() - successful;

        let output = if failed > 0 {
            format!(
                "Batch execution: {}/{} tools succeeded. {} failed.\n\nResults:\n{}",
                successful,
                final_results.len(),
                failed,
                serde_json::to_string_pretty(&final_results).unwrap_or_default()
            )
        } else {
            format!(
                "All {} tools executed successfully.\n\nKeep using the batch tool for optimal performance!\n\nResults:\n{}",
                successful,
                serde_json::to_string_pretty(&final_results).unwrap_or_default()
            )
        };

        let tools_list: Vec<&str> = final_results.iter().map(|r| r.tool.as_str()).collect();

        let mut metadata = Metadata::new();
        metadata.insert("total".to_string(), serde_json::json!(final_results.len()));
        metadata.insert("successful".to_string(), serde_json::json!(successful));
        metadata.insert("failed".to_string(), serde_json::json!(failed));
        metadata.insert("tools".to_string(), serde_json::json!(tools_list));
        metadata.insert(
            "details".to_string(),
            serde_json::json!(final_results
                .iter()
                .map(|r| serde_json::json!({
                    "tool": r.tool,
                    "success": r.success
                }))
                .collect::<Vec<_>>()),
        );

        Ok(ToolResult {
            output,
            title: format!("Batch execution ({}/{})", successful, final_results.len()),
            metadata,
            truncated: false,
        })
    }
}
