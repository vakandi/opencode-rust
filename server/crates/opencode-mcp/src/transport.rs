use async_trait::async_trait;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;

use crate::protocol::{JsonRpcMessage, JsonRpcRequest};
use crate::McpClientError;

// ---------------------------------------------------------------------------
// Transport trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn send(&self, request: &JsonRpcRequest) -> Result<(), McpClientError>;
    async fn receive(&self) -> Result<Option<JsonRpcMessage>, McpClientError>;
    async fn close(&self) -> Result<(), McpClientError>;
}

// ---------------------------------------------------------------------------
// StdioTransport
// ---------------------------------------------------------------------------

pub struct StdioTransport {
    process: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
}

impl StdioTransport {
    pub async fn new(
        command: &str,
        args: &[String],
        env: Option<Vec<(String, String)>>,
    ) -> Result<Self, McpClientError> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(env_vars) = env {
            for (key, value) in env_vars {
                cmd.env(key, value);
            }
        }
        let mut child = cmd.spawn().map_err(|e| {
            McpClientError::TransportError(format!("Failed to spawn process: {}", e))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpClientError::TransportError("Failed to get stdin".to_string()))?;

        Ok(Self {
            process: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
        })
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, request: &JsonRpcRequest) -> Result<(), McpClientError> {
        let mut stdin_guard = self.stdin.lock().await;
        let stdin = stdin_guard
            .as_mut()
            .ok_or_else(|| McpClientError::TransportError("Process not running".to_string()))?;

        let content = serde_json::to_string(request).map_err(|e| {
            McpClientError::ProtocolError(format!("Failed to serialize request: {}", e))
        })?;

        let message = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);

        stdin
            .write_all(message.as_bytes())
            .await
            .map_err(|e| McpClientError::TransportError(format!("Failed to write: {}", e)))?;

        stdin
            .flush()
            .await
            .map_err(|e| McpClientError::TransportError(format!("Failed to flush: {}", e)))?;

