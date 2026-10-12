use crate::schema::{
    AgentConfig, AgentConfigs, AgentMode, CommandConfig, PermissionAction, PermissionConfig,
    PermissionRule,
};
use crate::Config;
use anyhow::{Context, Result};
use jsonc_parser::{parse_to_serde_value, ParseOptions};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ConfigLoader {
    config: Config,
    config_paths: Vec<PathBuf>,
}

impl ConfigLoader {
    pub fn new() -> Self {
        Self {
            config: Config::default(),
            config_paths: Vec::new(),
        }
    }

    /// Current merged config (for tests and inspection).
    pub fn get_config(&self) -> Config {
        self.config.clone()
    }

    pub fn load_from_str(&mut self, content: &str) -> Result<()> {
        let config: Config =
            parse_jsonc(content).with_context(|| "Failed to parse config content")?;
        self.config.merge(config);
        Ok(())
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;

        // Apply {env:VAR} substitution
        let content = substitute_env_vars(&content);

        // Apply {file:path} substitution
        let base_dir = path.parent().unwrap_or(Path::new("."));
        let content = resolve_file_references(&content, base_dir)
            .with_context(|| format!("Failed to resolve file references in: {:?}", path))?;

        let config: Config = parse_jsonc(&content)
            .with_context(|| format!("Failed to parse config file: {:?}", path))?;

        self.config.merge(config);
        self.config_paths.push(path.to_path_buf());
        Ok(())
    }

    pub fn load_global(&mut self) -> Result<()> {
        let global_config_path = get_global_config_path();

        for ext in &["jsonc", "json"] {
            let path = global_config_path.with_extension(ext);
            if path.exists() {
                self.load_from_file(&path)?;
                break;
            }
        }

        if let Some(global_config_dir) = global_config_path.parent() {
            if let Some(migrated_path) =
                migrate_legacy_toml_config(global_config_dir, &mut self.config)
            {
                if !self.config_paths.contains(&migrated_path) {
                    self.config_paths.push(migrated_path);
                }
            }
        }

        Ok(())
    }

    pub fn load_project<P: AsRef<Path>>(&mut self, project_dir: P) -> Result<()> {
        let input = project_dir.as_ref();
        let start_dir = if input.is_dir() {
            input.to_path_buf()
        } else {
            input
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| input.to_path_buf())
        };
        let start_dir = normalize_existing_path(&start_dir);
        let stop_dir = detect_worktree_stop(&start_dir);

        // TS parity: findUp per target, then load from ancestor -> descendant.
        for target in [
            "opencode.jsonc",
            "opencode.json",
            ".opencode/opencode.jsonc",
            ".opencode/opencode.json",
        ] {
            let found = find_up(target, &start_dir, &stop_dir);
            for path in found.into_iter().rev() {
                self.load_from_file(path)?;
            }
        }

        Ok(())
    }

    pub fn load_from_env(&mut self) -> Result<()> {
        if let Ok(config_path) = env::var("OPENCODE_CONFIG") {
            self.load_from_file(&config_path)?;
        }

        Ok(())
    }

    /// Load inline config content from OPENCODE_CONFIG_CONTENT env var.
    /// Per TS parity, this is applied after project config but before managed config.
    pub fn load_from_env_content(&mut self) -> Result<()> {
        if let Ok(config_content) = env::var("OPENCODE_CONFIG_CONTENT") {
            self.load_from_str(&config_content)?;
        }

        Ok(())
    }

    /// Loads all config sources synchronously (without remote wellknown).
    /// Merge order (TS parity):
    /// 1. Global config (~/.config/opencode/opencode.json{,c})
    /// 2. Custom config (OPENCODE_CONFIG)
    /// 3. Project config (opencode.json{,c})
    /// 4. .opencode directories (agents, commands, plugins, modes, config)
    /// 5. Inline config (OPENCODE_CONFIG_CONTENT)
    /// 6. Managed config directory (enterprise, highest priority)
    /// Then: legacy migrations, flag overrides, plugin dedup
    pub fn load_all<P: AsRef<Path>>(&mut self, project_dir: P) -> Result<Config> {
        let project_dir = project_dir.as_ref();

        self.load_global()?;
        self.load_from_env()?;
        self.load_project(project_dir)?;

        // Scan .opencode directories
        let directories = collect_opencode_directories_isolated(project_dir);
        for dir in &directories {
            // Load config files from .opencode dirs
            for ext in &["opencode.jsonc", "opencode.json"] {
                let path = dir.join(ext);
                self.load_from_file(&path)?;
            }

            // Load commands, agents, modes from markdown files
            let commands = load_commands_from_dir(dir);
            if !commands.is_empty() {
                let mut cmd_map = self.config.command.take().unwrap_or_default();
                for (name, cmd) in commands {
                    cmd_map.insert(name, cmd);
                }
                self.config.command = Some(cmd_map);
            }

            let agents = load_agents_from_dir(dir);
            if !agents.is_empty() {
                let mut agent_configs = self.config.agent.take().unwrap_or_default();
                for (name, agent) in agents {
                    if let Some(existing) = agent_configs.entries.get_mut(&name) {
                        // Deep merge
                        merge_agent_config(existing, agent);
                    } else {
                        agent_configs.entries.insert(name, agent);
                    }
                }
                self.config.agent = Some(agent_configs);
            }

            let modes = load_modes_from_dir(dir);
            if !modes.is_empty() {
                let mut agent_configs = self.config.agent.take().unwrap_or_default();
                for (name, agent) in modes {
                    if let Some(existing) = agent_configs.entries.get_mut(&name) {
                        merge_agent_config(existing, agent);
                    } else {
                        agent_configs.entries.insert(name, agent);
                    }
                }
                self.config.agent = Some(agent_configs);
            }

            // Load plugins from .ts/.js files
            let plugins = load_plugins_from_dir(dir);
            for plugin in plugins {
                if !self.config.plugin.contains(&plugin) {
                    self.config.plugin.push(plugin);
                }
            }
        }

        // Inline config content overrides all non-managed config sources
        self.load_from_env_content()?;

        // Load managed config (enterprise, highest priority)
        self.load_managed_config()?;

        // Apply legacy migrations and flag overrides
        apply_post_load_transforms(&mut self.config);

        Ok(self.config.clone())
    }

    /// Loads all config sources including remote `.well-known/opencode` endpoints.
    /// Merge order (low -> high precedence):
    /// 1. Remote .well-known/opencode (org defaults) -- lowest priority
    /// 2. Global config (~/.config/opencode/opencode.json{,c})
    /// 3. Custom config (OPENCODE_CONFIG)
    /// 4. Project config (opencode.json{,c})
    /// 5. .opencode directories
    /// 6. Inline config (OPENCODE_CONFIG_CONTENT)
    /// 7. Managed config directory (enterprise, highest priority)
    pub async fn load_all_with_remote<P: AsRef<Path>>(&mut self, project_dir: P) -> Result<Config> {
        let wellknown_config = crate::wellknown::load_wellknown().await;
        self.config.merge(wellknown_config);

        // Delegate to load_all which handles everything else
        self.load_all(project_dir)
    }

    /// Load managed config files from enterprise directory (highest priority).
    fn load_managed_config(&mut self) -> Result<()> {
        let managed_dir = get_managed_config_dir();
        if managed_dir.exists() {
            for ext in &["opencode.jsonc", "opencode.json"] {
                let path = managed_dir.join(ext);
                self.load_from_file(&path)?;
            }
        }
        Ok(())
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_paths(&self) -> &[PathBuf] {
        &self.config_paths
    }
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self::new()
    }
}

