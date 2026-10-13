//! Shared HTTP plumbing for the network backends: credential lookup, auth
//! headers, and non-2xx → [`ProviderError`] mapping.
//!
//! Anthropic and OpenAI-shaped servers phrase errors differently but map to
//! the same taxonomy, so the union lives here once instead of once per
//! backend. Every status a contract test asserts on maps exactly as before;
//! the only deliberate widening is that a 5xx is `Overloaded` (retryable) on
//! every backend — a 500 from Anthropic is transient per their own docs, and
//! treating it as a fatal `BadRequest` retried nothing.

#[cfg(feature = "reqwest")]
use std::time::Duration;

use ::http::StatusCode;
use ::http::header::HeaderValue;
use llm_wire::ProviderError;

/// How long a TCP/TLS handshake may take before a call is `Network`. Shared
/// by every backend that builds its own `reqwest::Client`.
#[cfg(feature = "reqwest")]
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds the connection-pooled client every network backend uses: a
/// bounded connect timeout plus a read/idle timeout from the section's
/// `timeout_s`. The read timeout is the gap
/// between bytes on an open stream, not a whole-call cap, so a long answer
/// keeps streaming for minutes without ever going idle.
#[cfg(feature = "reqwest")]
pub fn client_with_timeout(timeout_s: u32) -> Result<reqwest::Client, ProviderError> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(Duration::from_secs(u64::from(timeout_s).max(1)))
        .build()
        .map_err(|_| ProviderError::Network)
}

/// `env_var` first, else the platform keyring entry `service`/`account` (the
/// native store per platform, not a mock). A blank env var counts as unset.
///
/// `switch_env` names an environment variable that turns the keyring off:
/// when it holds `off`, `0` or `false` (see [`keyring_enabled`]) the keyring
/// is never touched, so a test or dev run neither raises a keychain prompt
/// nor depends on the developer's stored keys. An unset or misspelt value
/// keeps the keyring on.
///
/// A missing or unreadable credential is [`ProviderError::Auth`]; callers
/// decide what that means for their section. Wires that always need a key
/// propagate the error with `?`; OpenAI-shaped sections build with an
/// `Option<String>` key and call `.ok()`, because a local server or a
/// self-hosted gateway commonly needs none. It never panics.
#[cfg(feature = "keyring")]
pub fn resolve_key(
    env_var: &str,
    service: &str,
    account: &str,
    switch_env: &str,
) -> Result<String, ProviderError> {
    resolve_key_with(env_var, account, |account| {
        let switch = std::env::var(switch_env).ok();
        platform_keyring(service, account, switch.as_deref())
    })
}

/// [`resolve_key`]'s body, taking the keyring lookup as a parameter so
/// tests can exercise the env-then-keyring precedence without touching the
/// real platform store: `keyring::Entry`'s v1-compatibility shim binds to
/// the real Keychain/Credential-Manager/Secret-Service the first time
/// *anything* touches it, once per process and irreversibly (it is a
/// `LazyLock` that always wins over a prior `set_default_store`), so no
/// mock swapped in afterwards would ever be consulted. `pub` so every
/// wire's own tests, wherever they live, can inject a fake lookup too
/// instead of calling [`resolve_key`] and reaching the real keyring.
pub fn resolve_key_with(
    env_var: &str,
    account: &str,
    keyring_lookup: impl FnOnce(&str) -> Option<String>,
) -> Result<String, ProviderError> {
    resolve_with_env(
        |name| std::env::var(name).ok(),
        env_var,
        account,
        keyring_lookup,
    )
}

/// The precedence rule with the environment injected, so tests can set a
/// variable without `unsafe` and without racing other tests' reads.
fn resolve_with_env(
    read_env: impl Fn(&str) -> Option<String>,
    env_var: &str,
    account: &str,
    keyring_lookup: impl FnOnce(&str) -> Option<String>,
) -> Result<String, ProviderError> {
    match read_env(env_var) {
        Some(k) if !k.trim().is_empty() => return Ok(k),
        _ => {}
    }
    keyring_lookup(account).ok_or(ProviderError::Auth)
}

/// Whether the keyring may be used, given the value of the switch variable
/// ([`resolve_key`]'s `switch_env`): only `off`, `0` or `false` (any case,
/// trimmed) disable it, so an unset or misspelt value keeps the documented
/// env-then-keyring behaviour.
pub fn keyring_enabled(value: Option<&str>) -> bool {
    !matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("off" | "0" | "false")
    )
}

