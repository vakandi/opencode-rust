use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::session::{FileDiff as SessionFileDiff, Session, SessionManager};
use crate::snapshot::Snapshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertInput {
    pub session_id: String,
    pub message_id: String,
    pub part_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertInfo {
    pub message_id: String,
    pub part_id: Option<String>,
    pub snapshot: Option<String>,
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertSummary {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

impl From<SessionFileDiff> for FileDiff {
    fn from(d: SessionFileDiff) -> Self {
        Self {
            path: d.path,
            additions: d.additions,
            deletions: d.deletions,
        }
    }
}

pub struct RevertManager {
    worktree: PathBuf,
}

impl RevertManager {
    pub fn new(worktree: PathBuf) -> Self {
        Self { worktree }
    }

    pub fn revert(
        &self,
        input: &RevertInput,
        manager: &mut SessionManager,
    ) -> Result<Option<Session>> {
        let session = match manager.get(&input.session_id) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };

        let messages = &session.messages;
        let mut last_user_id: Option<String> = None;
        let mut revert: Option<RevertInfo> = None;

        for msg in messages {
            if msg.role == crate::MessageRole::User {
                last_user_id = Some(msg.id.clone());
            }

            if revert.is_some() {
                continue;
            }

            if msg.id == input.message_id {
                if input.part_id.is_none() {
                    revert = Some(RevertInfo {
                        message_id: last_user_id.clone().unwrap_or_else(|| msg.id.clone()),
                        part_id: None,
                        snapshot: None,
                        diff: None,
                    });
                }
            }

            for part in &msg.parts {
                if Some(&part.id) == input.part_id.as_ref() {
                    revert = Some(RevertInfo {
                        message_id: msg.id.clone(),
                        part_id: input.part_id.clone(),
                        snapshot: None,
                        diff: None,
                    });
                }
            }
        }

        if let Some(mut rev) = revert {
            let existing_snapshot = session.revert.as_ref().and_then(|r| r.snapshot.clone());

            rev.snapshot = Some(
                existing_snapshot
                    .unwrap_or_else(|| Snapshot::track(&self.worktree).unwrap_or_default()),
            );

            if let Some(ref snapshot) = rev.snapshot {
                let diffs = Snapshot::diff(&self.worktree, snapshot)?;
                rev.diff = if diffs.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&diffs)?)
                };
            }

            if let Some(session) = manager.get_mut(&input.session_id) {
                let session_revert = crate::session::SessionRevert {
                    message_id: rev.message_id.clone(),
                    part_id: rev.part_id.clone(),
                    snapshot: rev.snapshot.clone(),
                    diff: rev.diff.clone(),
                };
                session.set_revert(session_revert);
                return Ok(Some(session.clone()));
            }
        }

        Ok(Some(session))
    }

    pub fn unrevert(
        &self,
        session_id: &str,
        manager: &mut SessionManager,
    ) -> Result<Option<Session>> {
        let session = match manager.get(session_id) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };

        let revert = match &session.revert {
            Some(r) => r.clone(),
            None => return Ok(Some(session)),
        };

        if let Some(ref snapshot) = revert.snapshot {
            let _ = Snapshot::restore(&self.worktree, snapshot);
        }

        if let Some(session) = manager.get_mut(session_id) {
            session.clear_revert();
            return Ok(Some(session.clone()));
        }

        Ok(Some(session))
    }

    pub fn cleanup(
        &self,
        session_id: &str,
        revert: &RevertInfo,
        manager: &mut SessionManager,
    ) -> Result<()> {
        let session = match manager.get(session_id) {
            Some(s) => s.clone(),
            None => return Ok(()),
        };

        let message_id = &revert.message_id;
        let messages = &session.messages;

        let mut preserve: Vec<crate::SessionMessage> = Vec::new();
        let mut target_msg: Option<crate::SessionMessage> = None;

        for msg in messages {
            if msg.id < *message_id {
                preserve.push(msg.clone());
                continue;
            }
            if msg.id > *message_id {
                continue;
            }
            if revert.part_id.is_some() {
                target_msg = Some(msg.clone());
                continue;
            }
        }

        if let (Some(part_id), Some(mut target)) = (&revert.part_id, target_msg) {
            let remove_start = target.parts.iter().position(|p| &p.id == part_id);

            if let Some(start) = remove_start {
                let preserve_parts: Vec<crate::MessagePart> =
                    target.parts.iter().take(start).cloned().collect();
                target.parts = preserve_parts;
                preserve.push(target);
            }
        }

        if let Some(session) = manager.get_mut(session_id) {
            session.messages = preserve;
            session.clear_revert();
        }

        Ok(())
    }

    /// Compute file diffs from session messages.
    ///
    /// Mirrors the TS `SessionSummary.computeDiff` behavior:
    /// 1. Scans messages for snapshot hashes stored in metadata
    ///    (keys `"step_start_snapshot"` / `"step_finish_snapshot"`).
    ///    If both an earliest "from" and a latest "to" snapshot are found,
    ///    delegates to `Snapshot::diff_full` (git diff between two refs).
    /// 2. Falls back to aggregating `Patch` parts by filepath, counting
    ///    per-line additions and deletions from `old_string` / `new_string`.
    pub fn compute_diff(&self, messages: &[crate::SessionMessage]) -> Result<Vec<FileDiff>> {
        // --- Strategy 1: snapshot-based diff (matches TS step-start / step-finish) ---
        let mut from_snapshot: Option<String> = None;
        let mut to_snapshot: Option<String> = None;

        for msg in messages {
            // Check message-level metadata for snapshot hashes
            if from_snapshot.is_none() {
                if let Some(serde_json::Value::String(s)) = msg.metadata.get("step_start_snapshot")
                {
                    if !s.is_empty() {
                        from_snapshot = Some(s.clone());
                    }
                }
            }
            if let Some(serde_json::Value::String(s)) = msg.metadata.get("step_finish_snapshot") {
                if !s.is_empty() {
                    to_snapshot = Some(s.clone());
                }
            }

            // Also scan parts: StepStart / StepFinish carry id+name / id+output,
            // but the metadata on the *message* is the canonical place for snapshots
            // in the v1 format. We also check part-level metadata stored in the
            // message metadata under "snapshot" as a generic key.
            if from_snapshot.is_none() {
                if let Some(serde_json::Value::String(s)) = msg.metadata.get("snapshot") {
                    if !s.is_empty() {
                        from_snapshot = Some(s.clone());
                    }
                }
            }
        }

        if let (Some(ref from), Some(ref to)) = (&from_snapshot, &to_snapshot) {
            let diffs = Snapshot::diff_full(&self.worktree, from, to)?;
            if !diffs.is_empty() {
                return Ok(diffs.into_iter().map(FileDiff::from).collect());
            }
        }

        // --- Strategy 2: aggregate Patch parts ---
        let mut file_stats: HashMap<String, (u64, u64)> = HashMap::new();

        for msg in messages {
            for part in &msg.parts {
                if let crate::PartType::Patch {
                    ref old_string,
                    ref new_string,
                    ref filepath,
                } = part.part_type
                {
                    let (additions, deletions) = count_line_changes(old_string, new_string);
                    let entry = file_stats.entry(filepath.clone()).or_insert((0, 0));
                    entry.0 += additions;
                    entry.1 += deletions;
                }
            }
        }

        let mut diffs: Vec<FileDiff> = file_stats
            .into_iter()
            .map(|(path, (additions, deletions))| FileDiff {
                path,
                additions,
                deletions,
            })
            .collect();

        // Sort by path for deterministic output
        diffs.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(diffs)
    }
}