fn get_global_config_path() -> PathBuf {
    let xdg_root = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());

    resolve_global_config_path(dirs::home_dir(), dirs::config_dir(), xdg_root)
}

fn global_config_base_exists(base: &Path) -> bool {
    base.with_extension("jsonc").exists() || base.with_extension("json").exists()
}

fn resolve_global_config_path(
    home: Option<PathBuf>,
    platform: Option<PathBuf>,
    xdg_root: Option<PathBuf>,
) -> PathBuf {
    let dot_root = xdg_root.or_else(|| home.as_ref().map(|h| h.join(".config")));

    if let Some(root) = dot_root {
        let candidate = root.join("opencode/opencode");
        if global_config_base_exists(&candidate) {
            return candidate;
        }
        if let Some(platform) = platform.as_ref() {
            let platform_candidate = platform.join("opencode/opencode");
            if global_config_base_exists(&platform_candidate) {
                return platform_candidate;
            }
        }
        return candidate;
    }

    platform
        .map(|p| p.join("opencode/opencode"))
        .unwrap_or_else(|| PathBuf::from(".config/opencode/opencode"))
}

/// Migrate legacy global TOML config (`~/.config/opencode/config`) into
/// `opencode.json` and merge it into the currently loaded config.
fn migrate_legacy_toml_config(config_dir: &Path, config: &mut Config) -> Option<PathBuf> {
    let legacy_path = config_dir.join("config");
    if !legacy_path.exists() {
        return None;
    }

    let content = match fs::read_to_string(&legacy_path) {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!(
                path = %legacy_path.display(),
                %error,
                "failed to read legacy TOML config"
            );
            return None;
        }
    };

    let legacy_toml: toml::Value = match toml::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                path = %legacy_path.display(),
                %error,
                "failed to parse legacy TOML config"
            );
            return None;
        }
    };
    let mut legacy_json = match serde_json::to_value(legacy_toml) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                path = %legacy_path.display(),
                %error,
                "failed to convert legacy TOML config to JSON value"
            );
            return None;
        }
    };

    let mut migrated = Config::default();
    if let Some(table) = legacy_json.as_object_mut() {
        let provider = table
            .remove("provider")
            .and_then(|value| value.as_str().map(str::to_owned));
        let model = table
            .remove("model")
            .and_then(|value| value.as_str().map(str::to_owned));
        if let (Some(provider), Some(model)) = (provider, model) {
            migrated.model = Some(format!("{provider}/{model}"));
        }
    }

    match serde_json::from_value::<Config>(legacy_json) {
        Ok(rest) => migrated.merge(rest),
        Err(error) => {
            tracing::warn!(
                path = %legacy_path.display(),
                %error,
                "failed to deserialize legacy TOML config into schema"
            );
        }
    }

    if migrated.schema.is_none() {
        migrated.schema = Some("https://opencode.ai/config.json".to_string());
    }
    config.merge(migrated);

    let json_path = config_dir.join("opencode.json");
    if let Some(parent) = json_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            tracing::warn!(
                path = %parent.display(),
                %error,
                "failed to create config directory during TOML migration"
            );
            return None;
        }
    }

    let serialized = match serde_json::to_string_pretty(config) {
        Ok(json) => json,
        Err(error) => {
            tracing::warn!(
                path = %json_path.display(),
                %error,
                "failed to serialize migrated config"
            );
            return None;
        }
    };

    if let Err(error) = fs::write(&json_path, serialized) {
        tracing::warn!(
            path = %json_path.display(),
            %error,
            "failed to write migrated JSON config"
        );
        return None;
    }

    if let Err(error) = fs::remove_file(&legacy_path) {
        tracing::warn!(
            path = %legacy_path.display(),
            %error,
            "failed to remove legacy TOML config after migration"
        );
    }

    tracing::info!(
        legacy = %legacy_path.display(),
        migrated = %json_path.display(),
        "migrated legacy TOML config"
    );

    Some(json_path)
}

/// Substitute `{env:VAR}` patterns with environment variable values.
/// Works on the raw JSONC text before parsing.
fn substitute_env_vars(text: &str) -> String {
    let re = regex::Regex::new(r"\{env:([^}]+)\}").unwrap();
    re.replace_all(text, |caps: &regex::Captures| {
        let var_name = &caps[1];
        std::env::var(var_name).unwrap_or_default()
    })
    .to_string()
}

/// Resolve `{file:path}` patterns by reading file contents.
/// Skips patterns on commented lines. Resolves relative paths from `base_dir`.
fn resolve_file_references(text: &str, base_dir: &Path) -> Result<String> {
    let re = regex::Regex::new(r"\{file:([^}]+)\}").unwrap();

    let mut result = text.to_string();

    // Collect all matches first to avoid borrow issues
    let matches: Vec<(String, String)> = re
        .captures_iter(text)
        .map(|caps| {
            let full_match = caps.get(0).unwrap().as_str().to_string();
            let file_path_str = caps[1].to_string();
            (full_match, file_path_str)
        })
        .collect();

    for (full_match, file_path_str) in matches {
        // Check if the match is on a commented line
        let match_start = match text.find(&full_match) {
            Some(pos) => pos,
            None => continue,
        };
        let line_start = text[..match_start].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let line_end = text[line_start..]
            .find('\n')
            .map(|p| line_start + p)
            .unwrap_or(text.len());
        let line = &text[line_start..line_end];
        if line.trim().starts_with("//") {
            continue;
        }

        // Resolve the file path
        let resolved = if file_path_str.starts_with("~/") {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("~"))
                .join(&file_path_str[2..])
        } else if Path::new(&file_path_str).is_absolute() {
            PathBuf::from(&file_path_str)
        } else {
            base_dir.join(&file_path_str)
        };

        // Read the file
        let content = fs::read_to_string(&resolved).with_context(|| {
            format!(
                "bad file reference: \"{}\" - {} does not exist",
                full_match,
                resolved.display()
            )
        })?;
        let content = content.trim();

        // Escape for JSON string context (newlines, quotes)
        let escaped = content
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");

        result = result.replace(&full_match, &escaped);
    }

    Ok(result)
}

fn parse_jsonc(content: &str) -> Result<Config> {
    let parse_options = ParseOptions {
        allow_trailing_commas: true,
        ..Default::default()
    };
    let parsed = parse_to_serde_value(content, &parse_options)
        .with_context(|| "Failed to parse JSONC")?
        .context("Config content is empty")?;
    serde_json::from_value(parsed).with_context(|| "Failed to parse config JSON")
}

fn normalize_existing_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn detect_worktree_stop(start: &Path) -> PathBuf {
    let mut current = normalize_existing_path(start);
    let mut topmost = current.clone();
    loop {
        if current.join(".git").exists() {
            return current;
        }
        let Some(parent) = current.parent() else {
            return topmost;
        };
        if parent == current {
            return topmost;
        }
        topmost = parent.to_path_buf();
        current = parent.to_path_buf();
    }
}

fn find_up(target: &str, start: &Path, stop: &Path) -> Vec<PathBuf> {
    let mut current = normalize_existing_path(start);
    let stop = normalize_existing_path(stop);
    let mut result = Vec::new();

    loop {
        let candidate = current.join(target);
        if candidate.exists() {
            result.push(candidate);
        }
        if current == stop {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }

    result
}

/// Get the managed config directory for enterprise deployments.
fn get_managed_config_dir() -> PathBuf {
    if let Ok(test_dir) = env::var("OPENCODE_TEST_MANAGED_CONFIG_DIR") {
        return PathBuf::from(test_dir);
    }
    if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/opencode")
    } else if cfg!(target_os = "windows") {
        let program_data =
            env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
        PathBuf::from(program_data).join("opencode")
    } else {
        PathBuf::from("/etc/opencode")
    }
}

