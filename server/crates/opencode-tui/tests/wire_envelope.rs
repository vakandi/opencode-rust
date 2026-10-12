//! The server's message wire contract, from the TUI's point of view.
//!
//! `crates/opencode-server/src/events.rs::message_to_ts_json` is the source of truth and it is
//! PINNED by `crates/opencode-server/tests/wire_contract.rs`:
//!
//! ```json
//! { "info": { "id", "sessionID", "role", "createdAt", "agent", "model", "metadata" },
//!   "parts": [ ... ] }
//! ```
//!
//! The parts of a tool call arrive as **two siblings sharing one `callID`**, differing only in
//! `state.status` (`pending` for the call, `completed`/`error` for the result). Nothing here
//! may paper over that by defaulting `info.id`/`info.role`/`info.createdAt` — a missing-field
//! error that empties the transcript is correct; a silent `id: ""` is corruption.

use opencode_tui::api::{ApiMessage, MessageInfoInner};
use serde_json::{json, Value};

/// Byte-for-byte what the server emits for one assistant turn with one `read` tool call.
fn server_envelope() -> Value {
    json!({
        "info": {
            "id": "msg_01",
            "sessionID": "ses_01",
            "role": "assistant",
            "createdAt": 1_700_000_000_000_i64,
            "agent": "build",
            "model": "claude-sonnet-4",
            "metadata": {},
        },
        "parts": [
            {
                "id": "prt_text",
                "sessionID": "ses_01",
                "messageID": "msg_01",
                "type": "text",
                "text": "let me look",
                "synthetic": false,
                "ignored": false,
                "time": { "start": 1_700_000_000_000_i64, "end": null },
            },
            {
                "id": "prt_call",
                "sessionID": "ses_01",
                "messageID": "msg_01",
                "type": "tool",
                "callID": "call_01",
                "tool": "read",
                "state": {
                    "status": "pending",
                    "input": { "path": "src/main.rs" },
                    "raw": "",
                },
                "time": { "start": 1_700_000_000_100_i64, "end": null },
            },
            {
                "id": "prt_result",
                "sessionID": "ses_01",
                "messageID": "msg_01",
                "type": "tool",
                "callID": "call_01",
                "state": { "status": "completed", "output": "fn main() {}" },
                "time": { "start": 1_700_000_000_200_i64, "end": null },
            },
            {
                "id": "prt_file",
                "sessionID": "ses_01",
                "messageID": "msg_01",
                "type": "file",
                "url": "file:///tmp/trace.log",
                "filename": "trace.log",
                "mime": "text/plain",
            },
        ],
    })
}

#[test]
fn server_envelope_deserializes_into_api_message() {
    let envelope = server_envelope();
    let parsed: ApiMessage = serde_json::from_value(envelope.clone())
        .unwrap_or_else(|e| panic!("the pinned {{info, parts}} envelope must parse, got: {e}"));

    assert_eq!(
        parsed.parts.len(),
        4,
        "every sibling part must survive parsing"
    );
    let file = &parsed.parts[3];
    assert_eq!(file.part_type, "file");
    assert_eq!(
        file.file_info().map(|f| f.filename),
        Some("trace.log".to_string()),
        "a file part must survive the envelope parse"
    );
    let info: &MessageInfoInner = &parsed.info;
    assert_eq!(info.id, "msg_01");
    assert_eq!(
        info.session_id, "ses_01",
        "`sessionID` is the server's spelling"
    );
    assert_eq!(info.role, "assistant");
    assert_eq!(
        info.created_at, 1_700_000_000_000_i64,
        "`createdAt` is camelCase on the wire"
    );
    assert_eq!(info.agent.as_deref(), Some("build"));
    assert_eq!(info.model.as_deref(), Some("claude-sonnet-4"));
}

