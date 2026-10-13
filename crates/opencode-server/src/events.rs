//! Server-side event emission for the session event stream.
//!
//! The engine (`opencode_session::SessionPrompt::prompt_with_update_hook`) delivers a
//! full `Session` snapshot on every tick (~20/sec, throttled at 50ms by
//! `prompt::maybe_emit_session_update`). Emitting those snapshots verbatim would be
//! O(session size) per tick, so this module diffs consecutive snapshots and emits
//! only what actually changed, shaped like the TypeScript reference server.

use std::collections::HashMap;

use opencode_session::{MessagePart, PartType, Session};
use serde_json::Value;

/// Event type emitted when a part appears or its full content changes.
pub const EVENT_PART_UPDATED: &str = "message.part.updated";
/// Event type emitted when text or reasoning is appended to an existing part.
pub const EVENT_PART_DELTA: &str = "message.part.delta";
/// Event type emitted when message-level metadata changes.
pub const EVENT_MESSAGE_UPDATED: &str = "message.updated";

/// Snapshot of every part we have already emitted, keyed `message_id -> part_id -> part_type`.
///
/// Kept between ticks so the next snapshot can be diffed against it. Deliberately holds
/// only part types rather than whole `Session` clones: a 200-message session snapshot is
/// large, and cloning it 20x/sec per agent is what makes naive snapshot emission unusable.
#[derive(Debug, Clone, Default)]
pub struct PartIndex {
    parts: HashMap<String, HashMap<String, PartType>>,
}

impl PartIndex {
    /// Build an index from a session snapshot.
    pub fn from_session(session: &Session) -> Self {
        let mut parts: HashMap<String, HashMap<String, PartType>> = HashMap::new();
        for message in &session.messages {
            parts.insert(message.id.clone(), index_message(message));
        }
        Self { parts }
    }

    fn part(&self, message_id: &str, part_id: &str) -> Option<&PartType> {
        self.parts.get(message_id)?.get(part_id)
    }
}

fn index_message(message: &opencode_session::SessionMessage) -> HashMap<String, PartType> {
    message
        .parts
        .iter()
        .map(|part| (part.id.clone(), part.part_type.clone()))
        .collect()
}

/// Wrap event properties in the TypeScript envelope: `{ id, type, properties }`.
///
/// The reference server always frames `event: message` with this shape, and clients select
/// on `type` rather than on the SSE event name.
pub fn envelope(event_type: &str, properties: Value) -> Value {
    serde_json::json!({
        "id": format!("evt_{}", uuid::Uuid::new_v4()),
        "type": event_type,
        "properties": properties,
    })
}

/// Compute the events that turn `prev` into `next`.
///
/// Returns `(event_type, properties)` pairs, already shaped like the TypeScript server:
/// - `message.part.updated` -> `{ sessionID, part, time }`
/// - `message.part.delta`    -> `{ sessionID, messageID, partID, field, delta }`
///
/// `prev` is `None` on the first tick, which emits every part as `message.part.updated`.
///
/// An unchanged session produces **zero** events — that is the property which makes this a
/// real differ rather than a snapshot echo, and it is asserted by the test suite.
pub fn diff_parts(
    prev: Option<&PartIndex>,
    next: &Session,
    now_ms: i64,
) -> Vec<(String, Value)> {
    let mut events = Vec::new();
    let session_id = next.id.clone();

    for message in &next.messages {
        for part in &message.parts {
            match prev.and_then(|index| index.part(&message.id, &part.id)) {
                None => events.push((
                    EVENT_PART_UPDATED.to_string(),
                    part_updated_props(&session_id, part, now_ms),
                )),
                Some(old) => {
                    // Append-only growth on text or reasoning is the hot path: the engine
                    // pushes an accumulated string on every delta, so emitting the whole part
                    // each tick would re-send the entire message 20 times a second.
                    if let Some(delta) = appended_suffix(old, &part.part_type) {
                        if !delta.is_empty() {
                            events.push((
                                EVENT_PART_DELTA.to_string(),
                                part_delta_props(
                                    &session_id,
                                    &message.id,
                                    &part.id,
                                    &delta,
                                ),
                            ));
                            continue;
                        }
                    }
                    if !same_part(old, &part.part_type) {
                        events.push((
                            EVENT_PART_UPDATED.to_string(),
                            part_updated_props(&session_id, part, now_ms),
                        ));
                    }
                }
            }
        }
    }

    events
}

