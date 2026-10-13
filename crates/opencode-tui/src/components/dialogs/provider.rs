use ratatui::prelude::Stylize;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub status: ProviderStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderStatus {
    Connected,
    Disconnected,
    Error,
}

pub struct ProviderDialog {
    pub providers: Vec<Provider>,
    pub state: ListState,
    pub open: bool,
    pub selected_provider: Option<Provider>,
    pub api_key_input: String,
    pub input_mode: bool,
}

impl ProviderDialog {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            state: ListState::default(),
            open: false,
            selected_provider: None,
            api_key_input: String::new(),
            input_mode: false,
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.state.select(Some(0));
    }

    pub fn close(&mut self) {
        self.open = false;
        self.input_mode = false;
        self.api_key_input.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_providers(&mut self, providers: Vec<Provider>) {
        self.providers = providers;
    }

    pub fn move_up(&mut self) {
        if let Some(selected) = self.state.selected() {
            let new = selected.saturating_sub(1);
            self.state.select(Some(new));
        }
    }

    pub fn move_down(&mut self) {
        if let Some(selected) = self.state.selected() {
            let new = (selected + 1).min(self.providers.len().saturating_sub(1));
            self.state.select(Some(new));
        }
    }

    pub fn selected_provider(&self) -> Option<&Provider> {
        self.state.selected().and_then(|i| self.providers.get(i))
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let height = 15u16;
        let width = 50u16;
        let popup_area = super::centered_rect(width, height, area);
        let block = Block::default()
            .title(" Connect Provider ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border));
        let content_area = super::dialog_inner(block.inner(popup_area));

        if self.input_mode {
            let content = vec![
                Line::from(Span::styled(
                    "Connect Provider",
                    Style::default().fg(theme.primary).bold(),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Provider: ", Style::default().fg(theme.text_muted)),
                    Span::styled(
                        self.selected_provider()
                            .map(|p| p.name.as_str())
                            .unwrap_or(""),
                        Style::default().fg(theme.text),
                    ),
                ]),
                Line::from(""),
                Line::from("Enter API Key:"),
                Line::from(Span::styled(
                    format!("> {}", self.api_key_input),
                    Style::default().fg(theme.primary),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[Enter] Connect  ", Style::default().fg(theme.success)),
                    Span::styled("[Esc] Cancel", Style::default().fg(theme.text_muted)),
                ]),
            ];

            frame.render_widget(block.clone().style(Style::default().bg(theme.background_panel)), popup_area);
            let paragraph = Paragraph::new(content).style(Style::default().bg(theme.background_panel));
            frame.render_widget(paragraph, content_area);
        } else {
            let items: Vec<ListItem> = self
                .providers
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let status_icon = match p.status {
                        ProviderStatus::Connected => "●",
                        ProviderStatus::Disconnected => "◯",
                        ProviderStatus::Error => "✗",
                    };
                    let status_color = match p.status {
                        ProviderStatus::Connected => theme.success,
                        ProviderStatus::Disconnected => theme.text_muted,
                        ProviderStatus::Error => theme.error,
                    };
                    let is_selected = self.state.selected() == Some(i);
                    let style = if is_selected {
                        Style::default()
                            .fg(theme.primary)
                            .bg(theme.background_element)
                    } else {
                        Style::default().fg(theme.text)
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(status_icon, Style::default().fg(status_color)),
                        Span::raw(" "),
                        Span::styled(&p.name, style),
                    ]))
                })
                .collect();

            frame.render_widget(block.style(Style::default().bg(theme.background_panel)), popup_area);
            let list = List::new(items).highlight_style(Style::default().fg(theme.primary));

            frame.render_widget(list, content_area);
        }
    }
}

impl Default for ProviderDialog {
    fn default() -> Self {
        Self::new()
    }
}