        Ok(())
    }

    async fn receive(&self) -> Result<Option<JsonRpcMessage>, McpClientError> {
        let mut process_guard = self.process.lock().await;
        let child = process_guard
            .as_mut()
            .ok_or_else(|| McpClientError::TransportError("Process not running".to_string()))?;

        let stdout = child
            .stdout
            .as_mut()
            .ok_or_else(|| McpClientError::TransportError("No stdout".to_string()))?;

        let mut reader = BufReader::new(stdout);
        let mut header_line = String::new();
        loop {
            header_line.clear();
            let bytes_read = reader.read_line(&mut header_line).await.map_err(|e| {
                McpClientError::TransportError(format!("Failed to read header: {}", e))
            })?;

            if bytes_read == 0 {
                return Ok(None);
            }

            let trimmed = header_line.trim();

            if trimmed.is_empty() {
                break;
            }

            if trimmed.starts_with("Content-Length:") {
                let content_length: usize = trimmed[15..].trim().parse().map_err(|e| {
                    McpClientError::ProtocolError(format!("Invalid content length: {}", e))
                })?;

                let mut content_buf = vec![0u8; content_length];
                reader.read_exact(&mut content_buf).await.map_err(|e| {
                    McpClientError::TransportError(format!("Failed to read content: {}", e))
                })?;

                let content = String::from_utf8_lossy(&content_buf);
                let message = JsonRpcMessage::from_str(&content).map_err(|e| {
                    McpClientError::ProtocolError(format!("Failed to parse message: {}", e))
                })?;

                return Ok(Some(message));
            }
        }

        Ok(None)
    }

    async fn close(&self) -> Result<(), McpClientError> {
        let mut process_guard = self.process.lock().await;
        if let Some(mut child) = process_guard.take() {
            child.kill().await.map_err(|e| {
                McpClientError::TransportError(format!("Failed to kill process: {}", e))
            })?;
        }
        let mut stdin_guard = self.stdin.lock().await;
        *stdin_guard = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HttpTransport (StreamableHTTP)
// ---------------------------------------------------------------------------

/// Transport that sends JSON-RPC requests over HTTP POST and reads streaming
/// (potentially chunked) JSON responses. Mirrors the TS `StreamableHTTPClientTransport`.
pub struct HttpTransport {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
    /// Buffer for responses received via streaming that haven't been consumed yet.
    response_rx: Mutex<tokio::sync::mpsc::UnboundedReceiver<JsonRpcMessage>>,
    response_tx: tokio::sync::mpsc::UnboundedSender<JsonRpcMessage>,
}

impl HttpTransport {
    pub fn new(url: String, headers: Option<HashMap<String, String>>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            url,
            headers: headers.unwrap_or_default(),
            client: reqwest::Client::new(),
            response_rx: Mutex::new(rx),
            response_tx: tx,
        }
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn send(&self, request: &JsonRpcRequest) -> Result<(), McpClientError> {
        let mut builder = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        for (key, value) in &self.headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        let body = serde_json::to_string(request).map_err(|e| {
            McpClientError::ProtocolError(format!("Failed to serialize request: {}", e))
        })?;

        let resp =
            builder.body(body).send().await.map_err(|e| {
                McpClientError::TransportError(format!("HTTP request failed: {}", e))
            })?;

        if !resp.status().is_success() {
            return Err(McpClientError::TransportError(format!(
                "HTTP {} from server",
                resp.status()
            )));
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            // Server chose to stream the response via SSE inside the POST response.
            let text = resp.text().await.map_err(|e| {
                McpClientError::TransportError(format!("Failed to read SSE body: {}", e))
            })?;
            for line in text.lines() {
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }
                    if let Ok(message) = JsonRpcMessage::from_str(data) {
                        let _ = self.response_tx.send(message);
                    }
                }
            }
        } else {
            // Plain JSON response.
            let text = resp.text().await.map_err(|e| {
                McpClientError::TransportError(format!("Failed to read response body: {}", e))
            })?;
            if !text.is_empty() {
                let message = JsonRpcMessage::from_str(&text).map_err(|e| {
                    McpClientError::ProtocolError(format!("Failed to parse response: {}", e))
                })?;
                let _ = self.response_tx.send(message);
            }
        }

        Ok(())
    }

    async fn receive(&self) -> Result<Option<JsonRpcMessage>, McpClientError> {
        let mut rx = self.response_rx.lock().await;
        match rx.recv().await {
            Some(msg) => Ok(Some(msg)),
            None => Ok(None),
        }
    }

    async fn close(&self) -> Result<(), McpClientError> {
        // Nothing to tear down – the reqwest client will be dropped with the struct.
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SseTransport
// ---------------------------------------------------------------------------

/// Transport that connects to an SSE endpoint for receiving messages and
/// POSTs JSON-RPC requests to the same base URL. Mirrors the TS
/// `SSEClientTransport`.
pub struct SseTransport {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
    response_rx: Mutex<tokio::sync::mpsc::UnboundedReceiver<JsonRpcMessage>>,
    response_tx: tokio::sync::mpsc::UnboundedSender<JsonRpcMessage>,
    /// Handle to the background SSE listener task so we can abort on close.
    sse_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SseTransport {
    pub fn new(url: String, headers: Option<HashMap<String, String>>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            url,
            headers: headers.unwrap_or_default(),
            client: reqwest::Client::new(),
            response_rx: Mutex::new(rx),
            response_tx: tx,
            sse_task: Mutex::new(None),
        }
    }

    /// Start the background SSE listener. Must be called before `send`/`receive`.
    pub async fn connect(&self) -> Result<(), McpClientError> {
        use futures::StreamExt;
        use reqwest_eventsource::{Event, EventSource};

        let mut builder = self.client.get(&self.url);
        builder = builder.header("Accept", "text/event-stream");
        for (key, value) in &self.headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        let mut es = EventSource::new(builder).map_err(|e| {
            McpClientError::TransportError(format!("Failed to create SSE connection: {}", e))
        })?;

        let tx = self.response_tx.clone();

        let handle = tokio::spawn(async move {
            while let Some(event) = es.next().await {
                match event {
                    Ok(Event::Message(msg)) => {
                        let data = msg.data.trim().to_string();
                        if data.is_empty() || data == "[DONE]" {
                            continue;
                        }
                        match JsonRpcMessage::from_str(&data) {
                            Ok(msg) => {
                                if tx.send(msg).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!("SSE: failed to parse message: {}", e);
                            }
                        }
                    }
                    Ok(Event::Open) => {
                        tracing::debug!("SSE connection opened");
                    }
                    Err(e) => {
                        tracing::error!("SSE error: {}", e);
                        break;
                    }
                }
            }
        });

        let mut task = self.sse_task.lock().await;
        *task = Some(handle);

        Ok(())
    }
}

#[async_trait]
impl McpTransport for SseTransport {
    async fn send(&self, request: &JsonRpcRequest) -> Result<(), McpClientError> {
        let mut builder = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json");

        for (key, value) in &self.headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        let body = serde_json::to_string(request).map_err(|e| {
            McpClientError::ProtocolError(format!("Failed to serialize request: {}", e))
        })?;

        let resp = builder
            .body(body)
            .send()
            .await
            .map_err(|e| McpClientError::TransportError(format!("HTTP POST failed: {}", e)))?;

        if !resp.status().is_success() {
            return Err(McpClientError::TransportError(format!(
                "HTTP {} from server",
                resp.status()
            )));
        }

        Ok(())
    }

    async fn receive(&self) -> Result<Option<JsonRpcMessage>, McpClientError> {
        let mut rx = self.response_rx.lock().await;
        match rx.recv().await {
            Some(msg) => Ok(Some(msg)),
            None => Ok(None),
        }
    }

    async fn close(&self) -> Result<(), McpClientError> {
        let mut task = self.sse_task.lock().await;
        if let Some(handle) = task.take() {
            handle.abort();
        }
        Ok(())
    }
}
