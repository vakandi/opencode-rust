use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub keybinds: Option<KeybindsConfig>,

    #[serde(
        rename = "logLevel",
        alias = "log_level",
        skip_serializing_if = "Option::is_none"
    )]
    pub log_level: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tui: Option<TuiConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<HashMap<String, CommandConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<SkillsConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub watcher: Option<WatcherConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugin: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub share: Option<ShareMode>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoshare: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoupdate: Option<AutoUpdateMode>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_providers: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_providers: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<AgentConfigs>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentConfigs>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<HashMap<String, ProviderConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<HashMap<String, McpServerConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatter: Option<FormatterConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<LspConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutMode>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<HashMap<String, bool>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise: Option<EnterpriseConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction: Option<CompactionConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<ExperimentalConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareMode {
    Manual,
    Auto,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AutoUpdateMode {
    Boolean(bool),
    Notify(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeybindsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_exit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_open: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollbar_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_view: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_export: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_new: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_timeline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_fork: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_rename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_delete: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash_delete: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_favorite_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_share: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_unshare: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_interrupt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_compact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_page_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_page_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_line_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_line_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_half_page_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_half_page_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_first: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_last: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_previous: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_last_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_copy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_undo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_redo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_toggle_conceal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_cycle_recent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_cycle_recent_reverse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_cycle_favorite: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_cycle_favorite_reverse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_cycle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_cycle_reverse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_cycle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_clear: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_paste: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_submit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_newline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_move_left: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_move_right: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_move_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_move_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_left: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_right: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_line_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_line_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_visual_line_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_visual_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_visual_line_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_visual_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_buffer_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_buffer_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_buffer_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_buffer_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_line: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_to_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_to_line_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_backspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_undo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_redo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_word_forward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_word_backward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_word_forward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_word_backward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_word_forward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_word_backward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_previous: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_child_cycle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_child_cycle_reverse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_suspend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_title_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tips_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_thinking: Option<String>,

    // Legacy fields kept for backward compatibility
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TuiConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_acceleration: Option<ScrollAccelerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScrollAccelerationConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mdns: Option<bool>,
    #[serde(
        rename = "mdnsDomain",
        alias = "mdns_domain",
        skip_serializing_if = "Option::is_none"
    )]
    pub mdns_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatcherConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfigs {
    #[serde(flatten)]
    pub entries: HashMap<String, AgentConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<AgentMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "maxSteps")]
    pub max_steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    // `permissions` is the key every agent file in config-home/agents actually uses. Without
