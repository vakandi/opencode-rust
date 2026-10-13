use std::cell::Cell;

use ratatui::prelude::Stylize;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug, PartialEq)]
pub enum QuestionType {
    Text,
    MultipleChoice,
    SingleChoice,
}

#[derive(Clone, Debug)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct QuestionRequest {
    pub id: String,
    pub question: String,
    pub question_type: QuestionType,
    pub options: Vec<QuestionOption>,
}

pub struct QuestionPrompt {
    current_question: Option<QuestionRequest>,
    pub is_open: bool,
    selected_index: usize,
    selected_options: Vec<bool>,
    text_input: String,
    last_rendered_area: Cell<Option<Rect>>,
}

impl QuestionPrompt {
    pub fn new() -> Self {
        Self {
            current_question: None,
            is_open: false,
            selected_index: 0,
            selected_options: Vec::new(),
            text_input: String::new(),
            last_rendered_area: Cell::new(None),
        }
    }

    pub fn ask(&mut self, question: QuestionRequest) {
        let option_count = question.options.len();
        self.current_question = Some(question);
        self.is_open = true;
        self.selected_index = 0;
        self.selected_options = vec![false; option_count];
        self.text_input.clear();
    }

    pub fn current(&self) -> Option<&QuestionRequest> {
        self.current_question.as_ref()
    }

    pub fn close(&mut self) {
        self.current_question = None;
        self.is_open = false;
        self.selected_index = 0;
        self.selected_options.clear();
        self.text_input.clear();
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if let Some(q) = &self.current_question {
            if !q.options.is_empty() && self.selected_index + 1 < q.options.len() {
                self.selected_index += 1;
            }
        }
    }

    pub fn toggle_selected(&mut self) {
        if let Some(q) = &self.current_question {
            if q.question_type == QuestionType::MultipleChoice
                && self.selected_index < self.selected_options.len()
            {
                self.selected_options[self.selected_index] =
                    !self.selected_options[self.selected_index];
            } else if q.question_type == QuestionType::SingleChoice {
                // For single choice, clear all and select current
                for opt in self.selected_options.iter_mut() {
                    *opt = false;
                }
                if self.selected_index < self.selected_options.len() {
                    self.selected_options[self.selected_index] = true;
                }
            }
        }
    }

    pub fn type_char(&mut self, c: char) {
        if let Some(q) = &self.current_question {
            if q.question_type == QuestionType::Text {
                self.text_input.push(c);
            } else if !q.options.is_empty() {
                // Letter key selects an option by index
                let idx = (c as u8).wrapping_sub(b'a') as usize;
                if idx < q.options.len() {
                    if q.question_type == QuestionType::SingleChoice {
                        for opt in self.selected_options.iter_mut() {
                            *opt = false;
                        }
                    }
                    if idx < self.selected_options.len() {
                        self.selected_options[idx] = !self.selected_options[idx];
                    }
                    self.selected_index = idx;
                }
            }
        }
    }

    pub fn backspace(&mut self) {
        if let Some(q) = &self.current_question {
            if q.question_type == QuestionType::Text {
                self.text_input.pop();
            }
        }
    }

    pub fn confirm(&mut self) -> Option<(QuestionRequest, String)> {
        let q = self.current_question.as_ref()?;
        let answer = if q.question_type == QuestionType::Text {
            self.text_input.clone()
        } else {
            // Collect selected option IDs
            q.options
                .iter()
                .enumerate()
                .filter(|(i, _)| self.selected_options.get(*i).copied().unwrap_or(false))
                .map(|(_, opt)| opt.id.clone())
                .collect::<Vec<_>>()
                .join(",")
        };
        let request = self.current_question.take()?;
        self.is_open = false;
        self.selected_index = 0;
        self.selected_options.clear();
        self.text_input.clear();
        Some((request, answer))
    }

    pub fn handle_click(&mut self, col: u16, row: u16) {
        if !self.is_open {
            return;
        }
        if let Some(area) = self.last_rendered_area.get() {
            if row < area.y
                || row >= area.y + area.height
                || col < area.x
                || col >= area.x + area.width
            {
                return;
            }
            // Options start at row area.y + 5 (border + title + blank + question + blank)
            let options_start = area.y + 5;
            if row >= options_start {
                let idx = (row - options_start) as usize;
                if let Some(q) = &self.current_question {
                    if idx < q.options.len() {
                        self.selected_index = idx;
                        self.toggle_selected();
                    }
                }
            }
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.is_open {
            return;
        }

        let question = match &self.current_question {
            Some(q) => q,
            None => return,
        };

        let mut content = vec![
            Line::from(Span::styled(
                "Question:",
                Style::default().fg(theme.primary).bold(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                &question.question,
                Style::default().fg(theme.text),
            )),
            Line::from(""),
        ];

        if !question.options.is_empty() {
            for (i, opt) in question.options.iter().enumerate() {
                let is_selected = self.selected_options.get(i).copied().unwrap_or(false);
                let is_highlighted = i == self.selected_index;
                let marker = if is_selected { "[x]" } else { "[ ]" };
                let key = (b'a' + i as u8) as char;
                let label_style = if is_highlighted {
                    Style::default().fg(theme.primary).bold()
                } else {
                    Style::default().fg(theme.text)
                };
                content.push(Line::from(vec![
                    Span::styled(
                        format!("{} ({}) ", marker, key),
                        Style::default().fg(theme.primary),
                    ),
                    Span::styled(&opt.label, label_style),
                ]));
            }
            content.push(Line::from(""));
            content.push(Line::from(Span::styled(
                "Up/Down to navigate, Space to toggle, Enter to confirm",
                Style::default().fg(theme.text_muted),
            )));
        } else {
            // Text input mode
            let input_display = if self.text_input.is_empty() {
                "Type your answer...".to_string()
            } else {
                format!("> {}", self.text_input)
            };
            let input_style = if self.text_input.is_empty() {
                Style::default().fg(theme.text_muted)
            } else {
                Style::default().fg(theme.text)
            };
            content.push(Line::from(Span::styled(input_display, input_style)));
            content.push(Line::from(""));
            content.push(Line::from(Span::styled(
                "Type your answer and press Enter",
                Style::default().fg(theme.text_muted),
            )));
        }

        let height = (content.len() as u16 + 2).min(area.height.saturating_sub(2));
        let width = area.width.saturating_sub(2).min(80);

        // Render inline at the bottom of the area
        let popup_area = Rect::new(
            area.x + 1,
            area.y + area.height.saturating_sub(height + 1),
            width,
            height,
        );

        // Track rendered area for click handling
        self.last_rendered_area.set(Some(popup_area));

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .title(" Question ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.primary)),
            )
            .style(Style::default().bg(theme.background_panel));

        frame.render_widget(paragraph, popup_area);
    }
}

impl Default for QuestionPrompt {
    fn default() -> Self {
        Self::new()
    }
}
