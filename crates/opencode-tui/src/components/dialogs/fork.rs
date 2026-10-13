use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug)]
pub struct ForkEntry {
    pub message_id: String,
    pub role: String,
    pub preview: String,
    pub timestamp: String,
}

pub struct ForkDialog {
    entries: Vec<ForkEntry>,
    state: ListState,
    session_id: Option<String>,
    open: bool,
}

impl ForkDialog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            state: ListState::default(),
            session_id: None,
            open: false,
        }
    }

    pub fn open(&mut self, session_id: String, entries: Vec<ForkEntry>) {
        self.session_id = Some(session_id);
        self.entries = entries;
        self.state.select(if self.entries.is_empty() {
            None
        } else {
            Some(0)
        });
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn move_up(&mut self) {
        if let Some(i) = self.state.selected() {
            if i > 0 {
                self.state.select(Some(i - 1));
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(i) = self.state.selected() {
            if i < self.entries.len().saturating_sub(1) {
                self.state.select(Some(i + 1));
            }
        }
    }

    pub fn selected_entry(&self) -> Option<&ForkEntry> {
        self.state.selected().and_then(|i| self.entries.get(i))
    }

    pub fn selected_message_id(&self) -> Option<&str> {
        self.selected_entry().map(|e| e.message_id.as_str())
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }
        let dialog_width = 70u16.min(area.width.saturating_sub(4));
        let dialog_height = 20u16.min(area.height.saturating_sub(4));
        let dialog_area = centered_rect(dialog_width, dialog_height, area);
        frame.render_widget(Clear, dialog_area);
        let block = Block::default()
            .title(Span::styled(
                " Fork Session ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let inner = super::dialog_inner(block.inner(dialog_area));
        frame.render_widget(block, dialog_area);
        if self.entries.is_empty() {
            let empty = ratatui::widgets::Paragraph::new("No messages to fork from")
                .style(Style::default().fg(theme.text_muted));
            frame.render_widget(empty, inner);
            return;
        }
        let hint = ratatui::widgets::Paragraph::new(
            "Select a message to fork from. Enter to confirm, Esc to cancel.",
        )
        .style(Style::default().fg(theme.text_muted));
        frame.render_widget(
            hint,
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );
        let list_area = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        };
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| {
                let role_icon = match entry.role.as_str() {
                    "user" => "[U]",
                    "assistant" => "[A]",
                    _ => "[S]",
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{} ", role_icon),
                        Style::default().fg(theme.primary),
                    ),
                    Span::styled(&entry.preview, Style::default().fg(theme.text)),
                    Span::styled(
                        format!("  {}", entry.timestamp),
                        Style::default().fg(theme.text_muted),
                    ),
                ]))
            })
            .collect();
        let list = List::new(items).highlight_style(
            Style::default()
                .bg(theme.background_element)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_stateful_widget(list, list_area, &mut self.state.clone());
    }
}

impl Default for ForkDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

