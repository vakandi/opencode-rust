//! Livestream fidelity contract.
//!
//! The user's requirement: a session started from a `PROMPT.md` or an API payload must
//! produce a live stream of the agent's AI tools, AI reasoning and AI messages, and that
//! stream must be provably real rather than a snapshot echo.
//!
//! Scenario ids map to the contract in `.ulw/notepad.md`:
//!   S1 = happy path      S2 = edge/boundary      S3 = adjacent-surface regression
//!   N1..N6 = negative controls proving the stream is not a stub

use std::collections::HashMap;

use chrono::Utc;
use opencode_server::events::{
    diff_parts, messages_to_ts_json, PartIndex, EVENT_PART_DELTA, EVENT_PART_UPDATED,
};
use opencode_session::{
    MessagePart, MessageRole, PartType, Session, SessionMessage, SessionTime,
};
use serde_json::Value;

// --------------------------------------------------------------------------------------
// fixtures
// --------------------------------------------------------------------------------------

fn session_with_assistant_parts(parts: Vec<PartType>) -> Session {
    let mut session = Session::new("default", "/tmp/project");
    session.time = SessionTime::default();
    session.messages.push(SessionMessage {
        id: "msg_assistant".into(),
        session_id: session.id.clone(),
        role: MessageRole::Assistant,
        parts: parts
            .into_iter()
            .enumerate()
            .map(|(i, part_type)| MessagePart {
                id: format!("prt_{i}"),
                part_type,
                created_at: Utc::now(),
                message_id: Some("msg_assistant".into()),
            })
            .collect(),
        created_at: Utc::now(),
        metadata: HashMap::new(),
        usage: None,
    });
    session
}

fn text(t: &str) -> PartType {
    PartType::Text {
        text: t.into(),
        synthetic: None,
        ignored: None,
    }
}

fn reasoning(t: &str) -> PartType {
    PartType::Reasoning { text: t.into() }
}

fn count(events: &[(String, Value)], event_type: &str) -> usize {
    events.iter().filter(|(t, _)| t == event_type).count()
}

fn deltas(events: &[(String, Value)]) -> Vec<String> {
    events
        .iter()
        .filter(|(t, _)| t == EVENT_PART_DELTA)
        .filter_map(|(_, props)| props.get("delta").and_then(Value::as_str).map(String::from))
        .collect()
}

// --------------------------------------------------------------------------------------
// WAVE 0 — baseline. Pins the defect as a failing assertion.
// --------------------------------------------------------------------------------------

/// S1/Wave-0: the update hook receives a full snapshot and must emit part data.
/// Today it emits nothing, so this asserts on the count and fails with a real message.
#[test]
fn baseline_event_payload_drops_part_data() {
    let session = session_with_assistant_parts(vec![text("alpha beta gamma")]);

    let events = diff_parts(None, &session, 1_700_000_000_000);

    assert!(
        !events.is_empty(),
        "diff_parts emitted 0 events for a session holding 1 text part — \
         the update hook is discarding the snapshot, so no text/reasoning/tool data can \
         ever reach GET /event"
    );
    assert_eq!(
        count(&events, EVENT_PART_UPDATED),
        1,
        "first tick must emit exactly one message.part.updated per existing part, got {:?}",
        events
    );
}

// --------------------------------------------------------------------------------------
// N1 — CORE STUB DETECTOR. Identical snapshots must emit nothing.
// --------------------------------------------------------------------------------------

/// A snapshot-echoing implementation returns N events for an unchanged session.
/// A real differ returns zero. This is the test that makes the whole design falsifiable.
#[test]
fn n1_identical_snapshots_emit_nothing() {
    let session = session_with_assistant_parts(vec![text("alpha beta gamma")]);
    let index = PartIndex::from_session(&session);

    let events = diff_parts(Some(&index), &session, 1_700_000_000_001);

    assert_eq!(
        events.len(),
        0,
        "an unchanged session produced {} events; a snapshot echo would re-emit the whole \
         session every tick",
        events.len()
    );
}

// --------------------------------------------------------------------------------------
// S1 — text arrives as incremental deltas, not one replacement blob.
// --------------------------------------------------------------------------------------

#[test]
fn s1_text_growth_emits_delta_not_replacement() {
    let before = session_with_assistant_parts(vec![text("Hel")]);
    let after = session_with_assistant_parts(vec![text("Hello")]);
    let index = PartIndex::from_session(&before);

    let events = diff_parts(Some(&index), &after, 1_700_000_000_002);

    assert_eq!(
        deltas(&events),
        vec!["lo".to_string()],
        "expected exactly the appended suffix as a delta"
    );
    assert_eq!(
        count(&events, EVENT_PART_UPDATED),
        0,
        "a pure append must not also emit message.part.updated (that would be a replacement)"
    );
}

