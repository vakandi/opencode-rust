//! Observability of child (subagent / team member) sessions created by oh-my-opencode.
//!
//! omo records every spawned member in its own SQLite database using singular table names
//! (`session`, `message`, `part`) — a different schema from the one this port writes. These tests
//! pin that `run` can resolve children and read their live output.

use opencode_cli::children::{list_children, omo_db_path};
use opencode_cli::livestream::poll_frames;

async fn omo_db(path: &std::path::Path) -> sqlx::SqlitePool {
    // mode=rwc creates the file; without it sqlx refuses to open a non-existent database and the
    // test fails in setup instead of on the assertion it is meant to pin.
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query(
        "CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, title TEXT, time_created INTEGER)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TABLE part (id TEXT PRIMARY KEY, session_id TEXT, data TEXT, time_created INTEGER)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[tokio::test]
async fn children_resolve_through_the_parent_link() {
    let dir = tempfile::tempdir().unwrap();
    let pool = omo_db(&dir.path().join("opencode.db")).await;
    let parent = "ses_parent";

    for (id, parent_id, title) in [
        ("ses_child_a", parent, "explore the run path (@explore subagent)"),
        ("ses_child_b", parent, "plan the fix (@plan subagent)"),
        ("ses_other", "ses_unrelated", "not mine"),
    ] {
        sqlx::query("INSERT INTO session (id, parent_id, title, time_created) VALUES (?,?,?,1)")
            .bind(id)
            .bind(parent_id)
            .bind(title)
            .execute(&pool)
            .await
            .unwrap();
    }

    let db = dir.path().join("opencode.db");
    let children = list_children(&db, parent).await;

    let ids: Vec<&str> = children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["ses_child_a", "ses_child_b"],
        "only the parent's own children must resolve"
    );
    assert!(children[0].title.contains("explore"), "title must survive");
}

#[tokio::test]
async fn children_are_empty_when_there_are_none() {
    let dir = tempfile::tempdir().unwrap();
    let _pool = omo_db(&dir.path().join("opencode.db")).await;
    let db = dir.path().join("opencode.db");
    assert!(list_children(&db, "ses_lonely").await.is_empty());
}

#[tokio::test]
async fn a_missing_database_yields_no_children_rather_than_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("does-not-exist.db");
    assert!(
        list_children(&db, "ses_parent").await.is_empty(),
        "no omo database means no omo children"
    );
}

#[tokio::test]
async fn frames_stream_incrementally_and_never_repeat() {
    let dir = tempfile::tempdir().unwrap();
    let pool = omo_db(&dir.path().join("opencode.db")).await;
    let session = "ses_child_a";

    for (id, kind, text) in [
        ("prt_1", "text", "first chunk"),
        ("prt_2", "text", "second chunk"),
        ("prt_3", "reasoning", "thinking out loud"),
    ] {
        let data = serde_json::json!({ "type": kind, "text": text });
        sqlx::query("INSERT INTO part (id, session_id, data, time_created) VALUES (?,?,?,?)")
            .bind(id)
            .bind(session)
            .bind(data.to_string())
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }

    let db = dir.path().join("opencode.db");

    let first = poll_frames(&db, session, "").await;
    assert_eq!(first.len(), 3, "a cold cursor must yield every frame");
    assert_eq!(first[0].text, "first chunk");
    assert_eq!(first[2].kind, "reasoning");

    let after_first = poll_frames(&db, session, &first[0].part_id).await;
    assert_eq!(
        after_first.len(),
        2,
        "an advanced cursor must not replay the frame it already reported"
    );

    let exhausted = poll_frames(&db, session, &first[2].part_id).await;
    assert!(exhausted.is_empty(), "no new parts means no new frames");

    assert!(
        first.iter().all(|f| f.session_id == session),
        "every frame must carry its originating session so the CLI can label it"
    );
}

#[tokio::test]
async fn omo_db_path_is_discovered_from_home() {
    let path = omo_db_path();
    match path {
        Some(p) => assert!(
            p.to_string_lossy().contains("opencode"),
            "expected an opencode database, got {}",
            p.display()
        ),
        None => {}
    }
}