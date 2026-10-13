use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use opencode_core::bus::{Bus, BusEventDef};
use opencode_plugin::{HookContext, HookEvent};

use crate::{MessagePart, MessageRole, SessionMessage};

// ============================================================================
// Bus Event Definitions (matches TS Session.Event)
// ============================================================================

pub static SESSION_CREATED_EVENT: BusEventDef = BusEventDef::new("session.created");
pub static SESSION_UPDATED_EVENT: BusEventDef = BusEventDef::new("session.updated");
pub static SESSION_DELETED_EVENT: BusEventDef = BusEventDef::new("session.deleted");
pub static SESSION_DIFF_EVENT: BusEventDef = BusEventDef::new("session.diff");
pub static SESSION_ERROR_EVENT: BusEventDef = BusEventDef::new("session.error");

// Message-level events (matches TS MessageV2.Event)
pub static MESSAGE_UPDATED_EVENT: BusEventDef = BusEventDef::new("message.updated");
pub static MESSAGE_REMOVED_EVENT: BusEventDef = BusEventDef::new("message.removed");
pub static PART_UPDATED_EVENT: BusEventDef = BusEventDef::new("message.part.updated");
pub static PART_REMOVED_EVENT: BusEventDef = BusEventDef::new("message.part.removed");
pub static PART_DELTA_EVENT: BusEventDef = BusEventDef::new("message.part.delta");
pub static COMMAND_EXECUTED_EVENT: BusEventDef = BusEventDef::new("command.executed");

// ============================================================================
// Session Info Schema
// ============================================================================

/// Summary of changes in a session
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSummary {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diffs: Option<Vec<FileDiff>>,
}

/// File diff entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

/// Share information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionShare {
    pub url: String,
}

/// Revert information for undo functionality
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRevert {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

/// Permission ruleset
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionRuleset {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// Time tracking for session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTime {
    pub created: i64,
    pub updated: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compacting: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<i64>,
}

impl Default for SessionTime {
    fn default() -> Self {
        let now = Utc::now().timestamp_millis();
        Self {
            created: now,
            updated: now,
            compacting: None,
            archived: None,
        }
    }
}

/// Usage statistics for a session
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
}

// ============================================================================
// Session Event Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionEvent {
    Created {
        info: Session,
    },
    Updated {
        info: Session,
    },
    Deleted {
        info: Session,
    },
    Diff {
        session_id: String,
        diff: Vec<FileDiff>,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        error: SessionError,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

// ============================================================================
// Session Status
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SessionStatus {
    Active,
    Completed,
    Archived,
    Compacting,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RunStatus {
    Idle,
    Busy,
    Retrying {
        attempt: u32,
        #[serde(default)]
        message: String,
        /// Timestamp (millis) of the next retry attempt.
        #[serde(default)]
        next: i64,
    },
}

impl Default for RunStatus {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionStateEvent {
    StatusChanged {
        session_id: String,
        status: RunStatus,
    },
    Error {
        session_id: String,
        error: String,
    },
}

pub struct SessionStateManager {
    states: HashMap<String, RunStatus>,
}

impl SessionStateManager {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
        }
    }

    pub fn set(&mut self, session_id: &str, status: RunStatus) {
        self.states.insert(session_id.to_string(), status);
    }

    pub fn get(&self, session_id: &str) -> RunStatus {
        self.states.get(session_id).cloned().unwrap_or_default()
    }

    pub fn is_busy(&self, session_id: &str) -> bool {
        matches!(
            self.get(session_id),
            RunStatus::Busy | RunStatus::Retrying { .. }
        )
    }

    pub fn assert_not_busy(&self, session_id: &str) -> Result<(), BusyError> {
        if self.is_busy(session_id) {
            return Err(BusyError {
                session_id: session_id.to_string(),
            });
        }
        Ok(())
    }

    pub fn set_busy(&mut self, session_id: &str) {
        self.set(session_id, RunStatus::Busy);
    }

    pub fn set_retrying(&mut self, session_id: &str, attempt: u32, message: String, next: i64) {
        self.set(
            session_id,
            RunStatus::Retrying {
                attempt,
                message,
                next,
            },
        );
    }

    pub fn set_idle(&mut self, session_id: &str) {
        self.set(session_id, RunStatus::Idle);
    }

    pub fn remove(&mut self, session_id: &str) {
        self.states.remove(session_id);
    }

    pub fn busy_sessions(&self) -> Vec<&str> {
        self.states
            .iter()
            .filter(|(_, s)| matches!(s, RunStatus::Busy | RunStatus::Retrying { .. }))
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// List all session statuses.
    /// Matches TS `SessionStatus.list()` returning all tracked states.
    pub fn list(&self) -> &HashMap<String, RunStatus> {
        &self.states
    }
}

impl Default for SessionStateManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Session
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time: SessionTime,
    pub messages: Vec<SessionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<SessionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share: Option<SessionShare>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revert: Option<SessionRevert>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionRuleset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsage>,
    #[serde(default)]
    pub status: SessionStatus,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing)]
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing)]
    pub updated_at: DateTime<Utc>,
}