/// N2 — monotonic, non-repeating. This is what actually kills a snapshot-echo.
///
/// The first tick legitimately emits a full `part.updated` (there is no previous state to
/// diff against), then each append emits only the new suffix. The invariant that matters is
/// what a client reconstructs: part.updated text + concatenated deltas == the full text.
/// A snapshot-echo fails this because its deltas repeat the accumulated prefix.
#[test]
fn n2_deltas_are_monotonic_and_non_repeating() {
    let scripted = ["alpha ", "beta ", "gamma"];
    let mut accumulated = String::new();
    let mut index: Option<PartIndex> = None;
    let mut initial: Option<String> = None;
    let mut observed: Vec<String> = Vec::new();

    for chunk in scripted {
        accumulated.push_str(chunk);
        let snapshot = session_with_assistant_parts(vec![text(&accumulated)]);
        let events = diff_parts(index.as_ref(), &snapshot, 1_700_000_000_003);

        for (event_type, props) in &events {
            if event_type == EVENT_PART_UPDATED && initial.is_none() {
                initial = props
                    .get("part")
                    .and_then(|p| p.get("text"))
                    .and_then(Value::as_str)
                    .map(String::from);
            }
        }
        observed.extend(deltas(&events));
        index = Some(PartIndex::from_session(&snapshot));
    }

    let reconstructed = format!("{}{}", initial.unwrap_or_default(), observed.join(""));

    assert_eq!(
        observed,
        vec!["beta ".to_string(), "gamma".to_string()],
        "each append must emit only its own suffix"
    );
    assert_eq!(
        reconstructed, "alpha beta gamma",
        "part.updated text + concatenated deltas must reconstruct the full text; a snapshot \
         echo reconstructs \"alpha alpha beta alpha beta gamma\" and fails here"
    );
    assert!(
        observed.len() >= 2,
        "expected >= 2 distinct deltas to prove incremental streaming, got {:?}",
        observed
    );
}

// --------------------------------------------------------------------------------------
// S1 — reasoning reaches the wire.
// --------------------------------------------------------------------------------------

#[test]
fn s1_reasoning_growth_emits_delta() {
    let before = session_with_assistant_parts(vec![reasoning("think")]);
    let after = session_with_assistant_parts(vec![reasoning("thinking harder")]);
    let index = PartIndex::from_session(&before);

    let events = diff_parts(Some(&index), &after, 1_700_000_000_004);

    assert_eq!(
        deltas(&events),
        vec!["ing harder".to_string()],
        "AI reasoning must stream as deltas like text does"
    );
}

#[test]
fn s1_new_reasoning_part_emits_part_updated() {
    let session = session_with_assistant_parts(vec![reasoning("let me consider")]);

    let events = diff_parts(None, &session, 1_700_000_000_005);

    assert_eq!(count(&events, EVENT_PART_UPDATED), 1);
    let (_, props) = &events[0];
    let part = props.get("part").expect("properties.part missing");
    assert_eq!(
        part.get("type").and_then(Value::as_str),
        Some("reasoning"),
        "part discriminator must be \"reasoning\", got {:?}",
        part
    );
    assert_eq!(
        part.get("text").and_then(Value::as_str),
        Some("let me consider")
    );
}

// --------------------------------------------------------------------------------------
// N3 — ordering: the stream is a timeline, not a re-serialization.
// --------------------------------------------------------------------------------------

#[test]
fn n3_text_and_reasoning_interleave_in_arrival_order() {
    let steps = [text("t1"), reasoning("r1"), text("t2")];

    let mut index: Option<PartIndex> = None;
    let mut order: Vec<String> = Vec::new();

    for (i, part) in steps.iter().enumerate() {
        let snapshot = session_with_assistant_parts(vec![part.clone()]);
        let events = diff_parts(index.as_ref(), &snapshot, 1_700_000_000_006 + i as i64);
        for (t, props) in &events {
            let kind = props
                .get("part")
                .and_then(|p| p.get("type"))
                .and_then(Value::as_str)
                .unwrap_or(t.as_str())
                .to_string();
            order.push(kind);
        }
        index = Some(PartIndex::from_session(&snapshot));
    }

    assert_eq!(
        order,
        vec!["text", "reasoning", "text"],
        "events must arrive in the order the engine produced them"
    );
}

// --------------------------------------------------------------------------------------
// S1 — tool activity reaches the wire.
// --------------------------------------------------------------------------------------

#[test]
fn s1_tool_call_emits_part_updated() {
    let session = session_with_assistant_parts(vec![PartType::ToolCall {
        id: "call_1".into(),
        name: "bash".into(),
        input: serde_json::json!({ "command": "ls" }),
    }]);

    let events = diff_parts(None, &session, 1_700_000_000_007);

    assert_eq!(count(&events, EVENT_PART_UPDATED), 1);
    let (_, props) = &events[0];
    let part = props.get("part").expect("properties.part missing");
    assert_eq!(part.get("type").and_then(Value::as_str), Some("tool"));
    assert_eq!(part.get("tool").and_then(Value::as_str), Some("bash"));
}

