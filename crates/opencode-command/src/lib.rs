//! Slash Commands System
//!
//! Provides a command system for loading and executing slash commands from:
//! - `.opencode/commands/*.md` files
//! - MCP prompts
//! - Built-in commands

use opencode_plugin::{HookContext, HookEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Command metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub template: String,
    pub source: CommandSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CommandSource {
    File(PathBuf),
    Builtin,
    Mcp { server: String, prompt: String },
    Skill { name: String },
}

/// Command execution context
#[derive(Debug, Clone)]
pub struct CommandContext {
    pub arguments: Vec<String>,
    pub variables: HashMap<String, String>,
    pub working_directory: PathBuf,
}

impl CommandContext {
    pub fn new(working_directory: PathBuf) -> Self {
        Self {
            arguments: Vec::new(),
            variables: HashMap::new(),
            working_directory,
        }
    }

    pub fn with_arguments(mut self, args: Vec<String>) -> Self {
        self.arguments = args;
        self
    }

    pub fn with_variable(mut self, key: String, value: String) -> Self {
        self.variables.insert(key, value);
        self
    }
}

/// Command registry for loading and executing commands
pub struct CommandRegistry {
    commands: HashMap<String, Command>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            commands: HashMap::new(),
        };
        registry.register_builtin_commands();
        registry
    }

    /// Register built-in commands
    fn register_builtin_commands(&mut self) {
        self.register(Command {
            name: "init".to_string(),
            description: "Initialize OpenCode in the current project".to_string(),
            template: include_str!("../commands/init.md").to_string(),
            source: CommandSource::Builtin,
        });

        self.register(Command {
            name: "review".to_string(),
            description: "Review the current changes in the project".to_string(),
            template: include_str!("../commands/review.md").to_string(),
            source: CommandSource::Builtin,
        });

        self.register(Command {
            name: "commit".to_string(),
            description: "Create a git commit with the current changes".to_string(),
            template: include_str!("../commands/commit.md").to_string(),
            source: CommandSource::Builtin,
        });

        self.register(Command {
            name: "test".to_string(),
            description: "Run tests for the project".to_string(),
            template: include_str!("../commands/test.md").to_string(),
            source: CommandSource::Builtin,
        });
    }

    /// Register a new command
    pub fn register(&mut self, command: Command) {
        self.commands.insert(command.name.clone(), command);
    }

    /// Get a command by name
    pub fn get(&self, name: &str) -> Option<&Command> {
        self.commands.get(name)
    }

    /// List all available commands
    pub fn list(&self) -> Vec<&Command> {
        self.commands.values().collect()
    }

    /// Load commands from .opencode/commands directory
    pub fn load_from_directory(&mut self, project_dir: &Path) -> anyhow::Result<()> {
        let commands_dir = project_dir.join(".opencode/commands");

        if !commands_dir.exists() {
            return Ok(());
        }

        let pattern = commands_dir.join("*.md");
        let pattern_str = pattern.to_string_lossy();

        for entry in glob::glob(&pattern_str)? {
            let path = entry?;
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            let template = std::fs::read_to_string(&path)?;
            let description = extract_description(&template)
                .unwrap_or_else(|| format!("Custom command: {}", name));

            self.register(Command {
                name: name.clone(),
                description,
                template,
                source: CommandSource::File(path),
            });
        }

        Ok(())
    }

    pub fn parse(&self, input: &str) -> Option<(&Command, Vec<String>)> {
        let input = input.trim_start();

        if !input.starts_with('/') {
            return None;
        }

        let input = &input[1..];
        let parts: Vec<&str> = input.split_whitespace().collect();

        if parts.is_empty() {
            return None;
        }

        let name = parts[0];
        let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();

        self.commands.get(name).map(|cmd| (cmd, args))
    }

    /// Execute a command and return the rendered template
    pub fn execute(&self, name: &str, ctx: CommandContext) -> anyhow::Result<String> {
        let command = self
            .commands
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Command not found: {}", name))?;

        Ok(self.render_template(&command.template, ctx))
    }

    /// Execute a command with plugin hooks (async version)
    pub async fn execute_with_hooks(
        &self,
        name: &str,
        ctx: CommandContext,
    ) -> anyhow::Result<String> {
        let command = self
            .commands
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Command not found: {}", name))?;

        let mut rendered = self.render_template(&command.template, ctx.clone());

        // Plugin hook: command.execute.before
        let hook_outputs = opencode_plugin::trigger_collect(
            HookContext::new(HookEvent::CommandExecuteBefore)
                .with_data("command", serde_json::json!(name))
                .with_data("source", serde_json::json!(format!("{:?}", command.source)))
                .with_data("arguments", serde_json::json!(ctx.arguments.join(" ")))
                .with_data(
                    "parts",
                    serde_json::json!([{
                        "type": "text",
                        "text": rendered
                    }]),
                ),
        )
        .await;
        for output in hook_outputs {
            let Some(payload) = output.payload.as_ref() else {
                continue;
            };
            apply_command_hook_payload(&mut rendered, payload);
        }

        Ok(rendered)
    }

    fn render_template(&self, template: &str, ctx: CommandContext) -> String {
        let mut result = template.to_string();

        for (i, arg) in ctx.arguments.iter().enumerate() {
            let placeholder = format!("${}", i + 1);
            result = result.replace(&placeholder, arg);
        }

        let all_args = ctx.arguments.join(" ");
        result = result.replace("$ARGUMENTS", &all_args);

        for (key, value) in &ctx.variables {
            let placeholder = format!("${{{}}}", key);
            result = result.replace(&placeholder, value);
        }

        for (key, value) in std::env::vars() {
            let placeholder = format!("$ENV_{}", key);
            result = result.replace(&placeholder, &value);
        }

        result
    }
}

