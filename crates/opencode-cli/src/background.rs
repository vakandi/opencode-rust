//! Reading the oh-my-opencode continuation ledger.
//!
//! omo records background work in `<dir>/.omo/run-continuation/<sessionID>.json` as
//! `{"sessionID":…,"sources":{"background-task":{"state":"idle"|"active"}}}`. `run` blocks
//! until that turns idle so background tasks are not cut off.
//!
//! Entries in the wild are permanently stale — this repo had 3 of 106 stuck on `active` —
//! so a staleness guard and an absolute timeout are mandatory. Without them `run` would hang
//! forever on its own data.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundState {
    Idle,
    Active,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainReport {
    Idle,
    Stale,
    TimedOut,
}

#[derive(Debug, Clone, Copy)]
pub struct DrainOptions {
    pub timeout: Duration,
    pub stale_after: Duration,
    pub poll_interval: Duration,
}

impl Default for DrainOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            stale_after: Duration::from_secs(60),
            poll_interval: Duration::from_millis(500),
        }
    }
}

pub fn omo_root(dir: &Path) -> PathBuf {
    dir.join(".omo").join("run-continuation")
}

fn parse_updated_at(raw: Option<&str>) -> Option<SystemTime> {
    // omo writes ISO-8601; anything unparseable is treated as "no timestamp", which the
    // caller treats as fresh so a malformed clock never silently unblocks `run`.
    let raw = raw?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| SystemTime::UNIX_EPOCH + Duration::from_millis(dt.timestamp_millis() as u64))
}

fn evaluate(json: &serde_json::Value, stale_after: Duration) -> BackgroundState {
    let state = json
        .get("sources")
        .and_then(|s| s.get("background-task"))
        .and_then(|b| b.get("state"))
        .and_then(|v| v.as_str())
        .unwrap_or("idle");

    if state != "active" {
        return BackgroundState::Idle;
    }

    match parse_updated_at(
        json.get("sources")
            .and_then(|s| s.get("background-task"))
            .and_then(|b| b.get("updatedAt"))
            .and_then(|v| v.as_str()),
    ) {
        Some(updated) if updated.elapsed().unwrap_or_default() > stale_after => {
            BackgroundState::Stale
        }
        _ => BackgroundState::Active,
    }
}

pub fn read_ledger_state(dir: &Path, session_id: &str, stale_after: Duration) -> BackgroundState {
    let path = omo_root(dir).join(format!("{session_id}.json"));
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BackgroundState::Idle;
    };
    // Fail open: a corrupt ledger must not stall `run` forever.
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(json) => evaluate(&json, stale_after),
        Err(_) => BackgroundState::Idle,
    }
}

pub async fn wait_for_background_idle(
    dir: &Path,
    session_id: &str,
    opts: DrainOptions,
) -> DrainReport {
    let deadline = std::time::Instant::now() + opts.timeout;
    loop {
        match read_ledger_state(dir, session_id, opts.stale_after) {
            BackgroundState::Idle => return DrainReport::Idle,
            BackgroundState::Stale => return DrainReport::Stale,
            BackgroundState::Active => {}
        }
        if std::time::Instant::now() >= deadline {
            return DrainReport::TimedOut;
        }
        tokio::time::sleep(opts.poll_interval).await;
    }
}