/// Collect .opencode directories from project hierarchy and global config.
fn collect_opencode_directories(project_dir: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();

    // Global config directory
    let global_config = get_global_config_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    if global_config.exists() {
        directories.push(global_config);
    }

    // Project .opencode directories (walk up from project_dir to worktree root)
    let start_dir = normalize_existing_path(project_dir);
    let stop_dir = detect_worktree_stop(&start_dir);
    let found = find_up(".opencode", &start_dir, &stop_dir);
    // Reverse so ancestor dirs come first (lower priority)
    for path in found.into_iter().rev() {
        directories.push(path);
    }

    // Home directory .opencode
    if let Some(home) = dirs::home_dir() {
        let home_opencode = home.join(".opencode");
        if home_opencode.exists() && !directories.contains(&home_opencode) {
            directories.push(home_opencode);
        }
    }

    // OPENCODE_CONFIG_DIR override
    if let Ok(config_dir) = env::var("OPENCODE_CONFIG_DIR") {
        let dir = PathBuf::from(config_dir);
        if !directories.contains(&dir) {
            directories.push(dir);
        }
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    directories.retain(|d| seen.insert(d.clone()));

    directories
}

/// Same as `collect_opencode_directories`, but when OPENCODE_CONFIG_DIR_ISOLATED is set
/// the result is ONLY that directory.
///
/// Without this, OPENCODE_CONFIG_DIR is additive: the global `~/.config/opencode` is
/// pushed first (above) and always wins for plugins. In a contained deployment that is
/// not a preference but a leak — the harness loads the operator's live TS plugins, and a
/// plugin that fails to answer blocks startup before the TCP listener is ever bound.
///
/// Opt-in so default behaviour is bit-identical for every existing user.
pub fn collect_opencode_directories_isolated(project_dir: &Path) -> Vec<PathBuf> {
    let isolated = env::var("OPENCODE_CONFIG_DIR_ISOLATED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !isolated {
        return collect_opencode_directories(project_dir);
    }
    match env::var("OPENCODE_CONFIG_DIR") {
        Ok(dir) if !dir.trim().is_empty() => vec![PathBuf::from(dir.trim())],
        _ => Vec::new(),
    }
}

/// Load command definitions from markdown files in {command,commands}/**/*.md
fn load_commands_from_dir(dir: &Path) -> HashMap<String, CommandConfig> {
    let mut result = HashMap::new();

    for subdir_name in &["command", "commands"] {
        let subdir = dir.join(subdir_name);
        if !subdir.is_dir() {
            continue;
        }
        if let Ok(entries) = glob_md_files(&subdir) {
            for entry in entries {
                if let Some((name, content)) = parse_markdown_command(&entry, dir) {
                    result.insert(name, content);
                }
            }
        }
    }

    result
}

/// Load agent definitions from markdown files in {agent,agents}/**/*.md
fn load_agents_from_dir(dir: &Path) -> HashMap<String, AgentConfig> {
    let mut result = HashMap::new();

    for subdir_name in &["agent", "agents"] {
        let subdir = dir.join(subdir_name);
        if !subdir.is_dir() {
            continue;
        }
        if let Ok(entries) = glob_md_files(&subdir) {
            for entry in entries {
                if let Some((name, config)) = parse_markdown_agent(&entry, dir) {
                    result.insert(name, config);
                }
            }
        }
    }

    result
}

/// Load mode definitions from markdown files in {mode,modes}/*.md
fn load_modes_from_dir(dir: &Path) -> HashMap<String, AgentConfig> {
    let mut result = HashMap::new();

    for subdir_name in &["mode", "modes"] {
        let subdir = dir.join(subdir_name);
        if !subdir.is_dir() {
            continue;
        }
        if let Ok(entries) = glob_md_files(&subdir) {
            for entry in entries {
                if let Some((name, mut config)) = parse_markdown_agent(&entry, dir) {
                    // Modes are always primary agents
                    config.mode = Some(AgentMode::Primary);
                    result.insert(name, config);
                }
            }
        }
    }

    result
}

/// Load plugin paths from .ts/.js files in {plugin,plugins}/*.{ts,js}
fn load_plugins_from_dir(dir: &Path) -> Vec<String> {
    let mut plugins = Vec::new();

    for subdir_name in &["plugin", "plugins"] {
        let subdir = dir.join(subdir_name);
        if !subdir.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&subdir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "ts" || ext == "js" {
                        // Convert to file:// URL like TS does
                        let url = format!("file://{}", path.display());
                        plugins.push(url);
                    }
                }
            }
        }
    }

    plugins
}

/// Recursively find all .md files in a directory.
fn glob_md_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut results = Vec::new();
    glob_md_files_recursive(dir, &mut results)?;
    Ok(results)
}

fn glob_md_files_recursive(dir: &Path, results: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            glob_md_files_recursive(&path, results)?;
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            results.push(path);
        }
    }
    Ok(())
}

/// Parse a markdown file as a command definition.
/// Extracts YAML frontmatter and body content.
fn parse_markdown_command(path: &Path, base_dir: &Path) -> Option<(String, CommandConfig)> {
    let content = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = split_frontmatter(&content);

    // Derive name from relative path
    let name = derive_name_from_path(path, base_dir, &["command", "commands"]);

    let mut config = if let Some(fm) = frontmatter {
        serde_json::from_value::<CommandConfig>(serde_yaml_frontmatter_to_json(&fm))
            .unwrap_or_default()
    } else {
        CommandConfig::default()
    };

    config.template = Some(body.trim().to_string());

    Some((name, config))
}

/// Parse a markdown file as an agent definition.
fn parse_markdown_agent(path: &Path, base_dir: &Path) -> Option<(String, AgentConfig)> {
    let content = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = split_frontmatter(&content);

    let name = derive_name_from_path(path, base_dir, &["agent", "agents", "mode", "modes"]);

    let mut config = if let Some(fm) = frontmatter {
        serde_json::from_value::<AgentConfig>(serde_yaml_frontmatter_to_json(&fm))
            .unwrap_or_default()
    } else {
        AgentConfig::default()
    };

    config.prompt = Some(body.trim().to_string());

    Some((name, config))
}

/// Split markdown content into optional YAML frontmatter and body.
fn split_frontmatter(content: &str) -> (Option<String>, String) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, content.to_string());
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    if let Some(end_idx) = after_first.find("\n---") {
        let fm = after_first[..end_idx].trim().to_string();
        let body_start = end_idx + 4; // skip \n---
        let body = if body_start < after_first.len() {
            after_first[body_start..].to_string()
        } else {
            String::new()
        };
        (Some(fm), body)
    } else {
        (None, content.to_string())
    }
}

