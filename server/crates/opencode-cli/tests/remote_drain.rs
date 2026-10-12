//! `run --attach` must not return while the remote turn is still working.
//!
//! The `/stream` SSE body ends when the provider stream ends, which is not the same as the session
//! being finished. These pin the predicate that decides "still working", using the exact payload
//! `GET /session/status` emits.

use opencode_cli::remote_status::{is_busy, RemoteDrain};

#[test]
fn a_busy_session_is_reported_as_busy() {
    let body = serde_json::json!({
        "ses_a": { "type": "busy", "status": "busy", "idle": false, "busy": true }
    });
    assert!(is_busy(&body, "ses_a"));
}

#[test]
fn an_idle_session_is_not_busy() {
    let body = serde_json::json!({
        "ses_a": { "type": "idle", "status": "active", "idle": true, "busy": false }
    });
    assert!(!is_busy(&body, "ses_a"));
}

#[test]
fn a_retrying_session_counts_as_busy() {
    // `retry` means the server is still working on the turn; releasing here would cut it off.
    let body = serde_json::json!({
        "ses_a": { "type": "retry", "status": "retry", "idle": false, "busy": true, "attempt": 2 }
    });
    assert!(is_busy(&body, "ses_a"));
}

#[test]
fn a_missing_session_is_not_busy() {
    // `SESSION_RUN_STATUS` deletes the key once a session goes idle, so absence means finished.
    let body = serde_json::json!({ "ses_other": { "type": "idle", "idle": true, "busy": false } });
    assert!(!is_busy(&body, "ses_a"));
}

#[test]
fn a_malformed_body_is_not_busy() {
    assert!(!is_busy(&serde_json::json!(null), "ses_a"));
    assert!(!is_busy(&serde_json::json!({ "ses_a": "nonsense" }), "ses_a"));
}

#[tokio::test]
async fn an_unreachable_server_does_not_hang_forever() {
    let client = reqwest::Client::new();
    let started = std::time::Instant::now();
    // Port 1 is reserved and never listening.
    let report = opencode_cli::remote_status::wait_remote_idle(
        &client,
        "http://127.0.0.1:1",
        "ses_a",
        std::time::Duration::from_secs(30),
        std::time::Duration::from_millis(50),
    )
    .await;

    assert_eq!(report, RemoteDrain::Unreachable);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "an unreachable server must fail fast, not stall run"
    );
}