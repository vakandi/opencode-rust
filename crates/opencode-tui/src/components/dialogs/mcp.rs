use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug)]
pub struct McpItem {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub resources: usize,
    pub error: Option<String>,
}

pub struct McpDialog {
    items: Vec<McpItem>,
    state: ListState,
    open: bool,
}

impl McpDialog {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            items: Vec::new(),
            state,
            open: false,
        }
    }

    pub fn set_items(&mut self, mut items: Vec<McpItem>) {
        items.sort_by(|a, b| a.name.cmp(&b.name));
        self.items = items;
        self.state
            .select(if self.items.is_empty() { None } else { Some(0) });
    }

    pub fn open(&mut self) {
        self.open = true;
        if self.items.is_empty() {
            self.state.select(None);
        } else if self.state.selected().is_none() {
            self.state.select(Some(0));
        }
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
            if selected < self.items.len().saturating_sub(1) {
                self.state.select(Some(selected + 1));
            }
        }
    }

    pub fn selected_item(&self) -> Option<McpItem> {
        self.state
            .selected()
            .and_then(|idx| self.items.get(idx))
            .cloned()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(86, 20, area);
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " MCP Servers ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let inner = super::dialog_inner(block.inner(dialog_area));
        frame.render_widget(block, dialog_area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let items = if self.items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No MCP server found.",
                Style::default().fg(theme.text_muted),
            )))]
        } else {
            self.items
                .iter()
                .map(|server| {
                    let (status_label, status_color) = match server.status.as_str() {
                        "connected" => ("Connected".to_string(), theme.success),
                        "failed" => (
                            server.error.clone().unwrap_or_else(|| "Failed".to_string()),
                            theme.error,
                        ),
                        "needs_auth" => ("Needs authentication".to_string(), theme.warning),
                        "needs_client_registration" => {
                            ("Needs client registration".to_string(), theme.error)
                        }
                        "disabled" => ("Disabled".to_string(), theme.text_muted),
                        "disconnected" => ("Disconnected".to_string(), theme.text_muted),
                        other => (other.to_string(), theme.info),
                    };

                    let mut spans = vec![
                        Span::styled("● ", Style::default().fg(status_color)),
                        Span::styled(&server.name, Style::default().fg(theme.text)),
                        Span::styled(
                            format!("  {}", status_label),
                            Style::default().fg(theme.text_muted),
                        ),
                    ];
                    if server.tools > 0 || server.resources > 0 {
                        spans.push(Span::styled(
                            format!("  tools:{} resources:{}", server.tools, server.resources),
                            Style::default().fg(theme.text_muted),
                        ));
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect()
        };
        let list = List::new(items).highlight_style(
            Style::default()
                .bg(theme.background_element)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_stateful_widget(list, layout[0], &mut self.state.clone());

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Enter", Style::default().fg(theme.primary)),
                Span::styled(
                    " connect/disconnect  ",
                    Style::default().fg(theme.text_muted),
                ),
                Span::styled("a", Style::default().fg(theme.primary)),
                Span::styled(" auth  ", Style::default().fg(theme.text_muted)),
                Span::styled("x", Style::default().fg(theme.primary)),
                Span::styled(" clear auth  ", Style::default().fg(theme.text_muted)),
                Span::styled("r", Style::default().fg(theme.primary)),
                Span::styled(" refresh  ", Style::default().fg(theme.text_muted)),
                Span::styled("Esc", Style::default().fg(theme.primary)),
                Span::styled(" close", Style::default().fg(theme.text_muted)),
            ])),
            layout[1],
        );
    }
}

impl Default for McpDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