/// The real platform keyring entry `service`/`account`, or nothing when the
/// switch value turns the keyring off. The switch is checked before
/// `keyring::Entry` is constructed, so a disabled keyring is never touched.
#[cfg(feature = "keyring")]
fn platform_keyring(service: &str, account: &str, switch: Option<&str>) -> Option<String> {
    if !keyring_enabled(switch) {
        return None;
    }
    keyring::Entry::new(service, account)
        .and_then(|e| e.get_password())
        .ok()
}

/// `Authorization: Bearer <key>`, marked sensitive. A key with non-ASCII
/// bytes is a misconfigured credential (`Auth`), not a transport failure.
pub fn bearer(key: &str) -> Result<HeaderValue, ProviderError> {
    let mut v = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| ProviderError::Auth)?;
    v.set_sensitive(true);
    Ok(v)
}

/// `x-api-key: <key>`, marked sensitive. Same Auth-on-garbage rule as
/// [`bearer`].
pub fn api_key(key: &str) -> Result<HeaderValue, ProviderError> {
    let mut v = HeaderValue::from_str(key).map_err(|_| ProviderError::Auth)?;
    v.set_sensitive(true);
    Ok(v)
}

/// The `{"error": {"message": ...}}` envelope both API families use (Ollama
/// included); falls back to the raw body so a proxy's plain-text error is
/// not lost.
pub fn error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.get("message")?.as_str().map(str::to_string))
        .unwrap_or_else(|| body.to_string())
}

/// Best-effort `(got, max)` extraction from a too-long/context-exceeded
/// message. Union of both families' phrasings ("too long", "exceed" —
/// Ollama says "exceeds context length", Anthropic says "too long"); no
/// documented fixed schema exists, so anything else falls back to
/// `BadRequest` at the call site rather than guessing.
pub fn parse_context_too_long(message: &str) -> Option<(u32, u32)> {
    let lower = message.to_ascii_lowercase();
    if !(lower.contains("too long") || lower.contains("exceed")) {
        return None;
    }
    let mut numbers = message
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u32>().ok());
    let got = numbers.next()?;
    let max = numbers.next()?;
    Some((got, max))
}

