use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthInfo {
    #[serde(rename = "api")]
    Api { key: String },
    #[serde(rename = "oauth")]
    OAuth {
        access: String,
        #[serde(default)]
        refresh: String,
        expires: Option<i64>,
        #[serde(alias = "accountId")]
        account_id: Option<String>,
        #[serde(alias = "enterpriseUrl")]
        enterprise_url: Option<String>,
    },
    #[serde(rename = "wellknown")]
    WellKnown {
        /// Environment variable name to set with the token
        key: String,
        /// The authentication token value
        token: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthMethod {
    pub auth_type: AuthType,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthType {
    OAuth,
    Api,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Authorization {
    pub url: String,
    pub method: AuthMethodType,
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthMethodType {
    Auto,
    Code,
}

pub struct AuthManager {
    credentials: Arc<RwLock<HashMap<String, AuthInfo>>>,
    filepath: Option<PathBuf>,
}

impl AuthManager {
    pub fn new() -> Self {
        Self {
            credentials: Arc::new(RwLock::new(HashMap::new())),
            filepath: None,
        }
    }

    pub fn with_filepath(filepath: PathBuf) -> Self {
        Self {
            credentials: Arc::new(RwLock::new(HashMap::new())),
            filepath: Some(filepath),
        }
    }

    pub async fn load_from_file(data_dir: &Path) -> Self {
        let filepath = data_dir.join("auth.json");
        let manager = Self::with_filepath(filepath.clone());
        if let Ok(content) = tokio::fs::read_to_string(&filepath).await {
            if let Ok(data) = serde_json::from_str::<HashMap<String, AuthInfo>>(&content) {
                let mut creds = manager.credentials.write().await;
                *creds = data;
            }
        }
        manager
    }

    pub async fn get(&self, provider_id: &str) -> Option<AuthInfo> {
        let creds = self.credentials.read().await;
        creds.get(provider_id).cloned()
    }

    pub async fn set(&self, provider_id: &str, auth: AuthInfo) {
        {
            let mut creds = self.credentials.write().await;
            creds.insert(provider_id.to_string(), auth);
        }
        if let Err(error) = self.persist().await {
            tracing::warn!(%error, provider_id, "failed to persist auth store");
        }
    }

    pub async fn remove(&self, provider_id: &str) {
        {
            let mut creds = self.credentials.write().await;
            creds.remove(provider_id);
        }
        if let Err(error) = self.persist().await {
            tracing::warn!(%error, provider_id, "failed to persist auth store");
        }
    }

    pub async fn has_auth(&self, provider_id: &str) -> bool {
        let creds = self.credentials.read().await;
        creds.contains_key(provider_id)
    }

    pub async fn get_api_key(&self, provider_id: &str) -> Option<String> {
        let creds = self.credentials.read().await;
        match creds.get(provider_id) {
            Some(AuthInfo::Api { key }) => Some(key.clone()),
            _ => None,
        }
    }

    pub async fn get_oauth_token(&self, provider_id: &str) -> Option<String> {
        let creds = self.credentials.read().await;
        match creds.get(provider_id) {
            Some(AuthInfo::OAuth { access, .. }) => Some(access.clone()),
            _ => None,
        }
    }

    pub async fn list(&self) -> HashMap<String, AuthInfo> {
        let creds = self.credentials.read().await;
        creds.clone()
    }

    async fn persist(&self) -> anyhow::Result<()> {
        let Some(path) = self.filepath.as_ref() else {
            return Ok(());
        };

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let creds = self.credentials.read().await;
        let json = serde_json::to_vec_pretty(&*creds)?;
        tokio::fs::write(path, json).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
        }

        Ok(())
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("OAuth pending request not found for provider: {0}")]
    OauthMissing(String),

    #[error("OAuth authorization code missing for provider: {0}")]
    OauthCodeMissing(String),

    #[error("OAuth callback failed")]
    OauthCallbackFailed,

    #[error("API key not set for provider: {0}")]
    ApiKeyNotSet(String),
}

pub fn get_env_key(provider_id: &str) -> Option<String> {
    let env_var = format!("{}_API_KEY", provider_id.to_uppercase().replace("-", "_"));
    std::env::var(&env_var).ok()
}

pub fn get_env_key_or(provider_id: &str, default: &str) -> String {
    get_env_key(provider_id).unwrap_or_else(|| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_auth_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("opencode-auth-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp auth dir");
        dir
    }

    #[tokio::test]
    async fn auth_manager_persists_and_reloads_file() {
        let dir = temp_auth_dir();
        let manager = AuthManager::load_from_file(&dir).await;

        manager
            .set(
                "openai",
                AuthInfo::OAuth {
                    access: "access-token".to_string(),
                    refresh: "refresh-token".to_string(),
                    expires: Some(1234),
                    account_id: Some("acct_1".to_string()),
                    enterprise_url: Some("https://enterprise.example.com".to_string()),
                },
            )
            .await;
        manager
            .set(
                "github-copilot",
                AuthInfo::Api {
                    key: "copilot-key".to_string(),
                },
            )
            .await;

        let auth_path = dir.join("auth.json");
        assert!(auth_path.exists());

        let reloaded = AuthManager::load_from_file(&dir).await;
        match reloaded.get("openai").await {
            Some(AuthInfo::OAuth {
                access,
                refresh,
                account_id,
                enterprise_url,
                ..
            }) => {
                assert_eq!(access, "access-token");
                assert_eq!(refresh, "refresh-token");
                assert_eq!(account_id.as_deref(), Some("acct_1"));
                assert_eq!(
                    enterprise_url.as_deref(),
                    Some("https://enterprise.example.com")
                );
            }
            other => panic!("unexpected oauth entry: {other:?}"),
        }

        match reloaded.get("github-copilot").await {
            Some(AuthInfo::Api { key }) => assert_eq!(key, "copilot-key"),
            other => panic!("unexpected api entry: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn auth_manager_remove_persists_deletion() {
        let dir = temp_auth_dir();
        let manager = AuthManager::load_from_file(&dir).await;
        manager
            .set(
                "openai",
                AuthInfo::Api {
                    key: "to-delete".to_string(),
                },
            )
            .await;

        manager.remove("openai").await;
        let reloaded = AuthManager::load_from_file(&dir).await;
        assert!(reloaded.get("openai").await.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