// this alias it deserialised into nothing and every rule in those files was discarded —
// an agent asking for bash then had it, with no denial ever recorded.
#[serde(alias = "permissions", skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    Primary,
    Subagent,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<HashMap<String, ModelConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub whitelist: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blacklist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<HashMap<String, ModelVariantConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelVariantConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServerConfig {
    Enabled { enabled: bool },
    Full(McpServer),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServer {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub server_type: Option<String>,

    /// For local: command array; for remote: unused
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,

    /// For local: environment variables
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<HashMap<String, String>>,

    /// For remote: URL of the MCP server
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// For remote: headers to send
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// For remote: OAuth config (or false to disable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpOAuthConfig>,

    // Legacy fields kept for backward compatibility
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
}

/// OAuth configuration for remote MCP servers.
/// Can be a full config object or `false` to disable OAuth auto-detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpOAuthConfig {
    Disabled(bool),
    Config(McpOAuth),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpOAuth {
    #[serde(
        rename = "clientId",
        alias = "client_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_id: Option<String>,
    #[serde(
        rename = "clientSecret",
        alias = "client_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FormatterConfig {
    Disabled(bool),
    Enabled(HashMap<String, FormatterEntry>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FormatterEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LspConfig {
    Disabled(bool),
    Enabled(HashMap<String, LspServerConfig>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LspServerConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initialization: Option<HashMap<String, serde_json::Value>>,
}

/// Layout mode: "auto" or "stretch" (TS: z.enum(["auto", "stretch"]))
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutMode {
    Auto,
    Stretch,
}

/// Permission config: a record of tool name -> permission rule.
/// Each rule can be a simple action string ("ask"/"allow"/"deny") or
/// a record of sub-keys to actions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionConfig {
    #[serde(flatten)]
    pub rules: HashMap<String, PermissionRule>,
}

/// A permission rule: either a simple action or a map of sub-keys to actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PermissionRule {
    Action(PermissionAction),
    Object(HashMap<String, PermissionAction>),
}

/// Permission action: "ask", "allow", or "deny".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionAction {
    Ask,
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnterpriseConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_config_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prune: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExperimentalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_paste_summary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_tool: Option<bool>,
    #[serde(alias = "openTelemetry", skip_serializing_if = "Option::is_none")]
    pub open_telemetry: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continue_loop_on_deny: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_timeout: Option<u64>,
}

trait DeepMerge {
    fn deep_merge(&mut self, other: Self);
}

fn merge_option_replace<T>(target: &mut Option<T>, source: Option<T>) {
    if let Some(value) = source {
        *target = Some(value);
    }
}

fn merge_option_deep<T: DeepMerge>(target: &mut Option<T>, source: Option<T>) {
    if let Some(source_value) = source {
        if let Some(target_value) = target {
            target_value.deep_merge(source_value);
        } else {
            *target = Some(source_value);
        }
    }
}

fn merge_map_deep_values<T: DeepMerge>(
    target: &mut HashMap<String, T>,
    source: HashMap<String, T>,
) {
    for (key, source_value) in source {
        if let Some(target_value) = target.get_mut(&key) {
            target_value.deep_merge(source_value);
        } else {
            target.insert(key, source_value);
        }
    }
}

fn merge_option_map_deep_values<T: DeepMerge>(
    target: &mut Option<HashMap<String, T>>,
    source: Option<HashMap<String, T>>,
) {
    if let Some(source_map) = source {
        if let Some(target_map) = target {
            merge_map_deep_values(target_map, source_map);
        } else {
            *target = Some(source_map);
        }
    }
}

fn merge_option_map_overwrite_values<T>(
    target: &mut Option<HashMap<String, T>>,
    source: Option<HashMap<String, T>>,
) {
    if let Some(source_map) = source {
        if let Some(target_map) = target {
            for (key, value) in source_map {
                target_map.insert(key, value);
            }
        } else {
            *target = Some(source_map);
        }
    }
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
        (target_value, source_value) => *target_value = source_value,
    }
}

fn merge_option_json_map(
    target: &mut Option<HashMap<String, serde_json::Value>>,
    source: Option<HashMap<String, serde_json::Value>>,
) {
    if let Some(source_map) = source {
        if let Some(target_map) = target {
            for (key, source_value) in source_map {
                if let Some(target_value) = target_map.get_mut(&key) {
                    merge_json_value(target_value, source_value);
                } else {
                    target_map.insert(key, source_value);
                }
            }
        } else {
            *target = Some(source_map);
        }
    }
}

fn append_unique_keep_order(target: &mut Vec<String>, source: Vec<String>) {
    for item in source {
        if !target.contains(&item) {
            target.push(item);
        }
    }
}

