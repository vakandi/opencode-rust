use chrono::Utc;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use super::message_palette;
use super::sidebar::SidebarState;
use crate::components::{Prompt, Sidebar};
use crate::context::{AppContext, Message, MessagePart, MessageRole, SidebarMode};

const SIDEBAR_WIDTH: u16 = 42;
const HEADER_NARROW_THRESHOLD: u16 = 80;
const THINKING_PREVIEW_LINES: usize = 2;
const MOUSE_SCROLL_LINES: usize = 3;
const MESSAGE_BLOCK_RIGHT_PADDING: usize = 1;
const SIDEBAR_CLOSE_BUTTON_WIDTH: u16 = 3;
const SIDEBAR_OPEN_BUTTON_WIDTH: u16 = 3;

struct ThinkingToggleHit {
    line_index: usize,
    reasoning_id: String,
}

pub struct SessionView {
    context: Arc<AppContext>,
    session_id: String,
    scroll_offset: usize,
    rendered_line_count: usize,
    messages_viewport_height: usize,
    expanded_reasoning: HashSet<String>,
    thinking_toggle_hits: Vec<ThinkingToggleHit>,
    last_messages_area: Option<Rect>,
    line_to_message: Vec<Option<String>>,
    message_first_lines: HashMap<String, usize>,
    sidebar_state: SidebarState,
    sidebar_close_button_area: Option<Rect>,
    sidebar_open_button_area: Option<Rect>,
}

