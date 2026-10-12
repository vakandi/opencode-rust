//! Resolving child (subagent / team member) sessions from the oh-my-opencode database.
//!
//! omo spawns its subagents and team members as real rows in `~/.local/share/opencode/opencode.db`
//! with `session.parent_id` pointing at the parent. This is a *separate* database from the one the
//! Rust port writes (that one uses plural table names and lives under the platform data dir), so the
//! reads here go through their own read-only connection.

use std::path::PathBuf;

use sqlx::Row;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSession {
    pub id: String,
    pub title: String,
}

/// Where omo keeps its database. `None` when the file is absent, which means no omo
/// children can exist.
pub fn omo_db_path() -> Option<PathBuf> {
    let path = PathBuf::from(shellexpand_home())
        .join(".local/share/opencode/opencode.db");
    path.exists().then_some(path)
}

fn shellexpand_home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

pub async fn list_children(db: &std::path::Path, parent_id: &str) -> Vec<ChildSession> {
    if !db.exists() {
        return Vec::new();
    }
    let url = format!("sqlite://{}?mode=ro", db.display());
    let Ok(pool) = sqlx::SqlitePool::connect(&url).await else {
        return Vec::new();
    };

    let rows = sqlx::query("SELECT id, title FROM session WHERE parent_id = ? ORDER BY time_created")
        .bind(parent_id)
        .fetch_all(&pool)
        .await;

    let Ok(rows) = rows else {
        return Vec::new();
    };

    rows.into_iter()
        .map(|row| ChildSession {
            id: row.get::<String, _>("id"),
            title: row.get::<Option<String>, _>("title").unwrap_or_default(),
        })
        .collect()
}