impl DeepMerge for KeybindsConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.leader, other.leader);
        merge_option_replace(&mut self.app_exit, other.app_exit);
        merge_option_replace(&mut self.editor_open, other.editor_open);
        merge_option_replace(&mut self.theme_list, other.theme_list);
        merge_option_replace(&mut self.sidebar_toggle, other.sidebar_toggle);
        merge_option_replace(&mut self.scrollbar_toggle, other.scrollbar_toggle);
        merge_option_replace(&mut self.username_toggle, other.username_toggle);
        merge_option_replace(&mut self.status_view, other.status_view);
        merge_option_replace(&mut self.session_export, other.session_export);
        merge_option_replace(&mut self.session_new, other.session_new);
        merge_option_replace(&mut self.session_list, other.session_list);
        merge_option_replace(&mut self.session_timeline, other.session_timeline);
        merge_option_replace(&mut self.session_fork, other.session_fork);
        merge_option_replace(&mut self.session_rename, other.session_rename);
        merge_option_replace(&mut self.session_delete, other.session_delete);
        merge_option_replace(&mut self.stash_delete, other.stash_delete);
        merge_option_replace(&mut self.model_provider_list, other.model_provider_list);
        merge_option_replace(&mut self.model_favorite_toggle, other.model_favorite_toggle);
        merge_option_replace(&mut self.session_share, other.session_share);
        merge_option_replace(&mut self.session_unshare, other.session_unshare);
        merge_option_replace(&mut self.session_interrupt, other.session_interrupt);
        merge_option_replace(&mut self.session_compact, other.session_compact);
        merge_option_replace(&mut self.messages_page_up, other.messages_page_up);
        merge_option_replace(&mut self.messages_page_down, other.messages_page_down);
        merge_option_replace(&mut self.messages_line_up, other.messages_line_up);
        merge_option_replace(&mut self.messages_line_down, other.messages_line_down);
        merge_option_replace(&mut self.messages_half_page_up, other.messages_half_page_up);
        merge_option_replace(
            &mut self.messages_half_page_down,
            other.messages_half_page_down,
        );
        merge_option_replace(&mut self.messages_first, other.messages_first);
        merge_option_replace(&mut self.messages_last, other.messages_last);
        merge_option_replace(&mut self.messages_next, other.messages_next);
        merge_option_replace(&mut self.messages_previous, other.messages_previous);
        merge_option_replace(&mut self.messages_last_user, other.messages_last_user);
        merge_option_replace(&mut self.messages_copy, other.messages_copy);
        merge_option_replace(&mut self.messages_undo, other.messages_undo);
        merge_option_replace(&mut self.messages_redo, other.messages_redo);
        merge_option_replace(
            &mut self.messages_toggle_conceal,
            other.messages_toggle_conceal,
        );
        merge_option_replace(&mut self.tool_details, other.tool_details);
        merge_option_replace(&mut self.model_list, other.model_list);
        merge_option_replace(&mut self.model_cycle_recent, other.model_cycle_recent);
        merge_option_replace(
            &mut self.model_cycle_recent_reverse,
            other.model_cycle_recent_reverse,
        );
        merge_option_replace(&mut self.model_cycle_favorite, other.model_cycle_favorite);
        merge_option_replace(
            &mut self.model_cycle_favorite_reverse,
            other.model_cycle_favorite_reverse,
        );
        merge_option_replace(&mut self.command_list, other.command_list);
        merge_option_replace(&mut self.agent_list, other.agent_list);
        merge_option_replace(&mut self.agent_cycle, other.agent_cycle);
        merge_option_replace(&mut self.agent_cycle_reverse, other.agent_cycle_reverse);
        merge_option_replace(&mut self.variant_cycle, other.variant_cycle);
        merge_option_replace(&mut self.input_clear, other.input_clear);
        merge_option_replace(&mut self.input_paste, other.input_paste);
        merge_option_replace(&mut self.input_submit, other.input_submit);
        merge_option_replace(&mut self.input_newline, other.input_newline);
        merge_option_replace(&mut self.input_move_left, other.input_move_left);
        merge_option_replace(&mut self.input_move_right, other.input_move_right);
        merge_option_replace(&mut self.input_move_up, other.input_move_up);
        merge_option_replace(&mut self.input_move_down, other.input_move_down);
        merge_option_replace(&mut self.input_select_left, other.input_select_left);
        merge_option_replace(&mut self.input_select_right, other.input_select_right);
        merge_option_replace(&mut self.input_select_up, other.input_select_up);
        merge_option_replace(&mut self.input_select_down, other.input_select_down);
        merge_option_replace(&mut self.input_line_home, other.input_line_home);
        merge_option_replace(&mut self.input_line_end, other.input_line_end);
        merge_option_replace(
            &mut self.input_select_line_home,
            other.input_select_line_home,
        );
        merge_option_replace(&mut self.input_select_line_end, other.input_select_line_end);
        merge_option_replace(
            &mut self.input_visual_line_home,
            other.input_visual_line_home,
        );
        merge_option_replace(&mut self.input_visual_line_end, other.input_visual_line_end);
        merge_option_replace(
            &mut self.input_select_visual_line_home,
            other.input_select_visual_line_home,
        );
        merge_option_replace(
            &mut self.input_select_visual_line_end,
            other.input_select_visual_line_end,
        );
        merge_option_replace(&mut self.input_buffer_home, other.input_buffer_home);
        merge_option_replace(&mut self.input_buffer_end, other.input_buffer_end);
        merge_option_replace(
            &mut self.input_select_buffer_home,
            other.input_select_buffer_home,
        );
        merge_option_replace(
            &mut self.input_select_buffer_end,
            other.input_select_buffer_end,
        );
        merge_option_replace(&mut self.input_delete_line, other.input_delete_line);
        merge_option_replace(
            &mut self.input_delete_to_line_end,
            other.input_delete_to_line_end,
        );
        merge_option_replace(
            &mut self.input_delete_to_line_start,
            other.input_delete_to_line_start,
        );
        merge_option_replace(&mut self.input_backspace, other.input_backspace);
        merge_option_replace(&mut self.input_delete, other.input_delete);
        merge_option_replace(&mut self.input_undo, other.input_undo);
        merge_option_replace(&mut self.input_redo, other.input_redo);
        merge_option_replace(&mut self.input_word_forward, other.input_word_forward);
        merge_option_replace(&mut self.input_word_backward, other.input_word_backward);
        merge_option_replace(
            &mut self.input_select_word_forward,
            other.input_select_word_forward,
        );
        merge_option_replace(
            &mut self.input_select_word_backward,
            other.input_select_word_backward,
        );
        merge_option_replace(
            &mut self.input_delete_word_forward,
            other.input_delete_word_forward,
        );
        merge_option_replace(
            &mut self.input_delete_word_backward,
            other.input_delete_word_backward,
        );
        merge_option_replace(&mut self.history_previous, other.history_previous);
        merge_option_replace(&mut self.history_next, other.history_next);
        merge_option_replace(&mut self.session_child_cycle, other.session_child_cycle);
        merge_option_replace(
            &mut self.session_child_cycle_reverse,
            other.session_child_cycle_reverse,
        );
        merge_option_replace(&mut self.session_parent, other.session_parent);
        merge_option_replace(&mut self.terminal_suspend, other.terminal_suspend);
        merge_option_replace(&mut self.terminal_title_toggle, other.terminal_title_toggle);
        merge_option_replace(&mut self.tips_toggle, other.tips_toggle);
        merge_option_replace(&mut self.display_thinking, other.display_thinking);
        // Legacy fields
        merge_option_replace(&mut self.submit, other.submit);
        merge_option_replace(&mut self.cancel, other.cancel);
        merge_option_replace(&mut self.interrupt, other.interrupt);
    }
}

