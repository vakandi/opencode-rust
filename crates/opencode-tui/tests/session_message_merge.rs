//! `merge_messages` keeps an optimistic user bubble alive across a server sync.
//!
//! `set_messages` blind-replaced the transcript, so a prompt submitted before the
//! server persisted it vanished from the chat within one tick and only reappeared
//! once the assistant turn had finished.

use opencode_tui::context::{Message, MessagePart, MessageRole, SessionContext};

const OPTIMISTIC_USER_ID_PREFIX: &str = "local_user_";

fn user_message(id: &str, content: &str) -> Message {
    Message {
        id: id.to_string(),
        role: MessageRole::User,
        content: content.to_string(),
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
            text: content.to_string(),
        }],
    }
}

#[test]
fn optimistic_user_bubble_survives_a_sync_that_has_not_persisted_it_yet() {
    let mut ctx = SessionContext::default();
    let optimistic = format!("{OPTIMISTIC_USER_ID_PREFIX}1");
    ctx.messages.entry("ses_a".to_string()).or_default();
    ctx.add_message("ses_a", user_message(&optimistic, "fix the parser"));

    ctx.merge_messages("ses_a", Vec::new());

    let messages = ctx.messages.get("ses_a").expect("session present");
    assert_eq!(messages.len(), 1, "the submitted prompt must stay visible");
    assert_eq!(messages[0].content, "fix the parser");
}

#[test]
fn optimistic_bubble_is_dropped_once_the_server_echoes_the_same_prompt() {
    let mut ctx = SessionContext::default();
    ctx.messages.entry("ses_a".to_string()).or_default();
    ctx.add_message(
        "ses_a",
        user_message(&format!("{OPTIMISTIC_USER_ID_PREFIX}1"), "fix the parser"),
    );

    ctx.merge_messages("ses_a", vec![user_message("msg_server_1", "fix the parser")]);

    let messages = ctx.messages.get("ses_a").expect("session present");
    assert_eq!(messages.len(), 1, "no duplicate once the server persisted it");
    assert_eq!(messages[0].id, "msg_server_1");
}

#[test]
fn assistant_messages_are_never_carried_over_from_the_local_transcript() {
    let mut ctx = SessionContext::default();
    let mut assistant = user_message("msg_old", "stale answer");
    assistant.role = MessageRole::Assistant;
    ctx.messages.entry("ses_a".to_string()).or_default();
    ctx.add_message("ses_a", assistant);

    ctx.merge_messages("ses_a", vec![user_message("msg_server_1", "hi")]);

    let messages = ctx.messages.get("ses_a").expect("session present");
    assert_eq!(messages.len(), 1, "only the server transcript survives");
    assert_eq!(messages[0].id, "msg_server_1");
}