use std::collections::HashMap;
use std::hash::Hash;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Some(0);
    }

    let pattern = Pattern::parse(trimmed, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut utf32_buf = Vec::new();
    pattern
        .score(Utf32Str::new(target, &mut utf32_buf), &mut matcher)
        .map(|score| score.min(i32::MAX as u32) as i32)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CommandCategory {
    Session,
    ModelAgent,
    Display,
    Navigation,
    System,
    Prompt,
}

#[derive(Clone, Debug)]
pub struct SlashCommand {
    pub name: String,
    pub aliases: Vec<String>,
    pub title: String,
    pub description: String,
    pub category: CommandCategory,
    pub keybind: Option<String>,
    pub suggested: bool,
    pub action: CommandAction,
}

#[derive(Clone, Debug)]
pub enum CommandAction {
    // Session
    NewSession,
    ListSessions,
    ShareSession,
    UnshareSession,
    RenameSession,
    ForkSession,
    CompactSession,
    Timeline,
    Undo,
    Redo,
    CopySession,
    ExportSession,
    // Model/Agent
    SwitchModel,
    SwitchAgent,
    ManageMcp,
    ConnectProvider,
    CycleVariant,
    // Display
    SwitchTheme,
    ToggleAppearance,
    ShowStatus,
    ViewStatus,
    ShowHelp,
    ExternalEditor,
    ToggleTimestamps,
    ToggleThinking,
    ToggleToolDetails,
    ToggleDensity,
    ToggleSemanticHighlight,
    ToggleHeader,
    ToggleScrollbar,
    ToggleSidebar,
    ToggleMcp,
    ToggleTips,
    ToggleCommandPalette,
    // Navigation
    OpenSessionList,
    SwitchSession,
    OpenModelList,
    OpenAgentList,
    OpenMcpList,
    OpenThemeList,
    OpenStash,
    OpenSkills,
    // Prompt
    SubmitPrompt,
    ClearPrompt,
    PasteClipboard,
    CopyPrompt,
    CutPrompt,
    HistoryPrevious,
    HistoryNext,
    PromptStashPush,
    PromptStashList,
    PromptSkillList,
    // System
    Exit,
}

pub struct CommandRegistry {
    commands: HashMap<String, SlashCommand>,
    by_category: HashMap<CommandCategory, Vec<String>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            commands: HashMap::new(),
            by_category: HashMap::new(),
        };
        registry.register_all();
        registry
    }

    fn register(&mut self, cmd: SlashCommand) {
        let name = cmd.name.clone();
        self.commands.insert(name.clone(), cmd.clone());

        self.by_category
            .entry(cmd.category.clone())
            .or_insert_with(Vec::new)
            .push(name);

        for alias in &cmd.aliases {
            self.commands.insert(alias.clone(), cmd.clone());
        }
    }

    fn register_all(&mut self) {
        self.register(SlashCommand {
            name: "/new".to_string(),
            aliases: vec!["/clear".to_string()],
            title: "New Session".to_string(),
            description: "Start a new conversation".to_string(),
            category: CommandCategory::Session,
            keybind: Some("ctrl_n".to_string()),
            suggested: true,
            action: CommandAction::NewSession,
        });

        self.register(SlashCommand {
            name: "/sessions".to_string(),
            aliases: vec!["/resume".to_string(), "/continue".to_string()],
            title: "Switch Session".to_string(),
            description: "Switch to another session".to_string(),
            category: CommandCategory::Session,
            keybind: Some("ctrl_s".to_string()),
            suggested: true,
            action: CommandAction::OpenSessionList,
        });

        self.register(SlashCommand {
            name: "/share".to_string(),
            aliases: vec![],
            title: "Share Session".to_string(),
            description: "Generate a shareable link".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::ShareSession,
        });

        self.register(SlashCommand {
            name: "/unshare".to_string(),
            aliases: vec![],
            title: "Unshare Session".to_string(),
            description: "Revoke sharing link".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::UnshareSession,
        });

        self.register(SlashCommand {
            name: "/rename".to_string(),
            aliases: vec![],
            title: "Rename Session".to_string(),
            description: "Rename current session".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::RenameSession,
        });

        self.register(SlashCommand {
            name: "/fork".to_string(),
            aliases: vec![],
            title: "Fork Session".to_string(),
            description: "Create a fork from a message".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::ForkSession,
        });

        self.register(SlashCommand {
            name: "/compact".to_string(),
            aliases: vec!["/summarize".to_string()],
            title: "Compact Session".to_string(),
            description: "Summarize and compress session history".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::CompactSession,
        });

        self.register(SlashCommand {
            name: "/timeline".to_string(),
            aliases: vec![],
            title: "Timeline".to_string(),
            description: "Navigate to a message in timeline".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::Timeline,
        });

        self.register(SlashCommand {
            name: "/undo".to_string(),
            aliases: vec![],
            title: "Undo".to_string(),
            description: "Revert last message".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::Undo,
        });

        self.register(SlashCommand {
            name: "/redo".to_string(),
            aliases: vec![],
            title: "Redo".to_string(),
            description: "Restore reverted message".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::Redo,
        });

        self.register(SlashCommand {
            name: "/copy".to_string(),
            aliases: vec![],
            title: "Copy Session".to_string(),
            description: "Copy session to clipboard".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::CopySession,
        });

        self.register(SlashCommand {
            name: "/export".to_string(),
            aliases: vec![],
            title: "Export Session".to_string(),
            description: "Export session as markdown".to_string(),
            category: CommandCategory::Session,
            keybind: None,
            suggested: false,
            action: CommandAction::ExportSession,
        });

        self.register(SlashCommand {
            name: "/models".to_string(),
            aliases: vec![],
            title: "Switch Model".to_string(),
            description: "Choose a different model".to_string(),
            category: CommandCategory::ModelAgent,
            keybind: Some("ctrl_m".to_string()),
            suggested: true,
            action: CommandAction::OpenModelList,
        });

        self.register(SlashCommand {
            name: "/agents".to_string(),
            aliases: vec![],
            title: "Switch Agent".to_string(),
            description: "Choose a different agent".to_string(),
            category: CommandCategory::ModelAgent,
            keybind: None,
            suggested: false,
            action: CommandAction::OpenAgentList,
        });

        self.register(SlashCommand {
            name: "/mcps".to_string(),
            aliases: vec![],
            title: "Manage MCP".to_string(),
            description: "Manage MCP servers".to_string(),
            category: CommandCategory::ModelAgent,
            keybind: None,
            suggested: false,
            action: CommandAction::OpenMcpList,
        });

        self.register(SlashCommand {
            name: "/connect".to_string(),
            aliases: vec![],
            title: "Connect Provider".to_string(),
            description: "Connect to a new LLM provider".to_string(),
            category: CommandCategory::ModelAgent,
            keybind: None,
            suggested: false,
            action: CommandAction::ConnectProvider,
        });

        self.register(SlashCommand {
            name: "/themes".to_string(),
            aliases: vec![],
            title: "Switch Theme".to_string(),
            description: "Choose a color theme".to_string(),
            category: CommandCategory::Display,
            keybind: Some("ctrl_t".to_string()),
            suggested: true,
            action: CommandAction::OpenThemeList,
        });

        self.register(SlashCommand {
            name: "/status".to_string(),
            aliases: vec![],
            title: "Status".to_string(),
            description: "Show system status".to_string(),
            category: CommandCategory::System,
            keybind: None,
            suggested: false,
            action: CommandAction::ShowStatus,
        });

        self.register(SlashCommand {
            name: "/help".to_string(),
            aliases: vec!["/commands".to_string()],
            title: "Help".to_string(),
            description: "Show help and shortcuts".to_string(),
            category: CommandCategory::System,
            keybind: Some("f1".to_string()),
            suggested: true,
            action: CommandAction::ShowHelp,
        });

        self.register(SlashCommand {
            name: "/editor".to_string(),
            aliases: vec![],
            title: "External Editor".to_string(),
            description: "Open in external editor".to_string(),
            category: CommandCategory::System,
            keybind: None,
            suggested: false,
            action: CommandAction::ExternalEditor,
        });

        self.register(SlashCommand {
            name: "/exit".to_string(),
            aliases: vec!["/quit".to_string(), "/q".to_string()],
            title: "Exit".to_string(),
            description: "Exit the application".to_string(),
            category: CommandCategory::System,
            keybind: Some("ctrl_c".to_string()),
            suggested: true,
            action: CommandAction::Exit,
        });

        self.register(SlashCommand {
            name: "/timestamps".to_string(),
            aliases: vec!["/toggle-timestamps".to_string()],
            title: "Toggle Timestamps".to_string(),
            description: "Show/hide message timestamps".to_string(),
            category: CommandCategory::Display,
            keybind: None,
            suggested: false,
            action: CommandAction::ToggleTimestamps,
        });

        self.register(SlashCommand {
            name: "/tips.toggle".to_string(),
            aliases: vec!["/tips".to_string()],
            title: "Toggle Tips".to_string(),
            description: "Show/hide home page tips".to_string(),
            category: CommandCategory::Display,
            keybind: None,
            suggested: false,
            action: CommandAction::ToggleTips,
        });

        self.register(SlashCommand {
            name: "/thinking".to_string(),
            aliases: vec!["/toggle-thinking".to_string()],
            title: "Toggle Thinking".to_string(),
            description: "Show/hide thinking blocks".to_string(),
            category: CommandCategory::Display,
            keybind: None,
            suggested: false,
            action: CommandAction::ToggleThinking,
        });

        self.register(SlashCommand {
            name: "/density".to_string(),
            aliases: vec!["/toggle-density".to_string()],
            title: "Toggle Density".to_string(),
            description: "Switch between compact and cozy message layout".to_string(),
            category: CommandCategory::Display,
            keybind: None,
            suggested: false,
            action: CommandAction::ToggleDensity,
        });

        self.register(SlashCommand {
            name: "/highlight".to_string(),
            aliases: vec!["/toggle-highlight".to_string(), "/semantic".to_string()],
            title: "Toggle Semantic Highlight".to_string(),
            description: "Enable/disable semantic highlighting of paths, errors, commands"
                .to_string(),
            category: CommandCategory::Display,
            keybind: None,
            suggested: false,
            action: CommandAction::ToggleSemanticHighlight,
        });

        self.register(SlashCommand {
            name: "/sidebar".to_string(),
            aliases: vec![],
            title: "Toggle Sidebar".to_string(),
            description: "Show/hide sidebar".to_string(),
            category: CommandCategory::Display,
            keybind: Some("ctrl_b".to_string()),
            suggested: false,
            action: CommandAction::ToggleSidebar,
        });

        self.register(SlashCommand {
            name: "/header".to_string(),
            aliases: vec![],
            title: "Toggle Header".to_string(),
            description: "Show/hide session header".to_string(),
            category: CommandCategory::Display,
            keybind: None,
            suggested: false,
            action: CommandAction::ToggleHeader,
        });

        self.register(SlashCommand {
            name: "/scrollbar".to_string(),
            aliases: vec![],
            title: "Toggle Scrollbar".to_string(),
            description: "Show/hide session scrollbar".to_string(),
            category: CommandCategory::Display,
            keybind: None,
            suggested: false,
            action: CommandAction::ToggleScrollbar,
        });

        self.register(SlashCommand {
            name: "/command".to_string(),
            aliases: vec!["/cmd".to_string(), "/palette".to_string()],
            title: "Command Palette".to_string(),
            description: "Open command palette".to_string(),
            category: CommandCategory::Navigation,
            keybind: Some("ctrl_p".to_string()),
            suggested: true,
            action: CommandAction::ToggleCommandPalette,
        });

        self.register(SlashCommand {
            name: "/stash".to_string(),
            aliases: vec![],
            title: "Prompt Stash".to_string(),
            description: "Save current prompt to stash".to_string(),
            category: CommandCategory::Prompt,
            keybind: None,
            suggested: false,
            action: CommandAction::OpenStash,
        });

        self.register(SlashCommand {
            name: "/skills".to_string(),
            aliases: vec![],
            title: "Skills".to_string(),
            description: "Browse available skills".to_string(),
            category: CommandCategory::Navigation,
            keybind: None,
            suggested: false,
            action: CommandAction::OpenSkills,
        });
    }

    pub fn get(&self, name: &str) -> Option<&SlashCommand> {
        self.commands.get(name)
    }

    pub fn search(&self, query: &str) -> Vec<&SlashCommand> {
        let mut scored: Vec<(&SlashCommand, i32)> = self
            .commands
            .values()
            .filter_map(|cmd| {
                let name_score = fuzzy_match(query, &cmd.name);
                let alias_score = cmd
                    .aliases
                    .iter()
                    .filter_map(|a| fuzzy_match(query, a))
                    .max();
                let title_score = fuzzy_match(query, &cmd.title);
                let best = name_score
                    .into_iter()
                    .chain(alias_score)
                    .chain(title_score)
                    .max();
                best.map(|s| (cmd, s))
            })
            .collect();

        scored.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| b.0.suggested.cmp(&a.0.suggested))
        });

        scored.into_iter().map(|(cmd, _)| cmd).collect()
    }

    pub fn get_by_category(&self, category: &CommandCategory) -> Vec<&SlashCommand> {
        self.by_category
            .get(category)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| self.commands.get(name))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn all_commands(&self) -> Vec<&SlashCommand> {
        let mut seen = std::collections::HashSet::new();
        self.commands
            .values()
            .filter(|cmd| seen.insert(cmd.name.clone()))
            .collect()
    }

    pub fn suggested_commands(&self) -> Vec<&SlashCommand> {
        self.all_commands()
            .into_iter()
            .filter(|cmd| cmd.suggested)
            .collect()
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}
