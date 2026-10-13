use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::theme::Theme;

#[derive(Clone, Debug)]
pub struct ThemeOption {
    pub id: String,
    pub name: String,
}

pub struct ThemeListDialog {
    options: Vec<ThemeOption>,
    filtered: Vec<usize>,
    query: String,
    initial_theme: String,
    state: ListState,
    open: bool,
}

impl ThemeListDialog {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            options: Vec::new(),
            filtered: Vec::new(),
            query: String::new(),
            initial_theme: "opencode@dark".to_string(),
            state,
            open: false,
        }
    }

    pub fn set_options(&mut self, mut options: Vec<ThemeOption>) {
        options.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.options = options;
        self.filter();
    }

    pub fn open(&mut self, current_theme: &str) {
        self.open = true;
        self.initial_theme = canonical_theme_id(current_theme);
        self.query.clear();
        self.filter();
        let selected = self
            .filtered
            .iter()
            .position(|idx| {
                self.options
                    .get(*idx)
                    .is_some_and(|item| item.id == self.initial_theme)
            })
            .unwrap_or(0);
        self.state.select(if self.filtered.is_empty() {
            None
        } else {
            Some(selected)
        });
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn initial_theme_id(&self) -> &str {
        &self.initial_theme
    }

    pub fn move_up(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected > 0 {
                self.state.select(Some(selected - 1));
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected < self.filtered.len().saturating_sub(1) {
                self.state.select(Some(selected + 1));
            }
        }
    }

    pub fn handle_input(&mut self, c: char) {
        self.query.push(c);
        self.filter();
    }

    pub fn handle_backspace(&mut self) {
        self.query.pop();
        self.filter();
    }

    pub fn selected_theme_id(&self) -> Option<String> {
        self.state
            .selected()
            .and_then(|idx| self.filtered.get(idx))
            .and_then(|idx| self.options.get(*idx))
            .map(|theme| theme.id.clone())
    }

    fn filter(&mut self) {
        let query = self.query.to_lowercase();
        self.filtered = self
            .options
            .iter()
            .enumerate()
            .filter(|(_, option)| option.name.to_lowercase().contains(&query))
            .map(|(idx, _)| idx)
            .collect();
        self.state.select(if self.filtered.is_empty() {
            None
        } else {
            Some(0)
        });
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_area = centered_rect(58, 16, area);
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Themes ",
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
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(theme.primary)),
                Span::styled(&self.query, Style::default().fg(theme.text)),
                Span::styled("▏", Style::default().fg(theme.primary)),
            ])),
            layout[0],
        );

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .filter_map(|idx| self.options.get(*idx))
            .map(|item| {
                let mut spans = vec![Span::styled(&item.name, Style::default().fg(theme.text))];
                if item.id == self.initial_theme {
                    spans.push(Span::styled(
                        "  (current)",
                        Style::default().fg(theme.text_muted),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let list = List::new(items).highlight_style(
            Style::default()
                .bg(theme.background_element)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_stateful_widget(list, layout[1], &mut self.state.clone());

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Enter", Style::default().fg(theme.primary)),
                Span::styled(" apply  ", Style::default().fg(theme.text_muted)),
                Span::styled("Esc", Style::default().fg(theme.primary)),
                Span::styled(" cancel (revert)", Style::default().fg(theme.text_muted)),
            ])),
            layout[2],
        );
    }
}

impl Default for ThemeListDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}


fn canonical_theme_id(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "opencode@dark".to_string();
    }

    if let Some((base, variant)) = split_theme_variant(trimmed) {
        return format!("{base}@{variant}");
    }

    if trimmed.eq_ignore_ascii_case("dark") {
        return "opencode@dark".to_string();
    }
    if trimmed.eq_ignore_ascii_case("light") {
        return "opencode@light".to_string();
    }

    format!("{trimmed}@dark")
}

fn split_theme_variant(name: &str) -> Option<(&str, &str)> {
    let (base, variant) = name.rsplit_once('@').or_else(|| name.rsplit_once(':'))?;
    if base.is_empty() || !matches!(variant, "dark" | "light") {
        return None;
    }
    Some((base, variant))
}
