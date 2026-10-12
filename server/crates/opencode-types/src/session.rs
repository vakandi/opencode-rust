use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSummary {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diffs: Option<Vec<FileDiff>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionShare {
    pub url: String,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionRuleset {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

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
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            created: now,
            updated: now,
            compacting: None,
            archived: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
}

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
    Retrying { attempt: u32 },
}

use crate::message::SessionMessage;

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
    pub fn touch(&mut self) {
        let now = Utc::now();
        self.time.updated = now.timestamp_millis();
        self.updated_at = now;
    }
}