// --------------------------------------------------------------------------------------
// N4 — bandwidth. Protects the shared cgroup running many agents at once.
// --------------------------------------------------------------------------------------

#[test]
fn n4_long_session_emits_bounded_bytes() {
    let mut session = session_with_assistant_parts(vec![text("seed")]);

    // 200 historical messages, each with a part, to simulate a long-running session.
    for i in 0..200 {
        session.messages.push(SessionMessage {
            id: format!("msg_{i}"),
            session_id: session.id.clone(),
            role: MessageRole::User,
            parts: vec![MessagePart {
                id: format!("hist_prt_{i}"),
                part_type: text(&"x".repeat(512)),
                created_at: Utc::now(),
                message_id: Some(format!("msg_{i}")),
            }],
            created_at: Utc::now(),
            metadata: HashMap::new(),
            usage: None,
        });
    }

    let mut index = PartIndex::from_session(&session);
    let mut total_bytes = 0usize;

    // 50 deltas appended to the last part of the live assistant message.
    for i in 0..50 {
        let longer = format!("{}{}", "y".repeat(i), "z");
        session_with_assistant_parts(vec![text("seed")]);
        session.messages.pop();
        session.messages.push(SessionMessage {
            id: "msg_assistant".into(),
            session_id: session.id.clone(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart {
                id: "prt_0".into(),
                part_type: text(&longer),
                created_at: Utc::now(),
                message_id: Some("msg_assistant".into()),
            }],
            created_at: Utc::now(),
            metadata: HashMap::new(),
            usage: None,
        });
        let events = diff_parts(Some(&index), &session, 1_700_000_000_008 + i as i64);
        for (_, props) in &events {
            total_bytes += props.to_string().len();
        }
        index = PartIndex::from_session(&session);
    }

    assert!(
        total_bytes < 32 * 1024,
        "50 deltas against a 200-message session emitted {} bytes; a snapshot echo would \
         exceed 5 MB and starve every other agent on the cgroup",
        total_bytes
    );
}

// --------------------------------------------------------------------------------------
// S3 — ADJACENT: the message envelope must match the reference server's shape.
// --------------------------------------------------------------------------------------

/// The reference server returns `[{ info, parts }]` with camelCase ids. The Rust list
/// endpoint returned a flat `MessageInfo` with snake_case (`session_id`, `created_at`),
/// which is why a client reading `msg["info"]["role"]` always got an empty object.
#[test]
fn s3_message_list_nests_info_and_parts() {
    let mut session = session_with_assistant_parts(vec![text("hello")]);
    session.messages.insert(
        0,
        SessionMessage {
            id: "msg_user".into(),
            session_id: session.id.clone(),
            role: MessageRole::User,
            parts: vec![MessagePart {
                id: "prt_u".into(),
                part_type: text("hi there"),
                created_at: Utc::now(),
                message_id: None,
            }],
            created_at: Utc::now(),
            metadata: HashMap::new(),
            usage: None,
        },
    );

    let listed = messages_to_ts_json(&session);

    assert_eq!(listed.len(), session.messages.len());
    for entry in &listed {
        let obj = entry.as_object().expect("each entry must be an object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["info", "parts"],
            "entry must nest exactly {{info, parts}}, got {:?}",
            keys
        );
        let info = obj.get("info").expect("info missing");
        assert!(info.get("sessionID").is_some(), "info must use camelCase sessionID");
        assert!(info.get("createdAt").is_some(), "info must use camelCase createdAt");
        assert!(
            info.get("session_id").is_none() && info.get("created_at").is_none(),
            "snake_case ids must not leak onto the wire: {:?}",
            info
        );
        assert!(info.get("role").is_some(), "info.role missing");
        assert!(obj.get("parts").expect("parts missing").is_array());
    }
}