impl SessionView {
    pub fn new(context: Arc<AppContext>, session_id: String) -> Self {
        Self {
            context,
            session_id,
            scroll_offset: 0,
            rendered_line_count: 0,
            messages_viewport_height: 0,
            expanded_reasoning: HashSet::new(),
            thinking_toggle_hits: Vec::new(),
            last_messages_area: None,
            line_to_message: Vec::new(),
            message_first_lines: HashMap::new(),
            sidebar_state: SidebarState::default(),
            sidebar_close_button_area: None,
            sidebar_open_button_area: None,
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, prompt: &Prompt) {
        let show_sidebar = *self.context.show_sidebar.read();
        let sidebar_mode = self.context.sidebar_mode.read().clone();
        // Determine effective sidebar visibility (overlay on all widths)
        let effective_show = match sidebar_mode {
            SidebarMode::Auto => show_sidebar,
            SidebarMode::Show => show_sidebar,
            SidebarMode::Hide => false,
        };

        // Always render main content full-width; sidebar floats above it.
        if effective_show {
            self.sidebar_open_button_area = None;
            self.render_main(frame, area, prompt);
            self.render_sidebar_overlay(frame, area);
        } else {
            self.sidebar_state.reset_hidden();
            self.sidebar_close_button_area = None;
            self.render_main(frame, area, prompt);
            self.render_sidebar_open_button(frame, area);
        }
    }

    fn render_sidebar_overlay(&mut self, frame: &mut Frame, area: Rect) {
        let theme = self.context.theme.read();
        let sidebar = Sidebar::new(self.context.clone(), self.session_id.clone());

        // Overlay on the right portion of the screen
        let overlay_width = SIDEBAR_WIDTH.min(area.width);
        let sidebar_area = Rect {
            x: area.x + area.width.saturating_sub(overlay_width),
            y: area.y,
            width: overlay_width,
            height: area.height,
        };

        let overlay_tint = tint_sidebar_overlay(theme.background_menu, theme.primary);

        // Render a subtle underlay just for the sidebar area.
        let underlay = Block::default().style(Style::default().bg(overlay_tint));
        frame.render_widget(underlay, sidebar_area);

        self.sidebar_close_button_area = Some(Rect {
            x: sidebar_area
                .x
                .saturating_add(sidebar_area.width.saturating_sub(SIDEBAR_CLOSE_BUTTON_WIDTH)),
            y: sidebar_area.y,
            width: SIDEBAR_CLOSE_BUTTON_WIDTH.min(sidebar_area.width),
            height: 1,
        });

        sidebar.render(frame, sidebar_area, &mut self.sidebar_state, true);
        if let Some(close_area) = self.sidebar_close_button_area {
            let close = Paragraph::new("✕")
                .style(Style::default().fg(theme.text).bg(overlay_tint))
                .alignment(ratatui::layout::Alignment::Center);
            frame.render_widget(close, close_area);
        }
    }

    fn render_sidebar_open_button(&mut self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            self.sidebar_open_button_area = None;
            return;
        }
        let theme = self.context.theme.read();
        let button = Rect {
            x: area
                .x
                .saturating_add(area.width.saturating_sub(SIDEBAR_OPEN_BUTTON_WIDTH)),
            y: area.y.saturating_add(1).min(area.y + area.height.saturating_sub(1)),
            width: SIDEBAR_OPEN_BUTTON_WIDTH.min(area.width),
            height: 1,
        };
        self.sidebar_open_button_area = Some(button);
        let glyph = Paragraph::new("☰")
            .style(
                Style::default()
                    .fg(theme.primary)
                    .bg(theme.background_element)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(glyph, button);
    }

    fn render_main(&mut self, frame: &mut Frame, area: Rect, prompt: &Prompt) {
        // Apply breathing boundary padding
        let area = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        };
        if area.width == 0 || area.height == 0 {
            return;
        }

        let show_header = {
            let user_pref = *self.context.show_header.read();
            if !user_pref { false } else { true }
        };
        let header_height = if show_header {
            if area.width < HEADER_NARROW_THRESHOLD {
                3u16
            } else {
                2u16
            }
        } else {
            0u16
        };
        let session_footer_height = 0u16;
        let desired_prompt_height = prompt.desired_height(area.width).max(3);
        let total_height = area.height;
        let available_after_header = total_height.saturating_sub(header_height);
        let available_after_header_footer =
            available_after_header.saturating_sub(session_footer_height);
        let prompt_empty = prompt.get_input().trim().is_empty();
        let viewport_height = if self.messages_viewport_height == 0 {
            usize::from(available_after_header_footer)
        } else {
            self.messages_viewport_height
        };
        let near_bottom =
            self.scroll_offset.saturating_add(viewport_height) >= self.rendered_line_count;
        let show_prompt = !prompt_empty || near_bottom;
        let prompt_height = if show_prompt {
            desired_prompt_height.min(available_after_header_footer)
        } else {
            0
        };

        let layout = if !show_prompt {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(header_height),
                    Constraint::Min(0),
                    Constraint::Length(session_footer_height.min(available_after_header)),
                    Constraint::Length(0),
                    Constraint::Length(0),
                ])
                .split(area)
        } else if available_after_header_footer <= prompt_height {
            // Small terminal fallback: keep stable bottom input behavior.
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(header_height),
                    Constraint::Min(0),
                    Constraint::Length(session_footer_height.min(available_after_header)),
                    Constraint::Length(prompt_height.min(available_after_header_footer)),
                    Constraint::Min(0),
                ])
                .split(area)
        } else {
            // Follow content: prompt sits directly below rendered messages.
            let max_messages_height = available_after_header_footer.saturating_sub(prompt_height);
            let desired_messages_height = (self.rendered_line_count as u16)
                .max(1)
                .min(max_messages_height);

            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(header_height),
                    Constraint::Length(desired_messages_height),
                    Constraint::Length(session_footer_height.min(available_after_header)),
                    Constraint::Length(prompt_height),
                    Constraint::Min(0),
                ])
                .split(area)
        };

        if show_header && layout[0].height > 0 {
            self.render_header(frame, layout[0]);
        }
        self.render_messages(frame, layout[1]);
        if layout[2].height > 0 {
            self.render_session_footer(frame, layout[2]);
        }
        if show_prompt && layout[3].height > 0 {
            prompt.render(frame, layout[3]);
        }
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let theme = self.context.theme.read();
        let session_ctx = self.context.session.read();
        let is_narrow = area.width < HEADER_NARROW_THRESHOLD;

        let title = session_ctx
            .sessions
            .get(&self.session_id)
            .map(|s| s.title.as_str())
            .unwrap_or("New Session");

        let messages = session_ctx
            .messages
            .get(&self.session_id)
            .cloned()
            .unwrap_or_default();

        // Find the last assistant message with output tokens > 0 for token display
        let last_assistant = messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant) && m.tokens.output > 0);

        // TS parity: cost only sums assistant messages.
        let total_cost: f64 = messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::Assistant))
            .map(|m| m.cost)
            .sum();
        let mut context_and_cost = None;
        if let Some(assistant_msg) = last_assistant {
            let t = &assistant_msg.tokens;
            let total_tokens = t.input + t.output + t.reasoning + t.cache_read + t.cache_write;
            if total_tokens > 0 {
                let model_context_limit = {
                    let providers = self.context.providers.read();
                    let current_model = self.context.current_model.read();
                    assistant_msg
                        .model
                        .as_ref()
                        .or(current_model.as_ref())
                        .and_then(|model_id| {
                            providers.iter().find_map(|p| {
                                p.models
                                    .iter()
                                    .find(|m| {
                                        m.id == *model_id
                                            || m.id
                                                .rsplit_once('/')
                                                .map(|(_, suffix)| suffix == model_id)
                                                .unwrap_or(false)
                                    })
                                    .map(|m| m.context_window)
                            })
                        })
                        .unwrap_or(0)
                };

                let mut context_text = format_number(total_tokens);
                if model_context_limit > 0 {
                    let pct =
                        ((total_tokens as f64 / model_context_limit as f64) * 100.0).round() as u64;
                    context_text.push_str(&format!(" {}%", pct));
                }

                let cost_text = format!("${:.2}", total_cost);
                context_and_cost = Some(format!("{} ({})", context_text, cost_text));
            }
        }

        let content = if is_narrow {
            let mut lines = vec![Line::from(vec![Span::styled(
                format!(" # {}", title),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            )])];
            if let Some(info) = context_and_cost {
                lines.push(Line::from(Span::styled(
                    format!("   {}", info),
                    Style::default().fg(theme.text_muted),
                )));
            }
            lines
        } else {
            let title_span = Span::styled(
                format!(" # {}", title),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            );

            let mut title_line_spans = vec![title_span];
            if let Some(right_text) = context_and_cost {
                let right_text_len = right_text.len();
                let available = area.width as usize;
                let title_display_len = title.len() + 3; // " # " prefix
                if available > title_display_len + right_text_len + 2 {
                    let padding = available.saturating_sub(title_display_len + right_text_len + 2);
                    title_line_spans.push(Span::raw(" ".repeat(padding)));
                    title_line_spans.push(Span::styled(
                        right_text,
                        Style::default().fg(theme.text_muted),
                    ));
                    title_line_spans.push(Span::raw(" "));
                }
            }
            vec![Line::from(title_line_spans)]
        };

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(theme.border)),
            )
            .style(Style::default().bg(theme.background_panel));

        frame.render_widget(paragraph, area);
    }

    fn render_session_footer(&self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let theme = self.context.theme.read();
        let directory = self.context.directory.read().clone();
        let mcp_servers = self.context.mcp_servers.read();
        let lsp_status = self.context.lsp_status.read();
        let permission_count = *self.context.pending_permissions.read();
        let has_connected_provider = *self.context.has_connected_provider.read();

        let connected_lsp = lsp_status
            .iter()
            .filter(|s| matches!(s.status, crate::context::LspConnectionStatus::Connected))
            .count();
        let connected_mcp = mcp_servers
            .iter()
            .filter(|s| matches!(s.status, crate::context::McpConnectionStatus::Connected))
            .count();
        let has_mcp_failures = mcp_servers
            .iter()
            .any(|s| matches!(s.status, crate::context::McpConnectionStatus::Failed));
        let has_mcp_registration_needed = mcp_servers.iter().any(|s| {
            matches!(
                s.status,
                crate::context::McpConnectionStatus::NeedsClientRegistration
            )
        });
        let has_mcp_issues = has_mcp_failures || has_mcp_registration_needed;
        let show_connect_hint =
            !has_connected_provider && Utc::now().timestamp().rem_euclid(15) >= 10;

        let mut right_spans = Vec::new();
        if show_connect_hint {
            right_spans.push(Span::styled(
                "Get started ",
                Style::default().fg(theme.text_muted),
            ));
            right_spans.push(Span::styled("/connect", Style::default().fg(theme.primary)));
        } else {
            if permission_count > 0 {
                right_spans.push(Span::styled(
                    format!(
                        "△ {} Permission{}",
                        permission_count,
                        if permission_count == 1 { "" } else { "s" }
                    ),
                    Style::default().fg(theme.warning),
                ));
                right_spans.push(Span::raw("  "));
            }

            right_spans.push(Span::styled(
                format!("• {} LSP", connected_lsp),
                Style::default().fg(if connected_lsp > 0 {
                    theme.success
                } else {
                    theme.text_muted
                }),
            ));
            right_spans.push(Span::raw("  "));

            if connected_mcp > 0 || has_mcp_issues {
                let mcp_color = if has_mcp_failures {
                    theme.error
                } else if has_mcp_registration_needed {
                    theme.warning
                } else {
                    theme.success
                };
                right_spans.push(Span::styled(
                    format!("⊙ {} MCP", connected_mcp),
                    Style::default().fg(mcp_color),
                ));
                right_spans.push(Span::raw("  "));
            }

            right_spans.push(Span::styled(
                "/status",
                Style::default().fg(theme.text_muted),
            ));
        }

        let right_text_len: usize = right_spans.iter().map(|s| s.content.len()).sum();
        let dir_len = directory.len();
        let available = area.width as usize;
        let mut line_spans = vec![Span::styled(
            directory,
            Style::default().fg(theme.text_muted),
        )];
        if available > dir_len + right_text_len + 1 {
            line_spans.push(Span::raw(" ".repeat(available - dir_len - right_text_len)));
        } else {
            line_spans.push(Span::raw(" "));
        }
        line_spans.extend(right_spans);

        let paragraph =
            Paragraph::new(Line::from(line_spans)).style(Style::default().bg(theme.background));
        frame.render_widget(paragraph, area);
    }

    fn render_messages(&mut self, frame: &mut Frame, area: Rect) {
        if area.height == 0 || area.width == 0 {
            self.last_messages_area = None;
            self.rendered_line_count = 0;
            self.messages_viewport_height = 0;
            self.line_to_message.clear();
            self.message_first_lines.clear();
            return;
        }

        let was_near_bottom = self.is_near_bottom(2);
        let theme = self.context.theme.read();
        let user_bg = message_palette::user_message_bg(&theme);
        let thinking_bg = message_palette::thinking_message_bg(&theme);
        let assistant_border = message_palette::assistant_border_color(&theme);
        let thinking_border = message_palette::thinking_border_color(&theme);
        let show_scrollbar = *self.context.show_scrollbar.read() && area.width > 3;
        let messages_area = if show_scrollbar {
            Rect {
                x: area.x,
                y: area.y,
                width: area.width.saturating_sub(1),
                height: area.height,
            }
        } else {
            area
        };
        let scrollbar_area = show_scrollbar.then_some(Rect {
            x: area.x + area.width.saturating_sub(1),
            y: area.y,
            width: 1,
            height: area.height,
        });
        let content_width = usize::from(messages_area.width.saturating_sub(1));
        let session_ctx = self.context.session.read();
        let show_thinking = *self.context.show_thinking.read();
        let show_timestamps = *self.context.show_timestamps.read();
        let show_tool_details = *self.context.show_tool_details.read();
        let semantic_hl = *self.context.semantic_highlight.read();
        let fallback_model = self.context.current_model.read().clone();

        let messages = session_ctx
            .messages
            .get(&self.session_id)
            .cloned()
            .unwrap_or_default();
        let revert_info = session_ctx.revert.get(&self.session_id).cloned();
        let last_assistant_idx = messages
            .iter()
            .rposition(|m| matches!(m.role, MessageRole::Assistant));

        let message_gap_lines = 1usize;

        self.last_messages_area = Some(messages_area);
        self.thinking_toggle_hits.clear();
        let mut visible_reasoning_ids = HashSet::new();

        let mut lines = Vec::new();
        let mut line_to_message: Vec<Option<String>> = Vec::new();
        let mut message_first_lines: HashMap<String, usize> = HashMap::new();

        if let Some(revert) = revert_info.as_ref() {
            let card_lines = super::revert_card::render_revert_card(revert, &theme);
            let painted = paint_block_lines(
                card_lines,
                theme.background_panel,
                theme.warning,
                content_width,
            );
            append_non_message_lines(&mut lines, &mut line_to_message, painted);
            if !messages.is_empty() {
                push_spacing_lines(&mut lines, &mut line_to_message, message_gap_lines);
            }
        }

        for (idx, msg) in messages.iter().enumerate() {
            // Smart spacing: role transitions always get a blank line;
            // cozy mode adds an extra blank line for breathing room
            if idx > 0 {
                let prev_role = &messages[idx - 1].role;
                if *prev_role != msg.role || matches!(msg.role, MessageRole::User) {
                    push_spacing_lines(&mut lines, &mut line_to_message, message_gap_lines);
                }
            }
            message_first_lines
                .entry(msg.id.clone())
                .or_insert(lines.len());

            match msg.role {
                MessageRole::User => {
                    let message_bg = user_bg;
                    let message_border = user_border_color_for_agent(msg.agent.as_deref(), &theme);
                    let user_lines = super::session_message::render_user_message(
                        msg,
                        &theme,
                        show_timestamps,
                        msg.agent.as_deref(),
                    );
                    append_message_lines(
                        &mut lines,
                        &mut line_to_message,
                        &msg.id,
                        paint_block_lines(user_lines, message_bg, message_border, content_width),
                    );
                }
                MessageRole::Assistant => {
                    let message_bg = theme.background;
                    let message_border = assistant_border;
                    let message_thinking_bg = thinking_bg;
                    let message_thinking_border = thinking_border;
                    let mut tool_results: HashMap<String, (String, bool)> = HashMap::new();
                    for part in &msg.parts {
                        if let MessagePart::ToolResult {
                            id,
                            result,
                            is_error,
                        } = part
                        {
                            tool_results.insert(id.clone(), (result.clone(), *is_error));
                        }
                    }
                    let is_active_assistant = last_assistant_idx == Some(idx)
                        && msg.finish.is_none()
                        && msg.error.is_none();
                    let assistant_marker = assistant_marker_color(msg.agent.as_deref(), &theme);
                    let unresolved_tool_calls = msg
                        .parts
                        .iter()
                        .filter_map(|part| match part {
                            MessagePart::ToolCall { id, .. } if !tool_results.contains_key(id) => {
                                Some(id.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    let running_tool_call = if is_active_assistant {
                        unresolved_tool_calls.first().copied()
                    } else {
                        None
                    };

                    if msg.parts.is_empty() {
                        let mut text_lines =
                            super::session_text::render_text_part(&msg.content, &theme, assistant_marker);
                        if semantic_hl {
                            text_lines =
                                super::semantic_highlight::highlight_lines(text_lines, &theme);
                        }
                        append_message_lines(
                            &mut lines,
                            &mut line_to_message,
                            &msg.id,
                            paint_block_lines(
                                text_lines,
                                message_bg,
                                message_border,
                                content_width,
                            ),
                        );
                    } else {
                        let mut prev_was_text = false;
                        let mut prev_was_tool = false;
                        for (part_idx, part) in msg.parts.iter().enumerate() {
                            match part {
                                MessagePart::Text { text } => {
                                    // Add margin when transitioning from tool back to text
                                    if prev_was_tool {
                                        append_message_lines(
                                            &mut lines,
                                            &mut line_to_message,
                                            &msg.id,
                                            vec![Line::from("")],
                                        );
                                    }
                                    let mut text_lines =
                                        super::session_text::render_text_part(
                                            text,
                                            &theme,
                                            assistant_marker,
                                        );
                                    if semantic_hl {
                                        text_lines = super::semantic_highlight::highlight_lines(
                                            text_lines, &theme,
                                        );
                                    }
                                    append_message_lines(
                                        &mut lines,
                                        &mut line_to_message,
                                        &msg.id,
                                        paint_block_lines(
                                            text_lines,
                                            message_bg,
                                            message_border,
                                            content_width,
                                        ),
                                    );
                                    prev_was_text = true;
                                    prev_was_tool = false;
                                }
                                MessagePart::Reasoning { text } => {
                                    if show_thinking {
                                        if prev_was_text || prev_was_tool {
                                            append_message_lines(
                                                &mut lines,
                                                &mut line_to_message,
                                                &msg.id,
                                                vec![Line::from("")],
                                            );
                                        }
                                        let reasoning_id = format!("{}:{part_idx}", msg.id);
                                        let collapsed =
                                            !self.expanded_reasoning.contains(&reasoning_id);
                                        let start_line = lines.len();
                                        let rendered = super::session_text::render_reasoning_part(
                                            text,
                                            &theme,
                                            collapsed,
                                            THINKING_PREVIEW_LINES,
                                        );
                                        if !rendered.lines.is_empty() {
                                            let painted = paint_block_lines(
                                                rendered.lines,
                                                message_thinking_bg,
                                                message_thinking_border,
                                                content_width,
                                            );
                                            append_message_lines(
                                                &mut lines,
                                                &mut line_to_message,
                                                &msg.id,
                                                painted,
                                            );
                                            if rendered.collapsible {
                                                let end_line = lines.len().saturating_sub(1);
                                                visible_reasoning_ids.insert(reasoning_id.clone());
                                                self.thinking_toggle_hits.push(ThinkingToggleHit {
                                                    line_index: start_line,
                                                    reasoning_id: reasoning_id.clone(),
                                                });
                                                if end_line > start_line {
                                                    self.thinking_toggle_hits.push(
                                                        ThinkingToggleHit {
                                                            line_index: end_line,
                                                            reasoning_id,
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                        prev_was_text = false;
                                        prev_was_tool = false;
                                    }
                                }
                                MessagePart::ToolCall {
                                    id,
                                    name,
                                    arguments,
                                } => {
                                    // Add margin when transitioning from text to tool
                                    if prev_was_text {
                                        append_message_lines(
                                            &mut lines,
                                            &mut line_to_message,
                                            &msg.id,
                                            vec![Line::from("")],
                                        );
                                    }
                                    let state = if let Some((_, is_error)) = tool_results.get(id) {
                                        if *is_error {
                                            super::session_tool::ToolState::Failed
                                        } else {
                                            super::session_tool::ToolState::Completed
                                        }
                                    } else if running_tool_call == Some(id.as_str()) {
                                        super::session_tool::ToolState::Running
                                    } else {
                                        super::session_tool::ToolState::Pending
                                    };
                                    let tool_lines = super::session_tool::render_tool_call(
                                        id,
                                        name,
                                        arguments,
                                        state,
                                        &tool_results,
                                        show_tool_details,
                                        &theme,
                                    );
                                    append_message_lines(
                                        &mut lines,
                                        &mut line_to_message,
                                        &msg.id,
                                        tool_lines,
                                    );
                                    prev_was_text = false;
                                    prev_was_tool = true;
                                }
                                MessagePart::ToolResult { .. } => {}
                                MessagePart::File { path, mime } => {
                                    let file_line = Line::from(vec![
                                        Span::styled(
                                            "▸ ",
                                            Style::default().fg(assistant_marker),
                                        ),
                                        Span::styled("[file] ", Style::default().fg(theme.info)),
                                        Span::styled(path.clone(), Style::default().fg(theme.text)),
                                        Span::styled(
                                            format!(" ({})", mime),
                                            Style::default().fg(theme.text_muted),
                                        ),
                                    ]);
                                    append_message_lines(
                                        &mut lines,
                                        &mut line_to_message,
                                        &msg.id,
                                        paint_block_lines(
                                            vec![file_line],
                                            message_bg,
                                            message_border,
                                            content_width,
                                        ),
                                    );
                                }
                                MessagePart::Image { url } => {
                                    let image_line = Line::from(vec![
                                        Span::styled(
                                            "▸ ",
                                            Style::default().fg(assistant_marker),
                                        ),
                                        Span::styled("[image] ", Style::default().fg(theme.info)),
                                        Span::styled(
                                            url.clone(),
                                            Style::default().fg(theme.text_muted),
                                        ),
                                    ]);
                                    append_message_lines(
                                        &mut lines,
                                        &mut line_to_message,
                                        &msg.id,
                                        paint_block_lines(
                                            vec![image_line],
                                            message_bg,
                                            message_border,
                                            content_width,
                                        ),
                                    );
                                }
                            }
                        }
                    }

                    if let Some(footer) = assistant_footer(
                        &messages,
                        idx,
                        last_assistant_idx,
                        msg,
                        fallback_model.as_deref(),
                        &theme,
                    ) {
                        append_message_lines(
                            &mut lines,
                            &mut line_to_message,
                            &msg.id,
                            paint_block_lines(
                                vec![footer],
                                message_bg,
                                message_border,
                                content_width,
                            ),
                        );
                    }
                }
                MessageRole::System => {
                    let system_lines: Vec<Line<'static>> = msg
                        .content
                        .lines()
                        .map(|line_text| {
                            Line::from(Span::styled(
                                line_text.to_string(),
                                Style::default().fg(theme.text_muted),
                            ))
                        })
                        .collect();
                    append_message_lines(&mut lines, &mut line_to_message, &msg.id, system_lines);
                }
            }
        }

        self.expanded_reasoning
            .retain(|id| visible_reasoning_ids.contains(id));
        self.line_to_message = line_to_message;
        self.message_first_lines = message_first_lines;

        self.rendered_line_count = lines.len();
        self.messages_viewport_height = usize::from(messages_area.height);
        let max_scroll = self.max_scroll_offset();
        if was_near_bottom {
            self.scroll_offset = max_scroll;
        } else if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(theme.border)),
            )
            .scroll((self.scroll_offset as u16, 0));

        frame.render_widget(paragraph, messages_area);
        if let Some(scroll_area) = scrollbar_area {
            let mut scrollbar_state = ScrollbarState::new(self.rendered_line_count)
                .position(self.scroll_offset)
                .viewport_content_length(self.messages_viewport_height.max(1));
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(Style::default().fg(theme.border_subtle))
                .thumb_symbol("█")
                .thumb_style(Style::default().fg(theme.primary));
            frame.render_stateful_widget(scrollbar, scroll_area, &mut scrollbar_state);
        }
    }

    pub fn handle_click(&mut self, col: u16, row: u16) -> bool {
        let Some(area) = self.last_messages_area else {
            return false;
        };

        let max_x = area.x.saturating_add(area.width);
        let max_y = area.y.saturating_add(area.height);
        if col < area.x || col >= max_x || row < area.y || row >= max_y {
            return false;
        }

        let line_index = self.scroll_offset + usize::from(row.saturating_sub(area.y));
        if line_index >= self.rendered_line_count {
            return false;
        }
        let Some(reasoning_id) = self
            .thinking_toggle_hits
            .iter()
            .find(|hit| hit.line_index == line_index)
            .map(|hit| hit.reasoning_id.clone())
        else {
            return false;
        };

        if !self.expanded_reasoning.insert(reasoning_id.clone()) {
            self.expanded_reasoning.remove(&reasoning_id);
        }
        true
    }

    pub fn handle_sidebar_click(&mut self, col: u16, row: u16) -> bool {
        if point_in_optional_rect(self.sidebar_open_button_area, col, row) {
            *self.context.show_sidebar.write() = true;
            self.sidebar_open_button_area = None;
            return true;
        }
        if point_in_optional_rect(self.sidebar_close_button_area, col, row) {
            *self.context.show_sidebar.write() = false;
            self.sidebar_close_button_area = None;
            return true;
        }
        self.sidebar_state.handle_click(col, row)
    }

    pub fn is_point_in_sidebar(&self, col: u16, row: u16) -> bool {
        self.sidebar_state.contains_sidebar_point(col, row)
    }

    pub fn scroll_sidebar_up_at(&mut self, col: u16, row: u16) -> bool {
        self.sidebar_state.scroll_up_at(col, row)
    }

    pub fn scroll_sidebar_down_at(&mut self, col: u16, row: u16) -> bool {
        self.sidebar_state.scroll_down_at(col, row)
    }

    pub fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    pub fn scroll_down(&mut self) {
        let max_scroll = self.max_scroll_offset();
        if self.scroll_offset < max_scroll {
            self.scroll_offset += 1;
        }
    }

    pub fn scroll_up_by(&mut self, lines: usize) {
        let lines = lines.max(1);
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_down_by(&mut self, lines: usize) {
        let lines = lines.max(1);
        let max_scroll = self.max_scroll_offset();
        self.scroll_offset = self.scroll_offset.saturating_add(lines).min(max_scroll);
    }

    pub fn scroll_up_mouse(&mut self) {
        self.scroll_up_by(MOUSE_SCROLL_LINES);
    }

    pub fn scroll_down_mouse(&mut self) {
        self.scroll_down_by(MOUSE_SCROLL_LINES);
    }

    pub fn scroll_page_up(&mut self) {
        let step = self.messages_viewport_height.saturating_sub(1).max(1);
        self.scroll_offset = self.scroll_offset.saturating_sub(step);
    }

    pub fn scroll_page_down(&mut self) {
        let step = self.messages_viewport_height.saturating_sub(1).max(1);
        let max_scroll = self.max_scroll_offset();
        self.scroll_offset = (self.scroll_offset + step).min(max_scroll);
    }

    pub fn scroll_to_message(&mut self, message_id: &str) {
        if let Some(first_line) = self.message_first_lines.get(message_id).copied() {
            self.scroll_offset = first_line.min(self.max_scroll_offset());
            return;
        }

        let session_ctx = self.context.session.read();
        if let Some(messages) = session_ctx.messages.get(&self.session_id) {
            if let Some(idx) = messages.iter().position(|m| m.id == message_id) {
                // Approximate: each message takes ~3 lines, scroll to that position
                self.scroll_offset = idx.saturating_mul(3);
                let max_scroll = self.max_scroll_offset();
                if self.scroll_offset > max_scroll {
                    self.scroll_offset = max_scroll;
                }
            }
        }
    }

    fn max_scroll_offset(&self) -> usize {
        self.rendered_line_count
            .saturating_sub(self.messages_viewport_height)
    }

    fn is_near_bottom(&self, tolerance_lines: usize) -> bool {
        self.max_scroll_offset().saturating_sub(self.scroll_offset) <= tolerance_lines
    }
}

fn push_spacing_lines(
    lines: &mut Vec<Line<'static>>,
    line_to_message: &mut Vec<Option<String>>,
    count: usize,
) {
    for _ in 0..count {
        lines.push(Line::from(""));
        line_to_message.push(None);
    }
}

fn append_message_lines(
    lines: &mut Vec<Line<'static>>,
    line_to_message: &mut Vec<Option<String>>,
    message_id: &str,
    new_lines: Vec<Line<'static>>,
) {
    if new_lines.is_empty() {
        return;
    }
    let marker = Some(message_id.to_string());
    for _ in 0..new_lines.len() {
        line_to_message.push(marker.clone());
    }
    lines.extend(new_lines);
}

fn append_non_message_lines(
    lines: &mut Vec<Line<'static>>,
    line_to_message: &mut Vec<Option<String>>,
    new_lines: Vec<Line<'static>>,
) {
    if new_lines.is_empty() {
        return;
    }
    for _ in 0..new_lines.len() {
        line_to_message.push(None);
    }
    lines.extend(new_lines);
}

fn paint_block_lines(
    lines: Vec<Line<'static>>,
    background: Color,
    border_color: Color,
    width: usize,
) -> Vec<Line<'static>> {
    let painted: Vec<Line<'static>> = lines
        .into_iter()
        .flat_map(|line| wrap_block_line(line, width))
        .map(|line| paint_block_line(line, background, border_color, width))
        .collect();

    if painted.is_empty() {
        return painted;
    }

    let gutter = painted
        .first()
        .and_then(|line| line.spans.first())
        .map(|span| span.content.to_string())
        .filter(|value| is_gutter_span(value.as_str()));

    let padding_line = if let Some(gutter) = gutter {
        paint_block_line(
            Line::from(vec![Span::raw(gutter)]),
            background,
            border_color,
            width,
        )
    } else {
        paint_block_line(Line::from(""), background, border_color, width)
    };

    let mut padded = Vec::with_capacity(painted.len() + 2);
    padded.push(padding_line.clone());
    padded.extend(painted);
    padded.push(padding_line);
    padded
}

fn paint_block_line(
    line: Line<'static>,
    background: Color,
    border_color: Color,
    width: usize,
) -> Line<'static> {
    let mut styled = Vec::with_capacity(line.spans.len() + 1);
    let mut rendered_width = 0usize;

    for (idx, span) in line.spans.into_iter().enumerate() {
        rendered_width += UnicodeWidthStr::width(span.content.as_ref());
        let style = if idx == 0 && is_gutter_span(span.content.as_ref()) {
            span.style.fg(border_color).bg(background)
        } else {
            span.style.bg(background)
        };
        styled.push(Span::styled(span.content, style));
    }

    if rendered_width < width {
        styled.push(Span::styled(
            " ".repeat(width - rendered_width),
            Style::default().bg(background),
        ));
    }

    Line::from(styled)
}

fn wrap_block_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 || line.spans.is_empty() {
        return vec![line];
    }

    let mut iter = line.spans.into_iter();
    let Some(gutter) = iter.next() else {
        return vec![Line::from("")];
    };
    if !is_gutter_span(gutter.content.as_ref()) {
        let mut all_spans = vec![gutter];
        all_spans.extend(iter);
        let wrapped = wrap_spans(all_spans, width);
        return wrapped.into_iter().map(Line::from).collect();
    }

    let body_spans: Vec<Span<'static>> = iter.collect();
    let gutter_width = UnicodeWidthStr::width(gutter.content.as_ref());
    if gutter_width >= width {
        return vec![Line::from(vec![gutter])];
    }

    let body_width = width
        .saturating_sub(gutter_width)
        .saturating_sub(MESSAGE_BLOCK_RIGHT_PADDING);
    if body_width == 0 {
        return vec![Line::from(vec![gutter])];
    }
    let wrapped_body = wrap_spans(body_spans, body_width);
    wrapped_body
        .into_iter()
        .map(|body| {
            let mut spans = Vec::with_capacity(body.len() + 1);
            spans.push(gutter.clone());
            spans.extend(body);
            Line::from(spans)
        })
        .collect()
}

fn is_gutter_span(content: &str) -> bool {
    let mut has_border = false;
    for ch in content.chars() {
        match ch {
            '│' | '┃' => has_border = true,
            ' ' => {}
            _ => return false,
        }
    }
    has_border
}

fn point_in_optional_rect(area: Option<Rect>, col: u16, row: u16) -> bool {
    let Some(area) = area else {
        return false;
    };
    let max_x = area.x.saturating_add(area.width);
    let max_y = area.y.saturating_add(area.height);
    col >= area.x && col < max_x && row >= area.y && row < max_y
}

fn tint_sidebar_overlay(background: Color, accent: Color) -> Color {
    match (background, accent) {
        (Color::Rgb(br, bg, bb), Color::Rgb(ar, ag, ab)) => {
            // 80% base + 20% accent: colored but still subtle.
            let blend = |b: u8, a: u8| -> u8 { ((u16::from(b) * 4 + u16::from(a)) / 5) as u8 };
            Color::Rgb(blend(br, ar), blend(bg, ag), blend(bb, ab))
        }
        _ => background,
    }
}

fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![spans];
    }

    let mut out: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut current_width = 0usize;

    for span in spans {
        let style = span.style;
        for ch in span.content.chars() {
            if ch == '\n' {
                out.push(Vec::new());
                current_width = 0;
                continue;
            }

            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + ch_width > width && !out.last().is_some_and(|line| line.is_empty()) {
                out.push(Vec::new());
                current_width = 0;
            }

            push_merged_span(out.last_mut().expect("line exists"), ch, style);
            current_width += ch_width;
        }
    }

    if out.is_empty() {
        out.push(Vec::new());
    }
    out
}

fn push_merged_span(line: &mut Vec<Span<'static>>, ch: char, style: Style) {
    if let Some(last) = line.last_mut() {
        if last.style == style {
            last.content.to_mut().push(ch);
            return;
        }
    }

    line.push(Span::styled(ch.to_string(), style));
}

fn assistant_footer(
    messages: &[Message],
    idx: usize,
    last_assistant_idx: Option<usize>,
    message: &Message,
    fallback_model: Option<&str>,
    theme: &crate::theme::Theme,
) -> Option<Line<'static>> {
    if !matches!(message.role, MessageRole::Assistant) {
        return None;
    }

    let is_last_assistant = last_assistant_idx == Some(idx);
    let is_interrupted = is_assistant_interrupted(message);
    let is_final = is_assistant_final(message);

    if !is_last_assistant && !is_final && !is_interrupted {
        return None;
    }

    let mode = message.mode.as_deref().unwrap_or("default");
    let mut spans = vec![
        Span::styled(
            "▣ ",
            Style::default().fg(if is_interrupted {
                theme.text_muted
            } else {
                assistant_marker_color(message.agent.as_deref(), theme)
            }),
        ),
        Span::styled(titlecase(mode), Style::default().fg(theme.text)),
    ];

    if let Some(model) = message
        .model
        .as_deref()
        .or(fallback_model)
        .filter(|value| !value.trim().is_empty())
    {
        spans.push(Span::styled(" · ", Style::default().fg(theme.text_muted)));
        spans.push(Span::styled(
            model.to_string(),
            Style::default().fg(theme.text_muted),
        ));
    }

    if let Some(duration) = assistant_duration(messages, idx, message, is_final) {
        spans.push(Span::styled(" · ", Style::default().fg(theme.text_muted)));
        spans.push(Span::styled(
            duration,
            Style::default().fg(theme.text_muted),
        ));
    }

    if is_interrupted {
        spans.push(Span::styled(
            " · interrupted",
            Style::default().fg(theme.text_muted),
        ));
    }

    Some(Line::from(spans))
}

fn assistant_duration(
    messages: &[Message],
    idx: usize,
    message: &Message,
    is_final: bool,
) -> Option<String> {
    if !is_final {
        return None;
    }
    let user_start = messages[..idx]
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::User))
        .map(|m| m.created_at.timestamp_millis())?;
    let end = message.completed_at?.timestamp_millis();
    if end <= user_start {
        return None;
    }

    let elapsed = (end - user_start) as u64;
    if elapsed < 1_000 {
        Some(format!("{elapsed}ms"))
    } else {
        Some(format!("{:.1}s", elapsed as f64 / 1_000.0))
    }
}

