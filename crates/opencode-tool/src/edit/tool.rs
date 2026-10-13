use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs;

use super::replacers::CompositeReplacer;
use crate::{with_file_lock, Metadata, Tool, ToolContext, ToolError, ToolResult};

#[cfg(feature = "lsp")]
const MAX_DIAGNOSTICS_PER_FILE: usize = 20;

pub struct EditTool {
    directory: PathBuf,
}

impl EditTool {
    pub fn new() -> Self {
        Self {
            directory: std::env::current_dir().unwrap_or_default(),
        }
    }
}

impl Default for EditTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for EditTool {
    fn id(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Performs string replacements in a file with multiple matching strategies. Use this to make precise edits."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The text to replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The text to replace it with (must be different from old_string)"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences of old_string (default false)"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let file_path: String = args["file_path"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("file_path is required".into()))?
            .to_string();

        let old_string: String = args["old_string"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("old_string is required".into()))?
            .to_string();

        let new_string: String = args["new_string"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("new_string is required".into()))?
            .to_string();

        let replace_all = args["replace_all"].as_bool().unwrap_or(false);

        let base_dir = if ctx.directory.is_empty() {
            &self.directory
        } else {
            Path::new(&ctx.directory)
        };

        let path = if Path::new(&file_path).is_absolute() {
            PathBuf::from(&file_path)
        } else {
            base_dir.join(&file_path)
        };

        let path_str = path.to_string_lossy().to_string();

        if ctx.is_external_path(&path_str) {
            let parent = path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| path_str.clone());

            ctx.ask_permission(
                crate::PermissionRequest::new("external_directory")
                    .with_pattern(format!("{}/*", parent))
                    .with_metadata("filepath", serde_json::json!(&path_str))
                    .with_metadata("parentDir", serde_json::json!(parent)),
            )
            .await?;
        }

        let title = path
            .strip_prefix(&ctx.worktree)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let ctx_clone = ctx.clone();
        let path_clone = path.clone();
        let path_str_clone = path_str.clone();
        let title_clone = title.clone();
        let old_string_clone = old_string.clone();
        let new_string_clone = new_string.clone();

        with_file_lock(&path_str, || async {
            let content = fs::read_to_string(&path_clone)
                .await
                .map_err(|e| ToolError::ExecutionError(format!("Failed to read file: {}", e)))?;

            let content = normalize_line_endings(&content);

            let existed = !content.is_empty();
            if existed {
                ctx_clone
                    .do_file_time_assert(path_str_clone.clone())
                    .await?;
            }

            if old_string_clone.is_empty() {
                let new_string_normalized = normalize_line_endings(&new_string_clone);
                let diff = create_diff(&path_str_clone, "", &new_string_normalized);
                ctx_clone
                    .ask_permission(
                        crate::PermissionRequest::new("edit")
                            .with_pattern(&path_str_clone)
                            .with_metadata("diff", serde_json::json!(diff))
                            .always_allow(),
                    )
                    .await?;

                fs::write(&path_clone, &new_string_normalized)
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionError(format!("Failed to write file: {}", e))
                    })?;

                ctx_clone
                    .do_publish_bus(
                        "file.edited",
                        serde_json::json!({
                            "file": path_str_clone.clone()
                        }),
                    )
                    .await;

                ctx_clone
                    .do_publish_bus(
                        "file_watcher.updated",
                        serde_json::json!({
                            "file": path_str_clone.clone(),
                            "event": if existed { "change" } else { "add" }
                        }),
                    )
                    .await;

                ctx_clone
                    .do_lsp_touch_file(path_str_clone.clone(), true)
                    .await?;
                ctx_clone.do_file_time_read(path_str_clone.clone()).await?;

                let output = format!("Created new file content at {}", path_clone.display());
                let (lsp_output, lsp_diagnostics) =
                    get_lsp_diagnostics_with_meta(&path_clone, &ctx_clone).await;
                let final_output = if !lsp_output.is_empty() {
                    format!("{}\n\n{}", output, lsp_output)
                } else {
                    output
                };

                let path_for_metadata = path_str_clone.clone();

                return Ok(ToolResult {
                    title: title_clone,
                    output: final_output,
                    metadata: {
                        let mut m = Metadata::new();
                        m.insert("filepath".into(), serde_json::json!(path_for_metadata));
                        if !lsp_diagnostics.is_empty() {
                            m.insert("diagnostics".into(), serde_json::json!(lsp_diagnostics));
                        }
                        m
                    },
                    truncated: false,
                });
            }

            let replacer = CompositeReplacer::new();
            let old_string_normalized = normalize_line_endings(&old_string_clone);
            let new_string_normalized = normalize_line_endings(&new_string_clone);
            let new_content = replacer
                .replace(
                    &content,
                    &old_string_normalized,
                    &new_string_normalized,
                    replace_all,
                )
                .map_err(|e| ToolError::ExecutionError(e))?;

            let diff = create_diff(&path_str_clone, &content, &new_content);
            ctx_clone
                .ask_permission(
                    crate::PermissionRequest::new("edit")
                        .with_pattern(&path_str_clone)
                        .with_metadata("diff", serde_json::json!(diff))
                        .always_allow(),
                )
                .await?;

            let replacements = if replace_all {
                content.matches(&old_string_normalized).count()
            } else {
                1
            };

            fs::write(&path_clone, &new_content)
                .await
                .map_err(|e| ToolError::ExecutionError(format!("Failed to write file: {}", e)))?;