impl DeepMerge for TuiConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.mode, other.mode);
        merge_option_replace(&mut self.sidebar, other.sidebar);
        merge_option_replace(&mut self.scroll_speed, other.scroll_speed);
        merge_option_replace(&mut self.scroll_acceleration, other.scroll_acceleration);
        merge_option_replace(&mut self.diff_style, other.diff_style);
    }
}

impl DeepMerge for ServerConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.port, other.port);
        merge_option_replace(&mut self.hostname, other.hostname);
        merge_option_replace(&mut self.mdns, other.mdns);
        merge_option_replace(&mut self.mdns_domain, other.mdns_domain);
        merge_option_replace(&mut self.cors, other.cors);
    }
}

impl DeepMerge for CommandConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.name, other.name);
        merge_option_replace(&mut self.description, other.description);
        merge_option_replace(&mut self.template, other.template);
        merge_option_replace(&mut self.model, other.model);
        merge_option_replace(&mut self.agent, other.agent);
        merge_option_replace(&mut self.subtask, other.subtask);
    }
}

impl DeepMerge for SkillsConfig {
    fn deep_merge(&mut self, other: Self) {
        if !other.paths.is_empty() {
            self.paths = other.paths;
        }
        if !other.urls.is_empty() {
            self.urls = other.urls;
        }
    }
}

