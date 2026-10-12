use chrono::{Local, TimeZone};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug)]
pub struct SessionItem {
    pub id: String,
    pub title: String,
    pub directory: String,
    pub parent_id: Option<String>,
    pub updated_at: i64,
    pub is_busy: bool,
}

#[derive(Clone, Debug)]
pub enum DeleteState {
    Armed(String),
    Confirmed(String),
}

pub struct SessionListDialog {
    sessions: Vec<SessionItem>,
    filtered: Vec<usize>,
    query: String,
    state: ListState,
    open: bool,
    active_session_id: Option<String>,
    pending_delete_session_id: Option<String>,
    rename_session_id: Option<String>,
    rename_input: String,
}

impl SessionListDialog {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            sessions: Vec::new(),
            filtered: Vec::new(),
            query: String::new(),
            state,
            open: false,
            active_session_id: None,
            pending_delete_session_id: None,
            rename_session_id: None,
            rename_input: String::new(),
        }
    }

    pub fn set_sessions(&mut self, mut sessions: Vec<SessionItem>) {
        sessions.retain(|s| s.parent_id.is_none());
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        self.sessions = sessions;
        self.filter();
    }

    pub fn open(&mut self, current_session_id: Option<&str>) {
        self.open = true;
        self.query.clear();
        self.pending_delete_session_id = None;
        self.rename_session_id = None;
        self.rename_input.clear();
        self.active_session_id = current_session_id.map(|id| id.to_string());
        self.filter();

        if let Some(current_id) = current_session_id {
            if let Some(filtered_index) = self
                .filtered
                .iter()
                .position(|idx| self.sessions.get(*idx).is_some_and(|s| s.id == current_id))
            {
                self.state.select(Some(filtered_index));
            }
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.pending_delete_session_id = None;
        self.rename_session_id = None;
        self.rename_input.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_renaming(&self) -> bool {
        self.rename_session_id.is_some()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn handle_input(&mut self, c: char) {
        self.query.push(c);
        self.pending_delete_session_id = None;
        self.filter();
    }

    pub fn handle_backspace(&mut self) {
        self.query.pop();
        self.pending_delete_session_id = None;
        self.filter();
    }

    pub fn move_up(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected > 0 {
                self.state.select(Some(selected - 1));
                self.pending_delete_session_id = None;
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected < self.filtered.len().saturating_sub(1) {
                self.state.select(Some(selected + 1));
                self.pending_delete_session_id = None;
            }
        }
    }

    pub fn selected_session_id(&self) -> Option<String> {
        self.state
            .selected()
            .and_then(|idx| self.filtered.get(idx))
            .and_then(|session_idx| self.sessions.get(*session_idx))
            .map(|s| s.id.clone())
    }

    pub fn start_rename_selected(&mut self) -> bool {
        let Some(selected_id) = self.selected_session_id() else {
            return false;
        };
        let Some(session) = self
            .sessions
            .iter()
            .find(|session| session.id == selected_id)
        else {
            return false;
        };
        self.pending_delete_session_id = None;
        self.rename_session_id = Some(session.id.clone());
        self.rename_input = session.title.clone();
        true
    }

    pub fn cancel_rename(&mut self) {
        self.rename_session_id = None;
        self.rename_input.clear();
    }

    pub fn handle_rename_input(&mut self, c: char) {
        self.rename_input.push(c);
    }

    pub fn handle_rename_backspace(&mut self) {
        self.rename_input.pop();
    }

    pub fn confirm_rename(&mut self) -> Option<(String, String)> {
        let session_id = self.rename_session_id.clone()?;
        let title = self.rename_input.trim().to_string();
        if title.is_empty() {
            return None;
        }
        self.rename_session_id = None;
        self.rename_input.clear();
        Some((session_id, title))
    }

    pub fn trigger_delete_selected(&mut self) -> Option<DeleteState> {
        let selected_id = self.selected_session_id()?;
        if self.pending_delete_session_id.as_deref() == Some(selected_id.as_str()) {
            self.pending_delete_session_id = None;
            return Some(DeleteState::Confirmed(selected_id));
        }
        self.pending_delete_session_id = Some(selected_id.clone());
        Some(DeleteState::Armed(selected_id))
    }

    fn filter(&mut self) {
        let query = self.query.to_lowercase();
        self.filtered = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                session.title.to_lowercase().contains(&query)
                    || session.id.to_lowercase().contains(&query)
                    || session.directory.to_lowercase().contains(&query)
            })
            .map(|(idx, _)| idx)
            .collect();
        self.state.select(if self.filtered.is_empty() {
            None
        } else {
            Some(0)
        });
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(82, 24, area);
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Sessions ",
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
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(theme.primary)),
                Span::styled(&self.query, Style::default().fg(theme.text)),
                Span::styled("▏", Style::default().fg(theme.primary)),
            ])),
            layout[0],
        );

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .filter_map(|idx| self.sessions.get(*idx))
            .map(|session| {
                let is_current = self.active_session_id.as_deref() == Some(session.id.as_str());
                let is_pending_delete =
                    self.pending_delete_session_id.as_deref() == Some(session.id.as_str());
                let (category, time_label) = format_session_time(session.updated_at);
                let title = if is_pending_delete {
                    "Press ctrl+d again to confirm delete".to_string()
                } else {
                    session.title.clone()
                };
                let title_style = if is_pending_delete {
                    Style::default()
                        .fg(theme.error)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                };
                let marker = if is_current { "● " } else { "  " };
                let busy = if session.is_busy { "◌ " } else { "  " };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        marker,
                        Style::default().fg(if is_current {
                            theme.success
                        } else {
                            theme.text_muted
                        }),
                    ),
                    Span::styled(
                        busy,
                        Style::default().fg(if session.is_busy {
                            theme.warning
                        } else {
                            theme.text_muted
                        }),
                    ),
                    Span::styled(title, title_style),
                    Span::styled(
                        format!(
                            "  {}  {}  {}  {}",
                            category, time_label, session.id, session.directory
                        ),
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
        frame.render_stateful_widget(list, layout[1], &mut self.state.clone());

        let action_line = if self.is_renaming() {
            Line::from(vec![
                Span::styled("Rename: ", Style::default().fg(theme.primary)),
                Span::styled(&self.rename_input, Style::default().fg(theme.text)),
                Span::styled("▏", Style::default().fg(theme.primary)),
            ])
        } else {
            Line::from(vec![
                Span::styled("ctrl+r", Style::default().fg(theme.primary)),
                Span::styled(" rename  ", Style::default().fg(theme.text_muted)),
                Span::styled("ctrl+d", Style::default().fg(theme.primary)),
                Span::styled(" delete", Style::default().fg(theme.text_muted)),
            ])
        };
        frame.render_widget(Paragraph::new(action_line), layout[2]);

        let footer = if self.is_renaming() {
            Paragraph::new(Line::from(vec![
                Span::styled("Enter", Style::default().fg(theme.primary)),
                Span::styled(" save  ", Style::default().fg(theme.text_muted)),
                Span::styled("Esc", Style::default().fg(theme.primary)),
                Span::styled(" cancel", Style::default().fg(theme.text_muted)),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled("Enter", Style::default().fg(theme.primary)),
                Span::styled(" open session  ", Style::default().fg(theme.text_muted)),
                Span::styled("Esc", Style::default().fg(theme.primary)),
                Span::styled(" close", Style::default().fg(theme.text_muted)),
            ]))
        };
        frame.render_widget(footer, layout[3]);
    }
}

impl Default for SessionListDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}


fn format_session_time(updated_at_ms: i64) -> (String, String) {
    let Some(updated_local) = Local.timestamp_millis_opt(updated_at_ms).single() else {
        return ("Unknown".to_string(), "--:--".to_string());
    };

    let now = Local::now();
    let category = if updated_local.date_naive() == now.date_naive() {
        "Today".to_string()
    } else {
        updated_local.format("%Y-%m-%d").to_string()
    };

    (category, updated_local.format("%H:%M").to_string())
}