/// Fallback sanitization for invalid YAML frontmatter.
/// Matches TS `ConfigMarkdown.fallbackSanitization`: if a top-level value
/// contains a colon (which confuses simple YAML parsers), convert it to a
/// block scalar so the value is preserved verbatim.
fn fallback_sanitize_yaml(yaml: &str) -> String {
    let mut result: Vec<String> = Vec::new();
    for line in yaml.lines() {
        let trimmed = line.trim();
        // Pass through comments and empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            result.push(line.to_string());
            continue;
        }
        // Pass through continuation/indented lines
        if line.starts_with(char::is_whitespace) {
            result.push(line.to_string());
            continue;
        }
        // Match top-level key: value
        let kv_re = regex::Regex::new(r"^([a-zA-Z_][a-zA-Z0-9_]*)\s*:\s*(.*)$").unwrap();
        let Some(caps) = kv_re.captures(line) else {
            result.push(line.to_string());
            continue;
        };
        let key = &caps[1];
        let value = caps[2].trim();
        // Skip if value is empty, already quoted, or uses block scalar indicator
        if value.is_empty()
            || value == ">"
            || value == "|"
            || value == "|-"
            || value == ">-"
            || value.starts_with('"')
            || value.starts_with('\'')
        {
            result.push(line.to_string());
            continue;
        }
        // If value contains a colon, convert to block scalar
        if value.contains(':') {
            result.push(format!("{}: |-", key));
            result.push(format!("  {}", value));
            continue;
        }
        result.push(line.to_string());
    }
    result.join("\n")
}

/// Parse a YAML scalar value string into a JSON value.
fn yaml_scalar_to_json(value: &str) -> serde_json::Value {
    let value = value.trim();
    if value.is_empty() {
        return serde_json::Value::Null;
    }
    // Booleans
    if value == "true" || value == "True" || value == "TRUE" {
        return serde_json::Value::Bool(true);
    }
    if value == "false" || value == "False" || value == "FALSE" {
        return serde_json::Value::Bool(false);
    }
    if value == "null" || value == "Null" || value == "NULL" || value == "~" {
        return serde_json::Value::Null;
    }
    // Numbers
    if let Ok(n) = value.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    if let Ok(n) = value.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return serde_json::Value::Number(num);
        }
    }
    // Strip surrounding quotes
    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        return serde_json::Value::String(value[1..value.len() - 1].to_string());
    }
    serde_json::Value::String(value.to_string())
}

/// Parse an inline YAML flow sequence like `[a, b, c]` into a JSON array.
fn parse_inline_list(value: &str) -> Option<serde_json::Value> {
    let trimmed = value.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    if inner.is_empty() {
        return Some(serde_json::Value::Array(Vec::new()));
    }
    let items: Vec<serde_json::Value> = inner
        .split(',')
        .map(|item| yaml_scalar_to_json(item.trim()))
        .collect();
    Some(serde_json::Value::Array(items))
}

/// Parse an inline YAML flow mapping like `{a: 1, b: true}` into a JSON object.
fn parse_inline_map(value: &str) -> Option<serde_json::Value> {
    let trimmed = value.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return None;
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    if inner.is_empty() {
        return Some(serde_json::Value::Object(serde_json::Map::new()));
    }
    let mut map = serde_json::Map::new();
    for pair in inner.split(',') {
        if let Some((k, v)) = pair.split_once(':') {
            map.insert(k.trim().to_string(), yaml_scalar_to_json(v));
        }
    }
    Some(serde_json::Value::Object(map))
}

/// Compute the indentation level (number of leading spaces) of a line.
fn indent_level(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// YAML frontmatter to JSON conversion.
/// Handles: flat key-value, inline lists/maps, multi-line dash lists,
/// nested objects (indentation-based), and block scalars (| and >).
/// Falls back to sanitized re-parse on failure.
fn serde_yaml_frontmatter_to_json(yaml: &str) -> serde_json::Value {
    match parse_yaml_mapping(yaml) {
        Some(value) => value,
        None => {
            // Fallback: sanitize and retry
            let sanitized = fallback_sanitize_yaml(yaml);
            parse_yaml_mapping(&sanitized)
                .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()))
        }
    }
}

/// Parse a YAML mapping (object) from a string. Returns None on structural failure.
fn parse_yaml_mapping(yaml: &str) -> Option<serde_json::Value> {
    let lines: Vec<&str> = yaml.lines().collect();
    let (map, _) = parse_yaml_mapping_lines(&lines, 0, 0)?;
    Some(serde_json::Value::Object(map))
}

/// Parse YAML mapping lines starting at `start` index with expected `base_indent`.
/// Returns the parsed map and the index of the next unconsumed line.
fn parse_yaml_mapping_lines(
    lines: &[&str],
    start: usize,
    base_indent: usize,
) -> Option<(serde_json::Map<String, serde_json::Value>, usize)> {
    let mut map = serde_json::Map::new();
    let mut i = start;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        let current_indent = indent_level(line);

        // If we've dedented past our base, we're done with this mapping
        if current_indent < base_indent {
            break;
        }

        // Handle list items at this level (shouldn't appear in a mapping context at same indent
        // unless it's a top-level list, which we don't support as root)
        if trimmed.starts_with("- ") && current_indent == base_indent {
            // This is a list at the mapping level -- not valid for our use case, skip
            break;
        }

        // Parse key: value
        if let Some((key_part, value_part)) = trimmed.split_once(':') {
            let key = key_part.trim().to_string();
            let value_str = value_part.trim();

            if value_str.is_empty() {
                // Value is on subsequent indented lines -- could be a nested map, list, or block scalar
                i += 1;
                if i < lines.len() {
                    let next_trimmed = lines[i].trim();
                    let next_indent = indent_level(lines[i]);
                    if next_indent > current_indent && next_trimmed.starts_with("- ") {
                        // It's a dash-list
                        let (list, next_i) = parse_yaml_list_lines(lines, i, next_indent);
                        map.insert(key, serde_json::Value::Array(list));
                        i = next_i;
                    } else if next_indent > current_indent {
                        // It's a nested mapping
                        if let Some((nested_map, next_i)) =
                            parse_yaml_mapping_lines(lines, i, next_indent)
                        {
                            map.insert(key, serde_json::Value::Object(nested_map));
                            i = next_i;
                        } else {
                            // Treat as string value from remaining indented lines
                            let (text, next_i) = collect_block_text(lines, i, current_indent);
                            map.insert(key, serde_json::Value::String(text));
                            i = next_i;
                        }
                    } else {
                        // Empty value, next line is at same or lower indent
                        map.insert(key, serde_json::Value::Null);
                    }
                } else {
                    map.insert(key, serde_json::Value::Null);
                }
            } else if value_str == "|" || value_str == "|-" || value_str == "|+" {
                // Block scalar (literal)
                i += 1;
                let (text, next_i) = collect_block_text(lines, i, current_indent);
                let text = if value_str == "|-" {
                    text.trim_end().to_string()
                } else {
                    text
                };
                map.insert(key, serde_json::Value::String(text));
                i = next_i;
            } else if value_str == ">" || value_str == ">-" || value_str == ">+" {
                // Block scalar (folded)
                i += 1;
                let (text, next_i) = collect_block_text(lines, i, current_indent);
                // Folded: join lines with spaces
                let folded = text.lines().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
                let folded = if value_str == ">-" {
                    folded.trim_end().to_string()
                } else {
                    folded
                };
                map.insert(key, serde_json::Value::String(folded));
                i = next_i;
            } else if let Some(list) = parse_inline_list(value_str) {
                map.insert(key, list);
                i += 1;
            } else if let Some(obj) = parse_inline_map(value_str) {
                map.insert(key, obj);
                i += 1;
            } else {
                map.insert(key, yaml_scalar_to_json(value_str));
                i += 1;
            }
        } else {
            // Line doesn't match key: value pattern, skip
            i += 1;
        }
    }

    Some((map, i))
}

