//! The OS keyring as a [`Secrets`] store (feature `keyring`): entry
//! `<service>/mcp/<server>`, value = rmcp's `StoredCredentials` as JSON
//! (access and refresh token, expiry, client id). A host that wants a kill
//! switch for tests and dev runs names an environment variable; when it holds
//! `off`, `0` or `false` every entry is "not there": reads find nothing and
//! writes fail with a message, instead of a keychain prompt.

use std::sync::Arc;

use async_trait::async_trait;
use rmcp::transport::auth::{AuthError, CredentialStore, StoredCredentials};

use crate::auth::Secrets;

/// The OS keyring under one `service` name.
#[derive(Clone)]
pub struct Keyring {
    service: String,
    off_switch: Option<String>,
    // why: a test cannot set a process variable (`unsafe_code` is forbidden),
    // so it swaps the lookup instead.
    lookup: fn(&str) -> Option<String>,
}

fn env_lookup(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl Keyring {
    /// Entries are filed under `service`.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            off_switch: None,
            lookup: env_lookup,
        }
    }

    /// Names the environment variable that turns the keyring off.
    #[must_use]
    pub fn with_off_switch(mut self, variable: impl Into<String>) -> Self {
        self.off_switch = Some(variable.into());
        self
    }

    /// Reads the entry for `server` synchronously, for a caller with no
    /// runtime (a `doctor` command).
    pub fn stored(&self, server: &str) -> Result<Option<StoredCredentials>, AuthError> {
        self.config().read(&account(server))
    }

    fn config(&self) -> Config {
        Config {
            service: self.service.clone(),
            enabled: self.enabled(),
        }
    }

    fn enabled(&self) -> bool {
        let value = self.off_switch.as_deref().and_then(self.lookup);
        keyring_enabled(value.as_deref())
    }
}

impl Secrets for Keyring {
    fn store(&self, server: &str) -> Arc<dyn CredentialStore> {
        Arc::new(KeyringStore {
            keyring: self.clone(),
            account: account(server),
        })
    }
}

/// Whether the keyring may be used, given the off switch's value: only
/// `off`, `0` or `false` (any case, trimmed) disable it, so an unset or
/// misspelt value keeps the normal behaviour.
fn keyring_enabled(value: Option<&str>) -> bool {
    !matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("off" | "0" | "false")
    )
}

fn account(server: &str) -> String {
    format!("mcp/{server}")
}

/// What one blocking keyring call needs, owned so it can move into
/// `spawn_blocking`.
struct Config {
    service: String,
    enabled: bool,
}

fn store_error(e: impl ToString) -> AuthError {
    AuthError::CredentialStoreError(e.to_string())
}

impl Config {
    fn entry(&self, account: &str) -> Result<keyring::Entry, AuthError> {
        if !self.enabled {
            return Err(store_error("keyring disabled by its off switch"));
        }
        keyring::Entry::new(&self.service, account).map_err(store_error)
    }

    fn read(&self, account: &str) -> Result<Option<StoredCredentials>, AuthError> {
        if !self.enabled {
            return Ok(None);
        }
        match self.entry(account)?.get_password() {
            Ok(json) => serde_json::from_str(&json)
                .map(Some)
                .map_err(|e| store_error(format!("keyring entry unreadable: {e}"))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(e)),
        }
    }
}

/// One keyring entry. Reads and writes go through `spawn_blocking` because
/// the platform keychain may block on a user prompt.
struct KeyringStore {
    keyring: Keyring,
    account: String,
}

#[async_trait]
impl CredentialStore for KeyringStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let (config, account) = (self.keyring.config(), self.account.clone());
        tokio::task::spawn_blocking(move || config.read(&account))
            .await
            .map_err(store_error)?
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let (config, account) = (self.keyring.config(), self.account.clone());
        let json = serde_json::to_string(&credentials).map_err(store_error)?;
        tokio::task::spawn_blocking(move || {
            config
                .entry(&account)?
                .set_password(&json)
                .map_err(store_error)
        })
        .await
        .map_err(store_error)?
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let (config, account) = (self.keyring.config(), self.account.clone());
        tokio::task::spawn_blocking(move || match config.entry(&account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_error(e)),
        })
        .await
        .map_err(store_error)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_off_zero_and_false_disable_the_keyring() {
        for off in ["off", " OFF ", "0", "False"] {
            assert!(!keyring_enabled(Some(off)), "{off}");
        }
        for on in [None, Some(""), Some("on"), Some("offf")] {
            assert!(keyring_enabled(on), "{on:?}");
        }
    }

    fn switched_off() -> Keyring {
        let mut keyring = Keyring::new("mcp-client-host-test").with_off_switch("TEST_KEYRING");
        keyring.lookup = |_| Some("off".to_string());
        keyring
    }

    #[tokio::test]
    async fn a_switched_off_keyring_reads_nothing_and_refuses_writes() {
        let store = switched_off().store("srv");
        assert!(store.load().await.expect("load").is_none());
        let credentials = StoredCredentials::new("cid".into(), None, vec![], None);
        assert!(store.save(credentials).await.is_err());
        assert!(store.clear().await.is_err());
        assert!(switched_off().stored("srv").expect("stored").is_none());
    }
}
