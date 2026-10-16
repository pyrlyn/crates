//! Helpers for test doubles of [`Provider`](crate::traits::Provider), behind
//! the `test-util` feature so a production build carries no scenario parser,
//! no TOML and no hashing: scenario parsing and event building
//! ([`scripted`]) and cassette hashing, redaction and writing ([`replay`]).

pub mod replay;
pub mod scripted;
