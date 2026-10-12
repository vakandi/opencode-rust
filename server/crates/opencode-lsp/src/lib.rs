use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;

use lsp_types::{
    ClientCapabilities, Diagnostic, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    InitializeParams, Range, TextDocumentContentChangeEvent, TextDocumentIdentifier,
    TextDocumentItem, VersionedTextDocumentIdentifier, WorkspaceFolder,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::{debug, error};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: lsp_types::SymbolKind,
    #[serde(default)]
    pub tags: Option<Vec<lsp_types::SymbolTag>>,
    #[serde(default)]
    pub detail: Option<String>,
    pub uri: lsp_types::Uri,
    pub range: Range,
    pub selection_range: Range,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyPrepareParams {
    #[serde(flatten)]
    pub text_document_position_params: lsp_types::TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done_progress_params: lsp_types::WorkDoneProgressParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyIncomingCallsParams {
    pub item: CallHierarchyItem,
    #[serde(flatten)]
    pub work_done_progress_params: lsp_types::WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result_params: lsp_types::PartialResultParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyOutgoingCallsParams {
    pub item: CallHierarchyItem,
    #[serde(flatten)]
    pub work_done_progress_params: lsp_types::WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result_params: lsp_types::PartialResultParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyIncomingCall {
    pub from: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallHierarchyOutgoingCall {
    pub to: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

fn path_to_uri(path: &Path) -> Result<lsp_types::Uri, LspError> {
    let url = Url::from_file_path(path)
        .map_err(|_| LspError::InitializeError("Invalid file path".to_string()))?;
    lsp_types::Uri::from_str(url.as_str())
        .map_err(|e| LspError::InitializeError(format!("Invalid URI: {}", e)))
}

fn uri_to_path(uri: &lsp_types::Uri) -> PathBuf {
    Url::parse(&uri.to_string())
        .ok()
        .and_then(|u| u.to_file_path().ok())
        .unwrap_or_default()
}

#[derive(Debug, Error)]
pub enum LspError {
    #[error("Failed to start LSP server: {0}")]
    ServerStartError(String),

    #[error("Failed to initialize LSP: {0}")]
    InitializeError(String),

    #[error("JSON-RPC error: {0}")]
    JsonRpcError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Server not initialized")]
    NotInitialized,

    #[error("Timeout waiting for response")]
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

pub struct LspServerConfig {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub initialization_options: Option<Value>,
}

pub struct LspClient {
    root: PathBuf,
    stdin: Arc<Mutex<ChildStdin>>,
    request_id: Arc<Mutex<u64>>,
    pending_responses:
        Arc<RwLock<HashMap<u64, tokio::sync::oneshot::Sender<Result<Value, LspError>>>>>,
    diagnostics: Arc<RwLock<HashMap<PathBuf, Vec<Diagnostic>>>>,
    file_versions: Arc<RwLock<HashMap<PathBuf, u32>>>,
    event_tx: broadcast::Sender<LspEvent>,
}

#[derive(Debug, Clone)]
pub enum LspEvent {
    Diagnostics { path: PathBuf, server_id: String },
}

impl LspClient {
    pub async fn start(config: LspServerConfig, root: PathBuf) -> Result<Self, LspError> {
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| LspError::ServerStartError(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LspError::ServerStartError("Failed to get stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LspError::ServerStartError("Failed to get stdout".to_string()))?;

        let (event_tx, _) = broadcast::channel(256);

        let client = Self {
            root,
            stdin: Arc::new(Mutex::new(stdin)),
            request_id: Arc::new(Mutex::new(0)),
            pending_responses: Arc::new(RwLock::new(HashMap::new())),
            diagnostics: Arc::new(RwLock::new(HashMap::new())),
            file_versions: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        };

        let pending = client.pending_responses.clone();
        let diagnostics = client.diagnostics.clone();
        let server_id = config.id.clone();
        let event_tx_clone = client.event_tx.clone();

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if line.is_empty() || line.starts_with("Content-Length:") {
                    continue;
                }

                if let Ok(notification) = serde_json::from_str::<JsonRpcNotification>(&line) {
                    if notification.method == "textDocument/publishDiagnostics" {
                        if let Some(params) = notification.params {
                            if let Ok(diag_params) = serde_json::from_value::<
                                lsp_types::PublishDiagnosticsParams,
                            >(params)
                            {
                                let path = uri_to_path(&diag_params.uri);

                                diagnostics
                                    .write()
                                    .await
                                    .insert(path.clone(), diag_params.diagnostics);

                                let _ = event_tx_clone.send(LspEvent::Diagnostics {
                                    path,
                                    server_id: server_id.clone(),
                                });
                            }
                        }
                    }
                } else if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&line) {
                    if let Some(sender) = pending.write().await.remove(&response.id) {
                        let result = if let Some(error) = response.error {
                            Err(LspError::JsonRpcError(error.message))
                        } else {
                            Ok(response.result.unwrap_or(Value::Null))
                        };
                        let _ = sender.send(result);
                    }
                }
            }
        });

        let mut client = client;
        client.initialize(config.initialization_options).await?;

        Ok(client)
    }

    async fn initialize(&mut self, initialization_options: Option<Value>) -> Result<(), LspError> {
        let workspace_uri = path_to_uri(&self.root)?;

        let params = InitializeParams {
            initialization_options,
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: workspace_uri,
                name: "workspace".to_string(),
            }]),
            capabilities: ClientCapabilities::default(),
            ..Default::default()
        };

        let result = self
            .request("initialize", serde_json::to_value(params)?)
            .await?;
        debug!(?result, "LSP initialized");

        self.notify("initialized", Value::Null).await?;

        Ok(())
    }

    async fn next_id(&self) -> u64 {
        let mut id = self.request_id.lock().await;
        *id += 1;
        *id
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, LspError> {
        let id = self.next_id().await;
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.pending_responses.write().await.insert(id, tx);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        let content = serde_json::to_string(&request)?;
        let message = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(message.as_bytes()).await?;
        stdin.flush().await?;

        rx.await.map_err(|_| LspError::Timeout)?
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: if params.is_null() { None } else { Some(params) },
        };

        let content = serde_json::to_string(&notification)?;
        let message = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(message.as_bytes()).await?;
        stdin.flush().await?;

        Ok(())
    }

    pub async fn open_document(
        &self,
        path: &Path,
        content: &str,
        language_id: &str,
    ) -> Result<(), LspError> {
        let uri = path_to_uri(path)?;

        let version = {
            let mut versions = self.file_versions.write().await;
            let v = versions.entry(path.to_path_buf()).or_insert(0);
            *v
        };

        if version > 0 {
            let next_version = version + 1;
            self.file_versions
                .write()
                .await
                .insert(path.to_path_buf(), next_version);

            let params = DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri,
                    version: next_version as i32,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: content.to_string(),
                }],
            };

            self.notify("textDocument/didChange", serde_json::to_value(params)?)
                .await?;
        } else {
            self.file_versions
                .write()
                .await
                .insert(path.to_path_buf(), 0);
            self.diagnostics.write().await.remove(path);

            let params = DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri,
                    language_id: language_id.to_string(),
                    version: 0,
                    text: content.to_string(),
                },
            };

            self.notify("textDocument/didOpen", serde_json::to_value(params)?)
                .await?;
        }

        Ok(())
    }

    pub async fn get_diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
        self.diagnostics
            .read()
            .await
            .get(path)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns all diagnostics from all files this LSP server has reported on.
    pub async fn get_all_diagnostics(&self) -> HashMap<PathBuf, Vec<Diagnostic>> {
        self.diagnostics.read().await.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LspEvent> {
        self.event_tx.subscribe()
    }

    pub async fn goto_definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::Location>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("textDocument/definition", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let response: lsp_types::GotoDefinitionResponse = serde_json::from_value(result)?;
        match response {
            lsp_types::GotoDefinitionResponse::Scalar(loc) => Ok(Some(loc)),
            lsp_types::GotoDefinitionResponse::Array(locs) => Ok(locs.into_iter().next()),
            lsp_types::GotoDefinitionResponse::Link(links) => {
                Ok(links.into_iter().next().map(|l| lsp_types::Location {
                    uri: l.target_uri,
                    range: l.target_selection_range,
                }))
            }
        }
    }

    pub async fn completion(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<Vec<lsp_types::CompletionItem>>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::CompletionParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        };

        let result = self
            .request("textDocument/completion", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let response: lsp_types::CompletionResponse = serde_json::from_value(result)?;
        Ok(Some(match response {
            lsp_types::CompletionResponse::Array(items) => items,
            lsp_types::CompletionResponse::List(list) => list.items,
        }))
    }

    pub async fn references(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::ReferenceParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp_types::ReferenceContext {
                include_declaration: true,
            },
        };

        let result = self
            .request("textDocument/references", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let locations: Vec<lsp_types::Location> = serde_json::from_value(result)?;
        Ok(locations)
    }

    pub async fn hover(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::Hover>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::HoverParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = self
            .request("textDocument/hover", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let hover: lsp_types::Hover = serde_json::from_value(result)?;
        Ok(Some(hover))
    }

    pub async fn document_symbol(
        &self,
        path: &Path,
    ) -> Result<Vec<lsp_types::SymbolInformation>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("textDocument/documentSymbol", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let symbols: lsp_types::DocumentSymbolResponse = serde_json::from_value(result)?;
        Ok(match symbols {
            lsp_types::DocumentSymbolResponse::Flat(symbols) => symbols,
            lsp_types::DocumentSymbolResponse::Nested(nested) => nested
                .into_iter()
                .flat_map(|s| flatten_document_symbol(&s))
                .collect(),
        })
    }

    pub async fn workspace_symbol(
        &self,
        query: &str,
    ) -> Result<Vec<lsp_types::SymbolInformation>, LspError> {
        let params = lsp_types::WorkspaceSymbolParams {
            query: query.to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("workspace/symbol", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let symbols: Vec<lsp_types::SymbolInformation> = serde_json::from_value(result)?;
        Ok(symbols)
    }

    pub async fn goto_implementation(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::request::GotoImplementationParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("textDocument/implementation", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let response: lsp_types::GotoDefinitionResponse = serde_json::from_value(result)?;
        Ok(match response {
            lsp_types::GotoDefinitionResponse::Scalar(loc) => vec![loc],
            lsp_types::GotoDefinitionResponse::Array(locs) => locs,
            lsp_types::GotoDefinitionResponse::Link(links) => links
                .into_iter()
                .map(|l| lsp_types::Location {
                    uri: l.target_uri,
                    range: l.target_selection_range,
                })
                .collect(),
        })
    }

    pub async fn type_definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: lsp_types::Position { line, character },
        };

        let result = self
            .request("textDocument/typeDefinition", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let response: lsp_types::GotoDefinitionResponse = serde_json::from_value(result)?;
        Ok(match response {
            lsp_types::GotoDefinitionResponse::Scalar(loc) => vec![loc],
            lsp_types::GotoDefinitionResponse::Array(locs) => locs,
            lsp_types::GotoDefinitionResponse::Link(links) => links
                .into_iter()
                .map(|l| lsp_types::Location {
                    uri: l.target_uri,
                    range: l.target_selection_range,
                })
                .collect(),
        })
    }

    pub async fn rename(
        &self,
        path: &Path,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Result<Option<lsp_types::WorkspaceEdit>, LspError> {
        let uri = path_to_uri(path)?;

        let params = lsp_types::RenameParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        };

        let result = self
            .request("textDocument/rename", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let edit: lsp_types::WorkspaceEdit = serde_json::from_value(result)?;
        Ok(Some(edit))
    }

    pub async fn prepare_call_hierarchy(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<CallHierarchyItem>, LspError> {
        let uri = path_to_uri(path)?;

        let params = CallHierarchyPrepareParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = self
            .request(
                "textDocument/prepareCallHierarchy",
                serde_json::to_value(params)?,
            )
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let items: Vec<CallHierarchyItem> = serde_json::from_value(result)?;
        Ok(items)
    }

    pub async fn incoming_calls(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<CallHierarchyIncomingCall>, LspError> {
        let items = self.prepare_call_hierarchy(path, line, character).await?;

        if items.is_empty() {
            return Ok(vec![]);
        }

        let item = &items[0];
        let params = CallHierarchyIncomingCallsParams {
            item: item.clone(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("callHierarchy/incomingCalls", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let calls: Vec<CallHierarchyIncomingCall> = serde_json::from_value(result)?;
        Ok(calls)
    }

    pub async fn outgoing_calls(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<CallHierarchyOutgoingCall>, LspError> {
        let items = self.prepare_call_hierarchy(path, line, character).await?;

        if items.is_empty() {
            return Ok(vec![]);
        }

        let item = &items[0];
        let params = CallHierarchyOutgoingCallsParams {
            item: item.clone(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = self
            .request("callHierarchy/outgoingCalls", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let calls: Vec<CallHierarchyOutgoingCall> = serde_json::from_value(result)?;
        Ok(calls)
    }
}

#[allow(deprecated)]
fn flatten_document_symbol(
    symbol: &lsp_types::DocumentSymbol,
) -> Vec<lsp_types::SymbolInformation> {
    let mut result = vec![];

    result.push(lsp_types::SymbolInformation {
        name: symbol.name.clone(),
        kind: symbol.kind,
        tags: symbol.tags.clone(),
        deprecated: symbol.deprecated,
        location: lsp_types::Location {
            uri: path_to_uri(&std::path::PathBuf::new())
                .unwrap_or_else(|_| lsp_types::Uri::from_str("file:///").unwrap()),
            range: symbol.selection_range,
        },
        container_name: symbol.detail.clone(),
    });

    if let Some(children) = &symbol.children {
        for child in children {
            result.extend(flatten_document_symbol(child));
        }
    }

    result
}

pub struct LspClientRegistry {
    clients: RwLock<HashMap<String, Arc<LspClient>>>,
}

impl LspClientRegistry {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, id: String, client: Arc<LspClient>) {
        self.clients.write().await.insert(id, client);
    }

    pub async fn get(&self, id: &str) -> Option<Arc<LspClient>> {
        self.clients.read().await.get(id).cloned()
    }

    pub async fn list(&self) -> Vec<(String, Arc<LspClient>)> {
        self.clients
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Returns true if any LSP clients are currently registered.
    /// Mirrors the TS `LSP.hasClients(file)` which checks whether any server
    /// could handle the given file. Here we check if any registered client's
    /// id contains the detected language for the file.
    pub async fn has_clients(&self, path: &Path) -> bool {
        let clients = self.clients.read().await;
        if clients.is_empty() {
            return false;
        }
        let language = detect_language(path);
        clients.keys().any(|id| id.contains(language))
    }

    /// Opens or refreshes a file in all matching LSP clients.
    /// Mirrors the TS `LSP.touchFile(input, waitForDiagnostics)`.
    ///
    /// - Reads the file content from disk
    /// - For each registered client whose id matches the file's language,
    ///   calls `open_document` (which internally handles didOpen vs didChange)
    /// - If `wait_for_diagnostics` is true, waits briefly for diagnostics to arrive
    pub async fn touch_file(
        &self,
        path: &Path,
        wait_for_diagnostics: bool,
    ) -> Result<(), LspError> {
        let language = detect_language(path);
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| LspError::IoError(e))?;

        let clients = self.clients.read().await;
        let matching: Vec<Arc<LspClient>> = clients
            .iter()
            .filter(|(id, _)| id.contains(language))
            .map(|(_, c)| c.clone())
            .collect();
        drop(clients);

        for client in &matching {
            if let Err(e) = client.open_document(path, &content, language).await {
                error!("Failed to touch file {:?} in LSP: {}", path, e);
            }
        }

        if wait_for_diagnostics && !matching.is_empty() {
            // Give LSP servers a moment to produce diagnostics.
            // The TS version uses client.waitForDiagnostics which listens
            // for a publishDiagnostics notification. We approximate this
            // with a short sleep, since the broadcast-based event system
            // already stores diagnostics as they arrive.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        Ok(())
    }
}

impl Default for LspClientRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn detect_language(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("ts") => "typescript",
        Some("tsx") => "typescriptreact",
        Some("js") => "javascript",
        Some("jsx") => "javascriptreact",
        Some("py") => "python",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") => "c",
        Some("cpp") | Some("cc") | Some("cxx") => "cpp",
        Some("h") | Some("hpp") => "cpp",
        Some("rb") => "ruby",
        Some("php") => "php",
        Some("swift") => "swift",
        Some("kt") => "kotlin",
        Some("scala") => "scala",
        Some("lua") => "lua",
        Some("json") => "json",
        Some("yaml") | Some("yml") => "yaml",
        Some("toml") => "toml",
        Some("md") => "markdown",
        Some("html") => "html",
        Some("css") => "css",
        Some("scss") => "scss",
        _ => "plaintext",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language(Path::new("main.rs")), "rust");
        assert_eq!(detect_language(Path::new("index.ts")), "typescript");
        assert_eq!(detect_language(Path::new("app.tsx")), "typescriptreact");
        assert_eq!(detect_language(Path::new("main.py")), "python");
        assert_eq!(detect_language(Path::new("main.go")), "go");
        assert_eq!(detect_language(Path::new("unknown.xyz")), "plaintext");
    }

    #[test]
    fn test_json_rpc_request_serialization() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "initialize".to_string(),
            params: Value::Null,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"initialize\""));
    }

    #[test]
    fn test_json_rpc_response_deserialization() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#;
        let response: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.id, 1);
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_has_clients_empty_registry() {
        let registry = LspClientRegistry::new();
        assert!(!registry.has_clients(Path::new("main.rs")).await);
        assert!(!registry.has_clients(Path::new("index.ts")).await);
    }

    #[tokio::test]
    async fn test_registry_default() {
        let registry = LspClientRegistry::default();
        let clients = registry.list().await;
        assert!(clients.is_empty());
    }
}
