use async_trait::async_trait;
use std::collections::HashSet;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{timeout, Duration};

use crate::{Metadata, Tool, ToolContext, ToolError, ToolResult};
use opencode_permission::BashArity;
use opencode_plugin::{HookContext, HookEvent};

const DEFAULT_TIMEOUT_MS: u64 = 2 * 60 * 1000;
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

#[cfg(unix)]
async fn kill_process_tree(pid: u32) {
    let _ = tokio::process::Command::new("pkill")
        .arg("-TERM")
        .arg("-P")
        .arg(pid.to_string())
        .status()
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let _ = tokio::process::Command::new("pkill")
        .arg("-KILL")
        .arg("-P")
        .arg(pid.to_string())
        .status()
        .await;
}

pub struct BashTool;

impl BashTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BashTool {
    fn id(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Executes a bash command in the specified working directory."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Optional timeout in milliseconds"
                },
                "workdir": {
                    "type": "string",
                    "description": "The working directory to run the command in"
                },
                "description": {
                    "type": "string",
                    "description": "Clear, concise description of what this command does"
                }
            },
            "required": ["command", "description"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let command: String = args["command"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("command is required".into()))?
            .to_string();

        let timeout_ms: u64 = args["timeout"].as_u64().unwrap_or(DEFAULT_TIMEOUT_MS);

        let workdir: String = args["workdir"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| ctx.directory.clone());

        let description: String = args["description"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("description is required".into()))?
            .to_string();

        let title = description.clone();

        let mut env_vars = std::collections::HashMap::new();
        for (key, value) in std::env::vars() {
            env_vars.insert(key, value);
        }
        if let Some(extra_env) = ctx.extra.get("env") {
            if let Some(env_obj) = extra_env.as_object() {
                for (key, value) in env_obj {
                    if let Some(val_str) = value.as_str() {
                        env_vars.insert(key.clone(), val_str.to_string());
                    }
                }
            }
        }

        // Plugin hook: shell.env — let plugins inject environment variables
        let mut hook_ctx = HookContext::new(HookEvent::ShellEnv)
            .with_session(&ctx.session_id)
            .with_data("cwd", serde_json::json!(&workdir));
        if let Some(call_id) = &ctx.call_id {
            hook_ctx = hook_ctx.with_data("call_id", serde_json::json!(call_id));
        }
        let env_hook_outputs = opencode_plugin::trigger_collect(hook_ctx).await;
        for output in env_hook_outputs {
            let Some(payload) = output.payload.as_ref() else {
                continue;
            };
            let Some(object) = payload
                .get("output")
                .and_then(|value| value.as_object())
                .or_else(|| payload.as_object())
            else {
                continue;
            };
            let Some(env) = object.get("env").and_then(|value| value.as_object()) else {
                continue;
            };
            for (key, value) in env {
                if let Some(value_str) = value.as_str() {
                    env_vars.insert(key.clone(), value_str.to_string());
                }
            }
        }

        // Parse command with tree-sitter for proper AST-based permission extraction
        let parsed = parse_bash_command(&command);

        // Check external directories
        for path in &parsed.directories {
            if ctx.is_external_path(path) {
                let parent = std::path::Path::new(path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());

                ctx.ask_permission(
                    crate::PermissionRequest::new("external_directory")
                        .with_pattern(format!("{}/*", parent))
                        .with_metadata("filepath", serde_json::json!(path))
                        .with_metadata("parentDir", serde_json::json!(parent)),
                )
                .await?;
            }
        }

        if !parsed.patterns.is_empty() {
            let patterns: Vec<String> = parsed.patterns.into_iter().collect();
            let always: Vec<String> = parsed.always.into_iter().collect();
            let mut req = crate::PermissionRequest::new("bash")
                .with_patterns(patterns)
                .with_metadata("description", serde_json::json!(description));
            for a in always {
                req = req.with_always(a);
            }
            ctx.ask_permission(req).await?;
        } else {
            // No pattern could be extracted from the command, so the gate above was
            // skipped entirely and the shell ran unchecked. Judge it on its name alone.
            ctx.ask_permission(crate::PermissionRequest::new("bash")).await?;
        }

        let shell = if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "bash"
        };
        let flag = if cfg!(target_os = "windows") {
            "/C"
        } else {
            "-c"
        };

        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg(flag).arg(&command);
        cmd.current_dir(&workdir);
        for (key, value) in &env_vars {
            cmd.env(key, value);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::ExecutionError(format!("Failed to spawn process: {}", e)))?;

        let child_pid = child.id();

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut output = String::new();
        let mut truncated = false;

        let abort_token = ctx.abort.clone();

        let result = timeout(Duration::from_millis(timeout_ms), async {
            loop {
                tokio::select! {
                    _ = abort_token.cancelled() => {
                        #[cfg(unix)]
                        {
                            if let Some(pid) = child_pid {
                                kill_process_tree(pid).await;
                            }
                        }
                        let _ = child.kill().await;
                        return Err(ToolError::Cancelled);
                    }
                    line = stdout_reader.next_line() => {
                        match line {
                            Ok(Some(l)) => {
                                if output.len() + l.len() + 1 > MAX_OUTPUT_BYTES {
                                    truncated = true;
                                } else {
                                    output.push_str(&l);
                                    output.push('\n');
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                output.push_str(&format!("Error reading stdout: {}\n", e));
                            }
                        }
                    }
                    line = stderr_reader.next_line() => {
                        match line {
                            Ok(Some(l)) => {
                                if output.len() + l.len() + 1 > MAX_OUTPUT_BYTES {
                                    truncated = true;
                                } else {
                                    output.push_str(&l);
                                    output.push('\n');
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                output.push_str(&format!("Error reading stderr: {}\n", e));
                            }
                        }
                    }
                }
            }
            Ok::<_, ToolError>(())
        })
        .await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                #[cfg(unix)]
                {
                    if let Some(pid) = child_pid {
                        kill_process_tree(pid).await;
                    }
                }
                let _ = child.kill().await;
                return Err(ToolError::Timeout(format!(
                    "Command timed out after {}ms",
                    timeout_ms
                )));
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to wait for process: {}", e)))?;

        let exit_code = status.code().unwrap_or(-1);

        if !status.success() {
            output.push_str(&format!("\nCommand exited with code: {}", exit_code));
        }

        if truncated {
            output.push_str(&format!(
                "\n\n(Output truncated at {} bytes)",
                MAX_OUTPUT_BYTES
            ));
        }

        Ok(ToolResult {
            title,
            output,
            metadata: {
                let mut m = Metadata::new();
                m.insert("exit_code".into(), serde_json::json!(exit_code));
                m.insert("truncated".into(), serde_json::json!(truncated));
                m
            },
            truncated,
        })
    }
}

