use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub fn horizontal_split(area: Rect, constraints: Vec<Constraint>) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

pub fn vertical_split(area: Rect, constraints: Vec<Constraint>) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height as u16) / 2),
            Constraint::Length(height),
            Constraint::Percentage((100 - height as u16) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width as u16) / 2),
            Constraint::Length(width),
            Constraint::Percentage((100 - width as u16) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub fn footer(area: Rect, height: u16) -> Rect {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(height)])
        .split(area);
    layout[1]
}

pub fn header(area: Rect, height: u16) -> Rect {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(height), Constraint::Min(0)])
        .split(area);
    layout[0]
}

pub fn sidebar_left(area: Rect, width: u16) -> (Rect, Rect) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(width), Constraint::Min(0)])
        .split(area);
    (layout[0], layout[1])
}

pub fn sidebar_right(area: Rect, width: u16) -> (Rect, Rect) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(width)])
        .split(area);
    (layout[0], layout[1])
}
