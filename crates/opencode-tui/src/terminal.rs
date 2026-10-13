use std::io;

use crate::branding::{APP_NAME, APP_SHORT_NAME};

pub fn set_title(title: &str) -> io::Result<()> {
    crossterm::execute!(io::stdout(), crossterm::terminal::SetTitle(title))
}

pub fn set_session_title(session_title: &str) -> io::Result<()> {
    set_title(&format!("{} ({}) - {}", APP_NAME, APP_SHORT_NAME, session_title))
}

pub fn reset_title() -> io::Result<()> {
    set_title(&format!("{} ({})", APP_NAME, APP_SHORT_NAME))
}
