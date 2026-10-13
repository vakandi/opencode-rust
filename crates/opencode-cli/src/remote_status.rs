//! Draining a remote turn started by `run --attach`.
//!
//! The `/session/{id}/stream` route answers with that turn's provider stream. When that stream
//! ends, the turn is over — but omo may still have child sessions and background tasks running on
//! the machine that owns the plugin, and the SSE body carries none of that. `/session/status` is
//! the only cross-request signal, so `run --attach` polls it until the session stops being busy.
//!
//! `SESSION_RUN_STATUS` deletes the key when a session goes idle, and the endpoint reports every
//! known session, so a missing key means idle rather than unknown.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDrain {
    Idle,
    TimedOut,
    Unreachable,
}

/// Pure predicate over a decoded `GET /session/status` body.
pub fn is_busy(body: &serde_json::Value, session_id: &str) -> bool {
    body.get(session_id)
        .and_then(|entry| entry.get("busy"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub async fn wait_remote_idle(
    client: &reqwest::Client,
    base_url: &str,
    session_id: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> RemoteDrain {
    let endpoint = format!("{}/session/status", base_url.trim_end_matches('/'));
    let deadline = Instant::now() + timeout;
    let mut unreachable = 0u32;

    loop {
        match client.get(&endpoint).send().await {
            Ok(resp) if resp.status().is_success() => {
                unreachable = 0;
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if !is_busy(&body, session_id) {
                        return RemoteDrain::Idle;
                    }
                }
            }
            _ => {
                // A server that is down must not hang `run` forever; give it a few tries.
                unreachable += 1;
                if unreachable >= 3 {
                    return RemoteDrain::Unreachable;
                }
            }
        }

        if Instant::now() >= deadline {
            return RemoteDrain::TimedOut;
        }
        tokio::time::sleep(poll_interval).await;
    }
}