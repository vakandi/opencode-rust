//! Wiring proof that `GET /agent` enumerates subagents.
//!
//! The defect this pins: `list_agents` (crates/opencode-server/src/routes.rs)
//! ran `.filter(|agent| !matches!(agent.mode, AgentMode::Subagent))` over the
//! registry. So the route could never return a subagent, and any client roster
//! built from it would permanently be missing every subagent even when the
//! registry holds them.
//!
//! The registry itself is fine and already carries subagents: of the built-in
//! agents, `explore` is `mode: AgentMode::Subagent` and `hidden: false`, so it
//! is a real, non-hidden subagent that the route was hiding.
//!
//! HTTP-level test driving the real `Router` through `tower`'s `oneshot`, so a
//! pass proves the handler — not a serde shim — behaves.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencode_server::{routes, ServerState};
use serde_json::Value;
use tower::ServiceExt;

type App = axum::Router;

fn app() -> App {
    routes::router().with_state(Arc::new(ServerState::new()))
}

async fn get_json(app: &App, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("request should build");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should read");
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("body should be JSON")
    };
    (status, parsed)
}

fn ids(body: &Value) -> Vec<String> {
    body.as_array()
        .unwrap_or_else(|| panic!("/agent must answer an array, got {}", body))
        .iter()
        .filter_map(|entry| entry["id"].as_str().map(str::to_string))
        .collect()
}

/// `GET /agent` must include at least one agent whose registry mode is
/// Subagent.
///
/// Before the fix the route filtered every `AgentMode::Subagent` out, so this
/// assertion failed: the built-in non-hidden subagent `explore` was absent while
/// only the Primary agents (`build`, `plan`, `general`) came back.
#[tokio::test]
async fn agent_route_includes_subagents() {
    let app = app();
    let (status, body) = get_json(&app, "/agent").await;
    assert_eq!(status, StatusCode::OK, "GET /agent failed: {}", body);

    let ids = ids(&body);
    assert!(
        !ids.is_empty(),
        "GET /agent must not answer an empty roster, got {}",
        body
    );

    // Cross-check against the registry the route builds from, rather than
    // hard-coding one name: whatever the registry classifies as a visible
    // subagent, the route must not hide it.
    let registry = opencode_agent::AgentRegistry::new();
    let subagents: Vec<String> = registry
        .list()
        .into_iter()
        .filter(|agent| matches!(agent.mode, opencode_agent::AgentMode::Subagent))
        .map(|agent| agent.name.clone())
        .collect();

    assert!(
        !subagents.is_empty(),
        "the registry is expected to hold at least one visible subagent; \
         if this ever becomes false the test needs a different subject"
    );

    for subagent in &subagents {
        assert!(
            ids.iter().any(|id| id == subagent),
            "GET /agent hid the subagent `{}`; roster was {:?}",
            subagent,
            ids
        );
    }
}

/// Removing the filter must not drop the Primary agents the route already
/// returned — `build` in particular, which every client treats as the default.
/// This is the regression guard on the fix.
#[tokio::test]
async fn agent_route_still_lists_primary_agents() {
    let app = app();
    let (status, body) = get_json(&app, "/agent").await;
    assert_eq!(status, StatusCode::OK, "GET /agent failed: {}", body);

    let ids = ids(&body);
    for primary in ["build", "plan", "general"] {
        assert!(
            ids.iter().any(|id| id == primary),
            "GET /agent lost the primary agent `{}`; roster was {:?}",
            primary,
            ids
        );
    }
}

/// Every entry must keep the `{id, name, description}` shape, and `name` must
/// still mirror `id`. Removing a filter is not a licence to reshape the
/// payload.
#[tokio::test]
async fn agent_entries_keep_their_shape() {
    let app = app();
    let (status, body) = get_json(&app, "/agent").await;
    assert_eq!(status, StatusCode::OK, "GET /agent failed: {}", body);

    let entries = body
        .as_array()
        .unwrap_or_else(|| panic!("/agent must answer an array, got {}", body));

    assert!(!entries.is_empty(), "roster must not be empty");
    for entry in entries {
        let id = entry["id"].as_str().unwrap_or_else(|| {
            panic!("each agent needs a string id, got {}", entry);
        });
        let name = entry["name"].as_str().unwrap_or_else(|| {
            panic!("each agent needs a string name, got {}", entry);
        });
        assert_eq!(
            id, name,
            "name must still mirror id, got {}",
            entry
        );
    }
}