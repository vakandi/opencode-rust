use serde::{Deserialize, Serialize};

use opencode_core::bus::Bus;
use opencode_provider::{ChatRequest, Content, Message, Provider, Role};

use crate::message_v2::{MessageInfo, MessageWithParts, Part, StepFinishPart, StepStartPart};
use crate::session::{FileDiff as SessionFileDiff, Session, SessionSummary as SessionSummaryInfo};
use crate::snapshot::Snapshot;
use crate::{MessageRole, PartType};

// ============================================================================
// Data types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSummaryData {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    pub diffs: Vec<SummaryFileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryFileDiff {
    pub file: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleGenerationRequest {
    pub session_id: String,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleGenerationResponse {
    pub title: String,
}

const SESSION_DIFF_STORAGE_KEY_PREFIX: &str = "session_diff:";
const MESSAGE_SUMMARY_TITLE_KEY: &str = "summary_title";
const MESSAGE_SUMMARY_DIFFS_KEY: &str = "summary_diffs";

// ============================================================================
// Git path unquoting (matches TS SessionSummary.unquoteGitPath)
// ============================================================================

/// Unquote a git path that may contain escape sequences.
///
/// Git quotes paths containing non-ASCII or special characters by wrapping
/// them in double quotes and using octal or C-style escape sequences.
/// This function reverses that encoding.
pub fn unquote_git_path(input: &str) -> String {
    // PLACEHOLDER_CONTINUE
    if !input.starts_with('"') || !input.ends_with('"') {
        return input.to_string();
    }

    let body = &input[1..input.len() - 1];
    let mut bytes: Vec<u8> = Vec::with_capacity(body.len());
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch != '\\' {
            let mut buf = [0u8; 4];
            let encoded = ch.encode_utf8(&mut buf);
            bytes.extend_from_slice(encoded.as_bytes());
            i += 1;
            continue;
        }

        // Backslash escape
        i += 1;
        if i >= chars.len() {
            bytes.push(b'\\');
            continue;
        }

        let next = chars[i];

        // Octal escape: \NNN where N is 0-7
        if next >= '0' && next <= '7' {
            let start = i;
            let mut end = i;
            while end < chars.len() && end < start + 3 && chars[end] >= '0' && chars[end] <= '7' {
                end += 1;
            }
            let octal_str: String = chars[start..end].iter().collect();
            if let Ok(val) = u8::from_str_radix(&octal_str, 8) {
                bytes.push(val);
            } else {
                bytes.push(next as u8);
                end = start + 1;
            }
            i = end;
            continue;
        }

        // Named escapes
        let escaped: Option<u8> = match next {
            'n' => Some(b'\n'),
            'r' => Some(b'\r'),
            't' => Some(b'\t'),
            'b' => Some(0x08), // backspace
            'f' => Some(0x0C), // form feed
            'v' => Some(0x0B), // vertical tab
            '\\' => Some(b'\\'),
            '"' => Some(b'"'),
            _ => None,
        };

        match escaped {
            Some(b) => bytes.push(b),
            None => {
                let mut buf = [0u8; 4];
                let encoded = next.encode_utf8(&mut buf);
                bytes.extend_from_slice(encoded.as_bytes());
            }
        }
        i += 1;
    }

    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).to_string())
}

// ============================================================================
// Diff computation (matches TS SessionSummary.computeDiff)
// ============================================================================

