use ratatui::{layout::Rect, Frame};

pub struct Dialog {
    _title: String,
    open: bool,
}

impl Dialog {
    pub fn new(title: &str) -> Self {
        Self {
            _title: title.to_string(),
            open: false,
        }
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

    pub fn render(&self, _frame: &mut Frame, _area: Rect) {}
}