/// Count line-level additions and deletions between two strings.
///
/// Uses a simple longest-common-subsequence (LCS) diff on lines:
/// lines present only in `new` are additions, lines present only in `old`
/// are deletions.
fn count_line_changes(old: &str, new: &str) -> (u64, u64) {
    if old == new {
        return (0, 0);
    }

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Fast path: entirely new content (old was empty)
    if old_lines.is_empty() {
        return (new_lines.len() as u64, 0);
    }
    // Fast path: content deleted entirely
    if new_lines.is_empty() {
        return (0, old_lines.len() as u64);
    }

    // Myers-style simple diff via LCS length.
    // We only need counts, not the actual edit script.
    let lcs_len = lcs_length(&old_lines, &new_lines);
    let deletions = old_lines.len() as u64 - lcs_len;
    let additions = new_lines.len() as u64 - lcs_len;

    (additions, deletions)
}

/// Compute the length of the longest common subsequence of two line slices.
/// Uses the classic DP approach with O(min(m,n)) space.
fn lcs_length(a: &[&str], b: &[&str]) -> u64 {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let n = short.len();
    let mut prev = vec![0u64; n + 1];
    let mut curr = vec![0u64; n + 1];

    for long_line in long {
        for (j, short_line) in short.iter().enumerate() {
            if long_line == short_line {
                curr[j + 1] = prev[j] + 1;
            } else {
                curr[j + 1] = curr[j].max(prev[j + 1]);
            }
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.iter_mut().for_each(|v| *v = 0);
    }

    *prev.last().unwrap_or(&0)
}

impl Default for RevertManager {
    fn default() -> Self {
        Self::new(PathBuf::from("."))
    }
}

pub fn revert(input: &RevertInput, manager: &mut SessionManager) -> Result<Option<Session>> {
    RevertManager::default().revert(input, manager)
}

pub fn unrevert(session_id: &str, manager: &mut SessionManager) -> Result<Option<Session>> {
    RevertManager::default().unrevert(session_id, manager)
}

pub fn cleanup(session_id: &str, revert: &RevertInfo, manager: &mut SessionManager) -> Result<()> {
    RevertManager::default().cleanup(session_id, revert, manager)
}
