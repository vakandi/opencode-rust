//! End-to-end proof that the TUI reads the server's real transcript over HTTP.
//!
//! The bytes below are byte-for-byte what `crates/opencode-server/src/events.rs`
//! (`message_to_ts_json` + `part_to_ts_json`) puts on the wire for a user turn and an assistant
//! turn that called one tool. A real socket stands in for the server, so the whole transport ->
//! deserialization half of the contract is exercised: URL, status handling, the `{info, parts}`
//! envelope, and the two-sibling tool call that only `map_api_message`'s fold can pair up.
//!
//! No new dependency: the stub is a bare `TcpListener` speaking HTTP/1.1.

use opencode_tui::api::ApiClient;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

/// Answers one request with `body`, then shuts the connection down.
struct StubServer {
    base_url: String,
    requests: mpsc::Receiver<String>,
}

impl StubServer {
    fn start(status: &'static str, body: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&buffer[..read]).to_string());

                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        Self {
            base_url,
            requests: rx,
        }
    }

    fn requested_path(&self) -> String {
        let raw = self
            .requests
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("stub got no request");
        raw.lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default()
            .to_string()
    }
}

/// A user turn followed by an assistant turn that calls `read` and gets its result back.
fn realistic_transcript() -> Value {
    json!([
        {
            "info": {
                "id": "msg_u1",
                "sessionID": "ses_01",
                "role": "user",
                "createdAt": 1_700_000_000_000_i64,
                "agent": null,
                "model": null,
                "metadata": {},
            },
            "parts": [
                {
                    "id": "prt_u1", "sessionID": "ses_01", "messageID": "msg_u1",
                    "type": "text", "text": "what does main do?", "synthetic": false, "ignored": false,
                    "time": { "start": 1_700_000_000_000_i64, "end": null },
                },
            ],
        },
        {
            "info": {
                "id": "msg_a1",
                "sessionID": "ses_01",
                "role": "assistant",
                "createdAt": 1_700_000_000_500_i64,
                "agent": "build",
                "model": "claude-sonnet-4",
                "metadata": {},
            },
            "parts": [
                {
                    "id": "prt_a1", "sessionID": "ses_01", "messageID": "msg_a1",
                    "type": "text", "text": "let me read it", "synthetic": false, "ignored": false,
                    "time": { "start": 1_700_000_000_500_i64, "end": null },
                },
                {
                    "id": "prt_a2", "sessionID": "ses_01", "messageID": "msg_a1",
                    "type": "tool", "callID": "call_01", "tool": "read",
                    "state": {
                        "status": "pending",
                        "input": { "path": "src/main.rs" },
                        "raw": "",
                    },
                    "time": { "start": 1_700_000_000_600_i64, "end": null },
                },
                {
                    "id": "prt_a3", "sessionID": "ses_01", "messageID": "msg_a1",
                    "type": "tool", "callID": "call_01",
                    "state": { "status": "completed", "output": "fn main() { println!(\"hi\"); }" },
                    "time": { "start": 1_700_000_000_700_i64, "end": null },
                },
            ],
        },
    ])
}