/// Parse YAML list lines (lines starting with "- ") at the given indent level.
/// Returns the list of values and the next unconsumed line index.
fn parse_yaml_list_lines(
    lines: &[&str],
    start: usize,
    base_indent: usize,
) -> (Vec<serde_json::Value>, usize) {
    let mut list = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        let current_indent = indent_level(line);
        if current_indent < base_indent {
            break;
        }
        if current_indent > base_indent {
            // Continuation of previous item, skip
            i += 1;
            continue;
        }

        if let Some(item_str) = trimmed.strip_prefix("- ") {
            let item_str = item_str.trim();
            // Check if item itself is a key: value (nested object in list)
            if item_str.contains(": ") {
                // Could be an inline object item like "- key: value"
                // For simplicity, treat as scalar string
                list.push(yaml_scalar_to_json(item_str));
            } else if let Some(inline_list) = parse_inline_list(item_str) {
                list.push(inline_list);
            } else {
                list.push(yaml_scalar_to_json(item_str));
            }
            i += 1;
        } else {
            break;
        }
    }

    (list, i)
}

/// Collect indented block text lines (for block scalars or multi-line values).
/// Stops when a line at or below `parent_indent` is encountered.
fn collect_block_text(lines: &[&str], start: usize, parent_indent: usize) -> (String, usize) {
    let mut text_lines = Vec::new();
    let mut i = start;
    let mut block_indent: Option<usize> = None;

    while i < lines.len() {
        let line = lines[i];
        // Empty lines are part of the block
        if line.trim().is_empty() {
            text_lines.push("");
            i += 1;
            continue;
        }
        let current_indent = indent_level(line);
        if current_indent <= parent_indent {
            break;
        }
        // Determine the block's base indent from the first non-empty line
        let bi = *block_indent.get_or_insert(current_indent);
        if current_indent >= bi {
            text_lines.push(&line[bi..]);
        } else {
            text_lines.push(line.trim());
        }
        i += 1;
    }

    // Trim trailing empty lines
    while text_lines.last() == Some(&"") {
        text_lines.pop();
    }

    (text_lines.join("\n"), i)
}

/// Derive a name from a file path relative to the base directory.
fn derive_name_from_path(path: &Path, base_dir: &Path, strip_prefixes: &[&str]) -> String {
    let rel = path
        .strip_prefix(base_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    // Strip known directory prefixes
    let mut name = rel.as_str();
    for prefix in strip_prefixes {
        let with_sep = format!("{}/", prefix);
        if let Some(stripped) = name.strip_prefix(&with_sep) {
            name = stripped;
            break;
        }
        // Also try .opencode/ prefix
        let with_opencode = format!(".opencode/{}/", prefix);
        if let Some(stripped) = name.strip_prefix(&with_opencode) {
            name = stripped;
            break;
        }
    }

    // Remove .md extension
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

/// Merge one AgentConfig into another (simple field-level merge).
fn merge_agent_config(target: &mut AgentConfig, source: AgentConfig) {
    if source.name.is_some() {
        target.name = source.name;
    }
    if source.model.is_some() {
        target.model = source.model;
    }
    if source.variant.is_some() {
        target.variant = source.variant;
    }
    if source.temperature.is_some() {
        target.temperature = source.temperature;
    }
    if source.top_p.is_some() {
        target.top_p = source.top_p;
    }
    if source.prompt.is_some() {
        target.prompt = source.prompt;
    }
    if source.disable.is_some() {
        target.disable = source.disable;
    }
    if source.description.is_some() {
        target.description = source.description;
    }
    if source.mode.is_some() {
        target.mode = source.mode;
    }
    if source.hidden.is_some() {
        target.hidden = source.hidden;
    }
    if source.color.is_some() {
        target.color = source.color;
    }
    if source.steps.is_some() {
        target.steps = source.steps;
    }
    if source.max_steps.is_some() {
        target.max_steps = source.max_steps;
    }
    if source.max_tokens.is_some() {
        target.max_tokens = source.max_tokens;
    }
    if let Some(source_opts) = source.options {
        let target_opts = target.options.get_or_insert_with(HashMap::new);
        for (k, v) in source_opts {
            target_opts.insert(k, v);
        }
    }
    if let Some(source_perm) = source.permission {
        if let Some(target_perm) = &mut target.permission {
            for (k, v) in source_perm.rules {
                target_perm.rules.insert(k, v);
            }
        } else {
            target.permission = Some(source_perm);
        }
    }
    if let Some(source_tools) = source.tools {
        let target_tools = target.tools.get_or_insert_with(HashMap::new);
        for (k, v) in source_tools {
            target_tools.insert(k, v);
        }
    }
}

/// Extract canonical plugin name from a specifier.
/// - For file:// URLs: extracts filename without extension
/// - For npm packages: extracts package name without version
pub fn get_plugin_name(plugin: &str) -> String {
    if plugin.starts_with("file://") {
        return Path::new(plugin.trim_start_matches("file://"))
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| plugin.to_string());
    }
    // For npm packages: strip version after last @
    if let Some(last_at) = plugin.rfind('@') {
        if last_at > 0 {
            return plugin[..last_at].to_string();
        }
    }
    plugin.to_string()
}

/// Deduplicate plugins by name, with later entries (higher priority) winning.
/// Since plugins are added in low-to-high priority order,
/// we reverse, deduplicate (keeping first occurrence), then restore order.
pub fn deduplicate_plugins(plugins: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<String> = Vec::new();

    for specifier in plugins.iter().rev() {
        let name = get_plugin_name(specifier);
        if seen.insert(name) {
            unique.push(specifier.clone());
        }
    }

    unique.reverse();
    unique
}

/// Apply post-load transforms: legacy migrations, flag overrides, plugin dedup.
fn apply_post_load_transforms(config: &mut Config) {
    // Migrate deprecated `mode` field to `agent` field
    if let Some(mode_configs) = config.mode.take() {
        let agent_configs = config.agent.get_or_insert_with(AgentConfigs::default);
        for (name, mut mode_agent) in mode_configs.entries {
            mode_agent.mode = Some(AgentMode::Primary);
            if let Some(existing) = agent_configs.entries.get_mut(&name) {
                merge_agent_config(existing, mode_agent);
            } else {
                agent_configs.entries.insert(name, mode_agent);
            }
        }
    }

    // OPENCODE_PERMISSION env var override
    if let Ok(perm_json) = env::var("OPENCODE_PERMISSION") {
        if let Ok(perm) = serde_json::from_str::<PermissionConfig>(&perm_json) {
            let target = config
                .permission
                .get_or_insert_with(PermissionConfig::default);
            for (k, v) in perm.rules {
                target.rules.insert(k, v);
            }
        }
    }

    // Backwards compatibility: legacy top-level `tools` config -> permission
    if let Some(tools) = config.tools.take() {
        let mut perms = HashMap::new();
        for (tool, enabled) in tools {
            let action = if enabled {
                PermissionAction::Allow
            } else {
                PermissionAction::Deny
            };
            // write, edit, patch, multiedit all map to "edit" permission
            if tool == "write" || tool == "edit" || tool == "patch" || tool == "multiedit" {
                perms.insert("edit".to_string(), PermissionRule::Action(action));
            } else {
                perms.insert(tool, PermissionRule::Action(action));
            }
        }
        // Legacy tools have lower priority than explicit permission config
        if let Some(existing) = &config.permission {
            for (k, v) in existing.rules.clone() {
                perms.insert(k, v);
            }
        }
        config.permission = Some(PermissionConfig { rules: perms });
    }

    // Set default username from system
    if config.username.is_none() {
        config.username = env::var("USER").or_else(|_| env::var("USERNAME")).ok();
    }

    // Handle migration from autoshare to share field
    if config.autoshare == Some(true) && config.share.is_none() {
        config.share = Some(crate::schema::ShareMode::Auto);
    }

    // Apply flag overrides for compaction settings
    if env::var("OPENCODE_DISABLE_AUTOCOMPACT").is_ok() {
        let compaction = config.compaction.get_or_insert_with(Default::default);
        compaction.auto = Some(false);
    }
    if env::var("OPENCODE_DISABLE_PRUNE").is_ok() {
        let compaction = config.compaction.get_or_insert_with(Default::default);
        compaction.prune = Some(false);
    }

    // Deduplicate plugins
    let plugins = std::mem::take(&mut config.plugin);
    config.plugin = deduplicate_plugins(plugins);
}

/// Loads config synchronously (without remote wellknown fetching).
pub fn load_config<P: AsRef<Path>>(project_dir: P) -> Result<Config> {
    let mut loader = ConfigLoader::new();
    loader.load_all(project_dir)
}

/// Loads config including remote `.well-known/opencode` endpoints.
/// Use this in async contexts where you want the full config with remote sources.
pub async fn load_config_with_remote<P: AsRef<Path>>(project_dir: P) -> Result<Config> {
    let mut loader = ConfigLoader::new();
    loader.load_all_with_remote(project_dir).await
}

/// Update project-level config by merging a patch.
pub fn update_config(project_dir: &Path, patch: &Config) -> Result<()> {
    let config_path = project_dir.join("opencode.json");

    let existing = if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        parse_jsonc(&content).unwrap_or_default()
    } else {
        Config::default()
    };

    let mut merged = existing;
    merged.merge(patch.clone());

    let json =
        serde_json::to_string_pretty(&merged).with_context(|| "Failed to serialize config")?;
    fs::write(&config_path, json)
        .with_context(|| format!("Failed to write config to {:?}", config_path))?;

    Ok(())
}

