use std::cell::Cell;

use ratatui::prelude::Stylize;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug, PartialEq)]
pub enum PermissionType {
    ReadFile,
    WriteFile,
    Edit,
    ExecuteCommand,
    Bash,
    NetworkRequest,
    Glob,
    Grep,
    List,
    Task,
    WebFetch,
    WebSearch,
    CodeSearch,
    ExternalDirectory,
}

impl PermissionType {
    pub fn label(&self) -> &'static str {
        match self {
            PermissionType::ReadFile => "Read file",
            PermissionType::WriteFile => "Write file",
            PermissionType::Edit => "Edit file",
            PermissionType::ExecuteCommand => "Execute command",
            PermissionType::Bash => "Run shell command",
            PermissionType::NetworkRequest => "Network request",
            PermissionType::Glob => "Glob search",
            PermissionType::Grep => "Grep search",
            PermissionType::List => "List directory",
            PermissionType::Task => "Task operation",
            PermissionType::WebFetch => "Fetch web content",
            PermissionType::WebSearch => "Web search",
            PermissionType::CodeSearch => "Code search",
            PermissionType::ExternalDirectory => "External directory access",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            PermissionType::ReadFile => "[R]",
            PermissionType::WriteFile => "[W]",
            PermissionType::Edit => "[E]",
            PermissionType::ExecuteCommand => "[X]",
            PermissionType::Bash => "[!]",
            PermissionType::NetworkRequest => "[N]",
            PermissionType::Glob => "[G]",
            PermissionType::Grep => "[S]",
            PermissionType::List => "[L]",
            PermissionType::Task => "[T]",
            PermissionType::WebFetch => "[F]",
            PermissionType::WebSearch => "[Q]",
            PermissionType::CodeSearch => "[C]",
            PermissionType::ExternalDirectory => "[D]",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub id: String,
    pub permission_type: PermissionType,
    pub resource: String,
    pub tool_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PermissionAction {
    Approve,
    Deny,
    ApproveAlways,
}

pub struct PermissionPrompt {
    requests: Vec<PermissionRequest>,
    current_index: usize,
    pub is_open: bool,
    last_rendered_area: Cell<Option<Rect>>,
    pending_action: Option<PermissionAction>,
}

impl PermissionPrompt {
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
            current_index: 0,
            is_open: false,
            last_rendered_area: Cell::new(None),
            pending_action: None,
        }
    }

    pub fn add_request(&mut self, request: PermissionRequest) {
        self.requests.push(request);
        self.is_open = !self.requests.is_empty();
    }

    pub fn current_request(&self) -> Option<&PermissionRequest> {
        self.requests.get(self.current_index)
    }

    pub fn approve(&mut self) -> Option<PermissionRequest> {
        if self.current_index < self.requests.len() {
            let request = self.requests.remove(self.current_index);
            if self.requests.is_empty() {
                self.is_open = false;
            }
            Some(request)
        } else {
            None
        }
    }

    pub fn deny(&mut self) -> Option<PermissionRequest> {
        if self.current_index < self.requests.len() {
            let request = self.requests.remove(self.current_index);
            if self.requests.is_empty() {
                self.is_open = false;
            }
            Some(request)
        } else {
            None
        }
    }

    pub fn approve_always(&mut self) -> Option<PermissionRequest> {
        let mut request = self.approve()?;
        request.id = format!("{}_always", request.id);
        Some(request)
    }

    pub fn close(&mut self) {
        self.requests.clear();
        self.current_index = 0;
        self.is_open = false;
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    pub fn pending_count(&self) -> usize {
        self.requests.len()
    }

    pub fn handle_click(&mut self, col: u16, row: u16) {
        if !self.is_open || self.requests.is_empty() {
            return;
        }
        // Button row is the last content line inside the border.
        // We store the last rendered area to check clicks.
        // For simplicity, check if the click is on the button hints row
        // and map x-position to the three buttons.
        if let Some(area) = self.last_rendered_area.get() {
            if row < area.y
                || row >= area.y + area.height
                || col < area.x
                || col >= area.x + area.width
            {
                return;
            }
            // The button line is at the bottom of the content (area.y + area.height - 2 for border)
            let button_row = area.y + area.height - 2;
            if row == button_row {
                // "[y] Allow  [n] Deny  [a] Always allow"
                // Rough column ranges inside the border:
                let inner_col = col.saturating_sub(area.x + 1);
                if inner_col < 11 {
                    // [y] Allow
                    self.pending_action = Some(PermissionAction::Approve);
                } else if inner_col < 21 {
                    // [n] Deny
                    self.pending_action = Some(PermissionAction::Deny);
                } else {
                    // [a] Always allow
                    self.pending_action = Some(PermissionAction::ApproveAlways);
                }
            }
        }
    }

    pub fn take_pending_action(&mut self) -> Option<PermissionAction> {
        self.pending_action.take()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.is_open || self.requests.is_empty() {
            return;
        }

        let request = match self.current_request() {
            Some(r) => r,
            None => return,
        };

        let height = 8u16;
        let width = area.width.saturating_sub(2).min(80);

        // Render inline at the bottom of the session area
        let popup_area = Rect::new(
            area.x + 1,
            area.y + area.height.saturating_sub(height + 1),
            width,
            height,
        );

        self.last_rendered_area.set(Some(popup_area));

        let title = format!(
            "{} {} - Permission Request",
            request.permission_type.icon(),
            request.permission_type.label()
        );

        let content = vec![
            Line::from(Span::styled(
                &title,
                Style::default().fg(theme.primary).bold(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Tool: ", Style::default().fg(theme.text_muted)),
                Span::styled(&request.tool_name, Style::default().fg(theme.text)),
            ]),
            Line::from(vec![
                Span::styled("Resource: ", Style::default().fg(theme.text_muted)),
                Span::styled(&request.resource, Style::default().fg(theme.text)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("[y] Allow  ", Style::default().fg(theme.success)),
                Span::styled("[n] Deny  ", Style::default().fg(theme.error)),
                Span::styled("[a] Always allow", Style::default().fg(theme.primary)),
            ]),
        ];

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .title(" Permission ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.warning)),
            )
            .style(Style::default().bg(theme.background_panel));

        frame.render_widget(paragraph, popup_area);
    }
}

impl Default for PermissionPrompt {
    fn default() -> Self {
        Self::new()
    }
}
