use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs;
use walkdir::WalkDir;

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};

const DEFAULT_READ_LIMIT: usize = 2000;
const MAX_LINE_LENGTH: usize = 2000;
const MAX_BYTES: usize = 50 * 1024;

const INSTRUCTION_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "CONTEXT.md",
    "CONTEXT.txt",
    ".context",
    ".cursorrules",
    ".opencoderules",
];

pub struct ReadTool {
    directory: PathBuf,
}

impl ReadTool {
    pub fn new() -> Self {
        Self {
            directory: std::env::current_dir().unwrap_or_default(),
        }
    }

    pub fn with_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
}

impl Default for ReadTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn id(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Reads a file from the local filesystem. Can read text files, list directory contents, or handle binary files (images, PDFs) as base64."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file or directory to read"
                },
                "offset": {
                    "type": "number",
                    "description": "The line number to start reading from (1-indexed)"
                },
                "limit": {
                    "type": "number",
                    "description": "The maximum number of lines to read (defaults to 2000)"
                }
            },
            "required": ["file_path"]
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

        let offset: usize = args["offset"].as_u64().unwrap_or(1) as usize;

        let limit: usize = args["limit"].as_u64().unwrap_or(DEFAULT_READ_LIMIT as u64) as usize;

        if offset < 1 {
            return Err(ToolError::InvalidArguments("offset must be >= 1".into()));
        }

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
                    .with_metadata("filepath", serde_json::json!(path_str))
                    .with_metadata("parentDir", serde_json::json!(parent)),
            )
            .await?;
        }

        ctx.ask_permission(
            crate::PermissionRequest::new("read")
                .with_pattern(&path_str)
                .always_allow(),
        )
        .await?;

        let title = path
            .strip_prefix(&ctx.worktree)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let metadata = fs::metadata(&path).await.map_err(|_e| {
            let dir = path.parent().unwrap_or(Path::new("."));
            let base = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            if let Ok(entries) = std::fs::read_dir(dir) {
                let suggestions: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        let name = e.file_name().to_string_lossy().to_lowercase();
                        let target = base.to_lowercase();
                        name.contains(&target) || target.contains(&name)
                    })
                    .take(3)
                    .map(|e| e.path().to_string_lossy().to_string())
                    .collect();
                ToolError::with_suggestions(
                    format!("File not found: {}", path.display()),
                    &suggestions,
                )
            } else {
                ToolError::FileNotFound(format!("File not found: {}", path.display()))
            }
        })?;

        if metadata.is_dir() {
            ctx.do_file_time_read(path_str.clone()).await?;
            ctx.do_lsp_touch_file(path_str.clone(), false).await?;
            return read_directory(&path, offset, limit, title);
        }

        let content = fs::read(&path)
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to read file: {}", e)))?;

        let mime = detect_mime(&path);

        if is_image_mime(&mime) || mime == "application/pdf" {
            ctx.do_file_time_read(path_str.clone()).await?;
            ctx.do_lsp_touch_file(path_str.clone(), false).await?;
            return handle_binary_file(&path, &content, &mime, title);
        }

        if is_binary(&content) {
            return Err(ToolError::BinaryFile(path.display().to_string()));
        }

        ctx.do_file_time_read(path_str.clone()).await?;
        ctx.do_lsp_touch_file(path_str.clone(), false).await?;
        read_file_content(
            &path,
            &path_str,
            &content,
            offset,
            limit,
            title,
            &ctx.project_root,
        )
        .await
    }
}

fn detect_mime(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tiff" | "tif" => "image/tiff",
        "avif" => "image/avif",
        "heic" | "heif" => "image/heic",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "ts" => "application/typescript",
        "md" => "text/markdown",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn is_image_mime(mime: &str) -> bool {
    mime.starts_with("image/") && mime != "image/svg+xml" && mime != "image/vnd.fastbidsheet"
}

fn handle_binary_file(
    path: &Path,
    content: &[u8],
    mime: &str,
    title: String,
) -> Result<ToolResult, ToolError> {
    let base64_content =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, content);
    let data_url = format!("data:{};base64,{}", mime, base64_content);

    let file_type = if mime.starts_with("image/") {
        "Image"
    } else {
        "PDF"
    };
    let msg = format!("{} read successfully ({} bytes)", file_type, content.len());

    let output = format!(
        "<path>{}</path>\n<type>binary</type>\n<mime>{}</mime>\n<size>{}</size>\n<data-url>\n{}\n</data-url>",
        path.display(),
        mime,
        content.len(),
        data_url
    );

    Ok(ToolResult {
        title,
        output,
        metadata: {
            let mut m = Metadata::new();
            m.insert("preview".into(), serde_json::json!(msg));
            m.insert("truncated".into(), serde_json::json!(false));
            m.insert("mime".into(), serde_json::json!(mime));
            m.insert("size".into(), serde_json::json!(content.len()));
            m
        },
        truncated: false,
    })
}

fn read_directory(
    path: &Path,
    offset: usize,
    limit: usize,
    title: String,
) -> Result<ToolResult, ToolError> {
    let mut entries: Vec<String> = Vec::new();

    for entry in WalkDir::new(path)
        .max_depth(1)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path() == path {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().is_dir() {
            entries.push(format!("{}/", name));
        } else {
            entries.push(name);
        }
    }

    entries.sort();

    let start = offset.saturating_sub(1);
    let sliced: Vec<&str> = entries
        .iter()
        .skip(start)
        .take(limit)
        .map(|s| s.as_str())
        .collect();
    let truncated = start + sliced.len() < entries.len();

    let output = format!(
        "<path>{}</path>\n<type>directory</type>\n<entries>\n{}\n{}{}\n</entries>",
        path.display(),
        sliced.join("\n"),
        if truncated {
            format!(
                "\n(Showing {} of {} entries. Use 'offset' parameter to read beyond entry {})",
                sliced.len(),
                entries.len(),
                offset + sliced.len()
            )
        } else {
            format!("\n({} entries)", entries.len())
        },
        ""
    );

    let preview = sliced
        .iter()
        .take(20)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    Ok(ToolResult {
        title,
        output,
        metadata: {
            let mut m = Metadata::new();
            m.insert("preview".into(), serde_json::json!(preview));
            m.insert("truncated".into(), serde_json::json!(truncated));
            m
        },
        truncated,
    })
}

