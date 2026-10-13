use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::theme::Theme;

pub enum AlertType {
    Info,
    Success,
    Warning,
    Error,
}

pub struct AlertDialog {
    title: String,
    message: String,
    alert_type: AlertType,
    open: bool,
}

impl AlertDialog {
    pub fn new(title: &str, message: &str, alert_type: AlertType) -> Self {
        Self {
            title: title.to_string(),
            message: message.to_string(),
            alert_type,
            open: false,
        }
    }

    pub fn info(message: &str) -> Self {
        Self::new("Info", message, AlertType::Info)
    }

    pub fn success(message: &str) -> Self {
        Self::new("Success", message, AlertType::Success)
    }

    pub fn warning(message: &str) -> Self {
        Self::new("Warning", message, AlertType::Warning)
    }

    pub fn error(message: &str) -> Self {
        Self::new("Error", message, AlertType::Error)
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_message(&mut self, message: &str) {
        self.message = message.to_string();
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn get_color(&self, theme: &Theme) -> ratatui::style::Color {
        match self.alert_type {
            AlertType::Info => theme.info,
            AlertType::Success => theme.success,
            AlertType::Warning => theme.warning,
            AlertType::Error => theme.error,
        }
    }

    fn get_icon(&self) -> &str {
        match self.alert_type {
            AlertType::Info => "ℹ",
            AlertType::Success => "✓",
            AlertType::Warning => "⚠",
            AlertType::Error => "✗",
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let color = self.get_color(theme);
        let icon = self.get_icon();

        let line_count = self.message.lines().count().max(1);
        let dialog_width = 50.min(area.width.saturating_sub(4));
        let dialog_height = (line_count + 6).min(16) as u16;

        let dialog_area = centered_rect(dialog_width, dialog_height, area);

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                format!(" {} {} ", icon, self.title),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color))
            .style(Style::default().bg(theme.background_panel));

        let inner = super::dialog_inner(block.inner(dialog_area));
        frame.render_widget(block, dialog_area);

        let paragraph = Paragraph::new(self.message.clone())
            .style(Style::default().fg(theme.text))
            .wrap(Wrap { trim: false })
            .centered();

        frame.render_widget(
            paragraph,
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: inner.height.saturating_sub(4),
            },
        );

        let hint = Line::from(vec![
            Span::styled("[Enter] ", Style::default().fg(theme.text_muted)),
            Span::styled("Close", Style::default().fg(theme.text)),
            Span::styled("  [Ctrl+C] ", Style::default().fg(theme.text_muted)),
            Span::styled("Copy", Style::default().fg(theme.text)),
        ]);
        frame.render_widget(
            Paragraph::new(hint).centered(),
            Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(3),
                width: inner.width,
                height: 1,
            },
        );

        let ok_button = Line::from(vec![Span::styled(
            " [ OK ] ",
            Style::default().fg(theme.text).bg(color),
        )]);

        frame.render_widget(
            Paragraph::new(ok_button).centered(),
            Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(2),
                width: inner.width,
                height: 1,
            },
        );
    }
}

impl Default for AlertDialog {
    fn default() -> Self {
        Self::info("Message")
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

