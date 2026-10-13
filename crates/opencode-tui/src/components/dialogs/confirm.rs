use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme::Theme;

pub struct ConfirmDialog {
    title: String,
    message: String,
    confirm_text: String,
    cancel_text: String,
    focused: bool,
    open: bool,
}

impl ConfirmDialog {
    pub fn new(title: &str, message: &str) -> Self {
        Self {
            title: title.to_string(),
            message: message.to_string(),
            confirm_text: "Confirm".to_string(),
            cancel_text: "Cancel".to_string(),
            focused: false,
            open: false,
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.focused = false;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn toggle_focus(&mut self) {
        self.focused = !self.focused;
    }

    pub fn is_confirm(&self) -> bool {
        self.focused
    }

    pub fn handle_left(&mut self) {
        self.focused = false;
    }

    pub fn handle_right(&mut self) {
        self.focused = true;
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_width = 50;
        let dialog_height = 7;
        let dialog_area = centered_rect(dialog_width, dialog_height, area);

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                format!(" {} ", self.title),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));

        let inner = super::dialog_inner(block.inner(dialog_area));
        frame.render_widget(block, dialog_area);

        let message_line = Line::from(Span::styled(&self.message, Style::default().fg(theme.text)));
        frame.render_widget(
            Paragraph::new(message_line).centered(),
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: 2,
            },
        );

        let cancel_style = if self.focused {
            Style::default().fg(theme.text_muted)
        } else {
            Style::default().fg(theme.text).bg(theme.primary)
        };

        let confirm_style = if self.focused {
            Style::default().fg(theme.text).bg(theme.error)
        } else {
            Style::default().fg(theme.text_muted)
        };

        let buttons = Line::from(vec![
            Span::styled(format!(" {} ", self.cancel_text), cancel_style),
            Span::raw("   "),
            Span::styled(format!(" {} ", self.confirm_text), confirm_style),
        ]);

        frame.render_widget(
            Paragraph::new(buttons).centered(),
            Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(2),
                width: inner.width,
                height: 1,
            },
        );
    }
}

impl Default for ConfirmDialog {
    fn default() -> Self {
        Self::new("Confirm", "Are you sure?")
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