// ---------------------------------------------------------------------------
// Tree-sitter based bash command parsing
// ---------------------------------------------------------------------------

/// Result of parsing a bash command with tree-sitter.
struct ParsedCommand {
    /// Full command text for each individual command (for permission patterns).
    patterns: HashSet<String>,
    /// BashArity-derived prefix patterns with wildcard (for "always allow").
    always: HashSet<String>,
    /// External directory paths found in path-manipulating commands.
    directories: Vec<String>,
}

const PATH_COMMANDS: &[&str] = &[
    "cd", "rm", "cp", "mv", "mkdir", "touch", "chmod", "chown", "cat",
];

fn parse_bash_command(command: &str) -> ParsedCommand {
    let mut result = ParsedCommand {
        patterns: HashSet::new(),
        always: HashSet::new(),
        directories: Vec::new(),
    };

    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_bash::LANGUAGE;
    if parser.set_language(&language.into()).is_err() {
        // Fallback: treat entire command as a single pattern
        let tokens: Vec<String> = command.split_whitespace().map(String::from).collect();
        result.patterns.insert(command.to_string());
        let prefix = BashArity::prefix(&tokens);
        result.always.insert(format!("{} *", prefix.join(" ")));
        return result;
    }

    let Some(tree) = parser.parse(command, None) else {
        let tokens: Vec<String> = command.split_whitespace().map(String::from).collect();
        result.patterns.insert(command.to_string());
        let prefix = BashArity::prefix(&tokens);
        result.always.insert(format!("{} *", prefix.join(" ")));
        return result;
    };

    let root = tree.root_node();
    collect_commands(root, command.as_bytes(), &mut result);

    // If tree-sitter found no commands (e.g. variable assignment only), use full command
    if result.patterns.is_empty() && !command.trim().is_empty() {
        let tokens: Vec<String> = command.split_whitespace().map(String::from).collect();
        result.patterns.insert(command.to_string());
        let prefix = BashArity::prefix(&tokens);
        result.always.insert(format!("{} *", prefix.join(" ")));
    }

    result
}

fn collect_commands(node: tree_sitter::Node, source: &[u8], result: &mut ParsedCommand) {
    if node.kind() == "command" {
        process_command_node(node, source, result);
        return;
    }

    // Recurse into children to find command nodes inside pipelines, lists, etc.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_commands(child, source, result);
    }
}

fn process_command_node(node: tree_sitter::Node, source: &[u8], result: &mut ParsedCommand) {
    // Get full command text, including redirects if parent is redirected_statement
    let command_text = if node.parent().map(|p| p.kind()) == Some("redirected_statement") {
        node.parent()
            .unwrap()
            .utf8_text(source)
            .unwrap_or_default()
            .to_string()
    } else {
        node.utf8_text(source).unwrap_or_default().to_string()
    };

    // Extract tokens: command_name + word/string/raw_string/concatenation children
    let mut tokens: Vec<String> = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "command_name" | "word" | "string" | "raw_string" | "concatenation" => {
                let text = child.utf8_text(source).unwrap_or_default().to_string();
                tokens.push(text);
            }
            _ => {}
        }
    }

    if tokens.is_empty() {
        return;
    }

    // Check for path-manipulating commands and extract external paths
    if PATH_COMMANDS.contains(&tokens[0].as_str()) {
        for arg in &tokens[1..] {
            if arg.starts_with('-') || (tokens[0] == "chmod" && arg.starts_with('+')) {
                continue;
            }
            // Resolve path
            let path = if std::path::Path::new(arg).is_absolute() {
                arg.clone()
            } else if arg.starts_with('~') {
                if let Ok(home) = std::env::var("HOME") {
                    arg.replacen('~', &home, 1)
                } else {
                    arg.clone()
                }
            } else {
                // Relative path — can't resolve without cwd context here,
                // but the caller checks is_external_path which handles this
                arg.clone()
            };
            result.directories.push(path);
        }
    }

    // Skip "cd" from patterns (covered by directory check above)
    if tokens[0] != "cd" {
        result.patterns.insert(command_text);
        let prefix = BashArity::prefix(&tokens);
        result.always.insert(format!("{} *", prefix.join(" ")));
    }
}
