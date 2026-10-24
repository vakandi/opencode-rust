//! The chat transcript must stay scrollable.
//!
//! Regression: the transcript was pinned to the bottom and the mouse wheel never
//! moved it, so a long session could only ever be read at its end.

use opencode_tui::context::{Message, MessagePart, MessageRole, SessionStatus};
use opencode_tui::{App, Route};
use ratatui::{backend::TestBackend, Terminal};

const W: u16 = 120;
const H: u16 = 40;
const SESSION: &str = "ses_scroll_probe";

fn app_with_long_transcript(messages: usize) -> App {
    app_with_status(messages, SessionStatus::Idle)
}

fn app_with_status(messages: usize, status: SessionStatus) -> App {
    let mut app = App::new_for_test().expect("headless App should construct");
    let session_id = SESSION.to_string();

    let context = app.context();
    {
        let mut ctx = context.session.write();
        ctx.messages.entry(session_id.clone()).or_default();
        for i in 0..messages {
            ctx.add_message(
                &session_id,
                Message {
                    id: format!("msg_{i}"),
                    role: MessageRole::User,
                    content: format!("MARKER{i:03}"),
                    created_at: chrono::Utc::now(),
                    agent: None,
                    model: None,
                    mode: None,
                    finish: None,
                    error: None,
                    completed_at: None,
                    cost: 0.0,
                    tokens: Default::default(),
                    parts: vec![MessagePart::Text {
                        text: format!("MARKER{i:03}"),
                    }],
                },
            );
        }
        ctx.set_status(&session_id, status.clone());
    }

    app.navigate(Route::Session {
        session_id: session_id.clone(),
    });
    app
}

fn paint(app: &mut App, terminal: &mut Terminal<TestBackend>) -> String {
    terminal
        .draw(|frame| app.render(frame))
        .expect("render into TestBackend should succeed");
    let buf = terminal.backend().buffer();
    (0..H)
        .map(|y| (0..W).map(|x| buf.get(x, y).symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_transcript_scrolls_back_to_the_first_message() {
    let mut app = app_with_long_transcript(80);
    let mut terminal = Terminal::new(TestBackend::new(W, H)).unwrap();

    paint(&mut app, &mut terminal);
    for _ in 0..200 {
        app.scroll_session_up_mouse();
    }
    let top = paint(&mut app, &mut terminal);

    assert!(
        top.contains("MARKER000"),
        "scrolling to the top must show the first message, got:\n{top}"
    );
}

#[test]
fn scrolling_back_does_not_get_forced_to_the_bottom_on_the_next_frame() {
    let mut app = app_with_long_transcript(80);
    let mut terminal = Terminal::new(TestBackend::new(W, H)).unwrap();

    paint(&mut app, &mut terminal);
    for _ in 0..200 {
        app.scroll_session_up_mouse();
    }
    paint(&mut app, &mut terminal);
    let second = paint(&mut app, &mut terminal);
    let third = paint(&mut app, &mut terminal);

    assert_eq!(
        second, third,
        "an idle transcript must stay where the user scrolled it"
    );
    assert!(
        second.contains("MARKER000"),
        "the view must not snap back to the bottom on the next frame"
    );
}
/// The live case: the transcript grows on every tick while the agent streams, and
/// a reader who scrolled up must not be yanked back to the bottom each frame.
#[test]
fn scrolling_back_survives_a_transcript_that_is_still_growing() {
    let mut app = app_with_status(80, SessionStatus::Running);
    let mut terminal = Terminal::new(TestBackend::new(W, H)).unwrap();

    paint(&mut app, &mut terminal);
    for _ in 0..200 {
        app.scroll_session_up_mouse();
    }

    let context = app.context();
    for i in 80..90 {
        let mut ctx = context.session.write();
        ctx.add_message(
            SESSION,
            Message {
                id: format!("msg_{i}"),
                role: MessageRole::Assistant,
                content: format!("LATE{i:03}"),
                created_at: chrono::Utc::now(),
                agent: None,
                model: None,
                mode: None,
                finish: None,
                error: None,
                completed_at: None,
                cost: 0.0,
                tokens: Default::default(),
                parts: vec![MessagePart::Text {
                    text: format!("LATE{i:03}"),
                }],
            },
        );
        drop(ctx);
        let view = paint(&mut app, &mut terminal);
        assert!(
            view.contains("MARKER000"),
            "frame {i} yanked the view back to the bottom"
        );
    }
}