/// Maps a non-2xx response to a [`ProviderError`]. `retry_after` comes from
/// the `retry-after` header, read before the body is consumed.
pub fn map_http_error(status: StatusCode, body: &str, retry_after: Option<u64>) -> ProviderError {
    let message = error_message(body);
    match status.as_u16() {
        401 | 403 => ProviderError::Auth,
        429 => ProviderError::RateLimited { retry_after },
        500 | 502 | 503 | 504 | 529 => ProviderError::Overloaded,
        // A too-long prompt is a 400 `invalid_request_error` in practice
        // (413 is reserved for raw request-body size); handled the same way
        // regardless of which status carried it.
        400 | 413 => match parse_context_too_long(&message) {
            Some((got, max)) => ProviderError::ContextTooLong { max, got },
            None => ProviderError::BadRequest { message },
        },
        _ => ProviderError::BadRequest { message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_union_mapping_preserves_both_backends_cases() {
        // Anthropic's asserted cases.
        assert!(matches!(
            map_http_error(StatusCode::UNAUTHORIZED, "{}", None),
            ProviderError::Auth
        ));
        assert!(matches!(
            map_http_error(StatusCode::TOO_MANY_REQUESTS, "{}", Some(30)),
            ProviderError::RateLimited {
                retry_after: Some(30)
            }
        ));
        assert!(matches!(
            map_http_error(StatusCode::SERVICE_UNAVAILABLE, "{}", None),
            ProviderError::Overloaded
        ));
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 250000 tokens > 200000 maximum"}}"#;
        assert_eq!(
            map_http_error(StatusCode::BAD_REQUEST, body, None),
            ProviderError::ContextTooLong {
                max: 200_000,
                got: 250_000,
            }
        );
        // Chat's asserted cases: 403 is auth, Ollama's "exceeds" phrasing parses.
        assert!(matches!(
            map_http_error(StatusCode::FORBIDDEN, "{}", None),
            ProviderError::Auth
        ));
        let body = r#"{"error":{"message":"input length exceeds context length: 40000 tokens > 32768 maximum","type":"invalid_request_error"}}"#;
        assert_eq!(
            map_http_error(StatusCode::BAD_REQUEST, body, None),
            ProviderError::ContextTooLong {
                max: 32_768,
                got: 40_000,
            }
        );
        // Plain-text proxy body survives.
        assert_eq!(
            map_http_error(StatusCode::BAD_REQUEST, "nope", None),
            ProviderError::BadRequest {
                message: "nope".into()
            }
        );
    }

    #[test]
    fn http_bearer_and_api_key_reject_garbage_as_auth() {
        assert!(bearer("sk-test").is_ok());
        assert!(api_key("sk-test").is_ok());
        // A control byte can never be a credential: auth problem, not transport.
        assert!(matches!(bearer("a\nb"), Err(ProviderError::Auth)));
        assert!(matches!(api_key("a\nb"), Err(ProviderError::Auth)));
    }

    #[test]
    fn bearer_and_api_key_headers_are_marked_sensitive() {
        // A sensitive header is redacted from `Debug` output, so a logged
        // request never carries the key.
        for header in [bearer("sk-secret"), api_key("sk-secret")] {
            let header = header.expect("valid credential");
            assert!(header.is_sensitive());
            assert!(!format!("{header:?}").contains("sk-secret"));
        }
    }

    #[test]
    fn auth_error_text_never_carries_the_key() {
        let err = bearer("sk-secret\n").expect_err("control byte rejected");
        assert!(!format!("{err} {err:?}").contains("sk-secret"));
    }

    #[test]
    fn keyring_switch_disables_only_on_off_zero_false() {
        assert!(keyring_enabled(None));
        assert!(keyring_enabled(Some("on")));
        assert!(keyring_enabled(Some("offf")));
        assert!(keyring_enabled(Some("")));
        for off in ["off", "OFF", " 0 ", "False"] {
            assert!(!keyring_enabled(Some(off)), "{off}");
        }
    }

    /// A switched-off keyring answers before `keyring::Entry` exists, so this
    /// never reaches the real platform store.
    #[cfg(feature = "keyring")]
    #[test]
    fn platform_keyring_is_never_consulted_when_switched_off() {
        assert_eq!(platform_keyring("llm-http-test", "acct", Some("off")), None);
    }

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn resolve_key_prefers_the_env_var_over_the_keyring() {
        let got = resolve_with_env(env_of(&[("KEY_VAR", "sk-env")]), "KEY_VAR", "acct", |_| {
            panic!("the keyring must not be consulted when the env var is set")
        });
        assert_eq!(got.ok().as_deref(), Some("sk-env"));
    }

    /// A section with no env var set falls back to the keyring entry named
    /// after it, through the one resolver every section shares.
    #[test]
    fn resolve_key_falls_back_to_the_keyring_when_the_env_var_is_unset() {
        let got = resolve_with_env(env_of(&[]), "KEY_VAR", "test-section", |account| {
            assert_eq!(account, "test-section");
            Some("sk-from-keyring".to_string())
        });
        assert_eq!(got.ok().as_deref(), Some("sk-from-keyring"));
    }

    #[test]
    fn resolve_key_treats_a_blank_env_var_as_unset() {
        let got = resolve_with_env(env_of(&[("KEY_VAR", "  \t")]), "KEY_VAR", "acct", |_| {
            Some("sk-from-keyring".to_string())
        });
        assert_eq!(got.ok().as_deref(), Some("sk-from-keyring"));
    }

    #[test]
    fn resolve_key_is_auth_error_when_neither_env_nor_keyring_has_one() {
        let got = resolve_with_env(env_of(&[]), "KEY_VAR", "acct", |_| None);
        assert!(matches!(got, Err(ProviderError::Auth)));
    }

    /// The public entry point reads the real environment; an unset variable
    /// with the keyring switched off must resolve to `Auth` without touching
    /// the platform store.
    #[cfg(feature = "keyring")]
    #[test]
    fn resolve_key_with_the_keyring_switch_off_is_auth_when_env_is_unset() {
        let got = resolve_key_with("LLM_HTTP_TEST_NO_SUCH_VAR", "acct", |account| {
            platform_keyring("llm-http-test", account, Some("off"))
        });
        assert!(matches!(got, Err(ProviderError::Auth)));
    }
}