/// If `new` is `old` plus an appended suffix, return that suffix.
///
/// Returning `None` for the shrink case matters: if the engine ever rewrites a part rather
/// than appending, we must fall through to a full `part.updated` instead of emitting a delta
/// that would corrupt the client's accumulated text.
fn appended_suffix(old: &PartType, new: &PartType) -> Option<String> {
    match (old, new) {
        (
            PartType::Text { text: o, .. },
            PartType::Text { text: n, .. },
        ) => suffix_if_appended(o, n),
        (PartType::Reasoning { text: o }, PartType::Reasoning { text: n }) => {
            suffix_if_appended(o, n)
        }
        _ => None,
    }
}

fn suffix_if_appended(old: &str, new: &str) -> Option<String> {
    if new.len() > old.len() && new.starts_with(old) {
        Some(new[old.len()..].to_string())
    } else {
        None
    }
}

/// Structural equality without requiring `PartialEq` on `PartType`.
fn same_part(a: &PartType, b: &PartType) -> bool {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn part_updated_props(session_id: &str, part: &MessagePart, now_ms: i64) -> Value {
    serde_json::json!({
        "sessionID": session_id,
        "part": part_to_ts_json(part, session_id),
        "time": { "start": now_ms },
    })
}

fn part_delta_props(
    session_id: &str,
    message_id: &str,
    part_id: &str,
    delta: &str,
) -> Value {
    serde_json::json!({
        "sessionID": session_id,
        "messageID": message_id,
        "partID": part_id,
        "field": "text",
        "delta": delta,
    })
}

/// Serialize one message into the reference server's `{ info, parts }` envelope.
pub fn message_to_ts_json(message: &opencode_session::SessionMessage, session_id: &str) -> Value {
    let info = serde_json::json!({
        "id": message.id,
        "sessionID": session_id,
        "role": role_name(&message.role),
        "createdAt": message.created_at.timestamp_millis(),
        "agent": message.metadata.get("agent"),
        "model": message.metadata.get("model_id"),
        "metadata": message.metadata,
    });
    let parts: Vec<Value> = message
        .parts
        .iter()
        .map(|part| part_to_ts_json(part, session_id))
        .collect();
    serde_json::json!({ "info": info, "parts": parts })
}

/// Serialize every message of a session, preserving order.
pub fn messages_to_ts_json(session: &Session) -> Vec<Value> {
    session
        .messages
        .iter()
        .map(|message| message_to_ts_json(message, &session.id))
        .collect()
}

fn role_name(role: &opencode_session::MessageRole) -> &'static str {
    use opencode_session::MessageRole;
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    }
}

