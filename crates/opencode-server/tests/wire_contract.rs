//! Wire contract parity with the TypeScript reference server.
//!
//! Four defects were reproduced live against `opencode serve` and are pinned here:
//!
//!   A. `POST /session` silently dropped `agent`
//!   B. `GET /session/status` had no `type` field, which is the key every client reads
//!   C. `POST /session/:id/message` 422'd on the reference server's `parts[]` payload
//!   D. `POST /session/:id/message` answered with a flat `MessageInfo` while
//!      `GET /session/:id/message` answered with the migrated `{info, parts}` envelope
//!
//! These are HTTP-level tests: they drive the real `Router` through `tower`'s
//! `oneshot`, so a passing test proves the handler — not a serde shim — behaves.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencode_server::{routes, ServerState};
use serde_json::{json, Value};
use tower::ServiceExt;

type App = axum::Router;

// --------------------------------------------------------------------------------------
// harness
// --------------------------------------------------------------------------------------

fn app() -> App {
    routes::router().with_state(Arc::new(ServerState::new()))
}

async fn post_json(app: &App, path: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request should build");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should read");
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            panic!(
                "{} returned non-JSON body ({}): {} ({})",
                path,
                status,
                String::from_utf8_lossy(&bytes),
                err
            )
        })
    };
    (status, parsed)
}

async fn get_json(app: &App, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("request should build");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should read");
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("body should be JSON")
    };
    (status, parsed)
}

/// Create a session and return its id, asserting the create call itself succeeded.
async fn new_session(app: &App) -> String {
    let (status, body) = post_json(app, "/session", json!({ "title": "wire-contract" })).await;
    assert_eq!(status, StatusCode::OK, "session create failed: {}", body);
    body["id"]
        .as_str()
        .expect("created session must expose an id")
        .to_string()
}

/// The 400 body shape every malformed-payload test asserts on.
fn error_message(body: &Value) -> String {
    body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("400 body must carry error.message, got {}", body))
        .to_string()
}

// --------------------------------------------------------------------------------------
// Task A — `agent` on session create
// --------------------------------------------------------------------------------------

/// `POST /session {"agent":"gilfoyle"}` returned a session with no `agent` field at all:
/// the request body was accepted, the value was discarded, and the title came back
/// auto-generated. The agent must be persisted under `session.metadata["agent"]` — the
/// same key `session_prompt` already writes — so there is one source of truth.
#[tokio::test]
async fn create_session_accepts_agent() {
    let app = app();
    let (status, body) = post_json(
        &app,
        "/session",
        json!({ "title": "probe", "agent": "gilfoyle" }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "create failed: {}", body);
    assert_eq!(
        body["agent"].as_str(),
        Some("gilfoyle"),
        "the created session must reflect the requested agent, got {}",
        body
    );

    let id = body["id"].as_str().expect("session id").to_string();
    let (get_status, fetched) = get_json(&app, &format!("/session/{}", id)).await;
    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(
        fetched["agent"].as_str(),
        Some("gilfoyle"),
        "the agent must be persisted, not just echoed on create: {}",
        fetched
    );
}

// --------------------------------------------------------------------------------------
// Task B — `type` on session status
// --------------------------------------------------------------------------------------

/// Clients branch on `status["type"] == "busy" | "idle"`. The live response was
/// `{"status":"active","idle":true,"busy":false}` with no `type` key, so every client
/// fell through to its default branch.
#[tokio::test]
async fn session_status_exposes_type_field() {
    let app = app();
    let id = new_session(&app).await;

    let (status, body) = get_json(&app, "/session/status").await;
    assert_eq!(status, StatusCode::OK, "status failed: {}", body);

    let entry = body
        .get(&id)
        .unwrap_or_else(|| panic!("session {} missing from status map {}", id, body));
    let run_type = entry["type"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "every status entry must expose `type`; got {} for session {}",
                entry, id
            )
        })
        .to_string();

    assert!(
        ["idle", "busy", "retry"].contains(&run_type.as_str()),
        "`type` must be one of idle/busy/retry, got {:?} in {}",
        run_type,
        entry
    );

    // The pre-existing fields are a superset current consumers read; they must survive.
    for field in ["status", "idle", "busy"] {
        assert!(
            entry.get(field).is_some(),
            "`{}` was removed from the status entry; existing consumers read it: {}",
            field,
            entry
        );
    }
}

// --------------------------------------------------------------------------------------
// Task C — the message endpoint accepts the reference payload
// --------------------------------------------------------------------------------------

