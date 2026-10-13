use chrono::Local;

// Embed prompt templates at compile time (matching TS originals from session/prompt/*.txt)
const PROMPT_ANTHROPIC: &str = include_str!("prompt_templates/anthropic.txt");
const PROMPT_BEAST: &str = include_str!("prompt_templates/beast.txt");
const PROMPT_GEMINI: &str = include_str!("prompt_templates/gemini.txt");
const PROMPT_QWEN: &str = include_str!("prompt_templates/qwen.txt");
const PROMPT_CODEX: &str = include_str!("prompt_templates/codex_header.txt");
const PROMPT_TRINITY: &str = include_str!("prompt_templates/trinity.txt");
const MAX_MCP_RESOURCE_CHARS: usize = 12_000;

pub struct SystemPrompt;

impl SystemPrompt {
    /// Returns the codex/instructions prompt (used as base instructions).
    /// TS: `SystemPrompt.instructions()` → `PROMPT_CODEX.trim()`
    pub fn instructions() -> &'static str {
        PROMPT_CODEX.trim()
    }

    /// Wrap arbitrary text in a `<system-reminder>` block so it is treated as
    /// injected runtime context, not user-authored content.
    pub fn system_reminder(content: &str) -> String {
        format!("<system-reminder>\n{}\n</system-reminder>", content.trim())
    }

    /// Build a system-reminder block for MCP resource text content.
    pub fn mcp_resource_reminder(filename: &str, uri: &str, content: &str) -> String {
        let (content, truncated) = trim_for_prompt(content, MAX_MCP_RESOURCE_CHARS);
        let truncation_hint = if truncated {
            "\n\n[Content truncated for prompt safety.]"
        } else {
            ""
        };
        let body = format!(
            "MCP resource context from {} ({}):\n{}{}",
            filename, uri, content, truncation_hint
        );
        Self::system_reminder(&body)
    }

    /// Select the appropriate system prompt based on model API ID.
    /// TS: `SystemPrompt.provider(model)` in session/system.ts
    ///
    /// Matching rules (in priority order):
    ///   - gpt-5       → PROMPT_CODEX
    ///   - gpt-* / o1 / o3 → PROMPT_BEAST
    ///   - gemini-*    → PROMPT_GEMINI
    ///   - claude*     → PROMPT_ANTHROPIC
    ///   - trinity     → PROMPT_TRINITY
    ///   - fallback    → PROMPT_QWEN (anthropic without todo)
    pub fn for_model(model_api_id: &str) -> &'static str {
        let id = model_api_id.to_lowercase();

        if id.contains("gpt-5") {
            return PROMPT_CODEX;
        }
        if id.contains("gpt-") || id.contains("o1") || id.contains("o3") {
            return PROMPT_BEAST;
        }
        if id.contains("gemini-") {
            return PROMPT_GEMINI;
        }
        if id.contains("claude") {
            return PROMPT_ANTHROPIC;
        }
        if id.contains("trinity") {
            return PROMPT_TRINITY;
        }
        // Default fallback — same as TS (qwen.txt = anthropic without todo)
        PROMPT_QWEN
    }

    /// Build the environment context block.
    /// TS: `SystemPrompt.environment(model)` in session/system.ts
    ///
    /// Produces a string like:
    /// ```text
    /// You are powered by the model named claude-sonnet-4-20250514. The exact model ID is anthropic/claude-sonnet-4-20250514
    /// Here is some useful information about the environment you are running in:
    /// <env>
    ///   Working directory: /home/user/project
    ///   Is directory a git repo: yes
    ///   Platform: linux
    ///   Today's date: Wed Feb 19 2026
    /// </env>
    /// ```
    pub fn environment(env: &EnvironmentContext) -> String {
        let mut lines = Vec::with_capacity(10);

        lines.push(format!(
            "You are powered by the model named {}. The exact model ID is {}/{}",
            env.model_api_id, env.provider_id, env.model_api_id
        ));
        lines.push(
            "Here is some useful information about the environment you are running in:".to_string(),
        );
        lines.push("<env>".to_string());
        lines.push(format!("  Working directory: {}", env.working_directory));
        lines.push(format!(
            "  Is directory a git repo: {}",
            if env.is_git_repo { "yes" } else { "no" }
        ));
        lines.push(format!("  Platform: {}", env.platform));
        lines.push(format!(
            "  Today's date: {}",
            Local::now().format("%a %b %d %Y")
        ));
        lines.push("</env>".to_string());

        lines.join("\n")
    }
}

/// Context needed to build the environment block in the system prompt.
#[derive(Debug, Clone)]
pub struct EnvironmentContext {
    pub model_api_id: String,
    pub provider_id: String,
    pub working_directory: String,
    pub is_git_repo: bool,
    pub platform: String,
}

