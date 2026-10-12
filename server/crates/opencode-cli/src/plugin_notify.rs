//! Telling the oh-my-opencode plugin that a session turn has ended.
//!
//! `session.idle` is fired from exactly one place in the server (`run_prompt_turn`), so the CLI
//! never emitted it. omo keeps its background manager alive until it sees one, which is the
//! second independent reason `run` used to cut background work off.
//!
//! The plugin host already supports this: `sendNotification` on the TS side, correlated by
//! `params.requestId` on the Rust side.

use opencode_plugin::{HookContext, HookEvent};

pub async fn notify_session_idle(session_id: &str) {
    let payload = serde_json::json!({
        "type": "session.idle",
        "properties": { "sessionID": session_id },
    });
    opencode_plugin::trigger_collect(
        HookContext::new(HookEvent::PluginEvent)
            .with_data("event", payload)
            .with_session(session_id),
    )
    .await;
}