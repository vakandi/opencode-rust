use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::command::{CommandAction, CommandRegistry};
use crate::theme::Theme;

pub struct SlashCommandPopup {
    pub registry: CommandRegistry,
    pub query: String,
    pub filtered: Vec<String>,
    pub state: ListState,
    pub open: bool,
    pub selected_action: Option<CommandAction>,
}

impl SlashCommandPopup {
    pub fn new() -> Self {
        Self {
            registry: CommandRegistry::new(),
            query: String::new(),
            filtered: Vec::new(),
            state: ListState::default(),
            open: false,
            selected_action: None,
        }
    }

    pub fn open(&mut self) {
        self.query = String::new();
        self.refresh_filter();
        self.state.select(Some(0));
        self.open = true;
        self.selected_action = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.filtered.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn take_action(&mut self) -> Option<CommandAction> {
        self.selected_action.take()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    fn refresh_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = self
                .registry
                .suggested_commands()
                .iter()
                .map(|cmd| cmd.name.clone())
                .collect();
        } else {
            self.filtered = self
                .registry
                .search(&self.query)
                .iter()
                .map(|cmd| cmd.name.clone())
                .collect();
        }
        self.state.select(Some(0));
    }

    pub fn handle_input(&mut self, c: char) {
        self.query.push(c);
        self.refresh_filter();
    }

    pub fn handle_backspace(&mut self) -> bool {
        if self.query.pop().is_some() {
            self.refresh_filter();
            true
        } else {
            false
        }
    }

    pub fn move_up(&mut self) {
        if let Some(selected) = self.state.selected() {
            let new = selected.saturating_sub(1);
            self.state.select(Some(new));
        }
    }

    pub fn move_down(&mut self) {
        if let Some(selected) = self.state.selected() {
            let new = (selected + 1).min(self.filtered.len().saturating_sub(1));
            self.state.select(Some(new));
        }
    }

