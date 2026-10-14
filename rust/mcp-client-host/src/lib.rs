//! An MCP client host over `rmcp`: discovery of declared servers, a
//! fail-open `connect_all`, OAuth with an injectable token store,
//! elicitation mapping and deferred, namespaced tools. MCP servers are
//! untrusted network peers: a server that will not start is a notice, never
//! an error, and nothing here depends on an application crate.

pub(crate) mod asking;
pub mod auth;
pub mod client;
pub mod config;
pub mod discovery;
pub mod elicit;
#[cfg(feature = "keyring")]
pub mod keyring;
pub mod tool;