impl Session {
    const VERSION: &'static str = "1.0.0";

    /// Create a new session
    pub fn new(project_id: impl Into<String>, directory: impl Into<String>) -> Self {
        let now = Utc::now();
        let slug = Self::generate_slug();

        Self {
            id: crate::identifier::session_id(),
            slug,
            project_id: project_id.into(),
            directory: directory.into(),
            parent_id: None,
            title: format!("New session - {}", now.to_rfc3339()),
            version: Self::VERSION.to_string(),
            time: SessionTime::default(),
            messages: Vec::new(),
            summary: None,
            share: None,
            revert: None,
            permission: None,
            usage: None,
            status: SessionStatus::Active,
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a child session
    pub fn child(parent: &Session) -> Self {
        let now = Utc::now();
        let slug = Self::generate_slug();

        Self {
            id: crate::identifier::session_id(),
            slug,
            project_id: parent.project_id.clone(),
            directory: parent.directory.clone(),
            parent_id: Some(parent.id.clone()),
            title: format!("Child session - {}", now.to_rfc3339()),
            version: Self::VERSION.to_string(),
            time: SessionTime::default(),
            messages: Vec::new(),
            summary: None,
            share: None,
            revert: None,
            permission: parent.permission.clone(),
            usage: None,
            status: SessionStatus::Active,
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn generate_slug() -> String {
        let uuid_part = &Uuid::new_v4().simple().to_string()[..8];
        format!("session-{}", uuid_part)
    }

    /// Check if title is a default generated title
    pub fn is_default_title(&self) -> bool {
        let prefix = if self.parent_id.is_some() {
            "Child session - "
        } else {
            "New session - "
        };

        if !self.title.starts_with(prefix) {
            return false;
        }

        let timestamp_part = &self.title[prefix.len()..];
        chrono::DateTime::parse_from_rfc3339(timestamp_part).is_ok()
    }

    /// Get a forked title
    pub fn get_forked_title(&self) -> String {
        // Simple implementation without regex dependency
        if self.title.ends_with(")") && self.title.contains(" (fork #") {
            if let Some(pos) = self.title.rfind(" (fork #") {
                let base = &self.title[..pos];
                let num_part = &self.title[pos + 8..self.title.len() - 1];
                if let Ok(num) = num_part.parse::<u32>() {
                    return format!("{} (fork #{})", base, num + 1);
                }
            }
        }
        format!("{} (fork #1)", self.title)
    }

    /// Touch the session (update timestamp)
    pub fn touch(&mut self) {
        let now = Utc::now();
        self.time.updated = now.timestamp_millis();
        self.updated_at = now;
    }

    // ========================================================================
    // Message Operations
    // ========================================================================

    /// Add a user message
    pub fn add_user_message(&mut self, text: impl Into<String>) -> &mut SessionMessage {
        let msg = SessionMessage::user(&self.id, text);
        self.messages.push(msg);
        self.touch();
        self.messages.last_mut().unwrap()
    }

    /// Add an assistant message
    pub fn add_assistant_message(&mut self) -> &mut SessionMessage {
        let msg = SessionMessage::assistant(&self.id);
        self.messages.push(msg);
        self.touch();
        self.messages.last_mut().unwrap()
    }

    /// Get the last user message
    pub fn last_user_message(&self) -> Option<&SessionMessage> {
        self.messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::User))
    }

    /// Get the last assistant message
    pub fn last_assistant_message(&self) -> Option<&SessionMessage> {
        self.messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
    }

    /// Get message count
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Get a message by ID
    pub fn get_message(&self, id: &str) -> Option<&SessionMessage> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// Get a mutable message by ID
    pub fn get_message_mut(&mut self, id: &str) -> Option<&mut SessionMessage> {
        self.messages.iter_mut().find(|m| m.id == id)
    }

    /// Remove a message by ID
    pub fn remove_message(&mut self, id: &str) -> Option<SessionMessage> {
        if let Some(pos) = self.messages.iter().position(|m| m.id == id) {
            let msg = self.messages.remove(pos);
            self.touch();
            Some(msg)
        } else {
            None
        }
    }

    // ========================================================================
    // Part-Level Operations
    // ========================================================================

    /// Update a message by replacing it entirely
    pub fn update_message(&mut self, msg: SessionMessage) -> Option<&SessionMessage> {
        if let Some(pos) = self.messages.iter().position(|m| m.id == msg.id) {
            self.messages[pos] = msg;
            self.touch();
            Some(&self.messages[pos])
        } else {
            // New message - append
            self.messages.push(msg);
            self.touch();
            self.messages.last()
        }
    }

    /// Update a specific part within a message
    pub fn update_part(&mut self, msg_id: &str, part: MessagePart) -> Option<&MessagePart> {
        let part_id = part.id.clone();
        let msg = self.get_message_mut(msg_id)?;
        if let Some(pos) = msg.parts.iter().position(|p| p.id == part_id) {
            msg.parts[pos] = part;
        } else {
            msg.parts.push(part);
        }
        self.touch();
        // Return reference to the part
        let msg = self.get_message(msg_id)?;
        msg.parts.iter().find(|p| p.id == part_id)
    }

    /// Remove a specific part from a message
    pub fn remove_part(&mut self, msg_id: &str, part_id: &str) -> Option<MessagePart> {
        let msg = self.get_message_mut(msg_id)?;
        if let Some(pos) = msg.parts.iter().position(|p| p.id == part_id) {
            let removed = msg.parts.remove(pos);
            self.touch();
            Some(removed)
        } else {
            None
        }
    }

    // ========================================================================
    // Usage Aggregation
    // ========================================================================

    /// Aggregate usage across all assistant messages in the session
    pub fn get_usage(&self) -> SessionUsage {
        let mut usage = SessionUsage::default();
        for msg in &self.messages {
            if matches!(msg.role, MessageRole::Assistant) {
                if let Some(ref msg_usage) = msg.usage {
                    usage.input_tokens += msg_usage.input_tokens;
                    usage.output_tokens += msg_usage.output_tokens;
                    usage.reasoning_tokens += msg_usage.reasoning_tokens;
                    usage.cache_write_tokens += msg_usage.cache_write_tokens;
                    usage.cache_read_tokens += msg_usage.cache_read_tokens;
                    usage.total_cost += msg_usage.total_cost;
                }
            }
        }
        usage
    }

    /// Share the session (set share URL)
    pub fn share_session(&mut self, url: impl Into<String>) {
        self.share = Some(SessionShare { url: url.into() });
        self.touch();
    }

    /// Unshare the session
    pub fn unshare_session(&mut self) {
        self.share = None;
        self.touch();
    }

    /// Compute diff summary from messages
    pub fn diff(&self) -> Vec<FileDiff> {
        self.summary
            .as_ref()
            .and_then(|s| s.diffs.clone())
            .unwrap_or_default()
    }

    // ========================================================================
    // Setters
    // ========================================================================

    /// Set the title
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
        self.touch();
    }

    /// Set the archived status
    pub fn set_archived(&mut self, time: Option<i64>) {
        self.time.archived = time.or_else(|| Some(Utc::now().timestamp_millis()));
        self.status = SessionStatus::Archived;
        self.touch();
    }

    /// Set the permission ruleset
    pub fn set_permission(&mut self, permission: PermissionRuleset) {
        self.permission = Some(permission);
        self.touch();
    }

    /// Set the revert information
    pub fn set_revert(&mut self, revert: SessionRevert) {
        self.revert = Some(revert);
        self.touch();
    }

    /// Clear the revert information
    pub fn clear_revert(&mut self) {
        self.revert = None;
        self.touch();
    }

    /// Set the summary
    pub fn set_summary(&mut self, summary: SessionSummary) {
        self.summary = Some(summary);
        self.touch();
    }

    /// Set the share information
    pub fn set_share(&mut self, share: SessionShare) {
        self.share = Some(share);
        self.touch();
    }

    /// Clear the share information
    pub fn clear_share(&mut self) {
        self.share = None;
        self.touch();
    }

    /// Update usage statistics
    pub fn update_usage(&mut self, usage: SessionUsage) {
        self.usage = Some(usage);
        self.touch();
    }

    /// Start compacting
    pub fn start_compacting(&mut self) {
        self.time.compacting = Some(Utc::now().timestamp_millis());
        self.status = SessionStatus::Compacting;
    }

    /// Finish compacting
    pub fn finish_compacting(&mut self) {
        self.time.compacting = None;
        self.status = SessionStatus::Active;
        self.touch();
    }

    /// Mark as completed
    pub fn complete(&mut self) {
        self.status = SessionStatus::Completed;
        self.touch();
    }

    // ========================================================================
    // Serialization Helpers
    // ========================================================================

    /// Convert to a database row representation
    pub fn to_row(&self) -> SessionRow {
        SessionRow {
            id: self.id.clone(),
            slug: self.slug.clone(),
            project_id: self.project_id.clone(),
            directory: self.directory.clone(),
            parent_id: self.parent_id.clone(),
            title: self.title.clone(),
            version: self.version.clone(),
            time_created: self.time.created,
            time_updated: self.time.updated,
            time_compacting: self.time.compacting,
            time_archived: self.time.archived,
            share_url: self.share.as_ref().map(|s| s.url.clone()),
            summary_additions: self.summary.as_ref().map(|s| s.additions),
            summary_deletions: self.summary.as_ref().map(|s| s.deletions),
            summary_files: self.summary.as_ref().map(|s| s.files),
            revert: self.revert.clone(),
            permission: self.permission.clone(),
        }
    }

    /// Create from a database row representation
    pub fn from_row(row: SessionRow) -> Self {
        let summary = if row.summary_additions.is_some()
            || row.summary_deletions.is_some()
            || row.summary_files.is_some()
        {
            Some(SessionSummary {
                additions: row.summary_additions.unwrap_or(0),
                deletions: row.summary_deletions.unwrap_or(0),
                files: row.summary_files.unwrap_or(0),
                diffs: None,
            })
        } else {
            None
        };

        let share = row.share_url.map(|url| SessionShare { url });

        let status = if row.time_archived.is_some() {
            SessionStatus::Archived
        } else if row.time_compacting.is_some() {
            SessionStatus::Compacting
        } else {
            SessionStatus::Active
        };

        let created_at = DateTime::from_timestamp_millis(row.time_created).unwrap_or_else(Utc::now);
        let updated_at = DateTime::from_timestamp_millis(row.time_updated).unwrap_or_else(Utc::now);

        Self {
            id: row.id,
            slug: row.slug,
            project_id: row.project_id,
            directory: row.directory,
            parent_id: row.parent_id,
            title: row.title,
            version: row.version,
            time: SessionTime {
                created: row.time_created,
                updated: row.time_updated,
                compacting: row.time_compacting,
                archived: row.time_archived,
            },
            messages: Vec::new(),
            summary,
            share,
            revert: row.revert,
            permission: row.permission,
            usage: None,
            status,
            metadata: HashMap::new(),
            created_at,
            updated_at,
        }
    }
}

/// Database row representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time_created: i64,
    pub time_updated: i64,
    pub time_compacting: Option<i64>,
    pub time_archived: Option<i64>,
    pub share_url: Option<String>,
    pub summary_additions: Option<u64>,
    pub summary_deletions: Option<u64>,
    pub summary_files: Option<u64>,
    pub revert: Option<SessionRevert>,
    pub permission: Option<PermissionRuleset>,
}

