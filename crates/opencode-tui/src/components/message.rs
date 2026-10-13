use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::diff::DiffView;
use super::markdown::MarkdownRenderer;
use super::tool_call::ToolCallStatus;
use crate::theme::Theme;

pub enum MessagePart {
    Text {
        content: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
        status: ToolCallStatus,
        result: Option<String>,
    },
    Reasoning {
        content: String,
        duration_ms: Option<u64>,
    },
}

pub enum MessageRole {
    User,
    Assistant,
    System,
}

pub struct MessageView {
    pub role: MessageRole,
    pub parts: Vec<MessagePart>,
    pub model_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
    pub show_thinking: bool,
    pub concealed: bool,
}

impl MessageView {
    pub fn new(role: MessageRole) -> Self {
        Self {
            role,
            parts: Vec::new(),
            model_id: None,
            duration_ms: None,
            error: None,
            show_thinking: true,
            concealed: false,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let mut lines: Vec<Line> = Vec::new();

        for part in &self.parts {
            match part {
                MessagePart::Text { content } => {
                    lines.extend(self.render_text(content, theme));
                }
                MessagePart::ToolCall {
                    name,
                    status,
                    result,
                    arguments,
                    ..
                } => {
                    lines.extend(self.render_tool_call(
                        name,
                        arguments,
                        status,
                        result.as_deref(),
                        theme,
                    ));
                }
                MessagePart::Reasoning {
                    content,
                    duration_ms,
                } => {
                    if self.show_thinking {
                        lines.extend(self.render_reasoning(content, *duration_ms, theme));
                    }
                }
            }
        }

        // Error display (non-abort errors)
        if let Some(err) = &self.error {
            lines.push(Line::from(Span::styled(
                format!("Error: {}", err),
                Style::default().fg(theme.error),
            )));
        }

        // Completion footer for assistant messages
        if matches!(self.role, MessageRole::Assistant) {
            if let Some(model) = &self.model_id {
                let mut footer_spans = vec![Span::styled(
                    model.as_str(),
                    Style::default().fg(theme.text_muted),
                )];
                if let Some(ms) = self.duration_ms {
                    let duration_text = if ms < 1000 {
                        format!("  {}ms", ms)
                    } else {
                        format!("  {:.1}s", ms as f64 / 1000.0)
                    };
                    footer_spans.push(Span::styled(
                        duration_text,
                        Style::default().fg(theme.text_muted),
                    ));
                }
                lines.push(Line::from(footer_spans));
            }
        }

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, area);
    }

    fn render_text(&self, content: &str, theme: &Theme) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(theme.clone()).with_concealed(self.concealed);
        renderer.to_lines(content)
    }

    fn render_tool_call(
        &self,
        name: &str,
        arguments: &str,
        status: &ToolCallStatus,
        result: Option<&str>,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        let status_icon = match status {
            ToolCallStatus::Pending => "◯",
            ToolCallStatus::Running => "◐",
            ToolCallStatus::Completed => "●",
            ToolCallStatus::Failed => "✗",
        };

        let status_color = match status {
            ToolCallStatus::Pending => theme.text_muted,
            ToolCallStatus::Running => theme.warning,
            ToolCallStatus::Completed => theme.success,
            ToolCallStatus::Failed => theme.error,
        };

        // Tool header
        lines.push(Line::from(vec![
            Span::styled(status_icon, Style::default().fg(status_color)),
            Span::raw(" "),
            Span::styled(
                name.to_string(),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        // Dispatch to specialized tool views based on tool name
        match name {
            "bash" => {
                if !arguments.is_empty() {
                    let cmd = arguments.lines().next().unwrap_or(arguments).to_string();
                    lines.push(Line::from(vec![
                        Span::styled(
                            "$ ",
                            Style::default()
                                .fg(theme.primary)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(cmd, Style::default().fg(theme.text)),
                    ]));
                }
            }
            "edit" | "apply_patch" => {
                // Show diff if result contains diff content
                if let Some(res) = result {
                    let diff_view = DiffView::new().with_content(res);
                    let diff_lines = diff_view.to_lines(theme);
                    lines.extend(diff_lines);
                } else if !arguments.is_empty() {
                    for line in arguments.lines().take(5) {
                        lines.push(Line::from(Span::styled(
                            format!("  {}", line),
                            Style::default().fg(theme.text_muted),
                        )));
                    }
                }
            }
            "read" => {
                let path = arguments.lines().next().unwrap_or(arguments).to_string();
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(path, Style::default().fg(theme.info)),
                ]));
            }
            "write" => {
                let path = arguments.lines().next().unwrap_or(arguments).to_string();
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(path, Style::default().fg(theme.success)),
                ]));
            }
            "todowrite" => {
                if let Some(res) = result {
                    for line in res.lines().take(10) {
                        lines.push(Line::from(Span::styled(
                            format!("  {}", line),
                            Style::default().fg(theme.text),
                        )));
                    }
                }
            }
            _ => {
                // Generic: show truncated arguments
                if !arguments.is_empty() {
                    for line in arguments.lines().take(3) {
                        lines.push(Line::from(Span::styled(
                            format!("  {}", line),
                            Style::default().fg(theme.text_muted),
                        )));
                    }
                    if arguments.lines().count() > 3 {
                        lines.push(Line::from(Span::styled(
                            "  ...",
                            Style::default().fg(theme.text_muted),
                        )));
                    }
                }
            }
        }

        // Show result summary for completed tools
        if matches!(status, ToolCallStatus::Completed) {
            if let Some(res) = result {
                if !matches!(name, "edit" | "apply_patch" | "todowrite") {
                    let line_count = res.lines().count();
                    if line_count > 0 {
                        lines.push(Line::from(Span::styled(
                            format!("  ({} lines of output)", line_count),
                            Style::default().fg(theme.text_muted),
                        )));
                    }
                }
            }
        }

        // Show error for failed tools
        if matches!(status, ToolCallStatus::Failed) {
            if let Some(res) = result {
                lines.push(Line::from(Span::styled(
                    format!("  Error: {}", res.lines().next().unwrap_or("")),
                    Style::default().fg(theme.error),
                )));
            }
        }

        lines.push(Line::from(""));
        lines
    }

    fn render_reasoning(
        &self,
        content: &str,
        duration_ms: Option<u64>,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Strip [REDACTED] markers
        let cleaned = content.replace("[REDACTED]", "").trim().to_string();
        if cleaned.is_empty() {
            return lines;
        }

        let title = if let Some(ms) = duration_ms {
            if ms < 1000 {
                format!("Thinking ({}ms)", ms)
            } else {
                format!("Thinking ({:.1}s)", ms as f64 / 1000.0)
            }
        } else {
            "Thinking".to_string()
        };

        lines.push(Line::from(vec![
            Span::styled("▼ ", Style::default().fg(theme.primary)),
            Span::styled(
                title,
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        for line in cleaned.lines() {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(theme.text_muted),
            )));
        }

        lines.push(Line::from(""));
        lines
    }
}
