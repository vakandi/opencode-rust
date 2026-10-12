//! Lightweight JSON-RPC 2.0 types for the plugin subprocess protocol.
//!
//! These are intentionally independent of `opencode-mcp` so the plugin crate
//! does not pull in the full MCP dependency graph.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// JSON-RPC envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl RpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RpcResponse {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<Value>,
    pub error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct RpcNotification {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}
