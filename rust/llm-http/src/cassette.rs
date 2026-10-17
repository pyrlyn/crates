//! In-memory cassettes of HTTP exchanges for tests of a host that sends its
//! requests through its own HTTP stack.
//!
//! A [`Cassette`] stores the request body, the response status and the response
//! body of each call after secret masking, so a fixture file never keeps a
//! bearer token or an API key. Replay compares the masked request body. The
//! masking is [`llm_wire::test_util::replay::redact_secrets`], the same one the
//! provider-level cassettes use, so both kinds of fixture agree on what a
//! secret looks like. No socket is opened and no HTTP type is needed: the host
//! passes the body in and turns the [`Recorded`] answer into its own response.
//!
//! The provider-level cassette (`llm_wire::test_util::replay`) hashes a neutral
//! `Request` and stores an SSE stream; this one sits a layer lower and keeps the
//! raw bodies.

use std::fmt;

use llm_wire::test_util::replay::redact_secrets;
use serde::{Deserialize, Serialize};

/// Why a cassette could not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The text is not a cassette written by [`Cassette::save`].
    NotACassette,
    /// No stored exchange matches the masked request body.
    Miss,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotACassette => "text is not a cassette",
            Self::Miss => "cassette has no exchange for this request body",
        })
    }
}

impl std::error::Error for Error {}

/// One recorded call, already masked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exchange {
    /// Masked request body.
    pub request: String,
    /// HTTP status of the response.
    pub status: u16,
    /// Masked response body.
    pub response: String,
}

/// The answer a cassette gives for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recorded<'a> {
    /// HTTP status of the stored response.
    pub status: u16,
    /// Stored response body.
    pub body: &'a str,
}

/// A list of masked exchanges.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Cassette {
    exchanges: Vec<Exchange>,
}

impl Cassette {
    /// An empty cassette.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses text written by [`Cassette::save`].
    ///
    /// # Errors
    ///
    /// [`Error::NotACassette`] when `text` is not a cassette.
    pub fn load(text: &str) -> Result<Self, Error> {
        serde_json::from_str(text).map_err(|_| Error::NotACassette)
    }

    /// JSON for a fixture file. Exchanges are already masked.
    #[must_use]
    pub fn save(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| String::from("{\"exchanges\":[]}"))
    }

    /// Stores a call after masking secrets in both bodies.
    pub fn record(&mut self, request: &str, status: u16, response: &str) {
        self.exchanges.push(Exchange {
            request: redact_secrets(request),
            status,
            response: redact_secrets(response),
        });
    }

    /// The stored answer for `request`, compared after masking. When the same
    /// body was recorded more than once, the earliest match wins.
    ///
    /// # Errors
    ///
    /// [`Error::Miss`] when no exchange matches.
    pub fn replay(&self, request: &str) -> Result<Recorded<'_>, Error> {
        let key = redact_secrets(request);
        self.exchanges
            .iter()
            .find(|exchange| exchange.request == key)
            .map(|exchange| Recorded {
                status: exchange.status,
                body: exchange.response.as_str(),
            })
            .ok_or(Error::Miss)
    }

    /// Number of stored exchanges.
    #[must_use]
    pub fn len(&self) -> usize {
        self.exchanges.len()
    }

    /// Whether nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.exchanges.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST: &str = r#"{"authorization":"Bearer sk-fixturevalue1234","model":"m"}"#;

    #[test]
    fn a_saved_cassette_keeps_no_token_and_still_replays() {
        let mut cassette = Cassette::new();
        cassette.record(
            REQUEST,
            200,
            r#"{"text":"hello","key":"sk-ant-api03-abcdef123456"}"#,
        );
        let saved = cassette.save();
        assert!(!saved.contains("sk-fixturevalue1234"));
        assert!(!saved.contains("sk-ant-api03-abcdef123456"));
        assert!(saved.contains("redacted"));

        let loaded = Cassette::load(&saved).expect("saved text loads");
        let answer = loaded.replay(REQUEST).expect("same request replays");
        assert_eq!(answer.status, 200);
        assert!(answer.body.contains("hello"));
    }

    #[test]
    fn the_status_is_stored_and_replayed() {
        let mut cassette = Cassette::new();
        cassette.record("{}", 429, r#"{"error":"slow down"}"#);
        let answer = cassette.replay("{}").expect("stored");
        assert_eq!(answer.status, 429);
        assert_eq!(answer.body, r#"{"error":"slow down"}"#);
    }

    #[test]
    fn an_unknown_request_is_a_miss() {
        let mut cassette = Cassette::new();
        cassette.record("{\"a\":1}", 200, "x");
        assert_eq!(cassette.replay("{\"a\":2}"), Err(Error::Miss));
        assert_eq!(Cassette::new().replay("{}"), Err(Error::Miss));
    }

    #[test]
    fn the_earliest_of_two_equal_requests_wins() {
        let mut cassette = Cassette::new();
        cassette.record("{}", 500, "first");
        cassette.record("{}", 200, "second");
        let answer = cassette.replay("{}").expect("stored");
        assert_eq!((answer.status, answer.body), (500, "first"));
        assert_eq!(cassette.len(), 2);
        assert!(!cassette.is_empty());
    }

    #[test]
    fn text_that_is_not_a_cassette_is_rejected() {
        assert_eq!(Cassette::load("nope"), Err(Error::NotACassette));
        assert_eq!(Cassette::load("[]"), Err(Error::NotACassette));
    }
}
