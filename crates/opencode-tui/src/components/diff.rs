use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug, PartialEq)]
pub enum DiffMode {
    Split,
    Unified,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub content: String,
    pub line_type: DiffLineType,
    pub old_line_num: Option<u32>,
    pub new_line_num: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiffLineType {
    Context,
    Added,
    Removed,
    HunkHeader,
}

pub struct DiffView {
    lines: Vec<DiffLine>,
    mode: DiffMode,
    scroll_offset: u16,
}

impl DiffView {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            mode: DiffMode::Unified,
            scroll_offset: 0,
        }
    }

    pub fn with_content(mut self, content: &str) -> Self {
        self.parse_diff(content);
        self
    }

    pub fn with_mode(mut self, mode: DiffMode) -> Self {
        self.mode = mode;
        self
    }

    fn parse_diff(&mut self, content: &str) {
        let mut old_line: u32 = 0;
        let mut new_line: u32 = 0;

        for line in content.lines() {
            let (diff_line, _update_counts) = if let Some(stripped) = line.strip_prefix("@@ ") {
                let header = stripped.to_string();
                old_line = Self::parse_hunk_start(&header, true);
                new_line = Self::parse_hunk_start(&header, false);
                (
                    DiffLine {
                        content: line.to_string(),
                        line_type: DiffLineType::HunkHeader,
                        old_line_num: None,
                        new_line_num: None,
                    },
                    false,
                )
            } else if line.starts_with('+') && !line.starts_with("+++") {
                let content = line[1..].to_string();
                let num = new_line;
                new_line += 1;
                (
                    DiffLine {
                        content,
                        line_type: DiffLineType::Added,
                        old_line_num: None,
                        new_line_num: Some(num),
                    },
                    true,
                )
            } else if line.starts_with('-') && !line.starts_with("---") {
                let content = line[1..].to_string();
                let num = old_line;
                old_line += 1;
                (
                    DiffLine {
                        content,
                        line_type: DiffLineType::Removed,
                        old_line_num: Some(num),
                        new_line_num: None,
                    },
                    true,
                )
            } else if line.starts_with(' ') || line.is_empty() {
                let content = if line.is_empty() {
                    " ".to_string()
                } else {
                    line[1..].to_string()
                };
                let o = old_line;
                let n = new_line;
                old_line += 1;
                new_line += 1;
                (
                    DiffLine {
                        content,
                        line_type: DiffLineType::Context,
                        old_line_num: Some(o),
                        new_line_num: Some(n),
                    },
                    true,
                )
            } else {
                (
                    DiffLine {
                        content: line.to_string(),
                        line_type: DiffLineType::Context,
                        old_line_num: Some(old_line),
                        new_line_num: Some(new_line),
                    },
                    true,
                )
            };

            self.lines.push(diff_line);
        }
    }

    fn parse_hunk_start(header: &str, is_old: bool) -> u32 {
        let parts: Vec<&str> = header.split_whitespace().collect();
        if parts.len() >= 2 {
            let range = if is_old { parts[0] } else { parts[1] };
            range
                .trim_start_matches(&['-', '+'][..])
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1)
        } else {
            1
        }
    }

    pub fn set_mode(&mut self, mode: DiffMode) {
        self.mode = mode;
    }

    /// Convert diff lines to ratatui Lines for embedding in other widgets.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        for diff_line in &self.lines {
            let prefix = match diff_line.line_type {
                DiffLineType::Added => Span::styled("+ ", Style::default().fg(theme.diff_added)),
                DiffLineType::Removed => {
                    Span::styled("- ", Style::default().fg(theme.diff_removed))
                }
                DiffLineType::HunkHeader => Span::styled("@@ ", Style::default().fg(theme.primary)),
                DiffLineType::Context => Span::styled("  ", Style::default().fg(theme.text_muted)),
            };

            let line_num = if let Some(n) = diff_line.new_line_num.or(diff_line.old_line_num) {
                format!("{:4} ", n)
            } else {
                "     ".to_string()
            };

            let line_style = match diff_line.line_type {
                DiffLineType::Added => Style::default().fg(theme.diff_added),
                DiffLineType::Removed => Style::default().fg(theme.diff_removed),
                DiffLineType::HunkHeader => Style::default().fg(theme.primary),
                DiffLineType::Context => Style::default().fg(theme.text),
            };

            lines.push(Line::from(vec![
                Span::raw(line_num),
                prefix,
                Span::styled(diff_line.content.clone(), line_style),
            ]));
        }

        lines
    }

    pub fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    pub fn scroll_down(&mut self, max: u16) {
        if self.scroll_offset < max {
            self.scroll_offset += 1;
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.lines.is_empty() {
            let empty = Paragraph::new("No changes")
                .block(Block::default().title(" Diff ").borders(Borders::ALL))
                .style(Style::default().fg(theme.text_muted));
            frame.render_widget(empty, area);
            return;
        }

        let width = area.width as usize;

        if self.mode == DiffMode::Split && width > 120 {
            self.render_split(frame, area, theme);
        } else {
            self.render_unified(frame, area, theme, width);
        }
    }

    fn render_unified(&self, frame: &mut Frame, area: Rect, theme: &Theme, width: usize) {
        let mut lines = Vec::new();

        for diff_line in &self.lines {
            let prefix = match diff_line.line_type {
                DiffLineType::Added => Span::styled("+ ", Style::default().fg(theme.diff_added)),
                DiffLineType::Removed => {
                    Span::styled("- ", Style::default().fg(theme.diff_removed))
                }
                DiffLineType::HunkHeader => Span::styled("@@ ", Style::default().fg(theme.primary)),
                DiffLineType::Context => Span::styled("  ", Style::default().fg(theme.text_muted)),
            };

            let line_num = if let Some(n) = diff_line.new_line_num.or(diff_line.old_line_num) {
                format!("{:4} ", n)
            } else {
                "     ".to_string()
            };

            let line_style = match diff_line.line_type {
                DiffLineType::Added => Style::default().fg(theme.diff_added),
                DiffLineType::Removed => Style::default().fg(theme.diff_removed),
                DiffLineType::HunkHeader => Style::default().fg(theme.primary),
                DiffLineType::Context => Style::default().fg(theme.text),
            };

            let content = if diff_line.content.chars().count() > width.saturating_sub(12) {
                diff_line
                    .content
                    .chars()
                    .take(width.saturating_sub(15))
                    .collect::<String>()
            } else {
                diff_line.content.clone()
            };

            lines.push(Line::from(vec![
                Span::raw(line_num),
                prefix,
                Span::styled(content, line_style),
            ]));
        }

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Diff ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border)),
            )
            .scroll((self.scroll_offset, 0));

        frame.render_widget(paragraph, area);
    }

    fn render_split(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let constraints = vec![Constraint::Percentage(50), Constraint::Percentage(50)];
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(area);

        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();

        for diff_line in &self.lines {
            match diff_line.line_type {
                DiffLineType::Removed => {
                    old_lines.push(Line::from(vec![
                        Span::styled(
                            format!("{:4}", diff_line.old_line_num.unwrap_or(0)),
                            Style::default().fg(theme.text_muted),
                        ),
                        Span::styled(
                            diff_line.content.clone(),
                            Style::default().fg(theme.diff_removed),
                        ),
                    ]));
                    new_lines.push(Line::from(""));
                }
                DiffLineType::Added => {
                    old_lines.push(Line::from(""));
                    new_lines.push(Line::from(vec![
                        Span::styled(
                            format!("{:4}", diff_line.new_line_num.unwrap_or(0)),
                            Style::default().fg(theme.text_muted),
                        ),
                        Span::styled(
                            diff_line.content.clone(),
                            Style::default().fg(theme.diff_added),
                        ),
                    ]));
                }
                DiffLineType::Context => {
                    old_lines.push(Line::from(vec![
                        Span::styled(
                            format!("{:4}", diff_line.old_line_num.unwrap_or(0)),
                            Style::default().fg(theme.text_muted),
                        ),
                        Span::styled(diff_line.content.clone(), Style::default().fg(theme.text)),
                    ]));
                    new_lines.push(Line::from(vec![
                        Span::styled(
                            format!("{:4}", diff_line.new_line_num.unwrap_or(0)),
                            Style::default().fg(theme.text_muted),
                        ),
                        Span::styled(diff_line.content.clone(), Style::default().fg(theme.text)),
                    ]));
                }
                DiffLineType::HunkHeader => {
                    old_lines.push(Line::from(Span::styled(
                        diff_line.content.clone(),
                        Style::default().fg(theme.primary),
                    )));
                    new_lines.push(Line::from(Span::styled(
                        diff_line.content.clone(),
                        Style::default().fg(theme.primary),
                    )));
                }
            }
        }

        let old_block = Block::default()
            .title(" Old ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.diff_removed));

        let new_block = Block::default()
            .title(" New ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.diff_added));

        frame.render_widget(
            Paragraph::new(old_lines)
                .block(old_block)
                .scroll((self.scroll_offset, 0)),
            chunks[0],
        );
        frame.render_widget(
            Paragraph::new(new_lines)
                .block(new_block)
                .scroll((self.scroll_offset, 0)),
            chunks[1],
        );
    }
}

impl Default for DiffView {
    fn default() -> Self {
        Self::new()
    }
}
