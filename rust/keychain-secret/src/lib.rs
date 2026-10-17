//! Secrets for applications: an environment variable first, then the OS
//! keychain, and never a config file. A [`Secret`] cannot be printed by
//! accident; [`reject_inline_secrets`] refuses a TOML config that holds one;
//! [`MemoryStore`] and [`guard`] keep tests away from the real keychain.
//!
//! Extracted from runa's `runa-cloud` (`secrets.rs`), cox's
//! `cox-provider-http` key lookup and its `no_real_keychain_in_tests` guard,
//! and aulo's `aulo-server` token store.
//!
//! ```
//! use keychain_secret::{MemoryStore, SecretStore, Source, resolve_with};
//!
//! let store = MemoryStore::default();
//! store.set("openai", &"sk-from-keychain".into())?;
//! let env = |name: &str| (name == "OPENAI_API_KEY").then(|| "sk-from-env".to_owned());
//! let found = resolve_with("OPENAI_API_KEY", "openai", &store, env)?.expect("a key");
//! assert_eq!(found.source, Source::Env);
//! assert_eq!(found.secret.redacted(), "…-env");
//! # Ok::<(), keychain_secret::Error>(())
//! ```

pub mod guard;
mod inline;
mod store;

use std::fmt;

pub use inline::{is_secret_key, reject_inline_secrets};
pub use store::{Keychain, MemoryStore, NoStore, SecretStore, keychain_enabled, os_store};

/// Why a secret could not be read, stored or accepted. Messages never
/// contain the secret.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The OS keychain failed. Only the message is kept: some keychain errors
    /// carry the stored bytes in their `Debug` output.
    #[error("the OS keychain failed: {0}")]
    Keychain(String),
    /// The keychain is switched off, so nothing can be stored in it.
    #[error("the OS keychain is switched off")]
    Disabled,
    /// A config file holds a secret inline.
    #[error(
        "{origin}: the secret at `{key}` is rejected; set it in an environment variable or the OS keychain, not in a config file"
    )]
    Inline {
        /// The config file, as the caller names it.
        origin: String,
        /// The dotted path of the offending key.
        key: String,
    },
    /// A config file is not valid TOML.
    #[error("{origin}: invalid TOML: {message}")]
    InvalidToml {
        /// The config file, as the caller names it.
        origin: String,
        /// The parser's message.
        message: String,
    },
}

/// A secret value. Its `Debug` is redacted and it has no `Display`, so
/// formatting it by accident cannot leak it. No `PartialEq` either: compare
/// secrets in constant time where it matters, never with `==`.
#[derive(Clone)]
pub struct Secret(String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl Secret {
    /// The secret text, to send where it belongs. Never pass it to a log macro.
    pub fn reveal(&self) -> &str {
        &self.0
    }

    /// The last four characters for diagnostics (`…abcd`); `****` for four or
    /// fewer, `(empty)` for none.
    pub fn redacted(&self) -> String {
        let t = self.0.trim();
        let n = t.chars().count();
        match n {
            0 => "(empty)".into(),
            1..=4 => "****".into(),
            _ => format!("…{}", t.chars().skip(n - 4).collect::<String>()),
        }
    }
}

/// Where a resolved secret came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The environment variable.
    Env,
    /// The secret store (the OS keychain in production).
    Store,
}

/// A secret and its origin.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The value, trimmed.
    pub secret: Secret,
    /// Where it came from.
    pub source: Source,
}

/// `env_var` from the process environment when it is set and not blank,
/// else `account` in `store`. `None` when neither has one: the caller decides
/// whether a missing secret is an error.
///
/// # Errors
///
/// [`Error::Keychain`] when the store fails for a reason other than a
/// missing entry.
pub fn resolve(
    env_var: &str,
    account: &str,
    store: &dyn SecretStore,
) -> Result<Option<Resolved>, Error> {
    resolve_with(env_var, account, store, |name| std::env::var(name).ok())
}

/// [`resolve`] with an injected environment lookup, for tests.
///
/// # Errors
///
/// As [`resolve`].
pub fn resolve_with(
    env_var: &str,
    account: &str,
    store: &dyn SecretStore,
    getenv: impl Fn(&str) -> Option<String>,
) -> Result<Option<Resolved>, Error> {
    if let Some(value) = getenv(env_var).map(|v| v.trim().to_owned())
        && !value.is_empty()
    {
        return Ok(Some(Resolved {
            secret: Secret(value),
            source: Source::Env,
        }));
    }
    Ok(store
        .get(account)?
        .map(|s| Secret(s.0.trim().to_owned()))
        .filter(|s| !s.0.is_empty())
        .map(|secret| Resolved {
            secret,
            source: Source::Store,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn env_wins_over_the_store_and_is_trimmed() {
        let store = MemoryStore::default();
        store.set("openai", &"sk-store".into()).expect("set");
        let found = resolve_with(
            "OPENAI_API_KEY",
            "openai",
            &store,
            env(&[("OPENAI_API_KEY", " sk-env ")]),
        )
        .expect("resolves")
        .expect("found");
        assert_eq!(
            (found.secret.reveal(), found.source),
            ("sk-env", Source::Env)
        );
    }

    #[test]
    fn a_blank_env_falls_back_to_the_store() {
        let store = MemoryStore::default();
        store
            .set("anthropic", &" sk-ant-store ".into())
            .expect("set");
        let found = resolve_with(
            "ANTHROPIC_API_KEY",
            "anthropic",
            &store,
            env(&[("ANTHROPIC_API_KEY", "  ")]),
        )
        .expect("resolves")
        .expect("found");
        assert_eq!(
            (found.secret.reveal(), found.source),
            ("sk-ant-store", Source::Store)
        );
    }

    #[test]
    fn nothing_anywhere_is_none_and_a_blank_stored_value_too() {
        let store = MemoryStore::default();
        assert!(
            resolve_with("K", "a", &store, env(&[]))
                .expect("ok")
                .is_none()
        );
        store.set("a", &"   ".into()).expect("set");
        assert!(
            resolve_with("K", "a", &store, env(&[]))
                .expect("ok")
                .is_none()
        );
        assert!(
            resolve_with("K", "a", &NoStore, env(&[]))
                .expect("ok")
                .is_none()
        );
    }

    #[test]
    fn a_secret_never_formats_its_value() {
        let s = Secret::from("sk-live-abcdef");
        assert_eq!(format!("{s:?}"), "Secret([REDACTED])");
        assert_eq!(s.redacted(), "…cdef");
        assert_eq!(Secret::from("abc").redacted(), "****");
        assert_eq!(Secret::from(" ").redacted(), "(empty)");
        let found = Resolved {
            secret: s,
            source: Source::Env,
        };
        assert!(!format!("{found:?}").contains("abcdef"));
    }
}