            ctx_clone
                .do_publish_bus(
                    "file.edited",
                    serde_json::json!({
                        "file": path_str_clone.clone()
                    }),
                )
                .await;

            ctx_clone
                .do_publish_bus(
                    "file_watcher.updated",
                    serde_json::json!({
                        "file": path_str_clone.clone(),
                        "event": if existed { "change" } else { "add" }
                    }),
                )
                .await;

            ctx_clone
                .do_lsp_touch_file(path_str_clone.clone(), true)
                .await?;
            ctx_clone.do_file_time_read(path_str_clone.clone()).await?;

            let base_output = format!(
                "Successfully edited {} ({} replacement{})",
                path_clone.display(),
                replacements,
                if replacements != 1 { "s" } else { "" }
            );

            let (lsp_output, lsp_diagnostics) =
                get_lsp_diagnostics_with_meta(&path_clone, &ctx_clone).await;
            let final_output = if lsp_output.is_empty() {
                base_output
            } else {
                format!("{}\n\n{}", base_output, lsp_output)
            };

            let diff_for_metadata = diff.clone();
            let path_for_metadata = path_str_clone.clone();

            Ok(ToolResult {
                title: title_clone,
                output: final_output,
                metadata: {
                    let mut m = Metadata::new();
                    m.insert("replacements".into(), serde_json::json!(replacements));
                    m.insert("filepath".into(), serde_json::json!(path_for_metadata));
                    m.insert("diff".into(), serde_json::json!(diff_for_metadata));
                    if !lsp_diagnostics.is_empty() {
                        m.insert("diagnostics".into(), serde_json::json!(lsp_diagnostics));
                    }
                    m
                },
                truncated: false,
            })
        })
        .await
    }
}

async fn get_lsp_diagnostics_with_meta(
    path: &Path,
    ctx: &ToolContext,
) -> (String, Vec<serde_json::Value>) {
    #[cfg(feature = "lsp")]
    {
        use opencode_lsp::detect_language;

        if let Some(lsp_registry) = &ctx.lsp_registry {
            return get_lsp_diagnostics_impl_with_meta(path, lsp_registry.clone()).await;
        }
    }

    #[cfg(not(feature = "lsp"))]
    {
        let _ = (path, ctx);
    }

    (String::new(), Vec::new())
}

#[cfg(feature = "lsp")]
async fn get_lsp_diagnostics_impl_with_meta(
    path: &Path,
    lsp_registry: std::sync::Arc<opencode_lsp::LspClientRegistry>,
) -> (String, Vec<serde_json::Value>) {
    use opencode_lsp::detect_language;

    let language = detect_language(path);
    let clients = lsp_registry.list().await;

    let client = clients
        .iter()
        .find(|(id, _)| id.contains(language))
        .map(|(_, c)| c.clone());

    match client {
        Some(client) => {
            if let Ok(content) = tokio::fs::read_to_string(path).await {
                let _ = client.open_document(path, &content, language).await;

                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                let diagnostics = client.get_diagnostics(path).await;
                let errors: Vec<_> = diagnostics
                    .iter()
                    .filter(|d| d.severity == Some(lsp_types::DiagnosticSeverity::ERROR))
                    .collect();

                if errors.is_empty() {
                    return (String::new(), Vec::new());
                }

                let total_errors = errors.len();
                let limited: Vec<_> = errors.into_iter().take(MAX_DIAGNOSTICS_PER_FILE).collect();
                let suffix = if limited.len() < total_errors {
                    format!("\n... and {} more", total_errors - limited.len())
                } else {
                    String::new()
                };

                let error_lines: Vec<String> = limited
                    .iter()
                    .map(|d| {
                        let line = d.range.start.line + 1;
                        let msg = &d.message;
                        format!("  Line {}: {}", line, msg)
                    })
                    .collect();

                let diagnostics_meta: Vec<serde_json::Value> = limited.iter()
                    .map(|d| {
                        serde_json::json!({
                            "line": d.range.start.line + 1,
                            "message": d.message,
                            "severity": d.severity.as_ref().map(|s| format!("{:?}", s)).unwrap_or_else(|| "Unknown".to_string())
                        })
                    })
                    .collect();

                let output = format!(
                    "LSP errors detected in this file, please fix:\n<diagnostics file=\"{}\">\n{}{}\n</diagnostics>",
                    path.display(),
                    error_lines.join("\n"),
                    suffix
                );

                (output, diagnostics_meta)
            } else {
                (String::new(), Vec::new())
            }
        }
        None => (String::new(), Vec::new()),
    }
}

fn create_diff(filepath: &str, old_content: &str, new_content: &str) -> String {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    let mut diff = format!("--- {}\n+++ {}\n", filepath, filepath);

    let mut old_idx = 0;
    let mut new_idx = 0;

    while old_idx < old_lines.len() || new_idx < new_lines.len() {
        if old_idx >= old_lines.len() {
            diff.push_str(&format!("+{}\n", new_lines[new_idx]));
            new_idx += 1;
        } else if new_idx >= new_lines.len() {
            diff.push_str(&format!("-{}\n", old_lines[old_idx]));
            old_idx += 1;
        } else if old_lines[old_idx] == new_lines[new_idx] {
            old_idx += 1;
            new_idx += 1;
        } else {
            diff.push_str(&format!("-{}\n", old_lines[old_idx]));
            diff.push_str(&format!("+{}\n", new_lines[new_idx]));
            old_idx += 1;
            new_idx += 1;
        }
    }

    diff
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n")
}
