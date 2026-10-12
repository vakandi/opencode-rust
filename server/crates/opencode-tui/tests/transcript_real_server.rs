//! The acceptance test for the wire contract: the REAL server router, over a REAL socket, read
//! back by the TUI's REAL `ApiClient`.
//!
//! The other tests in this file drive a stub that answers bytes I wrote by hand. That proves the
//! TUI can parse a shape I believe the server emits; it cannot prove the server emits it. This
//! test closes that gap by serving `opencode_server::routes::router()` itself, so every field
//! asserted below was produced by `events.rs::message_to_ts_json` / `part_to_ts_json`.
//!
//! It is also the regression guard for the `#[serde(default)]` trap: a parser that defaults
//! `id` / `role` / `createdAt` would still deserialize this body, but the values asserted here
//! would come back as "" / User / 0.

use opencode_tui::api::{ApiClient, ApiMessage};
use serde_json::{json, Value};

use std::sync::Arc;

async fn serve_router() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app =
        opencode_server::routes::router().with_state(Arc::new(opencode_server::ServerState::new()));

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    base_url
}

/// Creates a session and returns its id.
async fn new_session(base_url: &str) -> String {
    let response = reqwest::Client::new()
        .post(format!("{base_url}/session"))
        .json(&json!({}))
        .send()
        .await
        .expect("POST /session reaches the real router");
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "POST /session failed with {status}: {text}"
    );
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    body["info"]["id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .unwrap_or_else(|| panic!("no session id in {body}"))
        .to_string()
}

/// `ApiClient` is deliberately blocking, which cannot run on a runtime worker thread.
async fn get_messages(base_url: &str, session_id: &str) -> Vec<ApiMessage> {
    let (base_url, session_id) = (base_url.to_string(), session_id.to_string());
    tokio::task::spawn_blocking(move || {
        ApiClient::new(base_url)
            .get_messages(&session_id)
            .expect("the real server's transcript must parse as envelopes")
    })
    .await
    .expect("spawn_blocking joins")
}

#[tokio::test]
async fn the_real_servers_transcript_parses_into_envelopes_with_real_values() {
    let base_url = serve_router().await;
    let session_id = new_session(&base_url).await;

    for text in ["first question", "second question"] {
        let response = reqwest::Client::new()
            .post(format!("{base_url}/session/{session_id}/message"))
            .json(&json!({ "parts": [{ "type": "text", "text": text }] }))
            .send()
            .await
            .expect("POST message reaches the real router");
        let status = response.status();
        let text_body = response.text().await.unwrap_or_default();
        assert!(
            status.is_success(),
            "POST message failed with {status}: {text_body}"
        );
    }

    let messages = get_messages(&base_url, &session_id).await;

    assert!(
        messages.len() >= 2,
        "the transcript must be NON-EMPTY; before the envelope fix it was 0, silently. got {}",
        messages.len()
    );

    for message in &messages {
        // This is the assertion a #[serde(default)] parser would fail: these are the server's
        // real values, not "" / User / 0.
        assert!(
            !message.info.id.is_empty(),
            "info.id must be the server's real id"
        );
        assert_ne!(
            message.info.created_at, 0,
            "info.createdAt must be the server's real timestamp"
        );
        assert_eq!(
            message.info.session_id, session_id,
            "info.sessionID must be the session we created"
        );
        assert!(
            matches!(
                message.info.role.as_str(),
                "user" | "assistant" | "system" | "tool"
            ),
            "unexpected role {:?}",
            message.info.role
        );

        // POSTing a user message also creates an assistant turn, and with no provider reachable that
        // turn is legitimately EMPTY. Only the user turns this test seeded must carry parts.
        if message.info.role == "user" {
            assert!(
                !message.parts.is_empty(),
                "the user turn this test seeded must carry its text part; got {} parts",
                message.parts.len()
            );
        }
        for part in &message.parts {
            assert!(!part.id.is_empty(), "every real part carries an id");
            assert!(
                matches!(
                    part.part_type.as_str(),
                    "text" | "reasoning" | "tool" | "file" | "step-start" | "step-finish"
                ),
                "unexpected part type {:?} from the real server",
                part.part_type
            );
        }
    }

    let first = &messages[0];
    assert_eq!(first.info.role, "user");
    assert_eq!(first.parts[0].text.as_deref(), Some("first question"));
}

