//! `PluginSubprocess` — manages a single plugin host child process.
//!
//! Communicates via Content-Length framed JSON-RPC 2.0 over stdin/stdout,
//! mirroring the MCP stdio transport pattern.

use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::protocol::{RpcError, RpcRequest, RpcResponse};
use super::runtime::JsRuntime;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum PluginSubprocessError {
    #[error("subprocess I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("plugin RPC error ({code}): {message}")]
    Rpc { code: i64, message: String },

    #[error("plugin subprocess not running")]
    NotRunning,

    #[error("plugin response timeout")]
    Timeout,

    #[error("protocol error: {0}")]
    Protocol(String),
}

impl From<RpcError> for PluginSubprocessError {
    fn from(e: RpcError) -> Self {
        Self::Rpc {
            code: e.code,
            message: e.message,
        }
    }
}

// ---------------------------------------------------------------------------
// Result types (deserialized from host responses)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct InitializeResult {
    pub name: String,
    pub hooks: Vec<String>,
    /// Tool definitions contributed by the plugin, keyed by tool name.
    ///
    /// Without this field the host's forwarded definitions were dropped during
    /// deserialization, so a plugin could register tools that were then unreachable.
    #[serde(default)]
    pub tools: std::collections::HashMap<String, serde_json::Value>,
    pub auth: Option<AuthMeta>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthMeta {
    pub provider: String,
    pub methods: Vec<AuthMethodMeta>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthMethodMeta {
    #[serde(rename = "type")]
    pub method_type: String,
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeResult {
    pub url: Option<String>,
    pub instructions: Option<String>,
    pub method: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuthLoadResult {
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(rename = "hasCustomFetch")]
    pub has_custom_fetch: bool,
}

#[derive(Debug, Deserialize)]
pub struct AuthFetchResult {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: String,
}

pub struct AuthFetchStreamResult {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub chunks: mpsc::Receiver<Result<String, PluginSubprocessError>>,
}

// ---------------------------------------------------------------------------
// Context passed to plugin on initialize
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PluginContext {
    pub worktree: String,
    pub directory: String,
    #[serde(rename = "serverUrl")]
    pub server_url: String,
}

// ---------------------------------------------------------------------------
// PluginSubprocess
// ---------------------------------------------------------------------------

pub struct PluginSubprocess {
    /// Human-readable plugin name (from initialize response).
    name: String,
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    rpc_lock: Arc<Mutex<()>>,
    process: Mutex<Child>,
    request_id: AtomicU64,
    /// Hook names this plugin registered.
    hooks: Vec<String>,
    /// Tool definitions this plugin contributed, keyed by tool name.
    tools: std::collections::HashMap<String, serde_json::Value>,
    /// Auth metadata, if the plugin provides auth.
    auth_meta: Option<AuthMeta>,
    /// RPC call timeout.
    timeout: Duration,
}

impl PluginSubprocess {
    /// Spawn a plugin host subprocess and run the `initialize` handshake.
    ///
    /// `cwd` sets the working directory for the subprocess so that bare-specifier
    /// `import("pkg")` calls resolve against the correct `node_modules/`.
    pub async fn spawn(
        runtime: JsRuntime,
        host_script: &str,
        plugin_path: &str,
        context: PluginContext,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self, PluginSubprocessError> {
        let args = runtime.run_args(host_script);
        let mut cmd = Command::new(runtime.command());
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()) // plugin logs go to parent stderr
            .kill_on_drop(true);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginSubprocessError::Protocol("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginSubprocessError::Protocol("no stdout".into()))?;

        let mut this = Self {
            name: String::new(),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            rpc_lock: Arc::new(Mutex::new(())),
            process: Mutex::new(child),
            request_id: AtomicU64::new(1),
            hooks: Vec::new(),
            tools: std::collections::HashMap::new(),
            auth_meta: None,
            timeout: Duration::from_secs(30),
        };

        // Send initialize
        let params = serde_json::json!({
            "pluginPath": plugin_path,
            "context": context,
        });
        let result: InitializeResult = this.call("initialize", Some(params)).await?;

        this.name = result.name;
        this.hooks = result.hooks;
        this.tools = result.tools;
        this.auth_meta = result.auth;

        Ok(this)
    }

    // -- Accessors ----------------------------------------------------------

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn tools(&self) -> &std::collections::HashMap<String, serde_json::Value> {
        &self.tools
    }

    pub async fn tool_execute(
        &self,
        tool: &str,
        args: serde_json::Value,
        context: serde_json::Value,
    ) -> Result<serde_json::Value, PluginSubprocessError> {
        let params = serde_json::json!({
            "tool": tool,
            "args": args,
            "context": context,
        });
        self.call("tool.execute", Some(params)).await
    }

    pub fn hooks(&self) -> &[String] {
        &self.hooks
    }

    pub fn auth_meta(&self) -> Option<&AuthMeta> {
        self.auth_meta.as_ref()
    }

    // -- RPC methods --------------------------------------------------------

    /// Invoke a hook on the plugin.
    pub async fn invoke_hook(
        &self,
        hook: &str,
        input: Value,
        output: Value,
    ) -> Result<Value, PluginSubprocessError> {
        let params = serde_json::json!({
            "hook": hook,
            "input": input,
            "output": output,
        });
        let result: Value = self.call("hook.invoke", Some(params)).await?;
        Ok(result.get("output").cloned().unwrap_or(Value::Null))
    }

    /// Trigger OAuth authorization flow.
    pub async fn auth_authorize(
        &self,
        method_index: usize,
        inputs: Option<Value>,
    ) -> Result<AuthorizeResult, PluginSubprocessError> {
        let params = serde_json::json!({
            "methodIndex": method_index,
            "inputs": inputs.unwrap_or(Value::Null),
        });
        self.call("auth.authorize", Some(params)).await
    }

    /// Complete OAuth callback.
    pub async fn auth_callback(&self, code: Option<&str>) -> Result<Value, PluginSubprocessError> {
        let params = serde_json::json!({ "code": code });
        self.call("auth.callback", Some(params)).await
    }

    /// Load auth provider configuration.
    pub async fn auth_load(&self, provider: &str) -> Result<AuthLoadResult, PluginSubprocessError> {
        let params = serde_json::json!({ "provider": provider });
        self.call("auth.load", Some(params)).await
    }

    /// Proxy an HTTP request through the plugin's custom fetch.
    pub async fn auth_fetch(
        &self,
        url: &str,
        method: &str,
        headers: &std::collections::HashMap<String, String>,
        body: Option<&str>,
    ) -> Result<AuthFetchResult, PluginSubprocessError> {
        let params = serde_json::json!({
            "url": url,
            "method": method,
            "headers": headers,
            "body": body,
        });
        self.call("auth.fetch", Some(params)).await
    }

    /// Proxy an HTTP request through the plugin's custom fetch as a real-time stream.
    pub async fn auth_fetch_stream(
        &self,
        url: &str,
        method: &str,
        headers: &std::collections::HashMap<String, String>,
        body: Option<&str>,
    ) -> Result<AuthFetchStreamResult, PluginSubprocessError> {
        let id = self.next_id();
        let params = serde_json::json!({
            "url": url,
            "method": method,
            "headers": headers,
            "body": body,
        });

        let rpc_guard = self.rpc_lock.clone().lock_owned().await;
        self.write_request(id, "auth.fetch.stream", Some(params))
            .await?;

        let (start_tx, start_rx) = oneshot::channel::<
            Result<(u16, std::collections::HashMap<String, String>), PluginSubprocessError>,
        >();
        let (chunk_tx, chunk_rx) = mpsc::channel(128);
        let stdout = Arc::clone(&self.stdout);

        tokio::spawn(async move {
            let _rpc_guard = rpc_guard;
            let mut start_tx = Some(start_tx);
            let mut reader = stdout.lock().await;

            loop {
                let raw = match Self::read_raw_message(&mut reader).await {
                    Ok(raw) => raw,
                    Err(err) => {
                        if let Some(tx) = start_tx.take() {
                            let _ = tx.send(Err(err));
                        } else {
                            let _ = chunk_tx.send(Err(err)).await;
                        }
                        break;
                    }
                };

                if raw.get("id").and_then(Value::as_u64) == Some(id) {
                    let response: RpcResponse = match serde_json::from_value(raw) {
                        Ok(response) => response,
                        Err(err) => {
                            let send_err = PluginSubprocessError::Json(err);
                            if let Some(tx) = start_tx.take() {
                                let _ = tx.send(Err(send_err));
                            } else {
                                let _ = chunk_tx.send(Err(send_err)).await;
                            }
                            break;
                        }
                    };

                    if let Some(error) = response.error {
                        let send_err = PluginSubprocessError::from(error);
                        if let Some(tx) = start_tx.take() {
                            let _ = tx.send(Err(send_err));
                        } else {
                            let _ = chunk_tx.send(Err(send_err)).await;
                        }
                        break;
                    }

                    let result = response.result.unwrap_or(Value::Null);
                    let status = result
                        .get("status")
                        .and_then(Value::as_u64)
                        .and_then(|v| u16::try_from(v).ok())
                        .unwrap_or(200);
                    let headers = result
                        .get("headers")
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                    if let Some(tx) = start_tx.take() {
                        let _ = tx.send(Ok((status, headers)));
                    }
                    continue;
                }

                let method = raw.get("method").and_then(Value::as_str);
                let params = raw.get("params").cloned().unwrap_or(Value::Null);
                let request_id = params.get("requestId").and_then(Value::as_u64);
                if request_id != Some(id) {
                    continue;
                }

                match method {
                    Some("auth.fetch.stream.chunk") => {
                        if let Some(chunk) = params.get("chunk").and_then(Value::as_str) {
                            if chunk_tx.send(Ok(chunk.to_string())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some("auth.fetch.stream.error") => {
                        let message = params
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("plugin custom fetch stream failed")
                            .to_string();
                        let error = PluginSubprocessError::Protocol(message);
                        if let Some(tx) = start_tx.take() {
                            let _ = tx.send(Err(error));
                        } else {
                            let _ = chunk_tx.send(Err(error)).await;
                        }
                        break;
                    }
                    Some("auth.fetch.stream.end") => {
                        break;
                    }
                    _ => {}
                }
            }
        });

        let (status, response_headers) = tokio::time::timeout(self.timeout, start_rx)
            .await
            .map_err(|_| PluginSubprocessError::Timeout)?
            .map_err(|_| PluginSubprocessError::NotRunning)??;

        Ok(AuthFetchStreamResult {
            status,
            headers: response_headers,
            chunks: chunk_rx,
        })
    }

    /// Gracefully shut down the plugin subprocess.
    pub async fn shutdown(&self) -> Result<(), PluginSubprocessError> {
        let _: Value = self.call("shutdown", None).await?;
        // Give the process a moment to exit, then kill if needed
        let mut proc = self.process.lock().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), proc.wait()).await;
        let _ = proc.kill().await;
        Ok(())
    }

    // -- Transport (Content-Length framing) ----------------------------------

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a JSON-RPC request and wait for the response.
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<T, PluginSubprocessError> {
        let _rpc_guard = self.rpc_lock.lock().await;
        let id = self.next_id();
        self.write_request(id, method, params).await?;

        // Read response with timeout
        let response = tokio::time::timeout(self.timeout, self.read_response_for_id(id))
            .await
            .map_err(|_| PluginSubprocessError::Timeout)??;

        if let Some(err) = response.error {
            return Err(err.into());
        }

        let result = response.result.unwrap_or(Value::Null);
        serde_json::from_value(result).map_err(Into::into)
    }

    async fn write_request(
        &self,
        id: u64,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), PluginSubprocessError> {
        let request = RpcRequest::new(id, method, params);
        let content = serde_json::to_string(&request)?;
        let frame = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(frame.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn read_response_for_id(
        &self,
        expected_id: u64,
    ) -> Result<RpcResponse, PluginSubprocessError> {
        let mut reader = self.stdout.lock().await;
        loop {
            let raw = Self::read_raw_message(&mut reader).await?;
            if raw.get("id").and_then(Value::as_u64) != Some(expected_id) {
                continue;
            }
            let response: RpcResponse = serde_json::from_value(raw)?;
            return Ok(response);
        }
    }

    /// Read one Content-Length framed JSON-RPC message from stdout.
    async fn read_raw_message(
        reader: &mut BufReader<ChildStdout>,
    ) -> Result<Value, PluginSubprocessError> {
        // Read headers until empty line
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Err(PluginSubprocessError::NotRunning);
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }

            if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
                content_length = Some(rest.trim().parse().map_err(|e| {
                    PluginSubprocessError::Protocol(format!("bad Content-Length: {e}"))
                })?);
            }
        }

        let len = content_length.ok_or_else(|| {
            PluginSubprocessError::Protocol("missing Content-Length header".into())
        })?;

        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).await?;
        let value: Value = serde_json::from_slice(&buf)?;
        Ok(value)
    }
}
