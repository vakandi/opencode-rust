//! Error-triggered rotation. The reference plugin rotated on a *timer*, which is the
//! one behaviour that cannot work: a free tier answers 403/429 the second a request
//! lands, long before the next tick, and a timer that fires mid-turn burns a healthy
//! egress for nothing.
//!
//! This module is the classifier the timer never had. It reads the provider error the
//! turn actually received and decides whether a different egress can fix it. The
//! strings below are not invented: they are the exact bodies stored in
//! `server/data/opencode/opencode.db` for the 2026-10-23 sessions, and the unit tests
//! pin them verbatim.
//!
//! The distinction that matters is **reputation vs configuration**. A `403
//! FreeTierError` ("abusive traffic from this IP") or a `429 FreeUsageLimitError` is
//! about the network the request left from, so rotating fixes it. A `410
//! ModelDeprecated`, a `400 Model not found`, or a `401` is about the request itself;
//! rotating only burns a good proxy and delays the same failure by one round trip.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Sent as the next user message after a rotation. Same wording as the reference
/// plugin: it must tell the model the failure was external, or it restarts the task
/// from scratch and loses everything it had already done.
pub const RESUME_NUDGE: &str = "Proxy rotated after a Zen rate limit. Continue exactly where you left off — retry the last failed step, do not restart the task.";

/// How many rotations one turn may cause. Past this the egress is not the problem.
pub const MAX_ROTATIONS_PER_TURN: u32 = 3;

/// How many rotations one session may cause per hour, across turns. Without the cap a
/// session that is pinned to a dead model rotates on every retry and empties the pool.
pub const MAX_ROTATIONS_PER_HOUR: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Not an egress problem. Do not rotate, do not resume.
    Ignore,
    /// The request left from an egress the provider refuses. Rotate and resume.
    Rotate,
}

impl Verdict {
    pub fn rotates(self) -> bool {
        matches!(self, Verdict::Rotate)
    }
}

/// Errors that are never about the network. Checked first, because several of them
/// carry a "403"/"429"-shaped status but name a configuration fault in the body.
const NEVER_ROTATE: &[&str] = &[
    "ModelDeprecated",
    "not found for provider",
    "Model not found",
    "invalid_request_error",
    "invalid api key",
    "Incorrect API key",
    "unauthorized",
    "MessageAbortedError",
    "Aborted",
    "encrypted_content was not issued",
];

/// The provider refused the *network* the request came from.
const EGREGIOUS_EGRESS: &[&str] = &[
    "FreeTierError",
    "abusive traffic from this IP",
    "FreeUsageLimitError",
    "GoUsageLimitError",
    "account_rate_limit",
    "free tier can only be used",
    "Free usage limit reached",
    "Free limit reached",
    "subscribe to Go",
    "429 Too Many Requests",
    "rate_limit_exceeded",
    "Rate limit exceeded",
    "service_overloaded",
    "temporarily overloaded",
];

/// The tunnel itself failed. A different egress is the only cure that does not need a
/// code change.
const TRANSPORT: &[&str] = &[
    "Cannot connect to API",
    "ECONNREFUSED",
    "socket connection was closed unexpectedly",
    "socket closed",
    "Unable to connect",
    "Endpoint is unavailable",
    "Network error: error sending request",
    "502 Bad Gateway",
    "503 Service Unavailable",
    "504 Gateway Timeout",
    "500 Internal Server Error",
    "Upstream request failed",
];

