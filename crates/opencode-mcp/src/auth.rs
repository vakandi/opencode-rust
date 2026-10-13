//! Persistent storage for MCP OAuth credentials.
//!
//! Mirrors the TypeScript `McpAuth` namespace – stores tokens, client info,
//! code verifiers and OAuth state in a JSON file inside the user data directory.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;

/// OAuth tokens obtained from the authorization server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix timestamp (seconds) at which the access token expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Dynamically-registered (or pre-configured) client information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthClientInfo {
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id_issued_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret_expires_at: Option<f64>,
}

/// A single entry in the auth store, keyed by MCP server name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<OAuthTokens>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_info: Option<OAuthClientInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_verifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_state: Option<String>,
    /// The server URL these credentials belong to – invalidated on URL change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
}
/// Resolve the path to the auth JSON file.
fn auth_file_path() -> PathBuf {
    opencode_paths::data_file(
        || dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")),
        "mcp-auth.json",
    )
}

/// Read the entire auth store from disk.
async fn read_all() -> HashMap<String, AuthEntry> {
    let path = auth_file_path();
    match fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Write the entire auth store to disk (creates parent dirs as needed).
async fn write_all(data: &HashMap<String, AuthEntry>) -> Result<(), std::io::Error> {
    let path = auth_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    fs::write(&path, json).await
}

// ---------------------------------------------------------------------------
// Public API – mirrors TS McpAuth namespace
// ---------------------------------------------------------------------------

/// Get the auth entry for a given MCP server name.
pub async fn get(mcp_name: &str) -> Option<AuthEntry> {
    let data = read_all().await;
    data.get(mcp_name).cloned()
}

/// Get the auth entry only if it was stored for the same `server_url`.
pub async fn get_for_url(mcp_name: &str, server_url: &str) -> Option<AuthEntry> {
    let entry = get(mcp_name).await?;
    match &entry.server_url {
        Some(url) if url == server_url => Some(entry),
        _ => None,
    }
}
/// Persist an auth entry (optionally updating the server URL).
pub async fn set(
    mcp_name: &str,
    entry: AuthEntry,
    server_url: Option<&str>,
) -> Result<(), std::io::Error> {
    let mut data = read_all().await;
    let mut entry = entry;
    if let Some(url) = server_url {
        entry.server_url = Some(url.to_string());
    }
    data.insert(mcp_name.to_string(), entry);
    write_all(&data).await
}

/// Remove all stored auth data for a server.
pub async fn remove(mcp_name: &str) -> Result<(), std::io::Error> {
    let mut data = read_all().await;
    data.remove(mcp_name);
    write_all(&data).await
}

/// Update only the tokens portion of an entry.
pub async fn update_tokens(
    mcp_name: &str,
    tokens: OAuthTokens,
    server_url: Option<&str>,
) -> Result<(), std::io::Error> {
    let mut entry = get(mcp_name).await.unwrap_or_default();
    entry.tokens = Some(tokens);
    set(mcp_name, entry, server_url).await
}

/// Update only the client info portion of an entry.
pub async fn update_client_info(
    mcp_name: &str,
    info: OAuthClientInfo,
    server_url: Option<&str>,
) -> Result<(), std::io::Error> {
    let mut entry = get(mcp_name).await.unwrap_or_default();
    entry.client_info = Some(info);
    set(mcp_name, entry, server_url).await
}

/// Store the PKCE code verifier.
pub async fn update_code_verifier(
    mcp_name: &str,
    code_verifier: &str,
) -> Result<(), std::io::Error> {
    let mut entry = get(mcp_name).await.unwrap_or_default();
    entry.code_verifier = Some(code_verifier.to_string());
    set(mcp_name, entry, None).await
}

/// Clear the stored code verifier.
pub async fn clear_code_verifier(mcp_name: &str) -> Result<(), std::io::Error> {
    if let Some(mut entry) = get(mcp_name).await {
        entry.code_verifier = None;
        set(mcp_name, entry, None).await?;
    }
    Ok(())
}
/// Store the OAuth state parameter.
pub async fn update_oauth_state(mcp_name: &str, state: &str) -> Result<(), std::io::Error> {
    let mut entry = get(mcp_name).await.unwrap_or_default();
    entry.oauth_state = Some(state.to_string());
    set(mcp_name, entry, None).await
}

/// Read the stored OAuth state.
pub async fn get_oauth_state(mcp_name: &str) -> Option<String> {
    get(mcp_name).await.and_then(|e| e.oauth_state)
}

/// Clear the stored OAuth state.
pub async fn clear_oauth_state(mcp_name: &str) -> Result<(), std::io::Error> {
    if let Some(mut entry) = get(mcp_name).await {
        entry.oauth_state = None;
        set(mcp_name, entry, None).await?;
    }
    Ok(())
}

/// Check whether stored tokens are expired.
/// Returns `None` if no tokens exist, `Some(false)` if not expired (or no
/// expiry set), `Some(true)` if expired.
pub fn is_token_expired(entry: &AuthEntry) -> Option<bool> {
    let tokens = entry.tokens.as_ref()?;
    match tokens.expires_at {
        Some(exp) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            Some(exp < now)
        }
        None => Some(false),
    }
}