/// Compute diffs from message step snapshots.
///
/// Scans messages for step-start and step-finish parts to find the earliest
/// "from" snapshot and the latest "to" snapshot, then computes a full git diff.
pub fn compute_diff(
    messages: &[MessageWithParts],
    worktree: &std::path::Path,
) -> Vec<SummaryFileDiff> {
    let mut from: Option<String> = None;
    let mut to: Option<String> = None;

    for msg in messages {
        for part in &msg.parts {
            match part {
                Part::StepStart(StepStartPart { snapshot, .. }) => {
                    if from.is_none() {
                        if let Some(ref s) = snapshot {
                            if !s.is_empty() {
                                from = Some(s.clone());
                            }
                        }
                    }
                }
                Part::StepFinish(StepFinishPart { snapshot, .. }) => {
                    if let Some(ref s) = snapshot {
                        if !s.is_empty() {
                            to = Some(s.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if let (Some(ref from_ref), Some(ref to_ref)) = (&from, &to) {
        match Snapshot::diff_full(worktree, from_ref, to_ref) {
            Ok(diffs) => {
                return diffs
                    .into_iter()
                    .map(|d| SummaryFileDiff {
                        file: d.path,
                        additions: d.additions,
                        deletions: d.deletions,
                    })
                    .collect();
            }
            Err(e) => {
                tracing::warn!("Failed to compute snapshot diff: {}", e);
            }
        }
    }

    Vec::new()
}

// ============================================================================
// Summarize (matches TS SessionSummary.summarize)
// ============================================================================

/// Input for the summarize operation.
pub struct SummarizeInput {
    pub session_id: String,
    pub message_id: String,
}

/// Summarize a session: compute diffs and update session summary.
///
/// This mirrors the TS `SessionSummary.summarize` which runs both
/// `summarizeSession` and `summarizeMessage` in parallel.
pub async fn summarize(
    input: &SummarizeInput,
    messages: &[MessageWithParts],
    worktree: &std::path::Path,
    bus: Option<&Bus>,
) -> SessionSummaryData {
    let diffs = clean_diffs(compute_diff(messages, worktree));

    let summary = SessionSummaryData {
        additions: diffs.iter().map(|d| d.additions).sum(),
        deletions: diffs.iter().map(|d| d.deletions).sum(),
        files: diffs.len() as u64,
        diffs: diffs.clone(),
    };

    // Publish diff event if bus is available
    if let Some(bus) = bus {
        let diff_event = opencode_core::bus::define_event("session.diff");
        let diff_data = serde_json::json!({
            "sessionID": input.session_id,
            "diff": diffs,
        });
        bus.publish(&diff_event, diff_data).await;
    }

    summary
}

fn session_diff_storage_key(session_id: &str) -> String {
    format!("{}{}", SESSION_DIFF_STORAGE_KEY_PREFIX, session_id)
}

/// Persist session-level diff results in session metadata.
/// This is the Rust rewrite equivalent of TS `Storage.write(["session_diff", sessionID], diffs)`.
pub fn persist_session_diffs(
    session: &mut Session,
    session_id: &str,
    diffs: &[SummaryFileDiff],
) -> anyhow::Result<()> {
    let key = session_diff_storage_key(session_id);
    let value = serde_json::to_value(diffs)?;
    session.metadata.insert(key, value);
    Ok(())
}

/// Load persisted session-level diff results from session metadata.
pub fn load_session_diffs(session: &Session, session_id: &str) -> Vec<SummaryFileDiff> {
    let key = session_diff_storage_key(session_id);
    session
        .metadata
        .get(&key)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn to_session_file_diffs(diffs: &[SummaryFileDiff]) -> Vec<SessionFileDiff> {
    diffs
        .iter()
        .map(|d| SessionFileDiff {
            path: d.file.clone(),
            additions: d.additions,
            deletions: d.deletions,
        })
        .collect()
}

/// Apply summary stats to the session record using `Session.set_summary()`.
pub fn set_session_summary(session: &mut Session, summary: &SessionSummaryData) {
    let diffs = if summary.diffs.is_empty() {
        None
    } else {
        Some(to_session_file_diffs(&summary.diffs))
    };

    session.set_summary(SessionSummaryInfo {
        additions: summary.additions,
        deletions: summary.deletions,
        files: summary.files,
        diffs,
    });
}

fn first_user_text_for_message(session: &Session, message_id: &str) -> Option<String> {
    let message = session.get_message(message_id)?;
    if !matches!(message.role, MessageRole::User) {
        return None;
    }

    message.parts.iter().find_map(|part| match &part.part_type {
        PartType::Text { text, .. } if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    })
}

fn fallback_message_title(text: &str) -> String {
    generate_title_from_messages(&[text.to_string()])
}

async fn generate_message_title_llm(
    text: &str,
    provider: &dyn Provider,
    model_id: &str,
) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }

    let fallback = fallback_message_title(text);
    let request = ChatRequest {
        model: model_id.to_string(),
        messages: vec![Message {
            role: Role::User,
            content: Content::Text(format!(
                "Generate a concise title (under 80 chars) for this user request.\n\n{}",
                text
            )),
            cache_control: None,
            provider_options: None,
        }],
        max_tokens: Some(64),
        temperature: Some(0.0),
        top_p: None,
        system: Some(
            "You generate short request titles. Reply with only the title text.".to_string(),
        ),
        tools: None,
        stream: Some(false),
        session_id: None,
        provider_options: None,
        variant: None,
    };

    let response = provider.chat(request).await.ok()?;
    let raw = response
        .choices
        .first()
        .map(|choice| match &choice.message.content {
            Content::Text(text) => text.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|part| part.text.clone())
                .collect::<Vec<_>>()
                .join(" "),
        })?;

    let cleaned = raw
        .replace(|c: char| c == '"' || c == '\'', "")
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("<think>"))
        .unwrap_or("")
        .to_string();

    if cleaned.is_empty() {
        Some(fallback)
    } else if cleaned.len() > 100 {
        Some(format!("{}...", &cleaned[..97]))
    } else {
        Some(cleaned)
    }
}

/// Summarize a specific user message and persist per-message summary metadata.
pub async fn summarize_message_for_session(
    session: &mut Session,
    message_id: &str,
    messages: &[MessageWithParts],
    worktree: &std::path::Path,
    provider: Option<&dyn Provider>,
    model_id: Option<&str>,
) -> anyhow::Result<()> {
    if message_id.is_empty() {
        return Ok(());
    }

    let diffs = clean_diffs(summarize_message(message_id, messages, worktree));
    let mut changed = false;

    if let Some(mut message) = session.get_message(message_id).cloned() {
        if matches!(message.role, MessageRole::User) {
            message.metadata.insert(
                MESSAGE_SUMMARY_DIFFS_KEY.to_string(),
                serde_json::to_value(&diffs)?,
            );
            changed = true;

            let has_title = message
                .metadata
                .get(MESSAGE_SUMMARY_TITLE_KEY)
                .and_then(|value| value.as_str())
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);

            if !has_title {
                if let Some(text) = first_user_text_for_message(session, message_id) {
                    let generated = if let (Some(provider), Some(model_id)) = (provider, model_id) {
                        generate_message_title_llm(&text, provider, model_id)
                            .await
                            .or_else(|| Some(fallback_message_title(&text)))
                    } else {
                        Some(fallback_message_title(&text))
                    };

                    if let Some(title) = generated.filter(|value| !value.trim().is_empty()) {
                        message.metadata.insert(
                            MESSAGE_SUMMARY_TITLE_KEY.to_string(),
                            serde_json::json!(title),
                        );
                    }
                }
            }
        }

        if changed {
            let _ = session.update_message(message);
        }
    }

    Ok(())
}

/// Run full session + message summarization and persist all outputs.
pub async fn summarize_into_session(
    input: &SummarizeInput,
    session: &mut Session,
    messages: &[MessageWithParts],
    worktree: &std::path::Path,
    provider: Option<&dyn Provider>,
    model_id: Option<&str>,
    bus: Option<&Bus>,
) -> anyhow::Result<SessionSummaryData> {
    let summary = summarize(input, messages, worktree, bus).await;
    set_session_summary(session, &summary);
    persist_session_diffs(session, &input.session_id, &summary.diffs)?;
    summarize_message_for_session(
        session,
        &input.message_id,
        messages,
        worktree,
        provider,
        model_id,
    )
    .await?;
    Ok(summary)
}

/// Summarize a specific message's diffs.
///
/// Filters messages to only those related to the given message_id
/// (the user message and its assistant responses), then computes diffs.
pub fn summarize_message(
    message_id: &str,
    messages: &[MessageWithParts],
    worktree: &std::path::Path,
) -> Vec<SummaryFileDiff> {
    let filtered: Vec<&MessageWithParts> = messages
        .iter()
        .filter(|m| {
            let id = match &m.info {
                MessageInfo::User { id, .. } => id,
                MessageInfo::Assistant { id, .. } => id,
            };
            let parent_id = match &m.info {
                MessageInfo::Assistant { parent_id, .. } => Some(parent_id.as_str()),
                _ => None,
            };
            id == message_id || parent_id == Some(message_id)
        })
        .collect();

    // Convert to owned for compute_diff
    let owned: Vec<MessageWithParts> = filtered.into_iter().cloned().collect();
    compute_diff(&owned, worktree)
}

/// Unquote git paths in diff results and return cleaned diffs.
///
/// Matches TS `SessionSummary.diff` which unquotes git paths.
pub fn clean_diffs(diffs: Vec<SummaryFileDiff>) -> Vec<SummaryFileDiff> {
    diffs
        .into_iter()
        .map(|d| {
            let file = unquote_git_path(&d.file);
            SummaryFileDiff {
                file,
                additions: d.additions,
                deletions: d.deletions,
            }
        })
        .collect()
}

// ============================================================================
// Title generation (simple fallback, LLM-based is in prompt.rs)
// ============================================================================

pub fn generate_title_from_messages(messages: &[String]) -> String {
    if messages.is_empty() {
        return "New Session".to_string();
    }

    let first_message = &messages[0];

    let words: Vec<&str> = first_message.split_whitespace().take(10).collect();
    if words.is_empty() {
        return "New Session".to_string();
    }

    let title = words.join(" ");
    if title.len() > 100 {
        format!("{}...", &title[..97])
    } else {
        title
    }
}

// ============================================================================
// Legacy compatibility
// ============================================================================

pub struct SessionSummary;

impl SessionSummary {
    pub fn new() -> SessionSummaryData {
        SessionSummaryData::default()
    }

    pub fn from_diffs(diffs: Vec<SummaryFileDiff>) -> SessionSummaryData {
        let additions = diffs.iter().map(|d| d.additions).sum();
        let deletions = diffs.iter().map(|d| d.deletions).sum();
        let files = diffs.len() as u64;

        SessionSummaryData {
            additions,
            deletions,
            files,
            diffs,
        }
    }

    pub fn merge(a: &SessionSummaryData, b: &SessionSummaryData) -> SessionSummaryData {
        let mut diffs = a.diffs.clone();
        diffs.extend(b.diffs.clone());

        SessionSummaryData {
            additions: a.additions + b.additions,
            deletions: a.deletions + b.deletions,
            files: a.files + b.files,
            diffs,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream;
    use opencode_provider::{ChatResponse, Choice, ModelInfo, ProviderError, StreamResult, Usage};

    struct MockProvider {
        model: ModelInfo,
        title: String,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn id(&self) -> &str {
            "mock"
        }

        fn name(&self) -> &str {
            "Mock"
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![self.model.clone()]
        }

        fn get_model(&self, id: &str) -> Option<&ModelInfo> {
            if id == self.model.id {
                Some(&self.model)
            } else {
                None
            }
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse {
                id: "chat_mock".to_string(),
                model: self.model.id.clone(),
                choices: vec![Choice {
                    index: 0,
                    message: Message::assistant(self.title.clone()),
                    finish_reason: Some("stop".to_string()),
                }],
                usage: Some(Usage {
                    prompt_tokens: 10,
                    completion_tokens: 3,
                    total_tokens: 13,
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                }),
            })
        }

        async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    #[test]
    fn test_unquote_git_path_plain() {
        assert_eq!(unquote_git_path("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn test_unquote_git_path_quoted_simple() {
        assert_eq!(unquote_git_path("\"src/main.rs\""), "src/main.rs");
    }

    #[test]
    fn test_unquote_git_path_octal_escape() {
        // \303\251 is UTF-8 for 'e' with acute accent (U+00E9)
        let input = "\"src/caf\\303\\251.rs\"";
        let result = unquote_git_path(input);
        assert_eq!(result, "src/caf\u{00e9}.rs");
    }

    #[test]
    fn test_unquote_git_path_named_escapes() {
        assert_eq!(unquote_git_path("\"hello\\nworld\""), "hello\nworld");
        assert_eq!(unquote_git_path("\"tab\\there\""), "tab\there");
        assert_eq!(unquote_git_path("\"back\\\\slash\""), "back\\slash");
        assert_eq!(unquote_git_path("\"quote\\\"inside\""), "quote\"inside");
    }

    #[test]
    fn test_unquote_git_path_not_quoted() {
        // Missing end quote
        assert_eq!(unquote_git_path("\"no end"), "\"no end");
        // Missing start quote
        assert_eq!(unquote_git_path("no start\""), "no start\"");
    }

    #[test]
    fn test_generate_title_empty() {
        assert_eq!(generate_title_from_messages(&[]), "New Session");
    }

    #[test]
    fn test_generate_title_short() {
        let msgs = vec!["Fix the login bug".to_string()];
        assert_eq!(generate_title_from_messages(&msgs), "Fix the login bug");
    }

    #[test]
    fn test_generate_title_long() {
        let long_msg = "a ".repeat(200);
        let msgs = vec![long_msg];
        let title = generate_title_from_messages(&msgs);
        assert!(title.len() <= 100);
    }

    #[test]
    fn test_summary_from_diffs() {
        let diffs = vec![
            SummaryFileDiff {
                file: "src/main.rs".into(),
                additions: 10,
                deletions: 5,
            },
            SummaryFileDiff {
                file: "src/lib.rs".into(),
                additions: 3,
                deletions: 2,
            },
        ];
        let summary = SessionSummary::from_diffs(diffs);
        assert_eq!(summary.additions, 13);
        assert_eq!(summary.deletions, 7);
        assert_eq!(summary.files, 2);
    }

    #[test]
    fn test_summary_merge() {
        let a = SessionSummaryData {
            additions: 10,
            deletions: 5,
            files: 2,
            diffs: vec![SummaryFileDiff {
                file: "a.rs".into(),
                additions: 10,
                deletions: 5,
            }],
        };
        let b = SessionSummaryData {
            additions: 3,
            deletions: 1,
            files: 1,
            diffs: vec![SummaryFileDiff {
                file: "b.rs".into(),
                additions: 3,
                deletions: 1,
            }],
        };
        let merged = SessionSummary::merge(&a, &b);
        assert_eq!(merged.additions, 13);
        assert_eq!(merged.deletions, 6);
        assert_eq!(merged.files, 3);
        assert_eq!(merged.diffs.len(), 2);
    }

    #[test]
    fn test_clean_diffs_unquotes() {
        let diffs = vec![SummaryFileDiff {
            file: "\"src/caf\\303\\251.rs\"".into(),
            additions: 1,
            deletions: 0,
        }];
        let cleaned = clean_diffs(diffs);
        assert_eq!(cleaned[0].file, "src/caf\u{00e9}.rs");
    }

    #[test]
    fn test_persist_and_load_session_diffs() {
        let mut session = Session::new("proj", ".");
        let session_id = session.id.clone();
        let diffs = vec![SummaryFileDiff {
            file: "src/main.rs".to_string(),
            additions: 5,
            deletions: 1,
        }];

        persist_session_diffs(&mut session, &session_id, &diffs).expect("persist should work");
        let loaded = load_session_diffs(&session, &session_id);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].file, "src/main.rs");
    }

    #[test]
    fn test_set_session_summary_updates_session_record() {
        let mut session = Session::new("proj", ".");
        let summary = SessionSummaryData {
            additions: 7,
            deletions: 2,
            files: 1,
            diffs: vec![SummaryFileDiff {
                file: "src/lib.rs".to_string(),
                additions: 7,
                deletions: 2,
            }],
        };

        set_session_summary(&mut session, &summary);

        let stored = session.summary.expect("summary should be set");
        assert_eq!(stored.additions, 7);
        assert_eq!(stored.deletions, 2);
        assert_eq!(stored.files, 1);
        assert_eq!(stored.diffs.expect("diffs should be present").len(), 1);
    }

    #[tokio::test]
    async fn test_summarize_message_for_session_generates_title() {
        let mut session = Session::new("proj", ".");
        let message = session.add_user_message("Implement summary pipeline for session diffs");
        let message_id = message.id.clone();
        let provider = MockProvider {
            model: ModelInfo {
                id: "mock-model".to_string(),
                name: "Mock".to_string(),
                provider: "mock".to_string(),
                context_window: 8192,
                max_output_tokens: 1024,
                supports_vision: false,
                supports_tools: false,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
            },
            title: "Summary Pipeline".to_string(),
        };

        summarize_message_for_session(
            &mut session,
            &message_id,
            &[],
            std::path::Path::new("."),
            Some(&provider),
            Some("mock-model"),
        )
        .await
        .expect("summarize_message_for_session should work");

        let updated = session
            .get_message(&message_id)
            .expect("message should still exist");
        assert_eq!(
            updated
                .metadata
                .get("summary_title")
                .and_then(|value| value.as_str()),
            Some("Summary Pipeline")
        );
        assert!(updated.metadata.contains_key("summary_diffs"));
    }
}