/// A hand-written fixture can drift from `events.rs` and keep passing, which is how this class
/// of bug survives. This test feeds the server's REAL serializer output straight into the TUI's
/// REAL parser, so any change to the wire shape breaks it immediately.
#[tokio::test]
async fn the_real_tool_serializer_output_parses_and_pairs_its_siblings() {
    use chrono::Utc;
    use opencode_session::{MessagePart, PartType};

    let session_id = "ses_01";
    let message_id = "msg_01";

    let parts = vec![
        MessagePart {
            id: "prt_call".into(),
            part_type: PartType::ToolCall {
                id: "call_01".into(),
                name: "read".into(),
                input: serde_json::json!({ "path": "src/main.rs" }),
            },
            created_at: Utc::now(),
            message_id: Some(message_id.into()),
        },
        MessagePart {
            id: "prt_result".into(),
            part_type: PartType::ToolResult {
                tool_call_id: "call_01".into(),
                content: "fn main() {}".into(),
                is_error: false,
            },
            created_at: Utc::now(),
            message_id: Some(message_id.into()),
        },
    ];

    let real_parts: Vec<serde_json::Value> = parts
        .iter()
        .map(|part| opencode_server::events::part_to_ts_json(part, session_id))
        .collect();

    let body = serde_json::json!({
        "info": {
            "id": message_id,
            "sessionID": session_id,
            "role": "assistant",
            "createdAt": 1_700_000_000_000_i64,
            "agent": null,
            "model": null,
            "metadata": {},
        },
        "parts": real_parts,
    });

    let parsed: ApiMessage = serde_json::from_value(body.clone())
        .unwrap_or_else(|e| panic!("the server's REAL output must parse, got: {e}"));

    assert_eq!(parsed.parts.len(), 2);
    assert!(
        parsed.parts.iter().all(|p| p.part_type == "tool"),
        "both siblings must arrive as `tool` parts"
    );
    assert!(
        parsed
            .parts
            .iter()
            .all(|p| p.call_id.as_deref() == Some("call_01")),
        "both siblings must share callID, or the fold cannot pair them: {:?}",
        parsed.parts.iter().map(|p| &p.call_id).collect::<Vec<_>>()
    );

    let call = &parsed.parts[0];
    assert_eq!(
        call.tool.as_deref(),
        Some("read"),
        "the call sibling names the tool"
    );
    assert_eq!(
        call.state.as_ref().map(|s| s.status.as_str()),
        Some("pending"),
        "the call sibling is born pending"
    );
    assert_eq!(
        call.state
            .as_ref()
            .and_then(|s| s.input.as_ref())
            .and_then(|i| i.get("path"))
            .and_then(|p| p.as_str()),
        Some("src/main.rs"),
        "the call sibling must carry the input the fold renders as arguments"
    );

    let result = &parsed.parts[1];
    assert_eq!(result.tool, None, "the result sibling names no tool");
    assert_eq!(
        result.state.as_ref().map(|s| s.status.as_str()),
        Some("completed"),
        "the result sibling is terminal"
    );
    assert_eq!(
        result.state.as_ref().and_then(|s| s.output.as_deref()),
        Some("fn main() {}"),
        "the result sibling must carry the output the fold renders as the result"
    );

    // Every key the real serializer emits for a file part must survive too.
    let file_part = opencode_session::MessagePart {
        id: "prt_file".into(),
        part_type: PartType::File {
            url: "file:///tmp/trace.log".into(),
            filename: "trace.log".into(),
            mime: "text/plain".into(),
        },
        created_at: Utc::now(),
        message_id: Some(message_id.into()),
    };
    let file_json = opencode_server::events::part_to_ts_json(&file_part, session_id);
    let file: opencode_tui::api::MessagePart =
        serde_json::from_value(file_json).expect("a real file part must parse");
    assert_eq!(
        file.file_info().map(|f| f.filename),
        Some("trace.log".to_string()),
        "the server spells file fields flat on the part; they must not be dropped"
    );
}