    pub fn select_current(&mut self) {
        if let Some(idx) = self.state.selected() {
            if let Some(name) = self.filtered.get(idx) {
                if let Some(cmd) = self.registry.get(name) {
                    self.selected_action = Some(cmd.action.clone());
                    self.close();
                }
            }
        }
    }

    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        prompt_area: Option<Rect>,
        theme: &Theme,
    ) {
        if !self.open {
            return;
        }

        let visible_rows = self.filtered.len().clamp(1, POPUP_MAX_VISIBLE_ROWS) as u16;
        let popup_area = popup_rect(area, prompt_area, visible_rows + 2);
        if popup_area.width < 4 || popup_area.height < 3 {
            return;
        }

        let selected = self.state.selected().unwrap_or(0);
        let selected_cmd = self
            .filtered
            .get(selected)
            .and_then(|name| self.registry.get(name));
        let footer = match selected_cmd {
            Some(cmd) if !cmd.description.trim().is_empty() => cmd.description.clone(),
            _ => format!("{} of {}", selected + 1, self.filtered.len()),
        };

        let block = Block::default()
            .title(Line::from(vec![
                Span::raw("/"),
                Span::styled(self.query.clone(), Style::default().fg(theme.primary)),
            ]))
            .title_bottom(Line::from(Span::styled(
                footer,
                Style::default().fg(theme.text_muted),
            )))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));

        frame.render_widget(Clear, popup_area);
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        if self.filtered.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "No matching commands",
                    Style::default().fg(theme.text_muted),
                ))),
                inner,
            );
            return;
        }

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .enumerate()
            .map(|(idx, name)| {
                let cmd = self.registry.get(name);
                let title = cmd.map(|c| c.title.as_str()).unwrap_or(name);
                let keybind = cmd.and_then(|c| c.keybind.clone());

                let style = if idx == selected {
                    Style::default()
                        .fg(theme.primary)
                        .bg(theme.background_element)
                } else {
                    Style::default().fg(theme.text)
                };

                let mut spans = vec![
                    Span::styled(name.clone(), style),
                    Span::styled(format!("  {}", title), style),
                ];
                if let Some(kb) = keybind {
                    spans.push(Span::styled(
                        format!("  ({})", kb),
                        Style::default().fg(theme.text_muted),
                    ));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let mut state = self.state.clone();
        frame.render_stateful_widget(List::new(items), inner, &mut state);
    }
}

const POPUP_MAX_WIDTH: u16 = 64;
const POPUP_MIN_WIDTH: u16 = 24;
const POPUP_MAX_VISIBLE_ROWS: usize = 8;

fn popup_rect(area: Rect, prompt_area: Option<Rect>, height: u16) -> Rect {
    let width = POPUP_MAX_WIDTH
        .min(area.width.saturating_sub(2))
        .max(POPUP_MIN_WIDTH.min(area.width))
        .min(area.width);

    let (anchor_x, bottom) = match prompt_area {
        Some(prompt) if prompt.height > 0 => (prompt.x, prompt.y),
        _ => (area.x, area.y + area.height),
    };

    Rect {
        x: anchor_x.min(area.x + area.width.saturating_sub(width)),
        y: bottom.saturating_sub(height + 1).max(area.y),
        width,
        height: height.min(area.height),
    }
}

impl Default for SlashCommandPopup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;

    fn render_to_lines(popup: &SlashCommandPopup, area: Rect, prompt_area: Rect) -> Vec<String> {
        let theme = Theme::dark();
        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| popup.render(frame, area, Some(prompt_area), &theme))
            .expect("popup render should succeed");
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| terminal.backend().buffer().get(x, y).symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn popup_sits_directly_above_the_prompt_instead_of_at_the_top_of_the_screen() {
        let area = Rect::new(0, 0, 100, 40);
        let prompt_area = Rect::new(2, 30, 96, 6);

        let mut popup = SlashCommandPopup::new();
        popup.open();
        let lines = render_to_lines(&popup, area, prompt_area);

        // `Borders::ALL` puts corners on the top/bottom rows; the vertical rule
        // appears on every row between them.
        let popup_rows: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                line.contains('┌') || line.contains('│') || line.contains('└')
            })
            .map(|(y, _)| y)
            .collect();

        assert!(
            !popup_rows.is_empty(),
            "the popup should paint a bordered box, got:\n{}",
            lines.join("\n")
        );

        let top = *popup_rows.first().unwrap();
        let bottom = *popup_rows.last().unwrap();

        assert!(
            top >= 20 && bottom < prompt_area.y as usize,
            "the popup should sit just above the prompt (rows 20..{}), but it painted rows {top}..{bottom}",
            prompt_area.y
        );
    }

    #[test]
    fn a_non_matching_query_still_renders_a_visible_no_results_state() {
        let area = Rect::new(0, 0, 100, 40);
        let prompt_area = Rect::new(2, 30, 96, 6);

        let mut popup = SlashCommandPopup::new();
        popup.open();
        for c in "zzzzqqq".chars() {
            popup.handle_input(c);
        }
        assert!(
            popup.filtered.is_empty(),
            "this query is expected to match nothing, got {:?}",
            popup.filtered
        );

        let lines = render_to_lines(&popup, area, prompt_area);
        let painted = lines.join("\n");
        assert!(
            painted.contains("No matching commands"),
            "an empty result set must still be visible so the user is not captured by an invisible popup, got:\n{painted}"
        );
    }

    #[test]
    fn the_listing_scrolls_so_a_selection_beyond_the_visible_window_stays_on_screen() {
        let area = Rect::new(0, 0, 100, 40);
        let prompt_area = Rect::new(2, 30, 96, 6);

        let mut popup = SlashCommandPopup::new();
        popup.open();
        popup.handle_input('s');
        let total = popup.filtered.len();
        assert!(
            total > POPUP_MAX_VISIBLE_ROWS,
            "this test needs more commands than one window shows, got {total}"
        );

        for _ in 0..POPUP_MAX_VISIBLE_ROWS + 3 {
            popup.move_down();
        }
        let selected = popup.state.selected().expect("a selection is active");
        let selected_name = popup.filtered[selected].clone();

        let lines = render_to_lines(&popup, area, prompt_area);
        assert!(
            lines.iter().any(|line| line.contains(&selected_name)),
            "after moving past the visible window the selected command {selected_name} must still be painted, got:\n{}",
            lines.join("\n")
        );
    }

    #[test]
    fn the_popup_lists_the_literal_token_as_well_as_the_title() {
        let area = Rect::new(0, 0, 100, 40);
        let prompt_area = Rect::new(2, 30, 96, 6);

        let mut popup = SlashCommandPopup::new();
        popup.open();
        let lines = render_to_lines(&popup, area, prompt_area);
        let painted = lines.join("\n");

        assert!(
            popup.filtered.iter().any(|name| painted.contains(name.as_str())),
            "each row should show the command token the user has to type, got:\n{painted}"
        );
    }
}
