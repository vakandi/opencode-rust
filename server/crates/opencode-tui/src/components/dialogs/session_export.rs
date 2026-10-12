use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme::Theme;

pub struct SessionExportDialog {
    open: bool,
    session_id: Option<String>,
    filename: String,
    pub include_thinking: bool,
    pub include_tool_details: bool,
    pub include_metadata: bool,
}

impl SessionExportDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            session_id: None,
            filename: String::new(),
            include_thinking: false,
            include_tool_details: true,
            include_metadata: false,
        }
    }

    pub fn open(&mut self, session_id: String, default_filename: String) {
        self.open = true;
        self.session_id = Some(session_id);
        self.filename = default_filename;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.session_id = None;
        self.filename.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn handle_input(&mut self, c: char) {
        match c {
            '1' => self.include_thinking = !self.include_thinking,
            '2' => self.include_tool_details = !self.include_tool_details,
            '3' => self.include_metadata = !self.include_metadata,
            _ => self.filename.push(c),
        }
    }

    pub fn handle_backspace(&mut self) {
        self.filename.pop();
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(80, 14, area);
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Export Session ",
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
                Constraint::Length(1), // "Output filename:"
                Constraint::Length(1), // filename input
                Constraint::Length(1), // spacer
                Constraint::Length(1), // "Options:"
                Constraint::Length(1), // [1] thinking
                Constraint::Length(1), // [2] tool details
                Constraint::Length(1), // [3] metadata
                Constraint::Length(1), // spacer
                Constraint::Length(1), // hint
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new("Output filename:").style(Style::default().fg(theme.text)),
            layout[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(theme.primary)),
                Span::styled(&self.filename, Style::default().fg(theme.text)),
                Span::styled("▏", Style::default().fg(theme.primary)),
            ])),
            layout[1],
        );

        frame.render_widget(
            Paragraph::new("Options:").style(Style::default().fg(theme.text_muted)),
            layout[3],
        );

        let check = |v: bool| if v { "[x]" } else { "[ ]" };
        frame.render_widget(
            Paragraph::new(format!(
                "  1  {} Include thinking blocks",
                check(self.include_thinking)
            ))
            .style(Style::default().fg(theme.text)),
            layout[4],
        );
        frame.render_widget(
            Paragraph::new(format!(
                "  2  {} Include tool call details",
                check(self.include_tool_details)
            ))
            .style(Style::default().fg(theme.text)),
            layout[5],
        );
        frame.render_widget(
            Paragraph::new(format!(
                "  3  {} Include metadata (tokens, cost)",
                check(self.include_metadata)
            ))
            .style(Style::default().fg(theme.text)),
            layout[6],
        );

        frame.render_widget(
            Paragraph::new(
                "Enter export  Ctrl+C copy transcript  1/2/3 toggle options  Esc cancel",
            )
            .style(Style::default().fg(theme.text_muted)),
            layout[8],
        );
    }
}

impl Default for SessionExportDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