/// The reference server's clients send `parts[]`. This body returned
/// `422 missing field 'content'`, so the endpoint failed for every real client.
#[tokio::test]
async fn send_message_accepts_reference_parts_array() {
    let app = app();
    let id = new_session(&app).await;

    let (status, body) = post_json(
        &app,
        &format!("/session/{}/message", id),
        json!({ "parts": [{ "type": "text", "text": "say hi" }] }),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the reference parts[] payload must not be rejected as unprocessable: {}",
        body
    );
    assert_eq!(status, StatusCode::OK, "parts[] send failed: {}", body);
    assert!(
        body.get("info").is_some(),
        "a successful send must return the message envelope: {}",
        body
    );
}

/// The two existing endpoints disagreed on `model` (`Option<String>` here,
/// `Option<serde_json::Value>` on the prompt path). Both shapes must now be accepted.
#[tokio::test]
async fn send_message_accepts_model_as_string_and_as_object() {
    let app = app();
    let id = new_session(&app).await;
    let path = format!("/session/{}/message", id);

    let (string_status, string_body) = post_json(
        &app,
        &path,
        json!({ "content": "hi", "model": "anthropic/claude-sonnet-4-5" }),
    )
    .await;
    assert_ne!(
        string_status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a bare-string model must be accepted, got {}: {}",
        string_status,
        string_body
    );

    let (object_status, object_body) = post_json(
        &app,
        &path,
        json!({
            "content": "hi again",
            "model": { "providerID": "anthropic", "modelID": "claude-sonnet-4-5" },
        }),
    )
    .await;
    assert_ne!(
        object_status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a {{providerID, modelID}} model object must be accepted, got {}: {}",
        object_status,
        object_body
    );
    assert_eq!(
        object_status, StatusCode::OK,
        "object model send failed: {}",
        object_body
    );
}

// --------------------------------------------------------------------------------------
// Task C — validation. 400, never 500 and never a silent success.
// --------------------------------------------------------------------------------------

#[tokio::test]
async fn send_message_rejects_missing_content_and_parts() {
    let app = app();
    let id = new_session(&app).await;

    let (status, body) = post_json(&app, &format!("/session/{}/message", id), json!({})).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an empty body must be rejected as 400, not 422/500/silent-success: {}",
        body
    );
    let message = error_message(&body);
    assert!(
        message.contains("content") && message.contains("parts"),
        "the 400 body must name both accepted fields, got {:?}",
        message
    );
}

#[tokio::test]
async fn send_message_rejects_empty_parts_array() {
    let app = app();
    let id = new_session(&app).await;

    let (status, body) = post_json(
        &app,
        &format!("/session/{}/message", id),
        json!({ "parts": [] }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "`parts: []` carries no content and must be rejected as 400: {}",
        body
    );
    error_message(&body);
}

#[tokio::test]
async fn send_message_rejects_malformed_part_naming_index() {
    let app = app();
    let id = new_session(&app).await;

    let (status, body) = post_json(
        &app,
        &format!("/session/{}/message", id),
        json!({
            "parts": [
                { "type": "text", "text": "fine" },
                { "type": "totally-not-a-real-part" }
            ]
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a malformed part must be rejected as 400: {}",
        body
    );
    let message = error_message(&body);
    assert!(
        message.contains('1'),
        "the 400 body must name the offending index 1, got {:?}",
        message
    );
}

// --------------------------------------------------------------------------------------
// Task D — one envelope for the same resource
// --------------------------------------------------------------------------------------

/// `GET /session/:id/message` was migrated to `[{info, parts}]` but
/// `POST /session/:id/message` still answered with a flat snake_case `MessageInfo`.
#[tokio::test]
async fn send_message_response_matches_list_envelope() {
    let app = app();
    let id = new_session(&app).await;
    let path = format!("/session/{}/message", id);

    let (post_status, posted) = post_json(
        &app,
        &path,
        json!({ "parts": [{ "type": "text", "text": "envelope check" }] }),
    )
    .await;
    assert_eq!(post_status, StatusCode::OK, "send failed: {}", posted);

    let mut post_keys: Vec<&str> = posted
        .as_object()
        .expect("send response must be an object")
        .keys()
        .map(String::as_str)
        .collect();
    post_keys.sort_unstable();
    assert_eq!(
        post_keys,
        vec!["info", "parts"],
        "POST must nest exactly {{info, parts}} like GET does, got {:?}",
        post_keys
    );

    let (get_status, listed) = get_json(&app, &path).await;
    assert_eq!(get_status, StatusCode::OK);
    let listed = listed.as_array().expect("list must be an array");
    assert!(!listed.is_empty(), "the send should have produced a message");

    for entry in listed {
        let mut keys: Vec<&str> = entry
            .as_object()
            .expect("each list entry must be an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["info", "parts"],
            "POST and GET disagree on the envelope for the same resource: {:?}",
            keys
        );
    }

    // The two endpoints must agree on the identity of the message that was just created.
    let posted_id = posted["info"]["id"].as_str().expect("info.id");
    assert!(
        listed.iter().any(|e| e["info"]["id"].as_str() == Some(posted_id)),
        "the message returned by POST must appear in the GET list"
    );
}