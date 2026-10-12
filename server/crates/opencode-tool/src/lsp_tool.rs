use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(feature = "lsp")]
use lsp_types;

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspParams {
    pub operation: LspOperation,
    pub file_path: String,
    pub line: Option<u32>,
    pub character: Option<u32>,
    pub query: Option<String>,
    pub new_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LspOperation {
    GoToDefinition,
    FindReferences,
    Hover,
    DocumentSymbol,
    WorkspaceSymbol,
    GoToImplementation,
    TypeDefinition,
    Rename,
    Diagnostics,
    PrepareCallHierarchy,
    IncomingCalls,
    OutgoingCalls,
}

pub struct LspTool;

#[async_trait]
impl Tool for LspTool {
    fn id(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "Language Server Protocol operations for code navigation and analysis. Supports goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, goToImplementation, typeDefinition, rename, diagnostics, prepareCallHierarchy, incomingCalls, and outgoingCalls."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["goToDefinition", "findReferences", "hover", "documentSymbol", "workspaceSymbol", "goToImplementation", "typeDefinition", "rename", "diagnostics", "prepareCallHierarchy", "incomingCalls", "outgoingCalls"],
                    "description": "The LSP operation to perform"
                },
                "filePath": {
                    "type": "string",
                    "description": "The absolute or relative path to the file"
                },
                "line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The line number (1-based, as shown in editors)"
                },
                "character": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The character offset (1-based, as shown in editors)"
                },
                "query": {
                    "type": "string",
                    "description": "Query string for workspaceSymbol operation"
                },
                "newName": {
                    "type": "string",
                    "description": "New name for rename operation"
                }
            },
            "required": ["operation", "filePath"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let params: LspParams = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid parameters: {}", e)))?;

        let path = PathBuf::from(&params.file_path);
        let path_str = path.to_string_lossy().to_string();

        if !path.exists() {
            return Err(ToolError::FileNotFound(format!(
                "File not found: {}",
                params.file_path
            )));
        }

        if ctx.is_external_path(&path_str) {
            ctx.ask_permission(
                crate::PermissionRequest::new("external_directory")
                    .with_pattern(&path_str)
                    .with_metadata("filepath", serde_json::json!(&path_str)),
            )
            .await?;
        }

        ctx.ask_permission(
            crate::PermissionRequest::new("lsp")
                .with_patterns(vec!["*".to_string()])
                .always_allow(),
        )
        .await?;

        let line = params.line.map(|l| l.saturating_sub(1)).unwrap_or(0);
        let character = params.character.map(|c| c.saturating_sub(1)).unwrap_or(0);

        #[cfg(feature = "lsp")]
        {
            execute_with_lsp(&params, &path, line, character, &ctx).await
        }

        #[cfg(not(feature = "lsp"))]
        {
            let output =
                format_lsp_placeholder(&params.operation, &params.file_path, line, character);
            let mut metadata = Metadata::new();
            metadata.insert("operation".to_string(), serde_json::json!(params.operation));
            metadata.insert("file_path".to_string(), serde_json::json!(params.file_path));

            Ok(ToolResult {
                output,
                title: format!("LSP: {:?} {}", params.operation, params.file_path),
                metadata,
                truncated: false,
            })
        }
    }
}

