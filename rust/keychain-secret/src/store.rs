//! Where secrets live between runs: the OS keychain, a map in memory for
//! tests, or nothing when the keychain is switched off.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use crate::{Error, Secret};

/// A store of secrets by account name.
///
/// Implementations may block (a keychain can wait on the user), so call them
/// before an async runtime starts or from a blocking task.
pub trait SecretStore: Send + Sync {
    /// The secret for `account`, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// [`Error::Keychain`] when the store fails.
    fn get(&self, account: &str) -> Result<Option<Secret>, Error>;

    /// Stores `secret` for `account`, replacing an earlier one.
    ///
    /// # Errors
    ///
    /// [`Error::Keychain`] when the store fails, [`Error::Disabled`] when it
    /// is switched off.
    fn set(&self, account: &str, secret: &Secret) -> Result<(), Error>;

    /// Removes the secret for `account`; a missing one is not an error.
    ///
    /// # Errors
    ///
    /// As [`SecretStore::set`].
    fn delete(&self, account: &str) -> Result<(), Error>;
}

/// The OS keychain: Keychain on macOS, Credential Manager on Windows, the
/// Secret Service on Linux, under one service name.
///
/// keyring binds to the platform store the first time anything touches it,
/// once per process and irreversibly, so no fake installed afterwards is ever
/// consulted. Tests therefore use [`MemoryStore`], never this type.
#[derive(Debug, Clone)]
pub struct Keychain {
    service: String,
}

impl Keychain {
    /// Entries under `service` (the application's name, usually).
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> Result<keyring::Entry, Error> {
        keyring::Entry::new(&self.service, account).map_err(keychain)
    }
}

// Only the message is kept: some keyring errors carry the stored bytes in
// their `Debug` output, and those bytes are the secret.
fn keychain(error: keyring::Error) -> Error {
    Error::Keychain(error.to_string())
}

impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Result<Option<Secret>, Error> {
        match self.entry(account)?.get_password() {
            Ok(text) => Ok(Some(Secret::from(text))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain(e)),
        }
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), Error> {
        self.entry(account)?
            .set_password(secret.reveal())
            .map_err(keychain)
    }

    fn delete(&self, account: &str) -> Result<(), Error> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain(e)),
        }
    }
}

/// Secrets in memory: the store tests use, and one for callers that pass a
/// secret in.
#[derive(Debug, Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl MemoryStore {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        // A panic while holding the lock cannot leave the map half-written.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<Secret>, Error> {
        Ok(self.map().get(account).map(|s| Secret::from(s.as_str())))
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), Error> {
        self.map()
            .insert(account.to_owned(), secret.reveal().to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), Error> {
        self.map().remove(account);
        Ok(())
    }
}

/// The keychain switched off: nothing is found and nothing can be stored.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStore;

impl SecretStore for NoStore {
    fn get(&self, _account: &str) -> Result<Option<Secret>, Error> {
        Ok(None)
    }

    fn set(&self, _account: &str, _secret: &Secret) -> Result<(), Error> {
        Err(Error::Disabled)
    }

    fn delete(&self, _account: &str) -> Result<(), Error> {
        Err(Error::Disabled)
    }
}

/// Whether a keychain switch value leaves the keychain on: unset, empty or
/// anything but `off`, `0`, `false` or `no` (any case). Development builds
/// set the switch off so a rebuilt binary never raises a keychain prompt.
pub fn keychain_enabled(switch: Option<&str>) -> bool {
    let off = ["off", "0", "false", "no"];
    !switch.is_some_and(|v| off.iter().any(|o| v.trim().eq_ignore_ascii_case(o)))
}

/// The OS keychain under `service`, or [`NoStore`] when the environment
/// variable `switch_var` turns it off (see [`keychain_enabled`]).
pub fn os_store(service: &str, switch_var: &str) -> Box<dyn SecretStore> {
    let switch = std::env::var(switch_var).ok();
    if keychain_enabled(switch.as_deref()) {
        Box::new(Keychain::new(service))
    } else {
        Box::new(NoStore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips_and_deletes() {
        let store = MemoryStore::default();
        assert!(store.get("a").expect("get").is_none());
        store.set("a", &"one".into()).expect("set");
        store.set("a", &"two".into()).expect("replace");
        assert_eq!(store.get("a").expect("get").expect("some").reveal(), "two");
        store.delete("a").expect("delete");
        store.delete("a").expect("deleting a missing entry is fine");
        assert!(store.get("a").expect("get").is_none());
    }

    #[test]
    fn no_store_finds_nothing_and_refuses_writes() {
        assert!(NoStore.get("a").expect("get").is_none());
        assert!(matches!(
            NoStore.set("a", &"x".into()),
            Err(Error::Disabled)
        ));
        assert!(matches!(NoStore.delete("a"), Err(Error::Disabled)));
    }

    #[test]
    fn the_switch_turns_the_keychain_off_only_when_it_says_so() {
        for on in [None, Some(""), Some("on"), Some("1"), Some("yes")] {
            assert!(keychain_enabled(on), "{on:?}");
        }
        for off in ["off", "OFF", "0", "false", " No "] {
            assert!(!keychain_enabled(Some(off)), "{off}");
        }
    }
}