/// Update global config by merging a patch.
pub fn update_global_config(patch: &Config) -> Result<()> {
    let global_path = get_global_config_path();

    // Try to find existing global config file
    let config_path = ["jsonc", "json"]
        .iter()
        .map(|ext| global_path.with_extension(ext))
        .find(|p| p.exists())
        .unwrap_or_else(|| global_path.with_extension("json"));

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        parse_jsonc(&content).unwrap_or_default()
    } else {
        Config::default()
    };

    let mut merged = existing;
    merged.merge(patch.clone());

    let json =
        serde_json::to_string_pretty(&merged).with_context(|| "Failed to serialize config")?;
    fs::write(&config_path, json)
        .with_context(|| format!("Failed to write global config to {:?}", config_path))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let unique = format!(
                "{}_{}_{}",
                prefix,
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock error")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("failed to create test temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_parse_jsonc_simple() {
        let content = r#"{"model": "claude-3-opus"}"#;
        let config: Config = parse_jsonc(content).unwrap();
        assert_eq!(config.model, Some("claude-3-opus".to_string()));
    }

    #[test]
    fn test_parse_jsonc_with_comments() {
        let content = r#"{
            // This is a comment
            "model": "claude-3-opus",
            /* Multi-line
                comment */
            "theme": "dark"
        }"#;
        let config: Config = parse_jsonc(content).unwrap();
        assert_eq!(config.model, Some("claude-3-opus".to_string()));
        assert_eq!(config.theme, Some("dark".to_string()));
    }

    #[test]
    fn test_parse_jsonc_allows_trailing_comma_in_object() {
        let content = r#"{
            "model": "claude-3-opus",
            "theme": "dark",
        }"#;
        let config: Config = parse_jsonc(content).unwrap();
        assert_eq!(config.model, Some("claude-3-opus".to_string()));
        assert_eq!(config.theme, Some("dark".to_string()));
    }

    #[test]
    fn test_parse_jsonc_allows_trailing_comma_in_array() {
        let content = r#"{
            "instructions": ["a.md", "b.md",],
            "plugin": ["p1", "p2",],
        }"#;
        let config: Config = parse_jsonc(content).unwrap();
        assert_eq!(
            config.instructions,
            vec!["a.md".to_string(), "b.md".to_string()]
        );
        assert_eq!(config.plugin, vec!["p1".to_string(), "p2".to_string()]);
    }

    #[test]
    fn test_parse_jsonc_preserves_comment_markers_inside_strings() {
        let content = r#"{
            "provider": {
                "openai": {
                    "base_url": "https://example.com/path//not-comment",
                    "api_key": "abc/*not-comment*/def"
                }
            }
        }"#;
        let config: Config = parse_jsonc(content).unwrap();
        let provider = config.provider.unwrap();
        let openai = provider.get("openai").unwrap();
        assert_eq!(
            openai.base_url.as_deref(),
            Some("https://example.com/path//not-comment")
        );
        assert_eq!(openai.api_key.as_deref(), Some("abc/*not-comment*/def"));
    }

    #[test]
    fn test_config_merge() {
        let mut config1 = Config {
            model: Some("model1".to_string()),
            instructions: vec!["inst1".to_string()],
            ..Default::default()
        };

        let config2 = Config {
            model: Some("model2".to_string()),
            instructions: vec!["inst2".to_string()],
            ..Default::default()
        };

        config1.merge(config2);

        assert_eq!(config1.model, Some("model2".to_string()));
        assert_eq!(
            config1.instructions,
            vec!["inst1".to_string(), "inst2".to_string()]
        );
    }

    #[test]
    fn test_load_project_finds_and_merges_parent_configs() {
        let temp = TestDir::new("opencode_config_findup");
        let root = temp.path.join("repo");
        let child = root.join("apps/web");
        fs::create_dir_all(&child).unwrap();

        fs::write(
            root.join("opencode.jsonc"),
            r#"{ "model": "parent-model" }"#,
        )
        .unwrap();
        fs::write(
            root.join("apps/opencode.jsonc"),
            r#"{ "theme": "dark", "instructions": ["parent.md"] }"#,
        )
        .unwrap();
        fs::write(
            child.join("opencode.jsonc"),
            r#"{ "instructions": ["child.md"] }"#,
        )
        .unwrap();

        let mut loader = ConfigLoader::new();
        loader.load_project(&child).unwrap();
        let cfg = loader.config();

        assert_eq!(cfg.model.as_deref(), Some("parent-model"));
        assert_eq!(cfg.theme.as_deref(), Some("dark"));
        assert_eq!(
            cfg.instructions,
            vec!["parent.md".to_string(), "child.md".to_string()]
        );
    }

    #[test]
    fn test_load_project_stops_at_git_root() {
        let temp = TestDir::new("opencode_config_gitroot");
        let outer = temp.path.join("outer");
        let repo = outer.join("repo");
        let child = repo.join("sub");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();

        fs::write(
            outer.join("opencode.jsonc"),
            r#"{ "model": "outer-model" }"#,
        )
        .unwrap();
        fs::write(repo.join("opencode.jsonc"), r#"{ "model": "repo-model" }"#).unwrap();
        fs::write(
            child.join("opencode.jsonc"),
            r#"{ "theme": "child-theme" }"#,
        )
        .unwrap();

        let mut loader = ConfigLoader::new();
        loader.load_project(&child).unwrap();
        let cfg = loader.config();

        assert_eq!(cfg.model.as_deref(), Some("repo-model"));
        assert_eq!(cfg.theme.as_deref(), Some("child-theme"));
    }

    #[test]
    fn test_load_project_finds_up_dot_opencode_configs() {
        let temp = TestDir::new("opencode_config_dotdir");
        let root = temp.path.join("repo");
        let child = root.join("service");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join(".opencode")).unwrap();
        fs::create_dir_all(child.join(".opencode")).unwrap();

        fs::write(
            root.join(".opencode/opencode.jsonc"),
            r#"{ "default_agent": "build", "instructions": ["root.md"] }"#,
        )
        .unwrap();
        fs::write(
            child.join(".opencode/opencode.jsonc"),
            r#"{ "default_agent": "reviewer", "instructions": ["child.md"] }"#,
        )
        .unwrap();

        let mut loader = ConfigLoader::new();
        loader.load_project(&child).unwrap();
        let cfg = loader.config();

        assert_eq!(cfg.default_agent.as_deref(), Some("reviewer"));
        assert_eq!(
            cfg.instructions,
            vec!["root.md".to_string(), "child.md".to_string()]
        );
    }

    #[test]
    fn test_substitute_env_vars() {
        std::env::set_var("OPENCODE_TEST_VAR", "test_value");
        let input = r#"{"api_key": "{env:OPENCODE_TEST_VAR}"}"#;
        let result = substitute_env_vars(input);
        assert_eq!(result, r#"{"api_key": "test_value"}"#);
        std::env::remove_var("OPENCODE_TEST_VAR");
    }

    #[test]
    fn test_substitute_env_vars_missing() {
        let input = r#"{"api_key": "{env:NONEXISTENT_VAR_12345}"}"#;
        let result = substitute_env_vars(input);
        assert_eq!(result, r#"{"api_key": ""}"#);
    }

    #[test]
    fn test_resolve_file_references() {
        let temp = TestDir::new("opencode_file_ref");
        let secret_path = temp.path.join("secret.txt");
        fs::write(&secret_path, "my-secret-key").unwrap();

        let input = r#"{"api_key": "{file:secret.txt}"}"#.to_string();
        let result = resolve_file_references(&input, &temp.path).unwrap();
        assert_eq!(result, r#"{"api_key": "my-secret-key"}"#);
    }

    #[test]
    fn test_resolve_file_references_skips_comments() {
        let temp = TestDir::new("opencode_file_ref_comment");
        let input = r#"{
            // "api_key": "{file:secret.txt}"
            "model": "claude"
        }"#;
        let result = resolve_file_references(input, &temp.path).unwrap();
        assert!(result.contains("{file:secret.txt}"));
    }

    #[test]
    fn test_resolve_file_references_absolute_path() {
        let temp = TestDir::new("opencode_file_ref_abs");
        let secret_path = temp.path.join("abs_secret.txt");
        fs::write(&secret_path, "absolute-secret").unwrap();

        let input = format!(r#"{{"api_key": "{{file:{}}}"}}"#, secret_path.display());
        let result = resolve_file_references(&input, &temp.path).unwrap();
        assert!(result.contains("absolute-secret"));
    }

    #[test]
    fn test_update_config() {
        let temp = TestDir::new("opencode_update_config");

        let patch = Config {
            model: Some("claude-3-opus".to_string()),
            ..Default::default()
        };

        update_config(&temp.path, &patch).unwrap();

        let content = fs::read_to_string(temp.path.join("opencode.json")).unwrap();
        let config: Config = serde_json::from_str(&content).unwrap();
        assert_eq!(config.model, Some("claude-3-opus".to_string()));
    }

    // ── YAML frontmatter parsing tests ──────────────────────────────

    #[test]
    fn test_split_frontmatter_basic() {
        let content = "---\nname: test\ndescription: hello\n---\nBody content here.";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_some());
        let fm = fm.unwrap();
        assert!(fm.contains("name: test"));
        assert!(fm.contains("description: hello"));
        assert!(body.contains("Body content here."));
    }

    #[test]
    fn test_split_frontmatter_no_frontmatter() {
        let content = "Just a regular markdown file.";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_yaml_frontmatter_flat_key_values() {
        let yaml = "name: reviewer\ndescription: Review code\nmodel: claude-3-opus";
        let json = serde_yaml_frontmatter_to_json(yaml);
        assert_eq!(json["name"], "reviewer");
        assert_eq!(json["description"], "Review code");
        assert_eq!(json["model"], "claude-3-opus");
    }

    #[test]
    fn test_yaml_frontmatter_booleans_and_numbers() {
        let yaml = "disable: true\nhidden: false\nsteps: 100\ntemperature: 0.7";
        let json = serde_yaml_frontmatter_to_json(yaml);
        assert_eq!(json["disable"], true);
        assert_eq!(json["hidden"], false);
        assert_eq!(json["steps"], 100);
        assert_eq!(json["temperature"], 0.7);
    }

    #[test]
    fn test_yaml_frontmatter_inline_list() {
        let yaml = "tools: [bash, read, write]";
        let json = serde_yaml_frontmatter_to_json(yaml);
        let tools = json["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0], "bash");
        assert_eq!(tools[1], "read");
        assert_eq!(tools[2], "write");
    }

    #[test]
    fn test_yaml_frontmatter_dash_list() {
        let yaml = "tools:\n  - bash\n  - read\n  - write";
        let json = serde_yaml_frontmatter_to_json(yaml);
        let tools = json["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0], "bash");
        assert_eq!(tools[1], "read");
        assert_eq!(tools[2], "write");
    }

    #[test]
    fn test_yaml_frontmatter_nested_object() {
        let yaml = "tools:\n  bash: true\n  read: false";
        let json = serde_yaml_frontmatter_to_json(yaml);
        let tools = json["tools"].as_object().unwrap();
        assert_eq!(tools["bash"], true);
        assert_eq!(tools["read"], false);
    }

    #[test]
    fn test_yaml_frontmatter_block_scalar_literal() {
        let yaml = "prompt: |\n  Line one\n  Line two";
        let json = serde_yaml_frontmatter_to_json(yaml);
        let prompt = json["prompt"].as_str().unwrap();
        assert!(prompt.contains("Line one"));
        assert!(prompt.contains("Line two"));
    }

    #[test]
    fn test_yaml_frontmatter_block_scalar_strip() {
        let yaml = "prompt: |-\n  Line one\n  Line two";
        let json = serde_yaml_frontmatter_to_json(yaml);
        let prompt = json["prompt"].as_str().unwrap();
        assert!(prompt.contains("Line one"));
        assert!(!prompt.ends_with('\n'));
    }

    #[test]
    fn test_yaml_frontmatter_comments_skipped() {
        let yaml = "# This is a comment\nname: test\n# Another comment\ndescription: hello";
        let json = serde_yaml_frontmatter_to_json(yaml);
        assert_eq!(json["name"], "test");
        assert_eq!(json["description"], "hello");
    }

    #[test]
    fn test_yaml_frontmatter_quoted_values() {
        let yaml = "name: \"quoted value\"\ndescription: 'single quoted'";
        let json = serde_yaml_frontmatter_to_json(yaml);
        assert_eq!(json["name"], "quoted value");
        assert_eq!(json["description"], "single quoted");
    }

    #[test]
    fn test_fallback_sanitize_yaml_colon_in_value() {
        let yaml = "description: Use model: claude-3 for tasks\nname: test";
        let sanitized = fallback_sanitize_yaml(yaml);
        assert!(sanitized.contains("description: |-"));
        assert!(sanitized.contains("  Use model: claude-3 for tasks"));
        assert!(sanitized.contains("name: test"));
    }

    #[test]
    fn test_fallback_sanitize_yaml_preserves_quoted() {
        let yaml = "description: \"already: quoted\"\nname: test";
        let sanitized = fallback_sanitize_yaml(yaml);
        // Quoted values should not be converted to block scalars
        assert!(sanitized.contains("description: \"already: quoted\""));
    }

    #[test]
    fn test_fallback_sanitize_yaml_preserves_block_scalar() {
        let yaml = "description: |\n  block content\nname: test";
        let sanitized = fallback_sanitize_yaml(yaml);
        assert!(sanitized.contains("description: |"));
    }

    #[test]
    fn test_yaml_frontmatter_value_with_colon_via_fallback() {
        // This YAML has a value with a colon, which would confuse naive parsers.
        // The fallback sanitization should handle it.
        let yaml = "description: Use model: claude-3 for tasks\nname: test";
        let json = serde_yaml_frontmatter_to_json(yaml);
        // After fallback, description should be preserved
        assert_eq!(json["name"], "test");
        let desc = json["description"].as_str().unwrap();
        assert!(desc.contains("model: claude-3"));
    }

    #[test]
    fn test_yaml_frontmatter_inline_map() {
        let yaml = "options: {verbose: true, timeout: 30}";
        let json = serde_yaml_frontmatter_to_json(yaml);
        let options = json["options"].as_object().unwrap();
        assert_eq!(options["verbose"], true);
        assert_eq!(options["timeout"], 30);
    }

    #[test]
    fn test_yaml_frontmatter_empty_value() {
        let yaml = "name:\ndescription: hello";
        let json = serde_yaml_frontmatter_to_json(yaml);
        assert!(json["name"].is_null());
        assert_eq!(json["description"], "hello");
    }

    #[test]
    fn test_parse_markdown_agent_with_frontmatter() {
        let temp = TestDir::new("opencode_md_agent");
        let agent_dir = temp.path.join("agents");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("reviewer.md"),
            "---\ndescription: Reviews code changes\nmode: subagent\nmodel: claude-3-opus\n---\n\nYou are a code reviewer.\n",
        )
        .unwrap();

        let result = parse_markdown_agent(&agent_dir.join("reviewer.md"), &temp.path);
        assert!(result.is_some());
        let (name, config) = result.unwrap();
        assert_eq!(name, "reviewer");
        assert_eq!(config.description.as_deref(), Some("Reviews code changes"));
        assert_eq!(config.model.as_deref(), Some("claude-3-opus"));
        assert!(config.prompt.unwrap().contains("You are a code reviewer."));
    }

    #[test]
    fn test_parse_markdown_command_with_frontmatter() {
        let temp = TestDir::new("opencode_md_cmd");
        let cmd_dir = temp.path.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        fs::write(
            cmd_dir.join("review.md"),
            "---\ndescription: Run a code review\nagent: reviewer\n---\n\nPlease review the changes.\n",
        )
        .unwrap();

        let result = parse_markdown_command(&cmd_dir.join("review.md"), &temp.path);
        assert!(result.is_some());
        let (name, config) = result.unwrap();
        assert_eq!(name, "review");
        assert_eq!(config.description.as_deref(), Some("Run a code review"));
        assert_eq!(config.agent.as_deref(), Some("reviewer"));
        assert!(config
            .template
            .unwrap()
            .contains("Please review the changes."));
    }

    #[test]
    fn test_parse_markdown_agent_with_tools_map() {
        let temp = TestDir::new("opencode_md_agent_tools");
        let agent_dir = temp.path.join("agents");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("safe.md"),
            "---\ndescription: Safe agent\ntools:\n  bash: false\n  read: true\n---\n\nSafe prompt.\n",
        )
        .unwrap();

        let result = parse_markdown_agent(&agent_dir.join("safe.md"), &temp.path);
        assert!(result.is_some());
        let (_name, config) = result.unwrap();
        assert_eq!(config.description.as_deref(), Some("Safe agent"));
        let tools = config.tools.unwrap();
        assert_eq!(tools.get("bash"), Some(&false));
        assert_eq!(tools.get("read"), Some(&true));
    }

    #[test]
    fn test_parse_markdown_agent_colon_in_description_fallback() {
        let temp = TestDir::new("opencode_md_agent_colon");
        let agent_dir = temp.path.join("agents");
        fs::create_dir_all(&agent_dir).unwrap();
        // Description contains a colon -- this is the case the fallback handles
        fs::write(
            agent_dir.join("tricky.md"),
            "---\ndescription: Use model: claude for tasks\nmode: primary\n---\n\nTricky prompt.\n",
        )
        .unwrap();

        let result = parse_markdown_agent(&agent_dir.join("tricky.md"), &temp.path);
        assert!(result.is_some());
        let (_name, config) = result.unwrap();
        let desc = config.description.unwrap();
        assert!(desc.contains("model: claude"));
    }

    #[test]
    fn legacy_toml_config_migrates_to_opencode_json() {
        let temp = TestDir::new("opencode_legacy_toml");
        let config_dir = temp.path.join("opencode");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config"),
            r#"
provider = "anthropic"
model = "claude-3-5-sonnet"
theme = "dark"
"#,
        )
        .unwrap();

        let mut config = Config::default();
        let migrated = migrate_legacy_toml_config(&config_dir, &mut config);
        assert!(migrated.is_some());
        assert_eq!(config.model.as_deref(), Some("anthropic/claude-3-5-sonnet"));
        assert_eq!(config.theme.as_deref(), Some("dark"));
        assert_eq!(
            config.schema.as_deref(),
            Some("https://opencode.ai/config.json")
        );

        let json_path = config_dir.join("opencode.json");
        assert!(json_path.exists());
        assert!(!config_dir.join("config").exists());

        let content = fs::read_to_string(json_path).unwrap();
        let written: Config = serde_json::from_str(&content).unwrap();
        assert_eq!(
            written.model.as_deref(),
            Some("anthropic/claude-3-5-sonnet")
        );
        assert_eq!(written.theme.as_deref(), Some("dark"));
    }

    /// `dirs::config_dir()` is the idiomatic choice and is WRONG here: on macOS it
    /// resolves to `~/Library/Application Support`, where opencode keeps only its
    /// database. Using it hid the entire global config (agents, MCP, providers, model).
    #[test]
    fn global_config_path_resolves_to_dot_config_not_platform_config_dir() {
        let home = tempfile::tempdir().unwrap();
        let dot_config = home.path().join(".config");
        let real = dot_config.join("opencode");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("opencode.json"), "{}").unwrap();

        let platform = home.path().join("Library/Application Support/opencode");
        std::fs::create_dir_all(&platform).unwrap();
        std::fs::write(platform.join("opencode.json"), "{}").unwrap();

        let resolved = resolve_global_config_path(Some(home.path().to_path_buf()), None, None);
        assert_eq!(
            resolved.with_extension("json"),
            real.join("opencode.json"),
            "global config must resolve to ~/.config/opencode, not the platform config dir"
        );
    }

    /// Platform locations must keep working for installs without `~/.config`.
    #[test]
    fn global_config_path_falls_back_to_platform_config_dir() {
        let home = tempfile::tempdir().unwrap();
        let platform = home.path().join("platform");
        std::fs::create_dir_all(platform.join("opencode")).unwrap();
        std::fs::write(platform.join("opencode/opencode.json"), "{}").unwrap();

        let resolved =
            resolve_global_config_path(Some(home.path().to_path_buf()), Some(platform.clone()), None);
        assert_eq!(
            resolved.with_extension("json"),
            platform.join("opencode/opencode.json")
        );
    }

    #[test]
    fn global_config_path_honours_xdg_config_home() {
        let home = tempfile::tempdir().unwrap();
        let xdg = home.path().join("xdg");
        std::fs::create_dir_all(xdg.join("opencode")).unwrap();
        std::fs::write(xdg.join("opencode/opencode.json"), "{}").unwrap();

        let resolved =
            resolve_global_config_path(Some(home.path().to_path_buf()), None, Some(xdg.clone()));
        assert_eq!(resolved.with_extension("json"), xdg.join("opencode/opencode.json"));
    }
}