#[cfg(feature = "lsp")]
async fn execute_with_lsp(
    params: &LspParams,
    path: &PathBuf,
    line: u32,
    character: u32,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    use opencode_lsp::{detect_language, LspClientRegistry};
    use std::sync::Arc;

    let lsp_registry: Option<Arc<LspClientRegistry>> = ctx.lsp_registry.clone();

    let output = match &lsp_registry {
        Some(registry) => {
            // Check if any LSP client can handle this file type (mirrors TS LSP.hasClients)
            if !registry.has_clients(path).await {
                return Err(ToolError::ExecutionError(
                    "No LSP server available for this file type.".to_string(),
                ));
            }

            // Touch the file to open/refresh it in all matching LSP clients,
            // waiting for diagnostics (mirrors TS LSP.touchFile(file, true))
            registry.touch_file(path, true).await.map_err(|e| {
                ToolError::ExecutionError(format!("Failed to touch file in LSP: {}", e))
            })?;

            let language = detect_language(path);
            let clients = registry.list().await;
            let client = clients
                .iter()
                .find(|(id, _)| id.contains(language))
                .map(|(_, c)| c.clone());

            match client {
                Some(client) => match &params.operation {
                    LspOperation::GoToDefinition => {
                        match client.goto_definition(path, line, character).await {
                            Ok(Some(loc)) => format_location_result("Definition", loc),
                            Ok(None) => "No definition found.".to_string(),
                            Err(e) => format!("LSP error: {}", e),
                        }
                    }
                    LspOperation::FindReferences => {
                        match client.references(path, line, character).await {
                            Ok(locs) if !locs.is_empty() => locs
                                .iter()
                                .map(|l| format_location(l))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            Ok(_) => "No references found.".to_string(),
                            Err(e) => format!("LSP error: {}", e),
                        }
                    }
                    LspOperation::Hover => match client.hover(path, line, character).await {
                        Ok(Some(hover)) => format_hover_result(hover),
                        Ok(None) => "No hover information available.".to_string(),
                        Err(e) => format!("LSP error: {}", e),
                    },
                    LspOperation::DocumentSymbol => match client.document_symbol(path).await {
                        Ok(symbols) if !symbols.is_empty() => symbols
                            .iter()
                            .map(|s| format!("{} ({:?})", s.name, s.kind))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        Ok(_) => "No document symbols found.".to_string(),
                        Err(e) => format!("LSP error: {}", e),
                    },
                    LspOperation::WorkspaceSymbol => {
                        let query = params.query.as_deref().unwrap_or("");
                        match client.workspace_symbol(query).await {
                            Ok(symbols) if !symbols.is_empty() => symbols
                                .iter()
                                .map(|s| format!("{} ({:?})", s.name, s.kind))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            Ok(_) => "No workspace symbols found.".to_string(),
                            Err(e) => format!("LSP error: {}", e),
                        }
                    }
                    LspOperation::GoToImplementation => {
                        match client.goto_implementation(path, line, character).await {
                            Ok(locs) if !locs.is_empty() => locs
                                .iter()
                                .map(|l| format_location(l))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            Ok(_) => "No implementations found.".to_string(),
                            Err(e) => format!("LSP error: {}", e),
                        }
                    }
                    LspOperation::TypeDefinition => {
                        match client.type_definition(path, line, character).await {
                            Ok(locs) if !locs.is_empty() => locs
                                .iter()
                                .map(|l| format_location(l))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            Ok(_) => "No type definitions found.".to_string(),
                            Err(e) => format!("LSP error: {}", e),
                        }
                    }
                    LspOperation::Rename => {
                        let new_name = params.new_name.as_deref().unwrap_or("new_name");
                        match client.rename(path, line, character, new_name).await {
                            Ok(Some(edit)) => format!(
                                "Rename preview available. Workspace edit ready for: {}",
                                new_name
                            ),
                            Ok(None) => "Cannot rename symbol at this location.".to_string(),
                            Err(e) => format!("LSP error: {}", e),
                        }
                    }
                    LspOperation::Diagnostics => {
                        let diags = client.get_diagnostics(path).await;
                        if diags.is_empty() {
                            "No diagnostics available.".to_string()
                        } else {
                            diags
                                .iter()
                                .map(|d| format!("{:?}: {}", d.severity, d.message))
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    }
                    LspOperation::PrepareCallHierarchy => {
                        match client.prepare_call_hierarchy(path, line, character).await {
                            Ok(items) if !items.is_empty() => items
                                .iter()
                                .map(|item| {
                                    format!(
                                        "{} ({:?}) - {}:{}",
                                        item.name,
                                        item.kind,
                                        item.uri.to_string(),
                                        item.range.start.line + 1
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n"),
                            Ok(_) => "No call hierarchy items found at this location.".to_string(),
                            Err(e) => format!("LSP error: {}", e),
                        }
                    }
                    LspOperation::IncomingCalls => {
                        match client.incoming_calls(path, line, character).await {
                            Ok(calls) if !calls.is_empty() => calls
                                .iter()
                                .map(|call| {
                                    let from = &call.from;
                                    format!(
                                        "{} ({:?}) calls from {}:{}",
                                        from.name,
                                        from.kind,
                                        from.uri.to_string(),
                                        from.range.start.line + 1
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n"),
                            Ok(_) => "No incoming calls found.".to_string(),
                            Err(e) => format!("LSP error: {}", e),
                        }
                    }
                    LspOperation::OutgoingCalls => {
                        match client.outgoing_calls(path, line, character).await {
                            Ok(calls) if !calls.is_empty() => calls
                                .iter()
                                .map(|call| {
                                    let to = &call.to;
                                    format!(
                                        "{} ({:?}) calls to {}:{}",
                                        to.name,
                                        to.kind,
                                        to.uri.to_string(),
                                        to.range.start.line + 1
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n"),
                            Ok(_) => "No outgoing calls found.".to_string(),
                            Err(e) => format!("LSP error: {}", e),
                        }
                    }
                },
                None => format!("No LSP client available for language: {}", language),
            }
        }
        None => "LSP registry not available. Enable 'lsp' feature and configure LSP servers."
            .to_string(),
    };

    let mut metadata = Metadata::new();
    metadata.insert("operation".to_string(), serde_json::json!(params.operation));
    metadata.insert("file_path".to_string(), serde_json::json!(params.file_path));

    Ok(ToolResult {
        output,
        title: format!("LSP: {:?} {}", params.operation, params.file_path),
        metadata,
        truncated: false,
    })
}

#[cfg(feature = "lsp")]
fn format_location(loc: &lsp_types::Location) -> String {
    let path = loc.uri.to_string();
    let line = loc.range.start.line + 1;
    let character = loc.range.start.character + 1;
    format!("{}:{}:{}", path, line, character)
}

#[cfg(feature = "lsp")]
fn format_location_result(label: &str, loc: lsp_types::Location) -> String {
    format!("{} found at:\n{}", label, format_location(&loc))
}

#[cfg(feature = "lsp")]
fn format_hover_result(hover: lsp_types::Hover) -> String {
    match hover.contents {
        lsp_types::HoverContents::Scalar(markup) => format_markup(markup),
        lsp_types::HoverContents::Array(markups) => markups
            .into_iter()
            .map(format_markup)
            .collect::<Vec<_>>()
            .join("\n"),
        lsp_types::HoverContents::Markup(content) => content.value,
    }
}

#[cfg(feature = "lsp")]
fn format_markup(markup: lsp_types::MarkedString) -> String {
    match markup {
        lsp_types::MarkedString::String(s) => s,
        lsp_types::MarkedString::LanguageString(ls) => {
            format!("```{}\n{}\n```", ls.language, ls.value)
        }
    }
}

#[cfg(not(feature = "lsp"))]
fn format_lsp_placeholder(
    operation: &LspOperation,
    file_path: &str,
    line: u32,
    character: u32,
) -> String {
    match operation {
        LspOperation::GoToDefinition => {
            format!("LSP goToDefinition at {}:{}:{}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path, line + 1, character + 1)
        }
        LspOperation::FindReferences => {
            format!("LSP findReferences at {}:{}:{}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path, line + 1, character + 1)
        }
        LspOperation::Hover => {
            format!("LSP hover at {}:{}:{}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path, line + 1, character + 1)
        }
        LspOperation::DocumentSymbol => {
            format!("Document symbols for: {}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path)
        }
        LspOperation::WorkspaceSymbol => {
            "LSP workspace symbols\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.".to_string()
        }
        LspOperation::GoToImplementation => {
            format!("LSP goToImplementation at {}:{}:{}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path, line + 1, character + 1)
        }
        LspOperation::TypeDefinition => {
            format!("LSP typeDefinition at {}:{}:{}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path, line + 1, character + 1)
        }
        LspOperation::Rename => {
            format!("LSP rename at {}:{}:{}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path, line + 1, character + 1)
        }
        LspOperation::Diagnostics => {
            format!("Diagnostics for: {}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path)
        }
        LspOperation::PrepareCallHierarchy => {
            format!("LSP prepareCallHierarchy at {}:{}:{}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path, line + 1, character + 1)
        }
        LspOperation::IncomingCalls => {
            format!("LSP incomingCalls at {}:{}:{}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path, line + 1, character + 1)
        }
        LspOperation::OutgoingCalls => {
            format!("LSP outgoingCalls at {}:{}:{}\n\nEnable 'lsp' feature and configure LSP servers for real LSP support.", file_path, line + 1, character + 1)
        }
    }
}