fn is_assistant_final(message: &Message) -> bool {
    matches!(
        message.finish.as_deref(),
        Some(reason) if reason != "tool-calls" && reason != "unknown"
    )
}

fn is_assistant_interrupted(message: &Message) -> bool {
    if message.finish.as_deref() == Some("abort") {
        return true;
    }
    message
        .error
        .as_deref()
        .map(|err| {
            let lower = err.to_ascii_lowercase();
            lower.contains("messageabortederror")
                || lower.contains("abortederror")
                || lower.contains("abort")
        })
        .unwrap_or(false)
}

fn assistant_marker_color(agent: Option<&str>, theme: &crate::theme::Theme) -> Color {
    let Some(agent) = agent else {
        return theme.primary;
    };
    let mut hasher = DefaultHasher::new();
    agent.hash(&mut hasher);
    theme.agent_color(hasher.finish() as usize)
}

fn user_border_color_for_agent(agent: Option<&str>, theme: &crate::theme::Theme) -> Color {
    let Some(agent) = agent else {
        return theme.primary;
    };
    let mut hasher = DefaultHasher::new();
    agent.hash(&mut hasher);
    theme.agent_color(hasher.finish() as usize)
}

fn titlecase(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn format_number(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + (digits.len() / 3));
    for (idx, ch) in digits.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}
