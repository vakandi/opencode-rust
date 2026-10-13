use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use opencode_config::{
    load_config, AgentConfig as LoadedAgentConfig, AgentConfigs as LoadedAgentConfigs,
    AgentMode as LoadedAgentMode, Config as LoadedConfig,
    PermissionAction as LoadedPermissionAction, PermissionConfig as LoadedPermissionConfig,
    PermissionRule as LoadedPermissionRule,
};
use opencode_permission::{
    build_agent_ruleset, evaluate as evaluate_permission, PermissionAction, PermissionRule,
    PermissionRuleset,
};

const PROMPT_GENERATE: &str = r#"You are an AI agent configuration generator. Given a description of what an agent should do, generate a JSON configuration with:
- identifier: A unique, lowercase, single-word identifier for the agent (use underscores if needed)
- whenToUse: A brief description of when this agent should be used
- systemPrompt: The system prompt that will be given to this agent

The identifier should be descriptive but concise. The system prompt should be detailed enough to guide the agent's behavior."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedAgentConfig {
    pub identifier: String,
    pub when_to_use: String,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuiltinAgent {
    Build,
    Plan,
    General,
    Explore,
    Compaction,
    Title,
}

impl BuiltinAgent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
            Self::General => "general",
            Self::Explore => "explore",
            Self::Compaction => "compaction",
            Self::Title => "title",
        }
    }

    pub const fn all() -> [BuiltinAgent; 6] {
        [
            BuiltinAgent::Build,
            BuiltinAgent::Plan,
            BuiltinAgent::General,
            BuiltinAgent::Explore,
            BuiltinAgent::Compaction,
            BuiltinAgent::Title,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub description: Option<String>,
    pub mode: AgentMode,
    pub model: Option<ModelRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_preference: Option<ModelRef>,
    pub system_prompt: Option<String>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u64>,
    pub max_steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    pub options: HashMap<String, serde_json::Value>,
    #[serde(default, alias = "permission_ruleset")]
    pub permission: PermissionRuleset,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub native: bool,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum AgentMode {
    #[default]
    Primary,
    Subagent,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny,
}

pub struct PermissionNext;

impl PermissionNext {
    pub fn evaluate(agent: &AgentInfo, tool_name: &str) -> PermissionDecision {
        Self::evaluate_path(agent, tool_name, "*")
    }

    /// Evaluate a tool call against the path it touches.
    ///
    /// The ruleset already understands folder patterns (`/srv/agent/**`), but every
    /// caller used to pass `"*"` here, so a path could never be tested and `"bash":
    /// "allow"` meant allow everywhere. `path` is the file or directory the call
    /// actually names; `*` when the call has no path.
    pub fn evaluate_path(
        agent: &AgentInfo,
        tool_name: &str,
        path: &str,
    ) -> PermissionDecision {
        if !agent.allowed_tools.is_empty()
            && !agent.allowed_tools.iter().any(|tool| tool == tool_name)
        {
            return PermissionDecision::Deny;
        }

        let permission = tool_to_permission(tool_name);
        let rule = evaluate_permission(permission, path, &[agent.permission.clone()]);
        match rule.action {
            PermissionAction::Allow => PermissionDecision::Allow,
            PermissionAction::Ask => PermissionDecision::Ask,
            PermissionAction::Deny => PermissionDecision::Deny,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRef {
    pub model_id: String,
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResult {
    pub content: String,
    pub tool_calls: Vec<ToolCallResult>,
    pub usage: Option<UsageInfo>,
    pub finished: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInfo {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateInput {
    pub description: String,
    pub model: Option<ModelRef>,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Provider error: {0}")]
    ProviderError(#[from] opencode_provider::ProviderError),

    #[error("Failed to parse generated config: {0}")]
    ParseError(String),

    #[error("No default model available")]
    NoDefaultModel,
}

impl AgentInfo {
    pub fn from_builtin(builtin: BuiltinAgent) -> Self {
        match builtin {
            BuiltinAgent::Build => Self::build(),
            BuiltinAgent::Plan => Self::plan(),
            BuiltinAgent::General => Self::general(),
            BuiltinAgent::Explore => Self::explore(),
            BuiltinAgent::Compaction => Self::compaction(),
            BuiltinAgent::Title => Self::title(),
        }
    }

    pub fn default_agent() -> Self {
        Self::general()
    }

    pub fn build() -> Self {
        Self {
            name: "build".to_string(),
            description: Some(
                "The default agent. Executes tools based on configured permissions.".to_string(),
            ),
            mode: AgentMode::Primary,
            model: None,
            model_preference: None,
            system_prompt: None,
            temperature: None,
            top_p: None,
            max_tokens: Some(8192),
            max_steps: Some(100),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: build_agent_ruleset("build", &[]),
            hidden: false,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn plan() -> Self {
        Self {
            name: "plan".to_string(),
            description: Some("Plan mode. Disallows all edit tools.".to_string()),
            mode: AgentMode::Primary,
            model: None,
            model_preference: None,
            system_prompt: Some("You are a planning assistant. Analyze the task and create a detailed plan before execution.".to_string()),
            temperature: Some(0.3),
            top_p: None,
            max_tokens: Some(8192),
            max_steps: Some(50),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: build_agent_ruleset("plan", &[]),
            hidden: false,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn general() -> Self {
        Self {
            name: "general".to_string(),
            description: Some("Default general-purpose agent.".to_string()),
            mode: AgentMode::Primary,
            model: None,
            model_preference: None,
            system_prompt: Some(
                "You are a helpful assistant. Complete the task given to you.".to_string(),
            ),
            temperature: Some(0.7),
            top_p: None,
            max_tokens: Some(8192),
            max_steps: Some(20),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: build_agent_ruleset("general", &[]),
            hidden: false,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn explore() -> Self {
        Self {
            name: "explore".to_string(),
            description: Some("Exploration subagent for searching and reading code.".to_string()),
            mode: AgentMode::Subagent,
            model: None,
            model_preference: None,
            system_prompt: Some("You are an exploration assistant. Search and read code to answer questions. Focus on read-only operations.".to_string()),
            temperature: Some(0.5),
            top_p: None,
            max_tokens: Some(8192),
            max_steps: Some(30),
            allowed_tools: vec![
                "grep".to_string(),
                "glob".to_string(),
                "read".to_string(),
                "bash".to_string(),
            ],
            options: HashMap::new(),
            permission: build_agent_ruleset("explore", &[]),
            hidden: false,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn title() -> Self {
        Self {
            name: "title".to_string(),
            description: Some("Generates concise session titles.".to_string()),
            mode: AgentMode::Subagent,
            model: None,
            model_preference: None,
            system_prompt: Some("You are a title generator. Generate a concise 3-5 word title that summarizes the conversation. Return only the title, nothing else.".to_string()),
            temperature: Some(0.3),
            top_p: None,
            max_tokens: Some(1024),
            max_steps: Some(1),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: vec![PermissionRule {
                permission: "*".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Deny,
            }],
            hidden: true,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn summary() -> Self {
        Self {
            name: "summary".to_string(),
            description: Some("Generates conversation summaries.".to_string()),
            mode: AgentMode::Subagent,
            model: None,
            model_preference: None,
            system_prompt: Some("You are a summary generator. Create a concise summary of the conversation. Focus on key decisions and outcomes.".to_string()),
            temperature: Some(0.3),
            top_p: None,
            max_tokens: Some(1024),
            max_steps: Some(1),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: vec![PermissionRule {
                permission: "*".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Deny,
            }],
            hidden: true,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn compaction() -> Self {
        Self {
            name: "compaction".to_string(),
            description: Some("Compacts conversation history while preserving context.".to_string()),
            mode: AgentMode::Subagent,
            model: None,
            model_preference: None,
            system_prompt: Some("You are a context compaction assistant. Summarize the conversation while preserving all important context for future interactions.".to_string()),
            temperature: Some(0.3),
            top_p: None,
            max_tokens: Some(1024),
            max_steps: Some(1),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: vec![PermissionRule {
                permission: "*".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Deny,
            }],
            hidden: true,
            native: true,
            variant: None,
            color: None,
        }
    }

    pub fn custom(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            name: name.clone(),
            description: None,
            mode: AgentMode::All,
            model: None,
            model_preference: None,
            system_prompt: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_steps: Some(100),
            allowed_tools: Vec::new(),
            options: HashMap::new(),
            permission: build_agent_ruleset(&name, &[]),
            hidden: false,
            native: false,
            variant: None,
            color: None,
        }
    }

    pub fn with_model(
        mut self,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
    ) -> Self {
        let model = ModelRef {
            model_id: model_id.into(),
            provider_id: provider_id.into(),
        };
        self.model_preference = Some(model.clone());
        self.model = Some(model);
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = Some(temp);
        self
    }

    pub fn with_max_steps(mut self, steps: u32) -> Self {
        self.max_steps = Some(steps);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_permission(mut self, permission: PermissionRuleset) -> Self {
        self.permission = permission;
        self
    }

    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn tool_permission_decision(&self, tool_name: &str) -> PermissionDecision {
        PermissionNext::evaluate(self, tool_name)
    }

    pub fn tool_permission_decision_at(
        &self,
        tool_name: &str,
        path: &str,
    ) -> PermissionDecision {
        PermissionNext::evaluate_path(self, tool_name, path)
    }

    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        matches!(
            self.tool_permission_decision(tool_name),
            PermissionDecision::Allow
        )
    }
}

pub struct AgentRegistry {
    agents: HashMap<String, AgentInfo>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        let mut agents = HashMap::new();
        for builtin in BuiltinAgent::all() {
            let agent = AgentInfo::from_builtin(builtin);
            agents.insert(builtin.as_str().to_string(), agent);
        }
        // Legacy hidden agent kept for backward compatibility with older task flows.
        agents.insert("summary".to_string(), AgentInfo::summary());
        Self { agents }
    }

    pub fn from_config(config: &LoadedConfig) -> Self {
        let mut registry = Self::new();
        registry.apply_config(config);
        registry
    }

    pub fn from_optional_config(config: Option<&LoadedConfig>) -> Self {
        if let Some(config) = config {
            return Self::from_config(config);
        }
        Self::new()
    }

    pub fn from_project_dir(project_dir: impl AsRef<Path>) -> Self {
        match load_config(project_dir) {
            Ok(config) => Self::from_config(&config),
            Err(_) => Self::new(),
        }
    }

    fn apply_config(&mut self, config: &LoadedConfig) {
        if let Some(mode_configs) = &config.mode {
            self.apply_agent_configs(mode_configs, Some(AgentMode::Primary));
        }
        if let Some(agent_configs) = &config.agent {
            self.apply_agent_configs(agent_configs, None);
        }
    }

    fn apply_agent_configs(
        &mut self,
        configs: &LoadedAgentConfigs,
        forced_mode: Option<AgentMode>,
    ) {
        for (key, cfg) in &configs.entries {
            self.apply_agent_config(key, cfg, forced_mode);
        }
    }

    fn apply_agent_config(
        &mut self,
        key: &str,
        cfg: &LoadedAgentConfig,
        forced_mode: Option<AgentMode>,
    ) {
        if cfg.disable.unwrap_or(false) {
            self.agents.remove(key);
            return;
        }

        let mut agent = self
            .agents
            .get(key)
            .cloned()
            .unwrap_or_else(|| AgentInfo::custom(key.to_string()));

        if let Some(name) = &cfg.name {
            agent.name = name.clone();
        }
        if let Some(description) = &cfg.description {
            agent.description = Some(description.clone());
        }
        if let Some(prompt) = &cfg.prompt {
            agent.system_prompt = Some(prompt.clone());
        }
        if let Some(variant) = &cfg.variant {
            agent.variant = Some(variant.clone());
        }
        if let Some(temperature) = cfg.temperature {
            agent.temperature = Some(temperature);
        }
        if let Some(top_p) = cfg.top_p {
            agent.top_p = Some(top_p);
        }
        if let Some(color) = &cfg.color {
            agent.color = Some(color.clone());
        }
        if let Some(hidden) = cfg.hidden {
            agent.hidden = hidden;
        }
        if let Some(mode) = forced_mode {
            agent.mode = mode;
        } else if let Some(mode) = cfg.mode.clone() {
            agent.mode = map_loaded_agent_mode(mode);
        }
        if let Some(steps) = cfg.steps.or(cfg.max_steps) {
            agent.max_steps = Some(steps);
        }
        if let Some(max_tokens) = cfg.max_tokens {
            agent.max_tokens = Some(max_tokens);
        }
        if let Some(model) = cfg.model.as_deref().and_then(parse_model_ref) {
            agent.model_preference = Some(model.clone());
            agent.model = Some(model);
        }
        if let Some(options) = &cfg.options {
            for (key, value) in options {
                if let Some(existing) = agent.options.get_mut(key) {
                    merge_json_value(existing, value.clone());
                } else {
                    agent.options.insert(key.clone(), value.clone());
                }
            }
        }
        if let Some(tool_overrides) = &cfg.tools {
            if !agent.allowed_tools.is_empty() {
                let mut merged: HashSet<String> = agent.allowed_tools.into_iter().collect();
                for (tool, enabled) in tool_overrides {
                    if *enabled {
                        merged.insert(tool.clone());
                    } else {
                        merged.remove(tool);
                    }
                }
                let mut out: Vec<String> = merged.into_iter().collect();
                out.sort();
                agent.allowed_tools = out;
            }
        }

        if cfg.permission.is_some() || cfg.tools.is_some() {
            let mut user_rules: PermissionRuleset = Vec::new();
            if let Some(permission_cfg) = &cfg.permission {
                user_rules.extend(permission_rules_from_config(permission_cfg));
            }
            if let Some(tool_overrides) = &cfg.tools {
                user_rules.extend(permission_rules_from_tools(tool_overrides));
            }
            agent.permission = build_agent_ruleset(key, &user_rules);
        }

        self.agents.insert(key.to_string(), agent);
    }

    pub fn get(&self, name: &str) -> Option<&AgentInfo> {
        self.agents.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut AgentInfo> {
        self.agents.get_mut(name)
    }

    pub fn register(&mut self, agent: AgentInfo) {
        self.agents.insert(agent.name.clone(), agent);
    }

    pub fn list(&self) -> Vec<&AgentInfo> {
        let mut agents: Vec<&AgentInfo> = self.agents.values().filter(|a| !a.hidden).collect();
        agents.sort_by(|a, b| {
            let a_is_build = a.name == "build";
            let b_is_build = b.name == "build";
            if a_is_build {
                return std::cmp::Ordering::Less;
            }
            if b_is_build {
                return std::cmp::Ordering::Greater;
            }
            a.name.cmp(&b.name)
        });
        agents
    }

    pub fn list_all(&self) -> Vec<&AgentInfo> {
        self.agents.values().collect()
    }

    pub fn list_primary(&self) -> Vec<&AgentInfo> {
        let mut agents: Vec<&AgentInfo> = self
            .agents
            .values()
            .filter(|a| matches!(a.mode, AgentMode::Primary) && !a.hidden)
            .collect();
        agents.sort_by(|a, b| {
            let a_is_build = a.name == "build";
            let b_is_build = b.name == "build";
            if a_is_build {
                return std::cmp::Ordering::Less;
            }
            if b_is_build {
                return std::cmp::Ordering::Greater;
            }
            a.name.cmp(&b.name)
        });
        agents
    }

    pub fn list_subagents(&self) -> Vec<&AgentInfo> {
        let mut agents: Vec<&AgentInfo> = self
            .agents
            .values()
            .filter(|a| matches!(a.mode, AgentMode::Subagent) && !a.hidden)
            .collect();
        agents.sort_by(|a, b| a.name.cmp(&b.name));
        agents
    }

    pub fn default_agent(&self) -> &AgentInfo {
        if let Some(general) = self.get(BuiltinAgent::General.as_str()) {
            return general;
        }

        if let Some(primary) = self
            .agents
            .values()
            .find(|a| !a.hidden && !matches!(a.mode, AgentMode::Subagent))
        {
            return primary;
        }

        self.agents
            .values()
            .next()
            .expect("Agent registry is empty; expected at least one agent")
    }

    pub async fn generate(
        &self,
        input: GenerateInput,
        provider_registry: &opencode_provider::ProviderRegistry,
    ) -> Result<GeneratedAgentConfig, AgentError> {
        let model_ref = input.model.clone().ok_or(AgentError::NoDefaultModel)?;

        let provider = provider_registry
            .get(&model_ref.provider_id)
            .ok_or_else(|| AgentError::NoDefaultModel)?;

        let existing_names: Vec<&str> = self.agents.keys().map(|s| s.as_str()).collect();
        let existing_list = existing_names.join(", ");

        let user_content = format!(
            "Create an agent configuration based on this request: \"{}\".\n\n\
             IMPORTANT: The following identifiers already exist and must NOT be used: {}\n\
             Return ONLY the JSON object, no other text, do not wrap in backticks",
            input.description, existing_list
        );

        let messages = vec![
            opencode_provider::Message::system(PROMPT_GENERATE),
            opencode_provider::Message::user(&user_content),
        ];

        let request = opencode_provider::ChatRequest::new(&model_ref.model_id, messages)
            .with_temperature(0.3)
            .with_stream(false);

        let response = provider.chat(request).await?;

        let content = response
            .choices
            .first()
            .and_then(|c| match &c.message.content {
                opencode_provider::Content::Text(text) => Some(text.clone()),
                opencode_provider::Content::Parts(parts) => {
                    parts.first().and_then(|p| p.text.clone())
                }
            })
            .unwrap_or_default();

        let cleaned = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        serde_json::from_str(cleaned)
            .map_err(|e| AgentError::ParseError(format!("{}: {}", e, cleaned)))
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn tool_to_permission(tool: &str) -> &str {
    match tool {
        "write" | "edit" | "multiedit" | "apply_patch" | "patch" => "edit",
        "ls" => "list",
        _ => tool,
    }
}

fn map_loaded_permission_action(action: &LoadedPermissionAction) -> PermissionAction {
    match action {
        LoadedPermissionAction::Ask => PermissionAction::Ask,
        LoadedPermissionAction::Allow => PermissionAction::Allow,
        LoadedPermissionAction::Deny => PermissionAction::Deny,
    }
}

fn permission_rules_from_config(permission: &LoadedPermissionConfig) -> PermissionRuleset {
    let mut rules = Vec::new();
    for (permission_name, rule) in &permission.rules {
        match rule {
            LoadedPermissionRule::Action(action) => rules.push(PermissionRule {
                permission: permission_name.clone(),
                pattern: "*".to_string(),
                action: map_loaded_permission_action(action),
            }),
            LoadedPermissionRule::Object(patterns) => {
                for (pattern, action) in patterns {
                    rules.push(PermissionRule {
                        permission: permission_name.clone(),
                        pattern: pattern.clone(),
                        action: map_loaded_permission_action(action),
                    });
                }
            }
        }
    }
    rules
}

fn permission_rules_from_tools(tool_overrides: &HashMap<String, bool>) -> PermissionRuleset {
    tool_overrides
        .iter()
        .map(|(tool, enabled)| PermissionRule {
            permission: tool_to_permission(tool).to_string(),
            pattern: "*".to_string(),
            action: if *enabled {
                PermissionAction::Allow
            } else {
                PermissionAction::Deny
            },
        })
        .collect()
}

fn map_loaded_agent_mode(mode: LoadedAgentMode) -> AgentMode {
    match mode {
        LoadedAgentMode::Primary => AgentMode::Primary,
        LoadedAgentMode::Subagent => AgentMode::Subagent,
        LoadedAgentMode::All => AgentMode::All,
    }
}

fn parse_model_ref(raw: &str) -> Option<ModelRef> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (provider, model) = raw.split_once(':').or_else(|| raw.split_once('/'))?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(ModelRef {
        provider_id: provider.to_string(),
        model_id: model.to_string(),
    })
}

fn merge_json_value(target: &mut serde_json::Value, source: serde_json::Value) {
    match (target, source) {
        (serde_json::Value::Object(target_map), serde_json::Value::Object(source_map)) => {
            for (key, source_value) in source_map {
                if let Some(target_value) = target_map.get_mut(&key) {
                    merge_json_value(target_value, source_value);
                } else {
                    target_map.insert(key, source_value);
                }
            }
        }
        (target, source) => *target = source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_agents_have_expected_defaults() {
        let registry = AgentRegistry::new();
        for builtin in BuiltinAgent::all() {
            let agent = registry
                .get(builtin.as_str())
                .unwrap_or_else(|| panic!("missing builtin agent '{}'", builtin.as_str()));
            assert!(agent.native, "builtin agent should be native");
            assert_eq!(agent.name, builtin.as_str());
        }

        assert!(matches!(
            registry.get("build").map(|a| a.mode),
            Some(AgentMode::Primary)
        ));
        assert!(matches!(
            registry.get("plan").map(|a| a.mode),
            Some(AgentMode::Primary)
        ));
        assert!(matches!(
            registry.get("general").map(|a| a.mode),
            Some(AgentMode::Primary)
        ));
        assert!(matches!(
            registry.get("explore").map(|a| a.mode),
            Some(AgentMode::Subagent)
        ));
        assert_eq!(registry.default_agent().name, "general");
    }

    #[test]
    fn explore_agent_permission_is_restricted_to_read_search_and_bash() {
        let explore = AgentInfo::explore();
        assert_eq!(
            PermissionNext::evaluate(&explore, "grep"),
            PermissionDecision::Allow
        );
        assert_eq!(
            PermissionNext::evaluate(&explore, "glob"),
            PermissionDecision::Allow
        );
        assert_eq!(
            PermissionNext::evaluate(&explore, "read"),
            PermissionDecision::Allow
        );
        assert_eq!(
            PermissionNext::evaluate(&explore, "bash"),
            PermissionDecision::Allow
        );

        assert_eq!(
            PermissionNext::evaluate(&explore, "write"),
            PermissionDecision::Deny
        );
        assert_eq!(
            PermissionNext::evaluate(&explore, "ls"),
            PermissionDecision::Deny
        );
        assert_eq!(
            PermissionNext::evaluate(&explore, "websearch"),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn config_can_override_builtin_agent_model() {
        let mut config = LoadedConfig::default();
        config.agent = Some(LoadedAgentConfigs {
            entries: HashMap::from([(
                "general".to_string(),
                LoadedAgentConfig {
                    model: Some("openai/gpt-4.1".to_string()),
                    ..Default::default()
                },
            )]),
        });

        let registry = AgentRegistry::from_config(&config);
        let general = registry.get("general").expect("general should exist");
        assert_eq!(
            general.model.as_ref().map(|m| m.provider_id.as_str()),
            Some("openai")
        );
        assert_eq!(
            general.model.as_ref().map(|m| m.model_id.as_str()),
            Some("gpt-4.1")
        );
        assert_eq!(
            general
                .model_preference
                .as_ref()
                .map(|m| m.provider_id.as_str()),
            Some("openai")
        );
        assert_eq!(
            general
                .model_preference
                .as_ref()
                .map(|m| m.model_id.as_str()),
            Some("gpt-4.1")
        );
    }

    #[test]
    fn registry_supports_dynamic_custom_agents_from_config() {
        let mut config = LoadedConfig::default();
        config.agent = Some(LoadedAgentConfigs {
            entries: HashMap::from([(
                "reviewer".to_string(),
                LoadedAgentConfig {
                    description: Some("Custom reviewer agent".to_string()),
                    mode: Some(LoadedAgentMode::Subagent),
                    model: Some("openai/gpt-4.1".to_string()),
                    prompt: Some("Review code carefully".to_string()),
                    steps: Some(12),
                    ..Default::default()
                },
            )]),
        });

        let registry = AgentRegistry::from_config(&config);
        let reviewer = registry.get("reviewer").expect("reviewer should exist");
        assert_eq!(
            reviewer.description.as_deref(),
            Some("Custom reviewer agent")
        );
        assert!(matches!(reviewer.mode, AgentMode::Subagent));
        assert_eq!(
            reviewer.model.as_ref().map(|m| m.provider_id.as_str()),
            Some("openai")
        );
        assert_eq!(
            reviewer.model.as_ref().map(|m| m.model_id.as_str()),
            Some("gpt-4.1")
        );
        assert_eq!(
            reviewer.system_prompt.as_deref(),
            Some("Review code carefully")
        );
        assert_eq!(reviewer.max_steps, Some(12));
        assert!(!reviewer.native);
    }

    #[test]
    fn registry_can_disable_builtin_agent_from_config() {
        let mut config = LoadedConfig::default();
        config.agent = Some(LoadedAgentConfigs {
            entries: HashMap::from([(
                "build".to_string(),
                LoadedAgentConfig {
                    disable: Some(true),
                    ..Default::default()
                },
            )]),
        });

        let registry = AgentRegistry::from_config(&config);
        assert!(registry.get("build").is_none());
    }

    #[test]
    fn deprecated_mode_config_forces_primary_mode() {
        let mut config = LoadedConfig::default();
        config.mode = Some(LoadedAgentConfigs {
            entries: HashMap::from([(
                "investigate".to_string(),
                LoadedAgentConfig {
                    mode: Some(LoadedAgentMode::Subagent),
                    ..Default::default()
                },
            )]),
        });

        let registry = AgentRegistry::from_config(&config);
        let agent = registry
            .get("investigate")
            .expect("investigate should be created from deprecated mode config");
        assert!(matches!(agent.mode, AgentMode::Primary));
    }

    // A folder rule must decide on the path, not on the tool alone: before `path` was
    // threaded through, every call was judged against "*", so "read": "allow" granted
    // the whole filesystem and no rule could ever confine an agent to its workspace.
    #[test]
    fn folder_rule_scopes_the_tool_to_that_folder() {
        let mut agent = AgentInfo::general();
        agent.permission = vec![
            // Last match wins, and the scan starts from the end, so the catch-all has to
            // come first or it shadows every specific rule.
            PermissionRule {
                permission: "read".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Deny,
            },
            // Single trailing star: the matcher strips it and tests `starts_with`, which
            // already covers nested folders. `/**` does NOT work — it leaves the asterisk
            // in the prefix and matches nothing.
            PermissionRule {
                permission: "read".to_string(),
                pattern: "/srv/agent/workspace/*".to_string(),
                action: PermissionAction::Allow,
            },
        ];

        let inside = PermissionNext::evaluate_path(
            &agent,
            "read",
            "/srv/agent/workspace/notes/todo.md",
        );
        let sibling = PermissionNext::evaluate_path(&agent, "read", "/srv/agent/secrets.env");
        let outside = PermissionNext::evaluate_path(&agent, "read", "/Users/wael/.ssh/id_rsa");

        assert_eq!(inside, PermissionDecision::Allow, "inside its workspace");
        assert_eq!(sibling, PermissionDecision::Deny, "a sibling folder is not covered");
        assert_eq!(outside, PermissionDecision::Deny, "outside the workspace");
    }

    // A write is a separate permission from a read, so the folder rule must not leak
    // across: allowing reads under the workspace must not authorise writing to /etc.
    #[test]
    fn folder_rule_does_not_authorise_another_tool() {
        let mut agent = AgentInfo::general();
        agent.permission = vec![
            PermissionRule {
                permission: "read".to_string(),
                pattern: "/srv/agent/**".to_string(),
                action: PermissionAction::Allow,
            },
            PermissionRule {
                permission: "*".to_string(),
                pattern: "*".to_string(),
                action: PermissionAction::Deny,
            },
        ];

        let write = PermissionNext::evaluate_path(&agent, "write", "/srv/agent/x.txt");
        assert_eq!(write, PermissionDecision::Deny);
    }
}