impl DeepMerge for WatcherConfig {
    fn deep_merge(&mut self, other: Self) {
        if !other.ignore.is_empty() {
            self.ignore = other.ignore;
        }
    }
}

impl DeepMerge for AgentConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.name, other.name);
        merge_option_replace(&mut self.model, other.model);
        merge_option_replace(&mut self.variant, other.variant);
        merge_option_replace(&mut self.temperature, other.temperature);
        merge_option_replace(&mut self.top_p, other.top_p);
        merge_option_replace(&mut self.prompt, other.prompt);
        merge_option_replace(&mut self.disable, other.disable);
        merge_option_replace(&mut self.description, other.description);
        merge_option_replace(&mut self.mode, other.mode);
        merge_option_replace(&mut self.hidden, other.hidden);
        merge_option_json_map(&mut self.options, other.options);
        merge_option_replace(&mut self.color, other.color);
        merge_option_replace(&mut self.steps, other.steps);
        merge_option_replace(&mut self.max_tokens, other.max_tokens);
        merge_option_replace(&mut self.max_steps, other.max_steps);
        merge_option_deep(&mut self.permission, other.permission);
        merge_option_map_overwrite_values(&mut self.tools, other.tools);
    }
}

impl DeepMerge for AgentConfigs {
    fn deep_merge(&mut self, other: Self) {
        merge_map_deep_values(&mut self.entries, other.entries);
    }
}

impl DeepMerge for ModelConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.name, other.name);
        merge_option_replace(&mut self.model, other.model);
        merge_option_replace(&mut self.api_key, other.api_key);
        merge_option_replace(&mut self.base_url, other.base_url);
        merge_option_map_deep_values(&mut self.variants, other.variants);
    }
}

impl DeepMerge for ModelVariantConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.disabled, other.disabled);
        for (key, value) in other.extra {
            self.extra.insert(key, value);
        }
    }
}

impl DeepMerge for ProviderConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.name, other.name);
        merge_option_replace(&mut self.api_key, other.api_key);
        merge_option_replace(&mut self.base_url, other.base_url);
        merge_option_map_deep_values(&mut self.models, other.models);
        merge_option_json_map(&mut self.options, other.options);
        merge_option_replace(&mut self.npm, other.npm);
        if !other.whitelist.is_empty() {
            self.whitelist = other.whitelist;
        }
        if !other.blacklist.is_empty() {
            self.blacklist = other.blacklist;
        }
    }
}

impl DeepMerge for McpServer {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.server_type, other.server_type);
        if !other.command.is_empty() {
            self.command = other.command;
        }
        merge_option_map_overwrite_values(&mut self.environment, other.environment);
        merge_option_replace(&mut self.url, other.url);
        merge_option_replace(&mut self.enabled, other.enabled);
        merge_option_replace(&mut self.timeout, other.timeout);
        merge_option_map_overwrite_values(&mut self.headers, other.headers);
        merge_option_replace(&mut self.oauth, other.oauth);
        // Legacy fields
        if !other.args.is_empty() {
            self.args = other.args;
        }
        merge_option_map_overwrite_values(&mut self.env, other.env);
        merge_option_replace(&mut self.client_id, other.client_id);
        merge_option_replace(&mut self.authorization_url, other.authorization_url);
    }
}