async fn read_file_content(
    path: &Path,
    path_str: &str,
    content: &[u8],
    offset: usize,
    limit: usize,
    title: String,
    project_root: &str,
) -> Result<ToolResult, ToolError> {
    let text = String::from_utf8_lossy(content);
    let lines: Vec<&str> = text.lines().collect();

    if offset > lines.len() {
        return Err(ToolError::InvalidArguments(format!(
            "Offset {} is out of range (file has {} lines)",
            offset,
            lines.len()
        )));
    }

    let start = offset.saturating_sub(1);
    let mut result_lines: Vec<String> = Vec::new();
    let mut bytes = 0;
    let mut truncated_by_bytes = false;

    for i in start..std::cmp::min(lines.len(), start + limit) {
        let line = if lines[i].len() > MAX_LINE_LENGTH {
            format!("{}...", &lines[i][..MAX_LINE_LENGTH])
        } else {
            lines[i].to_string()
        };

        let size = line.len() + if result_lines.is_empty() { 0 } else { 1 };
        if bytes + size > MAX_BYTES {
            truncated_by_bytes = true;
            break;
        }

        result_lines.push(format!("{}: {}", i + 1, line));
        bytes += size;
    }

    let preview = result_lines
        .iter()
        .take(20)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let total_lines = lines.len();
    let last_read_line = offset + result_lines.len() - 1;
    let has_more_lines = total_lines > last_read_line;
    let truncated = has_more_lines || truncated_by_bytes;

    let truncation_msg = if truncated_by_bytes {
        format!(
            "\n\n(Output truncated at {} bytes. Use 'offset' parameter to read beyond line {})",
            MAX_BYTES, last_read_line
        )
    } else if has_more_lines {
        format!(
            "\n\n(File has more lines. Use 'offset' parameter to read beyond line {})",
            last_read_line
        )
    } else {
        format!("\n\n(End of file - total {} lines)", total_lines)
    };

    let mut output = format!(
        "<path>{}</path>\n<type>file</type>\n<content>\n{}{}\n</content>",
        path.display(),
        result_lines.join("\n"),
        truncation_msg
    );

    let project_root_path = PathBuf::from(project_root);
    let instructions = resolve_instruction_prompts(path, &project_root_path).await;

    let mut loaded_files = vec![path_str.to_string()];

    if !instructions.is_empty() {
        let instruction_content: Vec<String> = instructions
            .iter()
            .map(|i| {
                loaded_files.push(i.filepath.clone());
                i.content.clone()
            })
            .collect();

        output.push_str("\n\n<system-reminder>\n");
        output.push_str(&instruction_content.join("\n\n"));
        output.push_str("\n</system-reminder>");
    }

    Ok(ToolResult {
        title,
        output,
        metadata: {
            let mut m = Metadata::new();
            m.insert("preview".into(), serde_json::json!(preview));
            m.insert("truncated".into(), serde_json::json!(truncated));
            m.insert("filepath".into(), serde_json::json!(path_str));
            m.insert("loaded".into(), serde_json::json!(loaded_files));
            m
        },
        truncated,
    })
}

fn is_binary(content: &[u8]) -> bool {
    if content.is_empty() {
        return false;
    }

    let check_len = std::cmp::min(4096, content.len());
    let bytes = &content[..check_len];

    if bytes.contains(&0) {
        return true;
    }

    let non_printable = bytes
        .iter()
        .filter(|&&b| b < 9 || (b > 13 && b < 32))
        .count();

    non_printable as f32 / check_len as f32 > 0.3
}

struct InstructionPrompt {
    filepath: String,
    content: String,
}

async fn resolve_instruction_prompts(
    file_path: &Path,
    project_root: &Path,
) -> Vec<InstructionPrompt> {
    let mut results = Vec::new();

    let target = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());
    let root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());

    let mut current = target.parent().unwrap_or(&target).to_path_buf();

    while current.starts_with(&root) && current != root {
        if let Some(found) = find_instruction_file(&current).await {
            let canonical = found.canonicalize().unwrap_or_else(|_| found.clone());
            if canonical != target {
                if let Ok(content) = tokio::fs::read_to_string(&found).await {
                    if !content.is_empty() {
                        results.push(InstructionPrompt {
                            filepath: found.to_string_lossy().to_string(),
                            content: format!("Instructions from: {}\n{}", found.display(), content),
                        });
                    }
                }
            }
        }

        if !current.pop() {
            break;
        }
    }

    if let Some(found) = find_instruction_file(&root).await {
        let canonical = found.canonicalize().unwrap_or_else(|_| found.clone());
        if canonical != target {
            if let Ok(content) = tokio::fs::read_to_string(&found).await {
                if !content.is_empty() {
                    results.push(InstructionPrompt {
                        filepath: found.to_string_lossy().to_string(),
                        content: format!("Instructions from: {}\n{}", found.display(), content),
                    });
                }
            }
        }
    }

    results
}

async fn find_instruction_file(dir: &Path) -> Option<PathBuf> {
    for name in INSTRUCTION_FILES {
        let path = dir.join(name);
        if path.exists() && path.is_file() {
            return Some(path);
        }
    }
    None
}
