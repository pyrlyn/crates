//! `ProviderError`: the failures a `Provider` can report. `Clone +
//! Serialize + Deserialize` because it rides inside `ProviderEvent::Error`
//! and must survive a JSONL rollout round trip, so a foreign error always
//! collapses to a bare variant or a `String` message at the boundary that
//! produced it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Failures from a `Provider` implementation (`cox-provider`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderError {
    /// The provider rejected the request's credentials.
    #[error("provider auth failed")]
    Auth,
    /// The provider is rate-limiting this key.
    #[error("rate limited{}", .retry_after.map(|s| format!(", retry after {s}s")).unwrap_or_default())]
    RateLimited {
        /// Seconds to wait before retrying, from the provider's `retry-after`, if given.
        retry_after: Option<u64>,
    },
    /// The provider is temporarily overloaded (5xx, no retry-after).
    #[error("provider overloaded")]
    Overloaded,
    /// The provider rejected the request shape itself (4xx, not auth).
    #[error("bad request: {message}")]
    BadRequest {
        /// The provider's error message, verbatim.
        message: String,
    },
    /// The assembled request exceeds the model's context window.
    #[error("context too long: {got} tokens > {max} max")]
    ContextTooLong {
        /// The model's max context, in tokens.
        max: u32,
        /// The estimated size of the request that was rejected.
        got: u32,
    },
    /// The model declined to continue (content policy, not an error).
    #[error("refusal: {detail}")]
    Refusal {
        /// The provider's refusal text, if any.
        detail: String,
    },
    /// A transport-level failure (DNS, TLS, connection reset).
    #[error("network error")]
    Network,
    /// The request exceeded `providers.*.timeout_s`.
    #[error("provider timed out")]
    Timeout,
    /// The stream was cancelled via `Interrupt`.
    #[error("provider call cancelled")]
    Cancelled,
    /// The SSE/JSON stream contained a line cox could not parse.
    #[error("parse error at line {line}")]
    Parse {
        /// 1-based line number within the stream where parsing failed.
        line: u64,
    },
    /// The request used a capability (`thinking`, `cache`, …) the provider lacks.
    #[error("unsupported feature: {feature}")]
    Unsupported {
        /// The capability name, matching a `Caps` field.
        feature: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn provider_error_json_roundtrip() {
        let err = ProviderError::RateLimited {
            retry_after: Some(30),
        };
        let json = serde_json::to_string(&err).expect("serialize");
        assert_eq!(json, r#"{"type":"rate_limited","retry_after":30}"#);
        let back: ProviderError = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(err, back);
    }
}
