use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug)]
pub struct Agent {
    pub name: String,
    pub description: String,
    pub color: ratatui::style::Color,
}

pub struct AgentSelectDialog {
    agents: Vec<Agent>,
    state: ListState,
    open: bool,
}

impl AgentSelectDialog {
    pub fn new() -> Self {
        let agents = vec![
            Agent {
                name: "build".into(),
                description: "Code generation and modification".into(),
                color: ratatui::style::Color::Cyan,
            },
            Agent {
                name: "oracle".into(),
                description: "Read-only consultation".into(),
                color: ratatui::style::Color::Magenta,
            },
            Agent {
                name: "metis".into(),
                description: "Pre-planning analysis".into(),
                color: ratatui::style::Color::Yellow,
            },
            Agent {
                name: "momus".into(),
                description: "Expert reviewer".into(),
                color: ratatui::style::Color::Green,
            },
            Agent {
                name: "explore".into(),
                description: "Codebase exploration".into(),
                color: ratatui::style::Color::Blue,
            },
            Agent {
                name: "librarian".into(),
                description: "Documentation lookup".into(),
                color: ratatui::style::Color::Rgb(180, 100, 255),
            },
        ];

        let mut state = ListState::default();
        state.select(Some(0));

        Self {
            agents,
            state,
            open: false,
        }
    }

    pub fn set_agents(&mut self, agents: Vec<Agent>) {
        self.agents = agents;
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    pub fn open(&mut self) {
        self.open = true;
        self.state.select(Some(0));
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
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
            if selected < self.agents.len().saturating_sub(1) {
                self.state.select(Some(selected + 1));
            }
        }
    }

    pub fn selected_agent(&self) -> Option<&Agent> {
        self.state.selected().and_then(|i| self.agents.get(i))
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_width = 45;
        let dialog_height = (self.agents.len() + 2).min(12) as u16;
        let dialog_area = centered_rect(dialog_width, dialog_height, area);

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Select Agent ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));

        let inner_area = super::dialog_inner(block.inner(dialog_area));
        frame.render_widget(block, dialog_area);

        let items: Vec<ListItem> = self
            .agents
            .iter()
            .enumerate()
            .map(|(i, agent)| {
                let is_selected = self.state.selected() == Some(i);
                let style = if is_selected {
                    Style::default().fg(theme.text).bg(theme.background_element)
                } else {
                    Style::default().fg(theme.text)
                };

                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(agent.color)),
                    Span::styled(&agent.name, style.add_modifier(Modifier::BOLD)),
                    Span::styled("  ", style),
                    Span::styled(&agent.description, Style::default().fg(theme.text_muted)),
                ]))
            })
            .collect();

        let list = List::new(items);
        frame.render_stateful_widget(list, inner_area, &mut self.state.clone());
    }
}

impl Default for AgentSelectDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

