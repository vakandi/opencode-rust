use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::command::{fuzzy_match, CommandAction};
use crate::context::MessageDensity;
use crate::theme::Theme;

#[derive(Clone, Debug)]
pub struct Command {
    pub action: CommandAction,
    pub title: String,
    pub keybind: Option<String>,
    pub category: String,
}

pub struct CommandPalette {
    commands: Vec<Command>,
    filtered: Vec<usize>,
    query: String,
    state: ListState,
    open: bool,
}

impl CommandPalette {
    pub fn new() -> Self {
        let commands = vec![
            Command {
                action: CommandAction::SubmitPrompt,
                title: "Submit prompt".to_string(),
                keybind: Some("enter".to_string()),
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::ClearPrompt,
                title: "Clear prompt".to_string(),
                keybind: Some("ctrl+u".to_string()),
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::PasteClipboard,
                title: "Paste from clipboard".to_string(),
                keybind: Some("ctrl+v".to_string()),
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::CopyPrompt,
                title: "Copy prompt to clipboard".to_string(),
                keybind: Some("ctrl+shift+c".to_string()),
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::CutPrompt,
                title: "Cut prompt to clipboard".to_string(),
                keybind: Some("ctrl+shift+x".to_string()),
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::HistoryPrevious,
                title: "Prompt history previous".to_string(),
                keybind: Some("alt+up".to_string()),
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::HistoryNext,
                title: "Prompt history next".to_string(),
                keybind: Some("alt+down".to_string()),
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::PromptStashPush,
                title: "Stash prompt".to_string(),
                keybind: None,
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::PromptStashList,
                title: "Open prompt stash".to_string(),
                keybind: None,
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::PromptSkillList,
                title: "Insert skill command".to_string(),
                keybind: None,
                category: "Prompt".to_string(),
            },
            Command {
                action: CommandAction::ToggleSidebar,
                title: "Toggle sidebar".to_string(),
                keybind: Some("ctrl+s".to_string()),
                category: "View".to_string(),
            },
            Command {
                action: CommandAction::ToggleHeader,
                title: "Hide header".to_string(),
                keybind: None,
                category: "View".to_string(),
            },
            Command {
                action: CommandAction::ToggleScrollbar,
                title: "Show scrollbar".to_string(),
                keybind: None,
                category: "View".to_string(),
            },
            Command {
                action: CommandAction::ToggleTips,
                title: "Hide tips".to_string(),
                keybind: None,
                category: "View".to_string(),
            },
            Command {
                action: CommandAction::ToggleThinking,
                title: "Toggle thinking".to_string(),
                keybind: None,
                category: "View".to_string(),
            },
            Command {
                action: CommandAction::ToggleToolDetails,
                title: "Toggle tool details".to_string(),
                keybind: None,
                category: "View".to_string(),
            },
            Command {
                action: CommandAction::ToggleDensity,
                title: "Switch to cozy density".to_string(),
                keybind: None,
                category: "View".to_string(),
            },
            Command {
                action: CommandAction::ToggleSemanticHighlight,
                title: "Disable semantic highlight".to_string(),
                keybind: None,
                category: "View".to_string(),
            },
            Command {
                action: CommandAction::SwitchSession,
                title: "Switch session".to_string(),
                keybind: None,
                category: "Session".to_string(),
            },
            Command {
                action: CommandAction::RenameSession,
                title: "Rename current session".to_string(),
                keybind: None,
                category: "Session".to_string(),
            },
            Command {
                action: CommandAction::ExportSession,
                title: "Export current session".to_string(),
                keybind: None,
                category: "Session".to_string(),
            },
            Command {
                action: CommandAction::SwitchModel,
                title: "Switch model".to_string(),
                keybind: Some("ctrl+m".to_string()),
                category: "Session".to_string(),
            },
            Command {
                action: CommandAction::SwitchAgent,
                title: "Open agent list".to_string(),
                keybind: None,
                category: "Session".to_string(),
            },
            Command {
                action: CommandAction::CycleVariant,
                title: "Cycle model variant".to_string(),
                keybind: Some("ctrl+v".to_string()),
                category: "Session".to_string(),
            },
            Command {
                action: CommandAction::NewSession,
                title: "New session".to_string(),
                keybind: Some("ctrl+n".to_string()),
                category: "Session".to_string(),
            },
            Command {
                action: CommandAction::SwitchTheme,
                title: "Switch theme".to_string(),
                keybind: None,
                category: "System".to_string(),
            },
            Command {
                action: CommandAction::ToggleAppearance,
                title: "Toggle appearance".to_string(),
                keybind: None,
                category: "System".to_string(),
            },
            Command {
                action: CommandAction::ViewStatus,
                title: "View status".to_string(),
                keybind: None,
                category: "System".to_string(),
            },
            Command {
                action: CommandAction::ToggleMcp,
                title: "Manage MCP servers".to_string(),
                keybind: None,
                category: "System".to_string(),
            },
            Command {
                action: CommandAction::ShowHelp,
                title: "Show help".to_string(),
                keybind: Some("ctrl+h".to_string()),
                category: "Help".to_string(),
            },
            Command {
                action: CommandAction::Exit,
                title: "Exit".to_string(),
                keybind: Some("ctrl+c".to_string()),
                category: "App".to_string(),
            },
        ];

        let filtered = (0..commands.len()).collect();
        let mut state = ListState::default();
        state.select(Some(0));

        Self {
            commands,
            filtered,
            query: String::new(),
            state,
            open: false,
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.filtered = (0..self.commands.len()).collect();
        self.state.select(Some(0));
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn handle_input(&mut self, c: char) {
        self.query.push(c);
        self.filter_commands();
    }

    pub fn handle_backspace(&mut self) {
        self.query.pop();
        self.filter_commands();
    }

    pub fn move_up(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected > 0 {
                self.state.select(Some(selected - 1));
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected < self.filtered.len().saturating_sub(1) {
                self.state.select(Some(selected + 1));
            }
        }
    }

    pub fn selected_command(&self) -> Option<&Command> {
        self.state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .and_then(|&idx| self.commands.get(idx))
    }

    pub fn selected_action(&self) -> Option<CommandAction> {
        self.selected_command().map(|cmd| cmd.action.clone())
    }

    pub fn sync_visibility_labels(
        &mut self,
        show_thinking: bool,
        show_tool_details: bool,
        density: MessageDensity,
        semantic_highlight: bool,
        show_header: bool,
        show_scrollbar: bool,
        tips_hidden: bool,
    ) {
        for command in &mut self.commands {
            if matches!(&command.action, CommandAction::ToggleHeader) {
                command.title = if show_header {
                    "Hide header".to_string()
                } else {
                    "Show header".to_string()
                };
            }
            if matches!(&command.action, CommandAction::ToggleScrollbar) {
                command.title = if show_scrollbar {
                    "Hide scrollbar".to_string()
                } else {
                    "Show scrollbar".to_string()
                };
            }
            if matches!(&command.action, CommandAction::ToggleTips) {
                command.title = if tips_hidden {
                    "Show tips".to_string()
                } else {
                    "Hide tips".to_string()
                };
            }
            if matches!(&command.action, CommandAction::ToggleThinking) {
                command.title = if show_thinking {
                    "Hide thinking".to_string()
                } else {
                    "Show thinking".to_string()
                };
            }
            if matches!(&command.action, CommandAction::ToggleToolDetails) {
                command.title = if show_tool_details {
                    "Hide tool details".to_string()
                } else {
                    "Show tool details".to_string()
                };
            }
            if matches!(&command.action, CommandAction::ToggleDensity) {
                command.title = match density {
                    MessageDensity::Compact => "Switch to cozy density".to_string(),
                    MessageDensity::Cozy => "Switch to compact density".to_string(),
                };
            }
            if matches!(&command.action, CommandAction::ToggleSemanticHighlight) {
                command.title = if semantic_highlight {
                    "Disable semantic highlight".to_string()
                } else {
                    "Enable semantic highlight".to_string()
                };
            }
        }

        self.filter_commands();
    }

    fn filter_commands(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.commands.len()).collect();
        } else {
            let mut scored: Vec<(usize, i32)> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(i, cmd)| {
                    let title_score = fuzzy_match(&self.query, &cmd.title);
                    let cat_score = fuzzy_match(&self.query, &cmd.category);
                    let best = title_score.into_iter().chain(cat_score).max();
                    best.map(|s| (i, s))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }

        if self.filtered.is_empty() {
            self.state.select(None);
        } else {
            self.state.select(Some(0));
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_width = 60;
        let dialog_height = (self.filtered.len() + 4).min(20) as u16;

        let dialog_area = centered_rect(dialog_width, dialog_height, area);

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Commands ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));

        let inner_area = super::dialog_inner(block.inner(dialog_area));
        frame.render_widget(block, dialog_area);

        let search_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(theme.primary)),
            Span::styled(&self.query, Style::default().fg(theme.text)),
            Span::styled("▏", Style::default().fg(theme.primary)),
        ]);

        let search_paragraph = Paragraph::new(search_line);
        frame.render_widget(
            search_paragraph,
            Rect {
                x: inner_area.x,
                y: inner_area.y,
                width: inner_area.width,
                height: 1,
            },
        );

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .filter_map(|&idx| {
                self.commands.get(idx).map(|cmd| {
                    let style = if Some(idx)
                        == self
                            .state
                            .selected()
                            .and_then(|s| self.filtered.get(s))
                            .copied()
                    {
                        Style::default()
                            .fg(theme.text)
                            .bg(theme.background_element)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    };

                    let mut spans = vec![Span::styled(&cmd.title, style)];

                    if let Some(ref keybind) = cmd.keybind {
                        spans.push(Span::raw("  "));
                        spans.push(Span::styled(
                            keybind.clone(),
                            Style::default().fg(theme.text_muted),
                        ));
                    }

                    ListItem::new(Line::from(spans))
                })
            })
            .collect();

        let list = List::new(items).style(Style::default().fg(theme.text));

        let list_area = Rect {
            x: inner_area.x,
            y: inner_area.y + 2,
            width: inner_area.width,
            height: inner_area.height.saturating_sub(2),
        };

        frame.render_stateful_widget(list, list_area, &mut self.state.clone());
    }
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

