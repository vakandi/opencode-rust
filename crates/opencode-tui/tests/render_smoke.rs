//! Headless render smoke test.
//!
//! `App::render` paints the whole widget tree into any `ratatui::Frame`, so it
//! can be driven by `TestBackend` instead of a real tty. That exercises the
//! route branch, every overlay render call and the screen-line snapshot used
//! for text selection.

use opencode_tui::{App, Route};
use ratatui::{backend::TestBackend, Terminal};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let screen = terminal.backend().buffer();
    (0..HEIGHT)
        .map(|y| {
            (0..WIDTH)
                .map(|x| screen.get(x, y).symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn render_paints_the_home_route_and_snapshots_screen_lines() {
    let mut app = App::new_for_test().expect("headless App should construct");
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal");

    terminal
        .draw(|frame| app.render(frame))
        .expect("render into TestBackend should succeed");

    let rendered = buffer_text(&terminal);
    assert!(
        rendered.split_whitespace().count() > 0,
        "the frame should not be blank, got:\n{rendered}"
    );
    assert!(
        rendered.contains("A Rusted OpenCode Version"),
        "the home route should paint the tagline, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Ask anything"),
        "the prompt placeholder should be painted, got:\n{rendered}"
    );

    assert_eq!(
        app.screen_lines().len(),
        HEIGHT as usize,
        "render should snapshot one line per terminal row"
    );
}

#[test]
fn render_paints_the_session_route_sidebar() {
    let mut app = App::new_for_test().expect("headless App should construct");
    app.navigate(Route::Session {
        session_id: "ses_render_smoke".to_string(),
    });

    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal");

    terminal
        .draw(|frame| app.render(frame))
        .expect("render into TestBackend should succeed");

    let rendered = buffer_text(&terminal);
    for section in ["Session", "Context", "MCP", "LSP"] {
        assert!(
            rendered.contains(section),
            "the session route should paint the {section} sidebar section, got:\n{rendered}"
        );
    }
}

#[test]
fn render_never_emits_the_alternate_screen_escape() {
    let mut app = App::new_for_test().expect("headless App should construct");
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal");

    terminal
        .draw(|frame| app.render(frame))
        .expect("render into TestBackend should succeed");

    assert!(
        !buffer_text(&terminal).contains("\u{1b}[?1049h"),
        "a raw terminal control escape leaked into the frame buffer"
    );
    assert!(
        app.screen_lines().iter().all(|line| !line.contains('\u{1b}')),
        "a raw escape byte leaked into the selection snapshot"
    );
}

#[test]
fn render_skips_areas_smaller_than_10x10() {
    let mut app = App::new_for_test().expect("headless App should construct");
    let mut terminal = Terminal::new(TestBackend::new(8, 6)).expect("test terminal");

    terminal
        .draw(|frame| app.render(frame))
        .expect("a too-small area must be skipped, not panic");

    assert!(
        app.screen_lines().is_empty(),
        "nothing should be snapshotted when the area is skipped, got {:?}",
        app.screen_lines()
    );
}
