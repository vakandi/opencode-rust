use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::theme::Theme;

pub struct ThinkingBlock {
    pub content: String,
    pub duration_ms: Option<u64>,
    pub collapsed: bool,
    pub visible: bool,
}

impl ThinkingBlock {
    pub fn new(content: String) -> Self {
        // Strip [REDACTED] markers from content
        let cleaned = content.replace("[REDACTED]", "").trim().to_string();
        Self {
            content: cleaned,
            duration_ms: None,
            collapsed: true,
            visible: true,
        }
    }

    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
    }

    pub fn expand(&mut self) {
        self.collapsed = false;
    }

    pub fn collapse(&mut self) {
        self.collapsed = true;
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.visible {
            return;
        }

        let title = if let Some(ms) = self.duration_ms {
            if ms < 1000 {
                format!("Thinking... ({}ms)", ms)
            } else {
                format!("Thinking... ({:.1}s)", ms as f64 / 1000.0)
            }
        } else {
            "Thinking...".to_string()
        };

        let expand_icon = if self.collapsed { "▶" } else { "▼" };

        let lines = if self.collapsed {
            vec![Line::from(vec![
                Span::styled(expand_icon, Style::default().fg(theme.primary)),
                Span::raw(" "),
                Span::styled(title, Style::default().fg(theme.text_muted)),
            ])]
        } else {
            let mut lines = vec![Line::from(vec![
                Span::styled(expand_icon, Style::default().fg(theme.primary)),
                Span::raw(" "),
                Span::styled(
                    title,
                    Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD),
                ),
            ])];

            lines.push(Line::from(""));
            // Show full content when expanded (no 10-line limit)
            for line in self.content.lines() {
                lines.push(Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(theme.text_muted),
                )));
            }
            lines
        };

        let paragraph = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(theme.info)),
        );

        frame.render_widget(paragraph, area);
    }
}
