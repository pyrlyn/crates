//! The one description of an MCP server the host hands to the client.
//! Neutral on purpose: a host maps its own config type onto it, so this crate
//! depends on no application.

use std::collections::HashMap;

/// One server: a stdio `command` or a remote `url`.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    /// Stdio launch command, for a local server.
    pub command: Option<String>,
    /// Arguments to `command`.
    pub args: Vec<String>,
    /// Remote Streamable HTTP URL.
    pub url: Option<String>,
    /// Extra environment variables for a stdio server.
    pub env: HashMap<String, String>,
    /// Whether the host wraps a stdio server in its sandbox. The crate never
    /// reads it; discovery sets it so the host sees which entries may opt out.
    pub sandbox: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            command: None,
            args: Vec::new(),
            url: None,
            env: HashMap::new(),
            sandbox: true,
        }
    }
}