impl EnvironmentContext {
    /// Build from the current runtime environment.
    pub fn from_current(
        model_api_id: impl Into<String>,
        provider_id: impl Into<String>,
        working_directory: impl Into<String>,
    ) -> Self {
        let wd: String = working_directory.into();
        let is_git = std::path::Path::new(&wd).join(".git").exists();
        Self {
            model_api_id: model_api_id.into(),
            provider_id: provider_id.into(),
            working_directory: wd,
            is_git_repo: is_git,
            platform: std::env::consts::OS.to_string(),
        }
    }
}

fn trim_for_prompt(input: &str, max_chars: usize) -> (&str, bool) {
    let trimmed = input.trim();
    if trimmed.chars().count() <= max_chars {
        return (trimmed, false);
    }

    let end = trimmed
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(trimmed.len());
    (&trimmed[..end], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_for_model_claude() {
        let prompt = SystemPrompt::for_model("claude-sonnet-4-20250514");
        assert!(prompt.contains("OpenCode"));
    }

    #[test]
    fn test_for_model_gpt4() {
        let prompt = SystemPrompt::for_model("gpt-4o");
        // beast.txt starts with "You are opencode, an agent"
        assert!(prompt.contains("opencode"));
    }

    #[test]
    fn test_for_model_gpt5() {
        let prompt = SystemPrompt::for_model("gpt-5-turbo");
        // codex_header.txt
        assert!(prompt.contains("OpenCode"));
    }

    #[test]
    fn test_for_model_gemini() {
        let prompt = SystemPrompt::for_model("gemini-2.0-flash");
        assert!(prompt.contains("opencode"));
    }

    #[test]
    fn test_for_model_trinity() {
        let prompt = SystemPrompt::for_model("Trinity-Large");
        assert!(prompt.contains("opencode"));
    }

    #[test]
    fn test_for_model_fallback() {
        let prompt = SystemPrompt::for_model("some-unknown-model");
        // qwen.txt fallback
        assert!(prompt.contains("opencode"));
    }

    #[test]
    fn test_environment_output() {
        let ctx = EnvironmentContext {
            model_api_id: "claude-sonnet-4-20250514".to_string(),
            provider_id: "anthropic".to_string(),
            working_directory: "/tmp/test".to_string(),
            is_git_repo: true,
            platform: "linux".to_string(),
        };
        let env = SystemPrompt::environment(&ctx);
        assert!(env.contains("claude-sonnet-4-20250514"));
        assert!(env.contains("anthropic/claude-sonnet-4-20250514"));
        assert!(env.contains("/tmp/test"));
        assert!(env.contains("Is directory a git repo: yes"));
        assert!(env.contains("Platform: linux"));
        assert!(env.contains("<env>"));
        assert!(env.contains("</env>"));
    }

    #[test]
    fn test_environment_no_git() {
        let ctx = EnvironmentContext {
            model_api_id: "gpt-4o".to_string(),
            provider_id: "openai".to_string(),
            working_directory: "/tmp/no-git".to_string(),
            is_git_repo: false,
            platform: "macos".to_string(),
        };
        let env = SystemPrompt::environment(&ctx);
        assert!(env.contains("Is directory a git repo: no"));
    }

    #[test]
    fn test_instructions() {
        let inst = SystemPrompt::instructions();
        assert!(!inst.is_empty());
        // codex_header.txt starts with "You are OpenCode"
        assert!(inst.starts_with("You are OpenCode"));
    }

    #[test]
    fn test_system_reminder_wraps_content() {
        let wrapped = SystemPrompt::system_reminder("hello");
        assert!(wrapped.starts_with("<system-reminder>"));
        assert!(wrapped.contains("hello"));
        assert!(wrapped.ends_with("</system-reminder>"));
    }

    #[test]
    fn test_mcp_resource_reminder_includes_filename_uri_and_content() {
        let wrapped = SystemPrompt::mcp_resource_reminder("rules.md", "repo/rules", "line1\nline2");
        assert!(wrapped.contains("MCP resource context from rules.md (repo/rules):"));
        assert!(wrapped.contains("line1"));
        assert!(wrapped.contains("<system-reminder>"));
    }

    #[test]
    fn test_mcp_resource_reminder_truncates_very_large_content() {
        let content = "a".repeat(20_000);
        let wrapped = SystemPrompt::mcp_resource_reminder("big.txt", "repo/big", &content);
        assert!(wrapped.contains("MCP resource context from big.txt (repo/big):"));
        assert!(wrapped.contains("Content truncated for prompt safety."));
        // sanity check: output should be significantly smaller than full payload.
        assert!(wrapped.len() < 15_000);
    }
}
