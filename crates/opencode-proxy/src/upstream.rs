//! Per-upstream identity for a residential egress.
//!
//! The only subtlety here: `proxy_authorization` returns `None`, never
//! `Some("Basic Oj==")`, when a credential is empty. Sending an empty Basic header
//! makes an authenticated upstream answer `407` and an open one answer `400`, so the
//! distinction is the difference between working and not.

use base64::Engine as _;

/// One residential proxy entry this process can adopt as its upstream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upstream {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
}

impl Upstream {
    pub fn new(host: impl Into<String>, port: u16, user: impl Into<String>, pass: impl Into<String>) -> Self {
        Self { host: host.into(), port, user: user.into(), pass: pass.into() }
    }

    /// Stable identity used for quarantine and claim bookkeeping. Same string the
    /// pool file's `host:port` prefix uses, so the two namespaces line up.
    pub fn key(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn has_credentials(&self) -> bool {
        !self.user.is_empty() && !self.pass.is_empty()
    }

    /// `None` when either credential is empty. An empty `Basic Oj==` is a lie that
    /// upstream rejects, so absence must be absence.
    pub fn proxy_authorization(&self) -> Option<String> {
        if !self.has_credentials() {
            return None;
        }
        let raw = format!("{}:{}", self.user, self.pass);
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        Some(format!("Basic {encoded}"))
    }

    pub fn authority(&self) -> String {
        self.key()
    }
}

impl std::fmt::Display for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_host_port() {
        assert_eq!(Upstream::new("1.2.3.4", 8080, "u", "p").key(), "1.2.3.4:8080");
    }

    #[test]
    fn empty_credentials_send_no_header() {
        assert!(Upstream::new("h", 1, "", "").proxy_authorization().is_none());
        assert!(Upstream::new("h", 1, "u", "").proxy_authorization().is_none());
        assert!(Upstream::new("h", 1, "", "p").proxy_authorization().is_none());
    }

    #[test]
    fn populated_credentials_encode_basic_auth() {
        let header = Upstream::new("h", 1, "u", "p").proxy_authorization().unwrap();
        assert_eq!(header, "Basic dTpw");
    }
}