/// Serialize a part into the TypeScript `Part` shape.
///
/// The internal `PartType` uses `camelCase` discriminants (`stepStart`, `toolCall`) and
/// models a tool call and its result as two sibling parts. The reference server uses kebab
/// discriminants (`step-start`) and a single `tool` part carrying `state`. This is the one
/// place that reconciliation happens.
pub fn part_to_ts_json(part: &MessagePart, session_id: &str) -> Value {
    let base = serde_json::json!({
        "id": part.id,
        "sessionID": session_id,
        "messageID": part.message_id,
    });
    let start = part.created_at.timestamp_millis();

    let body = match &part.part_type {
        PartType::Text { text, synthetic, ignored } => serde_json::json!({
            "type": "text",
            "text": text,
            "synthetic": synthetic,
            "ignored": ignored,
            "time": { "start": start, "end": null },
        }),
        PartType::Reasoning { text } => serde_json::json!({
            "type": "reasoning",
            "text": text,
            "time": { "start": start, "end": null },
        }),
        // Rust models a call and its result separately; TS folds both into one `tool` part
        // whose `state` moves pending -> running -> completed|error.
        PartType::ToolCall { id, name, input } => serde_json::json!({
            "type": "tool",
            "callID": id,
            "tool": name,
            "state": { "status": "pending", "input": input, "raw": "" },
            "time": { "start": start, "end": null },
        }),
        PartType::ToolResult { tool_call_id, content, is_error } => serde_json::json!({
            "type": "tool",
            "callID": tool_call_id,
            "state": {
                "status": if *is_error { "error" } else { "completed" },
                "output": content,
            },
            "time": { "start": start, "end": null },
        }),
        PartType::File { url, filename, mime } => serde_json::json!({
            "type": "file",
            "url": url,
            "filename": filename,
            "mime": mime,
        }),
        PartType::StepStart { id, name } => serde_json::json!({
            "type": "step-start",
            "id": id,
            "name": name,
            "snapshot": Value::Null,
        }),
        PartType::StepFinish { id, output } => serde_json::json!({
            "type": "step-finish",
            "id": id,
            "reason": output.clone().unwrap_or_default(),
            "snapshot": Value::Null,
            "cost": 0,
            "tokens": { "input": 0, "output": 0, "reasoning": 0,
                        "cacheRead": 0, "cacheWrite": 0 },
        }),
        PartType::Snapshot { content } => serde_json::json!({
            "type": "snapshot",
            "snapshot": content,
        }),
        PartType::Patch { old_string, new_string, filepath } => serde_json::json!({
            "type": "patch",
            "oldString": old_string,
            "newString": new_string,
            "path": filepath,
        }),
        PartType::Agent { name, status } => serde_json::json!({
            "type": "agent",
            "name": name,
            "status": status,
        }),
        PartType::Subtask { id, description, status } => serde_json::json!({
            "type": "subtask",
            "id": id,
            "description": description,
            "status": status,
        }),
        PartType::Retry { count, reason } => serde_json::json!({
            "type": "retry",
            "count": count,
            "reason": reason,
        }),
        PartType::Compaction { summary } => serde_json::json!({
            "type": "compaction",
            "summary": summary,
        }),
    };

    merge(base, body)
}

fn merge(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v) in b {
                a.insert(k, v);
            }
            Value::Object(a)
        }
        (_, overlay) => overlay,
    }
}

/// Incremental diff state for one live session.
///
/// Holds only the parts of the **last** message. While streaming, the engine appends to
/// that message and nothing else changes, so the common tick costs one message's worth of
/// work instead of the whole session. A new message id resets the index and announces
/// every part as new.
///
/// This exists so the update hook can diff without cloning the session: the hook receives
/// `&Session` and is synchronous, so it can fold the snapshot in place and emit directly.
#[derive(Debug, Default)]
pub struct StreamTracker {
    message_id: Option<String>,
    parts: HashMap<String, PartType>,
}

impl StreamTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one snapshot in, returning only what changed since the previous call.
    pub fn observe(&mut self, session: &Session, now_ms: i64) -> Vec<(String, Value)> {
        let Some(message) = session.messages.last() else {
            self.message_id = None;
            self.parts.clear();
            return Vec::new();
        };

        let session_id = session.id.as_str();
        let mut events = Vec::new();

        if self.message_id.as_deref() != Some(message.id.as_str()) {
            for part in &message.parts {
                events.push((
                    EVENT_PART_UPDATED.to_string(),
                    part_updated_props(session_id, part, now_ms),
                ));
            }
        } else {
            for part in &message.parts {
                match self.parts.get(&part.id) {
                    None => events.push((
                        EVENT_PART_UPDATED.to_string(),
                        part_updated_props(session_id, part, now_ms),
                    )),
                    Some(old) => {
                        if let Some(delta) = appended_suffix(old, &part.part_type) {
                            if !delta.is_empty() {
                                events.push((
                                    EVENT_PART_DELTA.to_string(),
                                    part_delta_props(
                                        session_id,
                                        &message.id,
                                        &part.id,
                                        &delta,
                                    ),
                                ));
                            }
                        } else if !same_part(old, &part.part_type) {
                            events.push((
                                EVENT_PART_UPDATED.to_string(),
                                part_updated_props(session_id, part, now_ms),
                            ));
                        }
                    }
                }
            }
        }

        self.message_id = Some(message.id.clone());
        self.parts = message
            .parts
            .iter()
            .map(|part| (part.id.clone(), part.part_type.clone()))
            .collect();

        events
    }
}
