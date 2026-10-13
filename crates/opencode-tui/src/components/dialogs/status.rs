use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug)]
pub enum StatusLineKind {
    Title,
    Normal,
    Muted,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct StatusLine {
    pub text: String,
    pub kind: StatusLineKind,
}

impl StatusLine {
    pub fn title(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Title,
        }
    }

    pub fn normal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Normal,
        }
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Muted,
        }
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Success,
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Warning,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusLineKind::Error,
        }
    }
}

pub struct StatusDialog {
    lines: Vec<StatusLine>,
    open: bool,
}

impl StatusDialog {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            open: false,
        }
    }

    pub fn set_lines(&mut self, lines: Vec<String>) {
        self.lines = lines.into_iter().map(StatusLine::normal).collect();
    }

    pub fn set_status_lines(&mut self, lines: Vec<StatusLine>) {
        self.lines = lines;
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

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(90, 24, area);
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Status ",
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

        let lines = if self.lines.is_empty() {
            vec![Line::from(Span::styled(
                "No status data available.",
                Style::default().fg(theme.text_muted),
            ))]
        } else {
            self.lines
                .iter()
                .map(|line| {
                    let style = match line.kind {
                        StatusLineKind::Title => Style::default()
                            .fg(theme.primary)
                            .add_modifier(Modifier::BOLD),
                        StatusLineKind::Normal => Style::default().fg(theme.text),
                        StatusLineKind::Muted => Style::default().fg(theme.text_muted),
                        StatusLineKind::Success => Style::default().fg(theme.success),
                        StatusLineKind::Warning => Style::default().fg(theme.warning),
                        StatusLineKind::Error => Style::default().fg(theme.error),
                    };
                    Line::from(Span::styled(&line.text, style))
                })
                .collect()
        };
        frame.render_widget(Paragraph::new(lines), layout[0]);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Esc", Style::default().fg(theme.primary)),
                Span::styled(" close", Style::default().fg(theme.text_muted)),
            ])),
            layout[1],
        );
    }
}

impl Default for StatusDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