impl DeepMerge for McpServerConfig {
    fn deep_merge(&mut self, other: Self) {
        match other {
            McpServerConfig::Enabled { enabled } => match self {
                McpServerConfig::Enabled {
                    enabled: target_enabled,
                } => *target_enabled = enabled,
                McpServerConfig::Full(target_server) => target_server.enabled = Some(enabled),
            },
            McpServerConfig::Full(mut source_server) => match self {
                McpServerConfig::Full(target_server) => target_server.deep_merge(source_server),
                McpServerConfig::Enabled { enabled } => {
                    if source_server.enabled.is_none() {
                        source_server.enabled = Some(*enabled);
                    }
                    *self = McpServerConfig::Full(source_server);
                }
            },
        }
    }
}

impl DeepMerge for FormatterEntry {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.disabled, other.disabled);
        if !other.command.is_empty() {
            self.command = other.command;
        }
        merge_option_map_overwrite_values(&mut self.environment, other.environment);
        if !other.extensions.is_empty() {
            self.extensions = other.extensions;
        }
    }
}

impl DeepMerge for FormatterConfig {
    fn deep_merge(&mut self, other: Self) {
        match other {
            FormatterConfig::Disabled(value) => *self = FormatterConfig::Disabled(value),
            FormatterConfig::Enabled(source_map) => match self {
                FormatterConfig::Disabled(_) => *self = FormatterConfig::Enabled(source_map),
                FormatterConfig::Enabled(target_map) => {
                    merge_map_deep_values(target_map, source_map);
                }
            },
        }
    }
}

impl DeepMerge for LspServerConfig {
    fn deep_merge(&mut self, other: Self) {
        if !other.command.is_empty() {
            self.command = other.command;
        }
        if !other.extensions.is_empty() {
            self.extensions = other.extensions;
        }
        merge_option_replace(&mut self.disabled, other.disabled);
        merge_option_map_overwrite_values(&mut self.env, other.env);
        merge_option_json_map(&mut self.initialization, other.initialization);
    }
}

impl DeepMerge for LspConfig {
    fn deep_merge(&mut self, other: Self) {
        match other {
            LspConfig::Disabled(value) => *self = LspConfig::Disabled(value),
            LspConfig::Enabled(source_map) => match self {
                LspConfig::Disabled(_) => *self = LspConfig::Enabled(source_map),
                LspConfig::Enabled(target_map) => {
                    merge_map_deep_values(target_map, source_map);
                }
            },
        }
    }
}

impl DeepMerge for PermissionConfig {
    fn deep_merge(&mut self, other: Self) {
        for (key, value) in other.rules {
            self.rules.insert(key, value);
        }
    }
}

impl DeepMerge for EnterpriseConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.url, other.url);
        merge_option_replace(&mut self.managed_config_dir, other.managed_config_dir);
    }
}

impl DeepMerge for CompactionConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.auto, other.auto);
        merge_option_replace(&mut self.prune, other.prune);
        merge_option_replace(&mut self.reserved, other.reserved);
    }
}

impl DeepMerge for ExperimentalConfig {
    fn deep_merge(&mut self, other: Self) {
        merge_option_replace(&mut self.disable_paste_summary, other.disable_paste_summary);
        merge_option_replace(&mut self.batch_tool, other.batch_tool);
        merge_option_replace(&mut self.open_telemetry, other.open_telemetry);
        if !other.primary_tools.is_empty() {
            self.primary_tools = other.primary_tools;
        }
        merge_option_replace(&mut self.continue_loop_on_deny, other.continue_loop_on_deny);
        merge_option_replace(&mut self.mcp_timeout, other.mcp_timeout);
    }
}