#[test]
fn flat_message_without_envelope_is_rejected_not_silently_defaulted() {
    // The old flat shape: parts nested inside the message, no `info` wrapper.
    let flat = json!({
        "id": "msg_01",
        "sessionID": "ses_01",
        "role": "assistant",
        "createdAt": 1_700_000_000_000_i64,
        "parts": [{ "id": "prt_text", "type": "text", "text": "hi" }],
    });

    let err = serde_json::from_value::<ApiMessage>(flat)
        .expect_err("a flat message must not parse as the envelope");

    assert!(
        err.to_string().contains("info"),
        "the error must name the missing `info` field, got: {err}"
    );
}

#[test]
fn envelope_missing_info_metadata_is_still_fine() {
    // `agent`/`model`/`metadata` are optional on the wire; only the four identity fields are
    // load-bearing, so their absence must not be papered over with a default.
    let mut envelope = server_envelope();
    let info = envelope.get_mut("info").unwrap().as_object_mut().unwrap();
    info.remove("agent");
    info.remove("model");
    info.remove("metadata");

    let parsed: ApiMessage =
        serde_json::from_value(envelope).expect("optional info fields may be absent");
    assert_eq!(parsed.info.agent, None);
    assert_eq!(parsed.info.model, None);
}

#[test]
fn sibling_tool_parts_keep_one_shared_call_id() {
    let parsed: ApiMessage = serde_json::from_value(server_envelope()).unwrap();

    let tool_parts: Vec<_> = parsed
        .parts
        .iter()
        .filter(|p| p.part_type == "tool")
        .collect();
    assert_eq!(
        tool_parts.len(),
        2,
        "the call and its result are two siblings"
    );

    let call_ids: Vec<_> = tool_parts
        .iter()
        .filter_map(|p| p.call_id.as_deref())
        .collect();
    assert_eq!(
        call_ids,
        vec!["call_01", "call_01"],
        "both siblings must carry the same callID so the envelope fold can group them"
    );

    // Only the `pending` sibling carries the tool name and input; only the terminal sibling
    // carries the output. The fold has to merge both halves.
    assert_eq!(tool_parts[0].tool.as_deref(), Some("read"));
    assert_eq!(
        tool_parts[0].state.as_ref().and_then(|s| s.status_rank()),
        Some(ToolRank::Pending)
    );
    assert_eq!(tool_parts[1].tool, None, "the result sibling names no tool");
    assert_eq!(
        tool_parts[1]
            .state
            .as_ref()
            .and_then(|s| s.output.as_deref()),
        Some("fn main() {}")
    );
}

use opencode_tui::api::ToolRank;

#[test]
fn status_precedence_is_pending_below_running_below_terminal() {
    let pending =
        serde_json::from_value::<opencode_tui::api::ToolState>(json!({ "status": "pending" }))
            .unwrap();
    let running =
        serde_json::from_value::<opencode_tui::api::ToolState>(json!({ "status": "running" }))
            .unwrap();
    let completed =
        serde_json::from_value::<opencode_tui::api::ToolState>(json!({ "status": "completed" }))
            .unwrap();
    let error =
        serde_json::from_value::<opencode_tui::api::ToolState>(json!({ "status": "error" }))
            .unwrap();

    assert!(pending.status_rank().unwrap() < running.status_rank().unwrap());
    assert!(running.status_rank().unwrap() < completed.status_rank().unwrap());
    assert!(running.status_rank().unwrap() < error.status_rank().unwrap());
    assert_eq!(
        completed.status_rank().unwrap(),
        error.status_rank().unwrap(),
        "completed and error are the same terminal rank"
    );
}

#[test]
fn an_unknown_status_never_deserializes_into_a_whole_message_failure() {
    // A status the TUI has never heard of must not blow up the whole transcript.
    let parsed: ApiMessage = serde_json::from_value(server_envelope()).unwrap();
    assert_eq!(parsed.parts.len(), 4);

    let state =
        serde_json::from_value::<opencode_tui::api::ToolState>(json!({ "status": "wat" })).unwrap();
    assert!(
        state.status_rank().is_none(),
        "an unknown status has no rank and must lose to every known one"
    );
}