/// Three casings existed for the same resource across two endpoints. Every discriminator the
/// reference server uses must be reachable, and none of the Rust-internal camelCase or
/// snake_case forms may appear.
#[test]
fn s3_part_discriminators_match_reference_server() {
    let session = session_with_assistant_parts(vec![
        text("t"),
        reasoning("r"),
        PartType::ToolCall {
            id: "call_1".into(),
            name: "bash".into(),
            input: serde_json::json!({}),
        },
        PartType::ToolResult {
            tool_call_id: "call_1".into(),
            content: "ok".into(),
            is_error: false,
        },
        PartType::StepStart {
            id: "st".into(),
            name: "step".into(),
        },
        PartType::StepFinish {
            id: "sf".into(),
            output: None,
        },
        PartType::File {
            url: "file:///x".into(),
            filename: "x".into(),
            mime: "text/plain".into(),
        },
    ]);

    let listed = messages_to_ts_json(&session);
    let serialized = serde_json::to_string(&listed).expect("serialize");
    let parts = listed[0]["parts"].as_array().expect("parts array").clone();

    let mut seen: Vec<String> = parts
        .iter()
        .map(|p| p["type"].as_str().unwrap_or_default().to_string())
        .collect();
    seen.sort();
    seen.dedup();

    for expected in ["text", "reasoning", "tool", "step-start", "step-finish", "file"] {
        assert!(
            seen.iter().any(|s| s == expected),
            "missing discriminator {:?}; got {:?}",
            expected,
            seen
        );
    }
    for forbidden in [
        "stepStart",
        "stepFinish",
        "toolCall",
        "toolResult",
        "step_start",
        "step_finish",
        "tool_call",
        "tool_result",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "internal discriminator {:?} leaked onto the wire",
            forbidden
        );
    }
}

/// A part must carry the SESSION it belongs to. This regressed once: `sessionID` was being
/// populated from `message_id`, so every part claimed to belong to a session named after a
/// message. Nothing else in the suite asserted this field, so it shipped silently.
#[test]
fn s1_part_session_id_is_the_session_not_the_message() {
    let mut session = session_with_assistant_parts(vec![text("payload")]);
    let session_id = session.id.clone();
    for message in &mut session.messages {
        message.session_id = session_id.clone();
        for part in &mut message.parts {
            part.message_id = Some(message.id.clone());
        }
    }

    let listed = messages_to_ts_json(&session);
    let parts = listed[0]["parts"].as_array().expect("parts array").clone();

    for part in &parts {
        assert_eq!(
            part["sessionID"].as_str(),
            Some(session_id.as_str()),
            "part.sessionID must be the session id, got {:?} (messageID was {:?})",
            part["sessionID"],
            part["messageID"]
        );
        assert_ne!(
            part["sessionID"], part["messageID"],
            "sessionID and messageID must not be the same value"
        );
    }
}

// --------------------------------------------------------------------------------------
// StreamTracker — the clone-free hot path. Target: 100 concurrent sessions in 6 GB.
// --------------------------------------------------------------------------------------

use opencode_server::events::StreamTracker;

/// A tracker fed the same snapshot twice must emit nothing the second time. This is the
/// property that lets the hook run per-tick without accumulating duplicate work.
#[test]
fn tracker_emits_nothing_when_folded_twice() {
    let session = session_with_assistant_parts(vec![text("alpha")]);
    let mut tracker = StreamTracker::new();

    let first = tracker.observe(&session, 1);
    let second = tracker.observe(&session, 2);

    assert_eq!(first.len(), 1, "first fold announces the part");
    assert!(
        second.is_empty(),
        "re-folding an unchanged snapshot emitted {} events: {:?}",
        second.len(),
        second
    );
}

/// Growth must still produce deltas, not whole-part resends.
#[test]
fn tracker_emits_delta_as_text_grows() {
    let mut tracker = StreamTracker::new();
    tracker.observe(&session_with_assistant_parts(vec![text("Hel")]), 1);

    let events = tracker.observe(&session_with_assistant_parts(vec![text("Hello")]), 2);

    assert_eq!(deltas(&events), vec!["lo".to_string()]);
}

/// A new message announces all of its parts, because none of them were seen before.
#[test]
fn tracker_announces_every_part_of_a_new_message() {
    let mut tracker = StreamTracker::new();
    tracker.observe(&session_with_assistant_parts(vec![text("turn one")]), 1);

    let mut next = session_with_assistant_parts(vec![text("turn two"), reasoning("why")]);
    next.messages[0].id = "msg_second".into();
    let events = tracker.observe(&next, 2);

    assert_eq!(count(&events, EVENT_PART_UPDATED), 2, "both parts are new");
}

/// The tracker's own footprint must stay proportional to one message, not the session.
/// A session carrying history must not make each tick more expensive.
#[test]
fn tracker_state_is_bounded_by_the_last_message_not_the_session() {
    let mut session = session_with_assistant_parts(vec![text("live")]);
    for i in 0..500 {
        session.messages.insert(
            0,
            SessionMessage {
                id: format!("old_{i}"),
                session_id: session.id.clone(),
                role: MessageRole::User,
                parts: vec![MessagePart {
                    id: format!("old_prt_{i}"),
                    part_type: text(&"h".repeat(256)),
                    created_at: Utc::now(),
                    message_id: None,
                }],
                created_at: Utc::now(),
                metadata: HashMap::new(),
                usage: None,
            },
        );
    }

    let mut tracker = StreamTracker::new();
    let events = tracker.observe(&session, 1);

    assert_eq!(
        events.len(),
        1,
        "500 historical messages must not produce events; only the live one is new"
    );
}
