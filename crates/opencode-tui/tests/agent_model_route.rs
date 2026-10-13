//! `/agent` must carry the model each agent declares in config.
//!
//! The TUI resolves its starting model from the active agent, so a route that
//! drops `model` silently leaves the TUI on the global default instead — which
//! is how the TUI ended up showing a model no agent had asked for.

use std::sync::Arc;

async fn serve_router() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = opencode_server::routes::router()
        .with_state(Arc::new(opencode_server::ServerState::new()));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    base_url
}

#[tokio::test]
async fn agent_listing_exposes_the_model_each_agent_declares() {
    let base_url = serve_router().await;
    let agents: Vec<serde_json::Value> = reqwest::Client::new()
        .get(format!("{base_url}/agent"))
        .send()
        .await
        .expect("GET /agent")
        .json()
        .await
        .expect("decode /agent");

    assert!(!agents.is_empty(), "/agent returned no agents");

    let with_model = agents
        .iter()
        .filter(|agent| {
            agent
                .get("model")
                .and_then(|value| value.as_str())
                .is_some()
        })
        .count();

    assert!(
        with_model >= agents.len() / 2,
        "agents declare models in config, so most /agent entries must carry one: \
         only {with_model}/{} did. First entry keys: {:?}",
        agents.len(),
        agents[0]
            .as_object()
            .map(|object| object.keys().collect::<Vec<_>>()),
    );
}