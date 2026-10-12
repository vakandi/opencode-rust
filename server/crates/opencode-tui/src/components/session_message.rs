use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ratatui::{
    style::Color,
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::markdown::MarkdownRenderer;
use crate::context::{Message, MessagePart};
use crate::theme::Theme;

/// Render a user message with shared left gutter shape.
pub fn render_user_message(
    msg: &Message,
    theme: &Theme,
    show_timestamps: bool,
    agent: Option<&str>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let border_char = "┃ ";
    let border_style = Style::default().fg(user_border_color_for_agent(agent, theme));

    if msg.parts.is_empty() {
        for line_text in msg.content.lines() {
            lines.push(Line::from(vec![
                Span::styled(border_char, border_style),
                Span::styled(line_text.to_string(), Style::default().fg(theme.text)),
            ]));
        }
    } else {
        for part in &msg.parts {
            match part {
                MessagePart::Text { text } => {
                    let md_renderer = MarkdownRenderer::new(theme.clone());
                    let md_lines = md_renderer.to_lines(text);
                    for md_line in md_lines {
                        let mut spans = vec![Span::styled(border_char, border_style)];
                        spans.extend(md_line.spans);
                        lines.push(Line::from(spans));
                    }
                }
                MessagePart::File { path, mime } => {
                    lines.push(Line::from(vec![
                        Span::styled(border_char, border_style),
                        Span::styled(
                            mime_badge(mime),
                            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::styled(path.clone(), Style::default().fg(theme.text)),
                    ]));
                }
                MessagePart::Image { url } => {
                    lines.push(Line::from(vec![
                        Span::styled(border_char, border_style),
                        Span::styled("[image] ", Style::default().fg(theme.info)),
                        Span::styled(url.clone(), Style::default().fg(theme.text_muted)),
                    ]));
                }
                _ => {}
            }
        }
    }

    if show_timestamps {
        let ts = msg.created_at.format("%H:%M").to_string();
        if !lines.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(border_char, border_style),
                Span::styled(ts, Style::default().fg(theme.text_muted)),
            ]));
        }
    }

    lines
}

fn mime_badge(mime: &str) -> String {
    let short = if let Some(sub) = mime.strip_prefix("image/") {
        sub.to_uppercase()
    } else if let Some(sub) = mime.strip_prefix("text/") {
        sub.to_uppercase()
    } else if let Some(sub) = mime.strip_prefix("application/") {
        sub.to_uppercase()
    } else {
        mime.to_uppercase()
    };
    format!("[{}]", short)
}

fn user_border_color_for_agent(agent: Option<&str>, theme: &Theme) -> Color {
    let Some(agent) = agent else {
        return theme.primary;
    };
    if theme.agent_colors.is_empty() {
        return theme.primary;
    }
    let mut hasher = DefaultHasher::new();
    agent.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % theme.agent_colors.len();
    theme.agent_colors[idx]
}
