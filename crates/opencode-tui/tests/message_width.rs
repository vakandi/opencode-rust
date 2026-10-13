//! Every line the message list paints must fit inside the content width.
//!
//! `render_messages` builds one `Paragraph` and hands it the whole messages
//! area, so it depends on an invariant it never enforces: every `Line` in that
//! vec must already be at most `content_width` wide. `paint_block_lines`
//! establishes it by wrapping *and* padding. Branches that append raw lines
//! silently drop out of the bubble and run edge to edge.

use chrono::Utc;
use opencode_tui::context::{Message, MessagePart, MessageRole};
use opencode_tui::{App, Route};
use ratatui::{backend::TestBackend, Terminal};

const WIDTH: u16 = 100;
const HEIGHT: u16 = 40;

/// render_main reserves 4 columns of breathing room and the block border takes 1.
const CONTENT_WIDTH_LIMIT: usize = (WIDTH - 4 - 1 - 1) as usize;

fn assistant(id: &str, content: &str) -> Message {
    Message {
        id: id.to_string(),
        role: MessageRole::Assistant,
        content: content.to_string(),
        created_at: Utc::now(),
        agent: None,
        model: None,
        mode: None,
        finish: None,
        error: None,
        completed_at: None,
        cost: 0.0,
        tokens: Default::default(),
        parts: Vec::new(),
    }
}

fn system(id: &str, content: &str) -> Message {
    Message {
        role: MessageRole::System,
        ..assistant(id, content)
    }
}

fn render_session(messages: Vec<Message>) -> String {
    let mut app = App::new_for_test().expect("headless App should construct");
    app.navigate(Route::Session {
        session_id: "ses_width".to_string(),
    });
    app.context()
        .session
        .write()
        .set_messages("ses_width", messages);

    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal");
    // `paint` snapshots `session_view` before `sync_render_prelude` builds it, so the
    // first frame after navigating falls back to Home. The second frame is the real one.
    terminal
        .draw(|frame| app.render(frame))
        .expect("first render should succeed");
    terminal
        .draw(|frame| app.render(frame))
        .expect("render into TestBackend should succeed");

    (0..HEIGHT)
        .map(|y| {
            (0..WIDTH)
                .map(|x| terminal.backend().buffer().get(x, y).symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The sidebar is a 42-column overlay on the right; rows it touches are not a
/// measure of message content, so only the columns left of it are inspected.
const MESSAGE_REGION_END: usize = (WIDTH - 42 - 1) as usize;

/// Rows carrying message content, identified by the block's left rule.
fn message_rows(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .map(|line| {
            let clipped: String = line.chars().take(MESSAGE_REGION_END).collect();
            clipped.trim_end().to_string()
        })
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with('│') || trimmed.starts_with('┃')
        })
        .collect()
}

#[test]
fn system_message_text_does_not_span_the_full_width() {
    let long = "SYSTEM NOTICE ".repeat(12);
    let rendered = render_session(vec![system("msg_sys", &long)]);

    assert!(
        rendered.contains("SYSTEM NOTICE"),
        "the system message should be visible, got:\n{rendered}"
    );
    // A 168-char sentence at a 94-column content width must occupy more than one
    // row: the bubble wraps it. Unwrapped output would put the whole sentence on
    // a single row and silently truncate the tail at the buffer edge.
    let rows_mentioning_notice = message_rows(&rendered)
        .iter()
        .filter(|row| row.contains("SYSTEM NOTICE"))
        .count();
    assert!(
        rows_mentioning_notice >= 2,
        "long system text should wrap across several rows inside the bubble, \
         but only {rows_mentioning_notice} row(s) carry it, got:\n{}",
        message_rows(&rendered).join("\n")
    );
}

#[test]
fn tool_call_output_does_not_span_the_full_width() {
    let mut message = assistant("msg_tool", "");
    message.parts.push(MessagePart::ToolCall {
        id: "call_1".to_string(),
        name: "bash".to_string(),
        arguments: format!("{{\"command\":\"{}\"}}", "rm -rf /tmp/scratch && ".repeat(20)),
    });
    message.parts.push(MessagePart::ToolResult {
        id: "call_1".to_string(),
        result: format!("MARKA {}", "tail ".repeat(200)),
        is_error: true,
    });

    let rendered = render_session(vec![message]);

    // The invariant render_messages relies on: no painted message row may be
    // wider than the bubble's content width. Unbounded output is one long line,
    // which ratatui clips at the buffer edge, silently losing the tail.
    let rows = message_rows(&rendered);
    let widest = rows.iter().map(|row| row.chars().count()).max().unwrap_or(0);
    assert!(
        widest <= CONTENT_WIDTH_LIMIT,
        "message rows must stay within the bubble content width ({CONTENT_WIDTH_LIMIT}), widest was {widest}, rendered:\n{}",
        rows.join("\n")
    );
}

#[test]
fn assistant_error_is_rendered_in_the_transcript() {
    let mut message = assistant("msg_err", "");
    message.error = Some("API error: 429 rate limited, please retry later".to_string());

    let rendered = render_session(vec![message]);

    assert!(
        rendered.contains("429"),
        "an assistant message carrying an error must show that error, got:\n{rendered}"
    );
}