fn command_payload_object(
    payload: &serde_json::Value,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    payload
        .get("output")
        .and_then(|value| value.as_object())
        .or_else(|| payload.as_object())
        .or_else(|| payload.get("data").and_then(|value| value.as_object()))
}

fn apply_command_hook_payload(rendered: &mut String, payload: &serde_json::Value) {
    let Some(object) = command_payload_object(payload) else {
        return;
    };

    if let Some(text) = object
        .get("output")
        .and_then(|value| value.as_str())
        .or_else(|| object.get("template").and_then(|value| value.as_str()))
    {
        *rendered = text.to_string();
        return;
    }

    let Some(parts) = object.get("parts").and_then(|value| value.as_array()) else {
        return;
    };

    let text = parts
        .iter()
        .filter_map(|part| part.as_object())
        .filter(|part| {
            part.get("type")
                .and_then(|value| value.as_str())
                .map(|kind| kind == "text")
                .unwrap_or(false)
        })
        .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
        .collect::<Vec<_>>()
        .join("\n");

    if !text.is_empty() {
        *rendered = text;
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract description from markdown (first line after # if present)
fn extract_description(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            return Some(trimmed.trim_start_matches('#').trim().to_string());
        }
        if !trimmed.is_empty() && !trimmed.starts_with("<!--") {
            return Some(format!(
                "Command: {}",
                trimmed.chars().take(50).collect::<String>()
            ));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command() {
        let registry = CommandRegistry::new();

        let result = registry.parse("/init my-project");
        assert!(result.is_some());

        let (cmd, args) = result.unwrap();
        assert_eq!(cmd.name, "init");
        assert_eq!(args, vec!["my-project"]);
    }

    #[test]
    fn test_render_template() {
        let registry = CommandRegistry::new();
        let ctx = CommandContext::new(PathBuf::from("/tmp"))
            .with_arguments(vec!["arg1".to_string(), "arg2".to_string()])
            .with_variable("PROJECT".to_string(), "test-project".to_string());

        let result = registry.render_template("Hello $1 and $2. Project: ${PROJECT}", ctx);
        assert_eq!(result, "Hello arg1 and arg2. Project: test-project");
    }
}