// ============================================================================
// Session Manager
// ============================================================================

/// Upper bound on buffered session events.
///
/// `SessionEvent::Created`/`Updated` each embed a full `Session` clone including every
/// message, so an unbounded buffer grows with total session history rather than a window.
pub const MAX_BUFFERED_EVENTS: usize = 1_024;

pub struct SessionManager {
    sessions: HashMap<String, Session>,
    events: Vec<SessionEvent>,
    bus: Option<Arc<Bus>>,
}

impl SessionManager {
    fn push_event(&mut self, event: SessionEvent) {
        // Drop the oldest half once the cap is hit, so trimming is amortised O(1) instead of
        // shifting the whole buffer on every push.
        if self.events.len() >= MAX_BUFFERED_EVENTS {
            let drop_to = MAX_BUFFERED_EVENTS / 2;
            self.events.drain(..drop_to);
        }
        self.events.push(event);
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            events: Vec::new(),
            bus: None,
        }
    }

    /// Create a new SessionManager with a Bus for event publishing
    pub fn with_bus(bus: Arc<Bus>) -> Self {
        Self {
            sessions: HashMap::new(),
            events: Vec::new(),
            bus: Some(bus),
        }
    }

    /// Publish an event to the Bus (fire-and-forget from sync context)
    fn publish_event(&self, def: &'static BusEventDef, properties: serde_json::Value) {
        if let Some(ref bus) = self.bus {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let bus = bus.clone();
                handle.spawn(async move {
                    bus.publish(def, properties).await;
                });
            }
        }
    }

    /// Publish a session info event (Created/Updated/Deleted)
    fn publish_session_event(&self, def: &'static BusEventDef, session: &Session) {
        if let Ok(json) = serde_json::to_value(session) {
            self.publish_event(def, serde_json::json!({ "info": json }));
        }
    }

    /// Publish a message event
    fn publish_message_event(&self, def: &'static BusEventDef, msg: &SessionMessage) {
        if let Ok(json) = serde_json::to_value(msg) {
            self.publish_event(def, serde_json::json!({ "info": json }));
        }
    }

    /// Publish a part event
    fn publish_part_event(&self, def: &'static BusEventDef, part: &MessagePart) {
        if let Ok(json) = serde_json::to_value(part) {
            self.publish_event(def, serde_json::json!({ "part": json }));
        }
    }

    /// Publish a part delta event (streaming text updates)
    pub fn publish_part_delta(
        &self,
        session_id: &str,
        message_id: &str,
        part_id: &str,
        field: &str,
        delta: &str,
    ) {
        self.publish_event(
            &PART_DELTA_EVENT,
            serde_json::json!({
                "sessionID": session_id,
                "messageID": message_id,
                "partID": part_id,
                "field": field,
                "delta": delta,
            }),
        );
    }

    /// Create a new session
    pub fn create(
        &mut self,
        project_id: impl Into<String>,
        directory: impl Into<String>,
    ) -> Session {
        let session = Session::new(project_id, directory);
        self.sessions.insert(session.id.clone(), session.clone());
        self.push_event(SessionEvent::Created {
            info: session.clone(),
        });

        // Publish to Bus
        self.publish_session_event(&SESSION_CREATED_EVENT, &session);

        // Plugin hook: session.start — notify plugins of new session
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let session_id = session.id.clone();
            handle.spawn(async move {
                opencode_plugin::trigger(
                    HookContext::new(HookEvent::SessionStart).with_session(&session_id),
                )
                .await;
            });
        }

        session
    }

    /// Create a child session
    pub fn create_child(&mut self, parent_id: &str) -> Option<Session> {
        let parent = self.sessions.get(parent_id)?;
        let child = Session::child(parent);
        let child_id = child.id.clone();
        self.sessions.insert(child_id, child.clone());
        self.push_event(SessionEvent::Created {
            info: child.clone(),
        });
        self.publish_session_event(&SESSION_CREATED_EVENT, &child);
        Some(child)
    }

    /// Fork a session at a specific message
    pub fn fork(&mut self, session_id: &str, message_id: Option<&str>) -> Option<Session> {
        let original = self.sessions.get(session_id)?;
        let forked_title = original.get_forked_title();

        let mut forked = Session::child(original);
        forked.parent_id = None;
        forked.title = forked_title;

        if let Some(msg_id) = message_id {
            for msg in &original.messages {
                if msg.id == msg_id {
                    break;
                }
                forked.messages.push(msg.clone());
            }
        } else {
            forked.messages = original.messages.clone();
        }

        let forked_id = forked.id.clone();
        self.sessions.insert(forked_id, forked.clone());
        self.push_event(SessionEvent::Created {
            info: forked.clone(),
        });
        self.publish_session_event(&SESSION_CREATED_EVENT, &forked);
        Some(forked)
    }

    /// Set share info and publish session.updated.
    pub fn share(&mut self, session_id: &str, url: impl Into<String>) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_share(SessionShare { url: url.into() });
            session.clone()
        };
        self.push_event(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Clear share info and publish session.updated.
    pub fn unshare(&mut self, session_id: &str) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.clear_share();
            session.clone()
        };
        self.push_event(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Set archived time and publish session.updated.
    pub fn set_archived(&mut self, session_id: &str, time: Option<i64>) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_archived(time);
            session.clone()
        };
        self.push_event(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Set permission rules and publish session.updated.
    pub fn set_permission(
        &mut self,
        session_id: &str,
        permission: PermissionRuleset,
    ) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_permission(permission);
            session.clone()
        };
        self.push_event(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Set revert info and publish session.updated.
    pub fn set_revert(&mut self, session_id: &str, revert: SessionRevert) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_revert(revert);
            session.clone()
        };
        self.push_event(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Clear revert info and publish session.updated.
    pub fn clear_revert(&mut self, session_id: &str) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.clear_revert();
            session.clone()
        };
        self.push_event(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Set summary and publish session.updated.
    pub fn set_summary(&mut self, session_id: &str, summary: SessionSummary) -> Option<Session> {
        let updated = {
            let session = self.sessions.get_mut(session_id)?;
            session.set_summary(summary);
            session.clone()
        };
        self.push_event(SessionEvent::Updated {
            info: updated.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &updated);
        Some(updated)
    }

    /// Publish command.executed event.
    pub fn publish_command_executed(
        &self,
        command_name: &str,
        session_id: &str,
        arguments: Vec<String>,
        message_id: &str,
    ) {
        self.publish_event(
            &COMMAND_EXECUTED_EVENT,
            serde_json::json!({
                "name": command_name,
                "sessionID": session_id,
                "arguments": arguments,
                "messageID": message_id,
            }),
        );
    }

    /// Get a session by ID
    pub fn get(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id)
    }

    /// Get a mutable session by ID
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    /// List all sessions
    pub fn list(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    /// List sessions with filters
    pub fn list_filtered(&self, filter: SessionFilter) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| {
                if let Some(ref dir) = filter.directory {
                    if s.directory != *dir {
                        return false;
                    }
                }
                if filter.roots && s.parent_id.is_some() {
                    return false;
                }
                if let Some(start) = filter.start {
                    if s.time.updated < start {
                        return false;
                    }
                }
                if let Some(ref search) = filter.search {
                    if !s.title.to_lowercase().contains(&search.to_lowercase()) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    /// Get children of a session
    pub fn children(&self, parent_id: &str) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| s.parent_id.as_deref() == Some(parent_id))
            .collect()
    }

    /// Delete a session
    pub fn delete(&mut self, id: &str) -> Option<Session> {
        let children: Vec<String> = self.children(id).iter().map(|s| s.id.clone()).collect();
        for child_id in children {
            self.delete(&child_id);
        }

        let session = self.sessions.remove(id)?;
        self.push_event(SessionEvent::Deleted {
            info: session.clone(),
        });

        // Publish to Bus
        self.publish_session_event(&SESSION_DELETED_EVENT, &session);

        // Plugin hook: session.end — notify plugins of session deletion
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let session_id = session.id.clone();
            handle.spawn(async move {
                opencode_plugin::trigger(
                    HookContext::new(HookEvent::SessionEnd).with_session(&session_id),
                )
                .await;
            });
        }

        Some(session)
    }

    /// Update a session
    pub fn update(&mut self, session: Session) {
        let id = session.id.clone();
        self.sessions.insert(id, session.clone());
        self.push_event(SessionEvent::Updated {
            info: session.clone(),
        });
        self.publish_session_event(&SESSION_UPDATED_EVENT, &session);
    }

    /// Get events (and clear them)
    pub fn drain_events(&mut self) -> Vec<SessionEvent> {
        self.events.drain(..).collect()
    }

    /// Get session count
    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    // ========================================================================
    // Message/Part Operations with Bus Publishing
    // ========================================================================

    /// Update a message in a session and publish Bus event
    pub fn update_message(&mut self, session_id: &str, msg: SessionMessage) -> Option<()> {
        let session = self.sessions.get_mut(session_id)?;
        session.update_message(msg.clone());
        self.publish_message_event(&MESSAGE_UPDATED_EVENT, &msg);
        Some(())
    }

    /// Remove a message from a session and publish Bus event
    pub fn remove_message(&mut self, session_id: &str, message_id: &str) -> Option<SessionMessage> {
        let session = self.sessions.get_mut(session_id)?;
        let msg = session.remove_message(message_id)?;
        self.publish_event(
            &MESSAGE_REMOVED_EVENT,
            serde_json::json!({
                "sessionID": session_id,
                "messageID": message_id,
            }),
        );
        Some(msg)
    }

    /// Update a part in a message and publish Bus event
    pub fn update_part(
        &mut self,
        session_id: &str,
        message_id: &str,
        part: MessagePart,
    ) -> Option<()> {
        let session = self.sessions.get_mut(session_id)?;
        session.update_part(message_id, part.clone());
        self.publish_part_event(&PART_UPDATED_EVENT, &part);
        Some(())
    }

    /// Remove a part from a message and publish Bus event
    pub fn remove_part(
        &mut self,
        session_id: &str,
        message_id: &str,
        part_id: &str,
    ) -> Option<MessagePart> {
        let session = self.sessions.get_mut(session_id)?;
        let part = session.remove_part(message_id, part_id)?;
        self.publish_event(
            &PART_REMOVED_EVENT,
            serde_json::json!({
                "sessionID": session_id,
                "messageID": message_id,
                "partID": part_id,
            }),
        );
        Some(part)
    }

    /// Publish a session error event
    pub fn publish_error(&self, session_id: Option<&str>, error: serde_json::Value) {
        let mut props = serde_json::json!({ "error": error });
        if let Some(sid) = session_id {
            props["sessionID"] = serde_json::Value::String(sid.to_string());
        }
        self.publish_event(&SESSION_ERROR_EVENT, props);
    }

    /// Publish a session diff event
    pub fn publish_diff(&self, session_id: &str, diffs: &[FileDiff]) {
        if let Ok(diff_json) = serde_json::to_value(diffs) {
            self.publish_event(
                &SESSION_DIFF_EVENT,
                serde_json::json!({
                    "sessionID": session_id,
                    "diff": diff_json,
                }),
            );
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    /// Set the Bus for event publishing (can be called after construction)
    pub fn set_bus(&mut self, bus: Arc<Bus>) {
        self.bus = Some(bus);
    }
}

/// Filter options for listing sessions
#[derive(Debug, Clone, Default)]
pub struct SessionFilter {
    pub directory: Option<String>,
    pub roots: bool,
    pub start: Option<i64>,
    pub search: Option<String>,
    pub limit: Option<usize>,
}

// ============================================================================
// Busy Error
// ============================================================================

#[derive(Debug, Clone)]
pub struct BusyError {
    pub session_id: String,
}

impl std::fmt::Display for BusyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Session {} is busy", self.session_id)
    }
}

