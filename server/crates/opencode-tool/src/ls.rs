use async_trait::async_trait;
use glob::Pattern;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::{Metadata, PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

const IGNORE_PATTERNS: &[&str] = &[
    "node_modules/",
    "__pycache__/",
    ".git/",
    "dist/",
    "build/",
    "target/",
    "vendor/",
    "bin/",
    "obj/",
    ".idea/",
    ".vscode/",
    ".zig-cache/",
    "zig-out",
    ".coverage",
    "coverage/",
    "vendor/",
    "tmp/",
    "temp/",
    ".cache/",
    "cache/",
    "logs/",
    ".venv/",
    "venv/",
    "env/",
];

const LIMIT: usize = 100;

pub struct LsTool {}

impl LsTool {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for LsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, serde::Deserialize)]
struct LsInput {
    path: Option<String>,
    ignore: Option<Vec<String>>,
}

fn has_glob_meta(pattern: &str) -> bool {
    pattern
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
}

#[async_trait]
impl Tool for LsTool {
    fn id(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        "Lists files and directories in a given path."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The absolute path to the directory to list (must be absolute, not relative)"
                },
                "ignore": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of glob patterns to ignore"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: LsInput = serde_json::from_value(args).unwrap_or(LsInput {
            path: None,
            ignore: None,
        });

        let requested_path = input.path.unwrap_or_else(|| ".".to_string());
        let mut base_dir = if Path::new(&requested_path).is_absolute() {
            PathBuf::from(&requested_path)
        } else {
            PathBuf::from(&ctx.directory).join(&requested_path)
        };
        if let Ok(canonical) = base_dir.canonicalize() {
            base_dir = canonical;
        }
        let base_dir_str = base_dir.to_string_lossy().to_string();

        ctx.ask_permission(
            PermissionRequest::new("list")
                .with_pattern(&base_dir_str)
                .with_metadata("path", serde_json::json!(&base_dir_str))
                .always_allow(),
        )
        .await?;

        if !base_dir.exists() {
            return Err(ToolError::FileNotFound(base_dir.display().to_string()));
        }

        if !base_dir.is_dir() {
            return Err(ToolError::ExecutionError(format!(
                "{} is not a directory",
                base_dir.display()
            )));
        }

        let mut ignore_set: HashSet<String> = IGNORE_PATTERNS
            .iter()
            .map(|s| s.trim_end_matches('/').to_string())
            .collect();
        let mut ignore_globs: Vec<Pattern> = Vec::new();

        if let Some(custom_ignore) = input.ignore {
            for pattern in custom_ignore {
                let normalized = pattern.trim_start_matches('!').trim();
                if normalized.is_empty() {
                    continue;
                }

                if has_glob_meta(normalized) {
                    if let Ok(glob) = Pattern::new(normalized) {
                        ignore_globs.push(glob);
                    }
                } else {
                    ignore_set.insert(normalized.trim_end_matches('/').to_string());
                }
            }
        }

        let mut files: Vec<String> = Vec::new();
        for entry in WalkDir::new(&base_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let rel_path = entry.path().strip_prefix(&base_dir).unwrap_or(entry.path());
            let rel_str = rel_path.to_string_lossy().replace('\\', "/");

            if rel_str.is_empty() {
                continue;
            }

            let should_skip = rel_str.split('/').any(|part| ignore_set.contains(part))
                || ignore_globs.iter().any(|glob| glob.matches(&rel_str));

            if should_skip {
                continue;
            }

            if entry.file_type().is_file() {
                files.push(rel_str);
                if files.len() >= LIMIT {
                    break;
                }
            }
        }

        let mut dirs: HashSet<String> = HashSet::new();
        let mut files_by_dir: HashMap<String, Vec<String>> = HashMap::new();

        for file in &files {
            let dir = Path::new(file)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string());

            let parts: Vec<&str> = if dir == "." {
                vec![]
            } else {
                dir.split('/').collect()
            };

            for i in 0..=parts.len() {
                let dir_path = if i == 0 {
                    ".".to_string()
                } else {
                    parts[..i].join("/")
                };
                dirs.insert(dir_path);
            }

            let file_name = Path::new(file)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| file.clone());

            files_by_dir.entry(dir).or_default().push(file_name);
        }

        fn render_dir(
            dir_path: &str,
            depth: usize,
            dirs: &HashSet<String>,
            files_by_dir: &HashMap<String, Vec<String>>,
        ) -> String {
            let indent = "  ".repeat(depth);
            let mut output = String::new();

            if depth > 0 {
                let name = Path::new(dir_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| dir_path.to_string());
                output.push_str(&format!("{}{}/\n", indent, name));
            }

            let child_indent = "  ".repeat(depth + 1);

            let parent_prefix = if dir_path == "." {
                String::new()
            } else {
                format!("{}/", dir_path)
            };

            let mut children: Vec<String> = dirs
                .iter()
                .filter(|d| {
                    let d_str = d.as_str();
                    if d_str == dir_path {
                        return false;
                    }
                    if dir_path == "." {
                        !d_str.contains('/')
                    } else {
                        d_str.starts_with(&parent_prefix)
                            && d_str[parent_prefix.len()..].matches('/').count() == 0
                    }
                })
                .cloned()
                .collect();
            children.sort();

            for child in &children {
                output.push_str(&render_dir(child, depth + 1, dirs, files_by_dir));
            }

            let mut files = files_by_dir.get(dir_path).cloned().unwrap_or_default();
            files.sort();
            for file in files {
                output.push_str(&format!("{}{}\n", child_indent, file));
            }

            output
        }

        let output = format!(
            "{}/\n{}",
            base_dir.display(),
            render_dir(".", 0, &dirs, &files_by_dir)
        );

        let title = match base_dir.strip_prefix(Path::new(&ctx.worktree)) {
            Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
            Ok(rel) => rel.to_string_lossy().to_string(),
            Err(_) => base_dir.display().to_string(),
        };

        Ok(ToolResult {
            title,
            output,
            metadata: {
                let mut m = Metadata::new();
                m.insert("count".into(), serde_json::json!(files.len()));
                m.insert("truncated".into(), serde_json::json!(files.len() >= LIMIT));
                m
            },
            truncated: files.len() >= LIMIT,
        })
    }
}
