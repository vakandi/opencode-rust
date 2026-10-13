use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

pub struct GlobTool {
    directory: PathBuf,
}

impl GlobTool {
    pub fn new() -> Self {
        Self {
            directory: std::env::current_dir().unwrap_or_default(),
        }
    }
}

impl Default for GlobTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn id(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Fast file pattern matching tool. Supports glob patterns like '**/*.js' or 'src/**/*.ts'. Returns files sorted by modification time (most recent first)."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in. Defaults to current directory."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let pattern: String = args["pattern"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("pattern is required".into()))?
            .to_string();

        let search_path: String = args["path"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| ctx.directory.clone());

        let base_dir = if search_path.is_empty() {
            &self.directory
        } else {
            Path::new(&search_path)
        };

        let base_dir_str = base_dir.to_string_lossy().to_string();

        if ctx.is_external_path(&base_dir_str) {
            ctx.ask_permission(
                crate::PermissionRequest::new("external_directory")
                    .with_pattern(format!("{}/*", base_dir_str))
                    .with_metadata("path", serde_json::json!(&base_dir_str)),
            )
            .await?;
        }

        ctx.ask_permission(
            crate::PermissionRequest::new("glob")
                .with_pattern(&pattern)
                .with_metadata("path", serde_json::json!(&base_dir_str))
                .always_allow(),
        )
        .await?;

        let glob_pattern = glob::Pattern::new(&pattern)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid glob pattern: {}", e)))?;

        let mut files_with_mtime: Vec<(String, SystemTime)> = Vec::new();

        for entry in WalkDir::new(base_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let rel_path = path.strip_prefix(base_dir).unwrap_or(path);
            let rel_str = rel_path.to_string_lossy();

            if glob_pattern.matches(&rel_str) {
                let mtime = path
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or_else(|_| SystemTime::UNIX_EPOCH);
                files_with_mtime.push((path.to_string_lossy().to_string(), mtime));
            }
        }

        files_with_mtime.sort_by(|a, b| b.1.cmp(&a.1));

        let total = files_with_mtime.len();
        let truncated = files_with_mtime.len() > 100;
        let matches: Vec<&str> = files_with_mtime
            .iter()
            .take(100)
            .map(|(p, _)| p.as_str())
            .collect();

        let title = format!("glob '{}'", pattern);
        let output = if matches.is_empty() {
            format!("No files matching pattern '{}' found", pattern)
        } else {
            let mut result = matches.join("\n");
            if truncated {
                result.push_str(&format!("\n\n(Results are truncated: showing first 100 of {}. Consider using a more specific path or pattern.)", total));
            } else {
                result.push_str(&format!("\n\n({} files)", total));
            }
            result
        };

        Ok(ToolResult {
            title,
            output,
            metadata: {
                let mut m = Metadata::new();
                m.insert("count".into(), serde_json::json!(total));
                m.insert("truncated".into(), serde_json::json!(truncated));
                m
            },
            truncated,
        })
    }
}