impl std::error::Error for BusyError {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MessageUsage;
    use std::sync::Arc;
    use tokio::time::{timeout, Duration};

    #[test]
    fn test_session_creation() {
        let session = Session::new("project-1", "/path/to/project");
        assert!(session.id.starts_with("ses_"));
        assert!(session.title.starts_with("New session"));
        assert!(session.parent_id.is_none());
        assert_eq!(session.status, SessionStatus::Active);
    }

    #[test]
    fn test_child_session() {
        let parent = Session::new("project-1", "/path/to/project");
        let child = Session::child(&parent);

        assert!(child.parent_id.is_some());
        assert_eq!(child.parent_id.unwrap(), parent.id);
        assert!(child.title.starts_with("Child session"));
    }

    #[test]
    fn test_add_messages() {
        let mut session = Session::new("project-1", "/path/to/project");

        session.add_user_message("Hello");
        assert_eq!(session.message_count(), 1);

        session.add_assistant_message();
        assert_eq!(session.message_count(), 2);
    }

    #[test]
    fn test_session_manager() {
        let mut manager = SessionManager::new();

        let session = manager.create("project-1", "/path/to/project");
        assert!(manager.get(&session.id).is_some());
        assert_eq!(manager.count(), 1);

        let child = manager.create_child(&session.id).unwrap();
        assert!(child.parent_id.is_some());

        manager.delete(&session.id);
        assert_eq!(manager.count(), 0);
    }

