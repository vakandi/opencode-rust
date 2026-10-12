//! Headless construction of the TUI `App`.
//!
//! `App::new()` enables raw mode and switches to the alternate screen, which
//! needs a real tty, so it cannot run inside `cargo test`. `App::new_for_test()`
//! builds the same struct with `terminal: None` so the rest of the app can be
//! exercised without a terminal.

use opencode_tui::App;

#[test]
fn new_for_test_builds_without_a_terminal() {
    let app = App::new_for_test().expect("headless App should construct");
    assert!(
        !app.has_terminal(),
        "new_for_test must not attach a terminal"
    );
}

#[test]
fn new_for_test_is_repeatable() {
    for _ in 0..3 {
        let app = App::new_for_test().expect("headless App should construct");
        assert!(!app.has_terminal());
    }
}
