//! The connection settings every HTTP wire reads from its config section.

/// The transport knobs one provider section carries. Mirrors cox's
/// `cox_protocol::config::Transport`, which carries cox's whole config
/// schema; a caller copies the four fields across.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transport {
    /// API base URL, e.g. `https://api.anthropic.com`.
    pub base_url: String,
    /// Env var holding the API key; the caller resolves it (a blank or unset
    /// key builds a keyless client with no auth header).
    pub api_key_env: String,
    /// Read/idle timeout in seconds, between stream chunks.
    pub timeout_s: u32,
    /// Max retries for retryable errors.
    pub max_retries: u32,
}
