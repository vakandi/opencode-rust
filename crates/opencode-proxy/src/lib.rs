//! Native rotating residential proxy for the Rust subworker server.
//!
//! A local HTTP/CONNECT forward proxy on loopback, fed by a repo-local pool file, that
//! re-selects the oldest-used upstream every 120 seconds. Probe-gated adoption: a
//! candidate must open a real CONNECT before it can become the process egress.
//!
//! The load-bearing ordering constraint lives in [`env::export_proxy_env`], and it is
//! not optional: reqwest snapshots the proxy environment when `Client::new()` is
//! called, so this must run before the provider registry is constructed.

pub mod claims;
pub mod config;
pub mod connect;
pub mod env;
pub mod listener;
pub mod pool;
pub mod probe;
pub mod quarantine;
pub mod rotate;
pub mod selection;
pub mod session;
pub mod state;
pub mod upstream;

use std::sync::Arc;

pub use config::ProxyConfig;
pub use state::ProxyState;
pub use upstream::Upstream;

/// A running proxy: the bound port, shared state, and the local URL.
pub struct RunningProxy {
    pub port: u16,
    pub state: Arc<ProxyState>,
    pub url: String,
}

/// Bind the listener, publish the environment, and start rotation.
///
/// # Ordering
/// This function MUST be called before any `reqwest::Client::new()`. It is not a
/// convenience wrapper — it is the only reason provider traffic goes through the proxy.
pub async fn start(cfg: ProxyConfig) -> std::io::Result<RunningProxy> {
    let cfg = Arc::new(cfg);
    if !cfg.enabled {
        tracing::info!("proxy disabled by configuration; leaving environment untouched");
        return Ok(RunningProxy {
            port: 0,
            state: Arc::new(ProxyState::new(Arc::clone(&cfg), pool::pool_file())),
            url: String::new(),
        });
    }

    let pool_path = pool::pool_file();
    if pool::load(&pool_path).is_empty() {
        // Refuse loudly rather than bind a listener that answers 502 for everything.
        // The reference starts a proxy with an empty pool and clients see a generic
        // connection error with no hint that the pool is the cause.
        tracing::error!(path = %pool_path.display(), "proxy pool is empty or unreadable");
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("proxy pool empty or unreadable: {}", pool_path.display()),
        ));
    }

    let (listener, port) = listener::bind_listener(&cfg).await?;
    let url = env::export_proxy_env(port).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "failed to export proxy environment")
    })?;
    tracing::info!(%port, %url, "local proxy listening");

    let state = Arc::new(ProxyState::new(Arc::clone(&cfg), pool_path));
    state.set_port(port);
    listener::accept_loop(listener, Arc::clone(&cfg), Arc::clone(&state));
    rotate::spawn(Arc::clone(&state));

    Ok(RunningProxy { port, state, url })
}