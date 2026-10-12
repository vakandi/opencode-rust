use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::Config;

const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

/// The JSON shape returned by `{url}/.well-known/opencode`.
#[derive(Debug, Deserialize)]
struct WellKnownResponse {
    #[serde(default)]
    config: Option<serde_json::Value>,
}

/// A single auth entry stored in `auth.json` with `type: "wellknown"`.
#[derive(Debug, Clone, Deserialize)]
struct WellKnownAuth {
    /// Environment variable name to expose the token as.
    key: String,
    /// The token value.
    token: String,
}

/// Wrapper for the full auth.json — we only care about wellknown entries.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum AuthEntry {
    #[serde(rename = "wellknown")]
    WellKnown { key: String, token: String },
    #[serde(other)]
    Other,
}

struct CacheEntry {
    config: Config,
    fetched_at: Instant,
}

static CACHE: Mutex<Option<HashMap<String, CacheEntry>>> = Mutex::new(None);
/// Returns the path to `auth.json` inside the opencode data directory.
fn auth_json_path() -> PathBuf {
    opencode_paths::data_file(
        || dirs::data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")),
        "auth.json",
    )
}

/// Reads `auth.json` and returns only the wellknown entries (url -> WellKnownAuth).
fn read_wellknown_entries() -> HashMap<String, WellKnownAuth> {
    let path = auth_json_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    let raw: HashMap<String, serde_json::Value> = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    let mut result = HashMap::new();
    for (url, value) in raw {
        if let Ok(AuthEntry::WellKnown { key, token }) = serde_json::from_value::<AuthEntry>(value)
        {
            result.insert(url, WellKnownAuth { key, token });
        }
    }
    result
}

/// Fetches remote config from all wellknown auth entries and returns a merged
/// `Config` representing the combined remote configuration.
///
/// This is the lowest-priority config source — it will be merged first so that
/// global and project configs override it.
///
/// Network failures are logged as warnings and never prevent startup.
pub async fn load_wellknown() -> Config {
    let entries = read_wellknown_entries();
    if entries.is_empty() {
        return Config::default();
    }

    let mut merged = Config::default();
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .unwrap_or_default();

    for (url, auth) in &entries {
        // Check cache first
        if let Some(cached) = get_cached(url) {
            tracing::debug!(url = %url, "using cached wellknown config");
            merged.merge(cached);
            continue;
        }

        // Set the env var so downstream code (e.g. provider auth) can use it,
        // matching the TS behaviour: `process.env[value.key] = value.token`
        std::env::set_var(&auth.key, &auth.token);

        let endpoint = format!("{}/.well-known/opencode", url.trim_end_matches('/'));
        tracing::debug!(url = %endpoint, "fetching remote wellknown config");

        match fetch_wellknown_config(&client, &endpoint).await {
            Ok(config) => {
                set_cached(url.clone(), config.clone());
                tracing::debug!(url = %url, "loaded remote config from well-known");
                merged.merge(config);
            }
            Err(e) => {
                tracing::warn!(url = %endpoint, error = %e, "failed to fetch wellknown config, skipping");
            }
        }
    }

    merged
}

async fn fetch_wellknown_config(client: &reqwest::Client, endpoint: &str) -> Result<Config> {
    let resp = client.get(endpoint).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} from {}", resp.status(), endpoint);
    }

    let wk: WellKnownResponse = resp.json().await?;
    let config_value = wk
        .config
        .unwrap_or(serde_json::Value::Object(Default::default()));
    let config: Config = serde_json::from_value(config_value)?;
    Ok(config)
}

fn get_cached(url: &str) -> Option<Config> {
    let guard = CACHE.lock().ok()?;
    let map = guard.as_ref()?;
    let entry = map.get(url)?;
    if entry.fetched_at.elapsed() < CACHE_TTL {
        Some(entry.config.clone())
    } else {
        None
    }
}

fn set_cached(url: String, config: Config) {
    let mut guard = match CACHE.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(
        url,
        CacheEntry {
            config,
            fetched_at: Instant::now(),
        },
    );
}

/// Clears the wellknown config cache. Useful for testing or forced refresh.
pub fn clear_cache() {
    if let Ok(mut guard) = CACHE.lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wellknown_auth_entries_from_json() {
        let json = r#"{
            "https://corp.example.com": {
                "type": "wellknown",
                "key": "CORP_TOKEN",
                "token": "secret-123"
            },
            "anthropic": {
                "type": "api",
                "key": "sk-ant-xxx"
            }
        }"#;

        let raw: HashMap<String, serde_json::Value> = serde_json::from_str(json).unwrap();
        let mut result = HashMap::new();
        for (url, value) in raw {
            if let Ok(AuthEntry::WellKnown { key, token }) =
                serde_json::from_value::<AuthEntry>(value)
            {
                result.insert(url, WellKnownAuth { key, token });
            }
        }

        assert_eq!(result.len(), 1);
        let entry = result.get("https://corp.example.com").unwrap();
        assert_eq!(entry.key, "CORP_TOKEN");
        assert_eq!(entry.token, "secret-123");
    }

    #[test]
    fn wellknown_response_parses_config_field() {
        let json = r#"{"config": {"model": "claude-3-opus"}}"#;
        let wk: WellKnownResponse = serde_json::from_str(json).unwrap();
        let config_val = wk.config.unwrap();
        let config: Config = serde_json::from_value(config_val).unwrap();
        assert_eq!(config.model.as_deref(), Some("claude-3-opus"));
    }

    #[test]
    fn wellknown_response_handles_missing_config() {
        let json = r#"{"auth": {"command": ["login"]}}"#;
        let wk: WellKnownResponse = serde_json::from_str(json).unwrap();
        assert!(wk.config.is_none());
    }

    #[test]
    fn cache_stores_and_retrieves() {
        clear_cache();
        let config = Config {
            model: Some("cached-model".to_string()),
            ..Default::default()
        };
        set_cached("https://test.example.com".to_string(), config);
        let cached = get_cached("https://test.example.com");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().model.as_deref(), Some("cached-model"));
        clear_cache();
    }

    #[test]
    fn cache_miss_for_unknown_url() {
        clear_cache();
        assert!(get_cached("https://unknown.example.com").is_none());
    }

    #[tokio::test]
    async fn load_wellknown_returns_default_when_no_auth_file() {
        // With no auth.json, should return default config without error
        let config = load_wellknown().await;
        assert!(config.model.is_none());
    }
}