    #[test]
    fn test_fork_title() {
        let session = Session::new("project-1", "/path/to/project");
        let title1 = session.get_forked_title();
        assert!(title1.ends_with("(fork #1)"));

        let temp = Session {
            title: title1,
            ..session.clone()
        };
        let title2 = temp.get_forked_title();
        assert!(title2.ends_with("(fork #2)"));
    }

    #[test]
    fn test_update_message() {
        let mut session = Session::new("project-1", "/path");
        let msg = session.add_user_message("Hello");
        let msg_id = msg.id.clone();

        let updated = session.get_message(&msg_id).unwrap().clone();
        session.update_message(updated);
        assert!(session.get_message(&msg_id).is_some());
    }

    #[test]
    fn test_update_message_new() {
        let mut session = Session::new("project-1", "/path");
        let new_msg = SessionMessage::user(&session.id, "Brand new");
        let new_id = new_msg.id.clone();
        session.update_message(new_msg);
        assert!(session.get_message(&new_id).is_some());
        assert_eq!(session.message_count(), 1);
    }

    #[test]
    fn test_update_part() {
        let mut session = Session::new("project-1", "/path");
        let msg = session.add_user_message("Hello");
        let msg_id = msg.id.clone();
        let part_id = msg.parts[0].id.clone();

        // Update existing part
        let replacement = MessagePart {
            id: part_id.clone(),
            part_type: crate::PartType::Text {
                text: "Updated".into(),
                synthetic: None,
                ignored: None,
            },
            created_at: Utc::now(),
            message_id: None,
        };
        let result = session.update_part(&msg_id, replacement);
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, part_id);
    }

    #[test]
    fn test_remove_part() {
        let mut session = Session::new("project-1", "/path");
        let msg = session.add_user_message("Hello");
        let msg_id = msg.id.clone();
        let part_id = msg.parts[0].id.clone();

        let removed = session.remove_part(&msg_id, &part_id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, part_id);
        assert_eq!(session.get_message(&msg_id).unwrap().parts.len(), 0);
    }

    #[test]
    fn test_remove_part_not_found() {
        let mut session = Session::new("project-1", "/path");
        let msg = session.add_user_message("Hello");
        let msg_id = msg.id.clone();

        let removed = session.remove_part(&msg_id, "nonexistent");
        assert!(removed.is_none());
    }

    #[test]
    fn test_share_unshare() {
        let mut session = Session::new("project-1", "/path");

        session.share_session("https://example.com/share/123");
        assert!(session.share.is_some());
        assert_eq!(
            session.share.as_ref().unwrap().url,
            "https://example.com/share/123"
        );

        session.unshare_session();
        assert!(session.share.is_none());
    }

    #[test]
    fn test_get_usage_empty() {
        let session = Session::new("project-1", "/path");
        let usage = session.get_usage();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.total_cost, 0.0);
    }

    #[test]
    fn test_get_usage_aggregation() {
        let mut session = Session::new("project-1", "/path");

        // Add an assistant message with usage
        let msg = session.add_assistant_message();
        msg.usage = Some(MessageUsage {
            input_tokens: 100,
            output_tokens: 50,
            reasoning_tokens: 10,
            cache_write_tokens: 20,
            cache_read_tokens: 30,
            total_cost: 0.005,
        });

        // Add another assistant message with usage
        let msg2 = session.add_assistant_message();
        msg2.usage = Some(MessageUsage {
            input_tokens: 200,
            output_tokens: 100,
            reasoning_tokens: 20,
            cache_write_tokens: 40,
            cache_read_tokens: 60,
            total_cost: 0.010,
        });

        // Add a user message (should not be counted)
        let user_msg = session.add_user_message("test");
        user_msg.usage = Some(MessageUsage {
            input_tokens: 999,
            output_tokens: 999,
            reasoning_tokens: 999,
            cache_write_tokens: 999,
            cache_read_tokens: 999,
            total_cost: 999.0,
        });

        let usage = session.get_usage();
        assert_eq!(usage.input_tokens, 300);
        assert_eq!(usage.output_tokens, 150);
        assert_eq!(usage.reasoning_tokens, 30);
        assert_eq!(usage.cache_write_tokens, 60);
        assert_eq!(usage.cache_read_tokens, 90);
        assert!((usage.total_cost - 0.015).abs() < f64::EPSILON);
    }

    #[test]
    fn test_diff_empty() {
        let session = Session::new("project-1", "/path");
        let diffs = session.diff();
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_diff_with_summary() {
        let mut session = Session::new("project-1", "/path");
        session.set_summary(SessionSummary {
            additions: 10,
            deletions: 5,
            files: 2,
            diffs: Some(vec![
                FileDiff {
                    path: "src/main.rs".into(),
                    additions: 7,
                    deletions: 3,
                },
                FileDiff {
                    path: "src/lib.rs".into(),
                    additions: 3,
                    deletions: 2,
                },
            ]),
        });

        let diffs = session.diff();
        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].path, "src/main.rs");
    }

    #[tokio::test]
    async fn session_share_publishes_updated_event() {
        let bus = Arc::new(Bus::new());
        let mut manager = SessionManager::with_bus(bus.clone());
        let session = manager.create("project-1", "/path");
        let mut rx = bus.subscribe_channel();

        let updated = manager
            .share(&session.id, "https://share.opencode.ai/test")
            .expect("session should exist");
        assert_eq!(
            updated.share.as_ref().map(|share| share.url.as_str()),
            Some("https://share.opencode.ai/test")
        );

        let event = timeout(Duration::from_secs(1), async {
            loop {
                let event = rx.recv().await.expect("event channel closed");
                if event.event_type == SESSION_UPDATED_EVENT.event_type {
                    break event;
                }
            }
        })
        .await
        .expect("event timeout");
        assert_eq!(event.event_type, SESSION_UPDATED_EVENT.event_type);
        assert_eq!(event.properties["info"]["id"], session.id);
        assert_eq!(
            event.properties["info"]["share"]["url"],
            "https://share.opencode.ai/test"
        );
    }

    #[tokio::test]
    async fn command_executed_event_is_published() {
        let bus = Arc::new(Bus::new());
        let manager = SessionManager::with_bus(bus.clone());
        let mut rx = bus.subscribe_channel();

        manager.publish_command_executed(
            "review",
            "session-1",
            vec!["--fast".to_string()],
            "message-1",
        );

        let event = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("event timeout")
            .expect("event channel closed");
        assert_eq!(event.event_type, COMMAND_EXECUTED_EVENT.event_type);
        assert_eq!(event.properties["name"], "review");
        assert_eq!(event.properties["sessionID"], "session-1");
        assert_eq!(event.properties["arguments"][0], "--fast");
        assert_eq!(event.properties["messageID"], "message-1");
    }
}

#[cfg(test)]
mod event_buffer_tests {
    use super::*;

    /// `SessionEvent::Created`/`Updated` each hold a full `Session` clone, including every
    /// message. `drain_events` had zero callers, so the buffer grew without bound and scaled
    /// with total session history rather than with a bounded window.
    #[test]
    fn event_buffer_is_bounded() {
        let mut manager = SessionManager::new();
        let dir = std::env::temp_dir();

        for _ in 0..5_000 {
            let session = manager.create("default", dir.to_string_lossy().to_string());
            manager.update(session);
        }

        let buffered = manager.events.len();
        assert!(
            buffered <= 1_024,
            "event buffer grew to {} entries after 10k operations; it must stay bounded",
            buffered
        );
    }
}
