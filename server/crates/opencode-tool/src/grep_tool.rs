use async_trait::async_trait;
use regex::Regex;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

const MAX_LINE_LENGTH: usize = 2000;

pub struct GrepTool {
    directory: PathBuf,
}

impl GrepTool {
    pub fn new() -> Self {
        Self {
            directory: std::env::current_dir().unwrap_or_default(),
        }
    }
}

impl Default for GrepTool {
    fn default() -> Self {
        Self::new()
    }
}

struct GrepMatch {
    path: String,
    mtime: SystemTime,
    line_num: usize,
    line_text: String,
}

#[async_trait]
impl Tool for GrepTool {
    fn id(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Fast content search tool. Searches file contents using regular expressions. Results sorted by file modification time (most recent first)."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in"
                },
                "glob": {
                    "type": "string",
                    "description": "File pattern to include (e.g., '*.js')"
                },
                "ignore_case": {
                    "type": "boolean",
                    "description": "Case insensitive search"
                },
                "hidden": {
                    "type": "boolean",
                    "description": "Search hidden files and directories (default: false)"
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

        let glob_filter: Option<String> = args["glob"].as_str().map(|s| s.to_string());

        let ignore_case: bool = args["ignore_case"].as_bool().unwrap_or(false);

        let include_hidden: bool = args["hidden"].as_bool().unwrap_or(false);

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
            crate::PermissionRequest::new("grep")
                .with_pattern(&pattern)
                .always_allow()
                .with_metadata("path", serde_json::json!(&base_dir_str)),
        )
        .await?;

        let regex_pattern = if ignore_case {
            format!("(?i){}", pattern)
        } else {
            pattern.clone()
        };

        let regex = Regex::new(&regex_pattern)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid regex: {}", e)))?;

        let glob_pattern = glob_filter
            .as_ref()
            .and_then(|g| glob::Pattern::new(g).ok());

        let mut matches: Vec<GrepMatch> = Vec::new();
        let mut has_errors = false;
        let limit = 100;

        for entry in WalkDir::new(base_dir)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| {
                if include_hidden {
                    true
                } else {
                    let name = e.file_name().to_string_lossy();
                    !name.starts_with('.')
                }
            })
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if let Some(ref gp) = glob_pattern {
                let rel_path = path.strip_prefix(base_dir).unwrap_or(path);
                if !gp.matches(&rel_path.to_string_lossy()) {
                    continue;
                }
            }

            let mtime = path
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or_else(|_| SystemTime::UNIX_EPOCH);

            if let Ok(file) = File::open(path) {
                let reader = BufReader::new(file);
                let path_str = path.to_string_lossy().to_string();

                for (line_num, line_result) in reader.lines().enumerate() {
                    if let Ok(line) = line_result {
                        if regex.is_match(&line) {
                            let truncated_line = if line.len() > MAX_LINE_LENGTH {
                                format!("{}...", &line[..MAX_LINE_LENGTH])
                            } else {
                                line.clone()
                            };

                            matches.push(GrepMatch {
                                path: path_str.clone(),
                                mtime,
                                line_num: line_num + 1,
                                line_text: truncated_line,
                            });
                        }
                    }
                }
            } else {
                has_errors = true;
            }
        }

        matches.sort_by(|a, b| b.mtime.cmp(&a.mtime));

        let total_matches = matches.len();
        let truncated = matches.len() > limit;

        let title = format!("grep '{}'", pattern);
        let output = if total_matches == 0 {
            format!("No matches found for pattern '{}'", pattern)
        } else {
            let mut output_lines = vec![format!(
                "Found {} matches{}",
                total_matches,
                if truncated {
                    format!(" (showing first {})", limit)
                } else {
                    String::new()
                }
            )];

            let display_matches: Vec<&GrepMatch> = matches.iter().take(limit).collect();
            let mut current_file = "";

            for m in display_matches {
                if current_file != m.path {
                    if !current_file.is_empty() {
                        output_lines.push(String::new());
                    }
                    current_file = &m.path;
                    output_lines.push(format!("{}:", m.path));
                }
                output_lines.push(format!("  Line {}: {}", m.line_num, m.line_text));
            }

            if truncated {
                output_lines.push(String::new());
                output_lines.push(format!(
                    "(Results truncated: showing {} of {} matches ({} hidden). Consider using a more specific path or pattern.)",
                    limit, total_matches, total_matches - limit
                ));
            }

            if has_errors {
                output_lines.push(String::new());
                output_lines.push("(Some paths were inaccessible and skipped)".to_string());
            }

            output_lines.join("\n")
        };

        Ok(ToolResult {
            title,
            output,
            metadata: {
                let mut m = Metadata::new();
                m.insert("matches".into(), serde_json::json!(total_matches));
                m.insert("truncated".into(), serde_json::json!(truncated));
                m.insert("hasErrors".into(), serde_json::json!(has_errors));
                m
            },
            truncated,
        })
    }
}
