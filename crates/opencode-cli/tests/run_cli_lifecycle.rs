//! Lifecycle tests for `opencode run`.
//!
//! The bug under test: `run` used `AgentExecutor::execute_streaming`, a single provider call that
//! never executes a tool, so a turn that spawned a subagent, a team or a background task returned
//! immediately and the CLI exited. These tests pin the two halves of the fix — the turn loop, and
//! the drain that holds the process open until child work is finished.

use std::time::Duration;

use opencode_cli::background::{
    read_ledger_state, wait_for_background_idle, BackgroundState, DrainOptions, DrainReport,
};

fn fast_opts() -> DrainOptions {
    DrainOptions {
        timeout: Duration::from_secs(2),
        stale_after: Duration::from_secs(60),
        poll_interval: Duration::from_millis(20),
    }
}

fn ledger(dir: &std::path::Path, session: &str, state: &str, updated_at: &str) {
    let root = dir.join(".omo").join("run-continuation");
    std::fs::create_dir_all(&root).unwrap();
    let body = serde_json::json!({
        "sessionID": session,
        "updatedAt": updated_at,
        "sources": {
            "background-task": { "state": state, "updatedAt": updated_at }
        }
    });
    std::fs::write(
        root.join(format!("{session}.json")),
        serde_json::to_string_pretty(&body).unwrap(),
    )
    .unwrap();
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[test]
fn t6_ledger_reports_active_then_idle() {
    let dir = tempfile::tempdir().unwrap();
    let session = "ses_ledger_a";
    ledger(dir.path(), session, "active", &now_iso());

    assert_eq!(
        read_ledger_state(dir.path(), session, Duration::from_secs(60)),
        BackgroundState::Active,
        "a fresh `active` entry must read as Active, otherwise run exits while work is live"
    );

    ledger(dir.path(), session, "idle", &now_iso());
    assert_eq!(
        read_ledger_state(dir.path(), session, Duration::from_secs(60)),
        BackgroundState::Idle
    );
}

#[tokio::test]
async fn t7_wait_times_out_on_a_stale_active_entry() {
    let dir = tempfile::tempdir().unwrap();
    let session = "ses_ledger_stale";
    // This repo has 3 ledger entries permanently stuck on `active`; without a staleness guard
    // `run` would hang forever on its own data.
    ledger(dir.path(), session, "active", "2020-01-01T00:00:00Z");

    let started = std::time::Instant::now();
    let report = wait_for_background_idle(
        dir.path(),
        session,
        DrainOptions {
            stale_after: Duration::from_millis(1),
            ..fast_opts()
        },
    )
    .await;

    assert_eq!(report, DrainReport::Stale);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a stale entry must short-circuit, not wait out the full timeout"
    );
}

#[tokio::test]
async fn t8_wait_returns_immediately_when_no_ledger_exists() {
    let dir = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();

    let report = wait_for_background_idle(
        dir.path(),
        "ses_absent",
        DrainOptions {
            timeout: Duration::from_secs(120),
            ..fast_opts()
        },
    )
    .await;

    assert_eq!(report, DrainReport::Idle);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a session with no omo background work must not stall for the full timeout"
    );
}

#[tokio::test]
async fn t9_wait_resolves_once_background_work_completes() {
    let dir = tempfile::tempdir().unwrap();
    let session = "ses_ledger_finishes";
    ledger(dir.path(), session, "active", &now_iso());

    let path = dir.path().to_path_buf();
    let sid = session.to_string();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        ledger(&path, &sid, "idle", &now_iso());
    });

    let report = wait_for_background_idle(
        dir.path(),
        session,
        DrainOptions {
            timeout: Duration::from_secs(10),
            ..fast_opts()
        },
    )
    .await;
    writer.join().unwrap();

    assert_eq!(
        report,
        DrainReport::Idle,
        "the drain must hold the process open and release once the ledger flips idle"
    );
}

#[tokio::test]
async fn t10_corrupt_ledger_does_not_stall_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let session = "ses_corrupt";
    let root = dir.path().join(".omo").join("run-continuation");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(format!("{session}.json")), "{not json").unwrap();

    let started = std::time::Instant::now();
    let report = wait_for_background_idle(
        dir.path(),
        session,
        DrainOptions {
            timeout: Duration::from_secs(120),
            ..fast_opts()
        },
    )
    .await;

    assert_eq!(report, DrainReport::Idle);
    assert!(started.elapsed() < Duration::from_secs(1));
}