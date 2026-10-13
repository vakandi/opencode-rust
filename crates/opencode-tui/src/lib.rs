pub mod api;
pub mod app;
pub mod branding;
pub mod command;
pub mod components;
pub mod context;
pub mod event;
pub mod file_index;
pub mod hooks;
pub mod router;
pub mod terminal;
pub mod theme;
pub mod ui;

pub use api::ApiClient;
pub use app::App;
pub use event::Event;
pub use router::{Route, Router};
pub use terminal::{reset_title, set_session_title, set_title};
pub use theme::Theme;

fn setup_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );
        eprintln!("\n\nPANIC: {}", panic_info);
    }));
}

pub fn run_tui() -> anyhow::Result<()> {
    setup_panic_hook();

    let mut app = App::new()?;
    app.run()
}
