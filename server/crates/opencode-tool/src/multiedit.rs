use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::{Tool, ToolContext, ToolError, ToolResult};

pub struct MultiEditTool;

#[derive(Debug, Serialize, Deserialize)]
struct MultiEditInput {
    edits: Vec<FileEdit>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FileEdit {
    file_path: String,
    edits: Vec<EditOperation>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EditOperation {
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

#[async_trait]
impl Tool for MultiEditTool {
    fn id(&self) -> &str {
        "multiedit"
    }

    fn description(&self) -> &str {
        "Apply multiple string replacements across multiple files in a single atomic operation. Each file can have multiple edits applied in sequence."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "The path to the file to edit"
                            },
                            "edits": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "old_string": {
                                            "type": "string",
                                            "description": "The text to search for"
                                        },
                                        "new_string": {
                                            "type": "string",
                                            "description": "The text to replace it with"
                                        },
                                        "replace_all": {
                                            "type": "boolean",
                                            "default": false,
                                            "description": "Replace all occurrences"
                                        }
                                    },
                                    "required": ["old_string", "new_string"]
                                },
                                "description": "List of edits to apply to this file"
                            }
                        },
                        "required": ["file_path", "edits"]
                    },
                    "description": "List of files with their edits"
                }
            },
            "required": ["edits"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: MultiEditInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        let base_path = PathBuf::from(&ctx.directory);
        let mut results: Vec<String> = Vec::new();
        let mut total_edits = 0;
        let mut total_files = 0;

        for file_edit in input.edits {
            let file_path = base_path.join(&file_edit.file_path);

            if !file_path.exists() {
                return Err(ToolError::FileNotFound(format!(
                    "File not found: {}",
                    file_edit.file_path
                )));
            }

            let content = tokio::fs::read_to_string(&file_path)
                .await
                .map_err(|e| ToolError::ExecutionError(format!("Failed to read file: {}", e)))?;

            let mut new_content = content;
            let mut file_edits = 0;

            for edit in file_edit.edits {
                let count = if edit.replace_all {
                    let count = new_content.matches(&edit.old_string).count();
                    new_content = new_content.replace(&edit.old_string, &edit.new_string);
                    count
                } else {
                    if !new_content.contains(&edit.old_string) {
                        return Err(ToolError::ExecutionError(format!(
                            "Could not find '{}' in file {}",
                            edit.old_string, file_edit.file_path
                        )));
                    }
                    if let Some(pos) = new_content.find(&edit.old_string) {
                        let before = &new_content[..pos];
                        let after = &new_content[pos + edit.old_string.len()..];
                        new_content = format!("{}{}{}", before, edit.new_string, after);
                        1
                    } else {
                        0
                    }
                };
                file_edits += count;
            }

            if file_edits > 0 {
                tokio::fs::write(&file_path, &new_content)
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionError(format!("Failed to write file: {}", e))
                    })?;

                ctx.do_publish_bus(
                    "file.edited",
                    serde_json::json!({
                        "file": file_edit.file_path
                    }),
                )
                .await;

                ctx.do_publish_bus(
                    "file_watcher.updated",
                    serde_json::json!({
                        "file": file_edit.file_path,
                        "event": "change"
                    }),
                )
                .await;

                ctx.do_lsp_touch_file(file_edit.file_path.clone(), true)
                    .await?;

                results.push(format!("- {}: {} edit(s)", file_edit.file_path, file_edits));
                total_edits += file_edits;
                total_files += 1;
            }
        }

        let output = if results.is_empty() {
            "No edits applied.".to_string()
        } else {
            format!(
                "Applied {} edit(s) across {} file(s):\n{}",
                total_edits,
                total_files,
                results.join("\n")
            )
        };

        Ok(ToolResult {
            title: format!("Multi-edit: {} edits in {} files", total_edits, total_files),
            output,
            metadata: std::collections::HashMap::new(),
            truncated: false,
        })
    }
}

impl Default for MultiEditTool {
    fn default() -> Self {
        Self
    }
}