#[test]
fn a_real_transcript_arrives_as_envelopes_with_tool_siblings_intact() {
    let body = realistic_transcript().to_string();
    let server = StubServer::start("200 OK", body);
    let client = ApiClient::new(server.base_url.clone());

    let messages = client
        .get_messages("ses_01")
        .expect("GET /session/:id/message must yield envelopes");

    assert_eq!(
        server.requested_path(),
        "/session/ses_01/message",
        "the client must hit the endpoint the server actually exposes"
    );

    assert_eq!(messages.len(), 2, "both turns must survive the round trip");

    let user = &messages[0];
    assert_eq!(user.info.role, "user");
    assert_eq!(user.info.id, "msg_u1");
    assert_eq!(user.info.session_id, "ses_01");
    assert_eq!(user.parts.len(), 1);
    assert_eq!(user.parts[0].text.as_deref(), Some("what does main do?"));

    let assistant = &messages[1];
    assert_eq!(assistant.info.role, "assistant");
    assert_eq!(assistant.info.agent.as_deref(), Some("build"));
    assert_eq!(assistant.info.model.as_deref(), Some("claude-sonnet-4"));
    assert_eq!(assistant.info.created_at, 1_700_000_000_500_i64);
    assert_eq!(assistant.parts.len(), 3);

    // The fold's input: two siblings sharing one callID, one carrying the tool and input, one
    // carrying the output.
    let tool_parts: Vec<_> = assistant
        .parts
        .iter()
        .filter(|part| part.part_type == "tool")
        .collect();
    assert_eq!(
        tool_parts.len(),
        2,
        "a call and its result arrive as two siblings"
    );

    let call_id = "call_01";
    assert!(tool_parts
        .iter()
        .all(|part| part.call_id.as_deref() == Some(call_id)));
    assert_eq!(tool_parts[0].tool.as_deref(), Some("read"));
    assert_eq!(
        tool_parts[0]
            .state
            .as_ref()
            .and_then(|state| state.input.as_ref())
            .and_then(|input| input.get("path"))
            .and_then(|path| path.as_str()),
        Some("src/main.rs"),
        "the call sibling must keep the tool input the fold renders as arguments"
    );
    assert_eq!(
        tool_parts[1]
            .state
            .as_ref()
            .and_then(|state| state.output.as_deref()),
        Some("fn main() { println!(\"hi\"); }"),
        "the result sibling must keep the output the fold renders as the result"
    );
    assert_eq!(
        tool_parts[1]
            .state
            .as_ref()
            .map(|state| state.status.as_str()),
        Some("completed")
    );
}

#[test]
fn an_empty_session_is_an_empty_list_not_a_failure() {
    let server = StubServer::start("200 OK", "[]".to_string());
    let client = ApiClient::new(server.base_url.clone());

    let messages = client
        .get_messages("ses_empty")
        .expect("an empty list is not an error");
    assert!(messages.is_empty());
}

#[test]
fn a_flat_message_list_is_rejected_instead_of_parsed_into_corrupt_entries() {
    // The regression this whole contract exists to prevent: a flat list, or an `info` block
    // missing its identity fields, must surface as an error. Defaulting them would yield
    // id="", role=user, createdAt=0 — a transcript that looks populated and is entirely wrong.
    let server = StubServer::start(
        "200 OK",
        json!([
            { "id": "msg_1", "sessionID": "ses_01", "role": "assistant", "createdAt": 1_i64,
              "parts": [{ "id": "p1", "type": "text", "text": "flat" }] }
        ])
        .to_string(),
    );
    let client = ApiClient::new(server.base_url);

    let err = client
        .get_messages("ses_01")
        .expect_err("a flat list must not parse as the pinned envelope");
    assert!(
        err.to_string().contains("info"),
        "the failure must name the missing `info` field, got: {err}"
    );
}

#[test]
fn an_info_block_without_identity_fields_is_rejected_not_defaulted() {
    let server = StubServer::start(
        "200 OK",
        json!([{ "info": { "agent": "build" }, "parts": [] }]).to_string(),
    );
    let client = ApiClient::new(server.base_url);

    let err = client
        .get_messages("ses_01")
        .expect_err("missing id/role/createdAt must not be defaulted");
    let message = err.to_string();
    assert!(
        message.contains("missing field `id`"),
        "the failure must name the missing field rather than default it, got: {message}"
    );
    assert!(
        !message.contains("id: \"\""),
        "an empty id must never be reported as a successful parse, got: {message}"
    );
}

#[test]
fn a_server_error_status_is_reported_not_swallowed() {
    let server = StubServer::start("500 Internal Server Error", "boom".to_string());
    let client = ApiClient::new(server.base_url);

    let err = client
        .get_messages("ses_01")
        .expect_err("a 500 must not look like success");
    assert!(
        err.to_string().contains("Failed to get messages"),
        "the failure must be attributable, got: {err}"
    );
}
