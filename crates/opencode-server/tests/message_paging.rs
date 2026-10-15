//! `GET /session/{id}/message` paging: `limit`, `before`, and `x-total-messages`.
//!
//! These drive the real `Router` through `tower`'s `oneshot`, so they prove the
//! handler — including the header, which a body-only assertion cannot see.
//!
//! The shape these assertions encode: `POST .../message` stores the user message
//! *and* an empty assistant placeholder, so N posts produce 2N rows alternating
//! user/assistant. Only the user rows carry text.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencode_server::{routes, ServerState};
use serde_json::{json, Value};
use tower::ServiceExt;

type App = axum::Router;

fn app() -> App {
    routes::router().with_state(Arc::new(ServerState::new()))
}

async fn post_json(app: &App, path: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
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

/// Returns status, parsed body and the `x-total-messages` header.
async fn get_messages(app: &App, path: &str) -> (StatusCode, Value, Option<String>) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("request should build");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let total = response
        .headers()
        .get("x-total-messages")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should read");
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("body should be JSON")
    };
    (status, parsed, total)
}

async fn new_session(app: &App) -> String {
    let (status, body) = post_json(app, "/session", json!({ "title": "paging" })).await;
    assert_eq!(status, StatusCode::OK, "session create failed: {}", body);
    body["id"].as_str().expect("session id").to_string()
}

/// Post `n` user messages, returning the id of each stored user message in order.
async fn seed(app: &App, id: &str, n: usize) -> Vec<String> {
    let mut ids = Vec::new();
    for i in 0..n {
        let (status, body) = post_json(
            app,
            &format!("/session/{}/message", id),
            json!({ "parts": [{ "type": "text", "text": format!("m{}", i) }] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "seed {} failed: {}", i, body);
        ids.push(body["info"]["id"].as_str().expect("message id").to_string());
    }
    ids
}

fn ids_of(body: &Value) -> Vec<String> {
    body.as_array()
        .expect("message list must be an array")
        .iter()
        .map(|m| m["info"]["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Texts of the user-authored rows only; assistant placeholders carry no text.
fn user_texts(body: &Value) -> Vec<String> {
    body.as_array()
        .expect("message list must be an array")
        .iter()
        .filter(|m| m["info"]["role"].as_str() == Some("user"))
        .map(|m| m["parts"][0]["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn no_query_still_returns_the_whole_transcript() {
    let app = app();
    let id = new_session(&app).await;
    seed(&app, &id, 5).await;

    let (status, body, total) = get_messages(&app, &format!("/session/{}/message", id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_array().expect("array").len(),
        10,
        "no-query must stay byte-compatible: 5 posts are 10 rows"
    );
    assert_eq!(total.as_deref(), Some("10"));
}

#[tokio::test]
async fn limit_returns_the_tail_page_and_keeps_the_full_total() {
    let app = app();
    let id = new_session(&app).await;
    seed(&app, &id, 10).await;

    let (status, body, total) =
        get_messages(&app, &format!("/session/{}/message?limit=3", id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("array").len(), 3);
    assert_eq!(user_texts(&body), vec!["m9"], "limit takes the newest page");
    assert_eq!(
        total.as_deref(),
        Some("20"),
        "header counts the session, not the page"
    );
}

#[tokio::test]
async fn paging_backwards_reconstructs_the_transcript_without_overlap() {
    let app = app();
    let id = new_session(&app).await;
    seed(&app, &id, 10).await;

    let (_, full, _) = get_messages(&app, &format!("/session/{}/message", id)).await;
    let full_ids = ids_of(&full);

    // The cursor is the OLDEST row of the page just received — the same row the
    // control bridge publishes as `next_before`.
    let mut pages: Vec<Vec<String>> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let path = match &cursor {
            Some(c) => format!("/session/{}/message?limit=3&before={}", id, c),
            None => format!("/session/{}/message?limit=3", id),
        };
        let (_, page, _) = get_messages(&app, &path).await;
        let page_ids = ids_of(&page);
        if page_ids.is_empty() {
            break;
        }
        let seen: Vec<&String> = pages.iter().flatten().collect();
        assert!(
            page_ids.iter().all(|p| !seen.contains(&p)),
            "a page must not repeat rows already fetched"
        );
        cursor = Some(page_ids[0].clone());
        let short = page_ids.len() < 3;
        pages.push(page_ids);
        if short {
            break;
        }
    }

    // Pages arrive newest-first; each page is already ascending. Rebuild by walking
    // the pages backwards and flattening — reversing the flat list would also flip
    // the order inside every page.
    let rebuilt: Vec<String> = pages.into_iter().rev().flatten().collect();
    assert_eq!(
        rebuilt, full_ids,
        "paging backwards must reconstruct the transcript exactly"
    );
}

#[tokio::test]
async fn before_the_first_row_yields_an_empty_page() {
    let app = app();
    let id = new_session(&app).await;
    seed(&app, &id, 4).await;

    let (_, full, _) = get_messages(&app, &format!("/session/{}/message", id)).await;
    let first = ids_of(&full).first().expect("non-empty").clone();

    let (status, body, _) = get_messages(
        &app,
        &format!("/session/{}/message?limit=3&before={}", id, first),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_array().expect("array").len(),
        0,
        "nothing exists before the first row"
    );
}

#[tokio::test]
async fn an_unknown_cursor_returns_an_empty_page() {
    let app = app();
    let id = new_session(&app).await;
    seed(&app, &id, 6).await;

    let (status, body, total) = get_messages(
        &app,
        &format!("/session/{}/message?limit=3&before=msg_does_not_exist", id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_array().expect("array").len(),
        0,
        "a bad cursor must not fall back to the whole transcript"
    );
    assert_eq!(total.as_deref(), Some("12"));
}

#[tokio::test]
async fn limit_larger_than_the_transcript_is_harmless() {
    let app = app();
    let id = new_session(&app).await;
    seed(&app, &id, 2).await;

    let (status, body, _) =
        get_messages(&app, &format!("/session/{}/message?limit=500", id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user_texts(&body), vec!["m0", "m1"]);
}

#[tokio::test]
async fn a_missing_session_still_404s_with_the_query_attached() {
    let app = app();
    let (status, _, _) =
        get_messages(&app, "/session/ses_nope/message?limit=3&before=msg_x").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
