use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme::Theme;

pub struct HelpDialog {
    open: bool,
}

impl HelpDialog {
    pub fn new() -> Self {
        Self { open: false }
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

        let dialog_area = centered_rect(74, 20, area);
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Help ",
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

        let lines = vec![
            Line::from(Span::styled(
                "General",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  Ctrl+X  Open command list"),
            Line::from("  Ctrl+P  Open command palette"),
            Line::from("  Ctrl+H  Open help"),
            Line::from("  Ctrl+C/q Exit TUI"),
            Line::from(""),
            Line::from(Span::styled(
                "Prompt",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  Enter   Submit prompt"),
            Line::from("  Ctrl+U  Clear prompt"),
            Line::from("  Ctrl+V  Paste clipboard"),
            Line::from("  Ctrl+Shift+C  Copy prompt"),
            Line::from("  Ctrl+Shift+X  Cut prompt"),
            Line::from("  Alt+Up/Alt+Down  Prompt history"),
            Line::from(""),
            Line::from(Span::styled(
                "Dialogs",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("  Ctrl+M  Model list"),
            Line::from("  Ctrl+V  Cycle model variant"),
            Line::from("  Use /agents to open full agent list"),
            Line::from("  Ctrl+S  Toggle sidebar"),
            Line::from("  Use command palette for session/theme/status/MCP dialogs"),
            Line::from("  Use command palette -> Toggle appearance to switch dark/light"),
        ];

        frame.render_widget(
            Paragraph::new(lines).style(Style::default().fg(theme.text)),
            layout[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Esc", Style::default().fg(theme.primary)),
                Span::styled(" close", Style::default().fg(theme.text_muted)),
            ])),
            layout[1],
        );
    }
}

impl Default for HelpDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

