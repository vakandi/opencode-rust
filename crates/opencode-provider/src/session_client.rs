//! HTTP clients for provider traffic.
//!
//! The public build ships no proxy layer: one process-wide client is enough, and
//! provider requests reach the API directly (or through whatever the ambient
//! environment configures, which reqwest reads when this client is built).
//!
//! `for_session` and `release_session` keep their per-session signatures because the
//! provider adapters are shared with the private build. Here they resolve to the same
//! client; there is no session-scoped egress to give anyone.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::Client;

static CLIENT: OnceLock<Arc<Client>> = OnceLock::new();

/// The client every provider call uses.
pub fn for_session(_session_id: &str) -> Arc<Client> {
    client()
}

/// No-op in the public build: there is no per-session client to release.
pub fn release_session(_session_id: &str) {}

fn client() -> Arc<Client> {
    CLIENT
        .get_or_init(|| Arc::new(builder().build().unwrap_or_else(|_| Client::new())))
        .clone()
}

fn builder() -> reqwest::ClientBuilder {
    Client::builder()
        // Provider responses are long SSE streams. A read timeout short enough to catch
        // a dead network would also cut a healthy stream mid-answer.
        .connect_timeout(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(30))
}