/// Classify a provider error string into the only decision that matters here.
pub fn classify(error: &str) -> Verdict {
    let text = error.trim();
    if text.is_empty() {
        return Verdict::Ignore;
    }
    if NEVER_ROTATE.iter().any(|needle| text.contains(needle)) {
        return Verdict::Ignore;
    }
    if EGREGIOUS_EGRESS.iter().any(|needle| text.contains(needle)) {
        return Verdict::Rotate;
    }
    if TRANSPORT.iter().any(|needle| text.contains(needle)) {
        return Verdict::Rotate;
    }
    Verdict::Ignore
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

static RESUME_BUDGET: Mutex<Option<HashMap<String, Vec<i64>>>> = Mutex::new(None);

/// Take one rotation from the session's hourly budget. `false` means the budget is
/// spent and the caller must surface the error instead of rotating again.
pub fn budget_take(session_id: &str) -> bool {
    let now = now_secs();
    let mut guard = match RESUME_BUDGET.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let map = guard.get_or_insert_with(HashMap::new);
    let stamps = map.entry(session_id.to_string()).or_default();
    stamps.retain(|stamp| now - *stamp < 3600);
    if stamps.len() >= MAX_ROTATIONS_PER_HOUR {
        return false;
    }
    stamps.push(now);
    true
}

/// Give the budget back. Called when a rotation happened but no resume could be sent,
/// so a failed rotation does not consume the allowance.
pub fn budget_refund(session_id: &str) {
    let mut guard = match RESUME_BUDGET.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(map) = guard.as_mut() {
        if let Some(stamps) = map.get_mut(session_id) {
            stamps.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from `server/data/opencode/opencode.db`, 2026-10-23 19:30–21:03.
    const FREE_USAGE_LIMIT: &str = r#"API error: 429 Too Many Requests: {"type":"error","error":{"type":"FreeUsageLimitError","message":"Free usage limit reached. Subscribe to OpenCode Go to keep going: https://opencode.ai/go"}}"#;
    /// Verbatim from the 2026-10-23 sessions that predate the free-tier abuse block.
    const FREE_TIER_403: &str = r#"API error: 403 Forbidden: {"type":"error","error":{"type":"FreeTierError","message":"OpenCode's free models aren't available from your current network because we've detected abusive traffic from this IP address"}}"#;
    /// Verbatim: the space-bunny retirement, which rotation cannot fix.
    const MODEL_DEPRECATED: &str = r#"API error: 410 Gone: {"type":"error","error":{"type":"ModelDeprecated","message":"Model space-bunny-free has been deprecated. Use minimax-m3.1-flash-free instead."},"metadata":{"model":"space-bunny-free"}}"#;
    /// Verbatim: the minimax removal the engine reported as a 400.
    const MODEL_NOT_FOUND: &str = r#"API error: 400 Bad Request: {"error":{"message":"Model `minimax-m3.1-flash-free` not found for provider `opencode`","type":"bad_request"}}"#;
    /// Verbatim from 2026-10-23 21:03.
    const NETWORK: &str = "Network error: error sending request for url (https://opencode.ai/inference/openai/v1/chat/completions)";

    #[test]
    fn free_usage_limit_rotates() {
        assert_eq!(classify(FREE_USAGE_LIMIT), Verdict::Rotate);
    }

    #[test]
    fn abusive_ip_rotates() {
        assert_eq!(classify(FREE_TIER_403), Verdict::Rotate);
    }

    #[test]
    fn network_failure_rotates() {
        assert_eq!(classify(NETWORK), Verdict::Rotate);
    }

    #[test]
    fn deprecated_model_never_rotates() {
        assert_eq!(classify(MODEL_DEPRECATED), Verdict::Ignore);
    }

    #[test]
    fn unknown_model_never_rotates() {
        assert_eq!(classify(MODEL_NOT_FOUND), Verdict::Ignore);
    }

    #[test]
    fn abort_never_rotates() {
        assert_eq!(classify("MessageAbortedError: Aborted"), Verdict::Ignore);
    }

    #[test]
    fn empty_is_ignored() {
        assert_eq!(classify("   "), Verdict::Ignore);
    }

    #[test]
    fn hourly_budget_is_capped() {
        let session = "ses_budget_test_unique";
        for _ in 0..MAX_ROTATIONS_PER_HOUR {
            assert!(budget_take(session));
        }
        assert!(!budget_take(session), "fourth rotation must be refused");
    }
}
