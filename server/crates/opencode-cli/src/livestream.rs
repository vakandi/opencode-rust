//! Streaming the live output of child sessions.
//!
//! Each row in `part` is one frame of a child session's output (`data` holds JSON with a
//! `type` of `text`, `reasoning`, `tool`, `step-finish`, `patch`…). Polling by primary key
//! gives an incremental cursor so a run can print a child's output as it is produced.

use std::path::PathBuf;

use sqlx::Row;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildFrame {
    pub session_id: String,
    pub part_id: String,
    pub kind: String,
    pub text: String,
}

/// Parts written after `after` for one session, oldest first.
pub async fn poll_frames(db: &std::path::Path, session_id: &str, after: &str) -> Vec<ChildFrame> {
    if !db.exists() {
        return Vec::new();
    }
    let url = format!("sqlite://{}?mode=ro", db.display());
    let Ok(pool) = sqlx::SqlitePool::connect(&url).await else {
        return Vec::new();
    };

    // `id` is monotonically increasing in omo's storage, so it doubles as the cursor: frames
    // already reported are excluded and a poller never replays output the user has seen.
    let rows = sqlx::query(
        "SELECT id, data FROM part WHERE session_id = ? AND id > ? ORDER BY id",
    )
    .bind(session_id)
    .bind(after)
    .fetch_all(&pool)
    .await;

    let Ok(rows) = rows else {
        return Vec::new();
    };

    rows.into_iter()
        .map(|row| {
            let part_id: String = row.get::<String, _>("id");
            let raw: String = row.get::<String, _>("data");
            let parsed: serde_json::Value =
                serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
            ChildFrame {
                session_id: session_id.to_string(),
                part_id,
                kind: parsed
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                text: match parsed.get("text").and_then(|v| v.as_str()) {
                    Some(text) => text.to_string(),
                    None => parsed
                        .get("tool")
                        .and_then(|v| v.get("state"))
                        .and_then(|s| s.get("output"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                },
            }
        })
        .collect()
}