impl Config {
    pub fn merge(&mut self, other: Config) {
        merge_option_replace(&mut self.schema, other.schema);
        merge_option_replace(&mut self.theme, other.theme);
        merge_option_deep(&mut self.keybinds, other.keybinds);
        merge_option_replace(&mut self.log_level, other.log_level);
        merge_option_deep(&mut self.tui, other.tui);
        merge_option_deep(&mut self.server, other.server);
        merge_option_map_deep_values(&mut self.command, other.command);
        merge_option_deep(&mut self.skills, other.skills);
        merge_option_deep(&mut self.watcher, other.watcher);
        merge_option_replace(&mut self.snapshot, other.snapshot);
        merge_option_replace(&mut self.share, other.share);
        merge_option_replace(&mut self.autoshare, other.autoshare);
        merge_option_replace(&mut self.autoupdate, other.autoupdate);
        merge_option_replace(&mut self.model, other.model);
        merge_option_replace(&mut self.small_model, other.small_model);
        merge_option_replace(&mut self.default_agent, other.default_agent);
        merge_option_replace(&mut self.username, other.username);
        merge_option_deep(&mut self.mode, other.mode);
        merge_option_deep(&mut self.agent, other.agent);
        merge_option_map_deep_values(&mut self.provider, other.provider);
        merge_option_map_deep_values(&mut self.mcp, other.mcp);
        merge_option_deep(&mut self.formatter, other.formatter);
        merge_option_deep(&mut self.lsp, other.lsp);
        merge_option_replace(&mut self.layout, other.layout);
        merge_option_deep(&mut self.permission, other.permission);
        merge_option_map_overwrite_values(&mut self.tools, other.tools);
        merge_option_deep(&mut self.enterprise, other.enterprise);
        merge_option_deep(&mut self.compaction, other.compaction);
        merge_option_deep(&mut self.experimental, other.experimental);
        merge_option_map_overwrite_values(&mut self.env, other.env);

        append_unique_keep_order(&mut self.plugin, other.plugin);
        append_unique_keep_order(&mut self.instructions, other.instructions);

        if !other.disabled_providers.is_empty() {
            self.disabled_providers = other.disabled_providers;
        }
        if !other.enabled_providers.is_empty() {
            self.enabled_providers = other.enabled_providers;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_nested_structs_without_losing_existing_fields() {
        let mut base = Config {
            keybinds: Some(KeybindsConfig {
                submit: Some("enter".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let overlay = Config {
            keybinds: Some(KeybindsConfig {
                interrupt: Some("esc".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        base.merge(overlay);

        let merged = base.keybinds.unwrap();
        assert_eq!(merged.submit, Some("enter".to_string()));
        assert_eq!(merged.interrupt, Some("esc".to_string()));
    }

    #[test]
    fn merges_maps_recursively_for_same_keys() {
        let mut base = Config {
            provider: Some(HashMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    base_url: Some("https://old".to_string()),
                    models: Some(HashMap::from([(
                        "gpt-4o".to_string(),
                        ModelConfig {
                            api_key: Some("old-key".to_string()),
                            ..Default::default()
                        },
                    )])),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };

        let overlay = Config {
            provider: Some(HashMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    api_key: Some("new-provider-key".to_string()),
                    models: Some(HashMap::from([(
                        "gpt-4o".to_string(),
                        ModelConfig {
                            model: Some("gpt-4o-2026".to_string()),
                            ..Default::default()
                        },
                    )])),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };

        base.merge(overlay);

        let provider = base.provider.unwrap().remove("openai").unwrap();
        assert_eq!(provider.base_url, Some("https://old".to_string()));
        assert_eq!(provider.api_key, Some("new-provider-key".to_string()));

        let model = provider.models.unwrap().remove("gpt-4o").unwrap();
        assert_eq!(model.api_key, Some("old-key".to_string()));
        assert_eq!(model.model, Some("gpt-4o-2026".to_string()));
    }

    #[test]
    fn plugin_and_instruction_arrays_append_unique_keep_order() {
        let mut base = Config {
            plugin: vec!["a".to_string(), "b".to_string()],
            instructions: vec!["one".to_string(), "two".to_string()],
            ..Default::default()
        };

        let overlay = Config {
            plugin: vec!["b".to_string(), "c".to_string()],
            instructions: vec!["two".to_string(), "three".to_string()],
            ..Default::default()
        };

        base.merge(overlay);

        assert_eq!(
            base.plugin,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
        assert_eq!(
            base.instructions,
            vec!["one".to_string(), "two".to_string(), "three".to_string()]
        );
    }

    #[test]
    fn provider_lists_follow_replace_semantics_instead_of_union() {
        let mut base = Config {
            disabled_providers: vec!["anthropic".to_string()],
            enabled_providers: vec!["openai".to_string()],
            ..Default::default()
        };

        let overlay = Config {
            disabled_providers: vec!["google".to_string()],
            ..Default::default()
        };

        base.merge(overlay);

        assert_eq!(base.disabled_providers, vec!["google".to_string()]);
        assert_eq!(base.enabled_providers, vec!["openai".to_string()]);
    }

    #[test]
    fn mcp_enabled_flag_overlay_keeps_existing_full_server_fields() {
        let mut base = Config {
            mcp: Some(HashMap::from([(
                "repo".to_string(),
                McpServerConfig::Full(McpServer {
                    command: vec!["node".to_string(), "mcp.js".to_string()],
                    timeout: Some(3000),
                    ..Default::default()
                }),
            )])),
            ..Default::default()
        };

        let overlay = Config {
            mcp: Some(HashMap::from([(
                "repo".to_string(),
                McpServerConfig::Enabled { enabled: false },
            )])),
            ..Default::default()
        };

        base.merge(overlay);

        let server = base.mcp.unwrap().remove("repo").unwrap();
        match server {
            McpServerConfig::Full(server) => {
                assert_eq!(
                    server.command,
                    vec!["node".to_string(), "mcp.js".to_string()]
                );
                assert_eq!(server.timeout, Some(3000));
                assert_eq!(server.enabled, Some(false));
            }
            McpServerConfig::Enabled { .. } => panic!("expected full MCP server config"),
        }
    }

    #[test]
    fn agent_configs_support_dynamic_keys_and_deep_merge() {
        let mut base = Config {
            agent: Some(AgentConfigs {
                entries: HashMap::from([(
                    "reviewer".to_string(),
                    AgentConfig {
                        prompt: Some("old prompt".to_string()),
                        options: Some(HashMap::from([("a".to_string(), serde_json::json!(1))])),
                        ..Default::default()
                    },
                )]),
            }),
            ..Default::default()
        };

        let overlay = Config {
            agent: Some(AgentConfigs {
                entries: HashMap::from([
                    (
                        "reviewer".to_string(),
                        AgentConfig {
                            prompt: Some("new prompt".to_string()),
                            options: Some(HashMap::from([("b".to_string(), serde_json::json!(2))])),
                            ..Default::default()
                        },
                    ),
                    (
                        "research".to_string(),
                        AgentConfig {
                            mode: Some(AgentMode::Subagent),
                            ..Default::default()
                        },
                    ),
                ]),
            }),
            ..Default::default()
        };

        base.merge(overlay);

        let agents = base.agent.unwrap().entries;
        let reviewer = agents.get("reviewer").unwrap();
        assert_eq!(reviewer.prompt.as_deref(), Some("new prompt"));
        let options = reviewer.options.as_ref().unwrap();
        assert_eq!(options.get("a"), Some(&serde_json::json!(1)));
        assert_eq!(options.get("b"), Some(&serde_json::json!(2)));
        assert!(agents.contains_key("research"));
    }
}
